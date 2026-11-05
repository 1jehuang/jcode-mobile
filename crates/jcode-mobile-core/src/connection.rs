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
    let inbox = Mutex::new(Inbox { rx: commands, queued: vec![], stopped: false });
    let mut attempt: u32 = 0;
    loop {
        phase(if attempt == 0 { ConnectionPhase::Connecting } else { ConnectionPhase::Reconnecting { attempt } });
        let outcome = open(&config, &inbox, &emit).map_err(classify).and_then(|client| {
            let session = attach(&client, target_session.as_deref(), config.working_dir.clone())
                .map_err(classify)?;
            Ok((client, session))
        });
        if inbox.lock().unwrap().stopped {
            break;
        }
        let last_error: String;
        match outcome {
            Ok((client, session)) => {
                let sid = session.session_id.clone();
                target_session = Some(sid.clone());
                phase(ConnectionPhase::Connected);
                attempt = 0;
                emit(ConnectionOutput::Event(ApiEvent::HelloOk {
                    version: jcode_sdk::api::API_VERSION_MAJOR,
                    server: client.server.clone(),
                    capabilities: client.capabilities.clone(),
                }));
                emit(ConnectionOutput::Event(ApiEvent::Attached { session }));
                let stream = client.events(Some(&sid));
                // Initial snapshot. Failures here surface as a lost connection.
                for req in [
                    jcode_sdk::ApiRequest::GetHistory { session_id: sid.clone() },
                    jcode_sdk::ApiRequest::GetRuntimeInfo { session_id: sid.clone() },
                    jcode_sdk::ApiRequest::ListModels { session_id: sid.clone() },
                    jcode_sdk::ApiRequest::ListSessions {
                        include_archived: false,
                        limit: Some(SESSION_LIST_LIMIT),
                    },
                ] {
                    if let Ok(frame) = client.request(req) {
                        emit(ConnectionOutput::Event(frame.event));
                    }
                }
                if session_loop(&client, &sid, &stream, &inbox, &emit) {
                    break;
                }
                last_error = "Connection lost".into();
            }
            Err(Failure::Stopped) => break,
            Err(Failure::Fatal(reason)) => {
                phase(ConnectionPhase::Failed { reason });
                return;
            }
            Err(Failure::Retry(e)) => last_error = e,
        }
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
    stream: &jcode_sdk::EventStream,
    inbox: &Mutex<Inbox>,
    emit: &dyn Fn(ConnectionOutput),
) -> bool {
    let (results, outcomes) = mpsc::channel::<ApiEvent>();
    loop {
        let queued = {
            let mut i = inbox.lock().unwrap();
            i.drain();
            if i.stopped {
                return true;
            }
            std::mem::take(&mut i.queued)
        };
        for request in queued {
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
            Some(event) => emit(ConnectionOutput::Event(event)),
            None if client.is_closed() => return false,
            None => {}
        }
    }
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

    /// Fake harness: hello, create_session, snapshot, then one streamed turn.
    #[test]
    fn creates_session_and_streams_turn() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
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
                    "get_history" => reply(&mut w, id, json!({"ev":"history","session_id":"s1","messages":[]})),
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
        while let Ok(o) = rx.recv_timeout(Duration::from_secs(5)) {
            state.apply(o);
            if state.session_id.is_some() && !sent {
                state.apply_intent(crate::session::LocalIntent::UserSentMessage("hi".into()));
                conn.send(|id| Request::Message { id, content: "hi".into() });
                sent = true;
            }
            if sent && !state.is_processing && state.transcript.len() == 2 {
                break;
            }
        }
        conn.stop();
        assert_eq!(state.session_id.as_deref(), Some("s1"));
        assert_eq!(state.session_title.as_deref(), Some("Demo"));
        assert_eq!(state.server_version.as_deref(), Some("fake"));
        assert_eq!(state.transcript[1].text, "yo");
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
