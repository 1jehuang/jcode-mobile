//! Live probe: which events does attach push unprompted? Names and sizes only.
use jcode_mobile_core::{AccountStore, cloud};
use std::time::{Duration, Instant};

fn main() {
    let account = AccountStore::default_location().load().expect("signed-in account");
    let client = cloud::connect(&account, &|_| {}, &|| false).expect("connect");
    let sid = client.list_sessions_limited(3).expect("list")[1].session_id.clone();
    let c = cloud::connect(&account, &|_| {}, &|| false).expect("connect");
    let stream = c.events(None);
    let t = Instant::now();
    c.attach_session(&sid).expect("attach");
    println!("attach {:?}", t.elapsed());
    while t.elapsed() < Duration::from_secs(5) {
        if let Some(ev) = stream.next_timeout(Duration::from_millis(100)) {
            let v = serde_json::to_value(&ev).unwrap();
            let len = serde_json::to_vec(&v).unwrap().len();
            println!("{:?} {} {}B", t.elapsed(), v["ev"].as_str().unwrap_or("?"), len);
        }
    }
}
