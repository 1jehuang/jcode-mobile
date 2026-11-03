//! Jcode Cloud: account sign-in and the connection to the account's managed
//! host. The protocol lives in `jcode_sdk::cloud` (control plane) and
//! `jcode_sdk::NativeSshTransport` (pure-Rust SSH, no `ssh` binary, so it runs
//! on iOS). This module keeps the app-facing API small and string-errored and
//! owns on-device account persistence.

use std::path::PathBuf;

pub use jcode_sdk::cloud::{Account, CloudClient, DEFAULT_API_BASE, DeviceLogin};

/// Kept for API stability: the browser sign-in handle.
pub type DeviceStart = DeviceLogin;

/// Identifies this app to jcode.sh in sign-in and connect requests.
pub const CLIENT_NAME: &str = "jcode-ios";

pub fn client() -> CloudClient {
    CloudClient::new(CLIENT_NAME)
}

fn client_for(account: &Account) -> CloudClient {
    client().with_api_base(account.api_base.clone())
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
    Err(if message.is_empty() { format!("Account deletion failed (HTTP {status})") } else { message.to_string() })
}

/// Wake the account's host and open an SDK client over in-process SSH.
pub fn connect(
    account: &Account,
    progress: &dyn Fn(&str),
    cancelled: &dyn Fn() -> bool,
) -> jcode_sdk::Result<jcode_sdk::JcodeClient> {
    jcode_sdk::cloud::connect(&client_for(account), account, progress, cancelled)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_store_roundtrip() {
        let dir = std::env::temp_dir().join(format!("jm-acct-{}", std::process::id()));
        let store = AccountStore::new(dir.join("a.json"));
        let acct = Account { api_key: "k".into(), email: Some("e@x".into()), api_base: DEFAULT_API_BASE.into() };
        store.save(&acct).unwrap();
        assert_eq!(store.load(), Some(acct));
        store.clear();
        assert!(store.load().is_none());
    }

    #[test]
    fn client_identifies_as_ios_app() {
        assert_eq!(client().client_name, "jcode-ios");
        assert_eq!(client().api_base, DEFAULT_API_BASE);
    }
}
