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

use crate::text_input::{InputDraft, TextInput, TextInputEvent};
use crate::haptics::{self, Haptic};

#[path = "push_glue.rs"]
mod push_glue;
mod deep_link_glue;
mod a11y;
use a11y::A11yExt as _;
mod recent_dirs;
mod sidebar;
mod synced_sessions;
mod apple_sign_in;
mod ios_system;
mod ios_feel;
mod panel_preview;
mod voice;
mod cloud_machine;
#[cfg(test)]
mod interaction_fuzz;
#[cfg(test)]
mod offline_tests;
#[cfg(test)]
mod scroll_tests;
#[cfg(test)]
mod haptic_tests;
use gpui::Focusable as _;
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
    /// Test instances never read credentials, connect, or change persistent state.
    offline: bool,
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
    /// Move focus to the code field on the next frame (after the code is mailed).
    focus_code: bool,
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
    voice: voice::VoiceState,
    cloud_machine: cloud_machine::MachineState,
    /// Desktop preview's simulated software keyboard state.
    sim_kb: crate::sim_keyboard::SimKeyboard,
    /// Animated bottom inset (keyboard slide), eased toward the target.
    inset_anim: Option<f32>,
    inset_last: std::time::Instant,
    /// How far the keyboard currently pushes content up (animated).
    kb_lift: f32,
    /// Model/provider picker sheet open.
    model_picker: bool,
    /// Last focused field's keyboard config, kept so it can slide away.
    kb_config: gpui::TextInputConfiguration,
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
    follow_last: std::time::Instant,
    follow_max: gpui::Pixels,
    /// Long-press on a tab: generation of the pending hold timer.
    tab_hold: Option<u64>,
    /// The hold already fired, so the release must not also count as a tap.
    hold_fired: bool,
    /// Tab whose options sheet is open.
    tab_menu: Option<u64>,
    feel: ios_feel::Feel,
    /// Session id the empty-screen recent dirs list was last requested for.
    recent_dirs_asked: Option<String>,
    /// Directory sheet opened from the composer's location label.
    dir_picker: bool,
    /// The directory sheet offers a new session rather than moving this tab.
    dir_picker_new_session: bool,
    dir_input: Entity<TextInput>,
    /// Horizontal swipe between tabs, like the desktop workspace pull.
    swipe: crate::swipe::Swipe,
    spinner_step: usize,
    /// Desktop's thinking orbs: transcript activity, and connection/sign-in status.
    activity_orb: Entity<gpui_thinking_orbs::ThinkingOrb>,
    status_orb: Entity<gpui_thinking_orbs::ThinkingOrb>,
    /// Dev hook (JCODE_MOBILE_SEND): message sent once connected.
    auto_send: Option<String>,
    /// APNs device token (hex) once iOS hands it over.
    push_token: Option<String>,
    /// Session drawer (see `sidebar.rs`): target, eased progress, clock,
    /// and the (top, bottom) safe insets it pads by.
    drawer_open: bool,
    drawer_t: f32,
    drawer_last: std::time::Instant,
    drawer_insets: (f32, f32),
    /// Drawer drag-to-close travel (negative px) and last drag event.
    drawer_drag: f32,
    drawer_drag_last: Option<std::time::Instant>,
    /// Drawer session search field.
    drawer_search: Entity<TextInput>,
    drawer_directory: sidebar::DirectorySelection,
    synced: synced_sessions::State,
    transfer_input: Entity<TextInput>,
    /// Settings "Link existing Jcode account" panel (apple_sign_in.rs).
    apple_link: Option<Entity<apple_sign_in::AppleLinkPanel>>,
}

impl MobileApp {
    pub fn new(safe: SafeArea, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let offline = cfg!(test);
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
            t.max_lines = 6;
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
        let drawer_search = cx.new(|cx| {
            let mut t = TextInput::new("Search sessions", cx);
            t.bare = true;
            t.config = TextInputConfiguration {
                input_action: TextInputAction::Search,
                purpose: TextInputPurpose::Text,
                ..Default::default()
            };
            // Dev hook (JCODE_MOBILE_SEARCH=...): prefill the drawer search.
            if let Ok(q) = std::env::var("JCODE_MOBILE_SEARCH") {
                t.set_text(q, cx);
            }
            t
        });
        cx.subscribe(&drawer_search, |_, _, _, cx| cx.notify()).detach();
        let transfer_input = cx.new(|cx| TextInput::new("What should Jcode do next?", cx));
        cx.subscribe(&transfer_input, |this, _, event, cx| match event {
            TextInputEvent::Submit => this.continue_synced_in_cloud(cx),
            TextInputEvent::Changed => cx.notify(),
        }).detach();
        let dir_input = cx.new(|cx| {
            let mut t = TextInput::new("/home/ec2-user/src/app", cx);
            t.bare = true;
            t.config = TextInputConfiguration {
                input_action: TextInputAction::Go,
                purpose: TextInputPurpose::Text,
                ..Default::default()
            };
            t
        });
        cx.subscribe(&dir_input, |this, input, ev, cx| match ev {
            TextInputEvent::Submit => {
                let dir = input.read(cx).text().to_string();
                this.change_dir(dir, cx)
            }
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
        let account = (!offline).then(|| store.load().or_else(|| {
            std::env::var_os("JCODE_MOBILE_USE_CLI_ACCOUNT").and_then(|_| cloud::account_from_jcode_config())
        })).flatten();
        let mut app = Self {
            offline,
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
            focus_code: false,
            signing_in: false,
            // Dev hook: JCODE_MOBILE_ERROR=text shows a sign-in error banner.
            sign_in_error: std::env::var("JCODE_MOBILE_ERROR").ok(),
            delete_step: 0,
            delete_error: None,
            browser_login: None,
            started: std::time::Instant::now(),
            keyboard_up: false,
            sim_kb: Default::default(),
            inset_anim: None,
            inset_last: std::time::Instant::now(),
            kb_lift: 0.,
            model_picker: false,
            kb_config: Default::default(),
            composer,
            voice: voice::VoiceState::default(),
            cloud_machine: cloud_machine::MachineState::default(),
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
            follow_last: std::time::Instant::now(),
            follow_max: px(0.),
            tab_hold: None,
            hold_fired: false,
            tab_menu: None,
            feel: Default::default(),
            recent_dirs_asked: None,
            dir_picker: false,
            dir_picker_new_session: false,
            dir_input,
            swipe: Default::default(),
            spinner_step: 0,
            activity_orb: cx.new(|_| inline_orb()),
            status_orb: cx.new(|_| inline_orb()),
            auto_send: std::env::var("JCODE_MOBILE_SEND").ok(),
            push_token: None,
            // Dev hook (JCODE_MOBILE_DRAWER=1): start with the session drawer open.
            drawer_open: std::env::var_os("JCODE_MOBILE_DRAWER").is_some(),
            drawer_t: 0.,
            drawer_last: std::time::Instant::now(),
            drawer_insets: (0., 0.),
            drawer_drag: 0.,
            drawer_drag_last: None,
            drawer_search,
            drawer_directory: Default::default(),
            synced: Default::default(),
            transfer_input,
            apple_link: None,
        };
        if !offline {
            app.install_push(cx);
            app.install_deep_links(cx);
        }
        // Dev hook: JCODE_MOBILE_DRAFT=text prefills the composer.
        if let Ok(draft) = std::env::var("JCODE_MOBILE_DRAFT") {
            app.composer.update(cx, |c, cx| c.set_text(draft, cx));
        }
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

    #[cfg(test)]
    fn new_offline(safe: SafeArea, window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::new(safe, window, cx)
    }

    // MARK: actions

    /// Email step mails a code; code step exchanges it for an account key.
    fn sign_in_step(&mut self, cx: &mut Context<Self>) {
        if self.offline { return; }
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
        haptics::play(Haptic::Light);
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
                    Ok(Step::Sent(t)) => {
                        haptics::play(Haptic::Success);
                        this.login_token = Some(t);
                        this.focus_code = true;
                    }
                    Ok(Step::Done(account)) => {
                        if let Err(e) = this.store.save(&account) {
                            this.sign_in_error = Some(format!("Could not save sign-in: {e}"));
                        }
                        this.login_token = None;
                        this.code.update(cx, |t, cx| t.set_text("", cx));
                        this.connect(account, cx);
                    }
                    Err(e) => {
                        haptics::play(Haptic::Error);
                        this.sign_in_error = Some(e);
                    }
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
        if self.offline { return; }
        if let Some(c) = self.browser_login.take() {
            c.store(true, std::sync::atomic::Ordering::Relaxed);
            haptics::play(Haptic::Light);
            cx.notify();
            return;
        }
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        self.browser_login = Some(cancel.clone());
        haptics::play(Haptic::Light);
        self.sign_in_error = None;
        cx.notify();
        let start = cx.background_spawn(async move { cloud::start_browser_login() });
        cx.spawn(async move |this, cx| {
            let start = match start.await {
                Ok(s) => s,
                Err(e) => {
                    this.update(cx, |this, cx| {
                        this.browser_login = None;
                        haptics::play(Haptic::Error);
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
                    Err(e) => {
                        haptics::play(Haptic::Error);
                        this.sign_in_error = Some(e);
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn connect(&mut self, account: Account, cx: &mut Context<Self>) {
        if self.offline { return; }
        haptics::play(Haptic::Success);
        self.cloud_machine = cloud_machine::MachineState::default();
        if let Some(c) = self.connection.take() {
            c.stop();
        }
        self.session = SessionState::default();
        self.account = Some(account);
        self.synced = Default::default();
        self.transfer_input.update(cx, |input, cx| input.set_text("", cx));
        self.register_push(cx);
        self.start_connection(None, cx);
        self.screen = Screen::Chat;
        cx.notify();
    }

    /// Open a connection for the active tab, resuming `session_id` if given.
    fn start_connection(&mut self, resume: Option<String>, cx: &mut Context<Self>) {
        self.start_connection_in(resume, None, cx);
    }

    fn start_connection_in(&mut self, resume: Option<String>, working_dir: Option<String>, cx: &mut Context<Self>) {
        self.start_connection_target(resume, working_dir, None, cx);
    }

    fn start_connection_target(&mut self, resume: Option<String>, working_dir: Option<String>, fork_from: Option<String>, cx: &mut Context<Self>) {
        self.cancel_voice();
        if self.offline { return; }
        let Some(account) = self.account.clone() else { return };
        let mut config = Config::cloud(account);
        config.working_dir = working_dir;
        config.fork_from = fork_from;
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
        haptics::play(Haptic::Light);
        let id = self.next_tab;
        self.next_tab += 1;
        let at = self.tab_order.iter().position(|t| *t == self.active_tab).map_or(self.tab_order.len(), |i| i + 1);
        self.tab_order.insert(at, id);
        self.park_active(cx);
        self.active_tab = id;
        self.session = SessionState::default();
        self.start_connection(None, cx);
        cx.notify();
    }

    fn park_active(&mut self, cx: &mut Context<Self>) {
        self.cancel_voice();
        self.parked.push(ParkedTab {
            id: self.active_tab,
            connection: self.connection.take(),
            session: std::mem::take(&mut self.session),
            scroll: std::mem::replace(&mut self.transcript_scroll, ScrollHandle::new()),
            expanded_tools: std::mem::take(&mut self.expanded_tools),
            follow: self.follow,
            draft: self.composer.read(cx).save_draft(),
        });
        self.composer.update(cx, |input, cx| input.restore_draft(InputDraft::default(), cx));
    }

    fn focus_tab(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(i) = self.parked.iter().position(|p| p.id == id) else { return };
        self.park_active(cx);
        let p = self.parked.swap_remove(i);
        self.active_tab = p.id;
        self.connection = p.connection;
        self.session = p.session;
        self.transcript_scroll = p.scroll;
        self.expanded_tools = p.expanded_tools;
        self.follow = p.follow;
        self.follow_last = std::time::Instant::now();
        self.follow_max = self.transcript_scroll.max_offset().y;
        self.composer.update(cx, |input, cx| input.restore_draft(p.draft, cx));
        crate::haptics::play(crate::haptics::Haptic::Selection);
        cx.notify();
    }

    fn close_tab(&mut self, id: u64, cx: &mut Context<Self>) {
        if self.tab_order.len() <= 1 {
            return;
        }
        let Some(pos) = self.tab_order.iter().position(|t| *t == id) else { return };
        haptics::play(Haptic::Rigid);
        self.tab_order.remove(pos);
        self.tab_motion.remove(&id);
        if id == self.active_tab {
            self.cancel_voice();
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
            self.follow_last = std::time::Instant::now();
            self.follow_max = self.transcript_scroll.max_offset().y;
            self.composer.update(cx, |input, cx| input.restore_draft(p.draft, cx));
        } else if let Some(i) = self.parked.iter().position(|p| p.id == id) {
            if let Some(c) = self.parked.swap_remove(i).connection {
                c.stop();
            }
        }
        cx.notify();
    }

    fn render_tab_menu(&self, id: u64, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let s = self.tab_session(id);
        let title = s.and_then(|s| s.session_title.clone()).unwrap_or_else(|| "New session".into());
        let session_id = s.and_then(|s| s.session_id.clone());
        let many = self.tab_order.len() > 1;
        let can_fork = self.account.is_some() && !self.offline && session_id.is_some();
        let row = |key: &'static str, text: &'static str, color: Hsla, enabled: bool| {
            div()
                .id(key)
                .debug_selector(move || key.into())
                .px(px(12.))
                .py(px(12.))
                .rounded(px(10.))
                .when(enabled, |d| d.cursor_pointer().hover(|d| d.bg(t.surface)))
                .child(label(text, if enabled { color } else { t.text_muted }, 14.))
        };
        let close = |this: &mut Self, cx: &mut Context<Self>| {
            this.tab_menu = None;
            cx.notify();
        };
        let mut list = div().flex().flex_col().px(px(8.)).pt(px(6.));
        if id != self.active_tab {
            list = list.child(row("tm-open", "Switch to tab", t.text, true).on_click(cx.listener(
                move |this, _, _, cx| {
                    close(this, cx);
                    this.focus_tab(id, cx)
                },
            )));
        }
        if let Some(sid) = session_id {
            list = list.child(row("tm-copy", "Copy session ID", t.text, true).on_click(cx.listener(
                move |this, _, _, cx| {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(sid.clone()));
                    haptics::play(Haptic::Success);
                    close(this, cx)
                },
            )));
        }
        list = list
            .child(row("tm-rename", "Rename", t.text, true).on_click(cx.listener(move |this, _, _, cx| {
                this.feel_rename_tab(id, cx)
            })))
            .child(row("tm-fork", "Fork session", if can_fork { t.text } else { t.text_muted }, can_fork)
                .when(can_fork, |d| d.on_click(cx.listener(move |this, _, _, cx| {
                    this.fork_tab(id, cx)
                }))))
            .child(row("tm-others", "Close other tabs", t.text, many).when(many, |d| {
                d.on_click(cx.listener(move |this, _, _, cx| {
                    close(this, cx);
                    this.focus_tab(id, cx);
                    for other in this.tab_order.clone() {
                        if other != id {
                            this.close_tab(other, cx);
                        }
                    }
                }))
            }))
            .child(row("tm-close", "Close tab", t.error, many).when(many, |d| {
                d.on_click(cx.listener(move |this, _, _, cx| {
                    close(this, cx);
                    this.close_tab(id, cx)
                }))
            }));
        div()
            .absolute()
            .inset_0()
            .flex()
            .flex_col()
            .bg(gpui::black().opacity(0.5))
            .child(div().id("tm-dismiss").debug_selector(|| "tm-dismiss".into()).flex_1().on_click(cx.listener(move |this, _, _, cx| close(this, cx))))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .bg(t.surface_elevated)
                    .rounded_t(px(18.))
                    .border_t_1()
                    .border_color(t.border)
                    .pt(px(8.))
                    .pb(px(16.))
                    .child(
                        div()
                            .w_full()
                            .flex()
                            .justify_center()
                            .child(div().w(px(36.)).h(px(5.)).rounded_full().bg(t.text_muted)),
                    )
                    .child(label(title, t.text_strong, 16.).px(px(20.)).pt(px(8.)).truncate())
                    .child(list),
            )
            .into_any_element()
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
                                for a in p.session.alerts.drain(..) {
                                    crate::notify::post(&a);
                                }
                            }
                            this.sync_live_activity();
                            cx.notify();
                            return;
                        }
                        let was = this.session.is_processing;
                        for o in batch {
                            this.session.apply(o);
                        }
                        ios_feel::turn_haptic(was, &this.session);
                        for a in this.session.alerts.drain(..) {
                            crate::notify::post(&a);
                        }
                        let busy = this.session.is_processing
                            || this.parked.iter().any(|p| p.session.is_processing);
                        crate::notify::keep_alive(busy);
                        this.sync_live_activity();
                        if this.session.phase == ConnectionPhase::Connected
                            && this.session.session_id.is_some()
                            && let Some(msg) = this.auto_send.take()
                        {
                            this.composer.update(cx, |t, cx| t.set_text(msg, cx));
                            this.send_draft(cx);
                        }
                        // Follow mode glides to the new end in render_chat.
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
        if self.offline { return; }
        if let Some(c) = &self.connection {
            c.send(build);
        }
    }

    fn send_draft(&mut self, cx: &mut Context<Self>) {
        let text = self.composer.read(cx).text().trim().to_string();
        if text.is_empty() || self.session.phase != ConnectionPhase::Connected || self.session.session_id.is_none() {
            return;
        }
        static ASKED: std::sync::Once = std::sync::Once::new();
        self.cancel_voice();
        if !self.offline {
            ASKED.call_once(crate::notify::request_permission);
        }
        if self.session.is_processing {
            self.session.apply_intent(LocalIntent::UserQueuedInterrupt(text.clone()));
            self.send(|id| Request::SoftInterrupt { id, content: text, urgent: false });
        } else {
            self.session.apply_intent(LocalIntent::UserSentMessage(text.clone()));
            self.send(|id| Request::Message { id, content: text });
        }
        self.feel_sent();
        self.composer.update(cx, |t, cx| t.set_text("", cx));
        self.follow = true;
        // render_chat glides to the end.
        cx.notify();
    }

    fn cancel(&mut self, cx: &mut Context<Self>) {
        if !self.session.is_processing { return; }
        haptics::play(Haptic::Medium);
        self.send(|id| Request::Cancel { id });
        cx.notify();
    }

    /// First tap arms, second tap deletes the account server-side, then signs out.
    fn delete_account_tap(&mut self, cx: &mut Context<Self>) {
        if self.offline { return; }
        let Some(account) = self.account.clone() else { return };
        match self.delete_step {
            0 => {
                haptics::play(Haptic::Warning);
                self.delete_step = 1;
            }
            1 => {
                haptics::play(Haptic::Rigid);
                self.delete_step = 2;
                self.delete_error = None;
                let task = cx.background_spawn(async move { cloud::delete_account(&account) });
                cx.spawn(async move |this, cx| {
                    let result = task.await;
                    this.update(cx, |this, cx| {
                        this.delete_step = 0;
                        match result {
                            Ok(()) => this.sign_out(cx),
                            Err(e) => {
                                haptics::play(Haptic::Error);
                                this.delete_error = Some(e);
                            }
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
        if self.account.is_some() { haptics::play(Haptic::Rigid); }
        self.cloud_machine = cloud_machine::MachineState::default();
        self.cancel_voice();
        if !self.offline {
            ios_system::forget_index();
        }
        self.unregister_push(cx);
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
        if !self.offline {
            self.store.clear();
        }
        self.account = None;
        self.composer.update(cx, |input, cx| input.restore_draft(InputDraft::default(), cx));
        self.synced = Default::default();
        self.transfer_input.update(cx, |input, cx| input.set_text("", cx));
        self.apple_link = None;
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
    draft: InputDraft,
    expanded_tools: std::collections::HashSet<String>,
    follow: bool,
}

fn label(text: impl Into<SharedString>, color: Hsla, size: f32) -> gpui::Div {
    div().text_size(crate::system::fs(size)).text_color(color).child(text.into())
}

/// Desktop's `composer_pill`: 22px rounded-full chip on prompt paper,
/// 10.5px dim text that truncates.
fn pill(id: &'static str, text: impl Into<SharedString>, t: &Theme) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .debug_selector(move || id.into())
        .a11y_text_button()
        .flex_none()
        .h(px(22.))
        .px(px(10.))
        .flex()
        .items_center()
        .rounded_full()
        .bg(t.surface)
        .text_size(crate::system::fs(10.5))
        .text_color(t.text_secondary)
        .whitespace_nowrap()
        .overflow_hidden()
        .cursor_pointer()
        .child(div().min_w_0().truncate().child(text.into()))
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
        .text_size(crate::system::fs(14.))
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

fn inline_orb() -> gpui_thinking_orbs::ThinkingOrb {
    use gpui_thinking_orbs::{OrbSize, OrbTheme, ThinkingOrb};
    ThinkingOrb::new().size(OrbSize::Inline).theme(OrbTheme::Dark)
}

/// Desktop's mapping (panel_activity.rs) from activity to orb animation.
fn orb_state(activity: &str) -> gpui_thinking_orbs::OrbState {
    use gpui_thinking_orbs::OrbState;
    match activity {
        "thinking" => OrbState::Reasoning,
        "responding" => OrbState::Composing,
        "running tools" => OrbState::Solving,
        _ => OrbState::Working,
    }
}

/// An orb followed by a status line.
fn orb_row(orb: &Entity<gpui_thinking_orbs::ThinkingOrb>, text: impl Into<SharedString>, color: Hsla) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .child(div().flex_none().size(px(20.)).child(orb.clone()))
        .child(label(text, color, 12.))
}

fn spinner_glyph(step: usize) -> &'static str {
    ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧"][step % 8]
}

impl MobileApp {
    fn render_sign_in(&mut self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let code_step = self.login_token.is_some();
        // Keep an explicit action: the iOS number pad has no return key.
        let status: Option<String> = match (code_step, self.signing_in) {
            (false, true) => Some("Sending a code…".into()),
            (true, true) => Some("Signing in…".into()),
            (true, false) => Some(format!(
                "We emailed a code to {}. Enter it above.",
                self.email.read(cx).text().trim()
            )),
            (false, false) => None,
        };
        let apple = (!code_step).then(|| self.render_apple_button(cx));
        // Fixed layout: branding fills whatever space is left above, the form
        // is pinned to the bottom. The screen's bottom padding grows with the
        // keyboard, so the form rides up above it and nothing else moves.
        div()
            .id("sign-in")
            .on_mouse_down(gpui::MouseButton::Left, |_, window, cx| {
                window.blur(cx);
                window.dismiss_virtual_keyboard();
            })
            .size_full()
            .min_w_0()
            .flex()
            .flex_col()
            .px(px(20.))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap(px(18.))
                    .child(crate::donut::spinning_donut(140., t.text_strong, self.started))
                    .child(label("Sign in to Jcode Cloud", t.text_strong, 22.).w_full().min_w_0().whitespace_normal().text_center().line_height(crate::system::fs(30.))),
            )
            .child(
                div()
                    .id("sign-in-form")
                    .w_full()
                    .min_w_0()
                    .max_h_full()
                    .overflow_y_scroll()
                    .flex_none()
                    .flex()
                    .flex_col()
                    .gap(px(14.))
                    // Keyboard hidden: a spacer holds the form near the
                    // middle. It shrinks as the keyboard slides up, so the
                    // form glides onto the keyboard instead of jumping.
                    .pb(px(16. + (220. - self.kb_lift).max(0.)))
                    .when_some(self.sign_in_error.clone(), |d, e| d.child(banner(&e, t.error, t)))
                    .children(apple)
                    // Apple is offered alongside Google on iOS and in the
                    // phone preview. Both use the same Jcode account backend.
                    .when(!code_step, |d| {
                        let waiting = self.browser_login.is_some();
                        d.child(
                            div()
                                .id("google")
                                .debug_selector(|| "google".into())
                                .w_full()
                                .h(px(44.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .gap(px(10.))
                                .rounded_full()
                                .bg(gpui::white())
                                .text_color(gpui::black())
                                .text_size(crate::system::fs(14.))
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
                                .child(label("or use email", t.text_muted, 12.))
                                .child(div().flex_1().h(px(1.)).bg(t.border)),
                        )
                    })
                    .child(if code_step { self.code.clone() } else { self.email.clone() })
                    .child(
                        outline_button(
                            "submit-sign-in",
                            if self.signing_in {
                                if code_step { "Signing in…" } else { "Sending code…" }
                            } else if code_step {
                                "Verify code"
                            } else {
                                "Send code"
                            },
                            !self.signing_in,
                            t,
                        )
                        .a11y_text_button()
                        .flex_none()
                        .on_click(cx.listener(|this, _, _, cx| this.sign_in_step(cx))),
                    )
                    .when_some(status, |d, s| {
                        let row = if self.signing_in {
                            orb_row(&self.status_orb, s, t.text_secondary)
                        } else {
                            div().child(label(s, t.text_secondary, 13.).w_full().min_w_0().whitespace_normal().text_center())
                        };
                        d.child(row.w_full().min_w_0().flex().justify_center())
                    })
                    .when(code_step && !self.signing_in, |d| {
                        d.child(
                            div()
                                .id("use-other-email")
                                .w_full()
                                .flex()
                                .justify_center()
                                .cursor_pointer()
                                .child(label("Use a different email", t.text_muted, 12.))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.login_token = None;
                                    this.sign_in_error = None;
                                    window.focus(&this.email.focus_handle(cx), cx);
                                    cx.notify()
                                })),
                        )
                    }),
            )
            .into_any_element()
    }

    /// Desktop's live tabs (live_tabs.rs): ≡, a centered strip of session
    /// tabs that shrink and then overlap like folder tabs to fit, and +.
    fn render_header(&mut self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        use crate::tabs;
        // Phone width minus side buttons and padding.
        let width = self.content_width();
        let track = (width - 2. * 10. - 2. * 44. - 2. * 6.).max(40.);
        let count = self.tab_order.len();
        let selected = self.tab_order.iter().position(|id| *id == self.active_tab).unwrap_or(0);
        let preview = self.swipe.preview(selected > 0, selected + 1 < count);
        let destination = preview.map(|(direction, progress)| {
            ((selected as i32 + direction) as usize, progress)
        });
        let morph = destination.filter(|_| !crate::system::reduce_motion());
        let targets = if let Some((target, progress)) = morph {
            tabs::swipe_layout(track, count, selected, target, progress)
        } else {
            tabs::layout(track, count, selected)
        };
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
                if snap || morph.is_some() {
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
        let visual_selected = morph.filter(|(_, progress)| *progress >= 0.5)
            .map(|(target, _)| target).unwrap_or(selected);
        let order = tabs::paint_order(count, visual_selected);
        let mut strip = div().id("session-tabs").role(gpui::Role::TabList)
            .relative().flex_none().w(px(track)).h(px(44.));
        for &pos in &order {
            let id = self.tab_order[pos];
            let g = geometry[pos];
            let focused = pos == selected;
            // Derive the outline from animated geometry so cancellation and
            // tap-driven switches fade with the size, rather than jumping.
            let outline_weight = ((g.height - (tabs::TAB_HEIGHT - 2.0)) / 2.0).clamp(0.0, 1.0);
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
            // Focused tab lifts slightly off the bar; others sit flush with it.
            let held = self.feel.g.tab_scrub().is_some_and(|(origin, dx)| {
                if dx.abs() < crate::gestures::SLOP { origin == id } else { focused }
            });
            let bg = if held { t.accent.opacity(0.18) } else if focused { t.raised } else { t.background };
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
                        label(title.clone(), if focused { t.text_strong } else { t.text_secondary }, 12.)
                            .min_w_0()
                            .truncate(),
                    )
                });
            strip = strip.child(
                div()
                    .id(("tab", id))
                    .debug_selector(move || format!("tab-{id}"))
                    .role(gpui::Role::Tab)
                    .aria_label(title)
                    .aria_selected(focused)
                    .absolute()
                    .left(px(g.left))
                    .bottom_0()
                    .w(px(g.width))
                    .h(px(44.))
                    .cursor_pointer()
                    // Preserve the compact pill while making the full row a
                    // forgiving touch target, including its long-press area.
                    .child(div().absolute().left_0().right_0()
                        .top(px((44. - g.height) / 2.)).h(px(g.height))
                        .rounded_full().border_1()
                        .border_color(if held { t.accent } else { t.border.opacity(outline_weight) })
                        .bg(bg))
                    .child(content)
                    .child(self.feel_tab_hit(id))
                    // The root recognizer owns hold-to-scrub; quick taps switch.
                    .on_mouse_up(gpui::MouseButton::Left, cx.listener(|this, _, _, _| this.tab_hold = None))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.tab_hold = None;
                        if !std::mem::take(&mut this.hold_fired) {
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
            .bg(t.background)
            .child(
                div()
                    .id("menu").a11y_button("Sessions")
                    .debug_selector(|| "menu".into())
                    .flex_none()
                    .size(px(44.))
                    .rounded_full()
                    .active(|s| s.bg(t.surface))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(t.text_secondary)
                    .text_size(crate::system::fs(16.))
                    .cursor_pointer()
                    .child("≡")
                    .on_click(cx.listener(|this, _, window, cx| this.open_drawer(window, cx))),
            )
            .child(strip)
            .child(
                div()
                    .id("new-session").a11y_button("New session")
                    .debug_selector(|| "new-session".into())
                    .flex_none()
                    .size(px(44.))
                    .rounded_full()
                    .when(connected, |d| d.active(|s| s.bg(t.surface)))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(if connected { t.text_strong } else { t.text_muted })
                    .text_size(crate::system::fs(20.))
                    .cursor_pointer()
                    .child("+")
                    .on_click(cx.listener(|this, _, _, cx| this.new_tab(cx))),
            )
    }

    /// Tab ids left and right of the active one.
    fn tab_neighbors(&self) -> (Option<u64>, Option<u64>) {
        let i = self.tab_order.iter().position(|id| *id == self.active_tab).unwrap_or(0);
        let prev = i.checked_sub(1).and_then(|p| self.tab_order.get(p).copied());
        (prev, self.tab_order.get(i + 1).copied())
    }

    fn end_swipe(&mut self, cx: &mut Context<Self>) {
        let width = self.content_width();
        let (prev, next) = self.tab_neighbors();
        let now = std::time::Instant::now();
        let offer_new = self.account.is_some() && self.swipe.new_session_preview(next.is_some()) == Some(1.);
        if let crate::swipe::Release::Switch(dir) = self.swipe.release(width, prev.is_some(), next.is_some(), now) {
            if let Some(id) = if dir < 0 { prev } else { next } {
                self.focus_tab(id, cx);
            }
        }
        if offer_new {
            self.offer_new_session(cx);
        }
        cx.notify();
    }

    /// Chat panel that slides with a horizontal swipe and settles into the neighbor tab.
    fn render_chat(&mut self, t: &Theme, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let width = self.content_width();
        let now = std::time::Instant::now();
        // Trackpads without touch phases: a pause ends the gesture.
        if self.swipe.active && self.swipe.last_event.is_some_and(|e| now.duration_since(e) > crate::swipe::IDLE_END) {
            self.end_swipe(cx);
        }
        let (prev, next) = self.tab_neighbors();
        let (offset, settling) = self.swipe.offset(width, prev.is_some(), next.is_some(), now);
        let neighbor = if offset < 0. { next } else if offset > 0. { prev } else { None };
        // Enlarge the restrained nudge into a useful peek, including the same
        // geometry during cancellation so the two panels snap back together.
        let offset = if neighbor.is_some() { (offset * 6.).clamp(-width * 0.8, width * 0.8) } else { offset };
        let preview = neighbor.and_then(|id| self.render_panel_preview(id, t));
        if settling || self.swipe.active {
            window.request_animation_frame();
        }
        let inner = self.render_chat_inner(t, window, cx);
        let new_session_progress = self.account.as_ref().and_then(|_| self.swipe.new_session_preview(next.is_some()));
        div()
            .id("chat-swipe")
            .relative()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .overflow_hidden()
            .when_some(new_session_progress, |d, progress| {
                d.child(label(if progress >= 1. { "Release to choose a directory for a new session" } else { "Keep swiping to create a new session →" }, t.accent, 12.)
                    .px(px(16.)).py(px(8.)))
            })
            .on_scroll_wheel(cx.listener(|this, ev: &gpui::ScrollWheelEvent, _window, cx| {
                if ev.touch_phase == gpui::TouchPhase::Cancelled {
                    this.swipe.cancel(this.content_width(), std::time::Instant::now());
                    cx.notify();
                    return;
                }
                let d = ev.delta.pixel_delta(px(20.));
                let now = std::time::Instant::now();
                if this.swipe.feed(f32::from(d.x), f32::from(d.y), now) {
                    // Not window.request_animation_frame(): it reads the render
                    // stack and panics outside render. Render re-arms frames.
                    cx.notify();
                }
                if ev.touch_phase == gpui::TouchPhase::Ended {
                    this.end_swipe(cx);
                }
            }))
            .child(
                div()
                    .relative()
                    .left(px(offset))
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .child(inner),
            )
            .when_some(preview, |d, preview| {
                d.child(div().absolute().top_0().bottom_0()
                    .left(px(if offset < 0. { width + offset } else { -width + offset }))
                    .w(px(width)).overflow_hidden().bg(t.background)
                    .border_l_1().border_color(t.border)
                    .child(preview))
            })
            .into_any_element()
    }

    fn render_chat_inner(&mut self, t: &Theme, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        // Follow is released by upward input, never by streamed layout growth.
        let max = self.transcript_scroll.max_offset().y;
        let mut off = -self.transcript_scroll.offset().y;
        // Streamed text grows the transcript a chunk at a time. Ease toward
        // the end instead of snapping, so following reads as one glide.
        let now = std::time::Instant::now();
        let dt = now.duration_since(self.follow_last).as_secs_f32().min(0.05);
        self.follow_last = now;
        // Layout lags one frame behind new content: keep ticking while the
        // end moves, or the glide stops short and text hides under the composer.
        let grew = max != self.follow_max;
        self.follow_max = max;
        if self.follow && grew {
            window.request_animation_frame();
        }
        if self.follow && off < max {
            off = px(crate::scroll_follow::offset(
                f32::from(off), f32::from(max),
                f32::from(self.feel.g.transcript.map_or(window.viewport_size().height, |b| b.size.height)),
                dt, crate::system::reduce_motion(),
            ));
            self.transcript_scroll.set_offset(gpui::point(px(0.), -off));
            if off < max {
                window.request_animation_frame();
            }
        }
        let at_bottom = max - off < px(56.);
        let entries = &self.session.transcript;
        let mut prompt_n = 0;
        let mut rows: Vec<AnyElement> = Vec::new();
        for e in entries {
            if e.role == Role::User {
                prompt_n += 1;
            }
            let el = self.render_entry(e, prompt_n, t, cx);
            rows.push(self.feel_hit(crate::gestures::Target::Message(e.id), el));
        }
        if let Some(a) = self.activity() {
            self.activity_orb.update(cx, |o, cx| o.set_state(orb_state(a), cx));
            rows.push(orb_row(&self.activity_orb, a, t.text_muted).into_any_element());
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
        if empty && self.session.phase == ConnectionPhase::Connected {
            self.ensure_recent_dirs();
        }
        let recent = if empty { self.render_recent_dirs(t, cx) } else { None };
        let body = if empty {
            // Desktop's fresh session: the question sits just above the composer,
            // with recent cloud dirs as quick nav in the space above it.
            div()
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .justify_end()
                .px(px(16.))
                .pb(px(14.))
                .gap(px(12.))
                .when(!self.feel_reconnect_visible(), |d| d.child(self.connection_status(t)))
                .when_some(self.session.error_banner.clone(), |d, e| {
                    d.child(banner(&e, t.error, t).id("err-empty").on_click(cx.listener(|this, _, _, cx| {
                        this.session.apply_intent(LocalIntent::DismissError);
                        this.reconnect(cx);
                    })))
                })
                .child(label("What would you like to work on?", t.text, 20.).line_height(crate::system::fs(28.)))
                .children(recent)
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
                        .on_scroll_wheel(cx.listener(|this, ev: &gpui::ScrollWheelEvent, window, cx| {
                            let max = this.transcript_scroll.max_offset().y;
                            let off = -this.transcript_scroll.offset().y;
                            let delta = ev.delta.pixel_delta(window.line_height());
                            // GPUI applies scrolling separately. Use direction to
                            // release immediately, regardless of listener order
                            // or how far the streaming animation trails the end.
                            if delta.y.abs() >= delta.x.abs() {
                                this.follow = crate::scroll_follow::follow_after_scroll(
                                    this.follow, f32::from(delta.y), f32::from(max - off),
                                );
                            }
                            cx.notify();
                        }))
                        .child(div().w_full().min_w_0().flex().flex_col().gap(px(14.)).px(px(14.)).py(px(12.)).children(rows))
                        .child({
                            let scroll = self.transcript_scroll.clone();
                            let follow = self.follow;
                            // Render sees the previous layout's maximum. Check
                            // again after layout so a lone tool update cannot
                            // leave the tail hidden until the next network event.
                            gpui::canvas(|_, _, _| (), move |_, _, window, _| {
                                if follow && scroll.max_offset().y != max {
                                    window.request_animation_frame();
                                }
                            }).absolute().inset_0()
                        }),
                )
                .when(!at_bottom && max > px(0.), |d| {
                    d.child(
                        div()
                            .id("jump").a11y_button("Jump to latest")
                            .absolute()
                            .right(px(14.))
                            .bottom(px(8.))
                            .size(px(44.))
                            .rounded_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(t.background)
                            .border_1()
                            .border_color(t.border_focus)
                            .text_color(t.text_strong)
                            .cursor_pointer()
                            .active(|s| s.bg(t.surface))
                            .child("↓")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.follow = true;
                                // render_chat glides to the end.
                                cx.notify()
                            })),
                    )
                })
                .into_any_element()
        };
        let body = self.feel_transcript(body, t);
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(self.render_header(t, cx))
            .when(!empty && !self.feel_reconnect_visible() && !matches!(self.session.phase, ConnectionPhase::Connected | ConnectionPhase::Failed { .. }), |d| {
                d.child(div().px(px(14.)).py(px(6.)).border_b_1().border_color(t.border)
                    .child(self.connection_status(t)))
            })
            .child(body)
            .child(self.render_composer(t, window, cx))
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
        orb_row(&self.status_orb, text, t.text_muted).into_any_element()
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
            // Desktop prompt: round number badge beside a USER_BG card.
            Role::User => {
                let bg = Theme::prompt_background(0);
                let badge_w = (prompt_n.to_string().len() as f32 * 7. + 8.).max(20.);
                div()
                    .flex()
                    .items_start()
                    .gap(px(6.))
                    .when(e.is_queued, |d| d.opacity(0.6))
                    .child(
                        div()
                            .flex_none()
                            .mt(px(4.))
                            .w(px(badge_w))
                            .h(px(20.))
                            .rounded_full()
                            .bg(bg)
                            .text_center()
                            .text_size(crate::system::fs(10.))
                            .line_height(crate::system::fs(20.))
                            .text_color(t.text_secondary)
                            .child(prompt_n.to_string()),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .flex_shrink_1()
                            .px(px(8.))
                            .py(px(4.))
                            .rounded(px(8.))
                            .bg(bg)
                            .text_size(crate::system::fs(13.5))
                            .line_height(crate::system::fs(21.))
                            .text_color(t.text_strong)
                            .child(e.text.clone()),
                    )
                    .into_any_element()
            }
            Role::System => label(e.text.clone(), t.text_muted, 12.).into_any_element(),
            Role::Assistant => {
                let mut col = div().w_full().min_w_0().flex().flex_col().gap(px(6.));
                if !e.reasoning.is_empty() {
                    col = col.child(crate::markdown::render_reasoning(&e.reasoning, t));
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
                haptics::play(Haptic::Selection);
                cx.notify()
            }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .text_size(crate::system::fs(12.))
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
                        .text_size(crate::system::fs(11.))
                        .text_color(t.text_secondary)
                        .child(if body.is_empty() { call.input.clone() } else { body }),
                )
            })
            .into_any_element()
    }

    /// Phone composer: 44pt actions inside a 22px-radius focused outline,
    /// then desktop's pill row: model, effort, `Provider · Auth` on the left,
    /// the machine chip on the right.
    fn render_composer(&self, t: &Theme, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let processing = self.session.is_processing;
        let ready = self.session.phase == ConnectionPhase::Connected && self.session.session_id.is_some();
        let has_text = !self.composer.read(cx).text().trim().is_empty();
        let focused = self.composer.read(cx).is_focused(window);
        let model_raw = self.session.model_name.clone();
        let model = crate::labels::pretty_model(model_raw.as_deref());
        let effort = self
            .session
            .reasoning_effort
            .clone()
            .filter(|e| !e.trim().is_empty() && model_raw.is_some());
        // The daemon's resolved credential wins; guessing from routes is
        // ambiguous when one model has both an OAuth and an API-key route.
        let api_method = self.session.auth_method.clone().or_else(|| {
            self.session
                .routes
                .iter()
                .find(|r| {
                    Some(&r.model) == model_raw.as_ref()
                        && self.session.provider_name.as_deref().is_some_and(|p| r.provider.eq_ignore_ascii_case(p))
                })
                .map(|r| r.api_method.clone())
        });
        let account = crate::labels::routed_account_label(
            self.session.provider_name.as_deref(),
            api_method.as_deref(),
            self.session.account_label.as_deref(),
        );
        let open_picker = cx.listener(|this, _, window, cx| {
            window.blur(cx);
            if !this.model_picker { haptics::play(Haptic::Light); }
            this.model_picker = true;
            cx.notify();
        });
        div()
            .id("chat-composer")
            .debug_selector(|| "chat-composer".into())
            .flex()
            .flex_col()
            .flex_none()
            .min_w_0()
            .gap(px(4.))
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
                    .pr(px(4.))
                    .min_h(px(48.))
                    .rounded(px(22.))
                    .bg(t.field)
                    .border_1()
                    .border_color(if focused { t.border_focus } else { t.input_border })
                    .child(div().flex_1().min_w_0().child(self.composer.clone()))
                    .when(cfg!(target_os = "ios"), |d| d.child(
                        round_button("microphone", if self.voice.recording.is_some() { "■" } else if self.voice.busy { "…" } else { "🎙" },
                            if self.voice.recording.is_some() { t.error } else { t.text_strong }, !self.voice.busy, t)
                            .a11y_button(if self.voice.recording.is_some() { "Stop recording and transcribe" } else { "Dictate message" })
                            .when(!self.voice.busy, |d| d.on_click(cx.listener(|this, _, _, cx| {
                                if !this.offline && !this.voice.busy && this.account.is_some() {
                                    haptics::play(Haptic::Light);
                                }
                                this.toggle_voice(cx);
                            })))
                    ))
                    .child(if processing && !has_text {
                        round_button("stop", "■", t.error, ready, t)
                            .a11y_button("Stop response")
                            .when(ready, |d| d.on_click(cx.listener(|this, _, _, cx| this.cancel(cx))))
                    } else {
                        round_button("send", if ready { "↑" } else { "…" }, t.text_strong, ready && has_text, t)
                            .a11y_button(if !ready { "Waiting for connection" } else if processing { "Queue follow-up" } else { "Send message" })
                            .when(ready && has_text, |d| d.on_click(cx.listener(|this, _, _, cx| this.send_draft(cx))))
                    }),
            )
            .when_some(self.voice.status(), |d, status| d.child(label(status, t.text_muted, 10.5).px(px(8.))))
            .when(self.voice.busy || self.voice.recording.is_some(), |d| d.child(
                pill("cancel-dictation", "Cancel dictation", t)
                    .a11y_button("Discard voice recording")
                    .on_click(cx.listener(|this, _, _, cx| {
                        if this.voice.busy || this.voice.recording.is_some() {
                            haptics::play(Haptic::Light);
                        }
                        this.cancel_voice();
                        cx.notify();
                    }))
            ))
            .when(!ready, |d| d.child(label(
                if matches!(self.session.phase, ConnectionPhase::Failed { .. } | ConnectionPhase::Disconnected) {
                    "Not connected. Your draft stays here."
                } else {
                    "Connecting… You can keep typing."
                }, t.text_muted, 10.5,
            ).px(px(8.))))
            .child(
                div()
                    .id("chat-metadata")
                    .debug_selector(|| "chat-metadata".into())
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(4.))
                    .px(px(4.))
                    .overflow_hidden()
                    .child(
                        div()
                            .flex()
                            .flex_auto()
                            .items_center()
                            .gap(px(4.))
                            .min_w_0()
                            .overflow_hidden()
                            .child(pill("pill-model", model, t).flex_shrink_1().min_w(px(48.)).on_click(open_picker))
                            .when_some(effort, |d, e| {
                                d.child(pill("pill-effort", e, t).flex_shrink_1().min_w(px(24.)))
                            })
                            .child(
                                pill("pill-account", account, t)
                                    .flex_shrink_1()
                                    .min_w(px(40.))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        window.blur(cx);
                                        if !this.model_picker { haptics::play(Haptic::Light); }
                                        this.model_picker = true;
                                        cx.notify();
                                    })),
                            ),
                    )
                    .child({
                        // Desktop's location label: cloud icon, repo name,
                        // then its dim parent path on the cloud VM.
                        let (name, parent) = crate::labels::cloud_location(self.session.working_dir.as_deref());
                        div()
                            .id("location")
                            .debug_selector(|| "location".into())
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _, window, cx| this.open_dir_picker(window, cx)))
                            .flex_shrink(8.)
                            .min_w_0()
                            .px(px(4.))
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .child(
                                gpui::svg()
                                    .flex_none()
                                    .path("icons/cloud.svg")
                                    .w(px(14.))
                                    .h(px(9.))
                                    .text_color(t.text_secondary),
                            )
                            .child(
                                div()
                                    .min_w_0()
                                    .flex_shrink_1()
                                    .truncate()
                                    .text_size(crate::system::fs(11.5))
                                    .font_weight(gpui::FontWeight::MEDIUM)
                                    .text_color(t.text_secondary)
                                    .child(name),
                            )
                            .when_some(parent, |d, p| {
                                d.child(
                                    div()
                                        .min_w_0()
                                        .truncate()
                                        .text_size(crate::system::fs(10.5))
                                        .text_color(t.text_muted)
                                        .child(p),
                                )
                            })
                    }),
            )
    }

    /// Bottom sheet listing every model route, grouped by model. Tapping one
    /// switches the session to that model on that provider.
    fn render_model_picker(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        use jcode_mobile_core::routes::{method_label, route_spec};
        let current_model = self.session.model_name.clone().unwrap_or_default();
        let current_provider = self.session.provider_name.clone().unwrap_or_default();
        let mut rows = div().flex().flex_col().gap(px(2.));
        let mut last_model = String::new();
        let routes = &self.session.routes;
        if routes.is_empty() {
            // Older servers: only bare model names.
            for (i, m) in self.session.available_models.iter().enumerate() {
                let spec = m.clone();
                let selected = *m == current_model;
                rows = rows.child(
                    picker_row(("bare", i), m.clone(), String::new(), selected, true, t)
                        .on_click(cx.listener(move |this, _, _, cx| this.pick_model(spec.clone(), cx))),
                );
            }
        }
        for (i, r) in routes.iter().enumerate() {
            if r.model != last_model {
                last_model = r.model.clone();
                rows = rows.child(label(r.model.clone(), t.text_strong, 13.).pt(px(10.)).pb(px(2.)).px(px(4.)));
            }
            let spec = route_spec(r);
            let selected = r.model == current_model && r.provider.eq_ignore_ascii_case(&current_provider);
            let detail = [method_label(r), if r.available { "" } else { r.detail.as_str() }]
                .into_iter()
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(" · ");
            let row = picker_row(("route", i), r.provider.clone(), detail, selected, r.available, t);
            rows = rows.child(if r.available {
                row.on_click(cx.listener(move |this, _, _, cx| this.pick_model(spec.clone(), cx)))
            } else {
                row
            });
        }
        let empty = routes.is_empty() && self.session.available_models.is_empty();
        div()
            .absolute()
            .inset_0()
            .flex()
            .flex_col()
            .bg(gpui::black().opacity(self.feel_backdrop()))
            .child(
                div()
                    .id("picker-dismiss")
                    .flex_1()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.feel_close_sheet();
                        cx.notify()
                    })),
            )
            .child(
                div()
                    .relative()
                    .top(px(self.feel_sheet_offsets().0))
                    .left(px(self.feel_sheet_offsets().1))
                    .mb(px(-self.feel_sheet_offsets().2))
                    .child(self.feel_sheet_recorder())
                    .flex()
                    .flex_col()
                    .max_h(px(520.))
                    .bg(t.surface_elevated)
                    .rounded_t(px(18.))
                    .border_t_1()
                    .border_color(t.border)
                    .pt(px(8.))
                    .pb(px(12.))
                    .child(
                        div()
                            .w_full()
                            .flex()
                            .justify_center()
                            .child(div().w(px(36.)).h(px(5.)).rounded_full().bg(t.text_muted)),
                    )
                    .child(label("Model", t.text_strong, 16.).px(px(16.)).pt(px(8.)))
                    .child(
                        div()
                            .id("picker-list")
                            .flex_1()
                            .overflow_y_scroll()
                            .px(px(12.))
                            .pb(px(8.))
                            .child(if empty {
                                label("Loading models…", t.text_muted, 13.).p(px(8.)).into_any_element()
                            } else {
                                rows.into_any_element()
                            }),
                    ),
            )
            .into_any_element()
    }

    fn pick_model(&mut self, spec: String, cx: &mut Context<Self>) {
        self.model_picker = false;
        let current = self.session.model_name.as_deref();
        let unchanged = if self.session.routes.is_empty() {
            current == Some(spec.as_str())
        } else {
            self.session.routes.iter().any(|route| {
                Some(route.model.as_str()) == current
                    && self.session.provider_name.as_deref().is_some_and(|provider| route.provider.eq_ignore_ascii_case(provider))
                    && jcode_mobile_core::routes::route_spec(route) == spec
            })
        };
        if !unchanged { haptics::play(Haptic::Selection); }
        self.send(|id| Request::SetModel { id, model: spec });
        cx.notify();
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
        let apple_link = self.render_apple_link(cx);
        let machine = self.render_cloud_machine(t, cx);
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
                            .id("back").a11y_button("Back")
                            .cursor_pointer()
                            .text_color(t.text_strong)
                            .text_size(crate::system::fs(13.))
                            .child("‹ back")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.feel_back();
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
                                .text_size(crate::system::fs(12.))
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
                                haptics::play(Haptic::Rigid);
                                this.session.apply_intent(LocalIntent::ClearedConversation);
                                this.send(|id| Request::Clear { id });
                                this.screen = Screen::Chat;
                                cx.notify()
                            },
                        )))
                        .child(machine)
                        .children(apple_link)
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

fn picker_row(
    id: (&'static str, usize),
    title: String,
    detail: String,
    selected: bool,
    available: bool,
    t: &Theme,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(gpui::ElementId::NamedInteger(id.0.into(), id.1 as u64))
        .flex()
        .items_center()
        .gap(px(8.))
        .px(px(12.))
        .py(px(10.))
        .rounded(px(10.))
        .when(selected, |d| d.bg(t.surface))
        .when(available, |d| d.cursor_pointer())
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(label(title, if available { t.text } else { t.text_muted }, 14.))
                .when(!detail.is_empty(), |d| d.child(label(detail, t.text_muted, 11.))),
        )
        .when(selected, |d| d.child(label("✓", t.text_strong, 14.)))
}

fn round_button(id: &'static str, glyph: &'static str, fg: Hsla, enabled: bool, t: &Theme) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .debug_selector(move || id.into())
        .size(px(44.))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .bg(t.surface_elevated)
        .text_color(fg)
        .text_size(crate::system::fs(15.))
        .when(enabled, |d| d.cursor_pointer().active(|s| s.bg(t.surface)))
        .when(!enabled, |d| d.opacity(0.4))
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
        .overflow_hidden()
        .child(label("✗", tint, 12.).flex_none())
        // min_w_0 lets long errors wrap instead of overflowing the card.
        .child(label(msg.to_string(), t.text, 12.).flex_1().min_w_0().whitespace_normal())
}

impl Render for MobileApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::notify::set_visible_session(self.session.session_id.clone());
        self.system_tick(window, cx);
        // Real phone: UIKit safe-area insets. Preview: the faked device insets.
        if std::mem::take(&mut self.focus_code) {
            window.focus(&self.code.focus_handle(cx), cx);
        }
        // Preview: the simulated keyboard shows whenever a field is focused
        // (JCODE_MOBILE_KEYBOARD=1 forces it, =0 disables it).
        let focused_input = self.focused_text_input(window, cx);
        let kb_env = std::env::var("JCODE_MOBILE_KEYBOARD").ok();
        let sim_kb = self.device.is_some()
            && kb_env.as_deref() != Some("0")
            && (focused_input.is_some() || kb_env.is_some());
        let (top, bottom) = if self.device.is_none() {
            let e = window.window_insets().effective();
            (f32::from(e.top), f32::from(e.bottom))
        } else {
            let kb = if sim_kb { crate::sim_keyboard::HEIGHT } else { 0. };
            (self.safe.top, self.safe.bottom.max(kb))
        };
        self.keyboard_up = f32::from(window.window_insets().ime.bottom) > 0. || sim_kb;
        // Ease the bottom inset instead of jumping, so the layout slides with
        // the keyboard (UIKit's own curve is ~250ms).
        let now = std::time::Instant::now();
        let dt = now.duration_since(self.inset_last).as_secs_f32().min(0.05);
        self.inset_last = now;
        let shown = match self.inset_anim {
            None => bottom,
            Some(v) => {
                let next = v + (bottom - v) * (1. - (-dt * 16.).exp());
                if (bottom - next).abs() < 0.5 { bottom } else { next }
            }
        };
        self.inset_anim = Some(shown);
        if shown != bottom {
            window.request_animation_frame();
        }
        let bottom = shown;
        let base = if self.device.is_none() {
            f32::from(window.window_insets().safe_area.bottom)
        } else {
            self.safe.bottom
        };
        self.kb_lift = (bottom - base).max(0.);
        if let Some(i) = &focused_input {
            self.kb_config = i.read(cx).config.clone();
        }
        // The simulated keyboard stays on screen while it slides away.
        let kb_visible = self.device.is_some() && bottom > self.safe.bottom + 0.5;
        let t = Theme::get(cx);
        let body = match self.screen {
            Screen::SignIn => self.render_sign_in(&t, cx),
            Screen::Chat => self.render_chat(&t, window, cx),
            Screen::Settings => self.render_settings(&t, cx),
        };
        let body = if let (Some(id), Screen::Chat) = (self.tab_menu, &self.screen) {
            div()
                .relative()
                .size_full()
                .flex()
                .flex_col()
                .child(body)
                .child(self.render_tab_menu(id, &t, cx))
                .into_any_element()
        } else {
            body
        };
        let body = if self.model_picker && matches!(self.screen, Screen::Chat) {
            div()
                .relative()
                .size_full()
                .flex()
                .flex_col()
                .child(body)
                .child(self.render_model_picker(&t, cx))
                .into_any_element()
        } else {
            body
        };
        let body = if self.dir_picker && matches!(self.screen, Screen::Chat) {
            div()
                .relative()
                .size_full()
                .flex()
                .flex_col()
                .child(body)
                .child(self.render_dir_picker(&t, cx))
                .into_any_element()
        } else {
            body
        };
        let body = self.feel_wrap(body, &t, window, cx);
        let body = self.wrap_wide(body, &t, window, cx);
        self.drawer_insets = (top, base);
        let drawer = self.render_drawer(&t, window, cx);
        let screen = div()
            .relative()
            .flex()
            .flex_col()
            .bg(t.background)
            .text_color(t.text)
            .font_family(FONT)
            .pt(px(top))
            .pb(px(bottom))
            .overflow_hidden()
            // Tapping outside a field dismisses the keyboard, like iOS.
            .on_mouse_down(gpui::MouseButton::Left, |_, window, cx| {
                window.blur(cx);
                window.dismiss_virtual_keyboard();
            })
            .child(body)
            .when(kb_visible, |d| {
                let this = cx.entity().downgrade();
                let on_key: std::rc::Rc<dyn Fn(crate::sim_keyboard::Key, &mut Window, &mut App)> =
                    std::rc::Rc::new(move |key, window, cx| {
                        this.update(cx, |this, cx| this.sim_key(key, window, cx)).ok();
                    });
                d.relative().child(
                    div()
                        .absolute()
                        .left_0()
                        .right_0()
                        .h(px(crate::sim_keyboard::HEIGHT))
                        .bottom(px(bottom - crate::sim_keyboard::HEIGHT))
                        .child(crate::sim_keyboard::render(
                            &self.sim_kb,
                            &self.kb_config,
                            self.safe.bottom,
                            on_key,
                            &t,
                        )),
                )
            })
            .children(drawer);
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
                        )
                        // GPUI overflow clipping is rectangular, even on a
                        // rounded div. Paint the bezel last so drawers and
                        // sheets cannot leak into the preview's corners.
                        .child(preview_corner_mask())
                        .child(div().absolute().inset_0().rounded(px(48.)).border_1().border_color(t.input_border)),
                )
                .into_any_element(),
            None => screen.size_full().into_any_element(),
        }
    }
}

fn preview_corner_mask() -> impl IntoElement {
    gpui::canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let width = f32::from(bounds.size.width);
            let height = f32::from(bounds.size.height);
            let radius = 48_f32.min(width / 2.).min(height / 2.);
            for (right, bottom) in [(false, false), (true, false), (false, true), (true, true)] {
                let point = |x: f32, y: f32| {
                    bounds.origin + gpui::point(px(if right { width - x } else { x }), px(if bottom { height - y } else { y }))
                };
                let mut path = gpui::PathBuilder::fill();
                path.move_to(point(0., 0.));
                path.line_to(point(radius, 0.));
                for step in 1..=48 {
                    let angle = std::f32::consts::FRAC_PI_2 * step as f32 / 48.;
                    path.line_to(point(radius * (1. - angle.sin()), radius * (1. - angle.cos())));
                }
                path.close();
                if let Ok(path) = path.build() {
                    window.paint_path(path, gpui::rgb(0x161616));
                }
            }
        },
    ).absolute().inset_0()
}

impl MobileApp {
    /// Keep keyboard visibility, configuration, and keystroke routing on the
    /// same field inventory, including fields in transient overlays.
    fn focused_text_input(&self, window: &Window, cx: &App) -> Option<Entity<TextInput>> {
        [&self.email, &self.code, &self.composer, &self.drawer_search,
            &self.dir_input, &self.transfer_input]
            .into_iter()
            .find(|input| input.read(cx).is_focused(window))
            .cloned()
            .or_else(|| self.focused_rename_input(window, cx))
            .or_else(|| self.apple_link.as_ref().and_then(|panel| panel.read(cx).focused_input(window, cx)))
    }

    /// A tap on the simulated keyboard, routed to the focused field.
    fn sim_key(&mut self, key: crate::sim_keyboard::Key, window: &mut Window, cx: &mut Context<Self>) {
        use crate::sim_keyboard::Key;
        let Some(input) = self.focused_text_input(window, cx) else {
            return;
        };
        match self.sim_kb.press(&key) {
            Some(Key::Char(c)) => input.update(cx, |t, cx| t.sim_insert(&c, window, cx)),
            Some(Key::Space) => input.update(cx, |t, cx| t.sim_insert(" ", window, cx)),
            Some(Key::Backspace) => input.update(cx, |t, cx| t.sim_backspace(window, cx)),
            Some(Key::Return) => {
                // "next" on the email field moves on like iOS, then sends the code.
                input.update(cx, |t, cx| t.sim_submit(cx));
            }
            _ => {}
        }
        cx.notify();
    }
}

pub fn init(cx: &mut App) {
    crate::system::install(cx);
    crate::text_input::bind_keys(cx);
}

#[cfg(test)]
mod monkey_tests {
    //! Headless random-gesture stress test. It drives the real MobileApp through
    //! gpui's test platform with mouse-to-touch emulation, so render-only APIs
    //! called from touch/scroll listeners panic here instead of on a phone.
    use super::*;
    use gpui::{
        Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PlatformInput,
        ScrollDelta, ScrollWheelEvent, TestAppContext, point, px,
    };

    fn rng(seed: &mut u64) -> f32 {
        *seed ^= *seed << 13;
        *seed ^= *seed >> 7;
        *seed ^= *seed << 17;
        (*seed % 10_000) as f32 / 10_000.
    }

    #[gpui::test]
    fn drafts_stay_with_their_tabs_and_return_when_active_tab_closes(cx: &mut TestAppContext) {
        cx.update(init);
        let (view, cx) = cx.add_window_view(|window, cx| {
            MobileApp::new(SafeArea { top: 47., bottom: 34. }, window, cx)
        });
        view.update(cx, |app, cx| {
            let first = app.active_tab;
            app.composer.update(cx, |input, cx| input.set_text("First draft 👋", cx));
            app.park_active(cx);
            assert_eq!(app.composer.read(cx).text(), "");
            let second = app.next_tab;
            app.next_tab += 1;
            app.active_tab = second;
            app.tab_order.push(second);
            app.composer.update(cx, |input, cx| input.set_text("Second draft", cx));
            app.focus_tab(first, cx);
            assert_eq!(app.composer.read(cx).text(), "First draft 👋");
            app.focus_tab(second, cx);
            assert_eq!(app.composer.read(cx).text(), "Second draft");
            app.close_tab(second, cx);
            assert_eq!(app.active_tab, first);
            assert_eq!(app.composer.read(cx).text(), "First draft 👋");
            assert!(app.parked.is_empty());
        });
    }

    #[gpui::test]
    fn primary_touch_targets_fit_small_phones_and_offline_send_preserves_draft(cx: &mut TestAppContext) {
        cx.update(init);
        let (view, cx) = cx.add_window_view(|window, cx| {
            let mut app = MobileApp::new(SafeArea { top: 20., bottom: 0. }, window, cx);
            app.screen = Screen::Chat;
            app.session.phase = ConnectionPhase::Connecting;
            app.composer.update(cx, |input, cx| input.set_text("Keep this draft", cx));
            app
        });
        for (width, height) in [(320., 568.), (375., 667.), (390., 844.)] {
            view.update(cx, |app, cx| { app.device = Some((width, height)); cx.notify(); });
            cx.simulate_resize(gpui::size(px(width), px(height)));
            cx.update(|window, cx| { window.draw(cx).clear(cx); });
            for selector in ["menu", "new-session", "send"] {
                let bounds = cx.debug_bounds(selector).expect("primary control rendered");
                assert!(bounds.size.width >= px(44.) && bounds.size.height >= px(44.), "{selector}: {bounds:?}");
                assert!(bounds.left() >= px(0.) && bounds.right() <= px(width), "{selector}: {bounds:?}");
            }
            let tab = cx.debug_bounds("tab-0").expect("tab rendered");
            assert!(tab.size.height >= px(44.));
            let send = cx.debug_bounds("send").unwrap();
            cx.simulate_click(send.center(), Modifiers::default());
            view.read_with(cx, |app, cx| {
                assert_eq!(app.composer.read(cx).text(), "Keep this draft");
                assert!(app.session.transcript.is_empty());
            });
        }
    }

    fn check_outside_tap_unfocuses_both_onboarding_fields(cx: &mut TestAppContext) {
        cx.update(init);
        for code_step in [false, true] {
            // A fresh window keeps this deterministic, independent of the
            // stress test's scroll momentum and pending multi-tap gestures.
            let (view, cx) = cx.add_window_view(|window, cx| {
                let mut app = MobileApp::new(SafeArea { top: 47., bottom: 34. }, window, cx);
                app.device = Some((390., 844.));
                app.login_token = code_step.then(|| "offline-test-token".to_string());
                app
            });
            cx.simulate_resize(gpui::size(px(390.), px(844.)));
            let input = view.read_with(cx, |app, _| {
                if code_step { app.code.clone() } else { app.email.clone() }
            });
            cx.update(|window, cx| {
                window.set_emulate_touch_from_mouse(true);
                window.focus(&input.focus_handle(cx), cx);
                window.draw(cx).clear(cx);
            });
            cx.simulate_click(point(px(10.), px(70.)), Modifiers::default());
            cx.update(|window, cx| assert!(!input.read(cx).is_focused(window), "code_step={code_step}"));
        }
    }

    #[gpui::test]
    fn random_gestures_on_every_screen_do_not_panic(cx: &mut TestAppContext) {
        check_outside_tap_unfocuses_both_onboarding_fields(cx);
        // Keep the test offline: no saved account, so nothing connects.
        cx.update(init);
        let (view, cx) = cx.add_window_view(|window, cx| {
            let mut app = MobileApp::new(SafeArea { top: 47., bottom: 34. }, window, cx);
            app.device = Some((390., 844.));
            app.synced = synced_sessions::State::fixture();
            app
        });
        cx.simulate_resize(gpui::size(px(390.), px(844.)));
        cx.update(|window, _| window.set_emulate_touch_from_mouse(true));
        cx.run_until_parked();
        let send = |input: PlatformInput, cx: &mut gpui::VisualTestContext| match input {
            PlatformInput::MouseDown(e) => cx.simulate_event(e),
            PlatformInput::MouseMove(e) => cx.simulate_event(e),
            PlatformInput::MouseUp(e) => cx.simulate_event(e),
            PlatformInput::ScrollWheel(e) => cx.simulate_event(e),
            _ => {}
        };
        let mut seed = 0x9E37_79B9_7F4A_7C15u64;
        for (screen, code_step) in [
            (Screen::SignIn, false),
            (Screen::SignIn, true),
            (Screen::Chat, false),
            (Screen::Settings, false),
            (Screen::Chat, false),
        ] {
            view.update(cx, |app, cx| {
                app.screen = screen;
                app.drawer_open = matches!(app.screen, Screen::Chat);
                app.drawer_t = if app.drawer_open { 1. } else { 0. };
                app.login_token = code_step.then(|| "offline-test-token".to_string());
                if code_step {
                    app.email.update(cx, |input, cx| {
                        input.set_text("a-long-email-address-for-narrow-phone-layout@example.com", cx)
                    });
                    app.focus_code = true;
                }
                cx.notify();
            });
            cx.run_until_parked();
            for _ in 0..150 {
                let p = point(px(rng(&mut seed) * 390.), px(rng(&mut seed) * 844.));
                let kind = rng(&mut seed);
                send(
                    PlatformInput::MouseDown(MouseDownEvent {
                        button: MouseButton::Left,
                        position: p,
                        modifiers: Modifiers::default(),
                        click_count: 1,
                        first_mouse: false,
                    }),
                    cx,
                );
                let mut end = p;
                if kind > 0.4 {
                    let (dx, dy) = ((rng(&mut seed) - 0.5) * 400., (rng(&mut seed) - 0.5) * 400.);
                    for s in 1..=6 {
                        end = point(p.x + px(dx * s as f32 / 6.), p.y + px(dy * s as f32 / 6.));
                        send(
                            PlatformInput::MouseMove(MouseMoveEvent {
                                position: end,
                                pressed_button: Some(MouseButton::Left),
                                modifiers: Modifiers::default(),
                            }),
                            cx,
                        );
                    }
                }
                send(
                    PlatformInput::MouseUp(MouseUpEvent {
                        button: MouseButton::Left,
                        position: end,
                        modifiers: Modifiers::default(),
                        click_count: 1,
                    }),
                    cx,
                );
                if kind > 0.85 {
                    send(
                        PlatformInput::ScrollWheel(ScrollWheelEvent {
                            position: end,
                            delta: ScrollDelta::Pixels(point(px(0.), px((rng(&mut seed) - 0.5) * 600.))),
                            ..Default::default()
                        }),
                        cx,
                    );
                }
            }
        }
    }
}
