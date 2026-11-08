//! Live startup profile against the saved account's cloud machine. Prints only
//! timings. Run with `--release`: debug crypto distorts SSH times. Each run is
//! a fresh process-equivalent start (in-memory SSH cache cleared), so it
//! measures what an app launch sees. Does not create sessions or send prompts.
use jcode_mobile_core::{AccountStore, cloud};
use std::time::Instant;

fn main() {
    let account = AccountStore::default_location()
        .load()
        .or_else(cloud::account_from_jcode_config)
        .expect("no saved account");
    let runs: usize = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(5);
    // PREWARM_MS=n: call cloud::prewarm at "app start", then connect n ms later
    // (the UI-load window), timing only from the connect call.
    let prewarm_ms: Option<u64> = std::env::var("PREWARM_MS").ok().and_then(|s| s.parse().ok());
    for run in 1..=runs {
        cloud::reset_in_memory_cache_for_benchmark();
        if let Some(ms) = prewarm_ms {
            cloud::prewarm(&account);
            std::thread::sleep(std::time::Duration::from_millis(ms));
        }
        let t = Instant::now();
        let client = cloud::connect(
            &account,
            &|m| println!("  +{:?} {m}", t.elapsed()),
            &|| false,
        )
        .expect("connect");
        let hello = t.elapsed();
        let sessions = client.list_sessions_limited(1).expect("list");
        let listed = t.elapsed();
        if let Some(s) = sessions.first() {
            client.attach_session(&s.session_id).expect("attach");
        }
        println!(
            "run {run}: ready_for_input(hello)={hello:?} listed={listed:?} attached={:?}",
            t.elapsed()
        );
        drop(client);
        std::thread::sleep(std::time::Duration::from_millis(300));
    }
}
