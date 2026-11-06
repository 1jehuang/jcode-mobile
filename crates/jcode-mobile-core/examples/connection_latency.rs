//! Live latency probe. Uses the saved account and creates one session, then
//! reattaches it through cold, immediate warm, and spaced warm mobile connections.
//! Prints no credentials. Intentional idle time is reported outside request timing.
use jcode_mobile_core::{AccountStore, cloud};
use std::time::{Duration, Instant};

const WARM_SAMPLES: usize = 10;
const WARM_PAUSE: Duration = Duration::from_millis(500);

#[derive(Clone, Copy)]
struct Sample {
    hello: Duration,
    attach: Duration,
    ready: Duration,
}

fn probe(
    account: &cloud::Account,
    session_id: &mut Option<String>,
    label: &str,
    sdk: bool,
) -> Sample {
    let start = Instant::now();
    let progress = |message: &str| println!("{label} +{:?}: {message}", start.elapsed());
    let client = if sdk {
        let baseline = cloud::client().with_api_base(account.api_base.clone());
        jcode_sdk::cloud::connect(&baseline, account, &progress, &|| false)
    } else {
        cloud::connect(account, &progress, &|| false)
    }
    .expect("connect");
    let hello = start.elapsed();
    let operation = if session_id.is_some() {
        "attach"
    } else {
        "create"
    };
    let attach_start = Instant::now();
    let session = match session_id.as_deref() {
        Some(id) => client.attach_session(id),
        None => client.create_session(None),
    }
    .expect("attach/create session");
    let attach = attach_start.elapsed();
    let ready = start.elapsed();
    *session_id = Some(session.session_id);
    println!("{label}: hello={hello:?}, {operation}={attach:?}, ready={ready:?}");
    Sample {
        hello,
        attach,
        ready,
    }
}

fn distribution(label: &str, metric: &str, values: impl Iterator<Item = Duration>) {
    let mut values: Vec<_> = values.collect();
    values.sort_unstable();
    if values.is_empty() {
        return;
    }
    let percentile = |percent: usize| values[(values.len() * percent).div_ceil(100) - 1];
    let mean = values.iter().copied().sum::<Duration>() / values.len() as u32;
    println!(
        "{label} {metric}: n={}, min={:?}, p50={:?}, p95={:?}, max={:?}, mean={mean:?}",
        values.len(),
        values[0],
        percentile(50),
        percentile(95),
        values.last().unwrap(),
    );
}

fn summary(label: &str, samples: &[Sample]) {
    distribution(label, "hello", samples.iter().map(|sample| sample.hello));
    distribution(label, "attach", samples.iter().map(|sample| sample.attach));
    distribution(label, "ready", samples.iter().map(|sample| sample.ready));
}

fn main() {
    let account = AccountStore::default_location()
        .load()
        .or_else(cloud::account_from_jcode_config)
        .expect("no saved account");
    let mut session_id: Option<String> = None;
    probe(&account, &mut session_id, "sdk baseline", true);
    probe(&account, &mut session_id, "mobile cold", false);

    println!(
        "Immediate warm: {WARM_SAMPLES} reconnects, no intentional pause or discarded samples"
    );
    let mut immediate = Vec::with_capacity(WARM_SAMPLES);
    for i in 1..=WARM_SAMPLES {
        immediate.push(probe(
            &account,
            &mut session_id,
            &format!("mobile warm immediate {i}"),
            false,
        ));
    }

    println!(
        "Spaced warm: {WARM_SAMPLES} reconnects, {WARM_PAUSE:?} idle before each, including the first; \
         idle is excluded from hello/attach/ready, not from batch wall time"
    );
    let batch_start = Instant::now();
    let mut spaced = Vec::with_capacity(WARM_SAMPLES);
    for i in 1..=WARM_SAMPLES {
        let idle_start = Instant::now();
        std::thread::sleep(WARM_PAUSE);
        println!("mobile warm spaced {i}: idle={:?}", idle_start.elapsed());
        spaced.push(probe(
            &account,
            &mut session_id,
            &format!("mobile warm spaced {i}"),
            false,
        ));
    }
    println!(
        "Spaced warm batch wall time (including idle): {:?}",
        batch_start.elapsed()
    );
    println!(
        "Summary: ready is total connect + attach; percentiles use nearest rank, no samples discarded"
    );
    summary("warm immediate", &immediate);
    summary("warm spaced", &spaced);
}
