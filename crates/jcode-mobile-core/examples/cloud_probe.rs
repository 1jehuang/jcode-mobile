//! Connects to the account's Jcode Cloud host and prints events.
//! Account: ~/.local/share/jcode-mobile/account.json, else the jcode CLI's.
//! `cargo run --example cloud_probe -- [message]`
use jcode_mobile_core::*;
use std::time::Duration;

fn main() {
    let account = AccountStore::default_location()
        .load()
        .or_else(cloud::account_from_jcode_config)
        .expect("no jcode account");
    let message = std::env::args().nth(1);
    let (conn, rx) = Connection::start_with_channel(Config::cloud(account), None);
    let mut sent = false;
    let mut state = SessionState::default();
    while let Ok(o) = rx.recv_timeout(Duration::from_secs(120)) {
        println!("{o:?}");
        state.apply(o);
        if state.session_id.is_some() && !sent {
            sent = true;
            match &message {
                Some(m) => {
                    conn.send(|id| Request::Message { id, content: m.clone() });
                }
                None => break,
            }
        } else if sent && !state.is_processing && state.transcript.len() >= 2 {
            break;
        }
    }
    println!("--- {} entries, last: {:?}", state.transcript.len(), state.transcript.last().map(|e| &e.text));
    conn.stop();
    std::thread::sleep(Duration::from_millis(500));
}
