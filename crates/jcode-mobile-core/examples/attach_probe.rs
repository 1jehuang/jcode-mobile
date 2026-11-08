//! Live probe for attaching to an already running session. Connection A
//! creates a session and starts a long tool-using turn. After a delay,
//! connection B attaches to the same session. Every event B sees is logged,
//! and B's reducer transcript is compared to a fresh reattach after the turn.
//! Usage: attach_probe [attach_delay_ms]
use jcode_mobile_core::{
    AccountStore,
    connection::{Config, Connection, Request},
    session::{ConnectionOutput, LocalIntent, SessionState},
};
use std::time::{Duration, Instant};

fn dump(label: &str, s: &SessionState) {
    println!("=== {label}: processing={} reasoning={} entries={}", s.is_processing, s.is_reasoning, s.transcript.len());
    for e in &s.transcript {
        let t: String = e.text.chars().take(70).collect::<String>().replace('\n', "\\n");
        let r: String = e.reasoning.chars().take(40).collect::<String>().replace('\n', "\\n");
        let tools: Vec<_> = e.tool_calls.iter().map(|t| format!("{}:{:?}", t.name, t.status)).collect();
        println!("  {:?} stream={} queued={} text={t:?} reasoning={r:?} tools={tools:?}", e.role, e.is_streaming, e.is_queued);
    }
}

fn short(o: &ConnectionOutput) -> String {
    let s = format!("{o:?}");
    if s.contains("History {") || s.contains("Attached {") { s } else { s.chars().take(200).collect() }
}

fn main() {
    let delay: u64 = std::env::args().nth(1).and_then(|a| a.parse().ok()).unwrap_or(8000);
    let account = AccountStore::default_location().load().expect("no account");
    let (a, arx) = Connection::start_with_channel(Config::cloud(account.clone()), None);
    let mut sa = SessionState::default();
    let start = Instant::now();
    // Wait for A to be ready (history arrived).
    loop {
        let o = arx.recv_timeout(Duration::from_secs(60)).expect("A timeout");
        let hist = matches!(&o, ConnectionOutput::Event(jcode_sdk::ApiEvent::History { .. }));
        sa.apply(o);
        if hist { break; }
    }
    let sid = sa.session_id.clone().unwrap();
    println!("A ready sid={sid} at {:?}", start.elapsed());
    let prompt_tools = "First say one short sentence. Then run the bash command `for i in 1 2 3 4 5 6; do echo tick $i; sleep 2; done`. Then run `echo second-tool`. Then write a 3 paragraph answer about rivers.";
    sa.apply_intent(LocalIntent::UserSentMessage("warmup: reply with just OK".into()));
    a.send(|id| Request::Message { id, content: "warmup: reply with just OK".into() });
    // Let warmup turn finish so history has prior content.
    let mut seen_running = false;
    loop {
        let o = arx.recv_timeout(Duration::from_secs(120)).expect("A warmup");
        sa.apply(o);
        if sa.is_processing { seen_running = true; }
        if seen_running && !sa.is_processing { break; }
    }
    let prompt = if std::env::args().nth(2).as_deref() == Some("text") { "Write 8 long paragraphs about rivers. No tools." } else { prompt_tools };
    sa.apply_intent(LocalIntent::UserSentMessage(prompt.into()));
    a.send(|id| Request::Message { id, content: prompt.into() });
    let t0 = Instant::now();
    let a_thread = std::thread::spawn(move || {
        let mut seen = false;
        while let Ok(o) = arx.recv_timeout(Duration::from_secs(180)) {
            sa.apply(o);
            if sa.is_processing { seen = true; }
            if seen && !sa.is_processing { break; }
        }
        sa
    });
    std::thread::sleep(Duration::from_millis(delay));
    println!("--- B attaching at +{:?} into turn", t0.elapsed());
    let (b, brx) = Connection::start_with_channel(Config::cloud(account.clone()), Some(sid.clone()));
    let mut sb = SessionState::default();
    let mut seen_running = false;
    let mut idle_after = 0;
    while let Ok(o) = brx.recv_timeout(Duration::from_secs(180)) {
        println!("B +{:?} {}", t0.elapsed(), short(&o));
        let is_hist = matches!(&o, ConnectionOutput::Event(jcode_sdk::ApiEvent::History { .. }));
        sb.apply(o);
        if is_hist { dump("B after history", &sb); }
        if sb.is_processing { seen_running = true; }
        if seen_running && !sb.is_processing {
            idle_after += 1;
            if idle_after > 0 { break; }
        }
    }
    // Catch the post-turn resync.
    while let Ok(o) = brx.recv_timeout(Duration::from_secs(4)) {
        println!("B(after) +{:?} {}", t0.elapsed(), short(&o));
        sb.apply(o);
    }
    dump("B final", &sb);
    let sa = a_thread.join().unwrap();
    dump("A final", &sa);
    b.stop();
    a.stop();
    // Fresh reattach after completion.
    let (c, crx) = Connection::start_with_channel(Config::cloud(account), Some(sid));
    let mut sc = SessionState::default();
    while let Ok(o) = crx.recv_timeout(Duration::from_secs(60)) {
        let h = matches!(&o, ConnectionOutput::Event(jcode_sdk::ApiEvent::History { .. }));
        sc.apply(o);
        if h { break; }
    }
    std::thread::sleep(Duration::from_secs(1));
    while let Ok(o) = crx.try_recv() { sc.apply(o); }
    dump("C fresh reattach", &sc);
    c.stop();
    let texts = |s: &SessionState| s.transcript.iter().map(|e| (e.role, e.text.clone())).collect::<Vec<_>>();
    println!("RESULT B==C texts: {}", texts(&sb) == texts(&sc));
}
