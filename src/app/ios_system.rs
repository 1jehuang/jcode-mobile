//! Per-frame glue for `crate::system`: re-read iOS settings, keep the orbs in
//! step with Reduce Motion and appearance, index sessions in Spotlight, open
//! sessions tapped there, and the persistent iPad sidebar on wide layouts.

use gpui::{AnyElement, Context, Window, div, prelude::*, px};

use super::MobileApp;
use crate::system;
use crate::theme::Theme;

/// Width at which the session list docks beside the chat (iPad, landscape).
pub(super) const WIDE_MIN: f32 = 700.;
const SIDEBAR_WIDTH: f32 = 300.;
static CONTENT_WIDTH: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

impl MobileApp {
    /// Call at the top of `render`.
    pub(super) fn system_tick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let w = self.screen_width(window);
        let content = if w >= WIDE_MIN && self.account.is_some() { w - SIDEBAR_WIDTH } else { w };
        CONTENT_WIDTH.store(content.to_bits(), std::sync::atomic::Ordering::Relaxed);
        let changed = system::poll(cx);
        static SYNCED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        if changed || !SYNCED.swap(true, std::sync::atomic::Ordering::Relaxed) {
            let reduce = system::reduce_motion();
            let theme = if system::is_light() {
                gpui_thinking_orbs::OrbTheme::Light
            } else {
                gpui_thinking_orbs::OrbTheme::Dark
            };
            for orb in [self.activity_orb.clone(), self.status_orb.clone()] {
                orb.update(cx, |o, cx| {
                    o.set_reduced_motion(reduce, cx);
                    o.set_theme(theme, cx);
                });
            }
            window.refresh();
        }
        for id in system::take_spotlight_opens() {
            self.open_session(id, None, cx);
        }
        if self.account.is_some() {
            let items: Vec<(String, String, Option<String>)> = self
                .session
                .session_list
                .iter()
                .filter(|s| s.parent_session_id.is_none())
                .map(|s| (s.session_id.clone(), super::sidebar::session_title(s), s.working_dir.clone()))
                .collect();
            if !items.is_empty() {
                system::index_sessions(&items);
            }
        }
    }

    /// Width of the chat column: the phone screen, or what the iPad sidebar leaves.
    pub(super) fn content_width(&self) -> f32 {
        let w = f32::from_bits(CONTENT_WIDTH.load(std::sync::atomic::Ordering::Relaxed));
        if w > 0. { w } else { self.device.map_or(390., |(w, _)| w) }
    }

    /// Logical screen width (preview device or real viewport).
    pub(super) fn screen_width(&self, window: &Window) -> f32 {
        self.device.map_or_else(|| f32::from(window.viewport_size().width), |(w, _)| w)
    }

    /// iPad/wide: chat with the session list docked on the left. Narrow:
    /// `body` unchanged (the slide-in drawer serves phones).
    pub(super) fn wrap_wide(&mut self, body: AnyElement, t: &Theme, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        if self.screen_width(window) < WIDE_MIN || self.account.is_none() {
            return body;
        }
        let sidebar = div()
            .id("wide-sidebar")
            .flex_none()
            .w(px(SIDEBAR_WIDTH))
            .h_full()
            .flex()
            .flex_col()
            .bg(t.raised)
            .border_r_1()
            .border_color(t.border)
            .child(self.render_quick_actions(t, cx))
            .child(self.render_session_list(t, cx))
            .child(self.render_account_footer(t, cx));
        div()
            .flex()
            .flex_row()
            .flex_1()
            .min_h_0()
            .w_full()
            .child(sidebar)
            .child(div().flex().flex_col().flex_1().min_w_0().h_full().child(body))
            .into_any_element()
    }
}

/// Sign-out: drop the Spotlight entries for this account.
pub(super) fn forget_index() {
    system::clear_index();
}
