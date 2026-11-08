//! Pinch out to zoom from the open session to a minimap of every live session,
//! laid out like jcode desktop: four horizontal strips of panels. The focused
//! session shrinks from full screen into its slot on its strip while the
//! others fade in. Drag to pan the map, tap a panel (or pinch back in) to zoom
//! into it, and tap a strip's + to start a session on that strip.
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
/// Strips, as on desktop.
pub(super) const STRIPS: usize = 4;
/// Mini panel size relative to the screen.
const SCALE: f32 = 0.2;
const GAP: f32 = 8.;
const ROW_GAP: f32 = 22.;
/// Canvas margin; also room for each strip's label.
const PAD: f32 = 24.;
const ADD_W: f32 = 44.;

#[derive(Default)]
pub(super) struct State {
    /// 0 = inside a session, 1 = full map.
    pub(super) progress: f32,
    target: f32,
    /// Accumulated pinch scale (1 at gesture start) and the progress then.
    pinch: Option<(f32, f32)>,
    last: Option<Instant>,
    /// Strip of each tab. Unlisted tabs live on strip 0.
    rows: std::collections::HashMap<u64, usize>,
    /// Map camera: canvas position of the screen's top-left corner.
    camera: (f32, f32),
}

/// Map geometry in canvas pixels.
#[derive(Debug, PartialEq)]
pub(super) struct Layout {
    /// (tab id, left, top) of each mini panel.
    pub panels: Vec<(u64, f32, f32)>,
    /// (strip, left, top) of each strip's + tile.
    pub adds: Vec<(usize, f32, f32)>,
    pub panel: (f32, f32),
    pub size: (f32, f32),
}

/// Lay strips out top to bottom, each a left-to-right run of panels in tab
/// order followed by its + tile.
pub(super) fn layout(order: &[u64], row_of: impl Fn(u64) -> usize, screen: (f32, f32)) -> Layout {
    // Every strip fits the screen height; panels keep the phone's shape.
    let fit_h = (screen.1 - PAD * 2. - ROW_GAP * (STRIPS as f32 - 1.)) / STRIPS as f32;
    let ph = (screen.1 * SCALE).min(fit_h).max(40.);
    let panel = (ph * screen.0 / screen.1, ph);
    let mut l = Layout { panels: vec![], adds: vec![], panel, size: (0., 0.) };
    for row in 0..STRIPS {
        let top = PAD + row as f32 * (panel.1 + ROW_GAP);
        let mut x = PAD;
        for &id in order.iter().filter(|id| row_of(**id).min(STRIPS - 1) == row) {
            l.panels.push((id, x, top));
            x += panel.0 + GAP;
        }
        l.adds.push((row, x, top));
        l.size.0 = l.size.0.max(x + ADD_W + PAD);
    }
    l.size.1 = PAD * 2. + STRIPS as f32 * panel.1 + (STRIPS as f32 - 1.) * ROW_GAP;
    l
}

/// Keep the camera inside the canvas (centered when the canvas is smaller).
fn clamp_camera(c: (f32, f32), size: (f32, f32), screen: (f32, f32)) -> (f32, f32) {
    let axis = |c: f32, size: f32, screen: f32| if size <= screen { (size - screen) / 2. } else { c.clamp(0., size - screen) };
    (axis(c.0, size.0, screen.0), axis(c.1, size.1, screen.1))
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

    pub(super) fn row(&self, id: u64) -> usize {
        self.rows.get(&id).copied().unwrap_or(0)
    }

    pub(super) fn set_row(&mut self, id: u64, row: usize) {
        self.rows.insert(id, row.min(STRIPS - 1));
    }

    /// Point the camera so `rect` (left, top, w, h) sits mid-screen.
    fn center_on(&mut self, rect: (f32, f32, f32, f32), size: (f32, f32), screen: (f32, f32)) {
        let c = (rect.0 + rect.2 / 2. - screen.0 / 2., rect.1 + rect.3 / 2. - screen.1 / 2.);
        self.camera = clamp_camera(c, size, screen);
    }

    fn pan(&mut self, dx: f32, dy: f32, size: (f32, f32), screen: (f32, f32)) {
        self.camera = clamp_camera((self.camera.0 - dx, self.camera.1 - dy), size, screen);
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

/// Dev hook (JCODE_MOBILE_MAP): believable titles and transcripts for screenshots.
pub(super) fn fill_demo(s: &mut SessionState, i: usize) {
    const DEMO: [(&str, &str, &str); 6] = [
        ("Fix login redirect", "the oauth callback loops back to /login", "Found it: the state cookie is SameSite=Strict, so the redirect drops it. Switching to Lax."),
        ("Pinch session map", "make the map look like desktop strips", "Strips now fit the screen height and each has its own + tile."),
        ("Flaky CI on main", "why does streaming_tests fail sometimes", "It races with the other test binaries. Passes alone; adding a serial guard."),
        ("Release notes 1.8", "draft notes for the testflight build", "Drafted five bullets covering voice, the session map and offline drafts."),
        ("Bench edit loop", "run the edit-loop bench again", "Median check went from 2.9s to 1.4s after the cache fix."),
        ("Refactor theme", "move colors into one palette", "Moved 41 colors into Palette and removed the duplicates."),
    ];
    let (title, user, reply) = DEMO[i % DEMO.len()];
    s.session_title = Some(title.into());
    let entry = |id, role, text: &str| jcode_mobile_core::session::TranscriptEntry {
        id, role, text: text.into(), reasoning: String::new(), tool_calls: vec![], is_streaming: false, is_queued: false,
    };
    s.transcript = vec![entry(1, Role::User, user), entry(2, Role::Assistant, reply)];
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
            if !self.map.visible() {
                let screen = self.map_screen();
                let l = self.map_layout(screen);
                if let Some(&(_, x, y)) = l.panels.iter().find(|p| p.0 == self.active_tab) {
                    self.map.center_on((x, y, l.panel.0, l.panel.1), l.size, screen);
                }
            }
        }
        if self.map.pinch(ev) {
            haptics::play(Haptic::Selection);
        }
        if matches!(ev.phase, TouchPhase::Ended | TouchPhase::Cancelled) && self.map.open() {
            haptics::play(Haptic::Light);
        }
        cx.notify();
    }

    fn map_screen(&self) -> (f32, f32) {
        (self.content_width(), self.map_height)
    }

    fn map_layout(&self, screen: (f32, f32)) -> Layout {
        layout(&self.tab_order, |id| self.map.row(id), screen)
    }

    fn zoom_into(&mut self, id: u64, cx: &mut Context<Self>) {
        if id != self.active_tab {
            self.focus_tab(id, cx);
        }
        haptics::play(Haptic::Light);
        self.map.set_open(false);
        cx.notify();
    }

    fn add_on_strip(&mut self, row: usize, cx: &mut Context<Self>) {
        self.new_tab(cx);
        self.map.set_row(self.active_tab, row);
        self.map.set_open(false);
        cx.notify();
    }

    /// The map layer drawn over the chat body, or None when fully zoomed in.
    pub(super) fn render_session_map(&mut self, t: &Theme, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let full = self.device.map_or_else(|| f32::from(window.viewport_size().height), |(_, h)| h);
        self.map_height = (full - self.safe.top - self.safe.bottom).max(200.);
        if self.map.tick() {
            window.request_animation_frame();
        }
        if !self.map.visible() || self.screen != Screen::Chat {
            return None;
        }
        if !cfg!(test) && std::env::var_os("JCODE_MOBILE_MAP").is_some() {
            let ids = self.tab_order.clone();
            for (i, id) in ids.into_iter().enumerate() {
                let s = if id == self.active_tab { Some(&mut self.session) } else { self.parked.iter_mut().find(|p| p.id == id).map(|p| &mut p.session) };
                if let Some(s) = s.filter(|s| s.transcript.is_empty()) {
                    fill_demo(s, i);
                }
            }
        }
        let p = self.map.progress.clamp(0., 1.);
        let over = (self.map.progress - 1.).max(0.);
        let screen = self.map_screen();
        let l = self.map_layout(screen);
        self.map.camera = clamp_camera(self.map.camera, l.size, screen);
        // Rubber band past full zoom: the whole map tightens toward the middle.
        let k = 1. - over * 0.6;
        let (cam_x, cam_y) = self.map.camera;
        let at = |x: f32, y: f32| {
            let (sx, sy) = (x - cam_x, y - cam_y);
            (screen.0 / 2. + (sx - screen.0 / 2.) * k, screen.1 / 2. + (sy - screen.1 / 2.) * k)
        };
        let (pw, ph) = (l.panel.0 * k, l.panel.1 * k);
        let mut layer = div()
            .id("session-map")
            .debug_selector(|| "session-map".into())
            .absolute()
            .inset_0()
            .overflow_hidden()
            .bg(t.background.opacity(p))
            .occlude()
            // The map owns its touches: nothing underneath reacts.
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            // One-finger drag (or trackpad scroll) pans the camera.
            .on_scroll_wheel(cx.listener(move |this, ev: &gpui::ScrollWheelEvent, window, cx| {
                let d = ev.delta.pixel_delta(window.line_height());
                let screen = this.map_screen();
                let size = this.map_layout(screen).size;
                this.map.pan(f32::from(d.x), f32::from(d.y), size, screen);
                cx.stop_propagation();
                cx.notify();
            }));
        // Strip tracks and labels behind the panels.
        for row in 0..STRIPS {
            let (_, _, top) = l.adds[row];
            let (x, y) = at(PAD - 8., top - 8.);
            let (x2, _) = at(l.size.0 - PAD + 8., top);
            layer = layer
                .child(div().absolute().left(px(x)).top(px(y)).w(px((x2 - x).max(0.))).h(px(ph + 16. * k))
                    .rounded(px(14.)).bg(t.raised).opacity(p * 0.6))
                .child(label(format!("{}", row + 1), t.text_muted, 10.).absolute().left(px(x - 14.)).top(px(y + 4.)).opacity(p));
        }
        for &(id, x, y) in &l.panels {
            let Some(session) = self.tab_session(id) else { continue };
            let focused = id == self.active_tab;
            let (mx, my) = at(x, y);
            let (left, top, w, h, alpha) = if focused {
                (lerp(0., mx, p), lerp(0., my, p), lerp(screen.0, pw, p), lerp(screen.1, ph, p), (p * 2.5).min(1.))
            } else {
                (mx, my, pw, ph, p)
            };
            let (dot, state) = status(session, t);
            let title: SharedString = session.session_title.clone().unwrap_or_else(|| "New session".into()).into();
            let lines = snippet(session);
            layer = layer.child(
                div()
                    .id(("map-card", id))
                    .debug_selector(move || format!("map-card-{id}"))
                    .a11y_button(format!("Open {title}, {state}"))
                    .absolute()
                    .left(px(left)).top(px(top)).w(px(w)).h(px(h))
                    .opacity(alpha)
                    .rounded(px(lerp(0., 10., p)))
                    .border_1()
                    .border_color(if focused { t.accent } else { t.border })
                    .bg(if focused { t.raised } else { t.surface })
                    .overflow_hidden()
                    .p(px(7.))
                    .flex().flex_col().gap(px(4.))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| this.zoom_into(id, cx)))
                    .child(div().flex().items_center().gap(px(4.))
                        .child(div().flex_none().size(px(6.)).rounded_full().bg(dot))
                        .child(label(title, t.text_strong, 10.).w(px((w - 24.).max(0.))).truncate()))
                    .child(div().flex_1().min_h_0().overflow_hidden().flex().flex_col().justify_end().gap(px(3.))
                        .children(lines.into_iter().map(|(user, text)| {
                            label(text, if user { t.text_strong } else { t.text_secondary }, 7.5)
                                .w(px((w - 14.).max(0.)))
                                .line_clamp(if user { 2 } else { 4 })
                                .when(user, |d| d.px(px(3.)).py(px(2.)).rounded(px(3.)).bg(t.surface_elevated))
                        }))),
            );
        }
        for &(row, x, y) in &l.adds {
            let (x, y) = at(x, y);
            layer = layer.child(
                div()
                    .id(("map-add", row))
                    .debug_selector(move || format!("map-add-{row}"))
                    .a11y_button(format!("New session on strip {}", row + 1))
                    .absolute()
                    .left(px(x)).top(px(y)).w(px(ADD_W * k)).h(px(ph))
                    .opacity(p)
                    .rounded(px(10.))
                    .border_1().border_color(t.border)
                    .flex().items_center().justify_center()
                    .cursor_pointer()
                    .child(label("+", t.text_muted, 20.))
                    .on_click(cx.listener(move |this, _, _, cx| this.add_on_strip(row, cx))),
            );
        }
        // The overview only helps when part of the map is off screen.
        let overflows = l.size.0 > screen.0 + 1. || l.size.1 > screen.1 + 1.;
        Some(layer.when(overflows, |d| d.child(self.render_map_inset(&l, screen, p, t))).into_any_element())
    }

    /// Corner overview: the whole canvas in miniature with a lens for what
    /// the screen shows, like desktop's minimap.
    fn render_map_inset(&self, l: &Layout, screen: (f32, f32), p: f32, t: &Theme) -> AnyElement {
        const W: f32 = 96.;
        let s = (W / l.size.0).min(64. / l.size.1);
        let (w, h) = (l.size.0 * s, l.size.1 * s);
        let (cx_, cy_) = self.map.camera;
        let mut inset = div()
            .absolute().right(px(12.)).bottom(px(12.)).w(px(w + 8.)).h(px(h + 8.)).p(px(4.))
            .rounded(px(8.)).bg(t.raised).border_1().border_color(t.border).opacity(p)
            .child(div().relative().size_full()
                .children(l.panels.iter().map(|&(id, x, y)| {
                    div().absolute().left(px(x * s)).top(px(y * s)).w(px(l.panel.0 * s)).h(px(l.panel.1 * s))
                        .rounded(px(1.5)).bg(if id == self.active_tab { t.accent } else { t.text_muted.opacity(0.6) })
                }))
                .child(div().absolute()
                    .left(px(cx_.max(0.) * s)).top(px(cy_.max(0.) * s))
                    .w(px((screen.0 * s).min(w))).h(px((screen.1 * s).min(h)))
                    .rounded(px(2.)).border_1().border_color(t.text_strong.opacity(0.7))));
        inset = inset.debug_selector(|| "map-inset".into());
        inset.into_any_element()
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
    fn strips_hold_their_sessions_with_an_add_tile_each() {
        let rows = std::collections::HashMap::from([(1u64, 0usize), (2, 2), (3, 0)]);
        let l = layout(&[1, 2, 3], |id| rows[&id], (390., 844.));
        assert_eq!(l.adds.len(), STRIPS);
        let at = |id| l.panels.iter().find(|p| p.0 == id).map(|p| (p.1, p.2)).unwrap();
        let ((x1, y1), (x3, y3), (_, y2)) = (at(1), at(3), at(2));
        assert_eq!(y1, y3, "same strip, same row");
        assert!(x3 > x1, "tab order runs left to right");
        assert!(y2 > y1, "strip 3 sits below strip 1");
        assert_eq!(l.adds[1].1, PAD, "empty strip shows only its +");
        assert!(l.adds[0].1 > x3);
    }

    #[test]
    fn camera_pans_within_the_canvas() {
        let mut s = State::default();
        let (size, screen) = ((900., 1200.), (390., 844.));
        s.pan(-10_000., -10_000., size, screen);
        assert_eq!(s.camera, (510., 356.));
        s.pan(10_000., 10_000., size, screen);
        assert_eq!(s.camera, (0., 0.));
        // A canvas narrower than the screen stays centered.
        s.pan(50., 0., (300., 1200.), screen);
        assert_eq!(s.camera.0, -45.);
    }
}
