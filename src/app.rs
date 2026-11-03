//! Root view: owns credentials, the live connection, and session state, and
//! switches between the pairing, chat and settings screens.

use std::time::Duration;

use futures::StreamExt;
use futures::channel::mpsc::{UnboundedReceiver, unbounded};
use gpui::{
    AnyElement, App, Context, Entity, FontWeight, Hsla, ScrollHandle, SharedString, Window, div,
    point, prelude::*, px,
};
use jcode_mobile_core::{
    Config, Connection, ConnectionOutput, ConnectionPhase, CredentialStore, FileCredentialStore,
    Gateway, LocalIntent, Request, Role, ServerCredential, SessionState, ToolStatus,
    TranscriptEntry, credentials::device_id, gateway, parse_pair_uri,
};

use crate::text_input::{TextInput, TextInputEvent};
use crate::theme::{Theme, with_alpha};

/// Insets the shell reserves for the notch / home indicator. On iOS these come
/// from UIKit; the desktop preview fakes an iPhone 15-class device.
#[derive(Clone, Copy)]
pub struct SafeArea {
    pub top: f32,
    pub bottom: f32,
}

#[derive(PartialEq)]
enum Screen {
    Pairing,
    Chat,
    Settings,
}

pub struct MobileApp {
    screen: Screen,
    safe: SafeArea,
    /// Fixed device size for the desktop preview; None on a real phone.
    pub device: Option<(f32, f32)>,
    store: FileCredentialStore,
    servers: Vec<ServerCredential>,
    active: Option<ServerCredential>,
    connection: Option<Connection>,
    session: SessionState,
    // Pairing form
    host: Entity<TextInput>,
    port: Entity<TextInput>,
    code: Entity<TextInput>,
    pairing: bool,
    pair_error: Option<String>,
    // Chat
    composer: Entity<TextInput>,
    transcript_scroll: ScrollHandle,
    expanded_tools: std::collections::HashSet<String>,
    follow: bool,
    spinner_step: usize,
    /// Dev hook (JCODE_MOBILE_SEND): message sent once connected.
    auto_send: Option<String>,
}

impl MobileApp {
    pub fn new(safe: SafeArea, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let host = cx.new(|cx| TextInput::new("devbox.tailnet.ts.net", cx));
        let port = cx.new(|cx| {
            let mut t = TextInput::new("7643", cx);
            t.set_text(gateway::DEFAULT_PORT.to_string(), cx);
            t
        });
        let code = cx.new(|cx| TextInput::new("123456", cx));
        let composer = cx.new(|cx| TextInput::new("Message jcode", cx));
        for input in [&host, &port, &code] {
            cx.subscribe(input, |this, _, ev, cx| match ev {
                TextInputEvent::Submit => this.pair(cx),
                TextInputEvent::Changed => cx.notify(),
            })
            .detach();
        }
        cx.subscribe(&composer, |this, _, ev, cx| match ev {
            TextInputEvent::Submit => this.send_draft(cx),
            TextInputEvent::Changed => cx.notify(),
        })
        .detach();

        // Spinner tick while processing.
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(125)).await;
                let alive = this
                    .update(cx, |this, cx| {
                        if this.session.is_processing || this.session.is_reasoning {
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

        let store = FileCredentialStore::default_location();
        let servers = store.load_all();
        let mut app = Self {
            screen: Screen::Pairing,
            safe,
            device: None,
            store,
            servers,
            active: None,
            connection: None,
            session: SessionState::default(),
            host,
            port,
            code,
            pairing: false,
            pair_error: None,
            composer,
            transcript_scroll: ScrollHandle::new(),
            expanded_tools: Default::default(),
            follow: true,
            spinner_step: 0,
            auto_send: std::env::var("JCODE_MOBILE_SEND").ok(),
        };
        // Dev convenience: JCODE_MOBILE_PAIR=jcode://pair?... pre-fills and pairs.
        if let Some(p) = std::env::var("JCODE_MOBILE_PAIR").ok().and_then(|s| parse_pair_uri(&s)) {
            app.host.update(cx, |t, cx| t.set_text(p.gateway.host.clone(), cx));
            app.port.update(cx, |t, cx| t.set_text(p.gateway.port.to_string(), cx));
            app.code.update(cx, |t, cx| t.set_text(p.code.clone(), cx));
            app.pair(cx);
        } else if let Some(first) = app.servers.first().cloned() {
            app.connect(first, cx);
        }
        let _ = window;
        app
    }

    // MARK: actions

    fn pair(&mut self, cx: &mut Context<Self>) {
        if self.pairing {
            return;
        }
        let host = self.host.read(cx).text().trim().to_string();
        let code = self.code.read(cx).text().trim().to_string();
        let Ok(port) = self.port.read(cx).text().trim().parse::<u16>() else {
            self.pair_error = Some("Port must be a number".into());
            cx.notify();
            return;
        };
        if host.is_empty() || code.is_empty() {
            return;
        }
        self.pairing = true;
        self.pair_error = None;
        cx.notify();
        let gw = Gateway::new(host, port);
        let task = cx.background_spawn({
            let gw = gw.clone();
            async move {
                let name = hostname();
                gateway::pair(&gw, &code, &device_id(), &name)
            }
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                this.pairing = false;
                match result {
                    Ok(resp) => {
                        let cred = ServerCredential {
                            host: gw.host.clone(),
                            port: gw.port,
                            token: resp.token,
                            server_name: resp.server_name,
                            server_version: resp.server_version,
                            paired_at: std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .map(|d| d.as_secs())
                                .unwrap_or(0),
                            working_dir: resp.working_dir,
                        };
                        this.store.save(cred.clone());
                        this.servers = this.store.load_all();
                        this.connect(cred, cx);
                    }
                    Err(e) => this.pair_error = Some(e.to_string()),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn connect(&mut self, cred: ServerCredential, cx: &mut Context<Self>) {
        if let Some(c) = self.connection.take() {
            c.stop();
        }
        self.session = SessionState::default();
        let mut config = Config::new(cred.gateway(), cred.token.clone());
        config.working_dir = cred.working_dir.clone();
        let (tx, rx) = unbounded();
        let conn = Connection::start(config, None, move |o| {
            let _ = tx.unbounded_send(o);
        });
        self.connection = Some(conn);
        self.active = Some(cred);
        self.screen = Screen::Chat;
        self.follow = true;
        self.pump(rx, cx);
        cx.notify();
    }

    fn pump(&mut self, mut rx: UnboundedReceiver<ConnectionOutput>, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            while let Some(first) = rx.next().await {
                // Coalesce bursts of streaming deltas into one frame.
                let mut batch = vec![first];
                while let Ok(o) = rx.try_recv() {
                    batch.push(o);
                }
                let ok = this
                    .update(cx, |this, cx| {
                        for o in batch {
                            this.session.apply(o);
                        }
                        if this.session.phase == ConnectionPhase::Connected
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
        if text.is_empty() {
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

    fn forget(&mut self, id: String, cx: &mut Context<Self>) {
        self.store.remove(&id);
        self.servers = self.store.load_all();
        if self.active.as_ref().map(|a| a.id()) == Some(id) {
            if let Some(c) = self.connection.take() {
                c.stop();
            }
            self.active = None;
            self.session = SessionState::default();
            self.screen = Screen::Pairing;
        }
        cx.notify();
    }
}

fn hostname() -> String {
    std::fs::read_to_string("/etc/hostname")
        .map(|s| s.trim().to_string())
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "jcode-mobile".into())
}

// MARK: rendering helpers

fn label(text: impl Into<SharedString>, color: Hsla, size: f32) -> gpui::Div {
    div().text_size(px(size)).text_color(color).child(text.into())
}

fn button(
    id: &'static str,
    text: impl Into<SharedString>,
    primary: bool,
    enabled: bool,
    t: &Theme,
) -> gpui::Stateful<gpui::Div> {
    let (bg, fg) = match (primary, enabled) {
        (true, true) => (t.accent, t.on_accent),
        (true, false) => (t.surface_elevated, t.text_muted),
        (false, _) => (t.surface, t.text),
    };
    div()
        .id(id)
        .w_full()
        .py(px(15.))
        .flex()
        .justify_center()
        .rounded(px(12.))
        .bg(bg)
        .when(!primary, |d| d.border_1().border_color(t.border))
        .text_color(fg)
        .text_size(px(17.))
        .font_weight(FontWeight::SEMIBOLD)
        .when(enabled, |d| d.cursor_pointer().active(|s| s.opacity(0.85)))
        .child(text.into())
}

fn card(t: &Theme) -> gpui::Div {
    div()
        .w_full()
        .p(px(16.))
        .rounded(px(14.))
        .bg(t.surface)
        .border_1()
        .border_color(t.border)
}

impl MobileApp {
    fn field(&self, name: &str, input: &Entity<TextInput>, t: &Theme) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(label(name.to_uppercase(), t.text_muted, 12.).font_weight(FontWeight::MEDIUM))
            .child(input.clone())
    }

    fn render_pairing(&mut self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let can_pair = !self.host.read(cx).text().trim().is_empty()
            && !self.code.read(cx).text().is_empty()
            && self.port.read(cx).text().parse::<u16>().is_ok();
        div()
            .id("pairing")
            .size_full()
            .overflow_y_scroll()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(20.))
                    .p(px(16.))
                    .pt(px(16. + 28.))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(10.))
                            .child(
                                div().flex().items_center().gap(px(10.)).child(brand_mark(t)).child(
                                    label("jcode", t.text, 34.).font_weight(FontWeight::SEMIBOLD),
                                ),
                            )
                            .child(label("Pair with a server on your tailnet", t.text_secondary, 15.)),
                    )
                    .when_some(self.pair_error.clone(), |d, e| d.child(banner(&e, t.error, t)))
                    .child(
                        card(t)
                            .flex()
                            .flex_col()
                            .gap(px(16.))
                            .child(self.field("Host", &self.host, t))
                            .child(self.field("Port", &self.port, t))
                            .child(self.field("Pairing code", &self.code, t)),
                    )
                    .child(
                        button(
                            "pair",
                            if self.pairing { "Pairing..." } else { "Pair" },
                            true,
                            can_pair && !self.pairing,
                            t,
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.pair(cx))),
                    )
                    .when(!self.servers.is_empty(), |d| {
                        d.child(label("PAIRED SERVERS", t.text_muted, 12.)).children(
                            self.servers.iter().enumerate().map(|(i, s)| {
                                let s2 = s.clone();
                                server_row(i, s, t).on_click(cx.listener(move |this, _, _, cx| {
                                    this.connect(s2.clone(), cx)
                                }))
                            }),
                        )
                    })
                    .child(label(
                        "Run `jcode pair` on your machine, then enter the code. Traffic stays on your tailnet.",
                        t.text_muted,
                        13.,
                    )),
            )
            .into_any_element()
    }

    fn render_header(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let title = self
            .session
            .session_title
            .clone()
            .or_else(|| self.active.as_ref().map(|a| a.server_name.clone()))
            .unwrap_or_else(|| "jcode".into());
        let (dot, status) = match &self.session.phase {
            ConnectionPhase::Connected => (t.ok, self.session.model_name.clone().unwrap_or_else(|| "Connected".into())),
            ConnectionPhase::Connecting => (t.warning, "Connecting".into()),
            ConnectionPhase::Reconnecting { attempt } => (t.warning, format!("Reconnecting ({attempt})")),
            ConnectionPhase::Failed { reason } => (t.error, reason.clone()),
            ConnectionPhase::Disconnected => (t.text_muted, "Disconnected".into()),
        };
        div()
            .flex()
            .items_center()
            .gap(px(10.))
            .px(px(16.))
            .py(px(10.))
            .border_b_1()
            .border_color(t.border)
            .child(brand_mark(t))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(label(title, t.text, 17.).font_weight(FontWeight::SEMIBOLD).truncate())
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .child(div().size(px(7.)).rounded_full().bg(dot))
                            .child(label(status, t.text_secondary, 12.).truncate()),
                    ),
            )
            .child(
                icon_button("settings", "⚙", t)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.screen = Screen::Settings;
                        cx.notify()
                    })),
            )
    }

    fn render_chat(&mut self, t: &Theme, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        // Unpin follow mode when the user scrolls up.
        let max = self.transcript_scroll.max_offset().y;
        let off = -self.transcript_scroll.offset().y;
        let at_bottom = max - off < px(56.);
        let entries = &self.session.transcript;
        let prompt_count = entries.iter().filter(|e| e.role == Role::User).count();
        let mut prompt_n = 0;
        let mut rows: Vec<AnyElement> = Vec::new();
        for e in entries {
            let distance = if e.role == Role::User {
                prompt_n += 1;
                prompt_count - prompt_n
            } else {
                0
            };
            rows.push(self.render_entry(e, prompt_n, distance, t, cx));
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
                        cx.notify()
                    }))
                    .into_any_element(),
            );
        }
        let empty = entries.is_empty() && !self.session.is_reasoning;
        let _ = window;
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(self.render_header(t, cx))
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .child(if empty {
                        empty_transcript(t).into_any_element()
                    } else {
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
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap(px(16.))
                                    .px(px(16.))
                                    .py(px(8.))
                                    .children(rows),
                            )
                            .into_any_element()
                    })
                    .when(!empty && !at_bottom && max > px(0.), |d| {
                        d.child(
                            div()
                                .id("jump")
                                .absolute()
                                .right(px(16.))
                                .bottom(px(8.))
                                .size(px(40.))
                                .rounded_full()
                                .flex()
                                .items_center()
                                .justify_center()
                                .bg(t.surface_elevated)
                                .border_1()
                                .border_color(t.border)
                                .text_color(t.text)
                                .cursor_pointer()
                                .child("↓")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.follow = true;
                                    this.transcript_scroll.scroll_to_bottom();
                                    cx.notify()
                                })),
                        )
                    }),
            )
            .child(self.render_composer(t, cx))
            .into_any_element()
    }

    fn activity(&self) -> Option<&'static str> {
        if self.session.is_reasoning {
            return Some("Thinking");
        }
        if !self.session.is_processing {
            return None;
        }
        let Some(last) = self.session.transcript.last().filter(|e| e.role == Role::Assistant) else {
            return Some("Working");
        };
        if last
            .tool_calls
            .iter()
            .any(|c| matches!(c.status, ToolStatus::StreamingInput | ToolStatus::Running))
        {
            return Some("Running tools");
        }
        Some(if last.text.is_empty() { "Working" } else { "Responding" })
    }

    fn render_entry(
        &self,
        e: &TranscriptEntry,
        prompt_n: usize,
        distance: usize,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match e.role {
            Role::User => div()
                .flex()
                .gap(px(10.))
                .p(px(12.))
                .rounded(px(12.))
                .bg(Theme::prompt_background(distance))
                .when(e.is_queued, |d| d.opacity(0.6))
                .child(label(prompt_n.to_string(), t.text_muted, 13.).font_family("Lilex"))
                .child(div().flex_1().min_w_0().text_size(px(16.)).text_color(t.text).child(e.text.clone()))
                .into_any_element(),
            Role::System => label(e.text.clone(), t.text_muted, 13.).into_any_element(),
            Role::Assistant => {
                let mut col = div().flex().flex_col().gap(px(10.));
                if !e.reasoning.is_empty() {
                    col = col.child(
                        div()
                            .pl(px(10.))
                            .border_l_2()
                            .border_color(t.border)
                            .child(label(e.reasoning.clone(), t.text_muted, 13.)),
                    );
                }
                for call in &e.tool_calls {
                    col = col.child(self.tool_card(call, t, cx));
                }
                if !e.text.is_empty() {
                    col = col.child(crate::markdown::render(&e.text, t));
                }
                col.into_any_element()
            }
        }
    }

    fn tool_card(&self, call: &jcode_mobile_core::ToolCall, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let expanded = self.expanded_tools.contains(&call.id);
        let (glyph, color) = match &call.status {
            ToolStatus::StreamingInput | ToolStatus::Running => ("◌", t.warning),
            ToolStatus::Succeeded => ("✓", t.ok),
            ToolStatus::Failed(_) => ("✕", t.error),
        };
        let headline = crate::tool_summary::headline(&call.input).unwrap_or_default();
        let id = call.id.clone();
        div()
            .id(SharedString::from(format!("tool-{}", call.id)))
            .flex()
            .flex_col()
            .rounded(px(10.))
            .border_1()
            .border_color(t.border)
            .bg(t.surface)
            .overflow_hidden()
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
                    .gap(px(8.))
                    .px(px(12.))
                    .py(px(10.))
                    .child(label(glyph, color, 14.))
                    .child(label(call.name.clone(), t.text, 14.).font_family("Lilex").font_weight(FontWeight::SEMIBOLD))
                    .child(label(headline, t.text_secondary, 13.).flex_1().min_w_0().truncate()),
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
                        .px(px(12.))
                        .py(px(10.))
                        .bg(t.code_bg)
                        .border_t_1()
                        .border_color(t.border)
                        .text_size(px(12.))
                        .font_family("Lilex")
                        .text_color(t.text_secondary)
                        .child(if body.is_empty() { call.input.clone() } else { body }),
                )
            })
            .into_any_element()
    }

    fn render_composer(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let processing = self.session.is_processing;
        let has_text = !self.composer.read(cx).text().trim().is_empty();
        div()
            .flex()
            .items_end()
            .gap(px(8.))
            .px(px(12.))
            .pt(px(8.))
            .pb(px(8.))
            .border_t_1()
            .border_color(t.border)
            .bg(t.background)
            .child(div().flex_1().min_w_0().child(self.composer.clone()))
            .child(if processing && !has_text {
                circle_button("stop", "■", t.error, t.background, t)
                    .on_click(cx.listener(|this, _, _, cx| this.cancel(cx)))
            } else {
                circle_button("send", "↑", if has_text { t.accent } else { t.surface_elevated }, t.on_accent, t)
                    .on_click(cx.listener(|this, _, _, cx| this.send_draft(cx)))
            })
    }

    fn render_settings(&mut self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let s = &self.session;
        let info = [
            ("Server", self.active.as_ref().map(|a| a.server_name.clone())),
            ("Version", s.server_version.clone().or_else(|| self.active.as_ref().map(|a| a.server_version.clone()))),
            ("Model", s.model_name.clone()),
            ("Provider", s.provider_name.clone()),
            ("Session", s.session_id.clone()),
            ("Tokens", Some(format!("{} in · {} out", s.token_input, s.token_output))),
        ];
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(
                div()
                    .flex()
                    .items_center()
                    .px(px(16.))
                    .py(px(12.))
                    .border_b_1()
                    .border_color(t.border)
                    .child(
                        div()
                            .id("back")
                            .cursor_pointer()
                            .text_color(t.accent)
                            .text_size(px(17.))
                            .child("‹ Back")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.screen = if this.active.is_some() { Screen::Chat } else { Screen::Pairing };
                                cx.notify()
                            })),
                    )
                    .child(div().flex_1().flex().justify_center().child(label("Settings", t.text, 17.).font_weight(FontWeight::SEMIBOLD)))
                    .child(div().w(px(56.))),
            )
            .child(
                div()
                    .id("settings")
                    .flex_1()
                    .overflow_y_scroll()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(16.))
                            .p(px(16.))
                            .child(card(t).flex().flex_col().gap(px(10.)).children(info.into_iter().map(|(k, v)| {
                                div()
                                    .flex()
                                    .justify_between()
                                    .gap(px(12.))
                                    .child(label(k, t.text_secondary, 15.))
                                    .child(label(v.unwrap_or_else(|| "—".into()), t.text, 15.).truncate())
                            })))
                            .child(label("PAIRED SERVERS", t.text_muted, 12.))
                            .children(self.servers.iter().enumerate().map(|(i, s)| {
                                let id = s.id();
                                server_row(i, s, t).child(
                                    div()
                                        .id(SharedString::from(format!("forget-{i}")))
                                        .text_color(t.error)
                                        .text_size(px(14.))
                                        .cursor_pointer()
                                        .child("Forget")
                                        .on_click(cx.listener(move |this, _, _, cx| this.forget(id.clone(), cx))),
                                )
                            }))
                            .child(
                                button("pair-new", "Pair another server", false, true, t).on_click(cx.listener(
                                    |this, _, _, cx| {
                                        this.screen = Screen::Pairing;
                                        cx.notify()
                                    },
                                )),
                            )
                            .child(
                                button("clear", "Clear conversation", false, true, t).on_click(cx.listener(
                                    |this, _, _, cx| {
                                        this.session.apply_intent(LocalIntent::ClearedConversation);
                                        this.send(|id| Request::Clear { id });
                                        this.screen = Screen::Chat;
                                        cx.notify()
                                    },
                                )),
                            ),
                    ),
            )
            .into_any_element()
    }
}

fn brand_mark(t: &Theme) -> impl IntoElement {
    div()
        .size(px(32.))
        .rounded_full()
        .border(px(7.))
        .border_color(t.accent)
}

fn icon_button(id: &'static str, glyph: &'static str, t: &Theme) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .size(px(44.))
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .text_size(px(20.))
        .text_color(t.text_secondary)
        .cursor_pointer()
        .active(|s| s.bg(t.surface_elevated))
        .child(glyph)
}

fn circle_button(id: &'static str, glyph: &'static str, bg: Hsla, fg: Hsla, _t: &Theme) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .size(px(44.))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .bg(bg)
        .text_color(fg)
        .text_size(px(20.))
        .font_weight(FontWeight::BOLD)
        .cursor_pointer()
        .active(|s| s.opacity(0.85))
        .child(glyph)
}

fn server_row(i: usize, s: &ServerCredential, t: &Theme) -> gpui::Stateful<gpui::Div> {
    card(t)
        .id(SharedString::from(format!("server-{i}")))
        .flex()
        .items_center()
        .gap(px(12.))
        .cursor_pointer()
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .child(label(s.server_name.clone(), t.text, 16.).font_weight(FontWeight::MEDIUM))
                .child(label(format!("{}:{}", s.host, s.port), t.text_muted, 13.).font_family("Lilex").truncate()),
        )
}

fn banner(msg: &str, tint: Hsla, t: &Theme) -> gpui::Div {
    div()
        .w_full()
        .flex()
        .items_center()
        .gap(px(10.))
        .px(px(12.))
        .py(px(12.))
        .rounded(px(12.))
        .bg(with_alpha(tint, 0.12))
        .border_1()
        .border_color(with_alpha(tint, 0.32))
        .child(label("!", tint, 14.).font_weight(FontWeight::BOLD))
        .child(label(msg.to_string(), t.text, 13.).flex_1())
}

fn activity_label(text: &'static str, step: usize, t: &Theme) -> impl IntoElement {
    // Eight dots on a ring; the highlight rotates one dot per tick.
    let mut ring = div().relative().size(px(14.));
    for i in 0..8 {
        let age = (step + 8 - i) % 8;
        let a = i as f32 * std::f32::consts::FRAC_PI_4;
        let p = point(7. + 5.5 * a.sin() - 1.5, 7. - 5.5 * a.cos() - 1.5);
        ring = ring.child(
            div()
                .absolute()
                .left(px(p.x))
                .top(px(p.y))
                .size(px(3.))
                .rounded_full()
                .bg(t.accent)
                .opacity((1. - age as f32 * 0.14).max(0.18)),
        );
    }
    div().flex().items_center().gap(px(8.)).child(ring).child(label(text, t.text_secondary, 13.))
}

fn empty_transcript(t: &Theme) -> impl IntoElement {
    div()
        .size_full()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(8.))
        .child(brand_mark(t))
        .child(label("What should we work on?", t.text, 17.).font_weight(FontWeight::MEDIUM))
        .child(label("Messages run on your paired machine.", t.text_muted, 13.))
}

impl Render for MobileApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::get(cx);
        let body = match self.screen {
            Screen::Pairing => self.render_pairing(&t, cx),
            Screen::Chat => self.render_chat(&t, window, cx),
            Screen::Settings => self.render_settings(&t, cx),
        };
        let screen = div()
            .flex()
            .flex_col()
            .bg(t.background)
            .text_color(t.text)
            .font_family("IBM Plex Sans")
            .pt(px(self.safe.top))
            .pb(px(self.safe.bottom))
            .overflow_hidden()
            .child(body);
        match self.device {
            // Preview: an exact-size device screen centred in whatever window
            // the compositor gives us (tiling WMs ignore the requested size).
            Some((w, h)) => div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(gpui::rgb(0x0E0D0C))
                .child(
                    screen
                        .relative()
                        .w(px(w))
                        .h(px(h))
                        .flex_none()
                        .rounded(px(48.))
                        .border_1()
                        .border_color(t.border)
                        .child(
                            // Dynamic Island and home indicator, for honest inset checks.
                            div()
                                .absolute()
                                .top(px(11.))
                                .left(px(w / 2. - 63.))
                                .w(px(126.))
                                .h(px(37.))
                                .rounded_full()
                                .bg(gpui::black()),
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
                ),
            None => screen.size_full(),
        }
    }
}

pub fn init(cx: &mut App) {
    cx.set_global(Theme::warm_neutral());
    crate::text_input::bind_keys(cx);
}
