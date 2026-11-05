//! Slide-in session drawer, modelled on jcode-desktop's workspace sidebar:
//! quick-action pills, sessions grouped by working directory with desktop's
//! row metrics, and the account footer. Opened by the header's ≡ button.

use gpui::{AnyElement, Context, Window, div, prelude::*, px, relative};
use jcode_mobile_core::Request;
use jcode_mobile_core::jcode_sdk::SessionInfo;

use super::{MobileApp, Screen};
use crate::theme::{Theme, with_alpha};

/// Drawer width as a share of the screen (iOS drawers leave a sliver of content).
const WIDTH_SHARE: f32 = 0.84;
const MAX_WIDTH: f32 = 340.;

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
        let gap = target - self.drawer_t;
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
            .child(self.render_quick_actions(t, cx))
            .child(self.render_session_list(t, cx))
            .child(self.render_account_footer(t, cx));
        Some(div().absolute().inset_0().child(scrim).child(panel).into_any_element())
    }

    /// Desktop's `render_sidebar_quick_actions`: 26px pills on TOOL_BG.
    fn render_quick_actions(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let quick = |id: &'static str, glyph: &'static str, text: &'static str| {
            div()
                .id(id)
                .flex_1()
                .min_w_0()
                .h(px(30.))
                .px(px(8.))
                .flex()
                .items_center()
                .justify_center()
                .gap(px(4.))
                .rounded_full()
                .cursor_pointer()
                .bg(t.surface)
                .text_size(px(12.))
                .text_color(t.text_secondary)
                .child(glyph)
                .child(div().min_w_0().truncate().child(text))
        };
        div()
            .flex_none()
            .flex()
            .gap(px(8.))
            .px(px(10.))
            .pt(px(10.))
            .pb(px(6.))
            .child(quick("drawer-new-thread", "+", "Thread").on_click(cx.listener(|this, _, _, cx| {
                this.new_tab(cx);
                this.screen = Screen::Chat;
                this.close_drawer(cx);
            })))
            .child(quick("drawer-refresh", "↻", "Refresh").on_click(cx.listener(|this, _, _, cx| {
                this.send(|id| Request::ListSessions { id });
                cx.notify();
            })))
    }

    fn render_session_list(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let current = self.session.session_id.clone();
        let open: Vec<String> = self.parked.iter().filter_map(|p| p.session.session_id.clone()).collect();
        let mut list = div().flex().flex_col().pb(px(8.));
        let sessions = &self.session.session_list;
        if sessions.is_empty() {
            list = list.child(
                div()
                    .px(px(16.))
                    .py(px(12.))
                    .text_size(px(12.))
                    .text_color(t.text_muted)
                    .child(if self.session.phase == jcode_mobile_core::ConnectionPhase::Connected {
                        "Loading sessions…"
                    } else {
                        "Sessions appear once connected."
                    }),
            );
        }
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as u64);
        for (dir, rows) in group_by_dir(sessions) {
            let (name, parent) = crate::labels::cloud_location(dir.as_deref());
            list = list.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .px(px(14.))
                    .pt(px(12.))
                    .pb(px(4.))
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
                    .child(div().flex_none().text_size(px(12.)).text_color(t.text_secondary).child(name))
                    .when_some(parent, |d, parent| {
                        d.child(div().min_w_0().truncate().text_size(px(11.)).text_color(t.text_muted).child(parent))
                    }),
            );
            for s in rows {
                let selected = current.as_deref() == Some(s.session_id.as_str());
                let is_open = selected || open.contains(&s.session_id);
                let working = s.status == "running";
                let live = is_open || working || s.user_connected;
                let sid = s.session_id.clone();
                list = list.child(
                    div()
                        .id(gpui::SharedString::from(format!("drawer-session-{}", s.session_id)))
                        .mx(px(8.))
                        .mb(px(2.))
                        .px(px(10.))
                        .py(px(7.))
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .rounded(px(10.))
                        .cursor_pointer()
                        .bg(if selected { t.background } else { t.raised })
                        .active(|el| el.bg(t.background))
                        .on_click(cx.listener(move |this, _, _, cx| this.drawer_open_session(sid.clone(), cx)))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(6.))
                                .child(
                                    // Desktop's open mark: a dot for sessions in a tab,
                                    // accent while a turn runs.
                                    div()
                                        .flex_none()
                                        .size(px(6.))
                                        .rounded_full()
                                        .bg(if working {
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
                                        .text_size(px(14.))
                                        .line_height(relative(1.4))
                                        .text_color(if live { t.text_strong } else { t.text_secondary })
                                        .child(session_title(s)),
                                ),
                        )
                        .when_some(session_meta(s, now_ms), |d, meta| {
                            d.child(div().pl(px(12.)).text_size(px(11.)).text_color(t.text_muted).child(meta))
                        }),
                );
            }
        }
        div().id("drawer-sessions").flex_1().min_h_0().overflow_y_scroll().child(list)
    }

    /// Desktop's `sidebar_account` footer: avatar initial, email, plan line.
    fn render_account_footer(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let email = self.account.as_ref().and_then(|a| a.email.clone()).unwrap_or_else(|| "Not signed in".into());
        let initial = email.chars().next().map(|c| c.to_ascii_uppercase().to_string()).unwrap_or_default();
        div()
            .id("drawer-account")
            .flex_none()
            .mx(px(8.))
            .mb(px(6.))
            .mt(px(4.))
            .px(px(8.))
            .py(px(8.))
            .flex()
            .items_center()
            .gap(px(10.))
            .rounded(px(12.))
            .border_t_1()
            .border_color(t.border)
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
                    .text_size(px(13.))
                    .text_color(t.text_strong)
                    .child(initial),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(div().truncate().text_size(px(13.)).text_color(t.text_strong).child(email))
                    .child(div().text_size(px(11.)).text_color(t.text_muted).child("Jcode Cloud")),
            )
            .child(div().flex_none().text_size(px(16.)).text_color(t.text_muted).child("⚙"))
    }
}

/// Sessions grouped by working directory, groups ordered by their newest session.
fn group_by_dir(sessions: &[SessionInfo]) -> Vec<(Option<String>, Vec<&SessionInfo>)> {
    let mut groups: Vec<(Option<String>, Vec<&SessionInfo>)> = Vec::new();
    let mut ordered: Vec<&SessionInfo> = sessions.iter().filter(|s| s.parent_session_id.is_none()).collect();
    ordered.sort_by_key(|s| std::cmp::Reverse(created_ms(&s.session_id).unwrap_or(0)));
    for s in ordered {
        let dir = s.working_dir.clone().filter(|d| !d.trim().is_empty());
        match groups.iter_mut().find(|(d, _)| *d == dir) {
            Some((_, rows)) => rows.push(s),
            None => groups.push((dir, vec![s])),
        }
    }
    groups
}

fn session_title(s: &SessionInfo) -> String {
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

/// Desktop's `sidebar_session_meta`: "3m ago · 12k tok".
fn session_meta(s: &SessionInfo, now_ms: u64) -> Option<String> {
    let created = created_ms(&s.session_id).map(|ms| time_ago(ms, now_ms));
    let tokens = s.transcript_bytes.filter(|b| *b > 0).map(|b| estimated_tokens(b / 4));
    match (created, tokens) {
        (Some(c), Some(t)) => Some(format!("{c} · {t}")),
        (c, t) => c.or(t),
    }
}

fn created_ms(session_id: &str) -> Option<u64> {
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
}
