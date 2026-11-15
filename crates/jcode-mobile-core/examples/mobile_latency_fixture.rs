//! Loopback-only, real pinned-SSH cloud fixture for the unmodified mobile app.
//! Run with --help. No account files are read and no external network is used.
//!
//! `justrust run -p jcode-mobile-core --example mobile_latency_fixture --
//! --self-test --history-size 200 --cp-delay-ms 30 --hello-delay-ms 40`
//!
//! For the actual native app, omit --self-test and save the ready JSON's
//! `account` object into an isolated account.json. Launch that app with isolated
//! XDG_DATA_HOME and XDG_CACHE_HOME, never with the user's real account/cache.
//! Use fixed --http-port and --ssh-port when bridging an Android emulator with
//! adb reverse, since control-plane replies intentionally advertise loopback.
//! Safe JSONL stages contain no request bodies, public/private keys or accounts.
use jcode_sdk::api::{API_VERSION_MAJOR, ServerFrame};
use russh::{ChannelId, server};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    io::Write,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const LIMIT: usize = 256 * 1024;
const TOKEN: &str = "mobile-latency-fixture-not-a-secret";
#[derive(Default)]
struct Options {
    http_port: u16,
    ssh_port: u16,
    cp: u64,
    auth: u64,
    handshake: u64,
    hello: u64,
    attach: u64,
    history: u64,
    size: usize,
    history_bytes: usize,
    boot: usize,
    duration: u64,
    log: Option<String>,
    stop: Option<String>,
    self_test: bool,
}
impl Options {
    fn parse() -> Result<Self, Box<dyn std::error::Error>> {
        let mut o = Self {
            size: 20,
            history_bytes: 128,
            duration: 3600,
            ..Self::default()
        };
        let mut args = std::env::args().skip(1);
        while let Some(key) = args.next() {
            if key == "--self-test" {
                o.self_test = true;
                continue;
            }
            if key == "--help" {
                println!(
                    "mobile_latency_fixture [--self-test] [--http-port 0] [--ssh-port 0] [--handshake-ms 0] [--cp-delay-ms 0] [--ssh-auth-delay-ms 0] [--hello-delay-ms 0] [--attach-delay-ms 0] [--history-delay-ms 0] [--history-size 20] [--boot-polls 0] [--duration-secs 3600] [--log JSONL] [--stop-file PATH]\nStop gracefully by creating stop-file, writing a newline to stdin, or waiting duration-secs. History size counts messages (max 10000). Delays max 60000ms. Only loopback is bound. Ready stdout contains a fake account suitable for the native app's account injection."
                );
                std::process::exit(0);
            }
            let v = args.next().ok_or("missing option value")?;
            if v.len() > 4096 {
                return Err("argument too long".into());
            }
            match key.as_str() {
                "--http-port" => o.http_port = v.parse()?,
                "--ssh-port" => o.ssh_port = v.parse()?,
                "--cp-delay-ms" | "--cp-ms" => o.cp = v.parse()?,
                "--ssh-auth-delay-ms" | "--auth-ms" => o.auth = v.parse()?,
                "--handshake-ms" => o.handshake = v.parse()?,
                "--hello-delay-ms" | "--hello-ms" => o.hello = v.parse()?,
                "--attach-delay-ms" | "--attach-ms" => o.attach = v.parse()?,
                "--history-delay-ms" | "--history-ms" => o.history = v.parse()?,
                "--history-size" | "--history-messages" => o.size = v.parse()?,
                "--history-bytes" => o.history_bytes = v.parse()?,
                "--boot-polls" => o.boot = v.parse()?,
                "--duration-secs" => o.duration = v.parse()?,
                "--log" => o.log = Some(v),
                "--stop-file" => o.stop = Some(v),
                _ => return Err("unknown option (use --help)".into()),
            }
        }
        if [o.cp, o.auth, o.handshake, o.hello, o.attach, o.history]
            .into_iter()
            .any(|v| v > 60000)
            || o.size > 10000
            || !(32..=8192).contains(&o.history_bytes)
            || o.size.saturating_mul(o.history_bytes) > 32 * 1024 * 1024
            || o.boot > 100
            || !(1..=86400).contains(&o.duration)
        {
            return Err("option exceeds fixture bounds".into());
        }
        Ok(o)
    }
}
struct State {
    options: Options,
    keys: Mutex<Vec<russh::keys::PublicKey>>,
    polls: AtomicUsize,
    next_session: AtomicUsize,
    pings: AtomicUsize,
    hellos: AtomicUsize,
    started: Instant,
    log: Mutex<Option<std::fs::File>>,
}
impl State {
    fn stage(&self, stage: &'static str) {
        let line = json!({"stage":stage,"elapsed_ms":self.started.elapsed().as_millis(),"connect_polls":self.polls.load(Ordering::Relaxed)});
        if let Some(file) = self.log.lock().unwrap().as_mut() {
            let _ = writeln!(file, "{line}");
            let _ = file.flush();
        }
    }
}
struct Harness {
    state: Arc<State>,
    buffers: HashMap<ChannelId, Vec<u8>>,
    attached: HashMap<ChannelId, String>,
    pending: Arc<tokio::sync::Semaphore>,
}
fn frame_bytes(id: Option<u64>, mut event: Value) -> Result<Vec<u8>, russh::Error> {
    event["v"] = json!(API_VERSION_MAJOR);
    if let Some(id) = id {
        event["reply_to"] = json!(id);
    }
    // Validate fixtures against the same protocol the real SDK consumes.
    let checked: ServerFrame =
        serde_json::from_value(event).map_err(|_| russh::Error::Disconnect)?;
    let mut bytes = serde_json::to_vec(&checked).map_err(|_| russh::Error::Disconnect)?;
    bytes.push(b'\n');
    Ok(bytes)
}
fn delayed_frames(
    handle: server::Handle,
    channel: ChannelId,
    frames: Vec<(Option<u64>, Value)>,
    delay_ms: u64,
    pending: Arc<tokio::sync::Semaphore>,
) -> Result<(), russh::Error> {
    let permit = pending
        .try_acquire_owned()
        .map_err(|_| russh::Error::Disconnect)?;
    let frames = frames
        .into_iter()
        .map(|(id, event)| frame_bytes(id, event))
        .collect::<Result<Vec<_>, _>>()?;
    tokio::spawn(async move {
        let _permit = permit;
        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
        for bytes in frames {
            if handle.data(channel, bytes).await.is_err() {
                break;
            }
        }
    });
    Ok(())
}
fn info(sid: &str) -> Value {
    json!({"session_id":sid,"status":"idle","title":"Latency fixture","working_dir":"/fixture"})
}
fn enrollment_key(command: &[u8]) -> Option<russh::keys::PublicKey> {
    let command = std::str::from_utf8(command).ok()?;
    let prefix =
        "umask 077; mkdir -p \"$HOME/.ssh\" && touch \"$HOME/.ssh/authorized_keys\" && (grep -qF '";
    let rest = command.strip_prefix(prefix)?;
    let (blob, _) = rest.split_once('\'')?;
    if blob.is_empty()
        || blob.len() > 128
        || !blob
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "+/=".contains(c))
    {
        return None;
    }
    let line = format!("ssh-ed25519 {blob} jcode-device");
    let expected = format!(
        "{prefix}{blob}' \"$HOME/.ssh/authorized_keys\" || echo '{line}' >> \"$HOME/.ssh/authorized_keys\")"
    );
    if command != expected {
        return None;
    }
    russh::keys::PublicKey::from_openssh(&line).ok()
}
impl server::Handler for Harness {
    type Error = russh::Error;
    async fn auth_publickey(
        &mut self,
        user: &str,
        key: server::VerifiedPublicKey<'_>,
    ) -> Result<server::Auth, Self::Error> {
        tokio::time::sleep(Duration::from_millis(self.state.options.auth)).await;
        let accepted = user == "fixture"
            && self
                .state
                .keys
                .lock()
                .unwrap()
                .iter()
                .any(|k| k.key_data() == key.public_key.key_data());
        self.state.stage(if accepted {
            "ssh_authenticated"
        } else {
            "ssh_auth_rejected"
        });
        Ok(if accepted {
            server::Auth::Accept
        } else {
            server::Auth::reject()
        })
    }
    async fn channel_open_session(
        &mut self,
        channel: russh::Channel<server::Msg>,
        reply: server::ChannelOpenHandle,
        _: &mut server::Session,
    ) -> Result<(), Self::Error> {
        if self.buffers.len() >= 16 {
            return Err(russh::Error::Disconnect);
        }
        self.buffers.insert(channel.id(), Vec::new());
        self.state.stage("ssh_channel");
        reply.accept().await;
        Ok(())
    }
    async fn exec_request(
        &mut self,
        channel: ChannelId,
        command: &[u8],
        session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        if let Some(key) = enrollment_key(command) {
            {
                let mut keys = self.state.keys.lock().unwrap();
                if !keys.contains(&key) {
                    if keys.len() >= 64 {
                        session.channel_failure(channel)?;
                        return session.close(channel);
                    }
                    keys.push(key);
                }
            }
            self.state.stage("device_key_enrolled");
            session.channel_success(channel)?;
            session.exit_status_request(channel, 0)?;
            session.eof(channel)?;
            return session.close(channel);
        }
        if command.len() > 4096 || !command.ends_with(b"jcode --no-update api --stdio") {
            session.channel_failure(channel)?;
            return session.close(channel);
        }
        self.state.stage("ssh_exec");
        session.channel_success(channel)
    }
    async fn data(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        let buffer = self.buffers.entry(channel).or_default();
        if buffer.len().saturating_add(data.len()) > LIMIT {
            return Err(russh::Error::Disconnect);
        }
        buffer.extend_from_slice(data);
        loop {
            let buffer = self.buffers.get_mut(&channel).unwrap();
            let Some(end) = buffer.iter().position(|b| *b == b'\n') else {
                break;
            };
            let line: Vec<_> = buffer.drain(..=end).collect();
            let req: Value = serde_json::from_slice(&line).map_err(|_| russh::Error::Disconnect)?;
            let id = req["id"].as_u64();
            let mut delay_ms = 0;
            let sid = req["session_id"]
                .as_str()
                .unwrap_or("fixture-resume")
                .to_owned();
            if sid.len() > 256 {
                return Err(russh::Error::Disconnect);
            }
            let event = match req["req"].as_str().unwrap_or("") {
                "hello" => {
                    self.state.hellos.fetch_add(1, Ordering::Relaxed);
                    self.state.stage("api_hello");
                    delay_ms = self.state.options.hello;
                    json!({"ev":"hello_ok","version":API_VERSION_MAJOR,"server":"mobile-latency-fixture"})
                }
                "create_session" | "attach_session" => {
                    self.state.stage(if req["req"] == "create_session" {
                        "api_create"
                    } else {
                        "api_attach"
                    });
                    let sid = if req["req"] == "create_session" {
                        format!(
                            "fixture-{}",
                            self.state.next_session.fetch_add(1, Ordering::Relaxed)
                        )
                    } else {
                        sid
                    };
                    self.attached.insert(channel, sid.clone());
                    delayed_frames(
                        session.handle(),
                        channel,
                        vec![
                            (id, json!({"ev":"attached","session":info(&sid)})),
                            (
                                None,
                                json!({"ev":"runtime_info","session_id":sid,"provider":"fixture","model":"fixture-model","routes":[]}),
                            ),
                        ],
                        self.state.options.attach,
                        self.pending.clone(),
                    )?;
                    continue;
                }
                "get_history" => {
                    self.state.stage("api_history");
                    delay_ms = self.state.options.history;
                    let messages: Vec<_> = (0..self.state.options.size)
                        .map(|i| {
                            let mut content = format!("Fixture message {i}");
                            content.push_str(&" x".repeat(self.state.options.history_bytes / 2));
                            content.truncate(self.state.options.history_bytes);
                            if i + 1 == self.state.options.size {
                                content.push_str(&format!(" SIMULATOR FRESH {sid}"));
                            }
                            json!({"role":if i%2==0 {"user"} else {"assistant"},"content":content})
                        })
                        .collect();
                    json!({"ev":"history","session_id":sid,"messages":messages})
                }
                "list_models" => {
                    self.state.stage("api_models");
                    json!({"ev":"models","session_id":sid,"models":["fixture-model"],"current":"fixture-model"})
                }
                "get_runtime_info" => {
                    self.state.stage("api_runtime");
                    json!({"ev":"runtime_info","session_id":sid,"model":"fixture-model","routes":[]})
                }
                "get_todo_state" => {
                    self.state.stage("api_todo");
                    json!({"ev":"todo_state","session_id":sid,"state":{}})
                }
                "list_sessions" => {
                    self.state.stage("api_sessions");
                    json!({"ev":"sessions","sessions":[info("fixture-resume")]})
                }
                "ping" => {
                    self.state.pings.fetch_add(1, Ordering::Relaxed);
                    self.state.stage("api_ping");
                    json!({"ev":"pong"})
                }
                _ => {
                    self.state.stage("api_unsupported");
                    json!({"ev":"error","code":"unknown_request","message":"Not implemented by offline latency fixture"})
                }
            };
            // A real sshd runs each API process independently. Never sleep in
            // this connection-wide callback: a spare channel's slow hello must
            // not stall history or pings on an already attached channel.
            delayed_frames(
                session.handle(),
                channel,
                vec![(id, event)],
                delay_ms,
                self.pending.clone(),
            )?;
        }
        Ok(())
    }
    async fn channel_close(
        &mut self,
        channel: ChannelId,
        session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        self.buffers.remove(&channel);
        self.attached.remove(&channel);
        self.state.stage("ssh_channel_closed");
        session.close(channel)
    }
}
async fn http(
    mut stream: tokio::net::TcpStream,
    state: Arc<State>,
    ssh_port: u16,
    pin: String,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 4096];
    let (header_end, length) = loop {
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            return Ok(());
        }
        bytes.extend_from_slice(&chunk[..n]);
        if bytes.len() > LIMIT {
            return Err("HTTP request too large".into());
        }
        if let Some(pos) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
            let header = std::str::from_utf8(&bytes[..pos])?;
            let length = header
                .lines()
                .skip(1)
                .find_map(|l| {
                    let (k, v) = l.split_once(':')?;
                    k.eq_ignore_ascii_case("content-length")
                        .then(|| v.trim().parse::<usize>().ok())
                        .flatten()
                })
                .unwrap_or(0);
            if length > LIMIT - pos - 4 {
                return Err("HTTP body too large".into());
            }
            break (pos + 4, length);
        }
        if bytes.len() > 16384 {
            return Err("HTTP headers too large".into());
        }
    };
    while bytes.len() < header_end + length {
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            return Err("short HTTP body".into());
        }
        bytes.extend_from_slice(&chunk[..n]);
        if bytes.len() > LIMIT {
            return Err("HTTP request too large".into());
        }
    }
    let header = std::str::from_utf8(&bytes[..header_end])?;
    let authorized = header.lines().any(|l| {
        l.split_once(':').is_some_and(|(k, v)| {
            k.eq_ignore_ascii_case("authorization") && v.trim() == format!("Bearer {TOKEN}")
        })
    });
    let (status, reply) = if !authorized {
        (
            "401 Unauthorized",
            json!({"error":"fixture account required"}),
        )
    } else if header
        .lines()
        .next()
        .unwrap_or("")
        .starts_with("GET /auth/verify ")
    {
        state.stage("account_profile");
        (
            "200 OK",
            json!({"given_name":"Offline","name":"Offline latency fixture","email":"latency-fixture@example.invalid"}),
        )
    } else if header
        .lines()
        .next()
        .unwrap_or("")
        .starts_with("POST /cloud/host/connect ")
    {
        let request: Value = serde_json::from_slice(&bytes[header_end..header_end + length])?;
        let public = request["public_key"].as_str().ok_or("missing public_key")?;
        if public.len() > 4096 {
            return Err("public key too long".into());
        }
        let key = russh::keys::PublicKey::from_openssh(public)?;
        {
            let mut keys = state.keys.lock().unwrap();
            if !keys.contains(&key) {
                if keys.len() >= 64 {
                    return Err("too many authorized fixture keys".into());
                }
                keys.push(key);
            }
        }
        let poll = state.polls.fetch_add(1, Ordering::Relaxed);
        state.stage("control_plane_connect");
        tokio::time::sleep(Duration::from_millis(state.options.cp)).await;
        if poll < state.options.boot {
            ("200 OK", json!({"ready":false,"state":"starting"}))
        } else {
            (
                "200 OK",
                json!({"ready":true,"state":"running","address":"127.0.0.1","port":ssh_port,"user":"fixture","host_keys":[pin]}),
            )
        }
    } else {
        (
            "404 Not Found",
            json!({"error":"offline fixture endpoint not found"}),
        )
    };
    let body = serde_json::to_vec(&reply)?;
    stream.write_all(format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",body.len()).as_bytes()).await?;
    stream.write_all(&body).await?;
    stream.shutdown().await?;
    Ok(())
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let options = Options::parse()?;
    // The desktop core remembers device keys in XDG_DATA_HOME. Keep self-test
    // entirely separate from any real saved account or direct-device key.
    let isolated = if options.self_test {
        let base = std::env::var_os("JCODE_SCRATCH_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let path = base.join(format!(
            "mobile-fixture-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&path)?;
        // SAFETY: this happens before spawning any threads or async runtime.
        unsafe {
            std::env::set_var("XDG_DATA_HOME", &path);
            std::env::set_var("XDG_CACHE_HOME", path.join("cache"));
        }
        Some(path)
    } else {
        None
    };
    let log = options
        .log
        .as_ref()
        .map(|p| {
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(p)
        })
        .transpose()?;
    let state = Arc::new(State {
        options,
        keys: Mutex::new(Vec::new()),
        polls: AtomicUsize::new(0),
        next_session: AtomicUsize::new(1),
        pings: AtomicUsize::new(0),
        hellos: AtomicUsize::new(0),
        started: Instant::now(),
        log: Mutex::new(log),
    });
    let (stop_tx, mut stop_rx) = tokio::sync::mpsc::channel(1);
    let self_test_stop = stop_tx.clone();
    std::thread::spawn(move || {
        use std::io::Read;
        let mut byte = [0u8; 1];
        if std::io::stdin().read(&mut byte).is_ok_and(|n| n > 0) {
            let _ = stop_tx.blocking_send(());
        }
    });
    let result = tokio::runtime::Builder::new_current_thread().enable_all().build()?.block_on(async move {
        let http_listener=tokio::net::TcpListener::bind(("127.0.0.1",state.options.http_port)).await?;
        let ssh_listener=tokio::net::TcpListener::bind(("127.0.0.1",state.options.ssh_port)).await?;
        let host=russh::keys::PrivateKey::random(&mut rand::rng(),russh::keys::Algorithm::Ed25519)?;
        let pin=host.public_key().to_openssh()?;
        let config=Arc::new(server::Config { keys:vec![host], inactivity_timeout:Some(Duration::from_secs(300)), ..Default::default() });
        let ssh_port=ssh_listener.local_addr()?.port();
        state.stage("ready");
        println!("{}",json!({"stage":"ready","api_base":format!("http://{}",http_listener.local_addr()?),"account":{"api_key":TOKEN,"email":"latency-fixture@example.invalid","name":"Offline latency fixture","api_base":format!("http://{}",http_listener.local_addr()?)},"ssh_port":ssh_port}));
        let self_test = if state.options.self_test {
            let base = format!("http://{}",http_listener.local_addr()?);
            let state = state.clone();
            Some(std::thread::spawn(move || {
                let result = smoke(&base,state.clone());
                state.stage(if result.is_ok() { "self_test_passed" } else { "self_test_failed" });
                let _ = self_test_stop.blocking_send(());
                result
            }))
        } else { None };
        let mut tasks=tokio::task::JoinSet::new();
        let (shutdown, _) = tokio::sync::watch::channel(false);
        let deadline=Instant::now()+Duration::from_secs(state.options.duration);
        let mut stop_poll=tokio::time::interval(Duration::from_millis(100));
        loop {
            tokio::select! {
                Some(())=stop_rx.recv()=>break,
                _=stop_poll.tick()=>{ if Instant::now()>=deadline || state.options.stop.as_ref().is_some_and(|p|std::path::Path::new(p).exists()) { break; } },
                result=http_listener.accept()=>{ let (stream,_)=result?; if tasks.len()<32 { let state=state.clone(); let pin=pin.clone(); tasks.spawn(async move { let _=tokio::time::timeout(Duration::from_secs(65),http(stream,state,ssh_port,pin)).await; }); } },
                result=ssh_listener.accept()=>{
                    let (stream,_)=result?; stream.set_nodelay(true)?;
                    if tasks.len()<32 {
                        let config=config.clone(); let state=state.clone(); let mut stopped=shutdown.subscribe();
                        tasks.spawn(async move {
                            tokio::time::sleep(Duration::from_millis(state.options.handshake)).await;
                            let handler=Harness { state,buffers:HashMap::new(),attached:HashMap::new(),pending:Arc::new(tokio::sync::Semaphore::new(64)) };
                            if let Ok(mut session)=server::run_stream(config,stream,handler).await {
                                let handle=session.handle();
                                tokio::select! {
                                    _=&mut session=>{},
                                    _=stopped.changed()=>{
                                        let _=handle.disconnect(russh::Disconnect::ByApplication,"Fixture stopping".into(),"en".into()).await;
                                        let _=tokio::time::timeout(Duration::from_secs(1),session).await;
                                    }
                                }
                            }
                        });
                    }
                },
                Some(_)=tasks.join_next(), if !tasks.is_empty()=>{},
            }
        }
        state.stage("stopping");
        let _ = shutdown.send(true);
        let _ = tokio::time::timeout(Duration::from_secs(2), async { while tasks.join_next().await.is_some() {} }).await;
        tasks.abort_all(); while tasks.join_next().await.is_some() {} state.stage("stopped");
        if let Some(test) = self_test { test.join().map_err(|_| "self-test thread panicked")?.map_err(std::io::Error::other)?; }
        Ok::<(),Box<dyn std::error::Error>>(())
    });
    if let Some(path) = isolated {
        std::fs::remove_dir_all(path)?;
    }
    result
}

fn smoke(base: &str, state: Arc<State>) -> Result<(), String> {
    use jcode_mobile_core::{
        Config, Connection, ConnectionOutput, ConnectionPhase, cloud::Account,
    };
    use jcode_sdk::api::ApiEvent;
    let account: Account = serde_json::from_value(json!({"api_key":TOKEN,"api_base":base}))
        .map_err(|_| "fixture account invalid")?;
    for (attempt, resume) in [
        None,
        Some("fixture-resume".to_string()),
        Some("fixture-direct".to_string()),
    ]
    .into_iter()
    .enumerate()
    {
        let polls_before = state.polls.load(Ordering::Relaxed);
        if attempt == 2 {
            let path = std::path::PathBuf::from(
                std::env::var_os("XDG_DATA_HOME").ok_or("missing isolated data home")?,
            )
            .join("jcode-mobile/cloud-direct.json");
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                let authorized = std::fs::read(&path)
                    .ok()
                    .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
                    .is_some_and(|v| v["authorized"] == true && v["target"].is_object());
                if authorized {
                    break;
                }
                if Instant::now() >= deadline {
                    return Err("self-test device enrollment not persisted".into());
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            jcode_mobile_core::cloud::reset_in_memory_cache_for_benchmark();
        }
        let mut config = Config::cloud(account.clone());
        config.max_reconnect_attempts = Some(0);
        let (connection, output) = Connection::start_with_channel(config, resume);
        let result = (|| {
            let deadline = Instant::now() + Duration::from_secs(120);
            let (mut history, mut runtime, mut models, mut todo, mut sessions) =
                (false, false, false, false, false);
            loop {
                let remaining = deadline.saturating_duration_since(Instant::now());
                match output
                    .recv_timeout(remaining)
                    .map_err(|_| "self-test timed out")?
                {
                    ConnectionOutput::Event(ApiEvent::History { messages, .. }) => {
                        if messages.len() != state.options.size {
                            return Err("self-test history size mismatch".to_string());
                        }
                        history = true;
                    }
                    ConnectionOutput::Event(ApiEvent::RuntimeInfo { .. }) => runtime = true,
                    ConnectionOutput::Event(ApiEvent::Models { .. }) => models = true,
                    ConnectionOutput::Event(ApiEvent::TodoState { .. }) => todo = true,
                    ConnectionOutput::Event(ApiEvent::Sessions { .. }) => sessions = true,
                    ConnectionOutput::Phase(ConnectionPhase::Failed { .. }) => {
                        return Err("self-test connection failed".to_string());
                    }
                    _ => {}
                }
                if history && runtime && models && todo && sessions {
                    let pings = state.pings.load(Ordering::Relaxed);
                    connection.send(|id| jcode_mobile_core::Request::Ping { id });
                    while state.pings.load(Ordering::Relaxed) == pings {
                        if Instant::now() >= deadline {
                            return Err("self-test ping timed out".into());
                        }
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    return Ok(());
                }
            }
        })();
        connection.stop();
        result?;
        if attempt == 2 {
            if state.polls.load(Ordering::Relaxed) != polls_before {
                return Err("self-test direct reconnect used control plane".into());
            }
            state.stage("self_test_direct_passed");
        }
    }
    if state.options.hello >= 300 && state.options.history <= state.options.hello / 4 {
        concurrency_smoke(&account, &state)?;
    }
    Ok(())
}

fn concurrency_smoke(
    account: &jcode_mobile_core::Account,
    state: &Arc<State>,
) -> Result<(), String> {
    use jcode_mobile_core::cloud_ssh::{CloudSsh, CloudSshKey};
    let key = CloudSshKey::generate("fixture-concurrency").map_err(|_| "concurrency key failed")?;
    let reply = jcode_mobile_core::cloud::client()
        .request_host_connect(account, key.public_openssh())
        .map_err(|_| "concurrency control plane failed")?;
    let target = jcode_sdk::cloud::CloudTarget::from_reply(&reply)
        .map_err(|_| "concurrency target failed")?;
    let ssh = CloudSsh::connect(target, key).map_err(|_| "concurrency SSH failed")?;
    let client = ssh
        .open_client()
        .map_err(|_| "concurrency initial hello failed")?;
    let hellos = state.hellos.load(Ordering::Relaxed);
    let spare = std::thread::spawn(move || {
        ssh.open_client()
            .map(|_| ())
            .map_err(|_| "concurrency spare hello failed")
    });
    let deadline = Instant::now() + Duration::from_secs(10);
    while state.hellos.load(Ordering::Relaxed) == hellos {
        if Instant::now() >= deadline {
            return Err("concurrency spare hello not received".into());
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    let started = Instant::now();
    let result = client
        .request(jcode_sdk::ApiRequest::GetHistory {
            session_id: "fixture-concurrency".into(),
        })
        .map_err(|_| "concurrency history failed");
    let elapsed = started.elapsed();
    spare
        .join()
        .map_err(|_| "concurrency spare thread panicked")?
        .map_err(str::to_owned)?;
    result?;
    if elapsed >= Duration::from_millis(state.options.hello * 3 / 4) {
        return Err("fixture history blocked behind unrelated channel hello".into());
    }
    state.stage("self_test_cross_channel_passed");
    Ok(())
}
