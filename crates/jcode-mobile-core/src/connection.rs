//! One connection to a jcode harness, run on a background thread.
//!
//! Speaks the harness API through `jcode_sdk::JcodeClient`: connect (Jcode
//! Cloud over in-process SSH, or any custom transport), attach or create a
//! session, load history, stream `ApiEvent`s, and reconnect with capped
//! exponential backoff. Sign-in and subscription failures stop for good.
//!
//! Threads instead of async keep the core runtime-free, which matters for the
//! iOS target where GPUI owns the main loop.

use crate::session::{ConnectionOutput, ConnectionPhase};
use jcode_sdk::{ApiEvent, ConnectOptions, ErrorKind, JcodeClient, Transport};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Opens a fresh transport for each (re)connect attempt.
pub type TransportFactory = Arc<dyn Fn() -> jcode_sdk::Result<Box<dyn Transport>> + Send + Sync>;

/// Where a connection goes.
#[derive(Clone)]
pub enum Endpoint {
    /// The user's Jcode Cloud host, woken on demand, over in-process SSH.
    Cloud(crate::cloud::Account),
    /// Any transport carrying harness API frames (tests, future gateways).
    Custom(TransportFactory),
}

impl std::fmt::Debug for Endpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Endpoint::Cloud(a) => f.debug_tuple("Cloud").field(&a.email).finish(),
            Endpoint::Custom(_) => f.write_str("Custom"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub endpoint: Endpoint,
    pub max_reconnect_attempts: Option<u32>,
    pub base_backoff: Duration,
    pub working_dir: Option<String>,
    /// Create an independent fork of this session on the first connection.
    /// Once created, reconnects attach to the fork rather than forking again.
    pub fork_from: Option<String>,
}

impl Config {
    pub fn cloud(account: crate::cloud::Account) -> Self {
        Self::with_endpoint(Endpoint::Cloud(account))
    }
    pub fn custom(factory: TransportFactory) -> Self {
        Self::with_endpoint(Endpoint::Custom(factory))
    }
    fn with_endpoint(endpoint: Endpoint) -> Self {
        Self {
            endpoint,
            max_reconnect_attempts: None,
            base_backoff: Duration::from_secs(1),
            working_dir: None,
            fork_from: None,
        }
    }
}

/// What the UI can ask of the attached session. `id` is a local
/// correlation id handed out by [`Connection::send`].
#[derive(Debug, Clone, PartialEq)]
pub enum Request {
    Message { id: u64, content: String },
    Cancel { id: u64 },
    SoftInterrupt { id: u64, content: String, urgent: bool },
    CancelSoftInterrupts { id: u64 },
    Ping { id: u64 },
    GetHistory { id: u64 },
    SetModel { id: u64, model: String },
    SetReasoningEffort { id: u64, effort: String },
    Compact { id: u64 },
    RenameSession { id: u64, title: Option<String> },
    Clear { id: u64 },
    /// Refresh the account's session list (answered with `ApiEvent::Sessions`).
    ListSessions { id: u64 },
}

enum Command {
    Send(Request),
    Stop,
}

/// Cheap, cloneable handle used by the UI to send requests.
#[derive(Clone)]
pub struct Connection {
    commands: Sender<Command>,
    next_id: Arc<Mutex<u64>>,
}

impl Connection {
    /// Starts the connection thread. `on_output` is called from that thread
    /// for each phase change and event (use it to wake the UI).
    pub fn start(
        config: Config,
        resume_session_id: Option<String>,
        on_output: impl Fn(ConnectionOutput) + Send + 'static,
    ) -> Connection {
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("jcode-connection".into())
            .spawn(move || run(config, resume_session_id, rx, on_output))
            .expect("spawn connection thread");
        Connection { commands: tx, next_id: Arc::new(Mutex::new(1)) }
    }

    /// Like `start` but returns a channel instead of a callback.
    pub fn start_with_channel(
        config: Config,
        resume_session_id: Option<String>,
    ) -> (Connection, Receiver<ConnectionOutput>) {
        let (tx, rx) = mpsc::channel();
        let c = Self::start(config, resume_session_id, move |o| {
            let _ = tx.send(o);
        });
        (c, rx)
    }

    /// Queues a request with a fresh id. Returns the id.
    pub fn send(&self, build: impl FnOnce(u64) -> Request) -> u64 {
        let id = {
            let mut g = self.next_id.lock().unwrap();
            *g += 1;
            *g - 1
        };
        let _ = self.commands.send(Command::Send(build(id)));
        id
    }

    pub fn stop(&self) {
        let _ = self.commands.send(Command::Stop);
    }
}

const CLIENT_NAME: &str = concat!("jcode-ios/", env!("CARGO_PKG_VERSION"));

enum Failure {
    /// Retrying cannot help (sign-in expired, no subscription, bad host key).
    Fatal(String),
    Retry(String),
    Stopped,
}

fn classify(e: jcode_sdk::Error) -> Failure {
    match e.kind {
        ErrorKind::Cancelled => Failure::Stopped,
        ErrorKind::CloudAuth | ErrorKind::CloudNotEntitled | ErrorKind::HostKeyMismatch => {
            Failure::Fatal(e.message)
        }
        _ => Failure::Retry(e.message),
    }
}

/// Commands that arrive before the session is live are kept, not dropped.
struct Inbox {
    rx: Receiver<Command>,
    queued: Vec<Request>,
    stopped: bool,
}

impl Inbox {
    fn drain(&mut self) {
        loop {
            match self.rx.try_recv() {
                Ok(Command::Send(r)) => self.queued.push(r),
                Ok(Command::Stop) | Err(TryRecvError::Disconnected) => {
                    self.stopped = true;
                    return;
                }
                Err(TryRecvError::Empty) => return,
            }
        }
    }
    /// Sleep up to `d`, waking early on stop. Returns false when stopped.
    fn sleep(&mut self, d: Duration) -> bool {
        let until = std::time::Instant::now() + d;
        while let Some(left) = until.checked_duration_since(std::time::Instant::now()) {
            match self.rx.recv_timeout(left) {
                Ok(Command::Send(r)) => self.queued.push(r),
                Ok(Command::Stop) | Err(RecvTimeoutError::Disconnected) => {
                    self.stopped = true;
                    return false;
                }
                Err(RecvTimeoutError::Timeout) => break,
            }
        }
        !self.stopped
    }
}

fn open(config: &Config, inbox: &Mutex<Inbox>, emit: &dyn Fn(ConnectionOutput)) -> jcode_sdk::Result<JcodeClient> {
    match &config.endpoint {
        Endpoint::Cloud(account) => {
            let progress =
                |m: &str| emit(ConnectionOutput::Phase(ConnectionPhase::Provisioning { message: m.into() }));
            let cancelled = || {
                let mut i = inbox.lock().unwrap();
                i.drain();
                i.stopped
            };
            crate::cloud::connect(account, &progress, &cancelled)
        }
        Endpoint::Custom(factory) => {
            let options = ConnectOptions {
                client_name: CLIENT_NAME.into(),
                ensure_runtime: false,
                ..Default::default()
            };
            JcodeClient::connect_with(factory()?, options)
        }
    }
}

/// Newest sessions fetched for the sidebar.
const SESSION_LIST_LIMIT: u32 = 80;

/// Attach the session to resume, or start a fresh one if it is gone.
fn attach(client: &JcodeClient, resume: Option<&str>, working_dir: Option<String>) -> jcode_sdk::Result<jcode_sdk::SessionInfo> {
    if let Some(id) = resume {
        match client.attach_session(id) {
            Ok(s) => return Ok(s),
            Err(e) if matches!(e.kind, ErrorKind::Harness(_)) => {}
            Err(e) => return Err(e),
        }
    }
    client.create_session(working_dir)
}

fn run(
    config: Config,
    mut target_session: Option<String>,
    commands: Receiver<Command>,
    emit: impl Fn(ConnectionOutput),
) {
    let phase = |p| emit(ConnectionOutput::Phase(p));
    let mut fork_from = config.fork_from.clone();
    let inbox = Mutex::new(Inbox { rx: commands, queued: vec![], stopped: false });
    let mut attempt: u32 = 0;
    loop {
        let started = std::time::Instant::now();
        phase(if attempt == 0 { ConnectionPhase::Connecting } else { ConnectionPhase::Reconnecting { attempt } });
        let outcome = open(&config, &inbox, &emit).map_err(classify).and_then(|client| {
            crate::timing::stage("session connect", started.elapsed());
            if let Some(source) = fork_from.as_deref() {
                // Attach exactly the requested source. Never fall back to a new
                // empty session when the source no longer exists.
                client.attach_session(source).map_err(classify)?;
                let fork = client.fork_session(source).map_err(|e| {
                    // A lost response may still have created a fork. Do not
                    // replay a non-idempotent request and create duplicates.
                    Failure::Fatal(format!("Could not fork session: {}", e.message))
                })?;
                target_session = Some(fork.session_id);
                fork_from = None;
                let session = client.attach_session(target_session.as_deref().unwrap()).map_err(classify)?;
                return Ok((client, session));
            }
            let session = if config.fork_from.is_some() {
                // A fork must never silently become a blank conversation.
                client.attach_session(target_session.as_deref().unwrap()).map_err(classify)?
            } else {
                attach(&client, target_session.as_deref(), config.working_dir.clone()).map_err(classify)?
            };
            Ok((client, session))
        });
        if inbox.lock().unwrap().stopped {
            break;
        }
        let last_error: String = match outcome {
            Ok((client, session)) => {
                crate::timing::stage("session ready", started.elapsed());
                let sid = session.session_id.clone();
                target_session = Some(sid.clone());
                phase(ConnectionPhase::Connected);
                attempt = 0;
                emit(ConnectionOutput::Event(ApiEvent::HelloOk {
                    version: jcode_sdk::api::API_VERSION_MAJOR,
                    server: client.server.clone(),
                    capabilities: client.capabilities.clone(),
                }));
                let busy = session_is_busy(&session.status);
                emit(ConnectionOutput::Event(ApiEvent::Attached { session }));
                let stream = client.events(Some(&sid));
                if session_loop(&client, &sid, busy, &stream, &inbox, &emit, started) {
                    break;
                }
                "Connection lost".into()
            }
            Err(Failure::Stopped) => break,
            Err(Failure::Fatal(reason)) => {
                phase(ConnectionPhase::Failed { reason });
                return;
            }
            Err(Failure::Retry(e)) => {
                crate::timing::stage("session connect failed", started.elapsed());
                e
            }
        };
        attempt += 1;
        if let Some(max) = config.max_reconnect_attempts
            && attempt > max
        {
            let mut reason = format!("Could not reach server after {max} attempts");
            if !last_error.is_empty() {
                reason = format!("{reason}: {last_error}");
            }
            phase(ConnectionPhase::Failed { reason });
            return;
        }
        let delay = (config.base_backoff * 2u32.saturating_pow(attempt - 1)).min(Duration::from_secs(30));
        if !inbox.lock().unwrap().sleep(delay) {
            break;
        }
    }
    phase(ConnectionPhase::Disconnected);
}

/// Pump events and requests until the link drops (false) or stop (true).
/// Requests run on short-lived threads so a slow reply never stalls the
/// stream; their outcomes come back over `results`.
fn session_loop(
    client: &JcodeClient,
    sid: &str,
    attached_busy: bool,
    stream: &jcode_sdk::EventStream,
    inbox: &Mutex<Inbox>,
    emit: &dyn Fn(ConnectionOutput),
    started: std::time::Instant,
) -> bool {
    let (results, outcomes) = mpsc::channel::<ApiEvent>();
    let (snapshots, initial) = mpsc::channel();
    for req in [
        jcode_sdk::ApiRequest::GetHistory { session_id: sid.into() },
        jcode_sdk::ApiRequest::GetRuntimeInfo { session_id: sid.into() },
        jcode_sdk::ApiRequest::ListModels { session_id: sid.into() },
        jcode_sdk::ApiRequest::ListSessions {
            include_archived: false,
            limit: Some(SESSION_LIST_LIMIT),
        },
    ] {
        let (client, snapshots) = (client.clone(), snapshots.clone());
        std::thread::spawn(move || {
            if let Ok(frame) = client.request(req) {
                let _ = snapshots.send(frame.event);
            }
        });
    }
    fetch_todo_state(client, sid, &snapshots);
    drop(snapshots);
    // History replaces the reducer's transcript. Once local commands have
    // advanced it, an initial snapshot may be stale and must not rewind that
    // state. Explicit GetHistory replies still use the normal results path.
    // Live stream events do not supersede it: when attaching to a turn that is
    // already running they are the tail of a conversation the snapshot holds,
    // and the reducer merges them after the history.
    let mut history_superseded = false;
    let mut initial_history_seen = false;
    // Joined a turn already in flight. If the first live content is a delta,
    // its start was never seen and the live message is truncated, so resync
    // from history when the turn ends. Tool events carry their own identity.
    let mut joined_busy = attached_busy;
    let mut first_content_seen = false;
    let mut resync_at_turn_end = false;
    loop {
        while let Ok(event) = initial.try_recv() {
            if matches!(event, ApiEvent::History { .. }) {
                if !initial_history_seen {
                    crate::timing::stage("session history", started.elapsed());
                }
                initial_history_seen = true;
                if history_superseded {
                    continue;
                }
            }
            emit(ConnectionOutput::Event(event));
        }
        let queued = {
            let mut i = inbox.lock().unwrap();
            i.drain();
            if i.stopped {
                return true;
            }
            std::mem::take(&mut i.queued)
        };
        for request in queued {
            if matches!(
                request,
                Request::Message { .. }
                    | Request::SoftInterrupt { .. }
                    | Request::Clear { .. }
                    | Request::Compact { .. }
                    | Request::GetHistory { .. }
            ) {
                history_superseded = true;
            }
            let (client, sid, results) = (client.clone(), sid.to_string(), results.clone());
            std::thread::spawn(move || {
                if let Some(event) = execute(&client, &sid, request) {
                    let _ = results.send(event);
                }
            });
        }
        while let Ok(event) = outcomes.try_recv() {
            emit(ConnectionOutput::Event(event));
        }
        match stream.next_timeout(Duration::from_millis(50)) {
            Some(event) => {
                // A streamed History is newer than the initial snapshot.
                history_superseded |= matches!(&event, ApiEvent::History { .. });
                if is_turn_content(&event) && !std::mem::replace(&mut first_content_seen, true) {
                    // A turn this client started was seen from its start.
                    // Otherwise content that beat the initial snapshot may be
                    // duplicated by it, and a joined turn misses everything
                    // before the attach, so resync once it ends.
                    resync_at_turn_end = !history_superseded && (!initial_history_seen || joined_busy);
                }
                let turn_ended = matches!(&event, ApiEvent::TurnDone { .. } | ApiEvent::TurnStopped { .. });
                // Todo writes change the pinned board; refresh it from the
                // server's persisted state (same source as the TUI card).
                let todo_changed = matches!(&event, ApiEvent::ToolDone { name, .. } if name.eq_ignore_ascii_case("todo"));
                if todo_changed || turn_ended {
                    fetch_todo_state(client, sid, &results);
                }
                if turn_ended {
                    joined_busy = false;
                }
                emit(ConnectionOutput::Event(event));
                if turn_ended && std::mem::take(&mut resync_at_turn_end) {
                    let (client, sid, results) = (client.clone(), sid.to_string(), results.clone());
                    std::thread::spawn(move || {
                        if let Some(event) = execute(&client, &sid, Request::GetHistory { id: 0 }) {
                            let _ = results.send(event);
                        }
                    });
                }
            }
            None if client.is_closed() => return false,
            None => {}
        }
    }
}

/// Fetch the pinned todo board in the background. Older servers lack the
/// capability; their error reply is dropped rather than shown as a banner.
fn fetch_todo_state(client: &JcodeClient, sid: &str, out: &mpsc::Sender<ApiEvent>) {
    let (client, sid, out) = (client.clone(), sid.to_string(), out.clone());
    std::thread::spawn(move || {
        if let Ok(frame) = client.request(jcode_sdk::ApiRequest::GetTodoState { session_id: sid })
            && matches!(frame.event, ApiEvent::TodoState { .. })
        {
            let _ = out.send(frame.event);
        }
    });
}

/// Session statuses that mean a turn is in flight. The bridge reports
/// `processing` on attach replies and `running` on status events.
pub fn session_is_busy(status: &str) -> bool {
    matches!(status, "running" | "processing")
}

fn is_turn_content(event: &ApiEvent) -> bool {
    matches!(
        event,
        ApiEvent::TextDelta { .. }
            | ApiEvent::TextReplace { .. }
            | ApiEvent::ReasoningDelta { .. }
            | ApiEvent::ToolStart { .. }
            | ApiEvent::ToolInputDelta { .. }
            | ApiEvent::ToolCall { .. }
            | ApiEvent::ToolExec { .. }
            | ApiEvent::ToolDone { .. }
    )
}

/// Run one request. Returns an event for the reducer: the reply worth
/// showing, or an `Error` when the request failed.
fn execute(client: &JcodeClient, sid: &str, request: Request) -> Option<ApiEvent> {
    use jcode_sdk::ApiRequest as A;
    let s = || sid.to_string();
    let result = match request {
        Request::Message { content, .. } => client.send_message(sid, &content, vec![], None).map(|_| None),
        Request::SoftInterrupt { id, content, urgent } => client
            .soft_interrupt_with_id(sid, &content, vec![], urgent, Some(&format!("ios-{id}")))
            .map(|_| None),
        Request::Cancel { .. } => client.cancel(sid).map(|_| None),
        Request::CancelSoftInterrupts { .. } => client.cancel_soft_interrupts(sid).map(|_| None),
        Request::Ping { .. } => client.ping().map(|_| None),
        Request::GetHistory { .. } => client.request(A::GetHistory { session_id: s() }).map(|f| Some(f.event)),
        Request::SetModel { model, .. } => client
            .set_model(sid, &model)
            .and_then(|_| client.request(A::GetRuntimeInfo { session_id: s() }))
            .map(|f| Some(f.event)),
        Request::SetReasoningEffort { effort, .. } => client
            .set_reasoning_effort(sid, &effort)
            .and_then(|_| client.request(A::GetRuntimeInfo { session_id: s() }))
            .map(|f| Some(f.event)),
        Request::Compact { .. } => client
            .compact(sid)
            .map(|message| Some(ApiEvent::Compacted { session_id: s(), message })),
        Request::RenameSession { title, .. } => client.rename_session(sid, title).map(|_| None),
        Request::Clear { .. } => client.clear(sid).map(|_| None),
        Request::ListSessions { .. } => client
            .list_sessions_limited(SESSION_LIST_LIMIT)
            .map(|sessions| Some(ApiEvent::Sessions { sessions })),
    };
    match result {
        Ok(event) => event,
        Err(e) if e.kind == ErrorKind::Disconnected => None,
        Err(e) => Some(ApiEvent::Error { code: jcode_sdk::api::ErrorCode::Internal, message: e.message }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jcode_sdk::api::{API_VERSION_MAJOR, ServerFrame};
    use serde_json::{Value, json};
    use std::io::{BufRead, BufReader, Write};
    use std::net::{TcpListener, TcpStream};

    struct Tcp(TcpStream);
    impl Transport for Tcp {
        fn split(self: Box<Self>) -> jcode_sdk::Result<(Box<dyn BufRead + Send>, Box<dyn Write + Send>)> {
            let r = self.0.try_clone().map_err(|e| jcode_sdk::Error::new(ErrorKind::Transport, e.to_string()))?;
            Ok((Box::new(BufReader::new(r)), Box::new(self.0)))
        }
    }

    fn reply(w: &mut TcpStream, to: Option<u64>, event: Value) {
        let mut v = event;
        v["v"] = json!(API_VERSION_MAJOR);
        if let Some(id) = to {
            v["reply_to"] = json!(id);
        }
        // Round-trip through the real frame type so the fake stays honest.
        let frame: ServerFrame = serde_json::from_value(v).unwrap();
        writeln!(w, "{}", serde_json::to_string(&frame).unwrap()).unwrap();
    }

    #[test]
    fn forks_source_into_distinct_session_and_does_not_replay_failed_forks() {
        for fail in [false, true] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = listener.local_addr().unwrap();
            let (record, requests) = mpsc::channel();
            let (shutdown, socket) = mpsc::channel();
            let server = std::thread::spawn(move || {
                let (s, _) = listener.accept().unwrap();
                shutdown.send(s.try_clone().unwrap()).unwrap();
                let mut w = s.try_clone().unwrap();
                for line in BufReader::new(s).lines() {
                    let req: Value = serde_json::from_str(&line.unwrap()).unwrap();
                    record.send(req.clone()).unwrap();
                    let id = req["id"].as_u64();
                    match req["req"].as_str().unwrap() {
                        "hello" => reply(&mut w, id, json!({"ev":"hello_ok","version":API_VERSION_MAJOR,"server":"fake"})),
                        "attach_session" => reply(&mut w, id, json!({"ev":"attached","session":{"session_id":req["session_id"],"status":"idle"}})),
                        "fork_session" if fail => reply(&mut w, id, json!({"ev":"error","code":"internal","message":"Cannot fork"})),
                        "fork_session" => {
                            assert_eq!(req["session_id"], "source");
                            reply(&mut w, id, json!({"ev":"session_forked","session":{"session_id":"fork","status":"idle"}}));
                        }
                        "get_history" => {
                            assert_eq!(req["session_id"], "fork");
                            reply(&mut w, id, json!({"ev":"history","session_id":"fork","messages":[{"role":"assistant","content":"Inherited conversation"}]}));
                        }
                        "create_session" => panic!("Fork must not create a blank session"),
                        _ => reply(&mut w, id, json!({"ev":"error","code":"unknown_request","message":"no"})),
                    }
                }
            });
            let factory: TransportFactory = Arc::new(move || {
                Ok(Box::new(Tcp(TcpStream::connect(addr).unwrap())) as Box<dyn Transport>)
            });
            let mut config = Config::custom(factory);
            config.fork_from = Some("source".into());
            config.base_backoff = Duration::from_millis(1);
            let (connection, output) = Connection::start_with_channel(config, None);
            loop {
                let event = output.recv_timeout(Duration::from_secs(5)).unwrap();
                match event {
                    ConnectionOutput::Phase(ConnectionPhase::Failed { reason }) => {
                        assert!(fail && reason.contains("Could not fork session"));
                        break;
                    }
                    ConnectionOutput::Event(ApiEvent::History { session_id, .. }) => {
                        assert!(!fail);
                        assert_eq!(session_id, "fork");
                        break;
                    }
                    _ => {}
                }
            }
            connection.stop();
            socket.recv_timeout(Duration::from_secs(5)).unwrap()
                .shutdown(std::net::Shutdown::Both).unwrap();
            server.join().unwrap();
            let requests: Vec<_> = requests.try_iter().collect();
            assert_eq!(requests.iter().filter(|r| r["req"] == "fork_session").count(), 1);
            let attached: Vec<_> = requests.iter().filter(|r| r["req"] == "attach_session")
                .map(|r| r["session_id"].as_str().unwrap()).collect();
            assert_eq!(attached, if fail { vec!["source"] } else { vec!["source", "fork"] });
        }
    }

    /// Fake harness: hello, create_session, snapshot, then one streamed turn.
    #[test]
    fn creates_session_keeps_history_after_metadata_and_streams_turn() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (release_history, metadata_applied) = mpsc::channel();
        std::thread::spawn(move || {
            let (s, _) = listener.accept().unwrap();
            let mut w = s.try_clone().unwrap();
            let session = json!({"session_id":"s1","status":"idle","title":"Demo"});
            for line in BufReader::new(s).lines() {
                let req: Value = serde_json::from_str(&line.unwrap()).unwrap();
                let id = req["id"].as_u64();
                match req["req"].as_str().unwrap() {
                    "hello" => reply(&mut w, id, json!({"ev":"hello_ok","version":API_VERSION_MAJOR,"server":"fake"})),
                    "create_session" => {
                        assert_eq!(req["working_dir"], "/w");
                        reply(&mut w, id, json!({"ev":"attached","session":session}))
                    }
                    "get_history" => {
                        reply(&mut w, None, json!({"ev":"runtime_info","session_id":"s1","model":"test-model","routes":[]}));
                        reply(&mut w, None, json!({"ev":"session_status","session_id":"s1","status":"idle"}));
                        // Wait until both events have actually traversed the
                        // session loop, not merely been written to the socket.
                        metadata_applied.recv_timeout(Duration::from_secs(5)).unwrap();
                        reply(&mut w, id, json!({"ev":"history","session_id":"s1","messages":[{"role":"assistant","content":"earlier"}]}));
                    }
                    "send_message" => {
                        assert_eq!(req["content"], "hi");
                        for e in [
                            json!({"ev":"message_accepted","session_id":"s1"}),
                            json!({"ev":"text_delta","session_id":"s1","text":"yo"}),
                            json!({"ev":"turn_done","session_id":"s1"}),
                        ] {
                            reply(&mut w, None, e);
                        }
                    }
                    _ => reply(&mut w, id, json!({"ev":"error","code":"unknown_request","message":"no"})),
                }
            }
        });
        let factory: TransportFactory =
            Arc::new(move || Ok(Box::new(Tcp(TcpStream::connect(addr).unwrap())) as Box<dyn Transport>));
        let mut cfg = Config::custom(factory);
        cfg.working_dir = Some("/w".into());
        let (conn, rx) = Connection::start_with_channel(cfg, None);
        let mut state = crate::session::SessionState::default();
        let mut sent = false;
        let mut runtime_seen = false;
        let mut status_seen = false;
        let mut history_released = false;
        while let Ok(o) = rx.recv_timeout(Duration::from_secs(5)) {
            let history = matches!(&o, ConnectionOutput::Event(ApiEvent::History { .. }));
            runtime_seen |= matches!(&o, ConnectionOutput::Event(ApiEvent::RuntimeInfo { .. }));
            status_seen |= matches!(&o, ConnectionOutput::Event(ApiEvent::SessionStatus { .. }));
            state.apply(o);
            if runtime_seen && status_seen && !history_released {
                release_history.send(()).unwrap();
                history_released = true;
            }
            if history && !sent {
                assert!(history_released, "history must follow streamed metadata");
                state.apply_intent(crate::session::LocalIntent::UserSentMessage("hi".into()));
                conn.send(|id| Request::Message { id, content: "hi".into() });
                sent = true;
            }
            if sent && !state.is_processing && state.transcript.len() == 3 {
                break;
            }
        }
        conn.stop();
        assert_eq!(state.session_id.as_deref(), Some("s1"));
        assert_eq!(state.session_title.as_deref(), Some("Demo"));
        assert_eq!(state.server_version.as_deref(), Some("fake"));
        assert_eq!(state.model_name.as_deref(), Some("test-model"));
        assert_eq!(state.transcript.len(), 3);
        assert_eq!(state.transcript[0].text, "earlier");
        assert_eq!(state.transcript[1].text, "hi");
        assert_eq!(state.transcript[2].text, "yo");
    }

    #[test]
    fn withheld_snapshots_do_not_block_commands_or_rewind_streamed_history() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (release, released) = mpsc::channel();
        let (finished, snapshots_replied) = mpsc::channel();
        let server = std::thread::spawn(move || {
            let (s, _) = listener.accept().unwrap();
            s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let mut w = s.try_clone().unwrap();
            let mut snapshots = Vec::new();
            let mut sent = false;
            for line in BufReader::new(s).lines() {
                let req: Value = serde_json::from_str(&line.unwrap()).unwrap();
                let id = req["id"].as_u64();
                match req["req"].as_str().unwrap() {
                    "hello" => reply(&mut w, id, json!({"ev":"hello_ok","version":API_VERSION_MAJOR,"server":"fake"})),
                    "create_session" => reply(&mut w, id, json!({"ev":"attached","session":{"session_id":"s1","status":"idle"}})),
                    "get_history" | "get_runtime_info" | "list_models" | "list_sessions" => snapshots.push(req),
                    "send_message" => {
                        assert_eq!(req["content"], "hi");
                        sent = true;
                        reply(&mut w, id, json!({"ev":"message_accepted","session_id":"s1"}));
                        reply(&mut w, None, json!({"ev":"text_delta","session_id":"s1","text":"live"}));
                        reply(&mut w, None, json!({"ev":"turn_done","session_id":"s1"}));
                    }
                    // Old server: never answers the todo board fetch.
                    "get_todo_state" => {}
                    other => panic!("unexpected request: {other}"),
                }
                if sent && snapshots.len() == 4 {
                    released.recv_timeout(Duration::from_secs(5)).unwrap();
                    for req in snapshots {
                        let event = if req["req"] == "get_history" {
                            json!({"ev":"history","session_id":"s1","messages":[{"role":"assistant","content":"stale"}]})
                        } else {
                            json!({"ev":"error","code":"unknown_request","message":"snapshot completed"})
                        };
                        reply(&mut w, req["id"].as_u64(), event);
                    }
                    finished.send(()).unwrap();
                    // Keep the connection open until the test explicitly stops it.
                    released.recv_timeout(Duration::from_secs(5)).unwrap();
                    break;
                }
            }
        });
        let factory: TransportFactory =
            Arc::new(move || Ok(Box::new(Tcp(TcpStream::connect(addr).unwrap())) as Box<dyn Transport>));
        let (conn, rx) = Connection::start_with_channel(Config::custom(factory), None);
        conn.send(|id| Request::Message { id, content: "hi".into() });
        let mut state = crate::session::SessionState::default();
        state.apply_intent(crate::session::LocalIntent::UserSentMessage("hi".into()));
        loop {
            let output = rx.recv_timeout(Duration::from_secs(5)).expect("live turn blocked by snapshots");
            let done = matches!(&output, ConnectionOutput::Event(ApiEvent::TurnDone { .. }));
            state.apply(output);
            if done {
                break;
            }
        }
        assert_eq!(state.transcript.len(), 2);
        assert_eq!(state.transcript[1].text, "live");
        release.send(()).unwrap();
        snapshots_replied.recv_timeout(Duration::from_secs(5)).unwrap();
        // All three non-history replies must arrive independently. A late
        // initial History must never replace the already-rendered live turn.
        let mut completed = 0;
        while completed < 3 {
            let output = rx.recv_timeout(Duration::from_secs(5)).unwrap();
            assert!(!matches!(&output, ConnectionOutput::Event(ApiEvent::History { .. })));
            if matches!(&output, ConnectionOutput::Event(ApiEvent::Error { .. })) {
                completed += 1;
            }
            state.apply(output);
        }
        while let Ok(output) = rx.recv_timeout(Duration::from_millis(150)) {
            assert!(!matches!(&output, ConnectionOutput::Event(ApiEvent::History { .. })));
            state.apply(output);
        }
        assert_eq!(state.transcript.len(), 2);
        assert_eq!(state.transcript[0].text, "hi");
        assert_eq!(state.transcript[1].text, "live");
        conn.stop();
        release.send(()).unwrap();
        server.join().unwrap();
    }

    #[test]
    fn gives_up_after_max_attempts() {
        let factory: TransportFactory =
            Arc::new(|| Err(jcode_sdk::Error::new(ErrorKind::ConnectFailed, "refused")));
        let mut cfg = Config::custom(factory);
        cfg.max_reconnect_attempts = Some(1);
        cfg.base_backoff = Duration::from_millis(10);
        let (_c, rx) = Connection::start_with_channel(cfg, None);
        let last = rx.iter().take_while(|_| true).last().unwrap();
        assert!(
            matches!(&last, ConnectionOutput::Phase(ConnectionPhase::Failed { reason }) if reason.contains("refused")),
            "{last:?}"
        );
    }
}
