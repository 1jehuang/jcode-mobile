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
        (self.account.api_key == account.api_key && self.account.api_base == account.api_base && self.expires > now)
            .then(|| self.ssh.clone())
    }
}

type CachedSsh = WarmTarget<crate::cloud_ssh::PreparedSsh>;
static WARM_TARGET: OnceLock<Mutex<Option<CachedSsh>>> = OnceLock::new();
static COLD_CONNECT: Mutex<Option<FailedPrewarm>> = Mutex::new(None);
const TARGET_TTL: Duration = Duration::from_secs(5 * 60);

struct FailedPrewarm {
    account: Account,
    finished: Instant,
}

/// Waiting behind another host opener must remain cancellable, even while
/// that opener is waiting for a sleeping machine. Never cancel its work.
fn lock_cold_connect(
    cancelled: &dyn Fn() -> bool,
) -> jcode_sdk::Result<std::sync::MutexGuard<'static, Option<FailedPrewarm>>> {
    lock_with_cancellation(&COLD_CONNECT, cancelled)
}

fn lock_with_cancellation<'a, T>(
    mutex: &'a Mutex<T>,
    cancelled: &dyn Fn() -> bool,
) -> jcode_sdk::Result<std::sync::MutexGuard<'a, T>> {
    loop {
        if cancelled() {
            return Err(jcode_sdk::Error::new(jcode_sdk::ErrorKind::Cancelled, "Connection cancelled"));
        }
        match mutex.try_lock() {
            Ok(guard) => return Ok(guard),
            Err(std::sync::TryLockError::WouldBlock) => std::thread::sleep(Duration::from_millis(10)),
            Err(std::sync::TryLockError::Poisoned(error)) => {
                // Match the existing fail-closed poisoned-lock behavior.
                drop(error.into_inner());
                return Err(jcode_sdk::Error::new(jcode_sdk::ErrorKind::Disconnected, "Connection coordinator unavailable"));
            }
        }
    }
}

/// Share only a failed speculative attempt that overlapped this foreground
/// call. This is not a negative cache: a later call still tries direct SSH,
/// and a different account can never suppress this account's attempt.
fn take_overlapping_prewarm_failure(
    failure: &mut Option<FailedPrewarm>,
    account: &Account,
    started: Instant,
) -> bool {
    failure.take().is_some_and(|failure| {
        failure.finished >= started
            && failure.account.api_key == account.api_key
            && failure.account.api_base == account.api_base
    })
}

fn warm_target() -> &'static Mutex<Option<CachedSsh>> {
    WARM_TARGET.get_or_init(|| Mutex::new(None))
}

fn clear_warm_target() {
    *warm_target().lock().unwrap() = None;
}

/// Forget `dead` only if it is still the cached master. Another tab may have
/// already replaced it with a fresh one that must not be thrown away.
fn forget_master(dead: &crate::cloud_ssh::PreparedSsh) {
    let mut cache = warm_target().lock().unwrap();
    if cache.as_ref().is_some_and(|entry| entry.ssh.same(dead)) {
        *cache = None;
    }
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
#[cfg(target_os = "android")]
pub const CLIENT_NAME: &str = "jcode-android";
#[cfg(not(target_os = "android"))]
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

/// Refresh the saved account identity from the authenticated profile.
pub fn refresh_account_profile(account: &Account) -> Result<Account, String> {
    client_for(account).refresh_account_profile(account).map_err(|e| e.message)
}

/// A dedicated pinned connection for read-only inspection. No API server is started.
/// How long an inspection may spend reaching the machine through the control
/// plane. Inspections are glanceable: they never wait out provisioning.
const INSPECTION_CONNECT: Duration = Duration::from_secs(10);

/// SSH for a bounded read-only inspection. Prefers the already authenticated
/// master (no network round trip), then this device's direct key, and only
/// then the control plane, with a short deadline instead of provisioning's.
pub(crate) fn inspection_ssh(account: &Account) -> Result<crate::cloud_ssh::CloudSsh, String> {
    let cached = {
        let cache = warm_target().lock().unwrap();
        cache.as_ref().and_then(|entry| entry.for_account(account, Instant::now()))
    };
    if let Some(ssh) = cached.filter(|ssh| !ssh.is_closed()) {
        return Ok(ssh.master().clone());
    }
    if let Some(state) = crate::cloud_direct::load_or_create(account)
        && let (Some(target), Some(key)) = (state.target(), state.key())
    {
        match crate::cloud_ssh::CloudSsh::connect_timeout(target, key, crate::cloud_direct::DIRECT_AUTH_TIMEOUT) {
            Ok(ssh) => return Ok(cache_master(account, ssh).master().clone()),
            Err(error) if error.kind == jcode_sdk::ErrorKind::HostKeyMismatch => {
                crate::cloud_direct::forget_target(account);
            }
            Err(_) => {}
        }
    }
    let unreachable = || "Could not reach your cloud machine for inspection".to_string();
    let key = crate::cloud_ssh::CloudSshKey::generate(CLIENT_NAME)
        .map_err(|_| "Could not prepare machine inspection".to_string())?;
    let mut cloud = client_for(account);
    cloud.poll_interval = Duration::from_millis(250);
    let deadline = Instant::now() + INSPECTION_CONNECT;
    let ssh = match wait_for_host_or_direct(&cloud, account, &key, &|_| {}, &|| Instant::now() >= deadline)
        .map_err(|_| unreachable())?
    {
        HostWait::Direct(ssh) => ssh,
        HostWait::Ready(target) => crate::cloud_ssh::CloudSsh::connect_timeout(target, key, INSPECTION_CONNECT)
            .map_err(|_| "Could not securely connect for machine inspection".to_string())?,
    };
    Ok(cache_master(account, ssh).master().clone())
}

/// Drop a cached master that an inspection found dead, so a retry reconnects.
pub(crate) fn forget_inspection_ssh(dead: &crate::cloud_ssh::CloudSsh) {
    let mut cache = warm_target().lock().unwrap();
    if cache.as_ref().is_some_and(|entry| entry.ssh.master().same_connection(dead)) {
        *cache = None;
    }
}

/// Android's application installs a Keystore-backed store before loading accounts.
/// Implementations must encrypt at rest and must never log these bytes.
#[cfg(any(target_os = "android", test))]
pub trait AndroidAccountStorage: Send + Sync + 'static {
    fn read(&self) -> std::io::Result<Option<Vec<u8>>>;
    fn write(&self, bytes: &[u8]) -> std::io::Result<()>;
    fn clear(&self) -> std::io::Result<()>;
}

#[cfg(any(target_os = "android", test))]
static ANDROID_ACCOUNT_STORAGE: std::sync::OnceLock<std::sync::Arc<dyn AndroidAccountStorage>> = std::sync::OnceLock::new();

/// Install once during Android bootstrap, before any account prewarming.
#[cfg(any(target_os = "android", test))]
pub fn install_android_account_storage(store: std::sync::Arc<dyn AndroidAccountStorage>) -> bool {
    ANDROID_ACCOUNT_STORAGE.set(store).is_ok()
}

#[cfg(any(target_os = "android", test))]
fn android_account_storage() -> std::io::Result<&'static dyn AndroidAccountStorage> {
    ANDROID_ACCOUNT_STORAGE.get().map(|store| store.as_ref()).ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::NotConnected, "Android secure account storage is not initialized")
    })
}

/// Persistent accounts use Keychain on iOS, a Keystore-backed provider on
/// Android, and an app-private file on desktop.
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
        #[cfg(target_os = "android")]
        {
            let bytes = android_account_storage().ok()?.read().ok()??;
            serde_json::from_slice(&bytes).ok()
        }
        #[cfg(not(any(target_os = "ios", target_os = "android")))]
        self.load_file()
    }
    pub fn save(&self, account: &Account) -> std::io::Result<()> {
        #[cfg(target_os = "ios")]
        {
            keychain::save(account)?;
            self.clear_file();
            Ok(())
        }
        #[cfg(target_os = "android")]
        {
            let bytes = serde_json::to_vec(account).map_err(std::io::Error::other)?;
            android_account_storage()?.write(&bytes)?;
            self.clear_file();
            Ok(())
        }
        #[cfg(not(any(target_os = "ios", target_os = "android")))]
        self.save_file(account)
    }
    pub fn clear(&self) {
        clear_warm_target();
        crate::cloud_direct::clear();
        #[cfg(target_os = "ios")]
        keychain::clear();
        #[cfg(target_os = "android")]
        if let Ok(store) = android_account_storage() {
            if let Err(error) = store.clear() {
                log::warn!("Android secure account removal failed: {error}");
            }
        }
        self.clear_file();
    }

    #[cfg(not(target_os = "android"))]
    fn load_file(&self) -> Option<Account> {
        let text = std::fs::read_to_string(&self.path).ok()?;
        serde_json::from_str(&text).ok()
    }
    #[cfg_attr(target_os = "ios", allow(dead_code))]
    #[cfg(not(target_os = "android"))]
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
        given_name: None,
        name: None,
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
///
/// `dismissed` reports that the in-app sign-in sheet was closed. The approval
/// may have landed just before, so one more poll runs, then this gives up
/// with an empty error (nothing to show: the person backed out).
pub fn finish_browser_login(
    start: &DeviceStart,
    cancel: impl Fn() -> bool,
    dismissed: impl Fn() -> bool,
) -> Result<Account, String> {
    wait_for_browser_login(&client(), start, &cancel, &dismissed)
}

fn wait_for_browser_login(
    client: &CloudClient,
    start: &DeviceStart,
    cancel: &dyn Fn() -> bool,
    dismissed: &dyn Fn() -> bool,
) -> Result<Account, String> {
    let mut login = start.clone();
    let deadline = Instant::now() + Duration::from_secs(if login.expires_in > 0 { login.expires_in } else { 900 });
    loop {
        let until = Instant::now() + Duration::from_secs(login.interval.max(2));
        while Instant::now() < until {
            if cancel() {
                return Err(String::new());
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        if Instant::now() > deadline {
            return Err("Sign-in expired. Try again.".into());
        }
        let last = dismissed();
        if let Some(account) = client.poll_device_login(&mut login).map_err(|e| e.message)? {
            return Ok(account);
        }
        if last {
            return Err(String::new());
        }
    }
}

/// Where "Continue with Google" should land: Google's account chooser itself.
///
/// The device flow's own link is jcode.sh/account, where the person would
/// have to press "Continue with Google" a second time. The API's Google start
/// endpoint takes the same public flow id, redirects straight to Google, and
/// approves the flow in its callback. Falls back to the account page when the
/// link carries no flow id.
pub fn google_login_url(start: &DeviceStart) -> String {
    google_login_url_at(DEFAULT_API_BASE, start)
}

pub fn google_login_url_at(api_base: &str, start: &DeviceStart) -> String {
    let link = &start.verification_uri_complete;
    let flow = link
        .split_once('?')
        .map(|(_, query)| query.split('#').next().unwrap_or(""))
        .and_then(|query| query.split('&').find_map(|pair| pair.strip_prefix("flow=")))
        .filter(|flow| {
            (16..=128).contains(&flow.len()) && flow.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        });
    match flow {
        Some(flow) => format!(
            "{}/browser/login/google/start?return=%2Faccount&flow={flow}",
            api_base.trim_end_matches('/')
        ),
        None => link.clone(),
    }
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
    let connect_started = Instant::now();
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
            // But a socket that died while the app was suspended also just
            // times out. Prove the master before keeping it, or every retry
            // would wait out the full open timeout on a dead connection.
            Err(error) if !ssh.is_closed() && error.kind != jcode_sdk::ErrorKind::Disconnected && ssh.revalidate() => {
                return Err(error);
            }
            // A dead master must go back through the authenticated control
            // plane. Its validated pins, never TOFU, authorize the next host.
            Err(_) => forget_master(&ssh),
        }
    }
    // Concurrent new tabs must not all register new keys and hit the cloud's
    // instance-connect rate limit. Recheck after waiting for another opener.
    let lock_started = Instant::now();
    let mut opening = lock_cold_connect(cancelled)?;
    crate::timing::stage("cold connect lock wait", lock_started.elapsed());
    check_cancelled()?;
    let prewarm_failed = take_overlapping_prewarm_failure(&mut opening, account, connect_started);
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
            // But a socket that died while the app was suspended also just
            // times out. Prove the master before keeping it, or every retry
            // would wait out the full open timeout on a dead connection.
            Err(error) if !ssh.is_closed() && error.kind != jcode_sdk::ErrorKind::Disconnected && ssh.revalidate() => {
                return Err(error);
            }
            Err(_) => forget_master(&ssh),
        }
    }
    check_cancelled()?;
    // Fast path: the machine is usually still awake. SSH straight to the last
    // control-plane-validated address with this device's authorized key and
    // the same pinned host keys. Any failure falls through to the control plane.
    if !prewarm_failed
        && let Some(state) = crate::cloud_direct::load_or_create(account)
        && let (Some(target), Some(key)) = (state.target(), state.key())
    {
        progress("Connecting to your Jcode Cloud machine…");
        let direct_started = Instant::now();
        match crate::cloud_ssh::CloudSsh::connect_timeout(target, key, crate::cloud_direct::DIRECT_AUTH_TIMEOUT) {
            Ok(ssh) => {
                crate::timing::stage("direct ssh", direct_started.elapsed());
                let ssh = cache_master(account, ssh);
                progress("Opening your Jcode session…");
                match ssh.open_client() {
                    Ok(client) => {
                        check_cancelled()?;
                        return Ok(client);
                    }
                    Err(_) => forget_master(&ssh),
                }
            }
            Err(error) if error.kind == jcode_sdk::ErrorKind::HostKeyMismatch => {
                // Address was reassigned. Never trust it again without the control plane.
                crate::cloud_direct::forget_target(account);
            }
            Err(_) => crate::timing::stage("direct ssh failed", direct_started.elapsed()),
        }
        check_cancelled()?;
    }
    let control_started = Instant::now();
    let mut cloud = client_for(account);
    cloud.poll_interval = Duration::from_millis(250);
    let key = crate::cloud_ssh::CloudSshKey::generate(CLIENT_NAME)?;
    progress("Checking your Jcode Cloud machine…");
    let target = match wait_for_host_or_direct(&cloud, account, &key, progress, cancelled)? {
        HostWait::Ready(target) => target,
        HostWait::Direct(ssh) => {
            crate::timing::stage("control plane wake + direct ssh", control_started.elapsed());
            let ssh = cache_master(account, ssh);
            // Provisioning is serialized, independent API hello is not. A
            // second tab may use the authenticated master immediately.
            drop(opening);
            progress("Opening your Jcode session…");
            let client = ssh.open_client()?;
            check_cancelled()?;
            return Ok(client);
        }
    };
    check_cancelled()?;
    crate::timing::stage("control plane ready", control_started.elapsed());
    progress("Connecting to your Jcode Cloud machine…");
    let master = crate::cloud_ssh::CloudSsh::connect(target.clone(), key)?;
    check_cancelled()?;
    // Authorize this device for direct reconnects, off the critical path.
    {
        let (account, master, target) = (account.clone(), master.clone(), target);
        std::thread::spawn(move || crate::cloud_direct::remember(&account, &master, &target));
    }
    let ssh = cache_master(account, master);
    drop(opening);
    progress("Opening your Jcode session…");
    let client = ssh.open_client()?;
    check_cancelled()?;
    Ok(client)
}

enum HostWait<S = crate::cloud_ssh::CloudSsh> {
    Ready(jcode_sdk::cloud::CloudTarget),
    Direct(S),
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
    let device_key = direct.and_then(|state| state.key());
    static PROBE_BUSY: OnceLock<std::sync::Arc<std::sync::atomic::AtomicBool>> = OnceLock::new();
    static REQUEST_BUSY: OnceLock<std::sync::Arc<std::sync::atomic::AtomicBool>> = OnceLock::new();
    let poll_interval = cloud.poll_interval;
    let (cloud, account_for_request, public_key) = (cloud.clone(), account.clone(), key.public_openssh().to_owned());
    let result = schedule_host_readiness(
        move || crate::timing::timed("control plane request", || {
            cloud.request_host_connect(&account_for_request, &public_key)
        }),
        device_key.map(|key| move |target: jcode_sdk::cloud::CloudTarget| {
            crate::timing::timed("wake direct ssh probe", || {
                crate::cloud_ssh::CloudSsh::connect_timeout(target, key.clone(), Duration::from_millis(1200)).ok()
            })
        }),
        PROBE_BUSY.get_or_init(|| std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false))).clone(),
        REQUEST_BUSY.get_or_init(|| std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false))).clone(),
        poll_interval,
        deadline,
        progress,
        cancelled,
        ProbeScheduling::Concurrent,
    )?;
    match result {
        HostWait::Direct((target, ssh)) => {
            crate::cloud_direct::remember_target(account, &target);
            Ok(HostWait::Direct(ssh))
        }
        HostWait::Ready(target) => Ok(HostWait::Ready(target)),
    }
}

#[derive(Clone, Copy)]
enum ProbeScheduling {
    Concurrent,
    // The old scheduling order, retained only for controlled causal fixtures.
    #[cfg(test)]
    SerialBaseline,
}

/// A cancelled caller cannot start unbounded replacement network work. Each
/// kind of worker retains its own global permit through an unread result.
struct ReadinessPermit(std::sync::Arc<std::sync::atomic::AtomicBool>);

impl Drop for ReadinessPermit {
    fn drop(&mut self) {
        self.0.store(false, std::sync::atomic::Ordering::Release);
    }
}

/// The production readiness loop. Only the request and authenticated transport
/// are injectable, so offline fixtures exercise the actual scheduling policy.
/// Polls retain the configured interval and never overlap each other. A single
/// bounded SSH probe may overlap polling, but never another speculative probe.
fn schedule_host_readiness<S: Send + 'static, R, P>(
    request: R,
    probe: Option<P>,
    probe_busy: std::sync::Arc<std::sync::atomic::AtomicBool>,
    request_busy: std::sync::Arc<std::sync::atomic::AtomicBool>,
    poll_interval: Duration,
    deadline: Instant,
    progress: &dyn Fn(&str),
    cancelled: &dyn Fn() -> bool,
    scheduling: ProbeScheduling,
) -> jcode_sdk::Result<HostWait<(jcode_sdk::cloud::CloudTarget, S)>>
where
    R: FnMut() -> jcode_sdk::Result<jcode_sdk::cloud::ConnectReply> + Send + 'static,
    P: Fn(jcode_sdk::cloud::CloudTarget) -> Option<S> + Clone + Send + 'static,
{
    let mut pending: Option<std::sync::mpsc::Receiver<_>> = None;
    let mut pending_request: Option<std::sync::mpsc::Receiver<_>> = None;
    let mut request = Some(request);
    let mut request_error = None;
    let mut next_request = Instant::now();
    let mut poll_started: Option<Instant> = None;
    let mut last = String::new();
    loop {
        if cancelled() {
            return Err(jcode_sdk::Error::new(jcode_sdk::ErrorKind::Cancelled, "Connection cancelled"));
        }
        if let Some(receive) = &pending {
            match receive.try_recv() {
                Ok((Some(ssh), _permit)) => return Ok(HostWait::Direct(ssh)),
                Ok((None, _)) | Err(std::sync::mpsc::TryRecvError::Disconnected) => pending = None,
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }
        // A failed control-plane request must not mask a still-running pinned
        // SSH fast path. Let that bounded probe finish before surfacing it.
        if pending.is_none() && let Some(error) = request_error.take() {
            return Err(error);
        }
        if Instant::now() >= deadline {
            return Err(jcode_sdk::Error::new(
                jcode_sdk::ErrorKind::StartupTimeout,
                "Jcode Cloud did not become ready in time. Retry to keep waiting.",
            ));
        }
        let mut completed = None;
        if let Some(receive) = &pending_request {
            match receive.try_recv() {
                Ok((reply, returned_request, _permit)) => {
                    request = Some(returned_request);
                    pending_request = None;
                    completed = Some(reply);
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    pending_request = None;
                    completed = Some(Err(jcode_sdk::Error::new(
                        jcode_sdk::ErrorKind::Disconnected, "Cloud readiness worker stopped",
                    )));
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }
        if completed.is_none() && pending_request.is_none() && request_error.is_none()
            && request.is_some() && Instant::now() >= next_request
            && request_busy.compare_exchange(false, true, std::sync::atomic::Ordering::AcqRel, std::sync::atomic::Ordering::Acquire).is_ok()
        {
            let permit = ReadinessPermit(request_busy.clone());
            if let Some(started) = poll_started.take() {
                crate::timing::stage("control plane poll delay", started.elapsed());
            }
            let mut owned_request = request.take().expect("request ownership checked above");
            match scheduling {
                ProbeScheduling::Concurrent => {
                    let (send, receive) = std::sync::mpsc::sync_channel(1);
                    // The HTTP request retains its original transport timeout.
                    // Losing this race drops only its receiver, not ownership
                    // of another account's state or the global worker permit.
                    std::thread::Builder::new().name("jcode-readiness-request".into()).spawn(move || {
                        let reply = owned_request();
                        let _ = send.send((reply, owned_request, permit));
                    }).map_err(|_| jcode_sdk::Error::new(
                        jcode_sdk::ErrorKind::Disconnected, "Could not start cloud readiness worker",
                    ))?;
                    pending_request = Some(receive);
                }
                #[cfg(test)]
                ProbeScheduling::SerialBaseline => {
                    completed = Some(owned_request());
                    request = Some(owned_request);
                }
            }
        }
        if let Some(reply) = completed {
            if cancelled() {
                return Err(jcode_sdk::Error::new(jcode_sdk::ErrorKind::Cancelled, "Connection cancelled"));
            }
            let reply = match reply {
                Ok(reply) => reply,
                Err(error) => {
                    request_error = Some(error);
                    continue;
                }
            };
            if reply.ready {
                return jcode_sdk::cloud::CloudTarget::from_reply(&reply).map(HostWait::Ready);
            }
            let message = reply.message.clone().unwrap_or_else(|| format!("Jcode Cloud machine is {}…", reply.state));
            if message != last {
                progress(&message);
                last = message;
            }
            if pending.is_none()
                && let Some(probe) = &probe
                && let Ok(target) = jcode_sdk::cloud::CloudTarget::from_reply(&reply)
                && probe_busy.compare_exchange(false, true, std::sync::atomic::Ordering::AcqRel, std::sync::atomic::Ordering::Acquire).is_ok()
            {
                let permit = ReadinessPermit(probe_busy.clone());
                let probe = probe.clone();
                match scheduling {
                    ProbeScheduling::Concurrent => {
                        let (send, receive) = std::sync::mpsc::sync_channel(1);
                        // Spawn failure loses only the speculative fast path.
                        // Dropping the closure releases its permit in that case.
                        if std::thread::Builder::new().name("jcode-readiness-probe".into()).spawn(move || {
                            let result = probe(target.clone()).map(|ssh| (target, ssh));
                            // Receiver drop disposes of a late SSH master and
                            // its permit without caching or remembering it.
                            let _ = send.send((result, permit));
                        }).is_ok() {
                            pending = Some(receive);
                        }
                    }
                    #[cfg(test)]
                    ProbeScheduling::SerialBaseline => {
                        let _permit = permit;
                        if let Some(ssh) = probe(target.clone()) {
                            return Ok(HostWait::Direct((target, ssh)));
                        }
                    }
                }
            }
            let now = Instant::now();
            next_request = now + poll_interval;
            poll_started = Some(now);
        }
        // Neither a slow HTTP response nor SSH authentication blocks the other
        // winner or foreground cancellation. No network I/O holds this loop.
        std::thread::sleep(Duration::from_millis(10).min(deadline.saturating_duration_since(Instant::now())));
    }
}

#[cfg(test)]
#[path = "cloud_startup_tests.rs"]
mod startup_tests;

/// Speculatively prepare a launch: SSH straight to the remembered machine and
/// pre-open one API channel (with hello done), so the next `connect` takes
/// it without any network round trip. Never contacts the control plane and
/// never wakes a sleeping machine. Safe to call often (app start, foreground).
pub fn prewarm(account: &Account) {
    let account = account.clone();
    let _ = std::thread::Builder::new().name("jcode-cloud-prewarm".into()).spawn(move || {
        let cached = {
            let cache = warm_target().lock().unwrap();
            cache.as_ref().and_then(|entry| entry.for_account(&account, Instant::now()))
        };
        if let Some(ssh) = cached.filter(|s| !s.is_closed()) {
            // Foregrounding after suspension leaves sockets that look open
            // but are dead. Prove the master now, not when the user taps a
            // session (which would otherwise wait 2s and then go cold).
            if ssh.revalidate() {
                return;
            }
            forget_master(&ssh);
        }
        // A real connect in progress already does this work.
        let Ok(mut opening) = COLD_CONNECT.try_lock() else { return };
        *opening = None;
        let started = Instant::now();
        if let Some(state) = crate::cloud_direct::load_or_create(&account)
            && let (Some(target), Some(key)) = (state.target(), state.key())
        {
            match crate::cloud_ssh::CloudSsh::connect_timeout(target, key, crate::cloud_direct::DIRECT_AUTH_TIMEOUT) {
                Ok(ssh) => {
                    crate::timing::stage("prewarm direct ssh", started.elapsed());
                    cache_master(&account, ssh).refill();
                }
                Err(_) => {
                    // A foreground opener waiting on this lock must not repeat
                    // the exact speculative attempt before using the control
                    // plane. Later independent calls retain the normal retry.
                    *opening = Some(FailedPrewarm { account, finished: Instant::now() });
                }
            }
        }
    });
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

    fn prewarm_fixture_account() -> Account {
        serde_json::from_value(serde_json::json!({
            "api_key": "fixture-only", "api_base": "http://127.0.0.1:1"
        })).unwrap()
    }

    #[test]
    fn prewarm_failure_is_shared_only_with_overlapping_same_account_call() {
        let account = prewarm_fixture_account();
        let started = Instant::now();
        let finished = started + Duration::from_millis(10);
        let marker = || Some(FailedPrewarm { account: account.clone(), finished });
        let mut failure = marker();
        assert!(take_overlapping_prewarm_failure(&mut failure, &account, started));
        assert!(!take_overlapping_prewarm_failure(&mut failure, &account, started));
        assert!(!take_overlapping_prewarm_failure(&mut marker(), &account, finished + Duration::from_nanos(1)));
        for other in [
            Account { api_key: "other-fixture".into(), ..account.clone() },
            Account { api_base: "http://127.0.0.1:2".into(), ..account.clone() },
        ] {
            assert!(!take_overlapping_prewarm_failure(&mut marker(), &other, started));
        }
    }

    /// Offline timing fixture, not a device speedup: an in-flight failed
    /// prewarm holds the same serialization lock as a foreground opener.
    #[test]
    fn overlapping_failed_prewarm_avoids_duplicate_probe_fixture() {
        use std::sync::{Arc, Barrier, atomic::{AtomicUsize, Ordering}};
        fn run(share_failure: bool) -> (usize, Duration) {
            let state = Arc::new(Mutex::new(None));
            let entered = Arc::new(Barrier::new(2));
            let release = Arc::new(Barrier::new(2));
            let probes = Arc::new(AtomicUsize::new(0));
            let account = prewarm_fixture_account();
            let worker = {
                let (state, entered, release, probes, account) =
                    (state.clone(), entered.clone(), release.clone(), probes.clone(), account.clone());
                std::thread::spawn(move || {
                    let mut guard = state.lock().unwrap();
                    entered.wait();
                    release.wait();
                    probes.fetch_add(1, Ordering::Relaxed);
                    std::thread::sleep(Duration::from_millis(80));
                    *guard = Some(FailedPrewarm { account, finished: Instant::now() });
                })
            };
            entered.wait();
            let started = Instant::now();
            release.wait();
            let mut guard = state.lock().unwrap();
            let shared = share_failure && take_overlapping_prewarm_failure(&mut guard, &account, started);
            if !shared {
                probes.fetch_add(1, Ordering::Relaxed);
                std::thread::sleep(Duration::from_millis(80));
            }
            let elapsed = started.elapsed();
            drop(guard);
            worker.join().unwrap();
            (probes.load(Ordering::Relaxed), elapsed)
        }
        let (before_probes, before) = run(false);
        let (after_probes, after) = run(true);
        assert_eq!(before_probes, 2);
        assert_eq!(after_probes, 1);
        // Exact counts, not scheduler-sensitive timing thresholds, establish
        // removal of one complete 80ms simulated authentication attempt.
        eprintln!("prewarm_overlap_fixture before_ms={:.2} after_ms={:.2} before_probes={before_probes} after_probes={after_probes}", before.as_secs_f64() * 1000.0, after.as_secs_f64() * 1000.0);
    }

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
        let acct = Account {
            api_key: "k".into(),
            email: Some("e@x".into()),
            given_name: Some("Ada".into()),
            name: Some("Ada Lovelace".into()),
            api_base: DEFAULT_API_BASE.into(),
        };
        store.save(&acct).unwrap();
        assert_eq!(store.load(), Some(acct));
        store.clear();
        assert!(store.load().is_none());
    }

    #[test]
    fn account_store_loads_legacy_json_without_names() {
        let dir = std::env::temp_dir().join(format!("jm-acct-legacy-{}", std::process::id()));
        let store = AccountStore::new(dir.join("a.json"));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            &store.path,
            serde_json::json!({ "api_key": "legacy-key", "email": "legacy@example.test", "api_base": DEFAULT_API_BASE }).to_string(),
        ).unwrap();
        let account = store.load_file().expect("legacy account remains readable");
        assert_eq!(account.api_key, "legacy-key");
        assert_eq!(account.email.as_deref(), Some("legacy@example.test"));
        assert_eq!(account.api_base, DEFAULT_API_BASE);
        assert_eq!(account.given_name, None);
        assert_eq!(account.name, None);
        store.clear_file();
        std::fs::remove_dir(dir).unwrap();
    }

    #[test]
    fn warm_master_is_scoped_to_credentials_endpoint_and_expiry() {
        let account = Account { given_name: None, name: None, api_key: "key-a".into(), email: None, api_base: DEFAULT_API_BASE.into() };
        let now = Instant::now();
        let entry = WarmTarget { account: account.clone(), ssh: 7, expires: now + TARGET_TTL };
        assert_eq!(entry.for_account(&account, now), Some(7));
        let mut profile_updated = account.clone();
        profile_updated.given_name = Some("Jeremy".into());
        profile_updated.name = Some("Jeremy Huang".into());
        assert_eq!(entry.for_account(&profile_updated, now), Some(7));
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
            Account { given_name: None, name: None, api_key: "unused".into(), email: None, api_base: "http://127.0.0.1:1".into() };
        let result = connect(&account, &|_| panic!("cancelled connection reported progress"), &|| true);
        assert!(matches!(result, Err(e) if e.kind == jcode_sdk::ErrorKind::Cancelled));
    }

    #[test]
    fn cloud_client_uses_compatible_protocol_identity() {
        assert_eq!(client().client_name, "jcode-cli");
        assert_eq!(client().api_base, DEFAULT_API_BASE);
        let account = Account {
            given_name: None,
            name: None,
            api_key: "unused".into(),
            email: None,
            api_base: "https://cloud.example/v1".into(),
        };
        assert_eq!(client_for(&account).client_name, "jcode-cli");
        assert_eq!(client_for(&account).api_base, account.api_base);
        assert_eq!(CLIENT_NAME, if cfg!(target_os = "android") { "jcode-android" } else { "jcode-ios" });
    }

    #[test]
    fn google_login_goes_straight_to_the_account_chooser() {
        let start = |link: &str| DeviceStart {
            device_code: "secret-device-code".into(),
            verification_uri_complete: link.into(),
            interval: 3,
            expires_in: 600,
        };
        let direct = google_login_url(&start("https://jcode.sh/account?flow=AbC_def-0123456789xyz"));
        assert_eq!(
            direct,
            "https://api.jcode.sh/v1/browser/login/google/start?return=%2Faccount&flow=AbC_def-0123456789xyz"
        );
        assert!(!direct.contains("secret-device-code"));
        assert_eq!(
            google_login_url_at("https://dev.example/v1/", &start("https://jcode.sh/account?x=1&flow=AbC_def-0123456789xyz#top")),
            "https://dev.example/v1/browser/login/google/start?return=%2Faccount&flow=AbC_def-0123456789xyz"
        );
        // No usable flow id: keep the server's own link rather than guess.
        for link in [
            "https://jcode.sh/account",
            "https://jcode.sh/account?flow=short",
            "https://jcode.sh/account?flow=bad%2Fchars0123456789",
            "https://jcode.sh/account?flow=AbC_def-0123456789xyz%26return=//evil",
        ] {
            assert_eq!(google_login_url(&start(link)), link);
        }
    }

    /// Serves one canned JSON reply per expected request, in order.
    fn canned_server(replies: Vec<(u16, &'static str)>) -> (String, std::thread::JoinHandle<()>) {
        use std::io::{BufRead, BufReader, Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let api_base = format!("http://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            for (status, reply) in replies {
                let (mut stream, _) = listener.accept().unwrap();
                stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                let mut reader = BufReader::new(&mut stream);
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                assert!(line.starts_with("POST /auth/token HTTP/1.1"), "{line}");
                let mut length = 0;
                loop {
                    line.clear();
                    assert_ne!(reader.read_line(&mut line).unwrap(), 0);
                    if line == "\r\n" { break; }
                    if let Some((name, value)) = line.split_once(':') {
                        if name.eq_ignore_ascii_case("content-length") { length = value.trim().parse().unwrap(); }
                    }
                }
                let mut body = vec![0; length];
                reader.read_exact(&mut body).unwrap();
                write!(stream, "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}", reply.len()).unwrap();
            }
        });
        (api_base, server)
    }

    const PENDING: (u16, &str) = (428, r#"{"error":{"code":"authorization_pending","message":"user has not approved yet"}}"#);

    #[test]
    fn browser_login_stops_after_one_more_poll_once_the_sheet_is_dismissed() {
        let start = DeviceStart { device_code: "d".into(), verification_uri_complete: "u".into(), interval: 2, expires_in: 600 };
        // Dismissed and still pending: exactly one poll, then a silent stop.
        let (api_base, server) = canned_server(vec![PENDING]);
        let result = wait_for_browser_login(&client().with_api_base(api_base), &start, &|| false, &|| true);
        assert_eq!(result.unwrap_err(), "");
        server.join().unwrap();
        // Approval that landed just before the dismissal still signs in.
        let (api_base, server) = canned_server(vec![(200, r#"{"api_key":"key-1","email":"a@b.c"}"#)]);
        let account = wait_for_browser_login(&client().with_api_base(api_base.clone()), &start, &|| false, &|| true).unwrap();
        assert_eq!((account.api_key.as_str(), account.email.as_deref()), ("key-1", Some("a@b.c")));
        server.join().unwrap();
        // Cancel never reaches the network.
        let result = wait_for_browser_login(&client().with_api_base(api_base), &start, &|| true, &|| false);
        assert_eq!(result.unwrap_err(), "");
    }

    #[test]
    fn android_account_store_requires_secure_provider() {
        // No Android provider is installed in host tests. Android bootstrap
        // must install one before saving, never fall back to account.json.
        match android_account_storage() {
            Ok(_) => panic!("host test unexpectedly installed an Android credential provider"),
            Err(error) => assert_eq!(error.kind(), std::io::ErrorKind::NotConnected),
        }
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
