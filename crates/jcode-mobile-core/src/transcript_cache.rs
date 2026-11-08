//! On-disk cache of each session's last History snapshot.
//!
//! Opening a session used to show nothing until the full transcript
//! (0.2-1.2 MB) came back over SSH. Now the connection emits the cached
//! snapshot immediately and the fresh one replaces it when it arrives.
//! Images are not cached (size). Only the newest `KEEP` sessions are kept.
use jcode_sdk::ApiEvent;
use std::path::PathBuf;

const KEEP: usize = 40;

fn dir() -> Option<PathBuf> {
    #[cfg(target_os = "ios")]
    let base = std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Caches"));
    #[cfg(not(target_os = "ios"))]
    let base = dirs::cache_dir().map(|d| d.join("jcode-mobile"));
    Some(base?.join("transcripts"))
}

fn file(session_id: &str) -> Option<PathBuf> {
    // Session ids are server-generated; refuse anything path-like.
    if session_id.is_empty()
        || !session_id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return None;
    }
    Some(dir()?.join(format!("{session_id}.json")))
}

/// The cached History event for `session_id`, if any.
pub fn load(session_id: &str) -> Option<ApiEvent> {
    let started = std::time::Instant::now();
    let bytes = std::fs::read(file(session_id)?).ok()?;
    let event: ApiEvent = serde_json::from_slice(&bytes).ok()?;
    let ApiEvent::History { session_id: cached, .. } = &event else { return None };
    if cached != session_id {
        return None;
    }
    crate::timing::stage("transcript cache load", started.elapsed());
    Some(event)
}

/// Save a History event in the background. Non-History events are ignored.
pub fn store(event: &ApiEvent) {
    let ApiEvent::History { session_id, messages, .. } = event else { return };
    let Some(path) = file(session_id) else { return };
    let event = ApiEvent::History {
        session_id: session_id.clone(),
        messages: messages.clone(),
        images: Vec::new(),
    };
    std::thread::spawn(move || {
        let Ok(bytes) = serde_json::to_vec(&event) else { return };
        let Some(dir) = path.parent() else { return };
        if std::fs::create_dir_all(dir).is_err() {
            return;
        }
        // Write then rename so a crash never leaves a torn file.
        let tmp = path.with_extension("tmp");
        if std::fs::write(&tmp, &bytes).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
        prune(dir);
    });
}

fn prune(dir: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut files: Vec<_> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect();
    if files.len() <= KEEP {
        return;
    }
    files.sort_by(|a, b| b.0.cmp(&a.0));
    for (_, path) in files.into_iter().skip(KEEP) {
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_path_like_ids() {
        assert!(file("../etc/passwd").is_none());
        assert!(file("a/b").is_none());
        assert!(file("").is_none());
        assert!(file("session_abc-123").is_some());
    }
}
