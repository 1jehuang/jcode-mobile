//! One WebSocket connection to a jcode server, run on a background thread.
//! Port of Connection.swift: connect with Bearer auth, subscribe + get_history,
//! decode NDJSON frames, reconnect with capped exponential backoff, stop on 401
//! or a server close request.
//!
//! Threads instead of async keep the core runtime-free, which matters for the
//! iOS target where GPUI owns the main loop.

use crate::gateway::Gateway;
use crate::session::{ConnectionOutput, ConnectionPhase};
use crate::wire::{Request, ServerEvent};
use std::io::ErrorKind;
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tungstenite::client::IntoClientRequest;
use tungstenite::{Message, WebSocket};

#[derive(Debug, Clone)]
pub struct Config {
    pub gateway: Gateway,
    pub auth_token: String,
    pub max_reconnect_attempts: Option<u32>,
    pub base_backoff: Duration,
    pub working_dir: Option<String>,
}

impl Config {
    pub fn new(gateway: Gateway, auth_token: impl Into<String>) -> Self {
        Self {
            gateway,
            auth_token: auth_token.into(),
            max_reconnect_attempts: None,
            base_backoff: Duration::from_secs(1),
            working_dir: None,
        }
    }
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
    /// for each phase change and decoded event (use it to wake the UI).
    pub fn start(
        config: Config,
        resume_session_id: Option<String>,
        on_output: impl Fn(ConnectionOutput) + Send + 'static,
    ) -> Connection {
        let (tx, rx) = mpsc::channel();
        let next_id = Arc::new(Mutex::new(1));
        let ids = next_id.clone();
        std::thread::Builder::new()
            .name("jcode-connection".into())
            .spawn(move || run(config, resume_session_id, rx, ids, on_output))
            .expect("spawn connection thread");
        Connection { commands: tx, next_id }
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
        let id = alloc_id(&self.next_id);
        let _ = self.commands.send(Command::Send(build(id)));
        id
    }

    pub fn stop(&self) {
        let _ = self.commands.send(Command::Stop);
    }
}

fn alloc_id(ids: &Mutex<u64>) -> u64 {
    let mut g = ids.lock().unwrap();
    let id = *g;
    *g += 1;
    id
}

enum ConnectError {
    Unauthorized,
    Other,
}

fn connect(config: &Config) -> Result<WebSocket<TcpStream>, ConnectError> {
    let gw = &config.gateway;
    let addr = (gw.host.as_str(), gw.port)
        .to_socket_addrs()
        .map_err(|_| ConnectError::Other)?
        .next()
        .ok_or(ConnectError::Other)?;
    let stream = TcpStream::connect_timeout(&addr, Duration::from_secs(10))
        .map_err(|_| ConnectError::Other)?;
    stream.set_read_timeout(Some(Duration::from_secs(10))).ok();
    let mut req = gw
        .web_socket_url()
        .into_client_request()
        .map_err(|_| ConnectError::Other)?;
    req.headers_mut().insert(
        "Authorization",
        format!("Bearer {}", config.auth_token).parse().unwrap(),
    );
    match tungstenite::client(req, stream) {
        Ok((ws, _)) => {
            // Short read timeout so the loop can interleave outgoing sends.
            ws.get_ref().set_read_timeout(Some(Duration::from_millis(50))).ok();
            ws.get_ref().set_nodelay(true).ok();
            Ok(ws)
        }
        Err(tungstenite::HandshakeError::Failure(tungstenite::Error::Http(resp)))
            if resp.status() == 401 =>
        {
            Err(ConnectError::Unauthorized)
        }
        Err(_) => Err(ConnectError::Other),
    }
}

enum SessionEnd {
    Closed,
    Stopped,
    CloseRequested(String),
    Reload,
}

fn run(
    config: Config,
    mut target_session: Option<String>,
    commands: Receiver<Command>,
    ids: Arc<Mutex<u64>>,
    emit: impl Fn(ConnectionOutput),
) {
    let phase = |p| emit(ConnectionOutput::Phase(p));
    let mut attempt: u32 = 0;
    loop {
        phase(if attempt == 0 {
            ConnectionPhase::Connecting
        } else {
            ConnectionPhase::Reconnecting { attempt }
        });
        let mut expect_reload = false;
        match connect(&config) {
            Ok(mut ws) => {
                phase(ConnectionPhase::Connected);
                attempt = 0;
                let subscribe = Request::Subscribe {
                    id: alloc_id(&ids),
                    target_session_id: target_session.clone(),
                    working_dir: config.working_dir.clone(),
                };
                let history = Request::GetHistory { id: alloc_id(&ids) };
                let ok = ws.send(Message::text(subscribe.encoded_line())).is_ok()
                    && ws.send(Message::text(history.encoded_line())).is_ok();
                let end = if ok {
                    session_loop(&mut ws, &commands, &mut target_session, &emit)
                } else {
                    SessionEnd::Closed
                };
                let _ = ws.close(None);
                let _ = ws.flush();
                match end {
                    SessionEnd::Stopped => break,
                    SessionEnd::CloseRequested(reason) => {
                        phase(ConnectionPhase::Failed { reason });
                        return;
                    }
                    SessionEnd::Reload => expect_reload = true,
                    SessionEnd::Closed => {}
                }
            }
            Err(ConnectError::Unauthorized) => {
                phase(ConnectionPhase::Failed {
                    reason: "This device is no longer paired with the server. Re-pair to reconnect."
                        .into(),
                });
                return;
            }
            Err(ConnectError::Other) => {}
        }
        attempt += 1;
        if let Some(max) = config.max_reconnect_attempts
            && attempt > max
        {
            phase(ConnectionPhase::Failed {
                reason: format!("Could not reach server after {max} attempts"),
            });
            return;
        }
        let delay = if expect_reload {
            config.base_backoff / 2
        } else {
            (config.base_backoff * 2u32.saturating_pow(attempt - 1)).min(Duration::from_secs(30))
        };
        // Sleep, but stay responsive to stop.
        match commands.recv_timeout(delay) {
            Ok(Command::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
            _ => {}
        }
    }
    phase(ConnectionPhase::Disconnected);
}

fn session_loop(
    ws: &mut WebSocket<TcpStream>,
    commands: &Receiver<Command>,
    target_session: &mut Option<String>,
    emit: &impl Fn(ConnectionOutput),
) -> SessionEnd {
    let mut reload = false;
    loop {
        loop {
            match commands.try_recv() {
                Ok(Command::Send(r)) => {
                    if ws.send(Message::text(r.encoded_line())).is_err() {
                        return SessionEnd::Closed;
                    }
                }
                Ok(Command::Stop) | Err(TryRecvError::Disconnected) => return SessionEnd::Stopped,
                Err(TryRecvError::Empty) => break,
            }
        }
        let text = match ws.read() {
            Ok(Message::Text(t)) => t.to_string(),
            Ok(Message::Binary(b)) => String::from_utf8_lossy(&b).into_owned(),
            Ok(Message::Close(_)) => {
                return if reload { SessionEnd::Reload } else { SessionEnd::Closed };
            }
            Ok(_) => continue,
            Err(tungstenite::Error::Io(e))
                if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) =>
            {
                continue;
            }
            Err(_) => return if reload { SessionEnd::Reload } else { SessionEnd::Closed },
        };
        for line in text.split('\n').filter(|l| !l.trim().is_empty()) {
            let Ok(event) = ServerEvent::decode(line) else { continue };
            let mut close = None;
            match &event {
                ServerEvent::SessionId { session_id } => *target_session = Some(session_id.clone()),
                ServerEvent::Reloading { .. } => reload = true,
                ServerEvent::SessionCloseRequested { reason } => {
                    close = Some(if reason.is_empty() {
                        "Server closed this session".to_string()
                    } else {
                        reason.clone()
                    })
                }
                _ => {}
            }
            emit(ConnectionOutput::Event(event));
            if let Some(reason) = close {
                return SessionEnd::CloseRequested(reason);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    /// Fake gateway: checks auth + subscribe, streams a reply to one message.
    #[test]
    fn subscribes_and_streams_events() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (s, _) = listener.accept().unwrap();
            let check = |req: &tungstenite::handshake::server::Request,
                         resp: tungstenite::handshake::server::Response| {
                assert_eq!(req.headers()["authorization"], "Bearer tok");
                Ok(resp)
            };
            let mut ws = tungstenite::accept_hdr(s, check).unwrap();
            let sub = ws.read().unwrap().into_text().unwrap();
            assert!(sub.contains(r#""type":"subscribe""#) && sub.contains(r#""working_dir":"/w""#));
            let hist = ws.read().unwrap().into_text().unwrap();
            assert!(hist.contains("get_history"));
            ws.send(Message::text(r#"{"type":"history","id":2,"session_id":"s1","messages":[]}"#))
                .unwrap();
            let msg = ws.read().unwrap().into_text().unwrap();
            assert!(msg.contains(r#""content":"hi""#));
            ws.send(Message::text(
                "{\"type\":\"text_delta\",\"text\":\"yo\"}\n{\"type\":\"done\",\"id\":3}",
            ))
            .unwrap();
            let _ = ws.read();
        });
        let mut cfg = Config::new(Gateway::new("127.0.0.1", port), "tok");
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
        assert_eq!(state.transcript[1].text, "yo");
    }

    #[test]
    fn unauthorized_fails_without_retry() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            use std::io::{Read, Write};
            let (mut s, _) = listener.accept().unwrap();
            let mut buf = [0; 2048];
            let _ = s.read(&mut buf);
            s.write_all(b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\n\r\n").unwrap();
        });
        let (_c, rx) =
            Connection::start_with_channel(Config::new(Gateway::new("127.0.0.1", port), "bad"), None);
        let phases: Vec<_> = rx.iter().take(2).collect();
        assert!(matches!(
            phases[1],
            ConnectionOutput::Phase(ConnectionPhase::Failed { .. })
        ));
    }
}
