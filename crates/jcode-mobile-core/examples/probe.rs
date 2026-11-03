//! Dev probe: connect with the first saved credential, optionally send a
//! message, and print every decoded event. `cargo run --example probe -- "hi"`.

use jcode_mobile_core::{Config, Connection, CredentialStore, FileCredentialStore, Request};
use std::time::{Duration, Instant};

fn main() {
    let cred = FileCredentialStore::default_location()
        .load_all()
        .into_iter()
        .next()
        .expect("no paired server; pair with the app first");
    let mut config = Config::new(cred.gateway(), cred.token);
    config.working_dir = cred.working_dir;
    let (conn, rx) = Connection::start_with_channel(config, None);
    let msg = std::env::args().nth(1);
    let mut sent = false;
    let deadline = Instant::now() + Duration::from_secs(60);
    while Instant::now() < deadline {
        let Ok(o) = rx.recv_timeout(Duration::from_millis(500)) else { continue };
        println!("{o:?}");
        if !sent
            && let Some(m) = &msg
            && matches!(o, jcode_mobile_core::ConnectionOutput::Phase(jcode_mobile_core::ConnectionPhase::Connected))
        {
            sent = true;
            let m = m.clone();
            conn.send(|id| Request::Message { id, content: m });
        }
        if format!("{o:?}").contains("Done") && sent && format!("{o:?}").contains("MessageEnd") {
            break;
        }
    }
    conn.stop();
}
