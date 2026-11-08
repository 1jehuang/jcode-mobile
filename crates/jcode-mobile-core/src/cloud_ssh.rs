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
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

const AUTH_TIMEOUT: Duration = Duration::from_secs(15);
// Starting a remote process on a cold or loaded machine is not a liveness
// probe. Keep one bounded allowance across channel open, exec, and API hello.
const OPEN_TIMEOUT: Duration = Duration::from_secs(15);
const CHANNEL_OPEN_TIMEOUT: Duration = Duration::from_secs(2);
// A spare is speculative, not a user-visible cold startup. Do not let an
// abandoned refill keep an evicted master alive for the cold-start allowance.
const SPARE_OPEN_TIMEOUT: Duration = Duration::from_secs(2);
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

    /// Serialize the private key for secure on-device storage only.
    pub(crate) fn private_openssh(&self) -> Option<String> {
        self.private
            .to_openssh(russh::keys::ssh_key::LineEnding::LF)
            .ok()
            .map(|s| s.to_string())
    }

    pub(crate) fn from_private_openssh(pem: &str, comment: &str) -> Option<Self> {
        let private = russh::keys::PrivateKey::from_openssh(pem).ok()?;
        if private.algorithm() != russh::keys::Algorithm::Ed25519 {
            return None;
        }
        let mut public_openssh = private.public_key().to_openssh().ok()?;
        let comment: String = comment.chars().filter(|c| c.is_ascii_graphic()).take(64).collect();
        if !comment.is_empty() {
            public_openssh.push(' ');
            public_openssh.push_str(&comment);
        }
        Some(Self { private: Arc::new(private), public_openssh })
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

/// A single unattached, handshaken API channel per cached SSH master.
/// Taking a spare transfers ownership, never shares a session between tabs.
/// Refilling is best-effort and never delays a foreground connection.
#[derive(Clone)]
pub(crate) struct PreparedSsh(Arc<PreparedInner>);

struct PreparedInner {
    ssh: CloudSsh,
    state: Mutex<PreparedState>,
}

#[derive(Default)]
struct PreparedState {
    spare: Option<(Instant, JcodeClient)>,
    filling: bool,
}

const SPARE_MAX_AGE: Duration = Duration::from_secs(30);

impl PreparedSsh {
    pub(crate) fn new(ssh: CloudSsh) -> Self {
        Self(Arc::new(PreparedInner {
            ssh,
            state: Mutex::new(PreparedState::default()),
        }))
    }

    pub(crate) fn open_client(&self) -> Result<JcodeClient> {
        self.open_client_timeout(OPEN_TIMEOUT)
    }

    pub(crate) fn is_closed(&self) -> bool {
        self.0.ssh.is_closed()
    }

    /// A caller may use a shorter budget for a speculative cached attempt.
    /// The budget applies only when no usable prepared channel is available.
    /// Timing out a channel does not imply that the shared master is dead.
    pub(crate) fn open_client_timeout(&self, timeout: Duration) -> Result<JcodeClient> {
        let spare = self.0.state.lock().unwrap().spare.take();
        let client = match spare {
            Some((created, client))
                if created.elapsed() < SPARE_MAX_AGE
                    && !client.is_closed()
                    && !self.0.ssh.is_closed() =>
            {
                Ok(client)
            }
            _ => self.0.ssh.open_client_timeout(timeout),
        }?;
        self.refill();
        Ok(client)
    }

    /// After the app was suspended, sockets are often dead while still
    /// looking open. Discard the spare and prove the master with a fresh
    /// channel. Returns false when the master should be thrown away.
    pub(crate) fn revalidate(&self) -> bool {
        let started = Instant::now();
        drop(self.0.state.lock().unwrap().spare.take());
        if self.0.ssh.is_closed() {
            return false;
        }
        match self.0.ssh.open_client_timeout(SPARE_OPEN_TIMEOUT) {
            Ok(client) => {
                let mut state = self.0.state.lock().unwrap();
                if state.spare.is_none() {
                    state.spare = Some((Instant::now(), client));
                }
                trace("master revalidated", started);
                true
            }
            Err(_) => {
                // A suspended socket usually times out rather than reporting
                // Disconnected. Any failure here means the master is unproven,
                // and keeping it would make the next tap stall then go cold.
                trace("master revalidation failed", started);
                false
            }
        }
    }

    pub(crate) fn refill(&self) {
        {
            let mut state = self.0.state.lock().unwrap();
            if state.filling || state.spare.is_some() || self.0.ssh.is_closed() {
                return;
            }
            state.filling = true;
        }
        // Do not retain the cache owner in the worker. Sign-out or cache
        // eviction drops the spare immediately and discards late refills.
        let owner = Arc::downgrade(&self.0);
        let ssh = self.0.ssh.clone();
        let spawn = std::thread::Builder::new()
            .name("jcode-cloud-api-spare".into())
            .spawn(move || {
                let client = ssh.open_client_timeout(SPARE_OPEN_TIMEOUT);
                if let Some(owner) = owner.upgrade() {
                    let mut state = owner.state.lock().unwrap();
                    state.filling = false;
                    if let Ok(client) = client {
                        state.spare = Some((Instant::now(), client));
                    }
                }
            });
        if spawn.is_err() {
            self.0.state.lock().unwrap().filling = false;
        }
    }
}

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
    Inspect {
        command: String,
        reply: mpsc::Sender<std::result::Result<Vec<u8>, String>>,
    },
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

/// Startup stage timings. Always logged (iOS: NSLog and Documents/gpui.log);
/// also on stderr when `JCODE_STARTUP_TRACE` is set. Never logs addresses.
fn trace(stage: &str, since: Instant) {
    crate::timing::stage(stage, since.elapsed());
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
        Self::connect_timeout(target, key, AUTH_TIMEOUT)
    }

    /// Same as [`Self::connect`] with a caller-chosen authentication budget.
    pub fn connect_timeout(target: CloudTarget, key: CloudSshKey, auth_timeout: Duration) -> Result<Self> {
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
                let t0 = Instant::now();
                let authenticated = tokio::time::timeout(auth_timeout, async {
                    let config = Arc::new(russh::client::Config {
                        // API hello and commands are small, latency-sensitive
                        // frames. Do not wait for Nagle/delayed-ACK batching.
                        nodelay: true,
                        // Mobile networks drop silently (backgrounding, cell
                        // handoff). Detect a dead peer in ~30s, not ~80s.
                        keepalive_interval: Some(Duration::from_secs(10)),
                        keepalive_max: 2,
                        ..Default::default()
                    });
                    let mut session = russh::client::connect(
                        config,
                        (target.address, target.port),
                        PinnedHost { keys, mismatch: mismatch.clone() },
                    ).await.map_err(|e| failed(format!("SSH connect failed: {e}")))?;
                    trace("ssh handshake", t0);
                    let auth = session.authenticate_publickey(
                        &target.user,
                        russh::keys::PrivateKeyWithHashAlg::new(key.private, None),
                    ).await.map_err(|e| failed(format!("SSH authentication failed: {e}")))?;
                    if !auth.success() {
                        return Err(failed("The host rejected the SSH key"));
                    }
                    trace("ssh authenticated", t0);
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
                            Some(Command::Inspect { command, reply }) => {
                                let session = session.clone();
                                tokio::spawn(async move {
                                    let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
                                    let result = async {
                                        let mut channel = tokio::time::timeout_at(deadline, session.channel_open_session())
                                            .await.map_err(|_| ())?.map_err(|_| ())?;
                                        let result = tokio::time::timeout_at(deadline, async {
                                            channel.exec(true, command.as_bytes()).await.map_err(|_| ())?;
                                            let mut output = Vec::new();
                                            let mut received = 0usize;
                                            let mut success = false;
                                            loop {
                                                match channel.wait().await {
                                                    Some(russh::ChannelMsg::Data { data }) => {
                                                        received = received.saturating_add(data.len());
                                                        if received > crate::cloud_machine::MAX_OUTPUT_BYTES { return Err(()); }
                                                        output.extend_from_slice(&data);
                                                    }
                                                    Some(russh::ChannelMsg::ExtendedData { data, .. }) => {
                                                        received = received.saturating_add(data.len());
                                                        if received > crate::cloud_machine::MAX_OUTPUT_BYTES { return Err(()); }
                                                    }
                                                    Some(russh::ChannelMsg::ExitStatus { exit_status }) => success = exit_status == 0,
                                                    Some(russh::ChannelMsg::Close) | None => break,
                                                    Some(russh::ChannelMsg::Failure | russh::ChannelMsg::ExitSignal { .. }) => return Err(()),
                                                    // Never retain or expose stderr, including gh auth diagnostics.
                                                    Some(_) => {}
                                                }
                                            }
                                            if success { Ok(output) } else { Err(()) }
                                        }).await;
                                        let _ = tokio::time::timeout(Duration::from_millis(250), channel.close()).await;
                                        result.map_err(|_| ())?
                                    }.await;
                                    let _ = reply.send(match result {
                                        Ok(output) => Ok(output),
                                        _ => Err("Machine inspection failed or timed out".into()),
                                    });
                                });
                            }
                            Some(Command::Open { deadline, reply }) => {
                                let session = session.clone();
                                tokio::spawn(async move {
                                    let t_open = Instant::now();
                                    let result = tokio::time::timeout_at(
                                        tokio::time::Instant::from_std(deadline),
                                        async {
                                            // Opening a channel requires only a live SSH peer,
                                            // not remote process startup. A stale mobile socket
                                            // must not consume the cold API hello allowance.
                                            let channel = tokio::time::timeout(
                                                CHANNEL_OPEN_TIMEOUT,
                                                session.channel_open_session(),
                                            ).await.map_err(|_| Error::new(
                                                ErrorKind::Disconnected,
                                                "SSH peer did not respond while opening an API channel",
                                            ))?
                                                .map_err(|e| failed(format!("SSH channel open failed: {e}")))?;
                                            trace("channel opened", t_open);
                                            // No exec reply wait: the API hello is pipelined
                                            // behind it (saves one round trip). A refused exec
                                            // closes the channel, so hello still fails cleanly.
                                            channel.exec(false, REMOTE_COMMAND).await
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
        match connected.recv_timeout(auth_timeout + Duration::from_millis(250)) {
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

    /// Execute only the built-in read-only inspection, never caller-supplied shell.
    pub(crate) fn inspect_machine(&self) -> std::result::Result<Vec<u8>, String> {
        self.inspect_bounded(crate::cloud_machine::INSPECTION_COMMAND.to_string())
    }

    /// Idempotently authorize this device's persistent public key. Only a
    /// validated `ssh-ed25519 <base64>` line is interpolated.
    pub(crate) fn authorize_device_key(&self, public_openssh: &str) -> std::result::Result<(), String> {
        let mut parts = public_openssh.split(' ');
        let (Some("ssh-ed25519"), Some(blob)) = (parts.next(), parts.next()) else {
            return Err("invalid device key".into());
        };
        if blob.is_empty() || blob.len() > 128 || !blob.chars().all(|c| c.is_ascii_alphanumeric() || "+/=".contains(c)) {
            return Err("invalid device key".into());
        }
        let line = format!("ssh-ed25519 {blob} jcode-device");
        let command = format!(
            "umask 077; mkdir -p \"$HOME/.ssh\" && touch \"$HOME/.ssh/authorized_keys\" && \
             (grep -qF '{blob}' \"$HOME/.ssh/authorized_keys\" || echo '{line}' >> \"$HOME/.ssh/authorized_keys\")"
        );
        self.inspect_bounded(command).map(|_| ())
    }

    pub(crate) fn inspect_git(&self, working_dir: &str) -> std::result::Result<Vec<u8>, String> {
        self.inspect_bounded(crate::git_info::inspection_command(working_dir)?)
    }

    // Only built-in inspection builders may supply commands.
    fn inspect_bounded(&self, command: String) -> std::result::Result<Vec<u8>, String> {
        let (reply, result) = mpsc::channel();
        self.0.commands.send(Command::Inspect { command, reply })
            .map_err(|_| "Machine inspection connection closed".to_string())?;
        result.recv_timeout(Duration::from_secs(9))
            .map_err(|_| "Machine inspection failed or timed out".to_string())?
    }

    /// Open a fresh, independently attached `jcode api --stdio` channel.
    /// Channel creation, exec and API hello share a 15-second deadline, allowing
    /// cold process startup. Channel establishment itself is capped at two
    /// seconds to detect a stale peer. Authentication and attach are separate.
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
                let t_hello = Instant::now();
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
                trace("api hello", t_hello);
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
        hello_delay: Duration,
        withhold_channel_open: bool,
        pending_channels: Vec<russh::server::ChannelOpenHandle>,
        inspection_output: Option<Vec<u8>>,
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
            if self.withhold_channel_open {
                // Retain the reply until the connection closes, simulating a
                // peer that authenticated but stopped acknowledging channels.
                self.pending_channels.push(reply);
                return Ok(());
            }
            reply.accept().await;
            Ok(())
        }

        async fn exec_request(
            &mut self,
            channel: russh::ChannelId,
            command: &[u8],
            session: &mut russh::server::Session,
        ) -> std::result::Result<(), Self::Error> {
            if let Some(output) = &self.inspection_output {
                assert_eq!(command, crate::cloud_machine::INSPECTION_COMMAND.as_bytes());
                session.channel_success(channel)?;
                // Empty fixture means the server never finishes this command.
                if output.is_empty() { return Ok(()); }
                session.extended_data(channel, 1, b"auth secret-token".to_vec())?;
                session.data(channel, output.clone())?;
                session.exit_status_request(channel, 0)?;
                session.eof(channel)?;
                return session.close(channel);
            }
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
                        tokio::time::sleep(self.hello_delay).await;
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
            Self::start_with_inspection(withhold_first_hello, ping_delay, None)
        }

        fn start_with_inspection(
            withhold_first_hello: bool,
            ping_delay: Duration,
            inspection_output: Option<Vec<u8>>,
        ) -> (Self, CloudTarget, CloudSshKey) {
            Self::start_with_options(
                withhold_first_hello,
                ping_delay,
                inspection_output,
                Duration::ZERO,
                false,
            )
        }

        fn start_with_options(
            withhold_first_hello: bool,
            ping_delay: Duration,
            inspection_output: Option<Vec<u8>>,
            hello_delay: Duration,
            withhold_channel_open: bool,
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
                hello_delay,
                withhold_channel_open,
                pending_channels: Vec::new(),
                inspection_output,
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
    fn cold_api_hello_can_exceed_cached_probe_budget_without_reauthentication() {
        let (server, target, key) = Server::start_with_options(
            false,
            Duration::ZERO,
            None,
            Duration::from_millis(2200),
            false,
        );
        let prepared = PreparedSsh::new(CloudSsh::connect(target, key).unwrap());
        // Avoid a background spare so this measures exactly one cold process.
        prepared.0.state.lock().unwrap().filling = true;
        let started = Instant::now();
        let client = prepared.open_client().unwrap();
        assert!(started.elapsed() >= Duration::from_secs(2));
        client.ping().unwrap();
        assert_eq!(server.counts.authenticated.load(Ordering::Relaxed), 1);
        assert_eq!(server.counts.channels.load(Ordering::Relaxed), 1);
        drop(client);
        drop(prepared);
        server.finish();
    }

    #[test]
    fn stale_peer_channel_open_does_not_consume_cold_api_budget() {
        let (server, target, key) = Server::start_with_options(
            false, Duration::ZERO, None, Duration::ZERO, true,
        );
        let master = CloudSsh::connect(target, key).unwrap();
        let started = Instant::now();
        let error = master.open_client().err().expect("stale peer must time out");
        assert_eq!(error.kind, ErrorKind::Disconnected);
        assert!(started.elapsed() >= CHANNEL_OPEN_TIMEOUT);
        assert!(started.elapsed() < Duration::from_secs(4));
        assert_eq!(server.counts.authenticated.load(Ordering::Relaxed), 1);
        drop(master);
        server.finish();
    }

    #[test]
    fn short_prepared_probe_timeout_preserves_master_for_cold_retry() {
        let (server, target, key) = Server::start(true, Duration::ZERO);
        let prepared = PreparedSsh::new(CloudSsh::connect(target, key).unwrap());
        prepared.0.state.lock().unwrap().filling = true;
        let started = Instant::now();
        let error = prepared
            .open_client_timeout(Duration::from_millis(150))
            .err()
            .expect("withheld hello must exhaust probe budget");
        assert_eq!(error.kind, ErrorKind::StartupTimeout);
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(!prepared.is_closed());
        let client = prepared.open_client().unwrap();
        client.ping().unwrap();
        assert_eq!(server.counts.authenticated.load(Ordering::Relaxed), 1);
        assert_eq!(server.counts.channels.load(Ordering::Relaxed), 2);
        drop(client);
        drop(prepared);
        server.finish();
    }

    #[test]
    fn stalled_inspection_times_out_without_exposing_auth_details() {
        let (server, target, key) = Server::start_with_inspection(false, Duration::ZERO, Some(Vec::new()));
        let master = CloudSsh::connect(target, key).unwrap();
        let started = Instant::now();
        assert_eq!(master.inspect_machine().unwrap_err(), "Machine inspection failed or timed out");
        assert!(started.elapsed() < Duration::from_secs(10));
        assert!(!master.is_closed());
        drop(master);
        server.finish();
    }

    #[test]
    fn inspection_uses_fixed_pinned_command_discards_stderr_and_caps_output() {
        for (output, valid) in [(b"only structured stdout".to_vec(), true), (vec![b'x'; crate::cloud_machine::MAX_OUTPUT_BYTES + 1], false)] {
            let (server, target, key) = Server::start_with_inspection(false, Duration::ZERO, Some(output.clone()));
            let master = CloudSsh::connect(target, key).unwrap();
            let result = master.inspect_machine();
            if valid {
                assert_eq!(result.unwrap(), output);
            } else {
                assert_eq!(result.unwrap_err(), "Machine inspection failed or timed out");
            }
            drop(master);
            server.finish();
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

    fn wait_until(mut condition: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(4);
        while !condition() {
            assert!(
                Instant::now() < deadline,
                "SSH background work did not finish"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn prepared_channel_is_single_use_bounded_and_independent() {
        let (server, target, key) = Server::start(false, Duration::ZERO);
        let prepared = PreparedSsh::new(CloudSsh::connect(target, key).unwrap());
        let first = prepared.open_client().unwrap();
        first.attach_session("first").unwrap();
        wait_until(|| prepared.0.state.lock().unwrap().spare.is_some());
        assert_eq!(server.counts.channels.load(Ordering::Relaxed), 2);
        for _ in 0..10 {
            prepared.refill();
        }
        // Stop automatic refill to prove taking a ready spare opens no channel.
        prepared.0.state.lock().unwrap().filling = true;
        let second = prepared.open_client_timeout(Duration::ZERO).unwrap();
        assert_eq!(server.counts.channels.load(Ordering::Relaxed), 2);
        assert!(prepared.0.state.lock().unwrap().spare.is_none());
        second.attach_session("second").unwrap();
        assert_eq!(first.get_history("first").unwrap()[0].content, "first");
        assert_eq!(second.get_history("second").unwrap()[0].content, "second");
        assert_eq!(server.counts.authenticated.load(Ordering::Relaxed), 1);
        drop(prepared);
        first.ping().unwrap();
        second.ping().unwrap();
        drop(first);
        drop(second);
        server.finish();
    }

    #[test]
    fn expired_prepared_channel_is_replaced_and_eviction_keeps_active_client_alive() {
        let (server, target, key) = Server::start(false, Duration::ZERO);
        let prepared = PreparedSsh::new(CloudSsh::connect(target, key).unwrap());
        let first = prepared.open_client().unwrap();
        wait_until(|| prepared.0.state.lock().unwrap().spare.is_some());
        {
            let mut state = prepared.0.state.lock().unwrap();
            state.spare.as_mut().unwrap().0 = Instant::now() - SPARE_MAX_AGE;
            state.filling = true;
        }
        let second = prepared.open_client().unwrap();
        assert_eq!(server.counts.channels.load(Ordering::Relaxed), 3);
        drop(prepared);
        drop(second);
        first.ping().unwrap();
        drop(first);
        server.finish();
    }

    #[test]
    fn blocked_refill_never_blocks_foreground_or_retains_evicted_cache() {
        let (server, target, key) = Server::start(true, Duration::ZERO);
        let prepared = PreparedSsh::new(CloudSsh::connect(target, key).unwrap());
        prepared.refill();
        wait_until(|| server.counts.channels.load(Ordering::Relaxed) == 1);
        let client = prepared.open_client().unwrap();
        client.ping().unwrap();
        assert!(prepared.0.state.lock().unwrap().filling);
        let owner = Arc::downgrade(&prepared.0);
        drop(prepared);
        assert!(owner.upgrade().is_none(), "refill retained evicted cache");
        client.ping().unwrap();
        drop(client);
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
