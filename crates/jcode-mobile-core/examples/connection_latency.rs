//! Live latency probe. Uses the saved account and creates one session, then
//! reattaches it through cold and warm mobile connections. Prints no credentials.
use jcode_mobile_core::{AccountStore, cloud};
use std::time::Instant;

fn main() {
    let account = AccountStore::default_location()
        .load()
        .or_else(cloud::account_from_jcode_config)
        .expect("no saved account");
    let mut session_id: Option<String> = None;
    let mut warm = Vec::new();
    let labels = ["sdk baseline".to_string(), "mobile cold".to_string()]
        .into_iter()
        .chain((1..=10).map(|i| format!("mobile warm {i}")));
    for label in labels {
        let start = Instant::now();
        let progress = |message: &str| println!("{label} +{:?}: {message}", start.elapsed());
        let client = if label == "sdk baseline" {
            jcode_sdk::cloud::connect(&cloud::client(), &account, &progress, &|| false)
        } else {
            cloud::connect(&account, &progress, &|| false)
        }
        .expect("connect");
        let hello = start.elapsed();
        let session = match &session_id {
            Some(id) => client.attach_session(id),
            None => client.create_session(None),
        }
        .expect("attach session");
        session_id = Some(session.session_id);
        let ready = start.elapsed();
        println!("{label}: hello={hello:?}, ready={ready:?}");
        if label.starts_with("mobile warm") {
            warm.push(ready);
        }
    }
    warm.sort();
    if !warm.is_empty() {
        println!(
            "warm ready: min={:?}, median={:?}, p95={:?}, max={:?}",
            warm[0],
            warm[warm.len() / 2],
            warm[(warm.len() * 95).div_ceil(100) - 1],
            warm.last().unwrap()
        );
    }
}
