//! Live probe of opening existing sessions the way the app does: connect,
//! attach, then the initial snapshot requests in parallel. Prints sizes and
//! timings only, never content or addresses.
use jcode_mobile_core::{AccountStore, cloud};
use jcode_sdk::ApiRequest;
use std::time::Instant;

fn main() {
    let account = AccountStore::default_location().load().expect("signed-in account");
    let client = cloud::connect(&account, &|_| {}, &|| false).expect("connect");
    let t = Instant::now();
    let sessions = client.list_sessions_limited(80).expect("list");
    println!("list_sessions n={} {:?}", sessions.len(), t.elapsed());
    for info in sessions.iter().take(8) {
        let sid = info.session_id.clone();
        let started = Instant::now();
        let c = cloud::connect(&account, &|_| {}, &|| false).expect("connect");
        let connect = started.elapsed();
        let a = Instant::now();
        let _ = c.attach_session(&sid).expect("attach");
        let attach = a.elapsed();
        let reqs: Vec<(&str, ApiRequest)> = vec![
            ("history", ApiRequest::GetHistory { session_id: sid.clone() }),
            ("runtime", ApiRequest::GetRuntimeInfo { session_id: sid.clone() }),
            ("models", ApiRequest::ListModels { session_id: sid.clone() }),
            ("sessions", ApiRequest::ListSessions { include_archived: false, limit: Some(80) }),
        ];
        if std::env::var_os("HISTORY_FIRST").is_some() {
            let s = Instant::now();
            let r = c.request(ApiRequest::GetHistory { session_id: sid.clone() });
            println!("  history alone={:?} ok={}", s.elapsed(), r.is_ok());
        }
        let handles: Vec<_> = reqs
            .into_iter()
            .map(|(name, req)| {
                let c = c.clone();
                std::thread::spawn(move || {
                    let s = Instant::now();
                    let r = c.request(req);
                    let bytes = r.as_ref().map(|f| serde_json::to_vec(&f.event).map(|v| v.len()).unwrap_or(0)).unwrap_or(0);
                    (name, s.elapsed(), bytes, r.is_ok())
                })
            })
            .collect();
        let mut parts = vec![];
        for h in handles {
            let (name, d, bytes, ok) = h.join().unwrap();
            parts.push(format!("{name}={d:?}/{}KB{}", bytes / 1024, if ok { "" } else { "(err)" }));
        }
        println!(
            "session status={} connect={connect:?} attach={attach:?} {} total={:?}",
            info.status,
            parts.join(" "),
            started.elapsed()
        );
    }
}
