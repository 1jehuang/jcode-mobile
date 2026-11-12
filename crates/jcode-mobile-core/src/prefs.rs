//! User preferences that outlive a session. Kept separate from the account
//! (which lives in the Keychain on iOS) because none of it is secret.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Prefs {
    /// Cloud directory new sessions start in when none is chosen explicitly.
    /// `None` keeps the server default (the cloud machine's home).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_dir: Option<String>,
}

impl Prefs {
    /// Normalized setter: trims, drops a trailing slash (but keeps `/`), and
    /// treats an empty value as "no default".
    pub fn set_default_dir(&mut self, dir: Option<&str>) {
        self.default_dir = dir.and_then(normalize_dir);
    }
}

/// Trim and drop a trailing slash. Only absolute paths are accepted: the
/// server uses the path literally, so `~` and relative paths would not resolve.
pub fn normalize_dir(dir: &str) -> Option<String> {
    let dir = dir.trim();
    if !dir.starts_with('/') || dir.contains('\0') {
        return None;
    }
    let trimmed = dir.trim_end_matches('/');
    Some(if trimmed.is_empty() { "/".into() } else { trimmed.into() })
}

pub struct PrefsStore {
    path: PathBuf,
}

impl PrefsStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// `<data dir>/jcode-mobile/prefs.json`, beside `account.json`.
    pub fn default_location() -> Self {
        let base = dirs::data_dir().unwrap_or_else(std::env::temp_dir);
        Self::new(base.join("jcode-mobile").join("prefs.json"))
    }

    /// Missing or unreadable prefs fall back to defaults, never an error.
    pub fn load(&self) -> Prefs {
        std::fs::read_to_string(&self.path)
            .ok()
            .and_then(|text| serde_json::from_str::<Prefs>(&text).ok())
            .map(|mut p| {
                p.default_dir = p.default_dir.as_deref().and_then(normalize_dir);
                p
            })
            .unwrap_or_default()
    }

    pub fn save(&self, prefs: &Prefs) -> std::io::Result<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = self.path.with_extension("tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(prefs).map_err(std::io::Error::other)?)?;
        std::fs::rename(tmp, &self.path)
    }

    pub fn clear(&self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (PrefsStore, PathBuf) {
        let dir = std::env::temp_dir().join(format!("jm-prefs-{}", uuid::Uuid::new_v4()));
        (PrefsStore::new(dir.join("prefs.json")), dir)
    }

    #[test]
    fn missing_file_is_default() {
        let (s, dir) = store();
        assert_eq!(s.load(), Prefs::default());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn round_trips_default_dir() {
        let (s, dir) = store();
        let mut p = Prefs::default();
        p.set_default_dir(Some("  /home/jeremy/jcode-mobile/  "));
        assert_eq!(p.default_dir.as_deref(), Some("/home/jeremy/jcode-mobile"));
        s.save(&p).unwrap();
        assert_eq!(s.load(), p);
        p.set_default_dir(Some(""));
        s.save(&p).unwrap();
        assert_eq!(s.load().default_dir, None);
        assert_eq!(std::fs::read_to_string(dir.join("prefs.json")).unwrap().trim(), "{}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn normalization_rejects_relative_and_empty() {
        assert_eq!(normalize_dir("/"), Some("/".into()));
        assert_eq!(normalize_dir("///"), Some("/".into()));
        assert_eq!(normalize_dir("~/src/app/"), None, "server does not expand ~");
        assert_eq!(normalize_dir("src/app"), None);
        assert_eq!(normalize_dir("   "), None);
        assert_eq!(normalize_dir("/a\0b"), None);
    }

    #[test]
    fn corrupt_or_invalid_file_falls_back() {
        let (s, dir) = store();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("prefs.json"), "not json").unwrap();
        assert_eq!(s.load(), Prefs::default());
        std::fs::write(dir.join("prefs.json"), r#"{"default_dir":"relative/path","future_field":1}"#).unwrap();
        assert_eq!(s.load(), Prefs::default());
        let _ = std::fs::remove_dir_all(dir);
    }
}
