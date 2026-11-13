//! Privacy-safe, bounded, durable mobile diagnostics. Never uploads log lines.
//!
//! Producers only enqueue allowlisted labels and numbers. Disk and HTTP work
//! happen on one background thread. Failed uploads retain the same batch ID.
use crate::cloud::{Account, AccountStore, CLIENT_NAME};
use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    sync::{
        OnceLock,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const EVENTS_PER_BATCH: usize = 64;
const MAX_BATCHES: usize = 32;
const MAX_FILE_BYTES: u64 = 512 * 1024;
const MAX_AGE_MS: u64 = 7 * 24 * 60 * 60 * 1000;
const FLUSH_INTERVAL: Duration = Duration::from_secs(10);
static SINK: OnceLock<mpsc::SyncSender<Event>> = OnceLock::new();
static SEQUENCE: AtomicU64 = AtomicU64::new(0);
static DROPPED: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Event {
    seq: u64,
    time_unix_ms: u64,
    uptime_ms: u64,
    kind: String,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    duration_ms: Option<f64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Batch {
    schema_version: u8,
    batch_id: String,
    run_id: String,
    build: String,
    app_version: String,
    os: String,
    events: Vec<Event>,
}

#[derive(Default, Serialize, Deserialize)]
struct Queue {
    // Local only, never transmitted. Prevents replaying one account's data as another.
    owner: Option<u64>,
    batches: VecDeque<Batch>,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Starts once, before UI initialization. Demo/test applications do not start it.
/// Explicit opt-out is available for local development and diagnostic isolation.
pub fn start() {
    if std::env::var_os("JCODE_MOBILE_DEMO").is_some()
        || std::env::var_os("JCODE_VOICE_DEMO").is_some()
        || std::env::var_os("JCODE_MOBILE_DIAGNOSTICS_OFF").is_some()
    {
        return;
    }
    let (tx, rx) = mpsc::sync_channel(512);
    if SINK.set(tx).is_err() {
        return;
    }
    let path = crate::crash::default_path().with_file_name("diagnostics-queue.json");
    if let Err(e) = std::thread::Builder::new()
        .name("mobile-diagnostics".into())
        .spawn(move || run(path, rx))
    {
        log::warn!("mobile diagnostics worker unavailable: {e}");
    }
    record("app diagnostics started", None);
}

/// Accept only known instrumentation labels, never arbitrary messages or errors.
fn safe_name(name: &str) -> Option<String> {
    const NAMES: &[&str] = &[
        "app diagnostics started",
        "app launched",
        "app fonts loaded",
        "app first render",
        "app prewarm",
        "app foreground",
        "app background",
        "app memory warning",
        "direct ssh",
        "direct ssh failed",
        "control plane wake + direct ssh",
        "control plane ready",
        "prewarm direct ssh",
        "master revalidated",
        "master revalidation failed",
        "ssh handshake",
        "ssh authenticated",
        "channel opened",
        "api hello",
        "session connect",
        "session fork",
        "session attach",
        "session ready",
        "session lost after",
        "session fatal",
        "session connect failed",
        "session connect error",
        "session retry",
        "session history",
        "wake probe failed",
        "wake probe timed out",
        "runtime_info not pushed",
        "session runtime_info pushed",
        "stream closed",
        "ui_history_apply",
        "ui_open_to_history",
        "ui slow frame build",
        "ui open to transcript render",
        "ui transcript render",
        "diagnostics events dropped",
        "connection wake",
        "connection start",
        "connection stop",
        "connection stop requested",
        "connection stopped",
        "wake probe succeeded",
        "ios foreground",
        "ios background",
        "ios memory warning",
        "connection error connect failed",
        "connection error handshake failed",
        "connection error timeout",
        "connection error disconnected",
        "connection error unexpected reply",
        "connection error harness",
        "connection error transport",
        "connection error launch failed",
        "connection error jcode not found",
        "connection error startup failed",
        "connection error startup timeout",
        "connection error invalid instance home",
        "connection error invalid option",
        "connection error unsupported transport",
        "connection error event buffer overflow",
        "connection error cancelled",
        "connection error cloud auth",
        "connection error cloud not entitled",
        "connection error host key mismatch",
    ];
    const REQUESTS: &[&str] = &[
        "message",
        "cancel",
        "soft_interrupt",
        "cancel_soft_interrupts",
        "ping",
        "history",
        "set_model",
        "set_reasoning_effort",
        "compact",
        "rename",
        "clear",
        "list_sessions",
    ];
    let allowed = NAMES.contains(&name)
        || name
            .strip_prefix("initial ")
            .is_some_and(|n| ["history", "models", "sessions", "other"].contains(&n))
        || name
            .strip_prefix("request ")
            .is_some_and(|n| REQUESTS.contains(&n));
    allowed.then(|| name.replace(" + ", "_").replace(' ', "_"))
}

/// Called by timing instrumentation. No filesystem, network, or blocking send.
pub(crate) fn record(name: &str, duration: Option<Duration>) {
    let Some(sink) = SINK.get() else { return };
    let Some(name) = safe_name(name) else { return };
    let event = Event {
        seq: SEQUENCE.fetch_add(1, Ordering::Relaxed),
        time_unix_ms: now_ms(),
        uptime_ms: (crate::timing::process_start().elapsed().as_millis() as u64)
            .min(30 * 24 * 60 * 60 * 1000),
        kind: if duration.is_some() {
            "timing"
        } else {
            "event"
        }
        .into(),
        name,
        duration_ms: duration.map(|d| d.as_secs_f64().mul_add(1000.0, 0.0).min(MAX_AGE_MS as f64)),
    };
    if sink.try_send(event).is_err() {
        DROPPED.fetch_add(1, Ordering::Relaxed);
    }
}

fn owner(account: &Account) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    account.api_base.hash(&mut h);
    account.api_key.hash(&mut h);
    h.finish()
}

impl Queue {
    fn push_events(
        &mut self,
        run_id: &str,
        build: &str,
        events: Vec<Event>,
        attempted: Option<&str>,
    ) {
        if let Some(last) = self.batches.back_mut()
            && last.run_id == run_id
            && Some(last.batch_id.as_str()) != attempted
            && last.events.len() + events.len() <= EVENTS_PER_BATCH
        {
            last.events.extend(events);
            last.events.sort_unstable_by_key(|event| event.seq);
        } else {
            self.batches.push_back(batch(run_id, build, events));
        }
    }

    fn bind(&mut self, account: Option<&Account>) {
        let next = account.map(owner);
        if self.owner != next {
            self.batches.clear();
            self.owner = next;
        }
    }

    fn prune(&mut self, now: u64) {
        self.batches.retain(|b| {
            b.events
                .first()
                .is_some_and(|e| now.saturating_sub(e.time_unix_ms) < MAX_AGE_MS)
        });
        while self.batches.len() > MAX_BATCHES {
            self.batches.pop_front();
        }
    }

    fn load(path: &Path) -> Self {
        if std::fs::metadata(path).is_ok_and(|m| m.len() <= MAX_FILE_BYTES) {
            if let Ok(bytes) = std::fs::read(path) {
                if let Ok(mut queue) = serde_json::from_slice::<Self>(&bytes) {
                    queue.prune(now_ms());
                    return queue;
                }
            }
        }
        Self::default()
    }

    fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let bytes = serde_json::to_vec(self)?;
        if bytes.len() as u64 > MAX_FILE_BYTES {
            return Err(std::io::Error::other("diagnostic queue exceeds bound"));
        }
        let temp = path.with_extension("json.tmp");
        std::fs::write(&temp, bytes)?;
        std::fs::rename(temp, path)
    }
}

fn batch(run_id: &str, build: &str, mut events: Vec<Event>) -> Batch {
    // Sequence allocation and nonblocking send may interleave across threads.
    events.sort_unstable_by_key(|event| event.seq);
    Batch {
        schema_version: 1,
        batch_id: uuid::Uuid::new_v4().to_string(),
        run_id: run_id.into(),
        build: build.into(),
        app_version: env!("CARGO_PKG_VERSION").into(),
        os: platform_description(),
        events,
    }
}

fn platform_description() -> String {
    let base = crate::crash::os_description();
    #[cfg(target_os = "ios")]
    {
        // Product model (e.g. iPhone17,1) and OS release, never a device ID.
        fn property(name: &std::ffi::CStr) -> Option<String> {
            unsafe extern "C" {
                fn sysctlbyname(
                    name: *const std::ffi::c_char,
                    old: *mut std::ffi::c_void,
                    len: *mut usize,
                    new: *mut std::ffi::c_void,
                    new_len: usize,
                ) -> std::ffi::c_int;
            }
            let mut bytes = [0u8; 64];
            let mut len = bytes.len();
            // SAFETY: name is NUL-terminated, bytes has len writable bytes.
            // A null new pointer makes this a read-only query.
            if unsafe {
                sysctlbyname(
                    name.as_ptr(),
                    bytes.as_mut_ptr().cast(),
                    &mut len,
                    std::ptr::null_mut(),
                    0,
                )
            } != 0
            {
                return None;
            }
            let value = std::ffi::CStr::from_bytes_until_nul(&bytes)
                .ok()?
                .to_str()
                .ok()?;
            Some(value.to_string())
        }
        return describe_apple_platform(
            &base,
            property(c"kern.osproductversion").as_deref(),
            property(c"hw.machine").as_deref(),
        );
    }
    #[cfg(not(target_os = "ios"))]
    base
}

#[cfg(any(target_os = "ios", test))]
fn describe_apple_platform(base: &str, version: Option<&str>, model: Option<&str>) -> String {
    let mut out = base.to_string();
    let numeric = |s: &str| !s.is_empty() && s.len() <= 3 && s.bytes().all(|b| b.is_ascii_digit());
    if let Some(version) =
        version.filter(|s| s.split('.').count() <= 3 && s.split('.').all(numeric))
    {
        out.push(' ');
        out.push_str(version);
        if let Some(model) = model.filter(|s| {
            s.strip_prefix("iPhone")
                .or_else(|| s.strip_prefix("iPad"))
                .and_then(|n| n.split_once(','))
                .is_some_and(|(major, minor)| numeric(major) && numeric(minor))
        }) {
            out.push(' ');
            out.push_str(model);
        }
    }
    out
}

fn run(path: PathBuf, rx: mpsc::Receiver<Event>) {
    let run_id = uuid::Uuid::new_v4().to_string();
    let build = crate::crash::bundle_version().unwrap_or_else(|| "dev".into());
    let mut queue = Queue::load(&path);
    let mut account = AccountStore::default_location().load();
    queue.bind(account.as_ref());
    let mut pending = Vec::new();
    let mut next_upload = Instant::now() + FLUSH_INTERVAL;
    let mut retry = FLUSH_INTERVAL;
    let mut attempted: Option<String> = None;
    let mut dirty = true;
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(10)))
        .http_status_as_error(false)
        .user_agent(CLIENT_NAME)
        .build()
        .into();
    loop {
        // Coalesce for up to one second, bounded even under noisy rendering.
        let deadline = Instant::now() + Duration::from_secs(1);
        while pending.len() < EVENTS_PER_BATCH {
            match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok(event) => pending.push(event),
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
                Err(mpsc::RecvTimeoutError::Timeout) => break,
            }
        }
        let current = AccountStore::default_location().load();
        if account.as_ref().map(owner) != current.as_ref().map(owner) {
            queue.bind(current.as_ref());
            pending.clear();
            account = current;
            dirty = true;
        }
        if !pending.is_empty() {
            if account.is_some() {
                queue.push_events(
                    &run_id,
                    &build,
                    std::mem::take(&mut pending),
                    attempted.as_deref(),
                );
                dirty = true;
            } else {
                pending.clear();
            }
        }
        let before = queue.batches.len();
        queue.prune(now_ms());
        dirty |= before != queue.batches.len();
        // Persist BEFORE HTTP so termination during an upload is safely retried.
        if dirty {
            if queue.save(&path).is_err() {
                continue;
            }
            dirty = false;
        }
        if Instant::now() < next_upload {
            continue;
        }
        next_upload = Instant::now() + FLUSH_INTERVAL;
        let Some(account) = account.as_ref() else {
            continue;
        };
        let Some(front) = queue.batches.front() else {
            continue;
        };
        attempted = Some(front.batch_id.clone());
        match upload(&agent, account, front) {
            Upload::Accepted | Upload::Rejected => {
                queue.batches.pop_front();
                dirty = queue.save(&path).is_err();
                retry = FLUSH_INTERVAL;
                // Drain backlog promptly, but at most one batch per second.
                next_upload = Instant::now()
                    + if queue.batches.is_empty() {
                        FLUSH_INTERVAL
                    } else {
                        Duration::from_secs(1)
                    };
            }
            Upload::Retry => {
                next_upload = Instant::now() + retry;
                retry = (retry * 2).min(Duration::from_secs(300));
            }
        }
        let dropped = DROPPED.swap(0, Ordering::Relaxed);
        if dropped > 0 {
            record("diagnostics events dropped", None);
        }
    }
}

#[derive(Debug, PartialEq)]
enum Upload {
    Accepted,
    Rejected,
    Retry,
}
fn upload(agent: &ureq::Agent, account: &Account, batch: &Batch) -> Upload {
    let url = format!(
        "{}/mobile/diagnostics",
        account.api_base.trim_end_matches('/')
    );
    let result = agent
        .post(&url)
        .header("Authorization", &format!("Bearer {}", account.api_key))
        .send_json(batch);
    match result.map(|r| r.status().as_u16()) {
        Ok(200..=299) => Upload::Accepted,
        // Malformed batches cannot recover. Auth, rollout and rate-limit errors can.
        Ok(400 | 409 | 413 | 415 | 422) => Upload::Rejected,
        _ => Upload::Retry,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_known_labels_can_leave_device() {
        assert_eq!(
            safe_name("session connect").as_deref(),
            Some("session_connect")
        );
        assert_eq!(
            safe_name("control plane wake + direct ssh").as_deref(),
            Some("control_plane_wake_direct_ssh")
        );
        assert!(safe_name("request message").is_some());
        for secret in [
            "Bearer secret",
            "request my private prompt",
            "https://host",
            "session secret-id",
        ] {
            assert!(safe_name(secret).is_none());
        }
    }

    fn event(time: u64) -> Event {
        Event {
            seq: 0,
            time_unix_ms: time,
            uptime_ms: 1,
            kind: "event".into(),
            name: "session_ready".into(),
            duration_ms: None,
        }
    }

    #[test]
    fn queue_bounds_retention_and_roundtrips() {
        let mut queue = Queue::default();
        queue.batches.push_back(batch("run", "dev", vec![event(1)]));
        for _ in 0..MAX_BATCHES + 5 {
            queue
                .batches
                .push_back(batch("run", "dev", vec![event(MAX_AGE_MS + 10)]));
        }
        queue.prune(MAX_AGE_MS + 10);
        assert_eq!(queue.batches.len(), MAX_BATCHES);
        assert!(queue.batches.iter().all(|b| b.events[0].time_unix_ms > 1));
        let path =
            std::env::temp_dir().join(format!("mobile-diagnostics-{}.json", uuid::Uuid::new_v4()));
        queue.save(&path).unwrap();
        let restored = Queue::load(&path);
        // Synthetic timestamps are expired relative to wall clock.
        assert!(restored.batches.is_empty());
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn batches_retry_with_stable_identity_and_no_sensitive_fields() {
        let b = batch("run", "dev", vec![event(now_ms())]);
        let json = serde_json::to_string(&b).unwrap();
        let restored: Batch = serde_json::from_str(&json).unwrap();
        assert_eq!(b.batch_id, restored.batch_id);
        assert!(!json.contains("api_key"));
        assert!(!json.contains("detail"));
        assert!(!json.contains("owner"));
    }

    #[test]
    fn concurrent_sequence_order_and_maximum_payload_fit_backend() {
        let events = (0..EVENTS_PER_BATCH)
            .rev()
            .map(|seq| {
                let mut e = event(now_ms());
                e.seq = seq as u64;
                e.name = "connection_error_invalid_instance_home".into();
                e.kind = "timing".into();
                e.duration_ms = Some(MAX_AGE_MS as f64);
                e
            })
            .collect();
        let b = batch(&uuid::Uuid::new_v4().to_string(), "1794611720", events);
        assert!(b.events.windows(2).all(|pair| pair[0].seq < pair[1].seq));
        assert!(serde_json::to_vec(&b).unwrap().len() <= 32 * 1024);
        let mut queue = Queue::default();
        let mut later = event(now_ms());
        later.seq = 2;
        queue.push_events("run", "dev", vec![later], None);
        queue.push_events("run", "dev", vec![event(now_ms())], None);
        assert_eq!(queue.batches[0].events[0].seq, 0);
        assert_eq!(queue.batches[0].events[1].seq, 2);
    }

    #[test]
    fn every_connection_error_classification_is_uploadable() {
        let production = include_str!("connection.rs")
            .split("#[cfg(test)]")
            .next()
            .unwrap();
        for line in production.lines() {
            if let Some(label) = line.split('"').nth(1)
                && label.starts_with("connection error ")
            {
                assert!(
                    safe_name(label).is_some(),
                    "missing allowlist entry: {label}"
                );
            }
        }
        for label in [
            "ios foreground",
            "ios background",
            "ios memory warning",
            "connection stopped",
        ] {
            assert!(safe_name(label).is_some());
        }
    }

    #[test]
    fn apple_metadata_handles_missing_properties_and_simulator_models() {
        assert_eq!(
            describe_apple_platform("ios aarch64", Some("26.1"), Some("iPhone17,1")),
            "ios aarch64 26.1 iPhone17,1"
        );
        assert_eq!(
            describe_apple_platform("ios aarch64", None, Some("iPhone17,1")),
            "ios aarch64"
        );
        assert_eq!(
            describe_apple_platform("ios aarch64", Some("26.1"), None),
            "ios aarch64 26.1"
        );
        assert_eq!(
            describe_apple_platform("ios aarch64", Some("26.1"), Some("arm64")),
            "ios aarch64 26.1"
        );
        assert_eq!(
            describe_apple_platform("ios aarch64", Some("private text"), Some("iPhone17,1")),
            "ios aarch64"
        );
        assert_eq!(
            describe_apple_platform("ios aarch64", Some("26.1"), Some("private text")),
            "ios aarch64 26.1"
        );
    }

    #[test]
    fn immutable_attempted_batches_and_account_isolation() {
        let a = Account {
            api_key: "key-a".into(),
            email: None,
            api_base: "https://example.test/v1".into(),
        };
        let b = Account {
            api_key: "key-b".into(),
            ..a.clone()
        };
        let mut queue = Queue::default();
        queue.bind(Some(&a));
        queue.push_events("run", "dev", vec![event(now_ms())], None);
        queue.push_events("run", "dev", vec![event(now_ms())], None);
        assert_eq!(queue.batches.len(), 1);
        assert_eq!(queue.batches[0].events.len(), 2);
        let attempted = queue.batches[0].batch_id.clone();
        queue.push_events("run", "dev", vec![event(now_ms())], Some(&attempted));
        assert_eq!(queue.batches.len(), 2);
        assert_eq!(queue.batches[0].events.len(), 2);
        queue.bind(Some(&a));
        assert_eq!(queue.batches.len(), 2);
        queue.bind(Some(&b));
        assert!(queue.batches.is_empty());
        queue.push_events("run", "dev", vec![event(now_ms())], None);
        queue.bind(None);
        assert!(queue.batches.is_empty());
    }

    #[test]
    fn persisted_batches_survive_restart_with_identical_ids() {
        let path =
            std::env::temp_dir().join(format!("mobile-diagnostics-{}.json", uuid::Uuid::new_v4()));
        let mut queue = Queue::default();
        queue
            .batches
            .push_back(batch("run", "dev", vec![event(now_ms())]));
        queue.save(&path).unwrap();
        let restored = Queue::load(&path);
        assert_eq!(restored.batches[0].batch_id, queue.batches[0].batch_id);
        assert_eq!(restored.batches[0].events.len(), 1);
        std::fs::write(&path, b"broken JSON").unwrap();
        assert!(Queue::load(&path).batches.is_empty());
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn upload_contract_and_status_handling() {
        use std::io::{BufRead, Read, Write};
        for (status, expected) in [
            (200, Upload::Accepted),
            (400, Upload::Rejected),
            (409, Upload::Rejected),
            (413, Upload::Rejected),
            (415, Upload::Rejected),
            (401, Upload::Retry),
            (404, Upload::Retry),
            (429, Upload::Retry),
            (500, Upload::Retry),
        ] {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let account = Account {
                api_key: "test-secret".into(),
                email: None,
                api_base: format!("http://{}/v1", listener.local_addr().unwrap()),
            };
            let server = std::thread::spawn(move || {
                let (stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut reader = std::io::BufReader::new(stream);
                let mut headers = String::new();
                let mut length = 0;
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" || line.is_empty() {
                        break;
                    }
                    if let Some(n) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        length = n.trim().parse::<usize>().unwrap();
                    }
                    headers.push_str(&line);
                }
                assert!(headers.starts_with("POST /v1/mobile/diagnostics HTTP/1.1"));
                assert!(
                    headers
                        .to_ascii_lowercase()
                        .contains("authorization: bearer test-secret")
                );
                let mut body = vec![0; length];
                reader.read_exact(&mut body).unwrap();
                let body = String::from_utf8(body).unwrap();
                assert!(!body.contains("test-secret"));
                let json: serde_json::Value = serde_json::from_str(&body).unwrap();
                assert_eq!(json["events"][0]["name"], "session_ready");
                assert_eq!(json["schema_version"], 1);
                write!(
                    reader.get_mut(),
                    "HTTP/1.1 {status} Test\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}"
                )
                .unwrap();
            });
            let agent: ureq::Agent = ureq::Agent::config_builder()
                .http_status_as_error(false)
                .timeout_global(Some(Duration::from_secs(5)))
                .build()
                .into();
            assert_eq!(
                upload(
                    &agent,
                    &account,
                    &batch("run", "dev", vec![event(now_ms())])
                ),
                expected
            );
            server.join().unwrap();
        }
    }
}
