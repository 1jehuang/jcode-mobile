//! iOS system surfaces outside the app window: deep links (share extension,
//! home-screen quick actions, Shortcuts "Open URL", Live Activity taps) and
//! the Live Activity / Dynamic Island.
//!
//! URL scheme (registered in Info.plist by scripts/ios-bundle.sh):
//!   jcode://new                 new session
//!   jcode://new?text=<prompt>   new session, send <prompt> once connected
//!   jcode://ask?text=<prompt>   same as new?text= (Shortcuts "Ask jcode")
//!   jcode://session/<id>        focus or resume that session
//!
//! The Live Activity lives in a Swift widget extension (ios-ext/). Rust drives
//! it through `JcodeBridge.framework`, loaded with `dlopen` so the app still
//! runs when the framework is absent (simulator, Linux preview).

use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use std::sync::Mutex;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeepLink {
    NewSession { prompt: Option<String> },
    OpenSession { session_id: String },
}

static TX: Mutex<Option<UnboundedSender<DeepLink>>> = Mutex::new(None);

/// Channel of parsed deep links. Call once from the app. Also donates the
/// "New jcode session" / "Ask jcode" Shortcuts actions.
pub fn install() -> UnboundedReceiver<DeepLink> {
    let (tx, rx) = unbounded();
    *TX.lock().unwrap() = Some(tx);
    imp::donate_shortcuts();
    rx
}

/// Platform URL callback (registered on the Application before `run`).
pub fn handle_urls(urls: Vec<String>) {
    for url in urls {
        match parse(&url) {
            Some(link) => {
                if let Some(tx) = TX.lock().unwrap().as_ref() {
                    let _ = tx.unbounded_send(link);
                }
            }
            None => log::warn!("deep link: ignored {url}"),
        }
    }
}

pub fn parse(url: &str) -> Option<DeepLink> {
    let rest = url.strip_prefix("jcode://")?;
    let (path, query) = rest.split_once('?').unwrap_or((rest, ""));
    let path = path.trim_end_matches('/');
    let param = |key: &str| {
        query.split('&').find_map(|kv| {
            let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
            (k == key).then(|| percent_decode(v))
        })
    };
    match path {
        "new" | "ask" => {
            let prompt = param("text").map(|t| t.trim().to_string()).filter(|t| !t.is_empty());
            Some(DeepLink::NewSession { prompt })
        }
        _ => {
            let id = path.strip_prefix("session/")?;
            let id = percent_decode(id);
            (!id.is_empty()).then_some(DeepLink::OpenSession { session_id: id })
        }
    }
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < b.len() => {
                match u8::from_str_radix(std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or("zz"), 16) {
                    Ok(v) => {
                        out.push(v);
                        i += 2;
                    }
                    Err(_) => out.push(b'%'),
                }
            }
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// What the Live Activity shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveState {
    pub session_id: String,
    pub title: String,
    /// "thinking", "responding", "tools".
    pub activity: &'static str,
    pub detail: String,
}

/// Derive the Live Activity state for a session (None when idle).
pub fn live_state(s: &jcode_mobile_core::SessionState) -> Option<LiveState> {
    use jcode_mobile_core::session::ToolStatus;
    if !s.is_processing {
        return None;
    }
    let running = s
        .transcript
        .iter()
        .rev()
        .take(4)
        .flat_map(|e| e.tool_calls.iter())
        .find(|t| matches!(t.status, ToolStatus::Running | ToolStatus::StreamingInput));
    let (activity, detail) = if let Some(t) = running {
        ("tools", t.name.clone())
    } else if s.is_reasoning {
        ("thinking", String::new())
    } else {
        ("responding", String::new())
    };
    Some(LiveState {
        session_id: s.session_id.clone().unwrap_or_default(),
        title: s.session_title.clone().filter(|t| !t.trim().is_empty()).unwrap_or_else(|| "jcode".into()),
        activity,
        detail,
    })
}

/// Push `state` to the Live Activity if it changed. `None` ends it.
pub fn sync_live(state: Option<LiveState>) {
    static LAST: Mutex<Option<LiveState>> = Mutex::new(None);
    let mut last = LAST.lock().unwrap();
    if *last == state {
        return;
    }
    let started = match (&*last, &state) {
        (Some(a), Some(b)) => a.session_id == b.session_id && a.title == b.title,
        _ => false,
    };
    *last = state.clone();
    drop(last);
    imp::apply(state, started);
}

#[cfg(target_os = "ios")]
mod imp {
    use super::LiveState;
    use std::ffi::{CString, c_char, c_void};
    use std::sync::OnceLock;

    type Start = unsafe extern "C" fn(*const c_char, *const c_char, *const c_char, *const c_char) -> i32;
    type Update = unsafe extern "C" fn(*const c_char, *const c_char);
    type End = unsafe extern "C" fn();

    struct Bridge {
        start: Start,
        update: Update,
        end: End,
        donate: Option<End>,
    }

    unsafe extern "C" {
        fn dlopen(path: *const c_char, mode: i32) -> *mut c_void;
        fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    }

    fn bridge() -> Option<&'static Bridge> {
        static B: OnceLock<Option<Bridge>> = OnceLock::new();
        B.get_or_init(|| unsafe {
            let path = c"@executable_path/Frameworks/JcodeBridge.framework/JcodeBridge";
            let h = dlopen(path.as_ptr(), 1);
            if h.is_null() {
                log::info!("live activity: JcodeBridge not bundled");
                return None;
            }
            let sym = |n: &std::ffi::CStr| dlsym(h, n.as_ptr());
            let (s, u, e) = (sym(c"jcode_live_start"), sym(c"jcode_live_update"), sym(c"jcode_live_end"));
            if s.is_null() || u.is_null() || e.is_null() {
                return None;
            }
            Some(Bridge {
                start: std::mem::transmute::<*mut c_void, Start>(s),
                update: std::mem::transmute::<*mut c_void, Update>(u),
                end: std::mem::transmute::<*mut c_void, End>(e),
                donate: {
                    let d = sym(c"jcode_donate_shortcuts");
                    (!d.is_null()).then(|| std::mem::transmute::<*mut c_void, End>(d))
                },
            })
        })
        .as_ref()
    }

    fn c(s: &str) -> CString {
        CString::new(s.replace('\0', "")).unwrap_or_default()
    }

    pub(super) fn donate_shortcuts() {
        if let Some(d) = bridge().and_then(|b| b.donate) {
            unsafe { d() };
        }
    }

    pub(super) fn apply(state: Option<LiveState>, started: bool) {
        let Some(b) = bridge() else { return };
        unsafe {
            match state {
                None => (b.end)(),
                Some(s) if started => (b.update)(c(s.activity).as_ptr(), c(&s.detail).as_ptr()),
                Some(s) => {
                    let ok = (b.start)(c(&s.title).as_ptr(), c(&s.session_id).as_ptr(), c(s.activity).as_ptr(), c(&s.detail).as_ptr());
                    log::info!("live activity: start {}", if ok == 1 { "ok" } else { "unavailable" });
                }
            }
        }
    }
}

#[cfg(not(target_os = "ios"))]
mod imp {
    pub(super) fn donate_shortcuts() {}
    pub(super) fn apply(state: Option<super::LiveState>, _started: bool) {
        log::debug!("live activity (preview): {state:?}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_links() {
        assert_eq!(parse("jcode://new"), Some(DeepLink::NewSession { prompt: None }));
        assert_eq!(
            parse("jcode://new?text=hello%20world+%26%20more"),
            Some(DeepLink::NewSession { prompt: Some("hello world & more".into()) })
        );
        assert_eq!(parse("jcode://ask?text=%E2%9C%93"), Some(DeepLink::NewSession { prompt: Some("✓".into()) }));
        assert_eq!(parse("jcode://new?text="), Some(DeepLink::NewSession { prompt: None }));
        assert_eq!(parse("jcode://session/abc%2D1/"), Some(DeepLink::OpenSession { session_id: "abc-1".into() }));
        assert_eq!(parse("jcode://session/"), None);
        assert_eq!(parse("https://x"), None);
        assert_eq!(parse("jcode://new?text=50%"), Some(DeepLink::NewSession { prompt: Some("50%".into()) }));
    }

    #[test]
    fn live_state_tracks_activity() {
        let mut s = jcode_mobile_core::SessionState::default();
        assert_eq!(live_state(&s), None);
        s.is_processing = true;
        s.session_id = Some("id".into());
        assert_eq!(live_state(&s).unwrap().activity, "responding");
        assert_eq!(live_state(&s).unwrap().title, "jcode");
        s.is_reasoning = true;
        assert_eq!(live_state(&s).unwrap().activity, "thinking");
    }
}
