//! Slide-in session drawer, modelled on jcode-desktop's workspace sidebar but
//! laid out for one thumb: search at the top, then open tabs, pinned sessions,
//! and recent sessions grouped by cloud directory (44pt rows with desktop's
//! age, size, edit, and agent metrics). Primary actions (new session, account)
//! sit at the bottom in the thumb zone. Drag it left to close.

use gpui::{AnyElement, Context, Window, div, prelude::*, px, relative};
use jcode_mobile_core::Request;
use super::a11y::A11yExt as _;
use jcode_mobile_core::jcode_sdk::SessionInfo;

use super::{MobileApp, Screen};
use crate::theme::{Theme, with_alpha};

/// Drawer width as a share of the screen (iOS drawers leave a sliver of content).
const WIDTH_SHARE: f32 = 0.84;
const MAX_WIDTH: f32 = 340.;
/// Apple HIG minimum touch target.
const ROW_MIN: f32 = 44.;
/// Drag-to-close travel (share of drawer width) that commits the close.
const CLOSE_SHARE: f32 = 0.3;

impl MobileApp {
    pub(super) fn open_drawer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.blur(cx);
        self.drawer_open = true;
        self.drawer_last = std::time::Instant::now();
        self.send(|id| Request::ListSessions { id });
        cx.notify();
    }

    pub(super) fn close_drawer(&mut self, cx: &mut Context<Self>) {
        self.drawer_open = false;
        self.drawer_drag = 0.;
        self.drawer_drag_last = None;
        self.drawer_last = std::time::Instant::now();
        cx.notify();
    }

    /// Focus the tab already showing `session_id`, or resume it in a new tab.
    fn drawer_open_session(&mut self, session_id: String, cx: &mut Context<Self>) {
        if self.account.is_none() {
            return;
        }
        if self.session.session_id.as_deref() != Some(&session_id) {
            match self.parked.iter().find(|p| p.session.session_id.as_deref() == Some(&session_id)).map(|p| p.id) {
                Some(tab) => self.focus_tab(tab, cx),
                None => {
                    let id = self.next_tab;
                    self.next_tab += 1;
                    let at = self.tab_order.iter().position(|t| *t == self.active_tab).map_or(self.tab_order.len(), |i| i + 1);
                    self.tab_order.insert(at, id);
                    self.park_active();
                    self.active_tab = id;
                    self.session = Default::default();
                    self.start_connection(Some(session_id), cx);
                }
            }
        }
        self.close_drawer(cx);
    }

    /// Eased drawer progress, 0 closed to 1 open. Call once per frame.
    fn drawer_progress(&mut self, window: &mut Window) -> f32 {
        let now = std::time::Instant::now();
        let dt = now.duration_since(self.drawer_last).as_secs_f32().min(0.05);
        self.drawer_last = now;
        let target = if self.drawer_open { 1. } else { 0. };
        let gap = if crate::system::reduce_motion() { 0. } else { target - self.drawer_t };
        if gap == 0. {
            self.drawer_t = target;
        }
        // Critically damped approach, like UIKit's ~300ms sheet curve.
        self.drawer_t = if gap.abs() < 0.002 { target } else { self.drawer_t + gap * (1. - (-dt * 14.).exp()) };
        if self.drawer_t != target {
            window.request_animation_frame();
        }
        self.drawer_t
    }

    /// The drawer and its scrim, laid over `content`. Nothing when fully closed.
    pub(super) fn render_drawer(&mut self, t: &Theme, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let p = self.drawer_progress(window);
        if p <= 0. {
            return None;
        }
        let screen_w = self.device.map_or_else(|| f32::from(window.viewport_size().width), |(w, _)| w);
        let width = (screen_w * WIDTH_SHARE).min(MAX_WIDTH);
        // Finger-follow while dragging closed; a pause or lift settles it.
        if let Some(last) = self.drawer_drag_last {
            if last.elapsed() > crate::swipe::IDLE_END {
                self.end_drawer_drag(width, cx);
            } else {
                window.request_animation_frame();
            }
        }
        let drag = self.drawer_drag.clamp(-width, 0.);
        let p = (p + drag / width).clamp(0., 1.);
        let x = -width * (1. - p);
        let scrim = div()
            .id("drawer-scrim")
            .absolute()
            .inset_0()
            .bg(gpui::black().opacity(0.45 * p))
            .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, window, cx| {
                window.prevent_default();
                cx.stop_propagation();
                this.close_drawer(cx);
            }));
        let panel = div()
            .id("drawer")
            .absolute()
            .top_0()
            .bottom_0()
            .left(px(x))
            .w(px(width))
            .flex()
            .flex_col()
            .bg(t.raised)
            .border_r_1()
            .border_color(t.border)
            .pt(px(self.drawer_insets.0))
            .pb(px(self.drawer_insets.1))
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_scroll_wheel(cx.listener(move |this, ev: &gpui::ScrollWheelEvent, _window, cx| {
                let d = ev.delta.pixel_delta(px(20.));
                let (dx, dy) = (f32::from(d.x), f32::from(d.y));
                // Only a mostly-horizontal leftward drag moves the drawer;
                // vertical drags keep scrolling the list.
                if this.drawer_drag_last.is_some() || (dx < -4. && dx.abs() > dy.abs() * 1.5) {
                    this.drawer_drag = (this.drawer_drag + dx).min(0.);
                    this.drawer_drag_last = Some(std::time::Instant::now());
                    // request_animation_frame panics outside render; notify suffices.
                    cx.stop_propagation();
                    cx.notify();
                }
                if ev.touch_phase == gpui::TouchPhase::Ended && this.drawer_drag_last.is_some() {
                    this.end_drawer_drag(width, cx);
                }
            }))
            .child(self.render_drawer_search(t))
            .child(self.render_session_list(t, cx))
            .child(self.render_quick_actions(t, cx))
            .child(self.render_account_footer(t, cx));
        Some(div().absolute().inset_0().child(scrim).child(panel).into_any_element())
    }

    /// Release of a drag-to-close: past the threshold closes, else springs back.
    fn end_drawer_drag(&mut self, width: f32, cx: &mut Context<Self>) {
        let drag = self.drawer_drag;
        self.drawer_drag_last = None;
        self.drawer_drag = 0.;
        if should_close_drawer(drag, width) {
            // Continue the slide from where the finger left it.
            self.drawer_t = (self.drawer_t + drag / width).clamp(0., 1.);
            crate::haptics::play(crate::haptics::Haptic::Light);
            self.close_drawer(cx);
        } else {
            self.drawer_t = (self.drawer_t + drag / width).clamp(0., 1.);
            self.drawer_last = std::time::Instant::now();
            cx.notify();
        }
    }

    /// Search field pinned at the top: filters by title, save label, and dir.
    fn render_drawer_search(&self, t: &Theme) -> impl IntoElement {
        div().flex_none().px(px(10.)).pt(px(10.)).pb(px(6.)).child(
            div()
                .h(px(36.))
                .px(px(10.))
                .flex()
                .items_center()
                .gap(px(8.))
                .rounded(px(10.))
                .bg(t.surface)
                .child(div().flex_none().text_size(crate::system::fs(13.)).text_color(t.text_muted).child("⌕"))
                .child(div().flex_1().min_w_0().child(self.drawer_search.clone())),
        )
    }

    /// Thumb-zone actions above the account row: a full-width New session
    /// button (44pt) and a compact refresh.
    pub(super) fn render_quick_actions(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex_none()
            .flex()
            .gap(px(8.))
            .px(px(10.))
            .pt(px(8.))
            .border_t_1()
            .border_color(t.border)
            .child(
                div()
                    .id("drawer-new-thread").a11y_text_button()
                    .flex_1()
                    .h(px(ROW_MIN))
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap(px(6.))
                    .rounded(px(12.))
                    .bg(t.surface_elevated)
                    .cursor_pointer()
                    .active(|el| el.opacity(0.7))
                    .text_size(crate::system::fs(14.))
                    .text_color(t.text_strong)
                    .child("+")
                    .child("New session")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.new_tab(cx);
                        this.screen = Screen::Chat;
                        this.close_drawer(cx);
                    })),
            )
            .child(
                div()
                    .id("drawer-refresh").a11y_button("Refresh sessions")
                    .flex_none()
                    .size(px(ROW_MIN))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(12.))
                    .bg(t.surface)
                    .cursor_pointer()
                    .active(|el| el.opacity(0.7))
                    .text_size(crate::system::fs(16.))
                    .text_color(t.text_secondary)
                    .child("↻")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.send(|id| Request::ListSessions { id });
                        cx.notify();
                    })),
            )
    }

    /// New tab with a fresh session in `dir` (the per-directory + button).
    pub(super) fn new_tab_in_dir(&mut self, dir: Option<String>, cx: &mut Context<Self>) {
        if self.account.is_none() {
            return;
        }
        let id = self.next_tab;
        self.next_tab += 1;
        let at = self.tab_order.iter().position(|t| *t == self.active_tab).map_or(self.tab_order.len(), |i| i + 1);
        self.tab_order.insert(at, id);
        self.park_active();
        self.active_tab = id;
        self.session = Default::default();
        self.session.working_dir = dir.clone();
        self.start_connection_in(None, dir, cx);
        self.screen = Screen::Chat;
        self.close_drawer(cx);
    }

    /// Open tabs, so they can be reached and closed without the header strip.
    fn render_open_tabs(&self, t: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.tab_order.len() < 2 {
            return None;
        }
        let mut col = div().flex().flex_col().child(section_label("Open tabs", t));
        for &id in &self.tab_order {
            let s = self.tab_session(id);
            let title = s.and_then(|s| s.session_title.clone()).unwrap_or_else(|| "New session".into());
            let active = id == self.active_tab;
            let busy = s.is_some_and(|s| s.is_processing);
            col = col.child(
                div()
                    .id(("drawer-tab", id))
                    .mx(px(8.))
                    .min_h(px(ROW_MIN))
                    .pl(px(12.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .rounded(px(10.))
                    .cursor_pointer()
                    .bg(if active { t.surface } else { t.raised })
                    .active(|el| el.bg(t.surface))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if id != this.active_tab {
                            this.focus_tab(id, cx);
                        }
                        this.screen = Screen::Chat;
                        this.close_drawer(cx);
                    }))
                    .child(div().flex_none().size(px(6.)).rounded_full().bg(if busy { t.accent } else { t.ok }))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(crate::system::fs(14.))
                            .text_color(if active { t.text_strong } else { t.text })
                            .child(title),
                    )
                    .child(
                        // Full 44pt hit box for the close glyph.
                        div()
                            .id(("drawer-tab-close", id)).a11y_button("Close tab")
                            .flex_none()
                            .size(px(ROW_MIN))
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_size(crate::system::fs(15.))
                            .text_color(t.text_muted)
                            .child("×")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.close_tab(id, cx);
                            })),
                    ),
            );
        }
        Some(col.into_any_element())
    }

    pub(super) fn render_session_list(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let current = self.session.session_id.clone();
        let open: Vec<String> = self.parked.iter().filter_map(|p| p.session.session_id.clone()).collect();
        let mut list = div().flex().flex_col().pb(px(8.));
        // Any tab's list will do; the active one may not have fetched yet.
        let sessions = std::iter::once(&self.session.session_list)
            .chain(self.parked.iter().map(|p| &p.session.session_list))
            .max_by_key(|l| l.len())
            .unwrap_or(&self.session.session_list);
        let query = self.drawer_search.read(cx).text().trim().to_lowercase();
        let now_ms = now_ms();
        let searching = !query.is_empty();
        if !searching {
            if let Some(tabs) = self.render_open_tabs(t, cx) {
                list = list.child(tabs);
            }
        }
        let agents = agent_counts(sessions);
        let visible: Vec<SessionInfo> = sessions
            .iter()
            .filter(|s| !s.archived && s.parent_session_id.is_none())
            .filter(|s| !searching || matches_query(s, &query))
            .cloned()
            .collect();
        let pinned: Vec<SessionInfo> = visible.iter().filter(|s| s.saved).cloned().collect();
        let rest: Vec<SessionInfo> = visible.iter().filter(|s| !s.saved).cloned().collect();
        if searching && visible.is_empty() {
            list = list.child(
                div()
                    .px(px(16.))
                    .py(px(12.))
                    .text_size(crate::system::fs(12.))
                    .text_color(t.text_muted)
                    .child("No matching sessions"),
            );
        }
        let row = |s: &SessionInfo, cx: &mut Context<Self>| -> AnyElement {
            let selected = current.as_deref() == Some(s.session_id.as_str());
            let is_open = selected || open.contains(&s.session_id);
            let working = s.status == "running";
            let live = is_open || working || s.user_connected;
            let sid = s.session_id.clone();
            let meta = session_meta(s, now_ms, agents.get(&s.session_id).copied().unwrap_or(0));
            let edits = s.edit_stats.as_ref().filter(|e| e.added + e.removed > 0).cloned();
            div()
                .id(gpui::SharedString::from(format!("drawer-session-{}", s.session_id)))
                .mx(px(8.))
                .min_h(px(ROW_MIN + 8.))
                .px(px(12.))
                .py(px(6.))
                .flex()
                .flex_col()
                .justify_center()
                .gap(px(1.))
                .rounded(px(10.))
                .cursor_pointer()
                .bg(if selected { t.surface } else { t.raised })
                .active(|el| el.bg(t.surface))
                .on_click(cx.listener(move |this, _, _, cx| this.drawer_open_session(sid.clone(), cx)))
                .child(
                    div()
                        .w_full()
                        .min_w_0()
                        .overflow_hidden()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .child(
                            // Desktop's open mark: a dot for sessions in a tab,
                            // accent while a turn runs.
                            div().flex_none().size(px(6.)).rounded_full().bg(if working {
                                t.accent
                            } else if is_open {
                                t.text_secondary
                            } else {
                                with_alpha(t.text_muted, 0.)
                            }),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_size(crate::system::fs(14.5))
                                .line_height(relative(1.35))
                                .text_color(if live { t.text_strong } else { t.text })
                                .child(session_title(s)),
                        ),
                )
                .child(
                    div()
                        .pl(px(12.))
                        .flex()
                        .gap(px(6.))
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_size(crate::system::fs(11.5))
                        .text_color(t.text_muted)
                        .when_some(meta, |d, m| d.child(m))
                        .when_some(edits, |d, e| {
                            d.child(div().text_color(t.ok).child(format!("+{}", e.added)))
                                .child(div().text_color(t.error).child(format!("−{}", e.removed)))
                        }),
                )
                .into_any_element()
        };
        if !pinned.is_empty() {
            list = list.child(section_label("Pinned", t));
            for s in &pinned {
                list = list.child(row(s, cx));
            }
        }
        if sessions.is_empty() {
            list = list.child(
                div()
                    .px(px(16.))
                    .py(px(12.))
                    .text_size(crate::system::fs(12.))
                    .text_color(t.text_muted)
                    .child(if self.session.phase == jcode_mobile_core::ConnectionPhase::Connected {
                        "Loading sessions…"
                    } else {
                        "Sessions appear once connected."
                    }),
            );
        }
        for (dir, rows) in group_by_dir(&rest) {
            let (name, parent) = crate::labels::cloud_location(dir.as_deref());
            let new_dir = dir.clone();
            list = list.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .pl(px(14.))
                    .pt(px(10.))
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .child(
                        gpui::svg()
                            .flex_none()
                            .path("icons/cloud.svg")
                            .w(px(13.))
                            .h(px(8.))
                            .text_color(t.text_muted),
                    )
                    .child(div().flex_none().text_size(crate::system::fs(12.)).text_color(t.text_secondary).child(name))
                    .when_some(parent, |d, parent| {
                        d.child(div().min_w_0().truncate().text_size(crate::system::fs(11.)).text_color(t.text_muted).child(parent))
                    })
                    .child(div().flex_1())
                    .child(
                        // New session in this directory; 44pt target.
                        div()
                            .id(gpui::SharedString::from(format!("drawer-new-in-{}", dir.as_deref().unwrap_or(""))))
                            .flex_none()
                            .size(px(ROW_MIN))
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_size(crate::system::fs(17.))
                            .text_color(t.text_muted)
                            .cursor_pointer()
                            .active(|el| el.text_color(t.text_strong))
                            .child("+")
                            .on_click(cx.listener(move |this, _, _, cx| this.new_tab_in_dir(new_dir.clone(), cx))),
                    ),
            );
            for s in rows {
                list = list.child(row(s, cx));
            }
        }
        div().id("drawer-sessions").flex_1().min_h_0().overflow_y_scroll().child(list)
    }

    /// Desktop's `sidebar_account` footer: avatar initial, email, plan line.
    pub(super) fn render_account_footer(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let email = self.account.as_ref().and_then(|a| a.email.clone()).unwrap_or_else(|| "Not signed in".into());
        let initial = email.chars().next().map(|c| c.to_ascii_uppercase().to_string()).unwrap_or_default();
        div()
            .id("drawer-account").a11y_button("Account settings")
            .flex_none()
            .mx(px(8.))
            .mb(px(6.))
            .mt(px(6.))
            .px(px(8.))
            .min_h(px(ROW_MIN + 8.))
            .flex()
            .items_center()
            .gap(px(10.))
            .rounded(px(12.))
            .cursor_pointer()
            .active(|el| el.bg(t.background))
            .on_click(cx.listener(|this, _, _, cx| {
                this.screen = Screen::Settings;
                this.close_drawer(cx);
            }))
            .child(
                div()
                    .flex_none()
                    .size(px(30.))
                    .rounded_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(t.surface_elevated)
                    .text_size(crate::system::fs(13.))
                    .text_color(t.text_strong)
                    .child(initial),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(div().truncate().text_size(crate::system::fs(13.)).text_color(t.text_strong).child(email))
                    .child(div().text_size(crate::system::fs(11.)).text_color(t.text_muted).child("Account, plan & settings")),
            )
            .child(div().flex_none().text_size(crate::system::fs(16.)).text_color(t.text_muted).child("⚙"))
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

fn section_label(text: &'static str, t: &Theme) -> impl IntoElement {
    div()
        .px(px(16.))
        .pt(px(12.))
        .pb(px(4.))
        .text_size(crate::system::fs(11.))
        .text_color(t.text_muted)
        .child(text)
}

/// Last activity: transcript update, then process activity, then creation.
fn activity_ms(s: &SessionInfo) -> u64 {
    s.updated_at_ms
        .or(s.last_active_at_ms)
        .map(|v| v.max(0) as u64)
        .or_else(|| created_ms(&s.session_id))
        .unwrap_or(0)
}

fn should_close_drawer(drag: f32, width: f32) -> bool {
    drag < -width * CLOSE_SHARE
}

fn matches_query(s: &SessionInfo, q: &str) -> bool {
    session_title(s).to_lowercase().contains(q)
        || s.save_label.as_deref().is_some_and(|l| l.to_lowercase().contains(q))
        || s.working_dir.as_deref().is_some_and(|d| d.to_lowercase().contains(q))
}

/// Swarm agents reporting to each owner session.
fn agent_counts(sessions: &[SessionInfo]) -> std::collections::HashMap<String, usize> {
    let mut out = std::collections::HashMap::new();
    for s in sessions {
        if let Some(p) = &s.parent_session_id {
            *out.entry(p.clone()).or_insert(0) += 1;
        }
    }
    out
}

/// Sessions grouped by working directory, groups ordered by their latest activity.
fn group_by_dir(sessions: &[SessionInfo]) -> Vec<(Option<String>, Vec<&SessionInfo>)> {
    let mut groups: Vec<(Option<String>, Vec<&SessionInfo>)> = Vec::new();
    let mut ordered: Vec<&SessionInfo> = sessions.iter().filter(|s| s.parent_session_id.is_none()).collect();
    ordered.sort_by_key(|s| std::cmp::Reverse(activity_ms(s)));
    for s in ordered {
        let dir = s.working_dir.clone().filter(|d| !d.trim().is_empty());
        match groups.iter_mut().find(|(d, _)| *d == dir) {
            Some((_, rows)) => rows.push(s),
            None => groups.push((dir, vec![s])),
        }
    }
    groups
}

pub(super) fn session_title(s: &SessionInfo) -> String {
    if let Some(t) = s.title.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
        return t.to_string();
    }
    // `session_<name>_<millis>[_<hex>]` → "Name", like the TUI picker.
    s.session_id
        .strip_prefix("session_")
        .and_then(|rest| rest.split('_').next())
        .map(|name| {
            let mut c = name.chars();
            c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
        })
        .unwrap_or_else(|| s.session_id.clone())
}

/// Desktop's `sidebar_session_meta`: "3m ago · 12k tok · 2 agents".
fn session_meta(s: &SessionInfo, now_ms: u64, agents: usize) -> Option<String> {
    let when = Some(activity_ms(s)).filter(|ms| *ms > 0).map(|ms| time_ago(ms, now_ms));
    let tokens = s.transcript_bytes.filter(|b| *b > 0).map(|b| estimated_tokens(b / 4));
    let agents = (agents > 0).then(|| format!("{agents} agent{}", if agents == 1 { "" } else { "s" }));
    let parts: Vec<String> = [when, tokens, agents].into_iter().flatten().collect();
    (!parts.is_empty()).then(|| parts.join(" · "))
}

pub(super) fn created_ms(session_id: &str) -> Option<u64> {
    session_id
        .split('_')
        .filter_map(|part| part.parse::<u64>().ok())
        .find(|v| (1_000_000_000_000..10_000_000_000_000).contains(v))
}

fn time_ago(created_ms: u64, now_ms: u64) -> String {
    let seconds = now_ms.saturating_sub(created_ms) / 1_000;
    let minutes = seconds / 60;
    let hours = minutes / 60;
    let days = hours / 24;
    match () {
        _ if seconds < 60 => format!("{seconds}s ago"),
        _ if minutes < 60 => format!("{minutes}m ago"),
        _ if hours < 24 => format!("{hours}h ago"),
        _ if days < 7 => format!("{days}d ago"),
        _ if days < 30 => format!("{}w ago", days / 7),
        _ => format!("{}mo ago", days / 30),
    }
}

fn estimated_tokens(tokens: u64) -> String {
    match tokens {
        t if t < 1_000 => format!("{t} tok"),
        t if t < 100_000 => format!("{:.1}k tok", t as f64 / 1_000.),
        t if t < 1_000_000 => format!("{}k tok", t / 1_000),
        t => format!("{:.1}M tok", t as f64 / 1_000_000.),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meta_and_titles() {
        assert_eq!(time_ago(0, 90_000), "1m ago");
        assert_eq!(estimated_tokens(1_234), "1.2k tok");
        assert_eq!(created_ms("session_fox_1700000000000_ab"), Some(1_700_000_000_000));
    }

    #[test]
    fn search_matches_title_label_and_dir() {
        let s: SessionInfo = serde_json::from_value(serde_json::json!({
            "session_id": "session_fox_1700000000000_ab",
            "status": "idle",
            "title": "Fix Turkey parser",
            "save_label": "Pinned Work",
            "working_dir": "/home/j/jcode-mobile",
        }))
        .expect("session json");
        for q in ["turkey", "pinned", "mobile"] {
            assert!(matches_query(&s, q), "{q}");
        }
        assert!(!matches_query(&s, "zebra"));
    }

    #[test]
    fn drawer_closes_past_threshold() {
        assert!(should_close_drawer(-0.31 * 300., 300.));
        assert!(!should_close_drawer(-0.29 * 300., 300.));
        assert!(!should_close_drawer(0., 300.));
    }
}
