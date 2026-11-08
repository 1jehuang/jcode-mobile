//! Paint-only streaming motion. Native `ScrollHandle::scroll_to_bottom` remains
//! the layout-time authority: no animation frame writes a scroll offset.
use std::{
    cell::RefCell,
    rc::Rc,
    time::{Duration, Instant},
};

use gpui::{
    AnyElement, App, Bounds, Element, ElementId, GlobalElementId, InspectorElementId, IntoElement,
    LayoutId, Pixels, ScrollHandle, Window, point, px,
};

const EASE: Duration = Duration::from_millis(120);

#[derive(Default, Debug)]
struct Motion {
    target: Option<f32>,
    visual: f32,
    from: f32,
    started: Option<Instant>,
    /// Actual pixel-snapped translation painted in the most recent frame.
    displacement: f32,
    released: bool,
    #[cfg(test)]
    now_for_test: Option<Instant>,
}

impl Motion {
    fn sample(&self, now: Instant) -> f32 {
        let Some(started) = self.started else {
            return self.visual;
        };
        let t = (now.saturating_duration_since(started).as_secs_f32() / EASE.as_secs_f32()).min(1.);
        let target = self.target.unwrap_or(self.visual);
        self.from + (target - self.from) * (1. - (1. - t).powi(3))
    }

    /// Returns the unsnapped downward translation and whether another frame is needed.
    fn advance(&mut self, target: f32, viewport: f32, now: Instant, instant: bool) -> (f32, bool) {
        let previous_target = self.target;
        let current = self.sample(now);
        if instant
            || previous_target.is_none()
            || target < previous_target.unwrap_or(target)
            || (target - previous_target.unwrap_or(target)).abs() > viewport.max(1.) * 3.
        {
            self.visual = target;
            self.from = target;
            self.started = None;
        } else if previous_target != Some(target) {
            // Retarget from the currently rendered curve, not the old logical max.
            self.from = current.max(target - (viewport * 0.6).max(240.)).min(target);
            self.visual = self.from;
            self.started = Some(now);
        } else {
            self.visual = current;
        }
        self.target = Some(target);
        if self
            .started
            .is_some_and(|start| now.saturating_duration_since(start) >= EASE)
        {
            self.visual = target;
            self.started = None;
        }
        let shift = (target - self.visual).max(0.);
        (shift, self.started.is_some())
    }
}

/// Share one instance between the transcript and its activity row. Reset when
/// changing sessions/history. Release before wheel, touch, or reader navigation.
#[derive(Clone, Default)]
pub(super) struct StreamMotion(Rc<RefCell<Motion>>);

impl StreamMotion {
    pub(super) fn shift(&self) -> Pixels {
        px(self.0.borrow().displacement)
    }

    #[cfg(test)]
    pub(super) fn set_now_for_test(&self, now: Instant) {
        self.0.borrow_mut().now_for_test = Some(now);
    }

    pub(super) fn reset(&self) {
        *self.0.borrow_mut() = Motion::default();
    }

    /// Hand off the exact displayed scroll position before manual input changes
    /// the native handle. Subsequent following paints are suppressed until the
    /// root renders `follow = false`, or explicitly resets for a new follow.
    pub(super) fn release(&self, scroll: &ScrollHandle) {
        let mut state = self.0.borrow_mut();
        let offset = scroll.offset();
        let y =
            (f32::from(offset.y) + state.displacement).clamp(-f32::from(scroll.max_offset().y), 0.);
        scroll.set_offset(point(offset.x, px(y)));
        *state = Motion {
            released: true,
            ..Motion::default()
        };
    }
}

enum Kind {
    Content { scroll: ScrollHandle, instant: bool },
    Activity,
}

pub(super) struct MotionElement {
    child: AnyElement,
    motion: StreamMotion,
    follow: bool,
    kind: Kind,
}

pub(super) fn stream_motion(
    child: impl IntoElement,
    scroll: ScrollHandle,
    motion: StreamMotion,
    follow: bool,
    instant: bool,
) -> MotionElement {
    MotionElement {
        child: child.into_any_element(),
        motion,
        follow,
        kind: Kind::Content { scroll, instant },
    }
}

/// The row must live inside `stream_motion`, after transcript blocks. The
/// viewport's content mask is the composer boundary, not the window bounds.
pub(super) fn steady_activity(
    child: impl IntoElement,
    motion: StreamMotion,
    follow: bool,
) -> MotionElement {
    MotionElement {
        child: child.into_any_element(),
        motion,
        follow,
        kind: Kind::Activity,
    }
}

impl IntoElement for MotionElement {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for MotionElement {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let child = self.child.request_layout(window, cx);
        let style = gpui::Style {
            flex_shrink: 0.,
            ..Default::default()
        };
        (window.request_layout(style, [child], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        let mask = window.content_mask().bounds;
        let shift = match &self.kind {
            Kind::Content { scroll, instant } => {
                // Parent div has already clamped native offset and published
                // max_offset for THIS layout, before traversing its children.
                let max = f32::from(scroll.max_offset().y);
                let offset = f32::from(scroll.offset().y);
                let mut state = self.motion.0.borrow_mut();
                if !self.follow {
                    *state = Motion::default();
                    0.
                } else if state.released || (offset + max).abs() > 1. {
                    // Never pull a manually positioned handle toward the tail.
                    *state = Motion {
                        released: true,
                        ..Motion::default()
                    };
                    0.
                } else {
                    let now = Instant::now();
                    #[cfg(test)]
                    let now = state.now_for_test.unwrap_or(now);
                    let (shift, animate) =
                        state.advance(max, f32::from(mask.size.height), now, *instant);
                    if animate {
                        window.request_animation_frame();
                    }
                    let scale = window.scale_factor().max(1.);
                    state.displacement = (shift * scale).round() / scale;
                    state.displacement
                }
            }
            Kind::Activity => {
                let state = self.motion.0.borrow();
                if self.follow && !state.released {
                    // Undo the transcript translation, then cap the natural row
                    // position at the viewport bottom. No measurement feedback.
                    let natural_top = f32::from(bounds.top()) - state.displacement;
                    let rest = f32::from(mask.bottom() - bounds.size.height);
                    natural_top.min(rest) - f32::from(bounds.top())
                } else {
                    0.
                }
            }
        };
        let scale = window.scale_factor().max(1.);
        let shift = (shift * scale).round() / scale;
        window.with_element_offset(point(px(0.), px(shift)), |window| {
            self.child.prepaint(window, cx);
        });
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        // AnyElement caches its translated prepaint bounds, including hitboxes.
        self.child.paint(window, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Harness {
        height: f32,
        motion: StreamMotion,
        scroll: ScrollHandle,
    }

    impl gpui::Render for Harness {
        fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
            use gpui::prelude::*;
            self.scroll.scroll_to_bottom();
            gpui::div()
                .id("motion-viewport")
                .w(px(300.))
                .h(px(200.))
                .overflow_y_scroll()
                .track_scroll(&self.scroll)
                .child(stream_motion(
                    gpui::div()
                        .flex()
                        .flex_col()
                        .py(px(12.))
                        .w_full()
                        .child(gpui::div().h(px(self.height)).flex_shrink_0())
                        .child(steady_activity(
                            gpui::div()
                                .debug_selector(|| "steady-row".to_string())
                                .h(px(40.))
                                .flex_shrink_0(),
                            self.motion.clone(),
                            true,
                        )),
                    self.scroll.clone(),
                    self.motion.clone(),
                    true,
                    false,
                ))
        }
    }

    #[gpui::test]
    fn fresh_layout_max_and_steady_activity(cx: &mut gpui::TestAppContext) {
        let now = Instant::now();
        let motion = StreamMotion::default();
        motion.set_now_for_test(now);
        let scroll = ScrollHandle::new();
        let (view, cx) = cx.add_window_view(|_, _| Harness {
            height: 500.,
            motion: motion.clone(),
            scroll: scroll.clone(),
        });
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });
        assert_eq!(scroll.max_offset().y, px(364.));
        assert_eq!(scroll.offset().y, px(-364.));
        let row = cx.debug_bounds("steady-row").unwrap();
        assert_eq!(row.bottom(), px(188.));
        view.update(cx, |view, cx| {
            view.height = 580.;
            cx.notify();
        });
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });
        assert_eq!(scroll.max_offset().y, px(444.));
        assert_eq!(scroll.offset().y, px(-444.));
        assert_eq!(motion.shift(), px(80.));
        assert_eq!(cx.debug_bounds("steady-row").unwrap(), row);
        motion.set_now_for_test(now + EASE / 2);
        view.update(cx, |_, cx| cx.notify());
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });
        assert_eq!(motion.shift(), px(10.));
        assert_eq!(scroll.offset().y, px(-444.));
        assert_eq!(cx.debug_bounds("steady-row").unwrap(), row);
        motion.release(&scroll);
        assert_eq!(scroll.offset().y, px(-434.));
        motion.release(&scroll);
        assert_eq!(scroll.offset().y, px(-434.));
    }

    #[test]
    fn finite_growth_ease_and_idle() {
        let now = Instant::now();
        let mut m = Motion::default();
        assert_eq!(m.advance(100., 500., now, false), (0., false));
        assert_eq!(m.advance(180., 500., now, false), (80., true));
        let halfway = m.advance(180., 500., now + EASE / 2, false).0;
        assert!((halfway - 10.).abs() < 0.001);
        assert_eq!(m.advance(180., 500., now + EASE, false), (0., false));
        assert_eq!(m.advance(180., 500., now + EASE * 10, false), (0., false));
    }

    #[test]
    fn retarget_is_continuous_and_lag_is_bounded() {
        let now = Instant::now();
        let mut m = Motion::default();
        m.advance(100., 500., now, false);
        m.advance(180., 500., now, false);
        let before = m.sample(now + EASE / 2);
        let (shift, _) = m.advance(220., 500., now + EASE / 2, false);
        assert!((220. - shift - before).abs() < 0.001);
        let (shift, _) = m.advance(1000., 500., now + EASE / 2, false);
        assert!(shift <= 300.);
    }

    #[test]
    fn restore_shrink_and_instant_snap() {
        let now = Instant::now();
        let mut m = Motion::default();
        m.advance(100., 500., now, false);
        assert_eq!(m.advance(2000., 500., now, false), (0., false));
        assert_eq!(m.advance(1900., 500., now, false), (0., false));
        m.advance(2000., 500., now, false);
        assert_eq!(m.advance(2100., 500., now, true), (0., false));
    }

    #[test]
    fn release_transfers_rendered_displacement_once() {
        let motion = StreamMotion::default();
        let scroll = ScrollHandle::new();
        motion.0.borrow_mut().displacement = 30.;
        // An empty handle clamps the handoff to zero, not positive overscroll.
        motion.release(&scroll);
        assert_eq!(scroll.offset().y, px(0.));
        assert!(motion.0.borrow().released);
        assert_eq!(motion.0.borrow().displacement, 0.);
        motion.reset();
        assert!(!motion.0.borrow().released);
    }
}
