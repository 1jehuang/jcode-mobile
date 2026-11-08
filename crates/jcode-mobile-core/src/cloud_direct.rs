//! Direct reconnect to an awake cloud machine, skipping the control plane.
//!
//! The control plane round trip (EC2 describe, console scrape, Instance
//! Connect key push) costs about 1.7s, and Instance Connect key lookup on the
//! host adds about 0.5s to SSH auth. After one successful control-plane
//! connect, this device authorizes a persistent Ed25519 key on its own
//! machine and remembers the pinned target. Later launches SSH straight to
//! the last address, still verifying the pinned host keys, and fall back to
//! the control plane if anything fails (machine asleep, address changed).

use crate::cloud::Account;
use crate::cloud_ssh::{CloudSsh, CloudSshKey};
use jcode_sdk::cloud::CloudTarget;
use serde::{Deserialize, Serialize};
use std::hash::{Hash, Hasher};
use std::time::Duration;

/// Budget for TCP, handshake and auth on the fast path. An asleep machine or
/// a reassigned address must not delay the control-plane fallback much.
pub(crate) const DIRECT_AUTH_TIMEOUT: Duration = Duration::from_millis(1500);
const KEY_COMMENT: &str = "jcode-device";

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub(crate) struct DirectState {
    account: u64,
    private_key: String,
    #[serde(default)]
    authorized: bool,
    #[serde(default)]
    target: Option<SavedTarget>,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
struct SavedTarget {
    address: std::net::IpAddr,
    port: u16,
    user: String,
    host_keys: Vec<String>,
}

fn account_id(account: &Account) -> u64 {
    // SipHash with fixed keys: stable, and avoids storing the API key twice.
    let mut h = std::collections::hash_map::DefaultHasher::new();
    account.api_base.hash(&mut h);
    account.api_key.hash(&mut h);
    h.finish()
}

impl DirectState {
    pub(crate) fn key(&self) -> Option<CloudSshKey> {
        CloudSshKey::from_private_openssh(&self.private_key, KEY_COMMENT)
    }

    pub(crate) fn target(&self) -> Option<CloudTarget> {
        if !self.authorized {
            return None;
        }
        let t = self.target.as_ref()?;
        Some(CloudTarget { address: t.address, port: t.port, user: t.user.clone(), host_keys: t.host_keys.clone() })
    }
}

/// Load this account's state, creating a fresh device key when absent.
pub(crate) fn load_or_create(account: &Account) -> Option<DirectState> {
    let id = account_id(account);
    if let Some(state) = store::load().filter(|s| s.account == id && s.key().is_some()) {
        return Some(state);
    }
    let key = CloudSshKey::generate(KEY_COMMENT).ok()?;
    let state = DirectState { account: id, private_key: key.private_openssh()?, authorized: false, target: None };
    store::save(&state).ok()?;
    Some(state)
}

/// Record a control-plane-validated target and authorize the device key on it.
/// Runs over an already authenticated master, so it costs one channel.
pub(crate) fn remember(account: &Account, ssh: &CloudSsh, target: &CloudTarget) {
    let Some(mut state) = load_or_create(account) else { return };
    let Some(key) = state.key() else { return };
    if ssh.authorize_device_key(key.public_openssh()).is_err() {
        return;
    }
    state.authorized = true;
    state.target = Some(SavedTarget {
        address: target.address,
        port: target.port,
        user: target.user.clone(),
        host_keys: target.host_keys.clone(),
    });
    let _ = store::save(&state);
}

/// Update the saved target after a pinned direct login succeeded at a new
/// control-plane-provided address (public IPs change across stop/start).
pub(crate) fn remember_target(account: &Account, target: &CloudTarget) {
    if let Some(mut state) = store::load().filter(|s| s.account == account_id(account) && s.authorized) {
        state.target = Some(SavedTarget {
            address: target.address,
            port: target.port,
            user: target.user.clone(),
            host_keys: target.host_keys.clone(),
        });
        let _ = store::save(&state);
    }
}

/// Forget the cached address (e.g. host key mismatch). Keeps the device key.
pub(crate) fn forget_target(account: &Account) {
    if let Some(mut state) = store::load().filter(|s| s.account == account_id(account)) {
        state.target = None;
        let _ = store::save(&state);
    }
}

pub(crate) fn clear() {
    store::clear();
}

#[cfg(not(target_os = "ios"))]
mod store {
    use super::*;
    use std::path::PathBuf;

    fn path() -> PathBuf {
        dirs::data_dir().unwrap_or_else(std::env::temp_dir).join("jcode-mobile").join("cloud-direct.json")
    }

    pub(super) fn load() -> Option<DirectState> {
        serde_json::from_str(&std::fs::read_to_string(path()).ok()?).ok()
    }

    pub(super) fn save(state: &DirectState) -> std::io::Result<()> {
        use std::io::Write;
        let path = path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("tmp");
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        opts.open(&tmp)?.write_all(serde_json::to_string(state).map_err(std::io::Error::other)?.as_bytes())?;
        std::fs::rename(tmp, path)
    }

    pub(super) fn clear() {
        let _ = std::fs::remove_file(path());
    }
}

/// The private key lives in the Keychain, like the account itself.
#[cfg(target_os = "ios")]
mod store {
    use super::*;
    use security_framework::passwords::{delete_generic_password, get_generic_password, set_generic_password};

    const SERVICE: &str = "sh.jcode.mobile";
    const ACCOUNT: &str = "cloud-direct";

    pub(super) fn load() -> Option<DirectState> {
        serde_json::from_slice(&get_generic_password(SERVICE, ACCOUNT).ok()?).ok()
    }
    pub(super) fn save(state: &DirectState) -> std::io::Result<()> {
        let bytes = serde_json::to_vec(state).map_err(std::io::Error::other)?;
        set_generic_password(SERVICE, ACCOUNT, &bytes).map_err(|e| std::io::Error::other(e.to_string()))
    }
    pub(super) fn clear() {
        let _ = delete_generic_password(SERVICE, ACCOUNT);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_roundtrips_and_target_requires_authorization() {
        let key = CloudSshKey::generate(KEY_COMMENT).unwrap();
        let mut state = DirectState {
            account: 1,
            private_key: key.private_openssh().unwrap(),
            authorized: false,
            target: Some(SavedTarget {
                address: "127.0.0.1".parse().unwrap(),
                port: 22,
                user: "ec2-user".into(),
                host_keys: vec!["ssh-ed25519 AAAA".into()],
            }),
        };
        assert_eq!(state.key().unwrap().public_openssh(), key.public_openssh());
        assert!(state.target().is_none());
        state.authorized = true;
        assert_eq!(state.target().unwrap().user, "ec2-user");
        let json = serde_json::to_string(&state).unwrap();
        assert_eq!(serde_json::from_str::<DirectState>(&json).unwrap(), state);
    }

    #[test]
    fn account_identity_depends_on_key_and_endpoint() {
        let a = Account { api_key: "k".into(), email: None, api_base: "https://a".into() };
        let b = Account { api_key: "k2".into(), ..a.clone() };
        let c = Account { api_base: "https://b".into(), ..a.clone() };
        assert_ne!(account_id(&a), account_id(&b));
        assert_ne!(account_id(&a), account_id(&c));
        assert_eq!(account_id(&a), account_id(&a.clone()));
    }
}
