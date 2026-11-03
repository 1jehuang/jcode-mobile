//! Paired-server credentials. JSON file under the platform data dir for now;
//! the iOS shell will swap in a Keychain-backed store behind the same trait.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::gateway::Gateway;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ServerCredential {
    pub host: String,
    pub port: u16,
    pub token: String,
    pub server_name: String,
    pub server_version: String,
    #[serde(default)]
    pub paired_at: u64,
    #[serde(default)]
    pub working_dir: Option<String>,
}

impl ServerCredential {
    pub fn id(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
    pub fn gateway(&self) -> Gateway {
        Gateway::new(self.host.clone(), self.port)
    }
}

pub trait CredentialStore: Send + Sync {
    fn load_all(&self) -> Vec<ServerCredential>;
    fn save(&self, credential: ServerCredential);
    fn remove(&self, id: &str);
}

pub struct FileCredentialStore {
    path: PathBuf,
}

impl FileCredentialStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// `$XDG_DATA_HOME/jcode-mobile/servers.json` (or the platform equivalent).
    pub fn default_location() -> Self {
        let base = dirs::data_dir().unwrap_or_else(std::env::temp_dir);
        Self::new(base.join("jcode-mobile").join("servers.json"))
    }

    fn write(&self, all: &[ServerCredential]) {
        if let Some(dir) = self.path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let tmp = self.path.with_extension("tmp");
        if std::fs::write(&tmp, serde_json::to_vec_pretty(all).unwrap()).is_ok() {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
            }
            let _ = std::fs::rename(&tmp, &self.path);
        }
    }
}

impl CredentialStore for FileCredentialStore {
    fn load_all(&self) -> Vec<ServerCredential> {
        std::fs::read(&self.path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    fn save(&self, credential: ServerCredential) {
        let mut all = self.load_all();
        all.retain(|c| c.id() != credential.id());
        all.insert(0, credential);
        self.write(&all);
    }

    fn remove(&self, id: &str) {
        let mut all = self.load_all();
        all.retain(|c| c.id() != id);
        self.write(&all);
    }
}

/// Stable per-install device id, persisted next to the credentials.
pub fn device_id() -> String {
    let base = dirs::data_dir().unwrap_or_else(std::env::temp_dir).join("jcode-mobile");
    let path = base.join("device_id");
    if let Ok(id) = std::fs::read_to_string(&path)
        && !id.trim().is_empty()
    {
        return id.trim().to_string();
    }
    let id = uuid::Uuid::new_v4().to_string();
    let _ = std::fs::create_dir_all(&base);
    let _ = std::fs::write(&path, &id);
    id
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_dedupes_and_orders_newest_first() {
        let dir = std::env::temp_dir().join(format!("jm-cred-{}", uuid::Uuid::new_v4()));
        let store = FileCredentialStore::new(dir.join("s.json"));
        let c = |host: &str, token: &str| ServerCredential {
            host: host.into(),
            port: 7643,
            token: token.into(),
            server_name: "x".into(),
            server_version: "v".into(),
            paired_at: 0,
            working_dir: None,
        };
        store.save(c("a", "1"));
        store.save(c("b", "2"));
        store.save(c("a", "3"));
        let all = store.load_all();
        assert_eq!(all.len(), 2);
        assert_eq!((all[0].host.as_str(), all[0].token.as_str()), ("a", "3"));
        store.remove("a:7643");
        assert_eq!(store.load_all().len(), 1);
        let _ = std::fs::remove_dir_all(dir);
    }
}
