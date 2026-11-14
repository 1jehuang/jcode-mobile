//! JNI is only a JSON/UI adapter. The shared core owns connections and reduction.
use jcode_mobile_core::{
    connection::{Config, Connection, Request},
    session::{ConnectionOutput, ConnectionPhase, LocalIntent, Role, SessionState},
};
use jni::{
    JNIEnv,
    objects::{JClass, JString},
    sys::jstring,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Write},
    net::{TcpStream, ToSocketAddrs},
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{Arc, Mutex, OnceLock, mpsc::Receiver},
    time::Duration,
};

const MAX_REQUEST_BYTES: usize = 256 * 1024;
const MAX_TEXT_BYTES: usize = 128 * 1024;

#[derive(Deserialize)]
struct Command {
    action: String,
    endpoint: Option<String>,
    token: Option<String>,
    session_id: Option<String>,
    text: Option<String>,
}

#[derive(Clone)]
enum Endpoint {
    Tcp { host: String, port: u16 },
    Unix(std::path::PathBuf),
}

impl Endpoint {
    fn parse(input: &str) -> Result<Self, &'static str> {
        let url = url::Url::parse(input).map_err(|_| "Invalid endpoint URL")?;
        if !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err("Endpoint must not contain credentials, query or fragment");
        }
        match url.scheme() {
            "tcp" if matches!(url.path(), "" | "/") => {
                let host = url
                    .host_str()
                    .filter(|s| !s.is_empty())
                    .ok_or("Endpoint host is required")?;
                let port = url
                    .port()
                    .filter(|p| *p != 0)
                    .ok_or("Endpoint port is required")?;
                Ok(Self::Tcp {
                    host: host.trim_matches(['[', ']']).into(),
                    port,
                })
            }
            "unix"
                if url.host_str().is_none()
                    && url.path().starts_with('/')
                    && url.path().len() > 1 =>
            {
                Ok(Self::Unix(url.path().into()))
            }
            _ => Err(
                "Use tcp://host:port for a forwarded harness API socket, not the legacy WebSocket gateway",
            ),
        }
    }

    fn open(&self) -> jcode_sdk::Result<Box<dyn jcode_sdk::Transport>> {
        match self {
            Self::Tcp { host, port } => {
                let addresses = (host.as_str(), *port)
                    .to_socket_addrs()
                    .map_err(|_| transport_error())?;
                let deadline = std::time::Instant::now() + Duration::from_secs(8);
                for address in addresses.take(8) {
                    let Some(left) = deadline.checked_duration_since(std::time::Instant::now())
                    else {
                        break;
                    };
                    if let Ok(stream) = TcpStream::connect_timeout(&address, left) {
                        stream.set_nodelay(true).map_err(|_| transport_error())?;
                        return Ok(Box::new(TcpTransport(stream)));
                    }
                }
                Err(transport_error())
            }
            Self::Unix(path) => Ok(Box::new(
                jcode_sdk::UnixTransport::connect(path).map_err(|_| transport_error())?,
            )),
        }
    }
}

fn transport_error() -> jcode_sdk::Error {
    jcode_sdk::Error::new(
        jcode_sdk::ErrorKind::ConnectFailed,
        "Could not connect to harness API endpoint",
    )
}

struct TcpTransport(TcpStream);
impl jcode_sdk::Transport for TcpTransport {
    fn shutdown_handle(&self) -> Option<Arc<dyn Fn() + Send + Sync>> {
        let socket = self.0.try_clone().ok()?;
        Some(Arc::new(move || {
            let _ = socket.shutdown(std::net::Shutdown::Both);
        }))
    }
    fn split(
        self: Box<Self>,
    ) -> jcode_sdk::Result<(Box<dyn BufRead + Send>, Box<dyn Write + Send>)> {
        let reader = self.0.try_clone().map_err(|_| transport_error())?;
        Ok((Box::new(BufReader::new(reader)), Box::new(self.0)))
    }
}

#[derive(Default)]
pub struct Bridge {
    state: SessionState,
    connection: Option<Connection>,
    outputs: Option<Receiver<ConnectionOutput>>,
    endpoint: Option<Endpoint>,
    demo: bool,
}

impl Drop for Bridge {
    fn drop(&mut self) {
        self.stop();
    }
}

impl Bridge {
    fn stop(&mut self) {
        if let Some(connection) = self.connection.take() {
            connection.stop();
        }
        self.outputs = None;
    }

    fn drain(&mut self) {
        if let Some(outputs) = &self.outputs {
            // Bound each poll so a busy stream cannot starve the Java UI.
            for output in outputs.try_iter().take(2048) {
                self.state.apply(output);
            }
        }
        self.state.alerts.clear();
    }

    fn start(&mut self, endpoint: Endpoint, session: Option<String>) {
        self.stop();
        self.demo = false;
        self.state = SessionState::default();
        self.state.phase = ConnectionPhase::Connecting;
        let target = endpoint.clone();
        let mut config = Config::custom(Arc::new(move || target.open()));
        config.max_reconnect_attempts = Some(3);
        let (connection, outputs) = Connection::start_with_channel(config, session);
        self.connection = Some(connection);
        self.outputs = Some(outputs);
        self.endpoint = Some(endpoint);
    }

    pub fn request(&mut self, input: &str) -> String {
        self.drain();
        let result = if input.len() > MAX_REQUEST_BYTES {
            Err("Request is too large")
        } else {
            serde_json::from_str::<Command>(input)
                .map_err(|_| "Invalid JSON request")
                .and_then(|command| self.execute(command))
        };
        self.drain();
        self.snapshot(result.err()).to_string()
    }

    fn execute(&mut self, command: Command) -> Result<(), &'static str> {
        match command.action.as_str() {
            "status" | "poll" => {}
            "connect" => {
                let endpoint =
                    Endpoint::parse(command.endpoint.as_deref().ok_or("Endpoint is required")?)?;
                if command.token.as_deref().is_some_and(|s| !s.is_empty()) {
                    return Err(
                        "Raw harness transport does not support gateway tokens. Use a trusted tunnel and leave token empty",
                    );
                }
                self.start(endpoint, command.session_id);
            }
            "sessions" => {
                if let Some(connection) = &self.connection {
                    connection.send(|id| Request::ListSessions { id });
                }
            }
            "select_session" => {
                let session = command
                    .session_id
                    .filter(|s| !s.trim().is_empty() && s.len() <= 512)
                    .ok_or("Session ID is required")?;
                if self.demo {
                    if session != "offline-demo" {
                        return Err("Unknown demo session");
                    }
                } else {
                    let endpoint = self.endpoint.clone().ok_or("Connect to a server first")?;
                    self.start(endpoint, Some(session));
                }
            }
            "send" => {
                let text = command
                    .text
                    .filter(|s| !s.trim().is_empty() && s.len() <= MAX_TEXT_BYTES)
                    .ok_or("Message must contain 1 to 131072 bytes")?;
                if self.demo {
                    self.state.apply_intent(LocalIntent::UserSentMessage(text));
                    self.state.apply(ConnectionOutput::Event(jcode_sdk::ApiEvent::TextDelta {
                        session_id: "offline-demo".into(),
                        text: "Offline demo: no message was sent to a server. Connect to a harness API endpoint for a real response.".into(),
                        message_id: None,
                    }));
                    self.state
                        .apply(ConnectionOutput::Event(jcode_sdk::ApiEvent::TurnDone {
                            session_id: "offline-demo".into(),
                            pending_soft_interrupts: None,
                        }));
                } else {
                    if self.state.phase != ConnectionPhase::Connected
                        || self.state.session_id.is_none()
                    {
                        return Err("Wait for a connected session before sending");
                    }
                    if self.state.is_processing {
                        return Err("Wait for the current response before sending");
                    }
                    let connection = self
                        .connection
                        .as_ref()
                        .ok_or("Connect to a server first")?;
                    connection.send(|id| Request::Message {
                        id,
                        content: text.clone(),
                    });
                    self.state.apply_intent(LocalIntent::UserSentMessage(text));
                }
            }
            "disconnect" => {
                self.stop();
                self.endpoint = None;
                self.demo = false;
                self.state = SessionState::default();
            }
            "demo" => {
                self.stop();
                self.endpoint = None;
                self.demo = true;
                self.state = SessionState::default();
                self.state.session_id = Some("offline-demo".into());
                self.state.session_title = Some("Offline demo".into());
                self.state
                    .transcript
                    .push(jcode_mobile_core::session::TranscriptEntry {
                        id: 0,
                        role: Role::System,
                        text:
                            "Offline demo. No server is connected and messages stay on this device."
                                .into(),
                        reasoning: String::new(),
                        tool_calls: vec![],
                        is_streaming: false,
                        is_queued: false,
                    });
            }
            _ => return Err("Unknown action"),
        }
        Ok(())
    }

    fn snapshot(&self, error: Option<&str>) -> Value {
        let status = if self.demo {
            "demo"
        } else {
            match &self.state.phase {
                ConnectionPhase::Disconnected => "disconnected",
                ConnectionPhase::Connecting => "connecting",
                ConnectionPhase::Provisioning { .. } => "provisioning",
                ConnectionPhase::Connected => "connected",
                ConnectionPhase::Reconnecting { .. } => "reconnecting",
                ConnectionPhase::Failed { .. } => "failed",
            }
        };
        // Connection and server errors can contain arbitrary remote content. Do not
        // expose them here: this adapter never returns endpoints, tokens, or logs.
        let state_error = if matches!(self.state.phase, ConnectionPhase::Failed { .. }) {
            Some("Could not connect to harness API server")
        } else if self.state.error_banner.is_some() {
            Some("Server request failed")
        } else {
            None
        };
        let sessions: Vec<Value> = if self.demo {
            vec![
                json!({"id":"offline-demo","session_id":"offline-demo","title":"Offline demo","status":"offline"}),
            ]
        } else {
            self.state.session_list.iter().map(|s| json!({"id":s.session_id,"session_id":s.session_id,"title":self.state.title_for(&s.session_id).unwrap_or(&s.session_id),"status":s.status})).collect()
        };
        let messages: Vec<Value> = self.state.transcript.iter().map(|m| json!({
            "id":m.id,"role":match m.role { Role::User => "user", Role::Assistant => "assistant", Role::System => "system" },
            "text":m.text,"reasoning":m.reasoning,"streaming":m.is_streaming,"queued":m.is_queued,
            "tools":m.tool_calls.iter().map(|t| json!({"id":t.id,"name":t.name,"input":t.input,"output":t.output})).collect::<Vec<_>>()
        })).collect();
        json!({"ok":error.is_none(),"error":error.or(state_error),"status":status,"offline_demo":self.demo,
            "sessions":sessions,"selected_session_id":self.state.session_id,"messages":messages,
            "is_processing":self.state.is_processing,"session_title":self.state.session_title,"model":self.state.model_name})
    }
}

static BRIDGE: OnceLock<Mutex<Bridge>> = OnceLock::new();

/// Never let a Rust panic unwind into the JVM. JNI allocation failure returns
/// null with the JVM exception pending, as required by JNI.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_jcode_mobile_NativeBridge_request(
    mut env: JNIEnv,
    _class: JClass,
    input: JString,
) -> jstring {
    let response = catch_unwind(AssertUnwindSafe(|| {
        let input: String = match env.get_string(&input) {
            Ok(value) => value.into(),
            Err(_) => return json!({"ok":false,"error":"Invalid JNI string"}).to_string(),
        };
        let mut bridge = BRIDGE
            .get_or_init(|| Mutex::new(Bridge::default()))
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        bridge.request(&input)
    }))
    .unwrap_or_else(|_| "{\"ok\":false,\"error\":\"Native request failed\"}".into());
    catch_unwind(AssertUnwindSafe(|| {
        env.new_string(response)
            .map(|s| s.into_raw())
            .unwrap_or(std::ptr::null_mut())
    }))
    .unwrap_or(std::ptr::null_mut())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wire_reply(writer: &mut TcpStream, id: Option<u64>, mut event: Value) {
        event["v"] = json!(jcode_sdk::api::API_VERSION_MAJOR);
        if let Some(id) = id {
            event["reply_to"] = json!(id);
        }
        let frame: jcode_sdk::api::ServerFrame = serde_json::from_value(event).unwrap();
        writeln!(writer, "{}", serde_json::to_string(&frame).unwrap()).unwrap();
    }

    fn poll_until(bridge: &mut Bridge, predicate: impl Fn(&Value) -> bool) -> Value {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            let snapshot = request(bridge, json!({"action":"poll"}));
            if predicate(&snapshot) {
                return snapshot;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "timed out: {snapshot}"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn real_tcp_transport_connects_lists_streams_and_selects() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (sent, received) = std::sync::mpsc::channel();
        let server = std::thread::spawn(move || {
            for generation in 0..2 {
                let (socket, _) = listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(10)))
                    .unwrap();
                let mut writer = socket.try_clone().unwrap();
                let sid = if generation == 0 { "s1" } else { "s2" };
                for line in BufReader::new(socket).lines() {
                    let Ok(line) = line else { break };
                    let req: Value = serde_json::from_str(&line).unwrap();
                    let id = req["id"].as_u64();
                    match req["req"].as_str().unwrap() {
                        "hello" => wire_reply(
                            &mut writer,
                            id,
                            json!({"ev":"hello_ok","version":jcode_sdk::api::API_VERSION_MAJOR,"server":"test"}),
                        ),
                        "create_session" | "attach_session" => {
                            if generation == 1 {
                                assert_eq!(req["session_id"], "s2");
                            }
                            wire_reply(
                                &mut writer,
                                id,
                                json!({"ev":"attached","session":{"session_id":sid,"status":"idle","title":"Test session"}}),
                            );
                        }
                        "list_sessions" => wire_reply(
                            &mut writer,
                            id,
                            json!({"ev":"sessions","sessions":[{"session_id":"s1","status":"idle"},{"session_id":"s2","status":"idle"}]}),
                        ),
                        "get_history" => wire_reply(
                            &mut writer,
                            id,
                            json!({"ev":"history","session_id":sid,"messages":[{"role":"assistant","content":"earlier"}]}),
                        ),
                        "send_message" => {
                            sent.send(req["content"].as_str().unwrap().to_owned())
                                .unwrap();
                            wire_reply(
                                &mut writer,
                                id,
                                json!({"ev":"message_accepted","session_id":sid}),
                            );
                            wire_reply(
                                &mut writer,
                                None,
                                json!({"ev":"text_delta","session_id":sid,"text":"actual streamed reply"}),
                            );
                            wire_reply(
                                &mut writer,
                                None,
                                json!({"ev":"turn_done","session_id":sid}),
                            );
                        }
                        _ => wire_reply(
                            &mut writer,
                            id,
                            json!({"ev":"error","code":"unknown_request","message":"unsupported"}),
                        ),
                    }
                }
            }
        });
        let mut bridge = Bridge::default();
        let connected = request(
            &mut bridge,
            json!({"action":"connect","endpoint":format!("tcp://{address}")}),
        );
        assert!(connected["ok"].as_bool().unwrap());
        poll_until(&mut bridge, |s| {
            s["selected_session_id"] == "s1"
                && s["sessions"].as_array().unwrap().len() == 2
                && s["messages"].as_array().unwrap().len() == 1
        });
        assert_eq!(
            request(&mut bridge, json!({"action":"send","text":"hello server"}))["ok"],
            true
        );
        assert_eq!(
            received.recv_timeout(Duration::from_secs(5)).unwrap(),
            "hello server"
        );
        let reply = poll_until(&mut bridge, |s| {
            s["is_processing"] == false
                && s["messages"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|m| m["text"] == "actual streamed reply")
        });
        assert_eq!(reply["offline_demo"], false);
        assert_eq!(
            request(
                &mut bridge,
                json!({"action":"select_session","session_id":"s2"})
            )["ok"],
            true
        );
        poll_until(&mut bridge, |s| s["selected_session_id"] == "s2");
        request(&mut bridge, json!({"action":"disconnect"}));
        server.join().unwrap();
    }
    fn request(bridge: &mut Bridge, command: Value) -> Value {
        serde_json::from_str(&bridge.request(&command.to_string())).unwrap()
    }

    #[test]
    fn offline_demo_is_explicit_and_disconnect_resets() {
        let mut bridge = Bridge::default();
        let demo = request(&mut bridge, json!({"action":"demo"}));
        assert_eq!(demo["status"], "demo");
        assert_eq!(demo["offline_demo"], true);
        let reply = request(&mut bridge, json!({"action":"send","text":"hello"}));
        assert_eq!(reply["messages"].as_array().unwrap().len(), 3);
        assert!(
            reply["messages"][2]["text"]
                .as_str()
                .unwrap()
                .contains("no message was sent")
        );
        let reset = request(&mut bridge, json!({"action":"disconnect"}));
        assert_eq!(reset["status"], "disconnected");
        assert_eq!(reset["offline_demo"], false);
        assert_eq!(reset["messages"], json!([]));
    }

    #[test]
    fn validates_endpoints_and_rejects_ignored_secrets() {
        for endpoint in [
            "https://host",
            "ws://host/ws",
            "tcp://host",
            "tcp://host:0",
            "tcp://user:secret@host:123",
            "tcp://host:123?token=secret",
            "tcp://host:123/path",
        ] {
            assert!(Endpoint::parse(endpoint).is_err(), "accepted {endpoint}");
        }
        assert!(Endpoint::parse("tcp://127.0.0.1:7643").is_ok());
        assert!(Endpoint::parse("tcp://[::1]:7643").is_ok());
        let response = request(
            &mut Bridge::default(),
            json!({"action":"connect","endpoint":"tcp://127.0.0.1:1234","token":"secret-token"}),
        );
        assert_eq!(response["ok"], false);
        assert!(!response.to_string().contains("secret-token"));
    }

    #[test]
    fn invalid_commands_are_safe() {
        let mut bridge = Bridge::default();
        for input in ["", "null", "{", "{\"action\":\"unknown\"}"] {
            let value: Value = serde_json::from_str(&bridge.request(input)).unwrap();
            assert_eq!(value["ok"], false);
        }
        assert_eq!(
            request(&mut bridge, json!({"action":"send","text":"hello"}))["ok"],
            false
        );
        assert_eq!(
            request(&mut bridge, json!({"action":"send","text":" "}))["ok"],
            false
        );
        assert_eq!(request(&mut bridge, json!({"action":"status"}))["ok"], true);
    }
}
