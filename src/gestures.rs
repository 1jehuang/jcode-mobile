//! iOS-style gesture physics, independent of the view: left-edge swipe back,
//! bottom-sheet drag to dismiss (rubber band past the top), pull to refresh,
//! and long press. Fed by mouse events in the desktop preview and by GPUI's
//! touch drag / long press / pan-scroll events on the phone.
//!
//! Positions are window coordinates. Everything animates by exponential
//! easing toward a target so nothing ever jumps.

use std::time::Instant;

use gpui::{Bounds, Pixels, Point};

use crate::haptics::{self, Haptic};

/// Left-edge band that starts a swipe back.
pub const EDGE: f32 = 28.;
/// Movement that cancels a long press / decides a drag.
pub const SLOP: f32 = 8.;
pub const LONG_PRESS_SECS: f32 = 0.45;
/// Pull distance (after rubber banding) that triggers a reconnect.
pub const PULL_TRIGGER: f32 = 64.;
/// Where the spinner rests while refreshing.
pub const PULL_REST: f32 = 48.;
/// Height of the draggable grabber/title area at the top of a sheet.
pub const SHEET_GRAB: f32 = 64.;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Target {
    Message(u64),
    Tab(u64),
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Action {
    /// Edge swipe finished sliding the screen away: pop it.
    Back,
    /// Sheet finished sliding down: close it.
    DismissSheet,
    /// Pull released past the threshold.
    Refresh,
    /// Long press (or right click) on a target, at a window position.
    Menu(Target, Point<Pixels>),
}

/// What the view looks like this frame, as far as gestures care.
#[derive(Clone, Copy, Default, Debug)]
pub struct Scene {
    /// A pushed screen (settings) is showing: edge swipe pops it.
    pub pushed: bool,
    pub sheet_open: bool,
    /// Transcript scrolled to the very top (or empty): pull is allowed.
    pub at_top: bool,
    /// Long-press targets are live (chat screen, no overlay).
    pub menus: bool,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Kind {
    Edge,
    Sheet,
    Pull,
}

#[derive(Clone, Copy, Debug)]
enum Drag {
    Idle,
    /// Pointer down, not yet decided.
    Pending { start: Point<Pixels>, t0: Instant, kind: Option<Kind>, target: Option<Target>, touch: bool },
    Active { kind: Kind, start: Point<Pixels> },
    /// Gesture consumed (menu opened, drag finished); swallow the release.
    Swallow,
    /// Pointer moved without a gesture: let it be.
    Ignored,
}

/// Exponential approach; snaps when close so animations terminate.
pub fn approach(cur: f32, target: f32, dt: f32, rate: f32) -> f32 {
    let next = cur + (target - cur) * (1. - (-dt * rate).exp());
    if (target - next).abs() < 0.5 { target } else { next }
}

/// iOS-like rubber band: linear at first, asymptotic to `dim`.
pub fn rubber(x: f32, dim: f32) -> f32 {
    if x <= 0. {
        return 0.;
    }
    dim * (1. - 1. / (x / dim + 1.))
}

pub struct Gestures {
    pub scene: Scene,
    /// Screen bounds (window coordinates).
    pub screen: Bounds<Pixels>,
    pub sheet: Option<Bounds<Pixels>>,
    pub transcript: Option<Bounds<Pixels>>,
    pub hits: Vec<(Target, Bounds<Pixels>)>,
    drag: Drag,
    last_pos: Point<Pixels>,
    last_t: Instant,
    /// Horizontal velocity (px/s), smoothed.
    vx: f32,
    vy: f32,
    /// Pushed-screen / sheet horizontal offset, and where it is heading.
    pub edge_x: f32,
    edge_target: f32,
    pending_back: bool,
    /// Sheet vertical offset (0 = fully open; may go negative, rubber band).
    pub sheet_y: f32,
    sheet_target: f32,
    pending_dismiss: bool,
    /// Sheet still drawn while it slides out after closing.
    pub sheet_linger: bool,
    was_sheet_open: bool,
    was_pushed: bool,
    /// Raw pull distance while dragging; `pull` is what is shown.
    pull_raw: f32,
    pub pull: f32,
    pull_armed: bool,
    pub refreshing: Option<Instant>,
    wheel_pull_at: Option<Instant>,
    last_frame: Instant,
}

impl Default for Gestures {
    fn default() -> Self {
        let now = Instant::now();
        Self {
            scene: Scene::default(),
            screen: Bounds::default(),
            sheet: None,
            transcript: None,
            hits: Vec::new(),
            drag: Drag::Idle,
            last_pos: Point::default(),
            last_t: now,
            vx: 0.,
            vy: 0.,
            edge_x: 0.,
            edge_target: 0.,
            pending_back: false,
            sheet_y: 0.,
            sheet_target: 0.,
            pending_dismiss: false,
            sheet_linger: false,
            was_sheet_open: false,
            was_pushed: false,
            pull_raw: 0.,
            pull: 0.,
            pull_armed: false,
            refreshing: None,
            wheel_pull_at: None,
            last_frame: now,
        }
    }
}

fn f(p: Pixels) -> f32 {
    f32::from(p)
}

impl Gestures {
    fn width(&self) -> f32 {
        f(self.screen.size.width).max(1.)
    }

    fn sheet_height(&self) -> f32 {
        self.sheet.map_or(520., |b| f(b.size.height)).max(1.)
    }

    pub fn dragging(&self) -> bool {
        matches!(self.drag, Drag::Active { .. })
    }

    /// Fraction of the sheet still on screen (1 open, 0 gone).
    pub fn sheet_presence(&self) -> f32 {
        let h = self.sheet_height();
        (1. - self.sheet_y.max(0.) / h).clamp(0., 1.) * (1. - self.edge_x / self.width()).clamp(0., 1.)
    }

    /// Animated back via the "‹ back" button: same slide as a swipe.
    pub fn begin_back(&mut self) {
        self.edge_target = self.width();
        self.pending_back = true;
    }

    /// Animated close via the backdrop tap.
    pub fn begin_dismiss_sheet(&mut self) {
        self.sheet_target = self.sheet_height();
        self.pending_dismiss = true;
    }

    fn hit(&self, p: Point<Pixels>) -> Option<Target> {
        // Later recorded = painted on top.
        self.hits.iter().rev().find(|(_, b)| b.contains(&p)).map(|(t, _)| *t)
    }

    fn candidate(&self, p: Point<Pixels>) -> Option<Kind> {
        let s = self.scene;
        let near_edge = f(p.x - self.screen.origin.x) < EDGE;
        if (s.pushed || s.sheet_open) && near_edge {
            return Some(Kind::Edge);
        }
        if s.sheet_open {
            let grab = self.sheet.is_some_and(|b| {
                b.contains(&p) && f(p.y - b.origin.y) < SHEET_GRAB
            });
            return grab.then_some(Kind::Sheet);
        }
        if !s.pushed && s.at_top && self.refreshing.is_none() && self.transcript.is_some_and(|b| b.contains(&p)) {
            return Some(Kind::Pull);
        }
        None
    }

    fn track(&mut self, p: Point<Pixels>) {
        let now = Instant::now();
        let dt = now.duration_since(self.last_t).as_secs_f32().max(1e-3);
        // Time-weighted: a pause before release forgets the earlier fling.
        let k = 1. - (-dt / 0.04).exp();
        self.vx = self.vx * (1. - k) + k * f(p.x - self.last_pos.x) / dt;
        self.vy = self.vy * (1. - k) + k * f(p.y - self.last_pos.y) / dt;
        self.last_pos = p;
        self.last_t = now;
    }

    /// Mouse button / touch down. Returns true if a menu was requested
    /// immediately (right click).
    pub fn down(&mut self, p: Point<Pixels>, touch: bool) {
        self.last_pos = p;
        self.last_t = Instant::now();
        self.vx = 0.;
        self.vy = 0.;
        let target = if self.scene.menus { self.hit(p) } else { None };
        let kind = self.candidate(p);
        self.drag = Drag::Pending { start: p, t0: Instant::now(), kind, target, touch };
    }

    /// Whether a touch drag starting here should be claimed (stealing it
    /// from scrolling). Pulls stay with the scroll view on touch.
    pub fn wants_touch_drag(&self, p: Point<Pixels>) -> bool {
        matches!(self.candidate(p), Some(Kind::Edge | Kind::Sheet))
    }

    pub fn right_click(&mut self, p: Point<Pixels>) -> Option<Action> {
        if !self.scene.menus {
            return None;
        }
        let t = self.hit(p)?;
        self.drag = Drag::Swallow;
        haptics::play(Haptic::Medium);
        Some(Action::Menu(t, p))
    }

    /// Touch long press recognized by GPUI.
    pub fn long_press(&mut self, p: Point<Pixels>) -> Option<Action> {
        if !self.scene.menus || self.dragging() {
            return None;
        }
        let t = self.hit(p)?;
        self.drag = Drag::Swallow;
        haptics::play(Haptic::Medium);
        Some(Action::Menu(t, p))
    }

    pub fn moved(&mut self, p: Point<Pixels>) {
        self.track(p);
        match self.drag {
            Drag::Pending { start, kind, .. } => {
                let d = p - start;
                let (dx, dy) = (f(d.x), f(d.y));
                if dx.abs().max(dy.abs()) < SLOP {
                    return;
                }
                let go = match kind {
                    Some(Kind::Edge) => dx > dy.abs(),
                    Some(Kind::Sheet) => dy.abs() > dx.abs(),
                    Some(Kind::Pull) => dy > dx.abs(),
                    None => false,
                };
                self.drag = match kind {
                    Some(k) if go => Drag::Active { kind: k, start },
                    _ => Drag::Ignored,
                };
                self.moved(p);
            }
            Drag::Active { kind, start } => {
                let d = p - start;
                match kind {
                    Kind::Edge => {
                        let x = f(d.x).max(0.);
                        self.edge_x = x;
                        self.edge_target = x;
                    }
                    Kind::Sheet => {
                        let y = f(d.y);
                        self.sheet_y = if y >= 0. { y } else { -rubber(-y, 40.) };
                        self.sheet_target = self.sheet_y;
                    }
                    Kind::Pull => self.set_pull(f(d.y).max(0.)),
                }
            }
            _ => {}
        }
    }

    fn set_pull(&mut self, raw: f32) {
        self.pull_raw = raw;
        self.pull = rubber(raw, 130.);
        let armed = self.pull >= PULL_TRIGGER;
        if armed && !self.pull_armed {
            haptics::play(Haptic::Light);
        }
        self.pull_armed = armed;
    }

    /// Abandon pointer recognition without treating cancellation as a release.
    /// Existing refresh work stays alive, but an armed pull cannot start new work.
    pub fn cancel(&mut self) {
        self.drag = Drag::Idle;
        self.vx = 0.;
        self.vy = 0.;
        self.last_t = Instant::now();
        self.edge_target = 0.;
        self.pending_back = false;
        self.sheet_target = 0.;
        self.pending_dismiss = false;
        self.sheet_linger = false;
        self.pull_raw = 0.;
        self.pull_armed = false;
        self.wheel_pull_at = None;
        // Keep visible offsets intact so tick springs them home instead of jumping.
    }

    /// Pointer released. Returns (action, swallow the click).
    pub fn up(&mut self) -> (Option<Action>, bool) {
        let drag = std::mem::replace(&mut self.drag, Drag::Idle);
        match drag {
            Drag::Active { kind, .. } => (self.release(kind), true),
            Drag::Swallow => (None, true),
            _ => (None, false),
        }
    }

    fn release(&mut self, kind: Kind) -> Option<Action> {
        match kind {
            Kind::Edge => {
                let w = self.width();
                if self.edge_x > w * 0.35 || self.vx > 500. {
                    self.edge_target = w;
                    self.pending_back = true;
                    haptics::play(Haptic::Light);
                } else {
                    self.edge_target = 0.;
                }
                None
            }
            Kind::Sheet => {
                if self.sheet_y > 110. || self.vy > 700. {
                    self.begin_dismiss_sheet();
                } else {
                    self.sheet_target = 0.;
                }
                None
            }
            Kind::Pull => self.release_pull(),
        }
    }

    fn release_pull(&mut self) -> Option<Action> {
        self.wheel_pull_at = None;
        self.pull_raw = 0.;
        let fire = self.pull_armed;
        self.pull_armed = false;
        if fire {
            self.refreshing = Some(Instant::now());
            haptics::play(Haptic::Medium);
            Some(Action::Refresh)
        } else {
            None
        }
    }

    /// Scroll wheel / touch pan. `dy > 0` pulls content down. Returns
    /// (action, consumed).
    pub fn wheel(&mut self, p: Point<Pixels>, dy: f32, ended: bool) -> (Option<Action>, bool) {
        let pulling = self.wheel_pull_at.is_some();
        if !pulling {
            let ok = dy > 0. && matches!(self.candidate(p), Some(Kind::Pull)) && !self.dragging();
            if !ok {
                return (None, false);
            }
        }
        self.wheel_pull_at = Some(Instant::now());
        self.set_pull((self.pull_raw + dy).max(0.));
        if ended || self.pull_raw <= 0. {
            return (self.release_pull(), true);
        }
        (None, true)
    }

    /// Mark a refresh finished (connection is back); the spinner eases away.
    pub fn refresh_done(&mut self) {
        if self.refreshing.is_some_and(|t| t.elapsed().as_secs_f32() > 0.7) {
            self.refreshing = None;
        }
    }

    /// Advance animations. Returns actions and whether to keep animating.
    pub fn tick(&mut self, scene: Scene) -> (Vec<Action>, bool) {
        let now = Instant::now();
        let dt = now.duration_since(self.last_frame).as_secs_f32().min(0.05);
        self.last_frame = now;
        let mut out = Vec::new();
        // Appear transitions: slide new screens/sheets in.
        if scene.pushed && !self.was_pushed {
            self.edge_x = self.width();
            self.edge_target = 0.;
            self.pending_back = false;
        }
        if scene.sheet_open && !self.was_sheet_open {
            self.sheet_y = self.sheet_height();
            self.sheet_target = 0.;
            self.edge_x = 0.;
            self.edge_target = 0.;
            self.pending_dismiss = false;
            self.sheet_linger = false;
        }
        if !scene.sheet_open && self.was_sheet_open && !self.pending_dismiss {
            // Closed from elsewhere (picked a model): slide out.
            if self.sheet_y < self.sheet_height() - 1. && self.edge_x < self.width() - 1. {
                self.sheet_linger = true;
                self.sheet_target = self.sheet_height();
            }
        }
        self.was_pushed = scene.pushed;
        self.was_sheet_open = scene.sheet_open;
        self.scene = scene;

        if let Drag::Pending { start, t0, target: Some(t), touch: false, .. } = self.drag
            && t0.elapsed().as_secs_f32() >= LONG_PRESS_SECS
            && scene.menus
        {
            self.drag = Drag::Swallow;
            haptics::play(Haptic::Medium);
            out.push(Action::Menu(t, start));
        }
        let pending_press = matches!(self.drag, Drag::Pending { target: Some(_), touch: false, .. });

        let active = self.dragging();
        let mut anim = active || pending_press;
        if !active {
            let w = self.width();
            self.edge_x = approach(self.edge_x, self.edge_target, dt, 16.);
            if self.edge_x != self.edge_target {
                anim = true;
            } else if self.pending_back && self.edge_x >= w {
                self.pending_back = false;
                self.edge_x = 0.;
                self.edge_target = 0.;
                if scene.sheet_open || self.sheet_linger {
                    self.sheet_linger = false;
                    self.sheet_y = self.sheet_height();
                    out.push(Action::DismissSheet);
                } else {
                    out.push(Action::Back);
                }
            }
            let h = self.sheet_height();
            self.sheet_y = approach(self.sheet_y, self.sheet_target, dt, 14.);
            if self.sheet_y != self.sheet_target {
                anim = true;
            } else if self.sheet_y >= h {
                if self.pending_dismiss {
                    self.pending_dismiss = false;
                    out.push(Action::DismissSheet);
                }
                self.sheet_linger = false;
            }
            // Wheel pulls have no release on most desktops: idle releases.
            if let Some(at) = self.wheel_pull_at {
                if at.elapsed().as_secs_f32() > 0.18 {
                    out.extend(self.release_pull());
                } else {
                    anim = true;
                }
            }
            if self.wheel_pull_at.is_none() {
                let rest = if self.refreshing.is_some() { PULL_REST } else { 0. };
                self.pull = approach(self.pull, rest, dt, 12.);
                if self.pull != rest {
                    anim = true;
                }
            }
        }
        (out, anim)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{point, px, size};

    fn g() -> Gestures {
        let mut g = Gestures::default();
        g.screen = Bounds::new(point(px(0.), px(0.)), size(px(390.), px(844.)));
        g
    }

    /// Advance real animation logic deterministically, without sleeping.
    fn assert_cancel_unwinds(g: &mut Gestures, scene: Scene) {
        assert!(!g.dragging());
        assert_eq!(g.up(), (None, false));
        assert_eq!((g.vx, g.vy), (0., 0.));
        assert!(!g.pending_back && !g.pending_dismiss && !g.pull_armed);
        assert_eq!(g.pull_raw, 0.);
        assert!(g.wheel_pull_at.is_none());
        for _ in 0..200 {
            g.last_frame = Instant::now() - std::time::Duration::from_millis(50);
            assert!(g.tick(scene).0.is_empty(), "cancel must not emit an action");
        }
        assert_eq!(g.edge_x, 0.);
        assert_eq!(g.sheet_y, 0.);
        assert_eq!(g.pull, if g.refreshing.is_some() { PULL_REST } else { 0. });
    }

    #[test]
    fn cancelled_edge_swipe_unwinds_without_back() {
        let mut g = g();
        let scene = Scene { pushed: true, ..Default::default() };
        g.tick(scene);
        g.edge_x = 0.;
        g.down(point(px(5.), px(400.)), true);
        g.moved(point(px(300.), px(400.)));
        assert!(g.dragging());
        assert!(g.edge_x > g.width() * 0.35);
        g.cancel();
        assert_eq!(g.edge_x, 295., "cancellation must spring, not jump");
        assert_cancel_unwinds(&mut g, scene);
    }

    #[test]
    fn cancelled_sheet_drag_unwinds_without_dismiss() {
        let mut g = g();
        let scene = Scene { sheet_open: true, ..Default::default() };
        g.sheet = Some(Bounds::new(point(px(0.), px(300.)), size(px(390.), px(500.))));
        g.tick(scene);
        g.sheet_y = 0.;
        g.down(point(px(200.), px(310.)), true);
        g.moved(point(px(200.), px(610.)));
        assert!(g.dragging());
        assert!(g.sheet_y > 110.);
        g.cancel();
        assert_eq!(g.sheet_y, 300.);
        assert_cancel_unwinds(&mut g, scene);
    }

    #[test]
    fn cancelled_armed_pull_unwinds_without_refresh() {
        let mut g = g();
        let scene = Scene { at_top: true, ..Default::default() };
        g.transcript = Some(Bounds::new(point(px(0.), px(100.)), size(px(390.), px(600.))));
        g.tick(scene);
        g.down(point(px(100.), px(200.)), true);
        g.moved(point(px(100.), px(420.)));
        assert!(g.pull_armed);
        let pull = g.pull;
        g.cancel();
        assert_eq!(g.pull, pull);
        assert_cancel_unwinds(&mut g, scene);
        assert!(g.refreshing.is_none());
    }

    #[test]
    fn cancelled_pending_press_does_not_open_menu() {
        let mut g = g();
        let scene = Scene { menus: true, ..Default::default() };
        g.tick(scene);
        g.hits.push((Target::Message(7), Bounds::new(point(px(0.), px(0.)), size(px(100.), px(100.)))));
        g.down(point(px(10.), px(10.)), false);
        if let Drag::Pending { t0, .. } = &mut g.drag {
            *t0 = Instant::now() - std::time::Duration::from_secs(1);
        } else {
            panic!("expected pending press");
        }
        g.cancel();
        assert_cancel_unwinds(&mut g, scene);
    }

    #[test]
    fn cancelled_wheel_pull_clears_idle_release_and_preserves_existing_refresh() {
        let mut g = g();
        let scene = Scene { at_top: true, ..Default::default() };
        g.transcript = Some(Bounds::new(point(px(0.), px(100.)), size(px(390.), px(600.))));
        g.tick(scene);
        assert_eq!(g.wheel(point(px(100.), px(200.)), 220., false), (None, true));
        assert!(g.pull_armed && g.wheel_pull_at.is_some());
        g.cancel();
        assert_cancel_unwinds(&mut g, scene);
        assert!(g.refreshing.is_none());
        let refresh = Instant::now();
        g.refreshing = Some(refresh);
        g.cancel();
        assert_cancel_unwinds(&mut g, scene);
        assert_eq!(g.refreshing, Some(refresh));
    }

    #[test]
    fn rubber_is_bounded_and_monotonic() {
        assert_eq!(rubber(0., 100.), 0.);
        assert!(rubber(50., 100.) < 50.);
        assert!(rubber(1000., 100.) < 100.);
        assert!(rubber(200., 100.) > rubber(100., 100.));
    }

    #[test]
    fn edge_swipe_commits_past_a_third() {
        let mut g = g();
        let scene = Scene { pushed: true, ..Default::default() };
        g.tick(scene);
        g.edge_x = 0.;
        g.edge_target = 0.;
        g.down(point(px(5.), px(400.)), false);
        g.moved(point(px(20.), px(402.)));
        g.moved(point(px(200.), px(405.)));
        assert!(g.dragging());
        assert_eq!(g.edge_x, 195.);
        let (_, swallow) = g.up();
        assert!(swallow);
        let mut back = false;
        for _ in 0..200 {
            std::thread::sleep(std::time::Duration::from_millis(2));
            back |= g.tick(scene).0.contains(&Action::Back);
        }
        assert!(back);
    }

    #[test]
    fn short_edge_swipe_springs_back() {
        let mut g = g();
        let scene = Scene { pushed: true, ..Default::default() };
        g.tick(scene);
        g.edge_x = 0.;
        g.edge_target = 0.;
        g.down(point(px(5.), px(400.)), false);
        g.moved(point(px(60.), px(400.)));
        std::thread::sleep(std::time::Duration::from_millis(200));
        g.moved(point(px(61.), px(400.)));
        g.up();
        for _ in 0..200 {
            std::thread::sleep(std::time::Duration::from_millis(2));
            assert!(!g.tick(scene).0.contains(&Action::Back));
        }
        assert_eq!(g.edge_x, 0.);
    }

    #[test]
    fn pull_past_threshold_refreshes() {
        let mut g = g();
        g.transcript = Some(Bounds::new(point(px(0.), px(100.)), size(px(390.), px(600.))));
        g.tick(Scene { at_top: true, menus: true, ..Default::default() });
        g.down(point(px(100.), px(200.)), false);
        g.moved(point(px(100.), px(220.)));
        g.moved(point(px(100.), px(420.)));
        assert!(g.pull >= PULL_TRIGGER);
        assert_eq!(g.up().0, Some(Action::Refresh));
    }

    #[test]
    fn upward_sheet_drag_rubber_bands() {
        let mut g = g();
        g.sheet = Some(Bounds::new(point(px(0.), px(300.)), size(px(390.), px(500.))));
        g.tick(Scene { sheet_open: true, ..Default::default() });
        g.sheet_y = 0.;
        g.sheet_target = 0.;
        g.down(point(px(200.), px(310.)), false);
        g.moved(point(px(200.), px(290.)));
        g.moved(point(px(200.), px(110.)));
        assert!(g.sheet_y < 0. && g.sheet_y > -40.);
    }

    #[test]
    fn right_click_opens_menu_on_hit() {
        let mut g = g();
        g.tick(Scene { menus: true, ..Default::default() });
        g.hits.push((Target::Message(7), Bounds::new(point(px(0.), px(0.)), size(px(100.), px(100.)))));
        assert!(matches!(g.right_click(point(px(10.), px(10.))), Some(Action::Menu(Target::Message(7), _))));
        assert_eq!(g.right_click(point(px(200.), px(200.))), None);
    }
}
