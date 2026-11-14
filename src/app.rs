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
    Prefs, PrefsStore, Request, Role, SessionState, ToolStatus, TranscriptEntry, cloud,
};

use crate::text_input::{InputDraft, TextInput, TextInputEvent};
use crate::haptics::{self, Haptic};

#[path = "push_glue.rs"]
mod push_glue;
mod deep_link_glue;
mod a11y;
use a11y::A11yExt as _;
mod recent_dirs;
mod default_dir;
mod sidebar;
mod synced_sessions;
mod account_profile;
mod apple_sign_in;
mod ios_system;
mod ios_feel;
mod panel_preview;
mod session_map;
mod voice;
mod cloud_machine;
mod git_info;
mod transcript_layout;
mod stream_presentation;
mod stream_motion;
#[cfg(test)]
mod streaming_tests;
#[cfg(test)]
mod performance_tests;
#[cfg(test)]
mod interaction_fuzz;
#[cfg(test)]
mod offline_tests;
#[cfg(test)]
mod account_coverage;
#[cfg(test)]
mod composer_coverage;
#[cfg(test)]
mod navigation_coverage;
#[cfg(test)]
mod lifecycle_coverage;
#[cfg(test)]
mod journey_fuzz;
#[cfg(test)]
mod local_control_coverage;
#[cfg(test)]
mod keyboard_bridge_coverage;
#[cfg(test)]
mod preview_geometry_coverage;
#[cfg(test)]
mod status_coverage;
#[cfg(test)]
mod modal_ownership_coverage;
#[cfg(test)]
mod input_event_coverage;
#[cfg(test)]
mod coverage_inventory;
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

enum SignInOutcome {
    Sent(String),
    Authenticated(Account),
}

pub struct MobileApp {
    /// Test instances never read credentials, connect, or change persistent state.
    offline: bool,
    /// Soft keyboard currently covers the bottom of the screen.
    keyboard_up: bool,
    screen: Screen,
    /// When the active tab started connecting, until its transcript is painted.
    open_started: Option<std::time::Instant>,
    /// First frame after launch not yet logged.
    first_frame_pending: bool,
    safe: SafeArea,
    /// Fixed device size for the desktop preview; None on a real phone.
    pub device: Option<(f32, f32)>,
    store: AccountStore,
    account: Option<Account>,
    account_profile_requested: bool,
    connection: Option<Connection>,
    session: SessionState,
    // Sign-in form: email, then the 6-digit code.
    email: Entity<TextInput>,
    code: Entity<TextInput>,
    login_token: Option<String>,
    /// Move focus to the code field on the next frame (after the code is mailed).
    focus_code: bool,
    signing_in: bool,
    auth_generation: u64,
    blur_on_render: bool,
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
    git_info: git_info::GitState,
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
    todos_expanded: bool,
    scroll_perf_started: Option<std::time::Instant>,
    expanded_tools: std::collections::HashSet<String>,
    follow: bool,
    stream_presentation: stream_presentation::Presentation,
    stream_motion: stream_motion::StreamMotion,
    stream_identity: Option<(u64, Option<String>)>,
    #[cfg(test)]
    animate_stream_in_tests: bool,
    #[cfg(test)]
    stream_test_now: Option<std::time::Instant>,
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
    dir_input: Entity<TextInput>,
    /// Persisted user preferences, including the default new-session directory.
    prefs: Prefs,
    /// Settings field editing `prefs.default_dir`.
    default_dir_input: Entity<TextInput>,
    /// Horizontal swipe between tabs, like the desktop workspace pull.
    swipe: crate::swipe::Swipe,
    spinner_step: usize,
    /// Armed only while a visible text spinner needs another tick.
    spinner_tick: Option<gpui::Task<()>>,
    transcript_heights: transcript_layout::Heights,
    #[cfg(test)]
    disable_transcript_windowing: bool,
    #[cfg(test)]
    disable_transcript_coalescing: bool,
    #[cfg(test)]
    rendered_transcript_rows: usize,
    /// Desktop's thinking orbs: transcript activity, and connection/sign-in status.
    activity_orb: Entity<gpui_thinking_orbs::ThinkingOrb>,
    status_orb: Entity<gpui_thinking_orbs::ThinkingOrb>,
    /// Dev hook (JCODE_MOBILE_SEND): message sent once connected.
    auto_send: Option<String>,
    /// APNs device token (hex) once iOS hands it over.
    push_token: Option<String>,
    /// Account-wide agent status from the Worker (all hosts, all sessions).
    agents_status: Option<jcode_mobile_core::agents::AgentsState>,
    /// Live Activity push tokens registered for this account.
    live_tokens: Vec<String>,
    /// Session drawer (see `sidebar.rs`): target, eased progress, clock,
    /// and the (top, bottom) safe insets it pads by.
    drawer_open: bool,
    drawer_t: f32,
    drawer_last: std::time::Instant,
    drawer_insets: (f32, f32),
    /// Drawer drag-to-close travel (negative px) and last drag event.
    drawer_drag: f32,
    drawer_drag_last: Option<std::time::Instant>,
    /// Drawer touch phases seen, and whether a finger is on the drawer now.
    drawer_phased: bool,
    drawer_finger: bool,
    /// Start-screen suggestions list, which scrolls inside the pull area.
    suggestions_scroll: ScrollHandle,
    suggestions_shown: bool,
    /// Drawer session search field.
    drawer_search: Entity<TextInput>,
    drawer_directory: sidebar::DirectorySelection,
    synced: synced_sessions::State,
    /// Pinch-out map of every live session (session_map.rs).
    map: session_map::State,
    /// Viewport height the map lays out against (set each frame).
    map_height: f32,
    transfer_input: Entity<TextInput>,
    /// Settings "Link existing Jcode account" panel (apple_sign_in.rs).
    apple_link: Option<Entity<apple_sign_in::AppleLinkPanel>>,
}

impl MobileApp {
    pub fn new(safe: SafeArea, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let offline = cfg!(test) || std::env::var("JCODE_MOBILE_DEMO").is_ok();
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

        let prefs = if offline { Prefs::default() } else { PrefsStore::default_location().load() };
        let default_dir_input = cx.new(|cx| {
            let mut t = TextInput::new("Cloud home (server default)", cx);
            t.bare = true;
            t.config = TextInputConfiguration {
                input_action: TextInputAction::Done,
                purpose: TextInputPurpose::Text,
                ..Default::default()
            };
            if let Some(dir) = &prefs.default_dir {
                t.set_text(dir.clone(), cx);
            }
            t
        });
        cx.subscribe(&default_dir_input, |this, input, ev, cx| match ev {
            TextInputEvent::Submit => {
                let dir = input.read(cx).text().to_string();
                this.set_default_dir(&dir, cx);
            }
            TextInputEvent::Changed => cx.notify(),
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
            account_profile_requested: false,
            connection: None,
            session: SessionState::default(),
            email,
            code,
            login_token: None,
            focus_code: false,
            signing_in: false,
            auth_generation: 0,
            blur_on_render: false,
            // Dev hook: JCODE_MOBILE_ERROR=text shows a sign-in error banner.
            sign_in_error: std::env::var("JCODE_MOBILE_ERROR").ok(),
            delete_step: 0,
            delete_error: None,
            browser_login: None,
            started: std::time::Instant::now(),
            keyboard_up: false,
            open_started: None,
            first_frame_pending: true,
            sim_kb: Default::default(),
            inset_anim: None,
            inset_last: std::time::Instant::now(),
            kb_lift: 0.,
            model_picker: false,
            kb_config: Default::default(),
            composer,
            voice: voice::VoiceState::default(),
            git_info: git_info::GitState::default(),
            cloud_machine: cloud_machine::MachineState::default(),
            active_tab: 0,
            next_tab: 1,
            parked: Vec::new(),
            tab_order: vec![0],
            tab_motion: Default::default(),
            tab_track_width: 0.0,
            last_frame: std::time::Instant::now(),
            transcript_scroll: ScrollHandle::new(),
            todos_expanded: false,
            scroll_perf_started: None,
            expanded_tools: Default::default(),
            follow: true,
            stream_presentation: Default::default(),
            stream_motion: Default::default(),
            stream_identity: None,
            #[cfg(test)]
            animate_stream_in_tests: false,
            #[cfg(test)]
            stream_test_now: None,
            tab_hold: None,
            hold_fired: false,
            tab_menu: None,
            feel: Default::default(),
            recent_dirs_asked: None,
            dir_picker: false,
            dir_input,
            prefs,
            default_dir_input,
            swipe: Default::default(),
            spinner_step: 0,
            spinner_tick: None,
            transcript_heights: Default::default(),
            #[cfg(test)]
            disable_transcript_windowing: false,
            #[cfg(test)]
            disable_transcript_coalescing: false,
            #[cfg(test)]
            rendered_transcript_rows: 0,
            activity_orb: cx.new(|_| inline_orb()),
            status_orb: cx.new(|_| inline_orb()),
            auto_send: (!offline).then(|| std::env::var("JCODE_MOBILE_SEND").ok()).flatten(),
            push_token: None,
            agents_status: None,
            live_tokens: Vec::new(),
            // Dev hook (JCODE_MOBILE_DRAWER=1): start with the session drawer open.
            drawer_open: std::env::var_os("JCODE_MOBILE_DRAWER").is_some(),
            drawer_t: 0.,
            drawer_last: std::time::Instant::now(),
            drawer_insets: (0., 0.),
            drawer_drag: 0.,
            drawer_drag_last: None,
            drawer_phased: false,
            drawer_finger: false,
            suggestions_scroll: ScrollHandle::new(),
            suggestions_shown: false,
            drawer_search,
            drawer_directory: Default::default(),
            synced: Default::default(),
            map: Default::default(),
            map_height: 844.,
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
            // Dev hook: JCODE_MOBILE_MAP=1 starts zoomed out, tabs spread over strips.
            if std::env::var_os("JCODE_MOBILE_MAP").is_some() {
                for (i, id) in app.tab_order.clone().into_iter().enumerate() {
                    app.map.set_row(id, i % 3);
                }
                app.map.set_open(true);
                app.map.progress = 1.;
            }
        }
        let _ = window;
        app
    }

    /// Desktop dev hook (JCODE_MOBILE_DEMO=todos): an offline chat showing the
    /// pinned todo list and a scrollable transcript, for screenshots.
    pub fn seed_todo_demo(&mut self, expanded: bool) {
        self.todos_expanded = expanded;
        use jcode_mobile_core::ToolCall;
        self.screen = Screen::Chat;
        self.session.phase = ConnectionPhase::Connected;
        let todo = |id: &str, input: &str| ToolCall {
            id: id.into(), name: "todo".into(), input: input.into(), output: String::new(), status: ToolStatus::Succeeded,
        };
        let mut id = 0;
        let mut push = |role, text: &str, calls: Vec<ToolCall>| {
            id += 1;
            self.session.transcript.push(TranscriptEntry { id, role, text: text.into(), reasoning: String::new(), tool_calls: calls, is_streaming: false, is_queued: false });
        };
        push(Role::User, "Add a scrollbar and a pinned todo list to the mobile app.", vec![]);
        push(Role::Assistant, "Planning the work.", vec![todo("t1", r#"{"plan":{"user_intention":"Bring the TUI pinned todo card to mobile"},"phases":[
            {"phase":1,"lanes":[{"lane":"core","items":[{"name":"Read TUI todo card renderer","status":"pending"},{"name":"Use server todo_state","status":"pending"}]}]},
            {"phase":2,"lanes":[{"lane":"ui","owner":"worker","items":[{"name":"Phase and lane sections","status":"pending"},{"name":"Blocker glyphs","status":"pending"}]},{"lane":"git","items":[{"name":"Right-edge pull tab","status":"pending"}]}]},
            {"phase":3,"lanes":[{"lane":"verify","items":[{"name":"Screenshot preview","status":"pending"},{"name":"Ship TestFlight build","status":"pending"}]}]}]}"#)]);
        for _ in 0..6 {
            push(Role::Assistant, "Reading the TUI's pinned todo card: phase pips in the header, purple lane headers with per-lane dot strips, and blocker glyphs.", vec![]);
        }
        push(Role::Assistant, "Core done, UI in progress.", vec![todo("t2", r#"{"update":[{"name":"Read TUI todo card renderer","status":"completed"},{"name":"Use server todo_state","status":"completed"},{"name":"Phase and lane sections","status":"in_progress"},{"name":"Right-edge pull tab","status":"completed"},{"name":"Blocker glyphs","blocked_by":["Phase and lane sections"]},{"name":"Ship TestFlight build","completely_blocked":{"reason":"needs App Store Connect login"}}]}"#)]);
    }

    #[cfg(test)]
    fn new_offline(safe: SafeArea, window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::new(safe, window, cx)
    }

    // MARK: actions

    /// Cancel pending authentication before navigation, logout, or a new flow.
    fn invalidate_auth(&mut self) {
        self.auth_generation = self.auth_generation.wrapping_add(1);
        self.signing_in = false;
        if let Some(cancel) = self.browser_login.take() {
            cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            crate::web_auth::close();
        }
    }

    fn begin_auth(&mut self) -> u64 {
        self.invalidate_auth();
        self.signing_in = true;
        self.sign_in_error = None;
        self.auth_generation
    }

    fn complete_sign_in(&mut self, generation: u64, result: Result<SignInOutcome, String>, cx: &mut Context<Self>) {
        if generation != self.auth_generation || !self.signing_in { return; }
        self.signing_in = false;
        match result {
            Ok(SignInOutcome::Sent(token)) => {
                haptics::play(Haptic::Success);
                self.login_token = Some(token);
                self.focus_code = true;
            }
            Ok(SignInOutcome::Authenticated(account)) => {
                if !self.offline {
                    if let Err(e) = self.store.save(&account) {
                        self.sign_in_error = Some(format!("Could not save sign-in: {e}"));
                    }
                }
                self.login_token = None;
                self.code.update(cx, |input, cx| input.set_text("", cx));
                self.adopt_account(account, cx);
            }
            Err(error) if error.is_empty() => {}
            Err(error) => {
                haptics::play(Haptic::Error);
                self.sign_in_error = Some(error);
            }
        }
        cx.notify();
    }

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
        let generation = self.begin_auth();
        cx.notify();
        let task = cx.background_spawn(async move {
            match token {
                None => cloud::start_login(&email).map(SignInOutcome::Sent),
                Some(t) => cloud::verify_login(&t, &code).map(SignInOutcome::Authenticated),
            }
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| this.complete_sign_in(generation, result, cx))
            .ok();
        })
        .detach();
    }

    fn browser_request_current(&self, cancel: &std::sync::Arc<std::sync::atomic::AtomicBool>) -> bool {
        !cancel.load(std::sync::atomic::Ordering::Relaxed)
            && self.browser_login.as_ref().is_some_and(|active| std::sync::Arc::ptr_eq(active, cancel))
    }

    fn complete_browser_sign_in(&mut self, cancel: &std::sync::Arc<std::sync::atomic::AtomicBool>, result: Result<Account, String>, cx: &mut Context<Self>) {
        if !self.browser_request_current(cancel) { return; }
        self.browser_login = None;
        crate::web_auth::close();
        match result {
            Ok(account) => {
                if !self.offline {
                    if let Err(error) = self.store.save(&account) {
                        self.sign_in_error = Some(format!("Could not save sign-in: {error}"));
                    }
                }
                self.adopt_account(account, cx);
            }
            Err(error) if error.is_empty() => {}
            Err(error) => {
                haptics::play(Haptic::Error);
                self.sign_in_error = Some(error);
            }
        }
        cx.notify();
    }

    /// "Continue with Google": shows Google's account chooser in the in-app
    /// sign-in sheet (system browser where there is none) and polls the device
    /// flow until it is approved, then dismisses the sheet.
    fn google_sign_in(&mut self, cx: &mut Context<Self>) {
        if let Some(c) = self.browser_login.take() {
            c.store(true, std::sync::atomic::Ordering::Relaxed);
            crate::web_auth::close();
            haptics::play(Haptic::Light);
            cx.notify();
            return;
        }
        if self.offline || self.signing_in { return; }
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
                    this.update(cx, |this, cx| this.complete_browser_sign_in(&cancel, Err(e), cx))
                    .ok();
                    return;
                }
            };
            let url = cloud::google_login_url(&start);
            let dismissed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let opened = this.update(cx, |this, cx| {
                if !this.browser_request_current(&cancel) { return false; }
                if !crate::web_auth::open(&url, dismissed.clone()) {
                    cx.open_url(&url);
                }
                true
            }).unwrap_or(false);
            if !opened { return; }
            let flag = cancel.clone();
            let result = cx
                .background_spawn(async move {
                    cloud::finish_browser_login(
                        &start,
                        || flag.load(std::sync::atomic::Ordering::Relaxed),
                        || dismissed.load(std::sync::atomic::Ordering::Relaxed),
                    )
                })
                .await;
            this.update(cx, |this, cx| this.complete_browser_sign_in(&cancel, result, cx))
            .ok();
        })
        .detach();
    }

    fn connect(&mut self, account: Account, cx: &mut Context<Self>) {
        if self.offline { return; }
        self.adopt_account(account, cx);
    }

    /// Apply a completed authentication result, with effects guarded at their boundary.
    fn adopt_account(&mut self, account: Account, cx: &mut Context<Self>) {
        self.invalidate_auth();
        self.focus_code = false;
        self.blur_on_render = true;
        self.login_token = None;
        self.code.update(cx, |input, cx| input.set_text("", cx));
        haptics::play(Haptic::Success);
        self.git_info = git_info::GitState::default();
        self.cloud_machine = cloud_machine::MachineState::default();
        if let Some(c) = self.connection.take() {
            c.stop();
        }
        self.session = SessionState::default();
        self.account = Some(account);
        self.account_profile_requested = false;
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
        self.cancel_voice(cx);
        if self.offline { return; }
        self.ensure_account_profile(cx);
        let Some(account) = self.account.clone() else { return };
        let working_dir = self.new_session_dir(resume.as_deref(), working_dir, fork_from.as_deref());
        if working_dir.is_some() && self.session.working_dir.is_none() {
            // Show the location label before the server confirms it.
            self.session.working_dir = working_dir.clone();
        }
        let mut config = Config::cloud(account);
        config.working_dir = working_dir;
        config.fork_from = fork_from;
        config.max_reconnect_attempts = Some(6);
        let (tx, rx) = unbounded();
        let conn = Connection::start(config, resume, move |o| {
            let _ = tx.unbounded_send(o);
        });
        self.connection = Some(conn);
        self.open_started = Some(std::time::Instant::now());
        self.stream_motion.reset();
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
        // New sessions join the strip of the session they were opened from.
        self.map.set_row(id, self.map.row(self.active_tab));
        self.park_active(cx);
        self.active_tab = id;
        self.session = SessionState::default();
        self.start_connection(None, cx);
        cx.notify();
    }

    fn park_active(&mut self, cx: &mut Context<Self>) {
        self.scroll_perf_started = None;
        self.cancel_voice(cx);
        let draft = self.composer.read(cx).save_draft();
        let preview_composer = cx.new(|cx| {
            let mut input = TextInput::new("Message jcode…", cx);
            input.bare = true;
            input.max_lines = 6;
            input.read_only = true;
            input.restore_draft(draft.clone(), cx);
            input
        });
        self.parked.push(ParkedTab {
            id: self.active_tab,
            connection: self.connection.take(),
            session: std::mem::take(&mut self.session),
            scroll: std::mem::replace(&mut self.transcript_scroll, ScrollHandle::new()),
            expanded_tools: std::mem::take(&mut self.expanded_tools),
            follow: self.follow,
            draft,
            preview_composer,
            preview_activity_orb: cx.new(|_| inline_orb()),
            preview_status_orb: cx.new(|_| inline_orb()),
            auto_send: self.auto_send.take(),
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
        self.auto_send = p.auto_send;
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
            self.cancel_voice(cx);
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
            self.auto_send = p.auto_send;
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
            .occlude()
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
        let opened = std::time::Instant::now();
        let mut history_logged = false;
        cx.spawn(async move |this, cx| {
            while let Some(first) = rx.next().await {
                // Coalesce bursts of streaming deltas into one frame.
                let mut batch = vec![first];
                while let Ok(o) = rx.try_recv() {
                    batch.push(o);
                }
                let has_history = !history_logged
                    && batch.iter().any(|o| {
                        matches!(o, ConnectionOutput::Event(jcode_mobile_core::ApiEvent::History { .. }))
                    });
                let apply_start = std::time::Instant::now();
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
                        this.flush_pending_send(cx);
                        // Presentation releases visible blocks in render_chat; raw data is already current.
                        cx.notify();
                    })
                    .is_ok();
                if has_history {
                    history_logged = true;
                    // UI side of open latency: main-thread apply cost and
                    // total time from tab open to transcript in state.
                    jcode_mobile_core::timing::stage("ui_history_apply", apply_start.elapsed());
                    jcode_mobile_core::timing::stage("ui_open_to_history", opened.elapsed());
                }
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

    /// External prompts belong to their tab and must not overwrite work typed
    /// while that tab was connecting. Preserve selection and scroll as well.
    fn flush_pending_send(&mut self, cx: &mut Context<Self>) {
        if self.session.phase != ConnectionPhase::Connected || self.session.session_id.is_none() { return; }
        let Some(text) = self.auto_send.take() else { return; };
        let draft = self.composer.read(cx).save_draft();
        self.composer.update(cx, |input, cx| input.set_text(text, cx));
        self.send_draft(cx);
        self.composer.update(cx, |input, cx| input.restore_draft(draft, cx));
    }

    fn send_draft(&mut self, cx: &mut Context<Self>) {
        let text = self.composer.read(cx).text().trim().to_string();
        if text.is_empty() || self.session.phase != ConnectionPhase::Connected || self.session.session_id.is_none() {
            return;
        }
        static ASKED: std::sync::Once = std::sync::Once::new();
        self.cancel_voice(cx);
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
        self.stream_motion.reset();
        self.follow = true;
        // A new outgoing prompt explicitly resumes native tail following.
        cx.notify();
    }

    fn cancel(&mut self, cx: &mut Context<Self>) {
        if !self.session.is_processing { return; }
        haptics::play(Haptic::Medium);
        self.send(|id| Request::Cancel { id });
        cx.notify();
    }

    fn complete_account_deletion(&mut self, expected: &Account, generation: u64, result: Result<(), String>, cx: &mut Context<Self>) {
        if generation != self.auth_generation || !account_profile::same_account(self.account.as_ref(), expected) || self.delete_step != 2 { return; }
        self.delete_step = 0;
        match result {
            Ok(()) => self.sign_out(cx),
            Err(error) => self.delete_error = Some(error),
        }
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
                let expected = account.clone();
                let generation = self.auth_generation;
                let task = cx.background_spawn(async move { cloud::delete_account(&account) });
                cx.spawn(async move |this, cx| {
                    let result = task.await;
                    this.update(cx, |this, cx| {
                        if result.is_err() && generation == this.auth_generation && account_profile::same_account(this.account.as_ref(), &expected) && this.delete_step == 2 {
                            haptics::play(Haptic::Error);
                        }
                        this.complete_account_deletion(&expected, generation, result, cx);
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
        self.invalidate_auth();
        self.auto_send = None;
        self.blur_on_render = true;
        self.focus_code = false;
        self.tab_hold = None;
        self.hold_fired = false;
        self.tab_menu = None;
        self.model_picker = false;
        self.dir_picker = false;
        self.drawer_open = false;
        self.drawer_t = 0.;
        self.drawer_drag = 0.;
        self.drawer_drag_last = None;
        self.drawer_finger = false;
        self.drawer_directory = Default::default();
        self.feel = Default::default();
        self.swipe = Default::default();
        self.expanded_tools.clear();
        self.recent_dirs_asked = None;
        self.delete_step = 0;
        self.delete_error = None;
        self.sign_in_error = None;
        self.code.update(cx, |input, cx| input.set_text("", cx));
        self.drawer_search.update(cx, |input, cx| input.set_text("", cx));
        self.dir_input.update(cx, |input, cx| input.set_text("", cx));
        if self.account.is_some() { haptics::play(Haptic::Rigid); }
        self.git_info = git_info::GitState::default();
        self.cloud_machine = cloud_machine::MachineState::default();
        self.cancel_voice(cx);
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
        self.account_profile_requested = false;
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
    /// Separate, unfocused viewer so a swipe never mounts the active editor twice.
    preview_composer: Entity<TextInput>,
    preview_activity_orb: Entity<gpui_thinking_orbs::ThinkingOrb>,
    preview_status_orb: Entity<gpui_thinking_orbs::ThinkingOrb>,
    auto_send: Option<String>,
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
        .debug_selector(move || id.into())
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
    ThinkingOrb::new().size(OrbSize::Inline)
        .theme(if crate::system::is_light() { OrbTheme::Light } else { OrbTheme::Dark })
        .reduced_motion(crate::system::reduce_motion())
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
    /// Only the browser button and live tool rows use this textual spinner.
    /// Orbs own their animation clocks. Idle/disconnected screens need no poll.
    fn needs_spinner_tick(&self) -> bool {
        match self.screen {
            Screen::SignIn => self.login_token.is_none() && self.browser_login.is_some(),
            Screen::Settings => false,
            Screen::Chat => self.session.transcript.iter().any(|entry| entry.tool_calls.iter().any(|call| {
                matches!(call.status, ToolStatus::StreamingInput | ToolStatus::Running)
            })),
        }
    }

    fn sync_spinner_tick(&mut self, cx: &mut Context<Self>) {
        if !self.needs_spinner_tick() || crate::system::reduce_motion() {
            self.spinner_tick = None;
        } else if self.spinner_tick.is_none() {
            self.spinner_tick = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(Duration::from_millis(125)).await;
                let _ = this.update(cx, |this, cx| {
                    this.spinner_tick = None;
                    if this.needs_spinner_tick() && !crate::system::reduce_motion() {
                        this.spinner_step = (this.spinner_step + 1) % 8;
                        cx.notify();
                    }
                });
            }));
        }
    }

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
                                    format!("{} Waiting for Google… (tap to cancel)", spinner_glyph(self.spinner_step))
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
                    .child(div().id(if code_step { "code-input" } else { "email-input" }).debug_selector(move || if code_step { "code-input".into() } else { "email-input".into() }).w_full().child(if code_step { self.code.clone() } else { self.email.clone() }))
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
                                .debug_selector(|| "use-other-email".into())
                                .w_full()
                                .flex()
                                .justify_center()
                                .cursor_pointer()
                                .child(label("Use a different email", t.text_muted, 12.))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.invalidate_auth();
                                    this.login_token = None;
                                    this.code.update(cx, |input, cx| input.set_text("", cx));
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
        self.end_swipe_with(true, cx);
    }

    fn end_swipe_with(&mut self, allow_new: bool, cx: &mut Context<Self>) {
        let width = self.content_width();
        let (prev, next) = self.tab_neighbors();
        let now = std::time::Instant::now();
        let offer_new = allow_new && self.account.is_some() && self.swipe.new_session_preview(next.is_some()) == Some(1.);
        let mut navigated = offer_new;
        if let crate::swipe::Release::Switch(dir) = self.swipe.release(width, prev.is_some(), next.is_some(), now) {
            if let Some(id) = if dir < 0 { prev } else { next } {
                self.focus_tab(id, cx);
                navigated = true;
            }
        }
        if offer_new {
            self.swipe.restart_spent();
            self.new_tab(cx);
        } else if navigated {
            self.swipe.mark_spent();
        }
        cx.notify();
    }

    /// Window-level end of a touch. The panel's own listener only hears events
    /// over its hitbox, so a lift elsewhere would leave the drag held forever.
    pub(crate) fn finish_panel_touch(&mut self, ev: &gpui::ScrollWheelEvent, cx: &mut Context<Self>) {
        let ending = matches!(ev.touch_phase, gpui::TouchPhase::Ended | gpui::TouchPhase::Cancelled);
        if ending && self.swipe_engaged() {
            self.panel_wheel(ev, cx);
        }
    }

    /// A lift reported without a scroll end still ends the drag.
    pub(crate) fn finish_panel_lift(&mut self, cx: &mut Context<Self>) {
        if self.swipe.finger_down() {
            self.swipe.touch_phase(gpui::TouchPhase::Ended);
            if self.swipe.active {
                self.end_swipe(cx);
            }
        }
    }

    fn swipe_engaged(&self) -> bool {
        self.swipe.active || self.swipe.finger_down()
    }

    /// Both the active viewport and the inert neighbor shield own the same
    /// drag. Crossing their boundary must not lose movement or cancellation.
    fn panel_wheel(&mut self, ev: &gpui::ScrollWheelEvent, cx: &mut Context<Self>) {
        let _perf = crate::perf::span("panel.scroll_input");
        let coasting = self.swipe.touch_phase(ev.touch_phase);
        if ev.touch_phase == gpui::TouchPhase::Cancelled {
            self.swipe.cancel(self.content_width(), std::time::Instant::now());
            cx.notify();
            return;
        }
        // One swipe moves one tab: the fling after a lift that already
        // navigated must not carry on into the next tab or a new session.
        if coasting && self.swipe.spent() {
            return;
        }
        let d = ev.delta.pixel_delta(px(20.));
        if self.swipe.feed(f32::from(d.x), f32::from(d.y), std::time::Instant::now()) {
            cx.notify();
        }
        if ev.touch_phase == gpui::TouchPhase::Ended {
            self.end_swipe_with(!coasting, cx);
        }
    }

    /// Chat panel that slides with a horizontal swipe and settles into the neighbor tab.
    fn render_chat(&mut self, t: &Theme, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let width = self.content_width();
        let now = std::time::Instant::now();
        // Trackpads without touch phases: a pause ends the gesture.
        // A finger still on the glass decides by lifting, not by pausing.
        if self.swipe.active && !self.swipe.finger_down()
            && self.swipe.last_event.is_some_and(|e| now.duration_since(e) > crate::swipe::IDLE_END) {
            self.end_swipe(cx);
        }
        let (prev, next) = self.tab_neighbors();
        let (offset, settling) = self.swipe.panel_offset(width, prev.is_some(), next.is_some(), now);
        let neighbor = if offset < 0. { next } else if offset > 0. { prev } else { None };
        let preview = neighbor.and_then(|id| self.render_panel_preview(id, t, window, cx));
        if settling || self.swipe.active {
            window.request_animation_frame();
        }
        let inner = self.render_chat_inner(t, window, cx);
        let new_session_progress = self.account.as_ref().and_then(|_| self.swipe.new_session_preview(next.is_some()));
        let panels = div()
            .id("chat-swipe")
            .relative()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .overflow_hidden()
            .on_scroll_wheel(cx.listener(|this, ev: &gpui::ScrollWheelEvent, _window, cx| {
                // The window-level handler already finished this touch.
                let ending = matches!(ev.touch_phase, gpui::TouchPhase::Ended | gpui::TouchPhase::Cancelled);
                if ending && !this.swipe_engaged() {
                    return;
                }
                this.panel_wheel(ev, cx);
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
                    .child(preview))
            })
            .when_some(new_session_progress, |d, progress| {
                d.child(label(if progress >= 1. { "Release to create a new session" } else { "Keep swiping to create a new session →" }, t.accent, 12.)
                    .absolute().top_0().left_0().right_0()
                    .px(px(16.)).py(px(8.)).bg(t.background))
            })
            .into_any_element();
        div().flex_1().min_h_0().flex().flex_col().relative()
            .child(self.render_header(t, cx))
            .child(panels)
            .child(self.render_git_handle(t, cx))
            .when(self.git_info.open, |d| d.child(self.render_git_panel(t, cx)))
            .into_any_element()
    }

    fn render_chat_inner(&mut self, t: &Theme, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let _perf = crate::perf::span("transcript.build");
        let identity = (self.active_tab, self.session.session_id.clone());
        if self.stream_identity.as_ref() != Some(&identity) {
            self.stream_motion.reset();
            self.stream_identity = Some(identity);
        }
        #[cfg(test)]
        let now = self.stream_test_now.unwrap_or_else(std::time::Instant::now);
        #[cfg(not(test))]
        let now = std::time::Instant::now();
        #[cfg(test)]
        let instant = crate::system::reduce_motion() || !self.animate_stream_in_tests;
        #[cfg(not(test))]
        let instant = crate::system::reduce_motion();
        #[cfg(test)]
        self.stream_motion.set_now_for_test(now);
        let presentation = self.stream_presentation.prepare(self.active_tab, &self.session, now, instant);
        if presentation.animating { window.request_animation_frame(); }
        // Native layout remains authoritative. Motion affects painting only.
        // Render only knows the previous layout's maximum. Easing toward that
        // stale maximum made each streamed chunk (and thinking-row transition)
        // drift behind, then catch up on later orb/animation frames. GPUI applies
        // this request after measuring the new content, before painting it.
        let max = self.transcript_scroll.max_offset().y;
        let off = -self.transcript_scroll.offset().y;
        if self.follow {
            self.transcript_scroll.scroll_to_bottom();
        }
        let at_bottom = self.follow || max - off < px(56.);
        let entries = &self.session.transcript;
        let mut prompt_n = 0;
        let mut rows = transcript_layout::Rows::default();
        #[cfg(test)]
        { rows.uncoalesced = self.disable_transcript_coalescing; }
        let viewport = self.transcript_scroll.bounds().size.height;
        // An anchored layout can move far beyond the previous offset (history
        // attach, jump-to-latest, or a large streamed append). Cull from the
        // bottom instead, using known suffix heights as a conservative lower
        // bound. Unknown/new rows count zero, never hiding a visible tail row.
        let mut geometry = Vec::with_capacity(entries.len());
        for e in entries {
            if e.role == Role::User { prompt_n += 1; }
            let visible = presentation.rows.get(&e.id);
            let text = visible.map_or(e.text.as_str(), |row| &e.text[..row.text_len]);
            let reasoning = visible.map_or(e.reasoning.as_str(), |row| &e.reasoning[..row.reasoning_len]);
            let key = if visible.is_some() {
                transcript_layout::key_visible(self.active_tab, e, prompt_n, &self.expanded_tools, self.content_width(), text, reasoning)
            } else {
                transcript_layout::key(self.active_tab, e, prompt_n, &self.expanded_tools, self.content_width())
            };
            geometry.push((key, self.transcript_heights.get(key), px(0.)));
        }
        let mut suffix = px(0.);
        for (_, height, after) in geometry.iter_mut().rev() {
            *after = suffix;
            suffix += height.unwrap_or(px(0.)) + px(14.);
        }
        prompt_n = 0;
        let mut top = Some(px(12.));
        #[cfg(test)]
        { self.rendered_transcript_rows = 0; }
        for (e, (key, height, after)) in entries.iter().zip(geometry) {
            if e.role == Role::User {
                prompt_n += 1;
            }
            let skip = height.is_some_and(|height| {
                if self.follow {
                    transcript_layout::before_tail(after, window.viewport_size().height + (window.viewport_size().height * 0.6).max(px(240.)))
                } else {
                    transcript_layout::offscreen(top, height, off, viewport)
                }
            });
            #[cfg(test)]
            let skip = skip && !self.disable_transcript_windowing;
            if skip {
                rows.hide(height.unwrap());
            } else {
                let row = div().id(("entry", e.id))
                    .debug_selector({ let id = e.id; move || format!("entry-{id}") });
                #[cfg(test)]
                { self.rendered_transcript_rows += 1; }
                let el = self.render_entry(e, prompt_n, &self.expanded_tools, presentation.rows.get(&e.id), t, cx);
                let heights = self.transcript_heights.clone();
                rows.push(row.relative()
                    .child(self.feel_hit(crate::gestures::Target::Message(e.id), el))
                    .child(gpui::canvas(move |bounds, _, _| heights.record(key, bounds.size.height), |_, _, _, _| {}).absolute().inset_0())
                    .into_any_element());
            }
            top = top.zip(height).map(|(top, height)| top + height + px(14.));
        }
        if let Some(a) = self.activity() {
            self.activity_orb.update(cx, |o, cx| o.set_state(orb_state(a), cx));
            rows.push(stream_motion::steady_activity(
                orb_row(&self.activity_orb, a, t.text_muted)
                    .id("transcript-activity").debug_selector(|| "transcript-activity".into()),
                self.stream_motion.clone(), self.follow,
            ).into_any_element());
        }
        if let Some(err) = self.session.error_banner.clone() {
            rows.push(
                banner(&err, t.error, t)
                    .id("err").debug_selector(|| "err".into())
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
            self.ensure_computer_stats(cx);
        }
        let recent = if empty { self.render_recent_dirs(t, cx) } else { None };
        self.suggestions_shown = recent.is_some();
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
                    d.child(banner(&e, t.error, t).id("err-empty").debug_selector(|| "err-empty".into()).on_click(cx.listener(|this, _, _, cx| {
                        this.session.apply_intent(LocalIntent::DismissError);
                        this.reconnect(cx);
                    })))
                })
                .child(label("What would you like to work on?", t.text, 20.).line_height(crate::system::fs(28.)))
                .children(self.render_computer_stats(t))
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
                            let _perf = crate::perf::span("transcript.scroll_input");
                            // Keep the oldest event if several inputs land before
                            // the next draw, so queuing latency remains visible.
                            if this.scroll_perf_started.is_none() {
                                this.scroll_perf_started = crate::perf::start();
                            }
                            let max = this.transcript_scroll.max_offset().y;
                            let off = -this.transcript_scroll.offset().y;
                            let delta = ev.delta.pixel_delta(window.line_height());
                            // GPUI applies scrolling separately. Use direction to
                            // release immediately, regardless of listener order
                            // or how far the streaming animation trails the end.
                            if delta.y.abs() >= delta.x.abs() {
                                if this.follow && delta.y > px(0.) { this.stream_motion.release(&this.transcript_scroll); }
                                this.follow = crate::scroll_follow::follow_after_scroll(
                                    this.follow, f32::from(delta.y), f32::from(max - off),
                                );
                            }
                            cx.notify();
                        }))
                        .child(stream_motion::stream_motion(div().w_full().min_w_0().flex().flex_col().gap(px(14.)).px(px(14.)).py(px(12.)).children(rows.finish()), self.transcript_scroll.clone(), self.stream_motion.clone(), self.follow, instant))
                        .child({
                            let scroll_started = self.scroll_perf_started.take();
                            gpui::canvas(|_, _, _| (), move |_, _, _, _| {
                                if let Some(started) = scroll_started {
                                    crate::perf::record("transcript.scroll_to_paint", started.elapsed());
                                }
                            }).absolute().inset_0()
                        }),
                )
                .when_some(transcript_scrollbar(viewport, max, off, t), |d, bar| d.child(bar))
                .when(!at_bottom && max > px(0.), |d| {
                    d.child(
                        div()
                            .id("jump").debug_selector(|| "jump".into()).a11y_button("Jump to latest")
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
                                this.stream_motion.reset();
                                this.follow = true;
                                // The next layout anchors to the current tail.
                                cx.notify()
                            })),
                    )
                })
                .into_any_element()
        };
        let body = self.feel_transcript(body, t);
        let content = div()
            .flex()
            .flex_col()
            .size_full()
            .when(!empty && !self.feel_reconnect_visible() && !matches!(self.session.phase, ConnectionPhase::Connected | ConnectionPhase::Failed { .. }), |d| {
                d.child(div().px(px(14.)).py(px(6.)).border_b_1().border_color(t.border)
                    .child(self.connection_status(t)))
            })
            .when(!empty, |d| d.children(self.pinned_todos(t, cx)))
            .child(body)
            .child(self.render_composer(t, window, cx))
            .into_any_element();
        self.feel_voice_surface(content, t, cx)
    }

    /// Pinned todo board, the mobile form of the TUI's pinned todo card.
    /// Source is the server's todo state (phases, lanes, owners, blockers,
    /// plan, goals); older servers fall back to replaying todo tool calls.
    ///
    /// Collapsed: plan intention, `phase 1 ●●● phase 2 ●○` pip header, the
    /// active item and any ✋ blockers. Expanded: every phase as a section,
    /// lanes as purple headers with owner chips and per-lane pips.
    fn pinned_todos(&self, t: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        use jcode_mobile_core::todos::{self, Mark};
        let fallback;
        let snapshot = match &self.session.todo_state {
            Some(s) => s,
            None => { fallback = todos::from_transcript(&self.session.transcript); &fallback }
        };
        let board = todos::board(snapshot);
        if board.is_empty() { return None; }
        let c = |r: u8, g: u8, b: u8| -> Hsla { gpui::Rgba { r: r as f32 / 255., g: g as f32 / 255., b: b as f32 / 255., a: 1. }.into() };
        let done_c = c(100, 180, 100);
        let active_c = c(110, 160, 245);
        let self_c = c(255, 200, 100);
        let warn_c = c(225, 165, 90);
        let lane_c = c(190, 165, 235);
        let meta_c = c(155, 165, 180);
        let dim_c = c(120, 120, 135);
        let pip_of = move |m: Mark| match m {
            Mark::Done => ("●", done_c), Mark::Cancelled => ("✗", c(190, 105, 115)),
            Mark::SelfActive => ("●", self_c), Mark::Active => ("●", active_c),
            Mark::Blocked => ("⊘", warn_c), Mark::CompletelyBlocked => ("✋", warn_c),
            Mark::Open => ("○", c(90, 90, 105)),
        };
        let pips = move |items: Vec<&jcode_mobile_core::jcode_sdk::todo::TodoItem>| {
            div().flex().flex_wrap().items_center().children(items.into_iter().map(move |x| {
                let (g, col) = pip_of(todos::mark(x));
                label(g, col, 11.)
            }))
        };
        let item_row = move |x: &jcode_mobile_core::jcode_sdk::todo::TodoItem| {
            let m = todos::mark(x);
            let (g, gc) = match m { Mark::Done => ("✓", c(105, 190, 125)), Mark::Open => ("○", c(135, 145, 160)), _ => pip_of(m) };
            let tc = match m {
                Mark::Done => c(135, 150, 145), Mark::Cancelled => c(145, 130, 135),
                Mark::Active | Mark::SelfActive => c(170, 190, 220), _ => c(155, 170, 195),
            };
            let note = x.external_blocker.as_ref().filter(|_| !m.finished()).map(|b| b.reason.clone())
                .or_else(|| (!x.blocked_by.is_empty() && !m.finished()).then(|| format!("after {}", x.blocked_by.join(", "))));
            div().flex().items_start().gap(px(8.)).min_w_0()
                .child(label(g, gc, 13.).flex_none().w(px(14.)))
                .child(div().flex_1().min_w_0().flex().flex_col()
                    .child(label(x.content.clone(), tc, 13.).when(m == Mark::Cancelled, |d| d.line_through()))
                    .when_some(note, |d, n| d.child(label(n, warn_c, 11.))))
        };
        let current = board.current_phase();
        let staged = board.staged();
        let total = board.items().count();
        let done = board.items().filter(|x| todos::mark(x).finished()).count();
        let all_done = done == total;
        let expanded = self.todos_expanded;

        // Header pips, labelled per phase like the TUI (`phase 1 ●●●  phase 2 ●○`).
        let header_pips = div().flex().flex_wrap().items_center().gap_x(px(10.)).gap_y(px(2.)).flex_1().min_w_0()
            .children(board.phases.iter().enumerate().map(|(i, p)| {
                let lc = if Some(i) == current { active_c } else if p.done() { done_c } else { dim_c };
                div().flex().items_center().gap(px(4.))
                    .when_some(p.number.filter(|_| staged), |d, n| d.child(label(format!("{n}"), lc, 11.)))
                    .child(pips(p.items().collect()))
            }));
        let header = div().flex().items_center().gap(px(10.)).min_w_0()
            .child(header_pips)
            .child(label(if all_done { "✓ done".to_string() } else { format!("{done}/{total}") }, if all_done { done_c } else { meta_c }, 12.).flex_none())
            .child(label(if expanded { "▴" } else { "▾" }, meta_c, 12.).flex_none());
        let intention = board.intention.clone().or(board.title.clone());
        let blockers: Vec<_> = board.blockers().map(|(x, r)| format!("✋ {}: {r}", x.content)).collect();

        let lane_view = |lane: &todos::Lane, named: bool| {
            div().flex().flex_col().gap(px(4.))
                .when(named, |d| d.child(div().flex().items_center().gap(px(6.)).min_w_0()
                    .child(label(lane.name.clone().unwrap_or_else(|| "other".into()), lane_c, 12.).font_weight(gpui::FontWeight::BOLD))
                    .when_some(lane.owner.clone(), |d, o| d.child(
                        div().px(px(5.)).rounded(px(4.)).bg(t.surface_elevated).child(label(o, meta_c, 10.))))
                    .child(pips(lane.items.iter().collect()))))
                .children(lane.items.iter().map(|x| item_row(x)))
        };
        let body = if expanded {
            div().id("todo-list").max_h(px(360.)).overflow_y_scroll().flex().flex_col().gap(px(10.)).pt(px(8.))
                .children(board.phases.iter().enumerate().map(|(i, p)| {
                    let named = p.lanes.iter().any(|l| l.name.is_some());
                    let n_done = p.items().filter(|x| todos::mark(x).finished()).count();
                    let n = p.items().count();
                    let lc = if Some(i) == current { active_c } else if p.done() { done_c } else { dim_c };
                    div().flex().flex_col().gap(px(6.))
                        .when_some(p.number.filter(|_| staged), |d, num| d.child(
                            div().flex().items_center().gap(px(8.)).pb(px(2.)).border_b_1().border_color(t.border)
                                .child(label(format!("Phase {num}"), lc, 12.).font_weight(gpui::FontWeight::BOLD))
                                .child(label(if p.done() { "✓".into() } else { format!("{n_done}/{n}") }, lc, 11.))
                                .when(Some(i) == current, |d| d.child(label("current", active_c, 10.)))))
                        .children(p.lanes.iter().map(|l| lane_view(l, named)))
                }))
                .children(board.goals.iter().map(|(g, f)| {
                    label(format!("{}↻ {f}", g.as_ref().map(|g| format!("{g} · ")).unwrap_or_default()), meta_c, 11.)
                }))
                .into_any_element()
        } else {
            div().pt(px(6.)).flex().flex_col().gap(px(4.))
                .children(board.focus().map(|x| {
                    let lane = board.phases.iter().flat_map(|p| &p.lanes).find(|l| l.items.iter().any(|i| i.id == x.id)).and_then(|l| l.name.clone());
                    div().flex().flex_col().gap(px(2.))
                        .when_some(lane, |d, l| d.child(label(l, lane_c, 11.)))
                        .child(item_row(x))
                }))
                .into_any_element()
        };
        Some(div().id("pinned-todos").debug_selector(|| "pinned-todos".into()).a11y_button("Todo list")
            .mx(px(14.)).my(px(6.)).px(px(12.)).py(px(9.)).rounded(px(12.))
            .bg(t.surface).border_1().border_color(t.border).cursor_pointer()
            .on_click(cx.listener(|this, _, _, cx| { this.todos_expanded = !this.todos_expanded; cx.notify(); }))
            .when_some(intention, |d, i| d.child(label(i, meta_c, 11.).truncate().pb(px(4.))))
            .child(header)
            .when(!all_done || expanded, |d| d.child(body))
            .children(blockers.into_iter().map(|b| label(b, warn_c, 11.).pt(px(4.))))
            .into_any_element())
    }

    /// Provisioning/connecting line shown on an empty session.
    fn connection_status_text(&self) -> Option<String> {
        Self::connection_status_text_for(&self.session)
    }

    fn connection_status_text_for(session: &SessionState) -> Option<String> {
        Some(match &session.phase {
            ConnectionPhase::Connected | ConnectionPhase::Failed { .. } => return None,
            ConnectionPhase::Connecting => "Connecting to Jcode Cloud…".to_string(),
            ConnectionPhase::Provisioning { message } => message.clone(),
            ConnectionPhase::Reconnecting { attempt } => format!("Reconnecting ({attempt})…"),
            ConnectionPhase::Disconnected => "Disconnected".into(),
        })
    }

    fn connection_status(&self, t: &Theme) -> AnyElement {
        let Some(text) = self.connection_status_text() else { return div().into_any_element(); };
        orb_row(&self.status_orb, text, t.text_muted)
            .id("connection-status").debug_selector(|| "connection-status".into())
            .into_any_element()
    }

    fn activity(&self) -> Option<&'static str> {
        Self::activity_for(&self.session)
    }

    fn activity_for(session: &SessionState) -> Option<&'static str> {
        if session.is_reasoning {
            return Some("thinking");
        }
        if !session.is_processing {
            return None;
        }
        let Some(last) = session.transcript.last().filter(|e| e.role == Role::Assistant) else {
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

    fn render_entry(&self, e: &TranscriptEntry, prompt_n: usize, expanded_tools: &std::collections::HashSet<String>, visible: Option<&stream_presentation::Row>, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let _perf = crate::perf::span("transcript.entry_build");
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
                let reasoning = visible.map_or(e.reasoning.as_str(), |row| &e.reasoning[..row.reasoning_len]);
                let text = visible.map_or(e.text.as_str(), |row| &e.text[..row.text_len]);
                if !reasoning.is_empty() {
                    col = col.child(match visible {
                        Some(row) => crate::markdown::render_reasoning_streaming(reasoning, t, row.reasoning_fade),
                        None => crate::markdown::render_reasoning(reasoning, t),
                    });
                }
                for call in &e.tool_calls {
                    col = col.child(self.tool_line(call, expanded_tools, t, cx));
                }
                if !text.is_empty() {
                    let body = match visible {
                        Some(row) => crate::markdown::render_streaming(text, t, row.text_fade),
                        None => crate::markdown::render(text, t),
                    };
                    col = col.child(div().pt(px(4.)).child(body));
                }
                col.into_any_element()
            }
        }
    }

    /// Desktop/TUI tool row: `✓ name · headline`, tap to expand output.
    fn tool_line(&self, call: &jcode_mobile_core::ToolCall, expanded_tools: &std::collections::HashSet<String>, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let expanded = expanded_tools.contains(&call.id);
        let (glyph, color) = match &call.status {
            ToolStatus::StreamingInput | ToolStatus::Running => (spinner_glyph(self.spinner_step), t.warning),
            ToolStatus::Succeeded => ("✓", t.ok),
            ToolStatus::Failed(_) => ("✗", t.error),
        };
        let headline = crate::tool_summary::cached_headline(&call.input).unwrap_or_default();
        let id = call.id.clone();
        div()
            .id(SharedString::from(format!("tool-{}", call.id)))
            .debug_selector({ let id = call.id.clone(); move || format!("tool-{id}") })
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
                let failure = match &call.status { ToolStatus::Failed(message) => Some(message.as_str()), _ => None };
                let body = crate::tool_output::body(&call.output, failure, &call.input).into_owned();
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
                        .child(body),
                )
            })
            .into_any_element()
    }

    /// Phone composer: 44pt actions inside a 22px-radius focused outline,
    /// then desktop's pill row: model, effort, `Provider · Auth` on the left,
    /// the machine chip on the right.
    fn render_composer(&self, t: &Theme, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.render_composer_for(&self.session, &self.composer, false, t, window, cx)
    }

    fn render_composer_for(
        &self,
        session: &SessionState,
        composer: &Entity<TextInput>,
        preview: bool,
        t: &Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let processing = session.is_processing;
        let ready = session.phase == ConnectionPhase::Connected && session.session_id.is_some();
        let has_text = !composer.read(cx).text().trim().is_empty();
        let focused = !preview && composer.read(cx).is_focused(window);
        let model_raw = session.model_name.clone();
        let model = crate::labels::pretty_model(model_raw.as_deref());
        let effort = session
            .reasoning_effort
            .clone()
            .filter(|e| !e.trim().is_empty() && model_raw.is_some());
        // The daemon's resolved credential wins; guessing from routes is
        // ambiguous when one model has both an OAuth and an API-key route.
        let api_method = session.auth_method.clone().or_else(|| {
            session
                .routes
                .iter()
                .find(|r| {
                    Some(&r.model) == model_raw.as_ref()
                        && session.provider_name.as_deref().is_some_and(|p| r.provider.eq_ignore_ascii_case(p))
                })
                .map(|r| r.api_method.clone())
        });
        let account = crate::labels::routed_account_label(
            session.provider_name.as_deref(),
            api_method.as_deref(),
            session.account_label.as_deref(),
        );
        let open_picker = cx.listener(|this, _, window, cx| {
            window.blur(cx);
            if !this.model_picker { haptics::play(Haptic::Light); }
            this.model_picker = true;
            cx.notify();
        });
        div()
            .id("chat-composer")
            .debug_selector(move || if preview { "preview-chat-composer" } else { "chat-composer" }.into())
            .flex()
            .flex_col()
            .flex_none()
            .min_w_0()
            .gap(px(4.))
            .px(px(12.))
            .pt(px(6.))
            .pb(px(8.))
            .bg(t.background)
            .when_some(if preview || self.voice.auto_send || self.feel.voice_gesture.active() || self.feel.voice_gesture.pull > 0.5 {
                None
            } else { self.voice.status() }, |d, status| d.child(label(status, t.text_muted, 10.5)
                .debug_selector(|| "voice-status".into()).px(px(8.))))
            .when(!ready, |d| d.child(label(
                if matches!(session.phase, ConnectionPhase::Failed { .. } | ConnectionPhase::Disconnected) {
                    "Not connected. Your draft stays here."
                } else {
                    "Connecting… You can keep typing."
                }, t.text_muted, 10.5,
            ).debug_selector(|| "composer-connection-status".into()).px(px(8.))))
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
                    .child(div().id("composer-input").debug_selector(move || if preview { "preview-composer-input" } else { "composer-input" }.into()).flex_1().min_w_0().child(composer.clone()))
                    .child(if !preview && (self.voice.busy || self.voice.starting || self.voice.recording.is_some() || self.voice.sending()) {
                        // Reuse the existing send slot. Never insert rows during a voice pull.
                        round_button("cancel-dictation", "×", t.error, true, t)
                            .a11y_button("Discard voice recording")
                            .on_click(cx.listener(|this, _, _, cx| {
                                haptics::play(Haptic::Light);
                                this.cancel_voice_animated(cx);
                                cx.notify();
                            }))
                    } else if processing && !has_text {
                        round_button("stop", "■", t.error, ready, t)
                            .a11y_button("Stop response")
                            .when(ready, |d| d.on_click(cx.listener(|this, _, _, cx| this.cancel(cx))))
                    } else {
                        round_button("send", if ready { "↑" } else { "…" }, t.text_strong, ready && has_text, t)
                            .a11y_button(if !ready { "Waiting for connection" } else if processing { "Queue follow-up" } else { "Send message" })
                            .when(ready && has_text, |d| d.on_click(cx.listener(|this, _, _, cx| this.send_draft(cx))))
                    }),
            )
            .child(
                div()
                    .id("chat-metadata")
                    .debug_selector(move || if preview { "preview-chat-metadata" } else { "chat-metadata" }.into())
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
                        let (name, parent) = crate::labels::cloud_location(session.working_dir.as_deref());
                        div()
                            .id("location")
                            .debug_selector(move || if preview { "preview-location" } else { "location" }.into())
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
            // The modal owns its whole backdrop. Otherwise a row can close
            // the sheet and the same click can reopen it via metadata below.
            .occlude()
            .absolute()
            .inset_0()
            .flex()
            .flex_col()
            .bg(gpui::black().opacity(self.feel_backdrop()))
            .child(
                div()
                    .id("picker-dismiss")
                    .debug_selector(|| "model-dismiss".into())
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
                            .debug_selector(|| "model-picker-list".into())
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
        let default_dir = self.render_default_dir(t, cx);
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
                            .id("back").debug_selector(|| "back".into()).a11y_button("Back")
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
                        .child(default_dir)
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
        .debug_selector(move || format!("model-row-{}-{}", id.0, id.1))
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

/// Thin overlay thumb showing transcript position. Hidden when nothing scrolls.
fn transcript_scrollbar(viewport: gpui::Pixels, max: gpui::Pixels, off: gpui::Pixels, t: &Theme) -> Option<gpui::Div> {
    let (vp, max, off) = (f32::from(viewport), f32::from(max), f32::from(off));
    if vp <= 0. || max <= 1. { return None; }
    let track = vp - 8.;
    let thumb = (track * vp / (vp + max)).clamp(28_f32.min(track), track);
    let top = 4. + (track - thumb) * (off / max).clamp(0., 1.);
    Some(div().debug_selector(|| "transcript-scrollbar".into())
        .absolute().right(px(2.)).top(px(top)).w(px(3.)).h(px(thumb))
        .rounded_full().bg(t.text_muted.opacity(0.45)))
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

/// Logs a frame build that blows the budget badly enough to feel like a hang.
struct FrameTimer(std::time::Instant);

impl Drop for FrameTimer {
    fn drop(&mut self) {
        let build = self.0.elapsed();
        if build > std::time::Duration::from_millis(50) {
            jcode_mobile_core::timing::stage("ui slow frame build", build);
        }
    }
}

impl Render for MobileApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _perf = crate::perf::span("app.build");
        // Always-on: logs launch-to-first-render, tap-to-transcript-render and
        // slow (>50ms) frame builds to gpui.log via conn_timing.
        let _frame = FrameTimer(std::time::Instant::now());
        if std::mem::take(&mut self.first_frame_pending) {
            jcode_mobile_core::timing::stage("app first render", jcode_mobile_core::timing::process_start().elapsed());
        }
        if let Some(opened) = self.open_started
            && !self.session.transcript.is_empty()
        {
            self.open_started = None;
            jcode_mobile_core::timing::stage("ui open to transcript render", opened.elapsed());
            jcode_mobile_core::timing::event("ui transcript render", &format!("entries={}", self.session.transcript.len()));
        }
        if !self.offline { crate::notify::set_visible_session(self.session.session_id.clone()); }
        self.system_tick(window, cx);
        self.sync_spinner_tick(cx);
        self.feel_prepare_voice(window, cx);
        if std::mem::take(&mut self.blur_on_render) {
            window.blur(cx);
            window.dismiss_virtual_keyboard();
        }
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
        let body = match self.render_session_map(&t, window, cx) {
            Some(map) => div().relative().size_full().flex().flex_col().child(body).child(map).into_any_element(),
            None => body,
        };
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
            // Two-finger pinch out: zoom to the map of live sessions.
            .on_pinch(cx.listener(|this, ev: &gpui::PinchEvent, _, cx| this.session_map_pinch(ev, cx)))
            .child(body)
            // A software keyboard belongs above every app overlay, as UIKit
            // does. The drawer must not occlude its own search/transfer keys.
            .children(drawer)
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
            });
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

/// The same polygon is used for painting and geometry regressions. No GPU or
/// screenshot acceptance is implied by inspecting these vertices.
fn preview_corner_polygon(
    bounds: gpui::Bounds<gpui::Pixels>, right: bool, bottom: bool,
) -> Option<[gpui::Point<gpui::Pixels>; 50]> {
    let width = f32::from(bounds.size.width);
    let height = f32::from(bounds.size.height);
    if !width.is_finite() || !height.is_finite() || width <= 0. || height <= 0.
        || !f32::from(bounds.origin.x).is_finite() || !f32::from(bounds.origin.y).is_finite() {
        return None;
    }
    let radius = 48_f32.min(width / 2.).min(height / 2.);
    Some(std::array::from_fn(|index| {
        let (x, y) = match index {
            0 => (0., 0.),
            1 => (radius, 0.),
            _ => {
                let angle = std::f32::consts::FRAC_PI_2 * (index - 1) as f32 / 48.;
                (radius * (1. - angle.sin()), radius * (1. - angle.cos()))
            }
        };
        bounds.origin + gpui::point(
            px(if right { width - x } else { x }),
            px(if bottom { height - y } else { y }),
        )
    }))
}

fn preview_corner_mask() -> impl IntoElement {
    gpui::canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            for (right, bottom) in [(false, false), (true, false), (false, true), (true, true)] {
                let Some(vertices) = preview_corner_polygon(bounds, right, bottom) else { continue; };
                let mut path = gpui::PathBuilder::fill();
                path.move_to(vertices[0]);
                for vertex in &vertices[1..] {
                    path.line_to(*vertex);
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
            &self.dir_input, &self.transfer_input, &self.default_dir_input]
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
