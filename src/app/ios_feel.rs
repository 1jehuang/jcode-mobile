//! iOS feel for the root view: left-edge swipe back from Settings (and the
//! model sheet), sheet drag-to-dismiss with rubber band, pull to reconnect on
//! the transcript, long-press context menus on messages (copy, retry, share)
//! and tabs (rename, duplicate in the tab sheet), and the haptics that go
//! with them. Physics live in `crate::gestures`; this file wires them to
//! GPUI events (mouse in the Linux preview, touch drag / long press / pan on
//! the phone) and renders the animated chrome.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use gpui::{
    AnyElement, Bounds, Context, DispatchPhase, Entity, LongPressEvent, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, Pixels, Point, ScrollWheelEvent, TouchDragEvent, TouchPhase, Window,
    canvas, div, prelude::*, px,
};
use jcode_mobile_core::{ConnectionPhase, LocalIntent, Request, Role};

use super::{MobileApp, Screen, label};
use crate::gestures::{Action, Gestures, Scene, Target};
use crate::haptics::{self, Haptic};
use crate::text_input::{TextInput, TextInputEvent};
use crate::theme::Theme;

#[cfg(test)]
#[path = "voice_controls_tests.rs"]
mod voice_controls_tests;

#[cfg(test)]
#[path = "voice_motion_tests.rs"]
mod voice_motion_tests;

#[cfg(test)]
#[path = "overlay_coverage.rs"]
mod overlay_coverage;

#[derive(Default)]
struct Recorded {
    screen: Option<Bounds<Pixels>>,
    sheet: Option<Bounds<Pixels>>,
    transcript: Option<Bounds<Pixels>>,
    hits: Vec<(Target, Bounds<Pixels>)>,
}

struct MsgMenu {
    entry: u64,
    at: Point<Pixels>,
    closing: bool,
}

#[derive(Default)]
pub(super) struct Feel {
    pub g: Gestures,
    pub voice_gesture: crate::voice_gesture::VoiceGesture,
    pub voice_wheel_at: Option<Instant>,
    voice_phased: bool,
    pub(super) voice_pan_ended: bool,
    rec: Rc<RefCell<Recorded>>,
    menu: Option<MsgMenu>,
    menu_anim: f32,
    rename: Option<(u64, Entity<TextInput>)>,
    selection: Option<Entity<TextInput>>,
    focus_rename: bool,
    blur_rename: bool,
    toast: Option<(String, Instant)>,
    last: Option<Instant>,
    voice_frame_at: Option<Instant>,
    voice_rest: f32,
}

fn f(p: Pixels) -> f32 {
    f32::from(p)
}

/// Invisible absolute layer that records its bounds each frame.
fn recorder(rec: Rc<RefCell<Recorded>>, put: fn(&mut Recorded, Bounds<Pixels>)) -> impl IntoElement {
    canvas(move |b, _, _| put(&mut rec.borrow_mut(), b), |_, _, _, _| {}).absolute().inset_0()
}

impl MobileApp {
    fn apply_voice_gesture(&mut self, actions: Vec<crate::voice_gesture::Action>, cx: &mut Context<Self>) {
        use crate::voice_gesture::Action as VoiceAction;
        for action in actions {
            match action {
                VoiceAction::Start => {
                    haptics::play(Haptic::Light);
                    self.start_overscroll_voice(cx);
                }
                VoiceAction::Lock => {
                    haptics::play(Haptic::Medium);
                    self.lock_overscroll_voice(cx);
                }
                VoiceAction::Finish => {
                    haptics::play(Haptic::Light);
                    self.finish_overscroll_voice(cx);
                }
                VoiceAction::Cancel => self.cancel_voice_animated(cx),
            }
        }
        cx.notify();
    }

    fn voice_distance_to_bottom(&self) -> f32 {
        let logical = f(self.transcript_scroll.max_offset().y + self.transcript_scroll.offset().y);
        let painted = if self.follow { f(self.stream_motion.shift()) } else { 0. };
        (logical + painted).max(0.)
    }

    fn voice_wheel_eligible(&self, event: &ScrollWheelEvent) -> bool {
        let delta = event.delta.pixel_delta(px(20.));
        cfg!(any(target_os = "ios", target_os = "linux"))
            && self.scene_allows_voice()
            && self.scene().menus
            && !matches!(event.touch_phase, TouchPhase::Ended | TouchPhase::Cancelled)
            && !self.feel.voice_pan_ended
            && self.feel.g.refreshing.is_none()
            && self.feel.g.transcript.is_some_and(|b| b.contains(&event.position))
            && delta.y < px(0.) && delta.y.abs() >= delta.x.abs()
            && self.voice_distance_to_bottom() < -f(delta.y) + 0.5
    }

    /// Capture only the deliberate overscroll, not ordinary scrolling or the
    /// post-release fling. Explicit touch phases never use an idle timeout.
    fn feel_voice_wheel(&mut self, event: &ScrollWheelEvent, cx: &mut Context<Self>) -> bool {
        if event.touch_phase == TouchPhase::Started {
            self.feel.voice_pan_ended = false;
            self.feel.voice_phased = true;
        }
        let tracking = self.feel.voice_wheel_at.is_some();
        if event.touch_phase == TouchPhase::Cancelled {
            self.feel.voice_pan_ended = true;
            if tracking {
                self.cancel_voice_animated(cx);
                cx.notify();
            }
            return tracking;
        }
        let ended = event.touch_phase == TouchPhase::Ended;
        if ended { self.feel.voice_pan_ended = true; }
        let delta = event.delta.pixel_delta(px(20.));
        let now = Instant::now();
        let dt = self.feel.voice_wheel_at.map_or(0., |at| now.duration_since(at).as_secs_f32());
        let mut dy = f(delta.y);
        if !tracking {
            if !self.voice_wheel_eligible(event) { return false; }
            let remaining = self.voice_distance_to_bottom();
            if remaining > 0. {
                // The same event reaches the latest text and uncovers the mic.
                // Only the excess is voice travel, never the ordinary scroll.
                let offset = self.transcript_scroll.offset();
                self.transcript_scroll.set_offset(gpui::point(offset.x, -self.transcript_scroll.max_offset().y));
                dy = (dy + remaining).min(0.);
            }
            self.stream_motion.reset();
            self.follow = true;
            self.feel.g.cancel();
            // Keep a settling displacement when the finger grabs it again.
            self.feel.voice_gesture.dismiss();
        }
        if !tracking || ended { self.feel.voice_frame_at = Some(now); }
        self.feel.voice_wheel_at = if ended { None } else { Some(now) };
        let actions = self.feel.voice_gesture.feed_timed(dy, ended, dt);
        self.apply_voice_gesture(actions, cx);
        true
    }

    pub(super) fn focused_rename_input(&self, window: &Window, cx: &gpui::App) -> Option<Entity<TextInput>> {
        self.feel.rename.as_ref().map(|(_, input)| input)
            .filter(|input| input.read(cx).is_focused(window)).cloned()
    }

    fn scene(&self) -> Scene {
        let chat = self.screen == Screen::Chat;
        let at_top = f(self.transcript_scroll.offset().y) >= -0.5;
        Scene {
            pushed: self.screen == Screen::Settings,
            sheet_open: self.model_picker && chat,
            at_top: at_top && !self.dir_picker,
            // Voice has its own held/locked lifecycle, separate from reconnect.
            dictate: false,
            menus: chat
                && !self.git_info.open
                && !self.model_picker
                && !self.dir_picker
                && self.tab_menu.is_none()
                && self.feel.menu.is_none()
                && self.feel.selection.is_none()
                && self.feel.rename.is_none(),
        }
    }

    fn sync_gestures(&mut self) {
        let scene = self.scene();
        let r = self.feel.rec.borrow();
        let g = &mut self.feel.g;
        if let Some(s) = r.screen {
            g.screen = s;
        }
        g.sheet = r.sheet;
        g.transcript = r.transcript;
        let clip = r.transcript;
        g.hits = r
            .hits
            .iter()
            .filter(|(t, b)| match t {
                // Messages scrolled under the header are not pressable.
                Target::Message(_) => clip.is_some_and(|c| c.contains(&b.center()) || c.intersects(b)),
                Target::Tab(_) => true,
            })
            .map(|(t, b)| {
                let b = match (t, clip) {
                    (Target::Message(_), Some(c)) => b.intersect(&c),
                    _ => *b,
                };
                (*t, b)
            })
            .collect();
        g.set_scene(scene);
    }

    /// Haptic for a sent message (call from send paths).
    pub(super) fn feel_sent(&self) {
        haptics::play(Haptic::Light);
    }

    /// Wrap one transcript entry so long press / right click finds it.
    pub(super) fn feel_hit(&self, target: Target, el: AnyElement) -> AnyElement {
        let rec = self.feel.rec.clone();
        let pressed = matches!(&self.feel.menu, Some(m) if Target::Message(m.entry) == target && !m.closing);
        div()
            .relative()
            .child(el)
            .when(pressed, |d| d.opacity(0.75))
            .child(canvas(move |b, _, _| rec.borrow_mut().hits.push((target, b)), |_, _, _, _| {}).absolute().inset_0())
            .into_any_element()
    }

    /// Recorder for a tab (add as a child of the tab element).
    pub(super) fn feel_tab_hit(&self, id: u64) -> impl IntoElement {
        let rec = self.feel.rec.clone();
        canvas(move |b, _, _| rec.borrow_mut().hits.push((Target::Tab(id), b)), |_, _, _, _| {}).absolute().inset_0()
    }

    /// Recorder for the model sheet panel.
    pub(super) fn feel_sheet_recorder(&self) -> impl IntoElement {
        recorder(self.feel.rec.clone(), |r, b| r.sheet = Some(b))
    }

    /// Sheet panel offsets: (top, left, extra bottom padding for rubber band).
    pub(super) fn feel_sheet_offsets(&self) -> (f32, f32, f32) {
        let y = self.feel.g.sheet_y;
        (y.max(0.), self.feel.g.edge_x, (-y).max(0.))
    }

    pub(super) fn feel_backdrop(&self) -> f32 {
        0.5 * self.feel.g.sheet_presence()
    }

    /// Animated sheet close (backdrop tap, model picked).
    pub(super) fn feel_close_sheet(&mut self) {
        self.feel.g.begin_dismiss_sheet();
    }

    /// Animated pop from Settings ("‹ back").
    pub(super) fn feel_back(&mut self) {
        self.feel.g.begin_back();
    }

    /// The pull indicator owns connection feedback until it has settled away.
    pub(super) fn feel_reconnect_visible(&self) -> bool {
        self.feel.g.refreshing.is_some() || (self.feel.g.pull > 0.5 && !self.feel.g.dictate_pull)
    }

    /// The transcript area (empty or not): records bounds for pull to refresh,
    /// shifts content down while pulling and shows the refresh indicator.
    pub(super) fn feel_transcript(&self, body: AnyElement, t: &Theme) -> AnyElement {
        let pull = self.feel.g.pull;
        let dictate = self.feel.g.dictate_pull;
        let refreshing = self.feel.g.refreshing.is_some();
        let armed = pull >= crate::gestures::PULL_TRIGGER || refreshing;
        let show = pull > 0.5 || refreshing;
        let progress = (pull / crate::gestures::PULL_TRIGGER).clamp(0., 1.);
        let finished = refreshing && self.session.phase == ConnectionPhase::Connected;
        let voice_visible = self.feel.voice_gesture.active() || self.feel.voice_gesture.pull > 0.5;
        div()
            .relative()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .overflow_hidden()
            .child(recorder(self.feel.rec.clone(), |r, b| r.transcript = Some(b)))
            .when(show, |d| {
                d.child(
                    div()
                        .absolute()
                        .left_0()
                        .right_0()
                        .top(px((pull - 34.).max(0.) / 2.))
                        .when(dictate, |d| d.top_auto().bottom(px((pull - 34.).max(0.) / 2.)))
                        .h(px(34.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .gap(px(6.))
                        .opacity(progress)
                        .child(
                            div()
                                .size(px(26.))
                                .flex_none()
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(self.status_orb.clone()),
                        )
                        .child(label(
                            if dictate && armed {
                                "Release to dictate"
                            } else if dictate {
                                "Scroll past latest to dictate"
                            } else if finished {
                                "Connected"
                            } else if refreshing && matches!(self.session.phase, ConnectionPhase::Failed { .. }) {
                                "Couldn't reconnect"
                            } else if refreshing {
                                "Reconnecting…"
                            } else if armed {
                                "Release to reconnect"
                            } else {
                                "Pull to reconnect"
                            },
                            t.text_muted,
                            11.,
                        )),
                )
            })
            .child(div().relative().top(px(if voice_visible { 0. } else if dictate { -pull } else { pull })).flex_1().min_h_0().flex().flex_col().child(body))
            .into_any_element()
    }

    /// Reveal the mic beyond the entire chat surface, including the input line.
    /// Translate instead of resizing so scroll layout and anchoring stay stable.
    pub(super) fn feel_voice_surface(&self, content: AnyElement, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        use crate::voice_visual::{self, Phase};
        let pull = self.feel.voice_gesture.pull;
        let visible = self.feel.voice_gesture.active() || pull > 0.5
            || (self.voice.auto_send && (self.voice.busy || self.voice.recording.is_some()));
        let phase = if self.voice.has_error() { Phase::Error }
            else if self.voice.locked { Phase::Locked }
            else if self.voice.starting { Phase::Preparing }
            else if self.voice.busy { Phase::Transcribing }
            else if self.voice.recording.is_some() { Phase::Recording }
            else { Phase::Pulling };
        let height = if visible { voice_visual::reveal_height(pull, phase) } else { 0. };
        let elapsed = self.voice.began.map_or(0., |at| at.elapsed().as_secs_f32());
        let indicator = voice_visual::render_live(pull, phase, elapsed, crate::system::reduce_motion(), t, self.voice.levels());
        let indicator = if self.voice.auto_send && (self.voice.starting || self.voice.recording.is_some()) {
            div().id("stop-dictation").debug_selector(|| "stop-dictation".into())
                .role(gpui::Role::Button)
                .aria_label(format!("{}. Tap microphone to stop recording and send", phase.label()))
                .w(px(voice_visual::LIVE_WIDTH)).h_full().cursor_pointer()
                .on_click(cx.listener(|this, _, _, cx| {
                    this.feel.voice_gesture.dismiss();
                    this.feel.voice_frame_at = Some(Instant::now());
                    this.feel.voice_wheel_at = None;
                    this.feel.voice_pan_ended = true;
                    this.finish_overscroll_voice(cx);
                }))
                .child(indicator).into_any_element()
        } else { indicator };
        div().relative().size_full().overflow_hidden()
            .child(div().relative().top(px(-height)).size_full().child(content))
            .when(visible, |d| d.child(
                div().absolute().left_0().right_0().bottom_0().h(px(height))
                    .flex().justify_center()
                    .child(indicator)
            ))
            .into_any_element()
    }

    fn feel_apply(&mut self, a: Action, window: &mut Window, cx: &mut Context<Self>) {
        match a {
            Action::Back => {
                self.screen = if self.account.is_some() { Screen::Chat } else { Screen::SignIn };
            }
            Action::DismissSheet => self.model_picker = false,
            Action::Refresh => self.reconnect(cx),
            Action::Dictate => self.start_overscroll_voice(cx),
            Action::Menu(Target::Message(entry), at) => {
                self.feel.menu = Some(MsgMenu { entry, at, closing: false });
                self.feel.menu_anim = 0.;
            }
            Action::Menu(Target::Tab(id), _) => {
                self.tab_hold = None;
                self.tab_menu = Some(id);
            }
        }
        let _ = window;
        cx.notify();
    }

    pub(super) fn feel_voice_motion_changed(&mut self) {
        self.feel.voice_frame_at = Some(Instant::now());
    }

    /// A single continuous displacement owns the reveal. Async capture changes
    /// retarget its spring, never impose a different minimum in the renderer.
    fn step_voice_motion(&mut self, dt: f32, reduce_motion: bool) -> bool {
        let rest = if self.feel.voice_gesture.held() {
            0.
        } else if self.voice.auto_send && self.voice.locked && (self.voice.starting || self.voice.recording.is_some()) {
            crate::voice_gesture::LOCK_REST
        } else if self.voice.auto_send && self.voice.busy {
            crate::voice_visual::PROCESSING_REST
        } else { 0. };
        let retargeted = self.feel.voice_rest != rest;
        self.feel.voice_rest = rest;
        self.feel.voice_gesture.rest_at(rest);
        if reduce_motion {
            self.feel.voice_gesture.snap_to_target();
            false
        } else { self.feel.voice_gesture.tick(if retargeted { 0. } else { dt }) }
    }

    /// Run before render_chat, so the last settling step is painted too. Advancing
    /// after building body would leave the displayed chat one frame behind state.
    pub(super) fn feel_prepare_voice(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let now = Instant::now();
        let dt = self.feel.voice_frame_at.map_or(0., |at| now.duration_since(at).as_secs_f32());
        self.feel.voice_frame_at = Some(now);
        if (self.feel.voice_gesture.active() || self.voice.auto_send) && !self.scene().menus {
            self.cancel_voice(cx);
        }
        let wheel_timer = !self.feel.voice_phased && self.feel.voice_wheel_at.is_some();
        if wheel_timer && self.feel.voice_wheel_at.is_some_and(|at| now.duration_since(at).as_secs_f32() > 0.18) {
            self.feel.voice_wheel_at = None;
            let actions = self.feel.voice_gesture.release();
            self.apply_voice_gesture(actions, cx);
        }
        // Held input redraws on input events. Capture callbacks redraw on state
        // changes. A settled mic must not rebuild the entire transcript at 120 Hz.
        if self.step_voice_motion(dt, crate::system::reduce_motion())
            || (wheel_timer && self.feel.voice_wheel_at.is_some()) {
            window.request_animation_frame();
        }
    }

    /// Per-frame: advance gestures, run finished actions, and wrap `body`
    /// with the edge-swipe stack and overlays. Call with the screen body.
    pub(super) fn feel_wrap(&mut self, body: AnyElement, t: &Theme, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        self.sync_gestures();
        {
            let mut r = self.feel.rec.borrow_mut();
            r.hits.clear();
            r.sheet = None;
            r.transcript = None;
        }
        if self.feel.g.refreshing.is_some() {
            match self.session.phase {
                ConnectionPhase::Connected | ConnectionPhase::Failed { .. } => self.feel.g.refresh_done(),
                _ => {}
            }
        }
        let (actions, mut anim) = self.feel.g.tick(self.scene());
        for a in actions {
            self.feel_apply(a, window, cx);
        }
        let now = Instant::now();
        let dt = self.feel.last.map_or(0., |l| now.duration_since(l).as_secs_f32().min(0.05));
        self.feel.last = Some(now);
        let menu_target = match &self.feel.menu {
            Some(m) if !m.closing => 1.,
            _ => 0.,
        };
        self.feel.menu_anim = crate::gestures::approach(self.feel.menu_anim * 100., menu_target * 100., dt, 22.) / 100.;
        if self.feel.menu_anim * 100. != menu_target * 100. {
            anim = true;
        } else if menu_target == 0. {
            self.feel.menu = None;
        }
        if self.feel.toast.as_ref().is_some_and(|(_, at)| at.elapsed().as_secs_f32() > 1.6) {
            self.feel.toast = None;
        }
        if self.feel.toast.is_some() || self.feel.g.refreshing.is_some() {
            anim = true;
        }
        if std::mem::take(&mut self.feel.blur_rename) {
            window.blur(cx);
            window.dismiss_virtual_keyboard();
            cx.notify();
        }
        if std::mem::take(&mut self.feel.focus_rename)
            && let Some((_, input)) = &self.feel.rename
        {
            use gpui::Focusable as _;
            window.focus(&input.focus_handle(cx), cx);
        }
        if anim {
            window.request_animation_frame();
        }

        let w = self.feel.g.screen.size.width;
        let w = if f(w) > 1. { f(w) } else { self.screen_width(window) };
        let edge = self.feel.g.edge_x;
        let body = if self.screen == Screen::Settings && edge > 0.5 {
            // Previous screen peeks in underneath with a parallax shift.
            let under = self.render_chat(t, window, cx);
            let frac = (edge / w).clamp(0., 1.);
            div()
                .relative()
                .size_full()
                .overflow_hidden()
                .child(div().absolute().top_0().bottom_0().w(px(w)).left(px(-(1. - frac) * w * 0.3)).child(under))
                .child(div().absolute().inset_0().bg(gpui::black().opacity(0.3 * (1. - frac))))
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .w(px(w))
                        .left(px(edge))
                        .bg(t.background)
                        .shadow_lg()
                        .child(body),
                )
                .into_any_element()
        } else {
            body
        };

        let this = cx.entity().downgrade();
        let listen = canvas(
            {
                let rec = self.feel.rec.clone();
                move |b, _, _| rec.borrow_mut().screen = Some(b)
            },
            move |_, _, window, _| {
                let h = this.clone();
                window.on_mouse_event(move |e: &MouseDownEvent, phase, window, cx| {
                    if phase != DispatchPhase::Capture {
                        return;
                    }
                    let (p, b) = (e.position, e.button);
                    h.update(cx, |this, cx| {
                        if b == MouseButton::Left && this.feel.menu.is_none()
                            && this.feel.selection.is_none() && this.feel.rename.is_none()
                            && this.git_down(p) { return; }
                        if this.git_info.open { return; }
                        this.sync_gestures();
                        match b {
                            MouseButton::Left => this.feel.g.down(p, false),
                            MouseButton::Right => {
                                if let Some(a) = this.feel.g.right_click(p) {
                                    this.feel_apply(a, window, cx);
                                }
                            }
                            _ => {}
                        }
                        cx.notify();
                    })
                    .ok();
                });
                let h = this.clone();
                window.on_mouse_event(move |e: &MouseMoveEvent, phase, _, cx| {
                    if phase != DispatchPhase::Capture || e.pressed_button != Some(MouseButton::Left) {
                        return;
                    }
                    let p = e.position;
                    let active = h
                        .update(cx, |this, cx| {
                            if this.git_move(p, cx) { return true; }
                            if this.git_info.open { return false; }
                            this.feel.g.moved(p);
                            this.feel_scrub_tabs(cx);
                            let d = this.feel.g.dragging();
                            if d {
                                cx.notify();
                            }
                            d
                        })
                        .unwrap_or(false);
                    if active {
                        cx.stop_propagation();
                    }
                });
                let h = this.clone();
                window.on_mouse_event(move |e: &MouseUpEvent, phase, window, cx| {
                    if phase != DispatchPhase::Capture || e.button != MouseButton::Left {
                        return;
                    }
                    let swallow = h.update(cx, |this, cx| {
                        if this.git_up(false, cx) { return true; }
                        if this.git_info.open { return false; }
                        this.feel_up(window, cx)
                    }).unwrap_or(false);
                    if swallow {
                        cx.stop_propagation();
                    }
                });
                let h = this.clone();
                window.on_mouse_event(move |e: &ScrollWheelEvent, phase, _, cx| {
                    if phase != DispatchPhase::Capture {
                        return;
                    }
                    let delta = e.delta.pixel_delta(px(20.));
                    let dy = f(delta.y);
                    let ended = e.touch_phase == TouchPhase::Ended;
                    let p = e.position;
                    let consumed = h
                        .update(cx, |this, cx| {
                            if this.git_info.open { return false; }
                            this.sync_gestures();
                            if this.feel_voice_wheel(e, cx) {
                                return true;
                            }
                            if e.touch_phase == TouchPhase::Cancelled {
                                this.feel.g.cancel();
                                cx.notify();
                                // Let the scroll owner unwind its own swipe/list state.
                                return false;
                            }
                            if delta.x.abs() > delta.y.abs() && !ended {
                                return false;
                            }
                            let (a, consumed) = this.feel.g.wheel(p, dy, ended);
                            match a {
                                Some(Action::Refresh) => this.reconnect(cx),
                                Some(Action::Dictate) => this.start_overscroll_voice(cx),
                                _ => {}
                            }
                            if consumed {
                                cx.notify();
                            }
                            consumed
                        })
                        .unwrap_or(false);
                    if consumed {
                        cx.stop_propagation();
                    }
                });
                let h = this.clone();
                window.on_mouse_event(move |e: &TouchDragEvent, phase, window, cx| {
                    if phase != DispatchPhase::Capture {
                        return;
                    }
                    let e = e.clone();
                    let claim = h
                        .update(cx, |this, cx| {
                            if this.git_info.open { return false; }
                            // Modal controls can overlap the recorded Git handle.
                            // Do not claim their raw touch before hit testing reaches
                            // the overlay (notably selection's top-right Done button).
                            if this.feel.selection.is_some() || this.feel.rename.is_some()
                                || this.feel.menu.is_some() { return false; }
                            if e.phase == TouchPhase::Started && this.git_down(e.start_position) {
                                this.git_move(e.position, cx);
                                return true;
                            }
                            if this.git_move(e.position, cx) {
                                if matches!(e.phase, TouchPhase::Ended | TouchPhase::Cancelled) {
                                    this.git_up(e.phase == TouchPhase::Cancelled, cx);
                                }
                                return true;
                            }
                            // Onboarding owns its hero drag, not the chat gesture stack.
                            if this.screen == Screen::SignIn {
                                return false;
                            }
                            this.sync_gestures();
                            if e.phase == TouchPhase::Cancelled {
                                this.feel.g.cancel();
                                this.tab_hold = None;
                                this.hold_fired = false;
                                cx.notify();
                                return true;
                            }
                            if this.feel.selection.is_some() {
                                return false;
                            }
                            match e.phase {
                                TouchPhase::Started => {
                                    let want = this.feel.g.wants_touch_drag(e.start_position);
                                    if want {
                                        if this.feel.g.tab_scrub().is_none() {
                                            this.feel.g.down(e.start_position, true);
                                        }
                                        this.feel.g.moved(e.position);
                                        this.feel_scrub_tabs(cx);
                                    }
                                    want
                                }
                                TouchPhase::Moved => {
                                    this.feel.g.moved(e.position);
                                    this.feel_scrub_tabs(cx);
                                    cx.notify();
                                    true
                                }
                                _ => {
                                    this.feel_up(window, cx);
                                    true
                                }
                            }
                        })
                        .unwrap_or(false);
                    if claim {
                        window.prevent_default();
                        cx.stop_propagation();
                    }
                });
                let h = this.clone();
                window.on_mouse_event(move |e: &LongPressEvent, phase, window, cx| {
                    if phase != DispatchPhase::Capture {
                        return;
                    }
                    let p = e.start_position;
                    let took = h
                        .update(cx, |this, cx| {
                            this.sync_gestures();
                            if e.phase != TouchPhase::Started {
                                if this.feel.g.tab_scrub().is_none() { return false; }
                                match e.phase {
                                    TouchPhase::Moved => {
                                        this.feel.g.moved(e.position);
                                        this.feel_scrub_tabs(cx);
                                    }
                                    TouchPhase::Cancelled => this.feel.g.cancel(),
                                    _ => { this.feel_up(window, cx); }
                                }
                                cx.notify();
                                return true;
                            }
                            match this.feel.g.long_press(p) {
                                Some(a) => {
                                    this.feel_apply(a, window, cx);
                                    true
                                }
                                None => {
                                    let armed = this.feel.g.tab_scrub().is_some();
                                    if armed { cx.notify(); }
                                    armed
                                }
                            }
                        })
                        .unwrap_or(false);
                    if took {
                        window.prevent_default();
                        cx.stop_propagation();
                    }
                });
            },
        )
        .absolute()
        .inset_0();

        div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .child(body)
            .child(listen)
            .when(self.feel.g.tab_scrub().is_some(), |d| d.child(
                div().id("tab-hold-feedback")
                    .debug_selector(|| "tab-hold-feedback".into())
                    .absolute().left_0().right_0().top(px(self.safe.top + 52.))
                    .flex().justify_center()
                    .child(div().px(px(12.)).py(px(7.)).rounded_full()
                        .bg(t.surface_elevated).border_1().border_color(t.accent)
                        .child(label(if self.feel.g.tab_options_ready() {
                            "Slide to browse · release for options"
                        } else {
                            "Slide to browse · release to select"
                        }, t.text_strong, 12.)))
            ))
            .when(self.feel.menu.is_some(), |d| d.child(self.render_msg_menu(t, cx)))
            .when(self.feel.selection.is_some(), |d| d.child(self.render_text_selection(t, cx)))
            .when(self.feel.rename.is_some(), |d| d.child(self.render_rename(t, cx)))
            .when_some(self.feel.toast.clone(), |d, (msg, at)| {
                let age = at.elapsed().as_secs_f32();
                let o = (age / 0.15).min(1.) * ((1.6 - age) / 0.3).clamp(0., 1.);
                d.child(
                    div().absolute().left_0().right_0().top(px(60.)).flex().justify_center().opacity(o).child(
                        div()
                            .px(px(14.))
                            .py(px(7.))
                            .rounded_full()
                            .bg(t.surface_elevated)
                            .border_1()
                            .border_color(t.border)
                            .child(label(msg, t.text_strong, 12.)),
                    ),
                )
            })
            .into_any_element()
    }

    fn feel_scrub_tabs(&mut self, cx: &mut Context<Self>) {
        let Some((origin, dx)) = self.feel.g.tab_scrub() else { return };
        let Some(index) = self.tab_order.iter().position(|id| *id == origin) else { return };
        let index = crate::tabs::scrub_index(index, self.tab_order.len(), dx);
        let id = self.tab_order[index];
        if id != self.active_tab {
            self.focus_tab(id, cx);
        }
    }

    fn feel_up(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let (a, swallow) = self.feel.g.up();
        if swallow {
            // The tab hold timer must not eat the next tap either.
            self.tab_hold = None;
            self.hold_fired = false;
        }
        match a {
            Some(Action::Refresh) => self.reconnect(cx),
            Some(Action::Dictate) => self.start_overscroll_voice(cx),
            Some(a @ Action::Menu(..)) => {
                haptics::play(Haptic::Light);
                self.feel_apply(a, window, cx);
            }
            _ => {}
        }
        cx.notify();
        swallow
    }

    fn toast(&mut self, msg: &str) {
        self.feel.toast = Some((msg.to_string(), Instant::now()));
    }

    fn close_menu(&mut self, cx: &mut Context<Self>) {
        if let Some(m) = &mut self.feel.menu {
            m.closing = true;
        }
        cx.notify();
    }

    /// The prompt a "Retry" on this entry resends.
    fn retry_text(&self, entry: u64) -> Option<String> {
        let tr = &self.session.transcript;
        let i = tr.iter().position(|e| e.id == entry)?;
        tr[..=i].iter().rev().find(|e| e.role == Role::User).map(|e| e.text.clone())
    }

    fn entry_text(&self, entry: u64) -> Option<String> {
        self.session.transcript.iter().find(|e| e.id == entry).map(|e| e.text.clone())
    }

    fn can_retry_entry(&self, entry: u64) -> bool {
        self.retry_text(entry).is_some()
            && !self.session.is_processing
            && self.session.phase == ConnectionPhase::Connected
    }

    fn render_msg_menu(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let Some(m) = &self.feel.menu else { return div().into_any_element() };
        let entry = m.entry;
        let a = self.feel.menu_anim.clamp(0., 1.);
        let screen = self.feel.g.screen;
        let (mw, mh) = (210., 4. * 44. + 8.);
        let sx = f(m.at.x - screen.origin.x);
        let sy = f(m.at.y - screen.origin.y);
        let left = (sx - mw / 2.).clamp(12., (f(screen.size.width) - mw - 12.).max(12.));
        let below = sy + 14. + mh < f(screen.size.height) - 40.;
        let top = if below { sy + 14. } else { (sy - 14. - mh).max(60.) };
        let can_retry = self.can_retry_entry(entry);
        let row = |id: &'static str, text: &'static str, glyph: &'static str, enabled: bool| {
            div()
                .id(id)
                .debug_selector(move || id.into())
                .h(px(44.))
                .px(px(14.))
                .flex()
                .items_center()
                .justify_between()
                .rounded(px(9.))
                .when(enabled, |d| d.cursor_pointer().hover(|d| d.bg(t.surface)))
                .child(label(text, if enabled { t.text_strong } else { t.text_muted }, 14.))
                .child(label(glyph, t.text_secondary, 14.))
        };
        div()
            .absolute()
            .inset_0()
            .child(
                div()
                    .id("msg-menu-dismiss")
                    .debug_selector(|| "msg-menu-dismiss".into())
                    .absolute()
                    .inset_0()
                    .bg(gpui::black().opacity(0.25 * a))
                    .on_click(cx.listener(|this, _, _, cx| this.close_menu(cx))),
            )
            .child(
                div()
                    .absolute()
                    .left(px(left))
                    .top(px(top + (1. - a) * if below { -8. } else { 8. }))
                    .w(px(mw))
                    .opacity(a)
                    .p(px(4.))
                    .flex()
                    .flex_col()
                    .rounded(px(13.))
                    .bg(t.surface_elevated)
                    .border_1()
                    .border_color(t.border)
                    .shadow_lg()
                    .child(row("msg-select", "Select text", "↔", true).on_click(cx.listener(move |this, _, window, cx| {
                        if let Some(text) = this.entry_text(entry) {
                            window.blur(cx);
                            window.dismiss_virtual_keyboard();
                            if crate::text_selection::present(&text) {
                                this.close_menu(cx);
                                return;
                            }
                            let input = cx.new(|cx| {
                                let mut input = TextInput::new("", cx);
                                input.set_text(text, cx);
                                input.read_only = true;
                                input.bare = true;
                                input.max_lines = usize::MAX;
                                input.select_all_text(cx);
                                input
                            });
                            use gpui::Focusable as _;
                            window.blur(cx);
                            window.dismiss_virtual_keyboard();
                            window.focus(&input.focus_handle(cx), cx);
                            this.feel.selection = Some(input);
                        }
                        this.close_menu(cx);
                    })))
                    .child(row("msg-copy", "Copy", "⧉", true).on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(text) = this.entry_text(entry) {
                            cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
                            haptics::play(Haptic::Success);
                            this.toast("Copied");
                        }
                        this.close_menu(cx)
                    })))
                    .child(row("msg-retry", "Retry", "↻", can_retry).when(can_retry, |d| {
                        d.on_click(cx.listener(move |this, _, _, cx| {
                            if this.can_retry_entry(entry) && let Some(text) = this.retry_text(entry) {
                                this.session.apply_intent(LocalIntent::UserSentMessage(text.clone()));
                                this.send(|id| Request::Message { id, content: text });
                                this.stream_motion.reset();
                                this.follow = true;
                                this.feel_sent();
                            }
                            this.close_menu(cx)
                        }))
                    }))
                    .child(row("msg-share", "Share", "↑", true).on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(text) = this.entry_text(entry)
                            && !crate::share::share_text(&text)
                        {
                            // Preview: no share sheet, copy instead.
                            cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
                            this.toast("Copied for sharing");
                        }
                        this.close_menu(cx)
                    }))),
            )
            .into_any_element()
    }

    fn render_text_selection(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let Some(input) = &self.feel.selection else { return div().into_any_element() };
        let button = |id: &'static str, text: &'static str| {
            div().id(id).debug_selector(move || id.into()).px(px(12.)).h(px(44.)).flex().items_center()
                .cursor_pointer().child(label(text, t.text_strong, 14.))
        };
        div()
            .occlude()
            .absolute().inset_0().p(px(16.)).pt(px(60.)).pb(px(32.))
            .bg(t.background).flex().flex_col().gap(px(8.))
            .child(
                div().flex().items_center().justify_between()
                    .child(label("Select text", t.text_strong, 16.))
                    .child(button("selection-done", "Done").on_click(cx.listener(|this, _, window, cx| {
                        this.feel.selection = None;
                        window.blur(cx);
                        window.dismiss_virtual_keyboard();
                        cx.notify();
                    }))),
            )
            .child(
                div().id("selection-scroll").debug_selector(|| "selection-scroll".into()).flex_1().min_h_0().overflow_y_scroll()
                    .child(input.clone()),
            )
            .child(
                div().flex().justify_between()
                    .child(button("selection-all", "Select all").on_click(cx.listener(|this, _, _, cx| {
                        if let Some(input) = &this.feel.selection {
                            input.update(cx, |input, cx| input.select_all_text(cx));
                        }
                    })))
                    .child(button("selection-copy", "Copy").on_click(cx.listener(|this, _, _, cx| {
                        if let Some(input) = &this.feel.selection {
                            let text = input.read(cx).selected_text().to_string();
                            if !text.is_empty() {
                                cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
                                this.toast("Copied");
                            }
                        }
                    }))),
            )
            .into_any_element()
    }

    /// Tab sheet "Rename": inline title editor.
    pub(super) fn feel_rename_tab(&mut self, id: u64, cx: &mut Context<Self>) {
        let current = self.tab_session(id).and_then(|s| s.session_title.clone()).unwrap_or_default();
        let input = cx.new(|cx| {
            let mut t = TextInput::new("Session name", cx);
            t.set_text(current, cx);
            t.config.input_action = gpui::TextInputAction::Done;
            t
        });
        cx.subscribe(&input, |this, _, ev, cx| match ev {
            TextInputEvent::Submit => this.commit_rename(cx),
            TextInputEvent::Changed => cx.notify(),
        })
        .detach();
        self.feel.g.cancel();
        self.feel.menu = None;
        self.feel.menu_anim = 0.;
        self.feel.selection = None;
        self.tab_menu = None;
        self.feel.rename = Some((id, input));
        self.feel.blur_rename = false;
        self.feel.focus_rename = true;
        cx.notify();
    }

    fn commit_rename(&mut self, cx: &mut Context<Self>) {
        let Some((id, input)) = self.feel.rename.take() else { return };
        self.feel.focus_rename = false;
        // Submit is an entity event without a Window. Finish its keyboard
        // dismissal in feel_wrap, including native iOS keyboard presentation.
        self.feel.blur_rename = true;
        let title = input.read(cx).text().trim().to_string();
        let title = (!title.is_empty()).then_some(title);
        let (conn, session) = if id == self.active_tab {
            (self.connection.as_ref(), Some(&mut self.session))
        } else {
            match self.parked.iter_mut().find(|p| p.id == id) {
                Some(p) => (p.connection.as_ref(), Some(&mut p.session)),
                None => (None, None),
            }
        };
        if let Some(c) = conn {
            let t2 = title.clone();
            c.send(|id| Request::RenameSession { id, title: t2 });
        }
        if let Some(s) = session {
            s.session_title = title;
        }
        haptics::play(Haptic::Success);
        cx.notify();
    }

    /// Fork the source conversation into an independent session in a new tab.
    pub(super) fn fork_tab(&mut self, id: u64, cx: &mut Context<Self>) {
        if self.account.is_none() || self.offline {
            return;
        }
        let Some(sid) = self.tab_session(id).and_then(|s| s.session_id.clone()) else { return };
        self.tab_menu = None;
        let new = self.next_tab;
        self.next_tab += 1;
        let at = self.tab_order.iter().position(|t| *t == id).map_or(self.tab_order.len(), |i| i + 1);
        self.tab_order.insert(at, new);
        let title = self.tab_session(id).and_then(|s| s.session_title.clone());
        self.park_active(cx);
        self.active_tab = new;
        self.session = Default::default();
        self.session.session_title = title;
        self.start_connection_target(None, None, Some(sid), cx);
        haptics::play(Haptic::Light);
        cx.notify();
    }

    fn render_rename(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let Some((_, input)) = &self.feel.rename else { return div().into_any_element() };
        let button = |id: &'static str, text: &'static str, strong: bool| {
            div()
                .id(id)
                .debug_selector(move || id.into())
                .flex_1()
                .h(px(44.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(10.))
                .cursor_pointer()
                .when(strong, |d| d.bg(t.surface))
                .hover(|d| d.bg(t.surface))
                .child(label(text, t.text_strong, 14.))
        };
        div()
            .id("rename-overlay")
            .occlude()
            .absolute()
            .inset_0()
            .flex()
            .flex_col()
            .items_center()
            .p(px(12.))
            .overflow_y_scroll()
            .bg(gpui::black().opacity(0.45))
            .child(
                div()
                    .id("rename-card")
                    .debug_selector(|| "rename-card".into())
                    .w(px(300.))
                    .max_w_full()
                    .flex_none()
                    .my_auto()
                    .p(px(16.))
                    .flex()
                    .flex_col()
                    .gap(px(12.))
                    .rounded(px(16.))
                    .bg(t.surface_elevated)
                    .border_1()
                    .border_color(t.border)
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(label("Rename session", t.text_strong, 15.))
                    .child(input.clone())
                    .child(
                        div()
                            .flex()
                            .gap(px(8.))
                            .child(button("rename-cancel", "Cancel", false).on_click(cx.listener(|this, _, window, cx| {
                                this.feel.rename = None;
                                this.feel.focus_rename = false;
                                window.blur(cx);
                                window.dismiss_virtual_keyboard();
                                cx.notify()
                            })))
                            .child(button("rename-save", "Save", true).on_click(cx.listener(|this, _, window, cx| {
                                this.commit_rename(cx);
                                window.blur(cx);
                                window.dismiss_virtual_keyboard();
                            }))),
                    ),
            )
            .into_any_element()
    }
}

/// Success/error buzz when the active tab's turn ends.
pub(super) fn turn_haptic(was_processing: bool, s: &jcode_mobile_core::SessionState) {
    haptics::turn_transition(was_processing, s);
}
