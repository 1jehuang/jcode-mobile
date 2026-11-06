//! One pinned, authenticated SSH master, with an independent API bridge per
//! client. The caller owns account-scoped caching. This module never caches
//! targets or keys globally and never trusts an unpinned host key.
//!
//! A private runtime keeps this synchronous API usable on iOS. Active API
//! clients retain the master, so evicting an idle-cache entry cannot interrupt
//! another session. Dropping the final master/client owner closes SSH.

use jcode_sdk::cloud::CloudTarget;
use jcode_sdk::{ConnectOptions, Error, ErrorKind, JcodeClient, Result, Transport};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

const AUTH_TIMEOUT: Duration = Duration::from_secs(15);
const OPEN_TIMEOUT: Duration = Duration::from_secs(2);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const REMOTE_COMMAND: &str = "PATH=\"$HOME/.local/bin:$HOME/.cargo/bin:$PATH\"; export PATH; exec jcode --no-update api --stdio";

/// An ephemeral Ed25519 key suitable for registering with the Cloud control plane.
#[derive(Clone)]
pub struct CloudSshKey {
    private: Arc<russh::keys::PrivateKey>,
    public_openssh: String,
}

impl CloudSshKey {
    pub fn generate(comment: &str) -> Result<Self> {
        let private =
            russh::keys::PrivateKey::random(&mut rand::rng(), russh::keys::Algorithm::Ed25519)
                .map_err(|e| failed(format!("Could not generate a connection key: {e}")))?;
        let mut public_openssh = private
            .public_key()
            .to_openssh()
            .map_err(|e| Error::new(ErrorKind::InvalidOption, e.to_string()))?;
        let comment: String = comment
            .chars()
            .filter(|c| c.is_ascii_graphic())
            .take(64)
            .collect();
        if !comment.is_empty() {
            public_openssh.push(' ');
            public_openssh.push_str(&comment);
        }
        Ok(Self {
            private: Arc::new(private),
            public_openssh,
        })
    }

    pub fn public_openssh(&self) -> &str {
        &self.public_openssh
    }
}

impl std::fmt::Debug for CloudSshKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CloudSshKey")
            .field("public_openssh", &self.public_openssh)
            .finish_non_exhaustive()
    }
}

/// Cloneable ownership of one authenticated connection, not an attached session.
#[derive(Clone)]
pub struct CloudSsh(Arc<Master>);

struct Master {
    commands: tokio::sync::mpsc::UnboundedSender<Command>,
    closed: Arc<AtomicBool>,
}

impl Drop for Master {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Stop);
    }
}

impl std::fmt::Debug for CloudSsh {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CloudSsh")
            .field("closed", &self.is_closed())
            .finish()
    }
}

enum Command {
    Open {
        deadline: Instant,
        reply: mpsc::Sender<Result<ApiChannel>>,
    },
    Stop,
}

struct ApiChannel {
    incoming: mpsc::Receiver<Vec<u8>>,
    outgoing: tokio::sync::mpsc::UnboundedSender<Outgoing>,
}

enum Outgoing {
    Data(Vec<u8>),
    Close,
}

fn failed(message: impl Into<String>) -> Error {
    Error::new(ErrorKind::ConnectFailed, message.into())
}

fn startup_timeout() -> Error {
    Error::new(
        ErrorKind::StartupTimeout,
        "SSH API channel startup timed out",
    )
}

fn pins(target: &CloudTarget) -> Result<Vec<russh::keys::PublicKey>> {
    if target.host_keys.is_empty() {
        return Err(Error::new(
            ErrorKind::InvalidOption,
            "no pinned SSH host keys",
        ));
    }
    target
        .host_keys
        .iter()
        .map(|line| {
            russh::keys::PublicKey::from_openssh(line).map_err(|e| {
                Error::new(
                    ErrorKind::InvalidOption,
                    format!("invalid pinned SSH host key: {e}"),
                )
            })
        })
        .collect()
}

struct PinnedHost {
    keys: Vec<russh::keys::PublicKey>,
    mismatch: Arc<AtomicBool>,
}

impl russh::client::Handler for PinnedHost {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        key: &russh::keys::PublicKeyOrCertificate,
    ) -> std::result::Result<bool, Self::Error> {
        let trusted = match key {
            russh::keys::PublicKeyOrCertificate::PublicKey { key, .. } => {
                self.keys.iter().any(|pin| pin.key_data() == key.key_data())
            }
            russh::keys::PublicKeyOrCertificate::Certificate(_) => false,
        };
        if !trusted {
            self.mismatch.store(true, Ordering::Relaxed);
        }
        Ok(trusted)
    }
}

impl CloudSsh {
    /// TCP connect, pinned host handshake and public-key authentication, with
    /// one 15-second deadline. No API channel is opened until `open_client`.
    pub fn connect(target: CloudTarget, key: CloudSshKey) -> Result<Self> {
        let keys = pins(&target)?;
        let (commands, mut requests) = tokio::sync::mpsc::unbounded_channel();
        let (ready, connected) = mpsc::channel();
        let closed = Arc::new(AtomicBool::new(false));
        let worker_closed = closed.clone();
        std::thread::Builder::new().name("jcode-cloud-ssh".into()).spawn(move || {
            // Also mark closed if runtime construction fails or the worker unwinds.
            struct MarkClosed(Arc<AtomicBool>);
            impl Drop for MarkClosed {
                fn drop(&mut self) { self.0.store(true, Ordering::Release); }
            }
            let _mark_closed = MarkClosed(worker_closed);
            let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                Ok(runtime) => runtime,
                Err(e) => { let _ = ready.send(Err(failed(e.to_string()))); return; }
            };
            runtime.block_on(async move {
                let mismatch = Arc::new(AtomicBool::new(false));
                let authenticated = tokio::time::timeout(AUTH_TIMEOUT, async {
                    let config = Arc::new(russh::client::Config {
                        // API hello and commands are small, latency-sensitive
                        // frames. Do not wait for Nagle/delayed-ACK batching.
                        nodelay: true,
                        keepalive_interval: Some(Duration::from_secs(20)),
                        ..Default::default()
                    });
                    let mut session = russh::client::connect(
                        config,
                        (target.address, target.port),
                        PinnedHost { keys, mismatch: mismatch.clone() },
                    ).await.map_err(|e| failed(format!("SSH connect failed: {e}")))?;
                    let auth = session.authenticate_publickey(
                        &target.user,
                        russh::keys::PrivateKeyWithHashAlg::new(key.private, None),
                    ).await.map_err(|e| failed(format!("SSH authentication failed: {e}")))?;
                    if !auth.success() {
                        return Err(failed("The host rejected the SSH key"));
                    }
                    Ok(session)
                }).await;
                let session = match authenticated {
                    Ok(Ok(session)) => Arc::new(session),
                    result => {
                        let error = if mismatch.load(Ordering::Relaxed) {
                            Error::new(ErrorKind::HostKeyMismatch, "the host's SSH key does not match the pinned key")
                        } else {
                            match result {
                                Ok(Err(e)) => e,
                                Err(_) => Error::new(ErrorKind::StartupTimeout, "SSH authentication timed out"),
                                Ok(Ok(_)) => unreachable!(),
                            }
                        };
                        let _ = ready.send(Err(error));
                        return;
                    }
                };
                if ready.send(Ok(())).is_err() { return; }
                loop {
                    tokio::select! {
                        request = requests.recv() => match request {
                            Some(Command::Open { deadline, reply }) => {
                                let session = session.clone();
                                tokio::spawn(async move {
                                    let result = tokio::time::timeout_at(
                                        tokio::time::Instant::from_std(deadline),
                                        async {
                                            let channel = session.channel_open_session().await
                                                .map_err(|e| failed(format!("SSH channel open failed: {e}")))?;
                                            channel.exec(true, REMOTE_COMMAND).await
                                                .map_err(|e| failed(format!("SSH API exec failed: {e}")))?;
                                            Ok(channel)
                                        },
                                    ).await;
                                    match result {
                                        Ok(Ok(channel)) => {
                                            let (incoming, rx) = mpsc::channel();
                                            let (outgoing, tx) = tokio::sync::mpsc::unbounded_channel();
                                            if reply.send(Ok(ApiChannel { incoming: rx, outgoing })).is_ok() {
                                                pump_channel(channel, incoming, tx).await;
                                            } else {
                                                let _ = channel.close().await;
                                            }
                                        }
                                        Ok(Err(e)) => { let _ = reply.send(Err(e)); }
                                        Err(_) => { let _ = reply.send(Err(startup_timeout())); }
                                    }
                                });
                            }
                            Some(Command::Stop) | None => break,
                        },
                        _ = tokio::time::sleep(Duration::from_millis(250)) => {
                            if session.is_closed() { break; }
                        }
                    }
                }
                let _ = tokio::time::timeout(
                    Duration::from_secs(2),
                    session.disconnect(russh::Disconnect::ByApplication, "", "en"),
                ).await;
                // Runtime teardown aborts remaining channel pumps, releasing
                // their incoming senders so every SDK reader sees EOF.
            });
        }).map_err(|e| failed(e.to_string()))?;
        match connected.recv_timeout(AUTH_TIMEOUT) {
            Ok(Ok(())) => Ok(Self(Arc::new(Master { commands, closed }))),
            Ok(Err(e)) => Err(e),
            Err(mpsc::RecvTimeoutError::Timeout) => Err(Error::new(
                ErrorKind::StartupTimeout,
                "SSH authentication timed out",
            )),
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                Err(failed("SSH runtime exited during authentication"))
            }
        }
    }

    pub fn is_closed(&self) -> bool {
        self.0.closed.load(Ordering::Acquire) || self.0.commands.is_closed()
    }

    /// Open a fresh, independently attached `jcode api --stdio` channel.
    /// Channel creation, exec and API hello share a two-second deadline.
    pub fn open_client(&self) -> Result<JcodeClient> {
        self.open_client_timeout(OPEN_TIMEOUT)
    }

    /// Override the total startup deadline without shortening normal requests.
    pub fn open_client_timeout(&self, timeout: Duration) -> Result<JcodeClient> {
        let deadline = Instant::now().checked_add(timeout).ok_or_else(|| {
            Error::new(
                ErrorKind::InvalidOption,
                "SSH API startup timeout is too large",
            )
        })?;
        if timeout.is_zero() {
            return Err(startup_timeout());
        }
        let (reply, result) = mpsc::channel();
        self.0
            .commands
            .send(Command::Open { deadline, reply })
            .map_err(|_| Error::new(ErrorKind::Disconnected, "SSH master closed"))?;
        let channel = match result.recv_timeout(deadline.saturating_duration_since(Instant::now()))
        {
            Ok(result) => result?,
            Err(mpsc::RecvTimeoutError::Timeout) => return Err(startup_timeout()),
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err(Error::new(ErrorKind::Disconnected, "SSH master closed"));
            }
        };
        let transport = ApiTransport {
            channel,
            master: self.clone(),
        };
        let shutdown = transport
            .shutdown_handle()
            .expect("API channel has shutdown handle");
        // The SDK uses request_timeout for hello too. Bound hello externally,
        // closing only this channel if late, while retaining 30s request timeouts.
        let (ready, connected) = mpsc::channel();
        std::thread::Builder::new()
            .name("jcode-cloud-api-hello".into())
            .spawn(move || {
                let result = JcodeClient::connect_with(
                    Box::new(transport),
                    ConnectOptions {
                        socket_path: Some(std::path::PathBuf::new()),
                        client_name: concat!("jcode-ios/", env!("CARGO_PKG_VERSION")).into(),
                        ensure_runtime: false,
                        supports_user_response: false,
                        request_timeout: Some(REQUEST_TIMEOUT),
                    },
                );
                let _ = ready.send(result);
            })
            .map_err(|e| failed(e.to_string()))?;
        match connected.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(result) => result,
            Err(error) => {
                shutdown();
                match error {
                    mpsc::RecvTimeoutError::Timeout => Err(startup_timeout()),
                    mpsc::RecvTimeoutError::Disconnected => Err(failed("API hello worker exited")),
                }
            }
        }
    }
}

async fn pump_channel(
    mut channel: russh::Channel<russh::client::Msg>,
    incoming: mpsc::Sender<Vec<u8>>,
    mut outgoing: tokio::sync::mpsc::UnboundedReceiver<Outgoing>,
) {
    loop {
        tokio::select! {
            event = channel.wait() => match event {
                Some(russh::ChannelMsg::Data { data }) => {
                    if incoming.send(data.to_vec()).is_err() { break; }
                }
                Some(russh::ChannelMsg::Eof | russh::ChannelMsg::Close | russh::ChannelMsg::Failure) | None => break,
                Some(_) => {}
            },
            output = outgoing.recv() => match output {
                Some(Outgoing::Data(bytes)) => {
                    if channel.data(&bytes[..]).await.is_err() { break; }
                }
                Some(Outgoing::Close) | None => break,
            }
        }
    }
    // Closing one bridge must never disconnect the shared master.
    let _ = tokio::time::timeout(Duration::from_secs(1), channel.close()).await;
}

struct ApiTransport {
    channel: ApiChannel,
    master: CloudSsh,
}

struct ChannelReader {
    incoming: mpsc::Receiver<Vec<u8>>,
    buffered: Vec<u8>,
    position: usize,
    _master: CloudSsh,
}

impl Read for ChannelReader {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if output.is_empty() {
            return Ok(0);
        }
        while self.position == self.buffered.len() {
            match self.incoming.recv() {
                Ok(bytes) => {
                    self.buffered = bytes;
                    self.position = 0;
                }
                Err(_) => return Ok(0),
            }
        }
        let len = output.len().min(self.buffered.len() - self.position);
        output[..len].copy_from_slice(&self.buffered[self.position..self.position + len]);
        self.position += len;
        Ok(len)
    }
}

struct ChannelWriter {
    outgoing: tokio::sync::mpsc::UnboundedSender<Outgoing>,
    _master: CloudSsh,
}

impl Write for ChannelWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.outgoing
            .send(Outgoing::Data(bytes.to_vec()))
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "SSH API channel closed"))?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Transport for ApiTransport {
    fn shutdown_handle(&self) -> Option<Arc<dyn Fn() + Send + Sync>> {
        let outgoing = self.channel.outgoing.clone();
        Some(Arc::new(move || {
            let _ = outgoing.send(Outgoing::Close);
        }))
    }

    fn split(self: Box<Self>) -> Result<(Box<dyn BufRead + Send>, Box<dyn Write + Send>)> {
        let reader = ChannelReader {
            incoming: self.channel.incoming,
            buffered: Vec::new(),
            position: 0,
            _master: self.master.clone(),
        };
        let writer = ChannelWriter {
            outgoing: self.channel.outgoing,
            _master: self.master,
        };
        Ok((Box::new(BufReader::new(reader)), Box::new(writer)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jcode_sdk::api::{API_VERSION_MAJOR, ApiEvent, ServerFrame};
    use serde_json::{Value, json};
    use std::collections::HashMap;
    use std::sync::atomic::AtomicUsize;

    #[derive(Default)]
    struct Counts {
        authenticated: AtomicUsize,
        channels: AtomicUsize,
    }

    struct Harness {
        authorized: russh::keys::PublicKey,
        counts: Arc<Counts>,
        buffers: HashMap<russh::ChannelId, Vec<u8>>,
        attached: HashMap<russh::ChannelId, String>,
        withhold_first_hello: bool,
        first_channel: Option<russh::ChannelId>,
        ping_delay: Duration,
    }

    fn frame(
        session: &mut russh::server::Session,
        channel: russh::ChannelId,
        id: Option<u64>,
        mut event: Value,
    ) -> std::result::Result<(), russh::Error> {
        event["v"] = json!(API_VERSION_MAJOR);
        if let Some(id) = id {
            event["reply_to"] = json!(id);
        }
        let checked: ServerFrame = serde_json::from_value(event).unwrap();
        let mut bytes = serde_json::to_vec(&checked).unwrap();
        bytes.push(b'\n');
        session.data(channel, bytes)
    }

    impl russh::server::Handler for Harness {
        type Error = russh::Error;

        async fn auth_publickey(
            &mut self,
            user: &str,
            key: russh::server::VerifiedPublicKey<'_>,
        ) -> std::result::Result<russh::server::Auth, Self::Error> {
            Ok(
                if user == "test" && key.public_key.key_data() == self.authorized.key_data() {
                    self.counts.authenticated.fetch_add(1, Ordering::Relaxed);
                    russh::server::Auth::Accept
                } else {
                    russh::server::Auth::reject()
                },
            )
        }

        async fn channel_open_session(
            &mut self,
            channel: russh::Channel<russh::server::Msg>,
            reply: russh::server::ChannelOpenHandle,
            _session: &mut russh::server::Session,
        ) -> std::result::Result<(), Self::Error> {
            self.counts.channels.fetch_add(1, Ordering::Relaxed);
            self.first_channel.get_or_insert(channel.id());
            reply.accept().await;
            Ok(())
        }

        async fn exec_request(
            &mut self,
            channel: russh::ChannelId,
            command: &[u8],
            session: &mut russh::server::Session,
        ) -> std::result::Result<(), Self::Error> {
            assert_eq!(command, REMOTE_COMMAND.as_bytes());
            session.channel_success(channel)
        }

        async fn data(
            &mut self,
            channel: russh::ChannelId,
            data: &[u8],
            session: &mut russh::server::Session,
        ) -> std::result::Result<(), Self::Error> {
            self.buffers
                .entry(channel)
                .or_default()
                .extend_from_slice(data);
            loop {
                let buffer = self.buffers.get_mut(&channel).unwrap();
                let Some(end) = buffer.iter().position(|b| *b == b'\n') else {
                    break;
                };
                let line: Vec<_> = buffer.drain(..=end).collect();
                let request: Value = serde_json::from_slice(&line).unwrap();
                let id = request["id"].as_u64();
                match request["req"].as_str().unwrap() {
                    "hello" => {
                        if self.withhold_first_hello && self.first_channel == Some(channel) {
                            continue;
                        }
                        frame(
                            session,
                            channel,
                            id,
                            json!({"ev":"hello_ok","version":API_VERSION_MAJOR,"server":"ssh-test"}),
                        )?;
                    }
                    "attach_session" => {
                        let sid = request["session_id"].as_str().unwrap().to_string();
                        self.attached.insert(channel, sid.clone());
                        frame(
                            session,
                            channel,
                            id,
                            json!({"ev":"attached","session":{"session_id":sid,"status":"idle"}}),
                        )?;
                    }
                    "get_history" => {
                        let sid = self.attached.get(&channel).unwrap();
                        assert_eq!(request["session_id"], *sid);
                        frame(
                            session,
                            channel,
                            id,
                            json!({"ev":"history","session_id":sid,"messages":[{"role":"user","content":sid}]}),
                        )?;
                    }
                    "send_message" => {
                        let sid = self.attached.get(&channel).unwrap();
                        assert_eq!(request["session_id"], *sid);
                        frame(
                            session,
                            channel,
                            None,
                            json!({"ev":"text_delta","session_id":sid,"text":request["content"]}),
                        )?;
                    }
                    "ping" => {
                        tokio::time::sleep(self.ping_delay).await;
                        frame(session, channel, id, json!({"ev":"pong"}))?;
                    }
                    other => panic!("unexpected API request: {other}"),
                }
            }
            Ok(())
        }

        async fn channel_close(
            &mut self,
            channel: russh::ChannelId,
            session: &mut russh::server::Session,
        ) -> std::result::Result<(), Self::Error> {
            self.buffers.remove(&channel);
            self.attached.remove(&channel);
            session.close(channel)
        }
    }

    struct Server {
        counts: Arc<Counts>,
        stopped: mpsc::Receiver<()>,
        thread: std::thread::JoinHandle<()>,
    }

    impl Server {
        fn start(
            withhold_first_hello: bool,
            ping_delay: Duration,
        ) -> (Self, CloudTarget, CloudSshKey) {
            let host = CloudSshKey::generate("host").unwrap();
            let key = CloudSshKey::generate("test-client").unwrap();
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let address = listener.local_addr().unwrap();
            let target = CloudTarget {
                address: address.ip(),
                port: address.port(),
                user: "test".into(),
                host_keys: vec![host.public_openssh().into()],
            };
            let counts = Arc::new(Counts::default());
            let handler = Harness {
                authorized: key.private.public_key().clone(),
                counts: counts.clone(),
                buffers: HashMap::new(),
                attached: HashMap::new(),
                withhold_first_hello,
                first_channel: None,
                ping_delay,
            };
            let (stop, stopped) = mpsc::channel();
            let thread = std::thread::spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap();
                runtime.block_on(async move {
                    let listener = tokio::net::TcpListener::from_std(listener).unwrap();
                    let config = Arc::new(russh::server::Config {
                        keys: vec![(*host.private).clone()],
                        ..Default::default()
                    });
                    tokio::time::timeout(Duration::from_secs(15), async move {
                        let (stream, _) = listener.accept().await.unwrap();
                        // run_stream accepts an existing socket, so configure
                        // TCP_NODELAY here rather than server Config.nodelay.
                        stream.set_nodelay(true).unwrap();
                        // Pin rejection intentionally terminates the handshake.
                        if let Ok(session) =
                            russh::server::run_stream(config, stream, handler).await
                        {
                            let _ = session.await;
                        }
                    })
                    .await
                    .expect("SSH master did not close after its final owner dropped");
                });
                let _ = stop.send(());
            });
            (
                Self {
                    counts,
                    stopped,
                    thread,
                },
                target,
                key,
            )
        }

        fn finish(self) {
            self.stopped
                .recv_timeout(Duration::from_secs(5))
                .expect("SSH master leaked");
            self.thread.join().unwrap();
        }
    }

    #[test]
    fn independent_clients_share_auth_and_survive_other_client_drop() {
        let (server, target, key) = Server::start(false, Duration::ZERO);
        let master = CloudSsh::connect(target, key).unwrap();
        let first = master.open_client().unwrap();
        let second = master.open_client().unwrap();
        first.attach_session("first").unwrap();
        second.attach_session("second").unwrap();
        assert_eq!(first.get_history("first").unwrap()[0].content, "first");
        assert_eq!(second.get_history("second").unwrap()[0].content, "second");
        let events = second.events(Some("second"));
        second
            .send_message("second", "only-second", vec![], None)
            .unwrap();
        assert!(matches!(events.next_timeout(Duration::from_secs(2)),
            Some(ApiEvent::TextDelta { session_id, text, .. }) if session_id == "second" && text == "only-second"));
        assert_eq!(server.counts.authenticated.load(Ordering::Relaxed), 1);
        assert_eq!(server.counts.channels.load(Ordering::Relaxed), 2);
        drop(first);
        second.ping().unwrap();
        // Evicting the cache's master handle must not kill an active session.
        let remaining = master.clone();
        drop(master);
        let third = remaining.open_client().unwrap();
        drop(remaining);
        second.ping().unwrap();
        third.ping().unwrap();
        assert_eq!(server.counts.authenticated.load(Ordering::Relaxed), 1);
        assert_eq!(server.counts.channels.load(Ordering::Relaxed), 3);
        drop(events);
        drop(second);
        drop(third);
        server.finish();
    }

    #[test]
    fn rejects_wrong_empty_and_malformed_pins() {
        let key = CloudSshKey::generate("unused").unwrap();
        let mut target = CloudTarget {
            address: "127.0.0.1".parse().unwrap(),
            port: 1,
            user: "test".into(),
            host_keys: vec![],
        };
        assert_eq!(
            CloudSsh::connect(target.clone(), key.clone())
                .unwrap_err()
                .kind,
            ErrorKind::InvalidOption
        );
        target.host_keys.push("ssh-ed25519 not-base64!".into());
        assert_eq!(
            CloudSsh::connect(target, key).unwrap_err().kind,
            ErrorKind::InvalidOption
        );
        let (server, mut target, key) = Server::start(false, Duration::ZERO);
        target.host_keys = vec![key.public_openssh().into()];
        assert_eq!(
            CloudSsh::connect(target, key).unwrap_err().kind,
            ErrorKind::HostKeyMismatch
        );
        assert_eq!(server.counts.authenticated.load(Ordering::Relaxed), 0);
        server.finish();
    }

    #[test]
    fn hello_deadline_closes_only_its_channel_and_does_not_shorten_requests() {
        let (server, target, key) = Server::start(true, Duration::from_millis(650));
        let master = CloudSsh::connect(target, key).unwrap();
        let zero = master
            .open_client_timeout(Duration::ZERO)
            .err()
            .expect("zero startup budget must fail");
        assert_eq!(zero.kind, ErrorKind::StartupTimeout);
        assert_eq!(server.counts.channels.load(Ordering::Relaxed), 0);
        assert!(!master.is_closed());
        let started = Instant::now();
        let error = master
            .open_client_timeout(Duration::from_millis(500))
            .err()
            .expect("withheld hello must time out");
        assert_eq!(error.kind, ErrorKind::StartupTimeout);
        assert!(started.elapsed() < Duration::from_secs(2));
        let client = master
            .open_client_timeout(Duration::from_millis(500))
            .unwrap();
        // Longer than the configured startup timeout, shorter than the normal
        // request budget: startup watchdogs must not poison successful clients.
        client.ping().unwrap();
        assert_eq!(server.counts.authenticated.load(Ordering::Relaxed), 1);
        assert!(!master.is_closed());
        drop(client);
        drop(master);
        server.finish();
    }
}
