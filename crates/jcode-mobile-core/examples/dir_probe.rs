//! Connect with a working dir and print phases. `cargo run --example dir_probe -- /path`
use jcode_mobile_core::*;
use std::time::Duration;
fn main() {
    let account = AccountStore::default_location().load().or_else(cloud::account_from_jcode_config).expect("no account");
    let mut cfg = Config::cloud(account);
    cfg.working_dir = std::env::args().nth(1);
    cfg.max_reconnect_attempts = Some(2);
    let (_c, rx) = Connection::start_with_channel(cfg, None);
    while let Ok(o) = rx.recv_timeout(Duration::from_secs(60)) {
        let s = format!("{o:?}"); println!("{}", &s[..s.len().min(400)]);
        if s.contains("Attached") || s.contains("Failed") { break; }
    }
}
