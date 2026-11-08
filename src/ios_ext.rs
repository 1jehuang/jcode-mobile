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
use jcode_mobile_core::agents::{AgentState, AgentsState, LiveTokenKind, LocalAgent, headline};
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

/// This tab's contribution to the account-wide Live Activity.
pub fn local_agent(s: &jcode_mobile_core::SessionState) -> Option<LocalAgent> {
    use jcode_mobile_core::session::ToolStatus;
    let id = s.session_id.clone().filter(|id| !id.is_empty())?;
    let title = s.session_title.clone().filter(|t| !t.trim().is_empty()).unwrap_or_else(|| "jcode".into());
    let (state, detail) = if s.is_processing {
        let running = s
            .transcript
            .iter()
            .rev()
            .take(4)
            .flat_map(|e| e.tool_calls.iter())
            .find(|t| matches!(t.status, ToolStatus::Running | ToolStatus::StreamingInput));
        let detail = match running {
            Some(t) => t.name.clone(),
            None if s.is_reasoning => "Thinking".into(),
            None => "Responding".into(),
        };
        (Some(AgentState::Running), detail)
    } else if s.error_banner.is_some() {
        (Some(AgentState::Error), String::new())
    } else {
        (None, String::new())
    };
    Some(LocalAgent { id, title, state, detail })
}

/// Live Activity token reported by the bridge.
pub type LiveToken = (LiveTokenKind, String);

static TOKEN_TX: Mutex<Option<UnboundedSender<LiveToken>>> = Mutex::new(None);

/// Channel of Live Activity push tokens (update and push-to-start). The app
/// registers them with the Worker so the activity updates while suspended.
pub fn install_live_tokens() -> UnboundedReceiver<LiveToken> {
    let (tx, rx) = unbounded();
    *TOKEN_TX.lock().unwrap() = Some(tx);
    imp::install_token_callback();
    rx
}

fn emit_token(kind: LiveTokenKind, hex: String) {
    if let Some(tx) = TOKEN_TX.lock().unwrap().as_ref() {
        let _ = tx.unbounded_send((kind, hex));
    }
}

/// Show `state` in the single account-wide Live Activity if its content
/// changed. An empty state ends it.
pub fn sync_live(state: &AgentsState) {
    static LAST: Mutex<Option<AgentsState>> = Mutex::new(None);
    let mut last = LAST.lock().unwrap();
    if last.as_ref().is_some_and(|l| l.same_content(state)) {
        return;
    }
    *last = Some(state.clone());
    drop(last);
    imp::apply(state);
}

#[cfg(target_os = "ios")]
mod imp {
    use super::{AgentsState, LiveTokenKind, emit_token};
    use std::ffi::{CStr, CString, c_char, c_void};
    use std::sync::OnceLock;

    type Sync = unsafe extern "C" fn(*const c_char) -> i32;
    type End = unsafe extern "C" fn();
    type TokenCb = extern "C" fn(i32, *const c_char);
    type SetCb = unsafe extern "C" fn(Option<TokenCb>);

    struct Bridge {
        sync: Sync,
        end: End,
        set_cb: Option<SetCb>,
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
            let sym = |n: &CStr| dlsym(h, n.as_ptr());
            let (s, e) = (sym(c"jcode_live_sync"), sym(c"jcode_live_end"));
            if s.is_null() || e.is_null() {
                log::info!("live activity: JcodeBridge too old");
                return None;
            }
            let opt = |n: &CStr| {
                let p = sym(n);
                (!p.is_null()).then_some(p)
            };
            Some(Bridge {
                sync: std::mem::transmute::<*mut c_void, Sync>(s),
                end: std::mem::transmute::<*mut c_void, End>(e),
                set_cb: opt(c"jcode_live_set_token_callback").map(|p| std::mem::transmute::<*mut c_void, SetCb>(p)),
                donate: opt(c"jcode_donate_shortcuts").map(|p| std::mem::transmute::<*mut c_void, End>(p)),
            })
        })
        .as_ref()
    }

    extern "C" fn on_token(kind: i32, hex: *const c_char) {
        if hex.is_null() {
            return;
        }
        let hex = unsafe { CStr::from_ptr(hex) }.to_string_lossy().into_owned();
        let kind = if kind == 1 { LiveTokenKind::Start } else { LiveTokenKind::Update };
        emit_token(kind, hex);
    }

    pub(super) fn install_token_callback() {
        if let Some(set) = bridge().and_then(|b| b.set_cb) {
            unsafe { set(Some(on_token)) };
        }
    }

    pub(super) fn donate_shortcuts() {
        if let Some(d) = bridge().and_then(|b| b.donate) {
            unsafe { d() };
        }
    }

    pub(super) fn apply(state: &AgentsState) {
        let Some(b) = bridge() else { return };
        if state.is_empty() {
            unsafe { (b.end)() };
            return;
        }
        let json = CString::new(serde_json::to_string(state).unwrap_or_default()).unwrap_or_default();
        let ok = unsafe { (b.sync)(json.as_ptr()) };
        log::debug!("live activity: sync {}", if ok == 1 { "showing" } else { "unavailable" });
    }
}

#[cfg(not(target_os = "ios"))]
mod imp {
    pub(super) fn install_token_callback() {
        let _ = super::emit_token;
    }
    pub(super) fn donate_shortcuts() {}
    pub(super) fn apply(state: &super::AgentsState) {
        log::debug!("live activity (preview): {}", super::headline(state));
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
    fn local_agent_tracks_activity() {
        let mut s = jcode_mobile_core::SessionState::default();
        assert_eq!(local_agent(&s), None);
        s.session_id = Some("id".into());
        let a = local_agent(&s).unwrap();
        assert_eq!((a.state, a.title.as_str()), (None, "jcode"));
        s.is_processing = true;
        assert_eq!(local_agent(&s).unwrap().detail, "Responding");
        s.is_reasoning = true;
        assert_eq!(local_agent(&s).unwrap().state, Some(AgentState::Running));
        assert_eq!(local_agent(&s).unwrap().detail, "Thinking");
        s.is_processing = false;
        s.error_banner = Some("boom".into());
        assert_eq!(local_agent(&s).unwrap().state, Some(AgentState::Error));
    }
}
