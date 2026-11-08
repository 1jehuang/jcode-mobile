//! Pinch out to zoom from the open session to a 2D map of every live session
//! (tab). The focused session's card shrinks from full screen into its grid
//! cell while the others fade in around it. Tap a card (or pinch back in) to
//! zoom into it.
use super::{ConnectionPhase, MobileApp, Screen, label};
use super::a11y::A11yExt as _;
use crate::haptics::{self, Haptic};
use crate::theme::Theme;
use gpui::{AnyElement, Context, Hsla, PinchEvent, SharedString, TouchPhase, Window, div, prelude::*, px};
use jcode_mobile_core::session::{Role, SessionState};
use std::time::Instant;

/// Map progress past which a released pinch settles on the map.
const COMMIT: f32 = 0.35;
/// Fingers closing to this fraction of their start spread reach the full map.
const FULL_PINCH: f32 = 0.55;
const COLUMNS: usize = 2;
const GAP: f32 = 12.;
const PAD: f32 = 16.;
const CARD_ASPECT: f32 = 1.35;

#[derive(Default)]
pub(super) struct State {
    /// 0 = inside a session, 1 = full map.
    pub(super) progress: f32,
    target: f32,
    /// Accumulated pinch scale (1 at gesture start) and the progress then.
    pinch: Option<(f32, f32)>,
    last: Option<Instant>,
}

impl State {
    pub(super) fn visible(&self) -> bool {
        self.progress > 0.001 || self.pinch.is_some()
    }

    pub(super) fn open(&self) -> bool {
        self.target >= 1.
    }

    /// Feed one pinch event. Returns true when the map became / stopped
    /// being the committed destination (for haptics).
    pub(super) fn pinch(&mut self, ev: &PinchEvent) -> bool {
        match ev.phase {
            TouchPhase::Started => {
                self.pinch = Some((1., self.progress));
                false
            }
            TouchPhase::Moved => {
                let (scale, base) = self.pinch.get_or_insert((1., self.progress));
                *scale = (*scale * (1. + ev.delta)).clamp(0.2, 3.);
                let before = self.progress >= COMMIT;
                // Closing the fingers (scale < 1) zooms out to the map.
                let travel = (1. - *scale) / (1. - FULL_PINCH);
                self.progress = (*base + travel).clamp(0., 1.08);
                self.target = self.progress;
                before != (self.progress >= COMMIT)
            }
            TouchPhase::Ended | TouchPhase::Cancelled => {
                let Some((_, base)) = self.pinch.take() else { return false };
                // Settle in the direction the fingers moved, past a small threshold.
                let open = if base >= 1. { self.progress > 1. - COMMIT } else { self.progress >= COMMIT };
                self.target = if open { 1. } else { 0. };
                self.last = Some(Instant::now());
                false
            }
        }
    }

    pub(super) fn set_open(&mut self, open: bool) {
        self.target = if open { 1. } else { 0. };
        self.pinch = None;
        self.last = Some(Instant::now());
    }

    /// Ease toward the target. Returns true while still animating.
    pub(super) fn tick(&mut self) -> bool {
        if self.pinch.is_some() {
            self.last = None;
            return false;
        }
        let now = Instant::now();
        let dt = self.last.map_or(0.016, |l| now.duration_since(l).as_secs_f32().min(0.05));
        self.last = Some(now);
        let next = crate::gestures::approach(self.progress, self.target, dt, 14.);
        self.progress = if (next - self.target).abs() < 0.002 { self.target } else { next };
        self.progress != self.target
    }
}

/// Card rectangle (left, top, width, height) of grid slot `i` within `width`.
pub(super) fn cell(i: usize, width: f32) -> (f32, f32, f32, f32) {
    let w = ((width - PAD * 2. - GAP * (COLUMNS as f32 - 1.)) / COLUMNS as f32).max(40.);
    let h = w * CARD_ASPECT;
    let (col, row) = (i % COLUMNS, i / COLUMNS);
    (PAD + col as f32 * (w + GAP), PAD + row as f32 * (h + GAP), w, h)
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn status(session: &SessionState, t: &Theme) -> (Hsla, &'static str) {
    match &session.phase {
        ConnectionPhase::Connected if session.is_processing => (t.accent, "Working"),
        ConnectionPhase::Connected => (t.ok, "Idle"),
        ConnectionPhase::Failed { .. } => (t.error, "Failed"),
        ConnectionPhase::Disconnected => (t.text_muted, "Offline"),
        _ => (t.warning, "Connecting"),
    }
}

/// Newest few transcript lines, as a miniature of the conversation.
fn snippet(session: &SessionState) -> Vec<(bool, String)> {
    session
        .transcript
        .iter()
        .rev()
        .filter(|e| !e.text.trim().is_empty() && e.role != Role::System)
        .take(3)
        .map(|e| {
            let text: String = e.text.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(160).collect();
            (e.role == Role::User, text)
        })
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}

impl MobileApp {
    pub(super) fn session_map_pinch(&mut self, ev: &PinchEvent, cx: &mut Context<Self>) {
        if self.screen != Screen::Chat || self.account.is_none() {
            return;
        }
        if ev.phase == TouchPhase::Started {
            // A pinch owns the fingers: drop any half-started tab swipe.
            self.swipe.cancel(self.content_width(), Instant::now());
        }
        if self.map.pinch(ev) {
            haptics::play(Haptic::Selection);
        }
        if matches!(ev.phase, TouchPhase::Ended | TouchPhase::Cancelled) && self.map.open() {
            haptics::play(Haptic::Light);
        }
        cx.notify();
    }

    fn zoom_into(&mut self, id: u64, cx: &mut Context<Self>) {
        if id != self.active_tab {
            self.focus_tab(id, cx);
        }
        haptics::play(Haptic::Light);
        self.map.set_open(false);
        cx.notify();
    }

    /// The map layer drawn over the chat body, or None when fully zoomed in.
    pub(super) fn render_session_map(&mut self, t: &Theme, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.map.tick() {
            window.request_animation_frame();
        }
        if !self.map.visible() || self.screen != Screen::Chat {
            return None;
        }
        let p = self.map.progress.clamp(0., 1.);
        let over = (self.map.progress - 1.).max(0.);
        let width = self.content_width();
        let viewport = f32::from(window.viewport_size().height);
        let active_pos = self.tab_order.iter().position(|id| *id == self.active_tab).unwrap_or(0);
        let (_, active_top, _, active_h) = cell(active_pos, width);
        // Keep the focused card on screen when the canvas is taller than it.
        let scroll = (active_top + active_h / 2. - viewport * 0.4).max(0.) * p;
        let ids = self.tab_order.clone();
        let mut canvas = div().relative().w(px(width)).h(px(cell(ids.len(), width).1 + cell(0, width).3 + PAD));
        for (i, id) in ids.iter().copied().enumerate() {
            let Some(session) = self.tab_session(id) else { continue };
            let focused = id == self.active_tab;
            let (l, top, w, h) = cell(i, width);
            // Rubber band past full zoom: the grid tightens slightly.
            let shrink = 1. - over * 0.5;
            let (l, top, w, h) = (l + w * (1. - shrink) / 2., top - scroll + h * (1. - shrink) / 2., w * shrink, h * shrink);
            let (l, top, w, h, alpha) = if focused {
                (lerp(0., l, p), lerp(0., top, p), lerp(width, w, p), lerp(viewport, h, p), (p * 2.5).min(1.))
            } else {
                (l, top, w, h, p)
            };
            let (dot, state) = status(session, t);
            let title: SharedString = session.session_title.clone().unwrap_or_else(|| "New session".into()).into();
            let dir = session.working_dir.as_deref().map(|d| d.rsplit('/').find(|s| !s.is_empty()).unwrap_or(d).to_string());
            let lines = snippet(session);
            let empty = lines.is_empty();
            let card = div()
                .id(("map-card", id))
                .debug_selector(move || format!("map-card-{id}"))
                .a11y_button(format!("Open {title}"))
                .absolute()
                .left(px(l))
                .top(px(top))
                .w(px(w))
                .h(px(h))
                .opacity(alpha)
                .rounded(px(lerp(0., 18., p)))
                .border_1()
                .border_color(if focused { t.border_focus } else { t.border })
                .bg(if focused { t.raised } else { t.surface })
                .overflow_hidden()
                .p(px(12.))
                .flex()
                .flex_col()
                .gap(px(8.))
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, _, cx| this.zoom_into(id, cx)))
                .child(
                    div().flex().items_center().gap(px(6.))
                        .child(div().flex_none().size(px(8.)).rounded_full().bg(dot))
                        .child(label(state, t.text_muted, 10.5))
                        .when_some(dir, |d, dir| d.child(label(dir, t.text_muted, 10.5).min_w_0().truncate())),
                )
                .child(label(title, t.text_strong, 14.).line_clamp(2))
                .child(
                    div().flex_1().min_h_0().overflow_hidden().flex().flex_col().justify_end().gap(px(6.))
                        .when(empty, |d| d.child(label("No messages yet", t.text_muted, 11.)))
                        .children(lines.into_iter().map(|(user, text)| {
                            label(text, if user { t.text_strong } else { t.text_secondary }, 10.5)
                                .line_clamp(4)
                                .when(user, |d| d.px(px(6.)).py(px(4.)).rounded(px(6.)).bg(t.surface_elevated))
                        })),
                );
            canvas = canvas.child(card);
        }
        // "+" slot after the last session.
        let (l, top, w, h) = cell(ids.len(), width);
        canvas = canvas.child(
            div()
                .id("map-new-session")
                .debug_selector(|| "map-new-session".into())
                .a11y_button("New session")
                .absolute()
                .left(px(l)).top(px(top - scroll)).w(px(w)).h(px(h))
                .opacity(p)
                .rounded(px(18.))
                .border_1()
                .border_color(t.border)
                .flex().items_center().justify_center()
                .cursor_pointer()
                .child(label("+", t.text_muted, 28.))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.new_tab(cx);
                    this.map.set_open(false);
                })),
        );
        Some(
            div()
                .id("session-map")
                .debug_selector(|| "session-map".into())
                .absolute()
                .inset_0()
                .bg(t.background.opacity(p))
                .overflow_y_scroll()
                // The map owns its touches: nothing underneath reacts.
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(canvas)
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::Modifiers;

    fn ev(phase: TouchPhase, delta: f32) -> PinchEvent {
        PinchEvent { position: Default::default(), delta, modifiers: Modifiers::default(), phase }
    }

    #[test]
    fn pinch_out_commits_to_map_and_pinch_in_returns() {
        let mut s = State::default();
        s.pinch(&ev(TouchPhase::Started, 0.));
        for _ in 0..10 {
            s.pinch(&ev(TouchPhase::Moved, -0.05));
        }
        assert!(s.progress > COMMIT);
        s.pinch(&ev(TouchPhase::Ended, 0.));
        assert!(s.open());
        s.progress = 1.;
        s.pinch(&ev(TouchPhase::Started, 0.));
        for _ in 0..4 {
            s.pinch(&ev(TouchPhase::Moved, 0.06));
        }
        s.pinch(&ev(TouchPhase::Ended, 0.));
        assert!(!s.open());
    }

    #[test]
    fn small_pinch_springs_back() {
        let mut s = State::default();
        s.pinch(&ev(TouchPhase::Started, 0.));
        s.pinch(&ev(TouchPhase::Moved, -0.05));
        s.pinch(&ev(TouchPhase::Ended, 0.));
        assert!(!s.open());
    }

    #[test]
    fn grid_cells_fill_two_columns() {
        let (l0, t0, w, _) = cell(0, 390.);
        let (l1, t1, _, _) = cell(1, 390.);
        let (_, t2, _, _) = cell(2, 390.);
        assert_eq!((l0, t0), (PAD, PAD));
        assert!((l1 - (PAD + w + GAP)).abs() < 1e-3 && t1 == t0);
        assert!(t2 > t0);
    }
}
