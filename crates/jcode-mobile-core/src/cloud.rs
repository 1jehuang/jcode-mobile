//! Jcode Cloud: account sign-in and a native-protocol connection to the
//! account's managed host, without pairing.
//!
//! Mirrors jcode-desktop's managed_cloud transport, but pure Rust so it runs
//! on iOS (no `ssh` binary): a fresh ed25519 key is authorized for 60 seconds
//! by `POST /cloud/host/connect`, then russh logs in pinned to the host keys
//! the control plane returned and execs `jcode server stdio`, which bridges
//! NDJSON Request/ServerEvent frames to the host's shared daemon.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

pub const DEFAULT_API_BASE: &str = "https://api.jcode.sh/v1";
const PROVISION_DEADLINE: Duration = Duration::from_secs(8 * 60);
const POLL_INTERVAL: Duration = Duration::from_secs(4);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Account {
    pub api_key: String,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default = "default_base")]
    pub api_base: String,
}

fn default_base() -> String {
    DEFAULT_API_BASE.into()
}

/// Persistent account storage (app data dir on iOS, XDG data on Linux).
pub struct AccountStore {
    path: PathBuf,
}

impl AccountStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }
    pub fn default_location() -> Self {
        let base = dirs::data_dir().unwrap_or_else(std::env::temp_dir);
        Self::new(base.join("jcode-mobile").join("account.json"))
    }
    pub fn load(&self) -> Option<Account> {
        let text = std::fs::read_to_string(&self.path).ok()?;
        serde_json::from_str(&text).ok()
    }
    pub fn save(&self, account: &Account) -> std::io::Result<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = serde_json::to_string_pretty(account).unwrap();
        let tmp = self.path.with_extension("tmp");
        {
            use std::io::Write;
            let mut opts = std::fs::OpenOptions::new();
            opts.write(true).create(true).truncate(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                opts.mode(0o600);
            }
            opts.open(&tmp)?.write_all(text.as_bytes())?;
        }
        std::fs::rename(tmp, &self.path)
    }
    pub fn clear(&self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Dev convenience: reuse the desktop/CLI account file
/// (`~/.config/jcode/jcode-subscription.env`) when no mobile account exists.
pub fn account_from_jcode_config() -> Option<Account> {
    let path = dirs::config_dir()?.join("jcode").join("jcode-subscription.env");
    let text = std::fs::read_to_string(path).ok()?;
    let get = |key: &str| {
        text.lines().find_map(|l| {
            let l = l.trim().strip_prefix("export ").unwrap_or(l.trim());
            let v = l.strip_prefix(key)?.strip_prefix('=')?;
            Some(v.trim().trim_matches('"').trim_matches('\'').to_string())
        })
    };
    let api_key = get("JCODE_API_KEY").filter(|k| !k.is_empty())?;
    let api_base = get("JCODE_API_BASE")
        .filter(|b| b.starts_with("https://"))
        .unwrap_or_else(default_base);
    Some(Account { api_key, email: get("JCODE_ACCOUNT_EMAIL"), api_base: api_base.trim_end_matches('/').into() })
}

// MARK: - HTTP

#[derive(Debug, Deserialize)]
struct ErrorReply {
    error: ErrorBody,
}
#[derive(Debug, Deserialize)]
struct ErrorBody {
    #[serde(default)]
    code: String,
    #[serde(default)]
    message: String,
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(30)))
        .http_status_as_error(false)
        .user_agent(concat!("jcode-mobile/", env!("CARGO_PKG_VERSION")))
        .build()
        .into()
}

fn post_json<T: for<'de> Deserialize<'de>>(
    url: &str,
    api_key: Option<&str>,
    body: serde_json::Value,
) -> Result<T, String> {
    let mut req = agent().post(url);
    if let Some(k) = api_key {
        req = req.header("Authorization", &format!("Bearer {k}"));
    }
    let mut resp = req.send_json(body).map_err(|e| format!("Could not reach jcode.sh: {e}"))?;
    let status = resp.status().as_u16();
    let text = resp.body_mut().read_to_string().map_err(|e| e.to_string())?;
    if (200..300).contains(&status) {
        return serde_json::from_str(&text).map_err(|e| format!("Unexpected reply from jcode.sh: {e}"));
    }
    let (code, message) = serde_json::from_str::<ErrorReply>(&text)
        .map(|r| (r.error.code, r.error.message))
        .unwrap_or_default();
    Err(describe_error(status, &code, &message))
}

fn describe_error(status: u16, code: &str, message: &str) -> String {
    match (status, code) {
        (401, _) => "Your Jcode sign-in has expired. Sign in again.".into(),
        (402, "cloud_not_entitled") => "Jcode Cloud requires an active paid Jcode subscription.".into(),
        _ if !message.is_empty() => message.into(),
        _ => format!("Request failed (HTTP {status})"),
    }
}

#[derive(Debug, Deserialize)]
pub struct LoginStart {
    pub login_token: String,
}
#[derive(Debug, Deserialize)]
struct LoginResult {
    api_key: String,
    #[serde(default)]
    email: Option<String>,
}

/// Mails a 6-digit code. Returns the login token for `verify_login`.
pub fn start_login(email: &str) -> Result<String, String> {
    let r: LoginStart = post_json(
        &format!("{DEFAULT_API_BASE}/auth/email/start"),
        None,
        serde_json::json!({"email": email, "client_name": "jcode-ios"}),
    )?;
    Ok(r.login_token)
}

#[derive(Debug, Clone, Deserialize)]
pub struct DeviceStart {
    pub device_code: String,
    pub verification_uri_complete: String,
    #[serde(default = "default_interval")]
    pub interval: u64,
    #[serde(default)]
    pub expires_in: u64,
}
fn default_interval() -> u64 {
    5
}

/// Browser sign-in: jcode.sh/account offers "Continue with Google" (and
/// email). Identifies as this app, like the email-code flow.
pub fn start_browser_login() -> Result<DeviceStart, String> {
    post_json(
        &format!("{DEFAULT_API_BASE}/auth/device"),
        None,
        serde_json::json!({"client_name": "jcode-ios"}),
    )
}

/// Polls until the browser approves. `cancel` returning true stops early.
pub fn finish_browser_login(start: &DeviceStart, cancel: impl Fn() -> bool) -> Result<Account, String> {
    let mut interval = start.interval.max(2);
    let deadline = std::time::Instant::now()
        + std::time::Duration::from_secs(if start.expires_in > 0 { start.expires_in } else { 900 });
    loop {
        for _ in 0..interval * 4 {
            if cancel() {
                return Err("Sign-in cancelled".into());
            }
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
        if std::time::Instant::now() > deadline {
            return Err("Sign-in expired. Try again.".into());
        }
        let resp = ureq::post(&format!("{DEFAULT_API_BASE}/auth/token"))
            .config()
            .http_status_as_error(false)
            .build()
            .send_json(serde_json::json!({"device_code": start.device_code}))
            .map_err(|e| e.to_string())?;
        let status = resp.status().as_u16();
        let body: serde_json::Value = resp.into_body().read_json().unwrap_or_default();
        if status == 200 {
            let r: LoginResult = serde_json::from_value(body).map_err(|e| e.to_string())?;
            return Ok(Account { api_key: r.api_key, email: r.email, api_base: default_base() });
        }
        let err = &body["error"];
        match err["code"].as_str().unwrap_or_default() {
            "authorization_pending" => {}
            "slow_down" => {
                interval = err["interval"]
                    .as_u64()
                    .or(body["interval"].as_u64())
                    .unwrap_or(interval + 2)
            }
            _ => {
                return Err(err["message"]
                    .as_str()
                    .map(String::from)
                    .unwrap_or_else(|| format!("Sign-in failed (HTTP {status})")));
            }
        }
    }
}

pub fn verify_login(token: &str, code: &str) -> Result<Account, String> {
    let r: LoginResult = post_json(
        &format!("{DEFAULT_API_BASE}/auth/email/verify"),
        None,
        serde_json::json!({"login_token": token, "code": code}),
    )?;
    Ok(Account { api_key: r.api_key, email: r.email, api_base: default_base() })
}

#[derive(Debug, Deserialize, Default)]
pub struct ConnectReply {
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub ready: bool,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub address: Option<String>,
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default)]
    pub user: Option<String>,
    #[serde(default)]
    pub host_keys: Vec<String>,
}

pub fn request_connect(account: &Account, public_key: &str) -> Result<ConnectReply, String> {
    post_json(
        &format!("{}/cloud/host/connect", account.api_base),
        Some(&account.api_key),
        serde_json::json!({"public_key": public_key}),
    )
}

/// A validated SSH target. Only IP literals, safe user names and
/// ed25519/ecdsa host keys are accepted.
#[derive(Debug, Clone, PartialEq)]
pub struct Target {
    pub address: std::net::IpAddr,
    pub port: u16,
    pub user: String,
    pub host_keys: Vec<String>,
}

pub fn validated_target(reply: &ConnectReply) -> Result<Target, String> {
    let address = reply
        .address
        .as_deref()
        .and_then(|a| a.parse().ok())
        .ok_or("Jcode Cloud returned an invalid host address")?;
    let user = reply
        .user
        .as_deref()
        .filter(|u| {
            !u.is_empty() && u.len() <= 32 && u.bytes().all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
        })
        .ok_or("Jcode Cloud returned an invalid user")?;
    let host_keys: Vec<String> = reply
        .host_keys
        .iter()
        .filter(|k| {
            let mut p = k.split(' ');
            matches!((p.next(), p.next(), p.next()),
                (Some("ssh-ed25519" | "ecdsa-sha2-nistp256"), Some(blob), None)
                    if blob.len() <= 1024 && blob.bytes().all(|b| b.is_ascii_alphanumeric() || b"+/=".contains(&b)))
        })
        .cloned()
        .collect();
    if host_keys.is_empty() {
        return Err("Jcode Cloud did not provide verified host keys".into());
    }
    Ok(Target { address, port: reply.port.unwrap_or(22), user: user.into(), host_keys })
}

// MARK: - SSH

pub struct EphemeralKey {
    pub private: russh::keys::PrivateKey,
    pub public_openssh: String,
}

impl EphemeralKey {
    pub fn generate() -> Result<Self, String> {
        let private = russh::keys::PrivateKey::random(
            &mut rand_core::OsRng,
            russh::keys::Algorithm::Ed25519,
        )
        .map_err(|e| format!("Could not generate a connection key: {e}"))?;
        let mut public_openssh = private.public_key().to_openssh().map_err(|e| e.to_string())?;
        public_openssh.push_str(" jcode-mobile");
        Ok(Self { private, public_openssh })
    }
}

struct PinnedHost {
    keys: Vec<russh::keys::PublicKey>,
}

impl russh::client::Handler for PinnedHost {
    type Error = russh::Error;
    async fn check_server_key(&mut self, key: &russh::keys::PublicKey) -> Result<bool, Self::Error> {
        Ok(self.keys.iter().any(|k| k.key_data() == key.key_data()))
    }
}

/// Wakes or provisions the host, polling until ready. `progress` gets
/// human-readable status lines.
pub fn wait_for_host(
    account: &Account,
    key: &EphemeralKey,
    progress: &dyn Fn(&str),
    cancelled: &dyn Fn() -> bool,
) -> Result<Target, String> {
    let deadline = std::time::Instant::now() + PROVISION_DEADLINE;
    let mut last = String::new();
    loop {
        let reply = request_connect(account, &key.public_openssh)?;
        if reply.ready {
            return validated_target(&reply);
        }
        let msg = reply
            .message
            .clone()
            .unwrap_or_else(|| format!("Jcode Cloud machine is {}…", reply.state));
        if msg != last {
            progress(&msg);
            last = msg;
        }
        if std::time::Instant::now() >= deadline {
            return Err("Jcode Cloud did not become ready in time.".into());
        }
        let until = std::time::Instant::now() + POLL_INTERVAL;
        while std::time::Instant::now() < until {
            if cancelled() {
                return Err("cancelled".into());
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

pub const REMOTE_COMMAND: &str = "PATH=\"$HOME/.local/bin:$HOME/.cargo/bin:$PATH\"; export PATH; exec jcode --no-update --no-selfdev server stdio";

/// A live `jcode server stdio` channel: line-oriented, blocking API on top of
/// a private single-threaded tokio runtime.
pub struct CloudStream {
    rt: tokio::runtime::Runtime,
    _session: russh::client::Handle<PinnedHost>,
    channel: russh::Channel<russh::client::Msg>,
    buf: Vec<u8>,
    pub handshake: serde_json::Value,
}

pub enum Read {
    Line(String),
    Idle,
    Closed,
}

impl CloudStream {
    pub fn open(target: &Target, key: &EphemeralKey) -> Result<Self, String> {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| e.to_string())?;
        let keys = target
            .host_keys
            .iter()
            .filter_map(|k| russh::keys::PublicKey::from_openssh(k).ok())
            .collect::<Vec<_>>();
        let (session, channel) = rt.block_on(async {
            let config = Arc::new(russh::client::Config {
                keepalive_interval: Some(Duration::from_secs(20)),
                ..Default::default()
            });
            let addr = (target.address, target.port);
            let mut session = tokio::time::timeout(
                Duration::from_secs(45),
                russh::client::connect(config, addr, PinnedHost { keys }),
            )
            .await
            .map_err(|_| "Connecting to Jcode Cloud timed out".to_string())?
            .map_err(|e| format!("Could not connect to Jcode Cloud: {e}"))?;
            let auth = session
                .authenticate_publickey(
                    &target.user,
                    russh::keys::PrivateKeyWithHashAlg::new(Arc::new(key.private.clone()), None),
                )
                .await
                .map_err(|e| e.to_string())?;
            if !auth.success() {
                return Err("Jcode Cloud rejected the connection key".to_string());
            }
            let channel = session.channel_open_session().await.map_err(|e| e.to_string())?;
            channel.exec(true, REMOTE_COMMAND).await.map_err(|e| e.to_string())?;
            Ok::<_, String>((session, channel))
        })?;
        let mut s = Self { rt, _session: session, channel, buf: Vec::new(), handshake: serde_json::Value::Null };
        // First line is the native-stdio handshake.
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        loop {
            match s.read_line(Duration::from_secs(1)) {
                Read::Line(l) => {
                    let v: serde_json::Value = serde_json::from_str(&l)
                        .map_err(|_| format!("Unexpected handshake from Jcode Cloud: {l}"))?;
                    if v["kind"] != "jcode-native-stdio" {
                        return Err(format!("Unexpected handshake from Jcode Cloud: {l}"));
                    }
                    s.handshake = v;
                    return Ok(s);
                }
                Read::Closed => return Err("Jcode Cloud closed the connection during startup".into()),
                Read::Idle if std::time::Instant::now() > deadline => {
                    return Err("Jcode Cloud startup timed out".into());
                }
                Read::Idle => {}
            }
        }
    }

    pub fn working_dir(&self) -> Option<String> {
        self.handshake["working_dir"].as_str().map(str::to_owned)
    }

    pub fn send_line(&mut self, line: &str) -> Result<(), String> {
        let mut data = line.as_bytes().to_vec();
        if !data.ends_with(b"\n") {
            data.push(b'\n');
        }
        let ch = &self.channel;
        self.rt.block_on(ch.data(&data[..])).map_err(|e| e.to_string())
    }

    /// Returns the next complete line, waiting at most `wait`.
    pub fn read_line(&mut self, wait: Duration) -> Read {
        loop {
            if let Some(pos) = self.buf.iter().position(|&b| b == b'\n') {
                let line: Vec<u8> = self.buf.drain(..=pos).collect();
                return Read::Line(String::from_utf8_lossy(&line).trim_end().to_string());
            }
            let ch = &mut self.channel;
            let msg = self.rt.block_on(async { tokio::time::timeout(wait, ch.wait()).await });
            match msg {
                Err(_) => return Read::Idle,
                Ok(None) => return Read::Closed,
                Ok(Some(russh::ChannelMsg::Data { data })) => self.buf.extend_from_slice(&data),
                Ok(Some(russh::ChannelMsg::Eof | russh::ChannelMsg::Close)) => return Read::Closed,
                Ok(Some(russh::ChannelMsg::ExitStatus { .. })) => return Read::Closed,
                Ok(Some(_)) => {}
            }
        }
    }

    pub fn close(&mut self) {
        let ch = &self.channel;
        let _ = self.rt.block_on(async { tokio::time::timeout(Duration::from_secs(2), ch.eof()).await });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(v: serde_json::Value) -> ConnectReply {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn target_validation_rejects_injection() {
        let ok = reply(serde_json::json!({"ready":true,"address":"203.0.113.7","user":"ec2-user","host_keys":["ssh-ed25519 AAAAC3"]}));
        assert_eq!(validated_target(&ok).unwrap().port, 22);
        for bad in [
            serde_json::json!({"address":"evil.example","user":"u","host_keys":["ssh-ed25519 AAAA"]}),
            serde_json::json!({"address":"203.0.113.7","user":"-l root","host_keys":["ssh-ed25519 AAAA"]}),
            serde_json::json!({"address":"203.0.113.7","user":"u","host_keys":["ssh-rsa AAAA"]}),
            serde_json::json!({"address":"203.0.113.7","user":"u","host_keys":["ssh-ed25519 A\n* x"]}),
        ] {
            assert!(validated_target(&reply(bad.clone())).is_err(), "accepted {bad}");
        }
    }

    #[test]
    fn ephemeral_keys_are_fresh_openssh_ed25519() {
        let a = EphemeralKey::generate().unwrap();
        let b = EphemeralKey::generate().unwrap();
        assert!(a.public_openssh.starts_with("ssh-ed25519 "));
        assert_ne!(a.public_openssh, b.public_openssh);
    }

    #[test]
    fn account_store_roundtrip() {
        let dir = std::env::temp_dir().join(format!("jm-acct-{}", std::process::id()));
        let store = AccountStore::new(dir.join("a.json"));
        let acct = Account { api_key: "k".into(), email: Some("e@x".into()), api_base: default_base() };
        store.save(&acct).unwrap();
        assert_eq!(store.load(), Some(acct));
        store.clear();
        assert!(store.load().is_none());
    }
}
