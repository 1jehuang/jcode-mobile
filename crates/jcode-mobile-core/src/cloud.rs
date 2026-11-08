//! Jcode Cloud: account sign-in and the connection to the account's managed
//! host. The protocol lives in `jcode_sdk::cloud` (control plane) and
//! `jcode_sdk::NativeSshTransport` (pure-Rust SSH, no `ssh` binary, so it runs
//! on iOS). This module keeps the app-facing API small and string-errored and
//! owns on-device account persistence.

use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

// Keep a pinned, authenticated SSH master in memory briefly. Each tab still
// gets an independent API channel because the bridge attaches one session per
// channel. Reusing a JcodeClient itself would silently detach other tabs.
struct WarmTarget<T> {
    account: Account,
    ssh: T,
    expires: Instant,
}

impl<T: Clone> WarmTarget<T> {
    fn for_account(&self, account: &Account, now: Instant) -> Option<T> {
        (self.account == *account && self.expires > now).then(|| self.ssh.clone())
    }
}

type CachedSsh = WarmTarget<crate::cloud_ssh::PreparedSsh>;
static WARM_TARGET: OnceLock<Mutex<Option<CachedSsh>>> = OnceLock::new();
static COLD_CONNECT: Mutex<()> = Mutex::new(());
const TARGET_TTL: Duration = Duration::from_secs(5 * 60);

fn warm_target() -> &'static Mutex<Option<CachedSsh>> {
    WARM_TARGET.get_or_init(|| Mutex::new(None))
}

fn clear_warm_target() {
    *warm_target().lock().unwrap() = None;
}

/// Drop the in-memory SSH master so a benchmark sees an app-launch start.
#[doc(hidden)]
pub fn reset_in_memory_cache_for_benchmark() {
    clear_warm_target();
}

pub use jcode_sdk::cloud::{
    Account, CloudClient, DEFAULT_API_BASE, DeviceLogin, PushDevice, PushEnvironment, push_token_hex,
};

/// Kept for API stability: the browser sign-in handle.
pub type DeviceStart = DeviceLogin;

/// Mobile identity for user agents, Apple sign-in and SSH key comments.
pub const CLIENT_NAME: &str = "jcode-ios";

/// The cloud email and device auth endpoints require this exact protocol
/// identifier, even for mobile clients. It is not the app's display identity.
const CLOUD_CLIENT_NAME: &str = "jcode-cli";

pub fn client() -> CloudClient {
    CloudClient::new(CLOUD_CLIENT_NAME)
}

fn client_for(account: &Account) -> CloudClient {
    client().with_api_base(account.api_base.clone())
}

/// A dedicated pinned connection for read-only inspection. No API server is started.
pub(crate) fn inspection_ssh(account: &Account) -> Result<crate::cloud_ssh::CloudSsh, String> {
    let key = crate::cloud_ssh::CloudSshKey::generate(CLIENT_NAME)
        .map_err(|_| "Could not prepare machine inspection".to_string())?;
    let target = client_for(account).wait_for_host(account, key.public_openssh(), &|_| {}, &|| false)
        .map_err(|_| "Could not reach your cloud machine for inspection".to_string())?;
    crate::cloud_ssh::CloudSsh::connect(target, key)
        .map_err(|_| "Could not securely connect for machine inspection".to_string())
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
        #[cfg(target_os = "ios")]
        {
            if let Some(a) = keychain::load() {
                return Some(a);
            }
            // One-time migration from the pre-Keychain file store.
            let a = self.load_file()?;
            if keychain::save(&a).is_ok() {
                self.clear_file();
            }
            Some(a)
        }
        #[cfg(not(target_os = "ios"))]
        self.load_file()
    }
    pub fn save(&self, account: &Account) -> std::io::Result<()> {
        #[cfg(target_os = "ios")]
        {
            keychain::save(account)?;
            self.clear_file();
            Ok(())
        }
        #[cfg(not(target_os = "ios"))]
        self.save_file(account)
    }
    pub fn clear(&self) {
        clear_warm_target();
        crate::cloud_direct::clear();
        #[cfg(target_os = "ios")]
        keychain::clear();
        self.clear_file();
    }

    fn load_file(&self) -> Option<Account> {
        let text = std::fs::read_to_string(&self.path).ok()?;
        serde_json::from_str(&text).ok()
    }
    #[cfg_attr(target_os = "ios", allow(dead_code))]
    fn save_file(&self, account: &Account) -> std::io::Result<()> {
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
    fn clear_file(&self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// iOS Keychain (generic password item) holding the account JSON.
#[cfg(target_os = "ios")]
mod keychain {
    use super::Account;
    use security_framework::passwords::{
        delete_generic_password, get_generic_password, set_generic_password,
    };

    const SERVICE: &str = "sh.jcode.mobile";
    const ACCOUNT: &str = "account";

    pub fn load() -> Option<Account> {
        let bytes = get_generic_password(SERVICE, ACCOUNT).ok()?;
        serde_json::from_slice(&bytes).ok()
    }
    pub fn save(account: &Account) -> std::io::Result<()> {
        let bytes = serde_json::to_vec(account).map_err(std::io::Error::other)?;
        set_generic_password(SERVICE, ACCOUNT, &bytes).map_err(|e| std::io::Error::other(e.to_string()))
    }
    pub fn clear() {
        let _ = delete_generic_password(SERVICE, ACCOUNT);
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
        .unwrap_or_else(|| DEFAULT_API_BASE.into());
    Some(Account {
        api_key,
        email: get("JCODE_ACCOUNT_EMAIL"),
        api_base: api_base.trim_end_matches('/').into(),
    })
}

/// Mails a 6-digit code. Returns the login token for `verify_login`.
pub fn start_login(email: &str) -> Result<String, String> {
    client().start_email_login(email).map_err(|e| e.message)
}

pub fn verify_login(token: &str, code: &str) -> Result<Account, String> {
    client().verify_email_login(token, code).map_err(|e| e.message)
}

/// Browser sign-in: jcode.sh/account offers "Continue with Google" (and email).
pub fn start_browser_login() -> Result<DeviceStart, String> {
    client().start_device_login().map_err(|e| e.message)
}

/// Polls until the browser approves. `cancel` returning true stops early.
pub fn finish_browser_login(start: &DeviceStart, cancel: impl Fn() -> bool) -> Result<Account, String> {
    client().wait_for_device_login(start, &cancel).map_err(|e| e.message)
}

/// Permanently deletes the jcode.sh account (App Store in-app deletion):
/// cancels any subscription, revokes every key and session, erases personal
/// data. Callers must sign out locally afterwards.
pub fn delete_account(account: &Account) -> Result<(), String> {
    let url = format!("{}/account", account.api_base.trim_end_matches('/'));
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(30)))
        .http_status_as_error(false)
        .user_agent(CLIENT_NAME)
        .build()
        .into();
    let mut resp = agent
        .delete(&url)
        .header("Authorization", &format!("Bearer {}", account.api_key))
        .force_send_body()
        .send_json(serde_json::json!({ "confirm": "DELETE" }))
        .map_err(|e| format!("Could not reach jcode.sh: {e}"))?;
    let status = resp.status().as_u16();
    if (200..300).contains(&status) {
        return Ok(());
    }
    let text = resp.body_mut().read_to_string().unwrap_or_default();
    let body: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
    let message = body["error"]["message"].as_str().unwrap_or_default();
    Err(if message.is_empty() {
        format!("Account deletion failed (HTTP {status})")
    } else {
        message.to_string()
    })
}

/// Wake the account's host and open an SDK client over in-process SSH.
pub fn connect(
    account: &Account,
    progress: &dyn Fn(&str),
    cancelled: &dyn Fn() -> bool,
) -> jcode_sdk::Result<jcode_sdk::JcodeClient> {
    let check_cancelled = || {
        if cancelled() {
            Err(jcode_sdk::Error::new(jcode_sdk::ErrorKind::Cancelled, "Connection cancelled"))
        } else {
            Ok(())
        }
    };
    check_cancelled()?;
    let cached = {
        let cache = warm_target().lock().unwrap();
        cache.as_ref().and_then(|entry| entry.for_account(account, Instant::now()))
    };
    if let Some(ssh) = cached {
        progress("Opening your Jcode session…");
        match ssh.open_client() {
            Ok(client) => {
                check_cancelled()?;
                return Ok(client);
            }
            // An API startup timeout does not mean SSH authentication is stale.
            // Keep the master so a retry does not register another key and
            // repeat cloud provisioning on an otherwise healthy connection.
            Err(error) if !ssh.is_closed() && error.kind != jcode_sdk::ErrorKind::Disconnected => {
                return Err(error);
            }
            // A dead master must go back through the authenticated control
            // plane. Its validated pins, never TOFU, authorize the next host.
            Err(_) => clear_warm_target(),
        }
    }
    // Concurrent new tabs must not all register new keys and hit the cloud's
    // instance-connect rate limit. Recheck after waiting for another opener.
    let _opening = COLD_CONNECT.lock().unwrap();
    check_cancelled()?;
    let cached = {
        let cache = warm_target().lock().unwrap();
        cache.as_ref().and_then(|entry| entry.for_account(account, Instant::now()))
    };
    if let Some(ssh) = cached {
        progress("Opening your Jcode session…");
        match ssh.open_client() {
            Ok(client) => {
                check_cancelled()?;
                return Ok(client);
            }
            Err(error) if !ssh.is_closed() && error.kind != jcode_sdk::ErrorKind::Disconnected => {
                return Err(error);
            }
            Err(_) => clear_warm_target(),
        }
    }
    check_cancelled()?;
    // Fast path: the machine is usually still awake. SSH straight to the last
    // control-plane-validated address with this device's authorized key and
    // the same pinned host keys. Any failure falls through to the control plane.
    if let Some(state) = crate::cloud_direct::load_or_create(account)
        && let (Some(target), Some(key)) = (state.target(), state.key())
    {
        progress("Connecting to your Jcode Cloud machine…");
        match crate::cloud_ssh::CloudSsh::connect_timeout(target, key, crate::cloud_direct::DIRECT_AUTH_TIMEOUT) {
            Ok(ssh) => {
                let ssh = cache_master(account, ssh);
                progress("Opening your Jcode session…");
                match ssh.open_client() {
                    Ok(client) => {
                        check_cancelled()?;
                        return Ok(client);
                    }
                    Err(_) => clear_warm_target(),
                }
            }
            Err(error) if error.kind == jcode_sdk::ErrorKind::HostKeyMismatch => {
                // Address was reassigned. Never trust it again without the control plane.
                crate::cloud_direct::forget_target(account);
            }
            Err(_) => {}
        }
        check_cancelled()?;
    }
    let mut cloud = client_for(account);
    cloud.poll_interval = Duration::from_millis(250);
    let key = crate::cloud_ssh::CloudSshKey::generate(CLIENT_NAME)?;
    progress("Checking your Jcode Cloud machine…");
    let target = match wait_for_host_or_direct(&cloud, account, &key, progress, cancelled)? {
        HostWait::Ready(target) => target,
        HostWait::Direct(ssh) => {
            let ssh = cache_master(account, ssh);
            progress("Opening your Jcode session…");
            let client = ssh.open_client()?;
            check_cancelled()?;
            return Ok(client);
        }
    };
    check_cancelled()?;
    progress("Connecting to your Jcode Cloud machine…");
    let master = crate::cloud_ssh::CloudSsh::connect(target.clone(), key)?;
    check_cancelled()?;
    // Authorize this device for direct reconnects, off the critical path.
    {
        let (account, master, target) = (account.clone(), master.clone(), target);
        std::thread::spawn(move || crate::cloud_direct::remember(&account, &master, &target));
    }
    let ssh = cache_master(account, master);
    progress("Opening your Jcode session…");
    let client = ssh.open_client()?;
    check_cancelled()?;
    Ok(client)
}

enum HostWait {
    Ready(jcode_sdk::cloud::CloudTarget),
    Direct(crate::cloud_ssh::CloudSsh),
}

/// Like `CloudClient::wait_for_host`, but on a wake the control plane shares
/// the pinned target before console readiness. If this device's key is
/// already authorized there, SSH in as soon as sshd answers (about 6s after
/// boot) instead of waiting ~30s for the console-scraped ready signal.
fn wait_for_host_or_direct(
    cloud: &CloudClient,
    account: &Account,
    key: &crate::cloud_ssh::CloudSshKey,
    progress: &dyn Fn(&str),
    cancelled: &dyn Fn() -> bool,
) -> jcode_sdk::Result<HostWait> {
    const DEADLINE: Duration = Duration::from_secs(15 * 60);
    let deadline = Instant::now() + DEADLINE;
    let direct = crate::cloud_direct::load_or_create(account).filter(|s| s.target().is_some());
    let mut last = String::new();
    loop {
        let reply = cloud.request_host_connect(account, key.public_openssh())?;
        if reply.ready {
            return jcode_sdk::cloud::CloudTarget::from_reply(&reply).map(HostWait::Ready);
        }
        let message = reply.message.clone().unwrap_or_else(|| format!("Jcode Cloud machine is {}…", reply.state));
        if message != last {
            progress(&message);
            last = message;
        }
        if let Some(state) = &direct
            && reply.address.is_some()
            && let Ok(target) = jcode_sdk::cloud::CloudTarget::from_reply(&reply)
            && let Some(device_key) = state.key()
            && let Ok(ssh) = crate::cloud_ssh::CloudSsh::connect_timeout(target.clone(), device_key, Duration::from_millis(1200))
        {
            crate::cloud_direct::remember_target(account, &target);
            return Ok(HostWait::Direct(ssh));
        }
        if Instant::now() >= deadline {
            return Err(jcode_sdk::Error::new(
                jcode_sdk::ErrorKind::StartupTimeout,
                "Jcode Cloud did not become ready in time. Retry to keep waiting.",
            ));
        }
        let until = Instant::now() + cloud.poll_interval;
        while Instant::now() < until {
            if cancelled() {
                return Err(jcode_sdk::Error::new(jcode_sdk::ErrorKind::Cancelled, "Connection cancelled"));
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

fn cache_master(account: &Account, master: crate::cloud_ssh::CloudSsh) -> crate::cloud_ssh::PreparedSsh {
    let ssh = crate::cloud_ssh::PreparedSsh::new(master);
    let expires = Instant::now() + TARGET_TTL;
    // Cache authentication before API startup, which may need longer on a
    // waking host. A failed API hello must not throw away successful SSH.
    *warm_target().lock().unwrap() = Some(WarmTarget {
        account: account.clone(),
        ssh: ssh.clone(),
        expires,
    });
    // Expire idle masters even when the user never opens another tab. Live
    // channels retain their own master handles and are not interrupted.
    std::thread::spawn(move || {
        std::thread::sleep(TARGET_TTL);
        let mut cache = warm_target().lock().unwrap();
        if cache.as_ref().is_some_and(|entry| entry.expires == expires) {
            *cache = None;
        }
    });
    ssh
}

/// Register this phone's APNs token with the account.
pub fn register_push(account: &Account, device: &PushDevice) -> Result<(), String> {
    client_for(account).register_push_device(account, device).map_err(|e| e.to_string())
}

/// Forget this phone's APNs token (sign-out).
pub fn unregister_push(account: &Account, token_hex: &str) -> Result<(), String> {
    client_for(account).unregister_push_device(account, token_hex).map_err(|e| e.to_string())
}

/// Reset the server-side unread badge counter.
pub fn reset_badge(account: &Account) -> Result<(), String> {
    client_for(account).reset_push_badge(account).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Uses real account credentials without printing them. Unlike SSH fixture
    /// tests, this includes the control plane and the installed cloud binary.
    /// Does not create a conversation or submit a model request.
    #[test]
    #[ignore = "contacts the real cloud using the locally saved account"]
    fn benchmark_real_cloud_startup() {
        let account = AccountStore::default_location().load()
            .or_else(account_from_jcode_config)
            .expect("a saved Jcode account is required");
        clear_warm_target();
        for pass in ["cold", "warm"] {
            let started = Instant::now();
            let client = connect(
                &account,
                &|stage| eprintln!(
                    "cloud_startup {pass} elapsed_ms={} stage={stage}",
                    started.elapsed().as_millis(),
                ),
                &|| false,
            ).unwrap_or_else(|error| {
                // Error messages can include server details. Keep the receipt
                // limited to a fixed error kind and timing.
                panic!(
                    "cloud_startup {pass} failed after {}ms: {:?}",
                    started.elapsed().as_millis(), error.kind,
                )
            });
            eprintln!("cloud_startup {pass} hello_ms={}", started.elapsed().as_millis());
            client.ping().expect("cloud API ping");
            eprintln!("cloud_startup {pass} ping_ms={}", started.elapsed().as_millis());
            let sessions = client.list_sessions_limited(1).expect("cloud session list");
            eprintln!("cloud_startup {pass} list_ms={}", started.elapsed().as_millis());
            if let Some(session) = sessions.first() {
                client.attach_session(&session.session_id).expect("attach existing cloud session");
                eprintln!("cloud_startup {pass} attached_ms={}", started.elapsed().as_millis());
            }
        }
        clear_warm_target();
    }

    #[test]
    fn account_store_roundtrip() {
        let dir = std::env::temp_dir().join(format!("jm-acct-{}", std::process::id()));
        let store = AccountStore::new(dir.join("a.json"));
        let acct =
            Account { api_key: "k".into(), email: Some("e@x".into()), api_base: DEFAULT_API_BASE.into() };
        store.save(&acct).unwrap();
        assert_eq!(store.load(), Some(acct));
        store.clear();
        assert!(store.load().is_none());
    }

    #[test]
    fn warm_master_is_scoped_to_credentials_endpoint_and_expiry() {
        let account = Account { api_key: "key-a".into(), email: None, api_base: DEFAULT_API_BASE.into() };
        let now = Instant::now();
        let entry = WarmTarget { account: account.clone(), ssh: 7, expires: now + TARGET_TTL };
        assert_eq!(entry.for_account(&account, now), Some(7));
        assert_eq!(entry.for_account(&account, now + TARGET_TTL), None);
        let mut other = account.clone();
        other.api_key = "key-b".into();
        assert_eq!(entry.for_account(&other, now), None);
        other = account.clone();
        other.api_base = "https://other.invalid/v1".into();
        assert_eq!(entry.for_account(&other, now), None);
    }

    #[test]
    fn cancelled_connection_does_not_contact_cloud() {
        let account =
            Account { api_key: "unused".into(), email: None, api_base: "http://127.0.0.1:1".into() };
        let result = connect(&account, &|_| panic!("cancelled connection reported progress"), &|| true);
        assert!(matches!(result, Err(e) if e.kind == jcode_sdk::ErrorKind::Cancelled));
    }

    #[test]
    fn cloud_client_uses_compatible_protocol_identity() {
        assert_eq!(client().client_name, "jcode-cli");
        assert_eq!(client().api_base, DEFAULT_API_BASE);
        let account = Account {
            api_key: "unused".into(),
            email: None,
            api_base: "https://cloud.example/v1".into(),
        };
        assert_eq!(client_for(&account).client_name, "jcode-cli");
        assert_eq!(client_for(&account).api_base, account.api_base);
        assert_eq!(CLIENT_NAME, "jcode-ios");
    }

    #[test]
    fn auth_requests_send_server_compatible_client_name() {
        use std::io::{BufRead, BufReader, Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let api_base = format!("http://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            for path in ["/auth/email/start", "/auth/device"] {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut reader = BufReader::new(&mut stream);
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                assert!(line.starts_with(&format!("POST {path} HTTP/1.1")), "{line}");
                let mut length = None;
                loop {
                    line.clear();
                    assert_ne!(reader.read_line(&mut line).unwrap(), 0, "incomplete headers");
                    if line == "\r\n" {
                        break;
                    }
                    if let Some((name, value)) = line.split_once(':') {
                        if name.eq_ignore_ascii_case("content-length") {
                            length = Some(value.trim().parse::<usize>().unwrap());
                        }
                    }
                }
                let mut body = vec![0; length.expect("JSON content length")];
                reader.read_exact(&mut body).unwrap();
                let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
                assert_eq!(body["client_name"], "jcode-cli");
                if path == "/auth/email/start" {
                    assert_eq!(body["email"], "mobile@example.com");
                }
                // A deliberate API error avoids coupling this wire-level test
                // to the SDK's successful device-login response shape.
                let response = r#"{"error":{"message":"compatible client accepted"}}"#;
                write!(stream, "HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}", response.len()).unwrap();
            }
        });
        let cloud = client().with_api_base(api_base);
        let email = cloud.start_email_login("mobile@example.com").unwrap_err();
        assert!(
            email.message.contains("compatible client accepted"),
            "{email}"
        );
        let device = cloud.start_device_login().unwrap_err();
        assert!(
            device.message.contains("compatible client accepted"),
            "{device}"
        );
        server.join().unwrap();
    }
}
