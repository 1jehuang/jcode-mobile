//! Root view: owns the Jcode account, the live Cloud connection, and session
//! state, and switches between the sign-in, chat and settings screens.

use std::time::Duration;

use futures::StreamExt;
use futures::channel::mpsc::{UnboundedReceiver, unbounded};
use gpui::{
    AnyElement, App, Context, Entity, Hsla, ScrollHandle, SharedString, Window, div, prelude::*,
    px,
};
use jcode_mobile_core::{
    Account, AccountStore, Config, Connection, ConnectionOutput, ConnectionPhase, LocalIntent,
    Request, Role, SessionState, ToolStatus, TranscriptEntry, cloud,
};

use crate::text_input::{TextInput, TextInputEvent};
use crate::theme::{FONT, Theme, with_alpha};

/// Insets the shell reserves for the notch / home indicator. On iOS these come
/// from UIKit; the desktop preview fakes an iPhone 15-class device.
#[derive(Clone, Copy)]
pub struct SafeArea {
    pub top: f32,
    pub bottom: f32,
}

#[derive(PartialEq)]
enum Screen {
    SignIn,
    Chat,
    Settings,
}

pub struct MobileApp {
    /// Soft keyboard currently covers the bottom of the screen.
    keyboard_up: bool,
    screen: Screen,
    safe: SafeArea,
    /// Fixed device size for the desktop preview; None on a real phone.
    pub device: Option<(f32, f32)>,
    store: AccountStore,
    account: Option<Account>,
    connection: Option<Connection>,
    session: SessionState,
    // Sign-in form: email, then the 6-digit code.
    email: Entity<TextInput>,
    code: Entity<TextInput>,
    login_token: Option<String>,
    signing_in: bool,
    sign_in_error: Option<String>,
    /// Account deletion: 0 idle, 1 armed (second tap confirms), 2 in flight.
    delete_step: u8,
    delete_error: Option<String>,
    /// Browser (Google) sign-in in flight; set to true to cancel polling.
    browser_login: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    /// Animation clock for the spinning mark.
    started: std::time::Instant,
    composer: Entity<TextInput>,
    /// Id of the live tab whose state sits in `connection`/`session`/scroll.
    active_tab: u64,
    next_tab: u64,
    /// Background tabs, in tab order alongside the active one (see `tab_order`).
    parked: Vec<ParkedTab>,
    /// Tab ids in display order (includes `active_tab`).
    tab_order: Vec<u64>,
    /// Animated tab geometry per id, eased toward the desktop layout.
    tab_motion: std::collections::HashMap<u64, crate::tabs::TabGeometry>,
    tab_track_width: f32,
    last_frame: std::time::Instant,
    transcript_scroll: ScrollHandle,
    expanded_tools: std::collections::HashSet<String>,
    follow: bool,
    spinner_step: usize,
    /// Dev hook (JCODE_MOBILE_SEND): message sent once connected.
    auto_send: Option<String>,
}

impl MobileApp {
    pub fn new(safe: SafeArea, window: &mut Window, cx: &mut Context<Self>) -> Self {
        use gpui::{Autocapitalize, TextInputAction, TextInputConfiguration, TextInputPurpose};
        let email = cx.new(|cx| {
            let mut t = TextInput::new("you@example.com", cx);
            t.config = TextInputConfiguration {
                input_action: TextInputAction::Next,
                purpose: TextInputPurpose::Email,
                ..Default::default()
            };
            t
        });
        let code = cx.new(|cx| {
            let mut t = TextInput::new("6-digit code", cx);
            t.config = TextInputConfiguration {
                input_action: TextInputAction::Go,
                purpose: TextInputPurpose::OneTimeCode,
                ..Default::default()
            };
            t
        });
        let composer = cx.new(|cx| {
            let mut t = TextInput::new("Message jcode…", cx);
            t.bare = true;
            t.config = TextInputConfiguration {
                autocorrect: true,
                autocapitalize: Autocapitalize::Sentences,
                suggestions: true,
                input_action: TextInputAction::Send,
                purpose: TextInputPurpose::Text,
            };
            t
        });
        for input in [&email, &code] {
            cx.subscribe(input, |this, _, ev, cx| match ev {
                TextInputEvent::Submit => this.sign_in_step(cx),
                TextInputEvent::Changed => cx.notify(),
            })
            .detach();
        }
        cx.subscribe(&composer, |this, _, ev, cx| match ev {
            TextInputEvent::Submit => this.send_draft(cx),
            TextInputEvent::Changed => cx.notify(),
        })
        .detach();

        // Spinner tick while processing or provisioning.
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(125)).await;
                let alive = this
                    .update(cx, |this, cx| {
                        if this.session.is_processing
                            || this.session.is_reasoning
                            || this.signing_in
                            || this.browser_login.is_some()
                            || !matches!(this.session.phase, ConnectionPhase::Connected | ConnectionPhase::Failed { .. })
                        {
                            this.spinner_step = (this.spinner_step + 1) % 8;
                            cx.notify();
                        }
                    })
                    .is_ok();
                if !alive {
                    break;
                }
            }
        })
        .detach();

        let store = AccountStore::default_location();
        // Dev convenience: JCODE_MOBILE_USE_CLI_ACCOUNT=1 reuses the jcode CLI login.
        let account = store.load().or_else(|| {
            std::env::var_os("JCODE_MOBILE_USE_CLI_ACCOUNT").and_then(|_| cloud::account_from_jcode_config())
        });
        let mut app = Self {
            screen: Screen::SignIn,
            safe,
            device: None,
            store,
            account: None,
            connection: None,
            session: SessionState::default(),
            email,
            code,
            login_token: None,
            signing_in: false,
            sign_in_error: None,
            delete_step: 0,
            delete_error: None,
            browser_login: None,
            started: std::time::Instant::now(),
            keyboard_up: false,
            composer,
            active_tab: 0,
            next_tab: 1,
            parked: Vec::new(),
            tab_order: vec![0],
            tab_motion: Default::default(),
            tab_track_width: 0.0,
            last_frame: std::time::Instant::now(),
            transcript_scroll: ScrollHandle::new(),
            expanded_tools: Default::default(),
            follow: true,
            spinner_step: 0,
            auto_send: std::env::var("JCODE_MOBILE_SEND").ok(),
        };
        if let Some(account) = account {
            app.connect(account, cx);
            // Dev hook: JCODE_MOBILE_TABS=N opens N session tabs.
            let n: usize = std::env::var("JCODE_MOBILE_TABS").ok().and_then(|v| v.parse().ok()).unwrap_or(1);
            for _ in 1..n {
                app.new_tab(cx);
            }
        }
        let _ = window;
        app
    }

    // MARK: actions

    /// Email step mails a code; code step exchanges it for an account key.
    fn sign_in_step(&mut self, cx: &mut Context<Self>) {
        if self.signing_in {
            return;
        }
        let email = self.email.read(cx).text().trim().to_string();
        let code = self.code.read(cx).text().trim().to_string();
        let token = self.login_token.clone();
        if token.is_none() && !email.contains('@') {
            return;
        }
        if token.is_some() && code.len() < 6 {
            return;
        }
        self.signing_in = true;
        self.sign_in_error = None;
        cx.notify();
        enum Step {
            Sent(String),
            Done(Account),
        }
        let task = cx.background_spawn(async move {
            match token {
                None => cloud::start_login(&email).map(Step::Sent),
                Some(t) => cloud::verify_login(&t, &code).map(Step::Done),
            }
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                this.signing_in = false;
                match result {
                    Ok(Step::Sent(t)) => this.login_token = Some(t),
                    Ok(Step::Done(account)) => {
                        if let Err(e) = this.store.save(&account) {
                            this.sign_in_error = Some(format!("Could not save sign-in: {e}"));
                        }
                        this.login_token = None;
                        this.code.update(cx, |t, cx| t.set_text("", cx));
                        this.connect(account, cx);
                    }
                    Err(e) => this.sign_in_error = Some(e),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// "Continue with Google": opens jcode.sh/account (Google sign-in) in the
    /// system browser and polls the device flow until it is approved.
    fn google_sign_in(&mut self, cx: &mut Context<Self>) {
        if let Some(c) = self.browser_login.take() {
            c.store(true, std::sync::atomic::Ordering::Relaxed);
            cx.notify();
            return;
        }
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        self.browser_login = Some(cancel.clone());
        self.sign_in_error = None;
        cx.notify();
        let start = cx.background_spawn(async move { cloud::start_browser_login() });
        cx.spawn(async move |this, cx| {
            let start = match start.await {
                Ok(s) => s,
                Err(e) => {
                    this.update(cx, |this, cx| {
                        this.browser_login = None;
                        this.sign_in_error = Some(e);
                        cx.notify()
                    })
                    .ok();
                    return;
                }
            };
            let url = start.verification_uri_complete.clone();
            this.update(cx, |_, cx| cx.open_url(&url)).ok();
            let flag = cancel.clone();
            let result = cx
                .background_spawn(async move {
                    cloud::finish_browser_login(&start, || flag.load(std::sync::atomic::Ordering::Relaxed))
                })
                .await;
            this.update(cx, |this, cx| {
                if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                    return;
                }
                this.browser_login = None;
                match result {
                    Ok(account) => {
                        if let Err(e) = this.store.save(&account) {
                            this.sign_in_error = Some(format!("Could not save sign-in: {e}"));
                        }
                        this.connect(account, cx);
                    }
                    Err(e) => this.sign_in_error = Some(e),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn connect(&mut self, account: Account, cx: &mut Context<Self>) {
        if let Some(c) = self.connection.take() {
            c.stop();
        }
        self.session = SessionState::default();
        self.account = Some(account);
        self.start_connection(None, cx);
        self.screen = Screen::Chat;
        cx.notify();
    }

    /// Open a connection for the active tab, resuming `session_id` if given.
    fn start_connection(&mut self, resume: Option<String>, cx: &mut Context<Self>) {
        let Some(account) = self.account.clone() else { return };
        let mut config = Config::cloud(account);
        config.max_reconnect_attempts = Some(6);
        let (tx, rx) = unbounded();
        let conn = Connection::start(config, resume, move |o| {
            let _ = tx.unbounded_send(o);
        });
        self.connection = Some(conn);
        self.follow = true;
        self.pump(self.active_tab, rx, cx);
    }

    fn reconnect(&mut self, cx: &mut Context<Self>) {
        if let Some(c) = self.connection.take() {
            c.stop();
        }
        let resume = self.session.session_id.clone();
        self.start_connection(resume, cx);
        cx.notify();
    }

    // MARK: tabs

    /// Desktop's + : a fresh session in a new tab, focused.
    fn new_tab(&mut self, cx: &mut Context<Self>) {
        if self.account.is_none() {
            return;
        }
        let id = self.next_tab;
        self.next_tab += 1;
        let at = self.tab_order.iter().position(|t| *t == self.active_tab).map_or(self.tab_order.len(), |i| i + 1);
        self.tab_order.insert(at, id);
        self.park_active();
        self.active_tab = id;
        self.session = SessionState::default();
        self.start_connection(None, cx);
        cx.notify();
    }

    fn park_active(&mut self) {
        self.parked.push(ParkedTab {
            id: self.active_tab,
            connection: self.connection.take(),
            session: std::mem::take(&mut self.session),
            scroll: std::mem::replace(&mut self.transcript_scroll, ScrollHandle::new()),
            expanded_tools: std::mem::take(&mut self.expanded_tools),
            follow: self.follow,
        });
    }

    fn focus_tab(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(i) = self.parked.iter().position(|p| p.id == id) else { return };
        self.park_active();
        let p = self.parked.swap_remove(i);
        self.active_tab = p.id;
        self.connection = p.connection;
        self.session = p.session;
        self.transcript_scroll = p.scroll;
        self.expanded_tools = p.expanded_tools;
        self.follow = p.follow;
        cx.notify();
    }

    fn close_tab(&mut self, id: u64, cx: &mut Context<Self>) {
        if self.tab_order.len() <= 1 {
            return;
        }
        let pos = self.tab_order.iter().position(|t| *t == id).unwrap_or(0);
        self.tab_order.remove(pos);
        self.tab_motion.remove(&id);
        if id == self.active_tab {
            if let Some(c) = self.connection.take() {
                c.stop();
            }
            let next = self.tab_order[pos.min(self.tab_order.len() - 1)];
            let i = self.parked.iter().position(|p| p.id == next).unwrap();
            let p = self.parked.swap_remove(i);
            self.active_tab = p.id;
            self.connection = p.connection;
            self.session = p.session;
            self.transcript_scroll = p.scroll;
            self.expanded_tools = p.expanded_tools;
            self.follow = p.follow;
        } else if let Some(i) = self.parked.iter().position(|p| p.id == id) {
            if let Some(c) = self.parked.swap_remove(i).connection {
                c.stop();
            }
        }
        cx.notify();
    }

    fn tab_session(&self, id: u64) -> Option<&SessionState> {
        if id == self.active_tab {
            Some(&self.session)
        } else {
            self.parked.iter().find(|p| p.id == id).map(|p| &p.session)
        }
    }

    fn pump(&mut self, tab: u64, mut rx: UnboundedReceiver<ConnectionOutput>, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            while let Some(first) = rx.next().await {
                // Coalesce bursts of streaming deltas into one frame.
                let mut batch = vec![first];
                while let Ok(o) = rx.try_recv() {
                    batch.push(o);
                }
                let ok = this
                    .update(cx, |this, cx| {
                        if tab != this.active_tab {
                            if let Some(p) = this.parked.iter_mut().find(|p| p.id == tab) {
                                for o in batch {
                                    p.session.apply(o);
                                }
                            }
                            cx.notify();
                            return;
                        }
                        for o in batch {
                            this.session.apply(o);
                        }
                        if this.session.phase == ConnectionPhase::Connected
                            && this.session.session_id.is_some()
                            && let Some(msg) = this.auto_send.take()
                        {
                            this.composer.update(cx, |t, cx| t.set_text(msg, cx));
                            this.send_draft(cx);
                        }
                        if this.follow {
                            this.transcript_scroll.scroll_to_bottom();
                        }
                        cx.notify();
                    })
                    .is_ok();
                if !ok {
                    break;
                }
            }
        })
        .detach();
    }

    fn send(&self, build: impl FnOnce(u64) -> Request) {
        if let Some(c) = &self.connection {
            c.send(build);
        }
    }

    fn send_draft(&mut self, cx: &mut Context<Self>) {
        let text = self.composer.read(cx).text().trim().to_string();
        if text.is_empty() || self.session.phase != ConnectionPhase::Connected {
            return;
        }
        if self.session.is_processing {
            self.session.apply_intent(LocalIntent::UserQueuedInterrupt(text.clone()));
            self.send(|id| Request::SoftInterrupt { id, content: text, urgent: false });
        } else {
            self.session.apply_intent(LocalIntent::UserSentMessage(text.clone()));
            self.send(|id| Request::Message { id, content: text });
        }
        self.composer.update(cx, |t, cx| t.set_text("", cx));
        self.follow = true;
        self.transcript_scroll.scroll_to_bottom();
        cx.notify();
    }

    fn cancel(&mut self, cx: &mut Context<Self>) {
        self.send(|id| Request::Cancel { id });
        cx.notify();
    }

    /// First tap arms, second tap deletes the account server-side, then signs out.
    fn delete_account_tap(&mut self, cx: &mut Context<Self>) {
        let Some(account) = self.account.clone() else { return };
        match self.delete_step {
            0 => self.delete_step = 1,
            1 => {
                self.delete_step = 2;
                self.delete_error = None;
                let task = cx.background_spawn(async move { cloud::delete_account(&account) });
                cx.spawn(async move |this, cx| {
                    let result = task.await;
                    this.update(cx, |this, cx| {
                        this.delete_step = 0;
                        match result {
                            Ok(()) => this.sign_out(cx),
                            Err(e) => this.delete_error = Some(e),
                        }
                        cx.notify();
                    })
                    .ok();
                })
                .detach();
            }
            _ => {}
        }
        cx.notify();
    }

    fn sign_out(&mut self, cx: &mut Context<Self>) {
        if let Some(c) = self.connection.take() {
            c.stop();
        }
        for p in self.parked.drain(..) {
            if let Some(c) = p.connection {
                c.stop();
            }
        }
        self.tab_order = vec![self.active_tab];
        self.tab_motion.clear();
        self.store.clear();
        self.account = None;
        self.session = SessionState::default();
        self.login_token = None;
        self.screen = Screen::SignIn;
        cx.notify();
    }
}

// MARK: rendering helpers

/// A background session tab: its own connection and transcript.
struct ParkedTab {
    id: u64,
    connection: Option<Connection>,
    session: SessionState,
    scroll: ScrollHandle,
    expanded_tools: std::collections::HashSet<String>,
    follow: bool,
}

fn label(text: impl Into<SharedString>, color: Hsla, size: f32) -> gpui::Div {
    div().text_size(px(size)).text_color(color).child(text.into())
}

/// Desktop's composer pill: 22px rounded-full chip on prompt paper.
fn pill(text: impl Into<SharedString>, t: &Theme) -> gpui::Div {
    div()
        .flex_none()
        .h(px(22.))
        .px(px(10.))
        .flex()
        .items_center()
        .rounded_full()
        .bg(t.surface)
        .text_size(px(11.))
        .text_color(t.text_secondary)
        .whitespace_nowrap()
        .child(text.into())
}

/// Outlined, fully rounded action (desktop's composer/tab outline language).
fn outline_button(
    id: &'static str,
    text: impl Into<SharedString>,
    enabled: bool,
    t: &Theme,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .w_full()
        .h(px(44.))
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .border_1()
        .border_color(if enabled { t.border_focus } else { t.input_border })
        .text_color(if enabled { t.text_strong } else { t.text_muted })
        .text_size(px(14.))
        .when(enabled, |d| d.cursor_pointer().active(|s| s.bg(t.surface)))
        .child(text.into())
}

fn google_mark() -> std::sync::Arc<gpui::Image> {
    static MARK: std::sync::OnceLock<std::sync::Arc<gpui::Image>> = std::sync::OnceLock::new();
    MARK.get_or_init(|| {
        std::sync::Arc::new(gpui::Image::from_bytes(
            gpui::ImageFormat::Png,
            include_bytes!("../assets/brand/google-g.png").to_vec(),
        ))
    })
    .clone()
}

fn spinner_glyph(step: usize) -> &'static str {
    ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧"][step % 8]
}

impl MobileApp {
    fn render_sign_in(&mut self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let code_step = self.login_token.is_some();
        let ready = if code_step {
            self.code.read(cx).text().trim().len() >= 6
        } else {
            self.email.read(cx).text().contains('@')
        };
        let action = match (code_step, self.signing_in) {
            (false, false) => "Email me a code",
            (false, true) => "Sending…",
            (true, false) => "Sign in",
            (true, true) => "Signing in…",
        };
        let keyboard_up = self.keyboard_up;
        div()
            .id("sign-in")
            .size_full()
            .flex()
            .flex_col()
            // Keyboard up: anchor to the top and scroll, so the field and
            // its button stay above the keyboard instead of centering under it.
            .when(keyboard_up, |d| d.justify_start().pt(px(12.)).overflow_y_scroll())
            .when(!keyboard_up, |d| d.justify_center())
            .px(px(20.))
            .gap(px(if keyboard_up { 12. } else { 18. }))
            .child(
                div().w_full().flex().justify_center().pb(px(8.)).child(
                    crate::donut::spinning_donut(
                        if keyboard_up { 64. } else { 140. },
                        t.text_strong,
                        self.started,
                    ),
                ),
            )
            .child(
                label("Sign in to Jcode Cloud", t.text_strong, 22.)
                    .line_height(px(30.))
                    .w_full()
                    .flex()
                    .justify_center(),
            )
            .when(code_step, |d| {
                d.child(label("Enter the 6-digit code we emailed you.", t.text_secondary, 13.).text_center())
            })
            .when_some(self.sign_in_error.clone(), |d, e| d.child(banner(&e, t.error, t)))
            // No third-party login on iOS: offering Google would oblige
            // Sign in with Apple (App Review 4.8). Email codes only there.
            .when(!code_step && !cfg!(target_os = "ios"), |d| {
                let waiting = self.browser_login.is_some();
                d.child(
                    div()
                        .id("google")
                        .w_full()
                        .h(px(44.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .gap(px(10.))
                        .rounded_full()
                        .bg(gpui::white())
                        .text_color(gpui::black())
                        .text_size(px(14.))
                        .cursor_pointer()
                        .active(|s| s.opacity(0.85))
                        .when(!waiting, |d| d.child(gpui::img(google_mark()).size(px(18.)).flex_none()))
                        .child(if waiting {
                            format!("{} Waiting for browser… (tap to cancel)", spinner_glyph(self.spinner_step))
                        } else {
                            "Continue with Google".into()
                        })
                        .on_click(cx.listener(|this, _, _, cx| this.google_sign_in(cx))),
                )
                .child(
                    div()
                        .w_full()
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .child(div().flex_1().h(px(1.)).bg(t.border))
                        .child(label("or", t.text_muted, 12.))
                        .child(div().flex_1().h(px(1.)).bg(t.border)),
                )
            })
            .child(if code_step { self.code.clone() } else { self.email.clone() })
            .child(
                outline_button("sign-in-go", action, ready && !self.signing_in, t)
                    .on_click(cx.listener(|this, _, _, cx| this.sign_in_step(cx))),
            )
            .when(code_step, |d| {
                d.child(
                    div()
                        .id("use-other-email")
                        .cursor_pointer()
                        .child(label("Use a different email", t.text_muted, 12.))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.login_token = None;
                            this.sign_in_error = None;
                            cx.notify()
                        })),
                )
            })
            .into_any_element()
    }

    /// Desktop's live tabs (live_tabs.rs): ≡, a centered strip of session
    /// tabs that shrink and then overlap like folder tabs to fit, and +.
    fn render_header(&mut self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        use crate::tabs;
        // Phone width minus side buttons and padding.
        let width = self.device.map_or(390., |(w, _)| w);
        let track = (width - 2. * 10. - 2. * 32. - 2. * 6.).max(40.);
        let count = self.tab_order.len();
        let selected = self.tab_order.iter().position(|id| *id == self.active_tab).unwrap_or(0);
        let targets = tabs::layout(track, count, selected);
        // Ease toward targets; snap on first sight or track change.
        let now = std::time::Instant::now();
        let dt = now.duration_since(self.last_frame).as_secs_f32().min(0.05);
        self.last_frame = now;
        let snap = self.tab_track_width != track;
        self.tab_track_width = track;
        let mut moving = false;
        let geometry: Vec<tabs::TabGeometry> = self
            .tab_order
            .iter()
            .zip(&targets)
            .map(|(id, target)| {
                let g = self.tab_motion.entry(*id).or_insert(*target);
                if snap {
                    *g = *target;
                } else {
                    moving |= tabs::step_toward(g, *target, dt);
                }
                *g
            })
            .collect();
        if moving {
            cx.notify();
        }
        let order = tabs::paint_order(count, selected);
        let mut strip = div().relative().flex_none().w(px(track)).h(px(tabs::TAB_HEIGHT));
        for &pos in &order {
            let id = self.tab_order[pos];
            let g = geometry[pos];
            let focused = pos == selected;
            let (exp_left, visible) = tabs::exposed(&geometry, pos, &order);
            let s = self.tab_session(id);
            let title: SharedString = s
                .and_then(|s| s.session_title.clone())
                .unwrap_or_else(|| "New session".into())
                .into();
            let dot = match s.map(|s| &s.phase) {
                Some(ConnectionPhase::Connected) if s.is_some_and(|s| s.is_processing) => t.accent,
                Some(ConnectionPhase::Connected) => t.ok,
                Some(ConnectionPhase::Failed { .. }) => t.error,
                Some(ConnectionPhase::Disconnected) | None => t.text_muted,
                _ => t.warning,
            };
            let pad = (visible / 12.).min(6.);
            let bg = if focused { t.background } else { t.raised };
            // Label lives only in the exposed part, so overlaps never hide it.
            let content = div()
                .absolute()
                .top_0()
                .bottom_0()
                .left(px(exp_left - g.left))
                .w(px((visible - 2.).max(0.)))
                .px(px(pad))
                .flex()
                .items_center()
                .gap(px((visible / 12.).min(6.)))
                .overflow_hidden()
                .when(visible > 14., |d| d.child(div().flex_none().size(px(7.)).rounded_full().bg(dot)))
                .when(visible > 40., |d| {
                    d.child(
                        label(title, if focused { t.text_strong } else { t.text_secondary }, 12.)
                            .min_w_0()
                            .truncate(),
                    )
                });
            strip = strip.child(
                div()
                    .id(("tab", id))
                    .absolute()
                    .left(px(g.left))
                    .bottom_0()
                    .w(px(g.width))
                    .h(px(g.height))
                    .rounded(px(8.))
                    .border_1()
                    .border_color(if focused { t.border_focus } else { t.border })
                    .bg(bg)
                    .cursor_pointer()
                    .child(content)
                    .on_click(cx.listener(move |this, ev: &gpui::ClickEvent, _, cx| {
                        // Double-tap closes, like the desktop's middle-click.
                        if ev.click_count() >= 2 {
                            this.close_tab(id, cx);
                        } else {
                            this.focus_tab(id, cx);
                        }
                    })),
            );
        }
        let connected = self.account.is_some();
        div()
            .flex()
            .items_center()
            .gap(px(6.))
            .px(px(10.))
            .h(px(48.))
            .bg(t.raised)
            .child(
                div()
                    .id("menu")
                    .flex_none()
                    .size(px(32.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(t.text_secondary)
                    .text_size(px(16.))
                    .cursor_pointer()
                    .child("≡")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.screen = Screen::Settings;
                        cx.notify()
                    })),
            )
            .child(strip)
            .child(
                div()
                    .id("new-session")
                    .flex_none()
                    .size(px(32.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(if connected { t.text_strong } else { t.text_muted })
                    .text_size(px(20.))
                    .cursor_pointer()
                    .child("+")
                    .on_click(cx.listener(|this, _, _, cx| this.new_tab(cx))),
            )
    }

    fn render_chat(&mut self, t: &Theme, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        // Unpin follow mode when the user scrolls up.
        let max = self.transcript_scroll.max_offset().y;
        let off = -self.transcript_scroll.offset().y;
        let at_bottom = max - off < px(56.);
        let entries = &self.session.transcript;
        let mut prompt_n = 0;
        let mut rows: Vec<AnyElement> = Vec::new();
        for e in entries {
            if e.role == Role::User {
                prompt_n += 1;
            }
            rows.push(self.render_entry(e, prompt_n, t, cx));
        }
        if let Some(a) = self.activity() {
            rows.push(activity_label(a, self.spinner_step, t).into_any_element());
        }
        if let Some(err) = self.session.error_banner.clone() {
            rows.push(
                banner(&err, t.error, t)
                    .id("err")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.session.apply_intent(LocalIntent::DismissError);
                        if matches!(this.session.phase, ConnectionPhase::Failed { .. }) {
                            this.reconnect(cx);
                        }
                        cx.notify()
                    }))
                    .into_any_element(),
            );
        }
        let empty = entries.is_empty() && !self.session.is_reasoning;
        let _ = window;
        let body = if empty {
            // Desktop's fresh session: the question sits just above the composer.
            div()
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .justify_end()
                .px(px(16.))
                .pb(px(14.))
                .gap(px(12.))
                .child(self.connection_status(t))
                .when_some(self.session.error_banner.clone(), |d, e| {
                    d.child(banner(&e, t.error, t).id("err-empty").on_click(cx.listener(|this, _, _, cx| {
                        this.session.apply_intent(LocalIntent::DismissError);
                        this.reconnect(cx);
                    })))
                })
                .child(label("What would you like to work on?", t.text_strong, 20.).line_height(px(28.)))
                .into_any_element()
        } else {
            div()
                .relative()
                .flex_1()
                .min_h_0()
                .child(
                    div()
                        .id("transcript")
                        .size_full()
                        .overflow_y_scroll()
                        .track_scroll(&self.transcript_scroll)
                        .on_scroll_wheel(cx.listener(|this, _, _, cx| {
                            let max = this.transcript_scroll.max_offset().y;
                            let off = -this.transcript_scroll.offset().y;
                            this.follow = max - off < px(56.);
                            cx.notify();
                        }))
                        .child(div().flex().flex_col().gap(px(14.)).px(px(14.)).py(px(12.)).children(rows)),
                )
                .when(!at_bottom && max > px(0.), |d| {
                    d.child(
                        div()
                            .id("jump")
                            .absolute()
                            .right(px(14.))
                            .bottom(px(8.))
                            .size(px(36.))
                            .rounded_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(t.background)
                            .border_1()
                            .border_color(t.border_focus)
                            .text_color(t.text_strong)
                            .cursor_pointer()
                            .child("↓")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.follow = true;
                                this.transcript_scroll.scroll_to_bottom();
                                cx.notify()
                            })),
                    )
                })
                .into_any_element()
        };
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(self.render_header(t, cx))
            .child(body)
            .child(self.render_composer(t, cx))
            .into_any_element()
    }

    /// Provisioning/connecting line shown on an empty session.
    fn connection_status(&self, t: &Theme) -> AnyElement {
        let text = match &self.session.phase {
            ConnectionPhase::Connected | ConnectionPhase::Failed { .. } => return div().into_any_element(),
            ConnectionPhase::Connecting => "Connecting to Jcode Cloud…".to_string(),
            ConnectionPhase::Provisioning { message } => message.clone(),
            ConnectionPhase::Reconnecting { attempt } => format!("Reconnecting ({attempt})…"),
            ConnectionPhase::Disconnected => "Disconnected".into(),
        };
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(label(spinner_glyph(self.spinner_step), t.text_secondary, 13.))
            .child(label(text, t.text_muted, 12.))
            .into_any_element()
    }

    fn activity(&self) -> Option<&'static str> {
        if self.session.is_reasoning {
            return Some("thinking");
        }
        if !self.session.is_processing {
            return None;
        }
        let Some(last) = self.session.transcript.last().filter(|e| e.role == Role::Assistant) else {
            return Some("working");
        };
        if last
            .tool_calls
            .iter()
            .any(|c| matches!(c.status, ToolStatus::StreamingInput | ToolStatus::Running))
        {
            return Some("running tools");
        }
        Some(if last.text.is_empty() { "working" } else { "responding" })
    }

    fn render_entry(&self, e: &TranscriptEntry, prompt_n: usize, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        match e.role {
            // Desktop prompt: "N›" prefix, strong ink on USER_BG paper.
            Role::User => div()
                .flex()
                .gap(px(6.))
                .px(px(8.))
                .py(px(4.))
                .rounded(px(6.))
                .bg(Theme::prompt_background(0))
                .when(e.is_queued, |d| d.opacity(0.6))
                .child(label(format!("{prompt_n}›"), t.text_muted, 13.).flex_none())
                .child(div().flex_1().min_w_0().text_size(px(13.)).text_color(t.text_strong).child(e.text.clone()))
                .into_any_element(),
            Role::System => label(e.text.clone(), t.text_muted, 12.).into_any_element(),
            Role::Assistant => {
                let mut col = div().flex().flex_col().gap(px(6.));
                if !e.reasoning.is_empty() {
                    col = col.child(label(e.reasoning.clone(), t.text_muted, 12.).italic());
                }
                for call in &e.tool_calls {
                    col = col.child(self.tool_line(call, t, cx));
                }
                if !e.text.is_empty() {
                    col = col.child(div().pt(px(4.)).child(crate::markdown::render(&e.text, t)));
                }
                col.into_any_element()
            }
        }
    }

    /// Desktop/TUI tool row: `✓ name · headline`, tap to expand output.
    fn tool_line(&self, call: &jcode_mobile_core::ToolCall, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let expanded = self.expanded_tools.contains(&call.id);
        let (glyph, color) = match &call.status {
            ToolStatus::StreamingInput | ToolStatus::Running => (spinner_glyph(self.spinner_step), t.warning),
            ToolStatus::Succeeded => ("✓", t.ok),
            ToolStatus::Failed(_) => ("✗", t.error),
        };
        let headline = crate::tool_summary::headline(&call.input).unwrap_or_default();
        let id = call.id.clone();
        div()
            .id(SharedString::from(format!("tool-{}", call.id)))
            .flex()
            .flex_col()
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| {
                if !this.expanded_tools.remove(&id) {
                    this.expanded_tools.insert(id.clone());
                }
                cx.notify()
            }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .text_size(px(12.))
                    .child(div().flex_none().text_color(color).child(glyph))
                    .child(div().flex_none().text_color(t.text).child(call.name.clone()))
                    .when(!headline.is_empty(), |d| {
                        d.child(div().flex_none().text_color(t.text_muted).child("·"))
                            .child(div().flex_1().min_w_0().truncate().text_color(t.text_secondary).child(headline))
                    }),
            )
            .when(expanded, |d| {
                let mut body = call.output.clone();
                if let ToolStatus::Failed(m) = &call.status
                    && body.is_empty()
                {
                    body = m.clone();
                }
                if body.len() > 4000 {
                    let mut cut = 4000;
                    while !body.is_char_boundary(cut) {
                        cut -= 1;
                    }
                    body.truncate(cut);
                    body.push_str("\n…");
                }
                d.child(
                    div()
                        .mt(px(4.))
                        .ml(px(6.))
                        .pl(px(10.))
                        .py(px(4.))
                        .border_l_1()
                        .border_color(t.border_focus)
                        .text_size(px(11.))
                        .text_color(t.text_secondary)
                        .child(if body.is_empty() { call.input.clone() } else { body }),
                )
            })
            .into_any_element()
    }

    /// Desktop composer: fully rounded outline, round action at the right,
    /// identity pills (model, provider, machine) underneath.
    fn render_composer(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let processing = self.session.is_processing;
        let has_text = !self.composer.read(cx).text().trim().is_empty();
        let model = self.session.model_name.clone();
        let provider = self.session.provider_name.clone();
        div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .px(px(12.))
            .pt(px(6.))
            .pb(px(8.))
            .bg(t.background)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .pl(px(16.))
                    .pr(px(5.))
                    .min_h(px(46.))
                    .rounded(px(23.))
                    .border_1()
                    .border_color(t.border_focus)
                    .child(div().flex_1().min_w_0().child(self.composer.clone()))
                    .child(if processing && !has_text {
                        round_button("stop", "■", t.error, t)
                            .on_click(cx.listener(|this, _, _, cx| this.cancel(cx)))
                    } else {
                        round_button("send", "↑", if has_text { t.text_strong } else { t.text_muted }, t)
                            .on_click(cx.listener(|this, _, _, cx| this.send_draft(cx)))
                    }),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .px(px(4.))
                    .overflow_hidden()
                    .child(
                        div()
                            .flex_none()
                            .h(px(22.))
                            .px(px(10.))
                            .flex()
                            .items_center()
                            .rounded_full()
                            .bg(t.surface_elevated)
                            .text_size(px(11.))
                            .text_color(t.accent)
                            .child("☁ jcode-cloud"),
                    )
                    .when_some(model, |d, m| d.child(pill(m, t)))
                    .when_some(provider, |d, p| d.child(pill(p, t))),
            )
    }

    fn render_settings(&mut self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let s = &self.session;
        let info = [
            ("Account", self.account.as_ref().and_then(|a| a.email.clone())),
            ("Machine", Some("jcode-cloud".to_string())),
            ("Version", s.server_version.clone()),
            ("Model", s.model_name.clone()),
            ("Provider", s.provider_name.clone()),
            ("Session", s.session_id.clone()),
            ("Tokens", Some(format!("↑{} ↓{}", s.token_input, s.token_output))),
        ];
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(
                div()
                    .flex()
                    .items_center()
                    .h(px(48.))
                    .px(px(14.))
                    .bg(t.raised)
                    .child(
                        div()
                            .id("back")
                            .cursor_pointer()
                            .text_color(t.text_strong)
                            .text_size(px(13.))
                            .child("‹ back")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.screen = if this.account.is_some() { Screen::Chat } else { Screen::SignIn };
                                cx.notify()
                            })),
                    )
                    .child(div().flex_1().flex().justify_center().child(label("settings", t.text_strong, 13.)))
                    .child(div().w(px(48.))),
            )
            .child(
                div().id("settings").flex_1().overflow_y_scroll().child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(14.))
                        .p(px(16.))
                        .child(div().flex().flex_col().gap(px(8.)).children(info.into_iter().map(|(k, v)| {
                            div()
                                .flex()
                                .justify_between()
                                .gap(px(12.))
                                .text_size(px(12.))
                                .child(div().flex_none().text_color(t.text_muted).child(k))
                                .child(
                                    div()
                                        .min_w_0()
                                        .truncate()
                                        .text_color(t.text)
                                        .child(v.unwrap_or_else(|| "—".into())),
                                )
                        })))
                        .child(div().h(px(1.)).bg(t.border))
                        .child(outline_button("new", "New session", true, t).on_click(cx.listener(
                            |this, _, _, cx| {
                                this.new_tab(cx);
                                this.screen = Screen::Chat;
                            },
                        )))
                        .child(outline_button("clear", "Clear conversation", true, t).on_click(cx.listener(
                            |this, _, _, cx| {
                                this.session.apply_intent(LocalIntent::ClearedConversation);
                                this.send(|id| Request::Clear { id });
                                this.screen = Screen::Chat;
                                cx.notify()
                            },
                        )))
                        .child(
                            outline_button("sign-out", "Sign out", true, t)
                                .text_color(t.error)
                                .on_click(cx.listener(|this, _, _, cx| this.sign_out(cx))),
                        )
                        .child(
                            outline_button(
                                "delete-account",
                                match self.delete_step {
                                    0 => "Delete account",
                                    1 => "Tap again to permanently delete",
                                    _ => "Deleting account…",
                                },
                                self.account.is_some() && self.delete_step < 2,
                                t,
                            )
                            .text_color(t.error)
                            .on_click(cx.listener(|this, _, _, cx| this.delete_account_tap(cx))),
                        )
                        .when(self.delete_step == 1, |d| {
                            d.child(label(
                                "Cancels your subscription and erases your jcode.sh account. This cannot be undone.",
                                t.text_muted,
                                11.,
                            ))
                        })
                        .when_some(self.delete_error.clone(), |d, e| d.child(label(e, t.error, 11.))),
                ),
            )
            .into_any_element()
    }
}

fn round_button(id: &'static str, glyph: &'static str, fg: Hsla, t: &Theme) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .size(px(36.))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .bg(t.surface_elevated)
        .text_color(fg)
        .text_size(px(15.))
        .cursor_pointer()
        .active(|s| s.bg(t.surface))
        .child(glyph)
}

fn banner(msg: &str, tint: Hsla, t: &Theme) -> gpui::Div {
    div()
        .w_full()
        .flex()
        .items_start()
        .gap(px(8.))
        .px(px(10.))
        .py(px(8.))
        .rounded(px(6.))
        .bg(with_alpha(tint, 0.06))
        .border_1()
        .border_color(with_alpha(tint, 0.3))
        .child(label("✗", tint, 12.))
        .child(label(msg.to_string(), t.text, 12.).flex_1())
}

fn activity_label(text: &'static str, step: usize, t: &Theme) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap(px(6.))
        .text_size(px(12.))
        .child(div().text_color(t.text_secondary).child(spinner_glyph(step)))
        .child(div().text_color(t.text_muted).child(text))
}

impl Render for MobileApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Real phone: UIKit safe-area insets. Preview: the faked device insets.
        let (top, bottom) = if self.device.is_none() {
            let e = window.window_insets().effective();
            (f32::from(e.top), f32::from(e.bottom))
        } else {
            // Dev hook (JCODE_MOBILE_KEYBOARD=1): fake an iPhone keyboard
            // (336pt incl. suggestion bar) to check layouts in the preview.
            let kb = std::env::var_os("JCODE_MOBILE_KEYBOARD").map_or(0., |_| 336.);
            (self.safe.top, self.safe.bottom.max(kb))
        };
        self.keyboard_up = f32::from(window.window_insets().ime.bottom) > 0.
            || (self.device.is_some() && std::env::var_os("JCODE_MOBILE_KEYBOARD").is_some());
        let t = Theme::get(cx);
        let body = match self.screen {
            Screen::SignIn => self.render_sign_in(&t, cx),
            Screen::Chat => self.render_chat(&t, window, cx),
            Screen::Settings => self.render_settings(&t, cx),
        };
        let screen = div()
            .flex()
            .flex_col()
            .bg(t.background)
            .text_color(t.text)
            .font_family(FONT)
            .pt(px(top))
            .pb(px(bottom))
            .overflow_hidden()
            .child(body);
        // Dev hook (JCODE_MOBILE_FRAMELESS): bare screen for App Store screenshots.
        if self.device.is_some() && std::env::var_os("JCODE_MOBILE_FRAMELESS").is_some() {
            return screen.size_full().into_any_element();
        }
        match self.device {
            // Preview: an exact-size device screen centred in whatever window
            // the compositor gives us (tiling WMs ignore the requested size).
            Some((w, h)) => div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(gpui::rgb(0x161616))
                .child(
                    screen
                        .relative()
                        .w(px(w))
                        .h(px(h))
                        .flex_none()
                        .rounded(px(48.))
                        .border_1()
                        .border_color(t.input_border)
                        .child(
                            // Dynamic Island and home indicator, for honest inset checks.
                            div()
                                .absolute()
                                .top(px(11.))
                                .left(px(w / 2. - 63.))
                                .w(px(126.))
                                .h(px(37.))
                                .rounded_full()
                                .bg(gpui::black())
                                .border_1()
                                .border_color(t.border),
                        )
                        .child(
                            div()
                                .absolute()
                                .bottom(px(8.))
                                .left(px(w / 2. - 67.))
                                .w(px(134.))
                                .h(px(5.))
                                .rounded_full()
                                .bg(t.text),
                        ),
                )
                .into_any_element(),
            None => screen.size_full().into_any_element(),
        }
    }
}

pub fn init(cx: &mut App) {
    cx.set_global(Theme::pure_black());
    crate::text_input::bind_keys(cx);
}
