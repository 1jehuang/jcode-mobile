//! The virtual new-session slot beyond the last session.

use gpui::{AnyElement, Context, div, prelude::*, px};

use super::{MobileApp, a11y::A11yExt as _};
use crate::theme::Theme;

impl MobileApp {
    pub(super) fn new_session_overscroll_ready(&self, allow_new: bool, can_next: bool) -> bool {
        // Ended has already cleared finger_down by the time release is handled.
        // The drag-only preview intentionally hides during coasting, so it must
        // not also be the release predicate. allow_new rejects momentum endings.
        allow_new
            && self.account.is_some()
            && !can_next
            && self.swipe.active
            && self.swipe.dx <= -crate::swipe::NEW_SESSION_COMMIT
    }

    pub(super) fn render_new_session_overscroll(
        &self,
        progress: f32,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let progress = progress.clamp(0., 1.);
        let ready = progress >= 1.;
        let diameter = 36. + 12. * progress;
        div()
            .id("new-session-overscroll")
            .debug_selector(|| "new-session-overscroll".into())
            .absolute()
            .top_0()
            .bottom_0()
            .right(px(-56. * (1. - progress)))
            .w(px(72.))
            .flex()
            .items_center()
            .justify_center()
            .opacity(progress)
            .child(
                div()
                    .id("new-session-overscroll-plus")
                    .debug_selector(|| "new-session-overscroll-plus".into())
                    .a11y_button(if ready {
                        "Release to create a new session"
                    } else {
                        "New session"
                    })
                    .size(px(diameter))
                    .rounded_full()
                    .bg(if ready { t.accent } else { t.surface })
                    .border_1()
                    .border_color(t.accent)
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(crate::system::fs(28.))
                    .text_color(if ready { t.background } else { t.accent })
                    .cursor_pointer()
                    .child("+")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.swipe.restart_spent();
                        this.new_tab(cx);
                    })),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::super::navigation_coverage::{draw, mount};
    use super::super::*;
    use gpui::{Modifiers, ScrollDelta, ScrollWheelEvent, TestAppContext, TouchPhase, point};

    fn wheel(phase: TouchPhase, dx: f32) -> ScrollWheelEvent {
        ScrollWheelEvent {
            position: point(px(195.), px(300.)),
            delta: ScrollDelta::Pixels(point(px(dx), px(0.))),
            modifiers: Modifiers::default(),
            touch_phase: phase,
        }
    }

    #[gpui::test]
    fn new_session_overscroll_lift_creates_once_and_preserves_draft(cx: &mut TestAppContext) {
        let (view, cx) = mount(cx);
        view.update(cx, |app, cx| {
            app.composer
                .update(cx, |input, cx| input.set_text("Keep this draft", cx));
        });
        draw(cx);
        for (phase, dx) in [(TouchPhase::Started, 0.), (TouchPhase::Moved, -180.)] {
            cx.simulate_event(wheel(phase, dx));
            draw(cx);
        }
        let partial = cx
            .debug_bounds("new-session-overscroll-plus")
            .expect("plus appears during edge pull");
        cx.simulate_event(wheel(TouchPhase::Moved, -80.));
        draw(cx);
        let ready = cx.debug_bounds("new-session-overscroll-plus").unwrap();
        assert!(
            ready.size.width > partial.size.width,
            "plus grows toward the threshold"
        );
        assert!(ready.left() >= px(0.) && ready.right() <= px(390.));
        cx.simulate_event(wheel(TouchPhase::Ended, 0.));
        draw(cx);
        view.read_with(cx, |app, cx| {
            assert_eq!(app.tab_order.len(), 2, "physical lift creates the session");
            assert_eq!(app.active_tab, 1);
            assert_eq!(
                app.parked[0].preview_composer.read(cx).text(),
                "Keep this draft"
            );
            assert!(app.composer.read(cx).text().is_empty());
        });
        for (phase, dx) in [(TouchPhase::Moved, -300.), (TouchPhase::Ended, 0.)] {
            cx.simulate_event(wheel(phase, dx));
            draw(cx);
        }
        view.read_with(cx, |app, _| {
            assert_eq!(app.tab_order.len(), 2, "momentum does not create twice")
        });
        assert!(cx.debug_bounds("new-session-overscroll-plus").is_none());
    }

    #[gpui::test]
    fn new_session_overscroll_raw_lift_creates_but_existing_neighbor_does_not(
        cx: &mut TestAppContext,
    ) {
        let (view, cx) = mount(cx);
        view.update(cx, |app, cx| {
            app.panel_wheel(&wheel(TouchPhase::Started, 0.), cx);
            app.panel_wheel(&wheel(TouchPhase::Moved, -260.), cx);
            app.finish_panel_lift(cx);
            assert_eq!(
                app.tab_order.len(),
                2,
                "fallback raw lift creates the session"
            );
            assert!(!app.swipe.active);
            app.focus_tab(0, cx);
            app.panel_wheel(&wheel(TouchPhase::Started, 0.), cx);
            app.panel_wheel(&wheel(TouchPhase::Moved, -260.), cx);
            assert!(!app.new_session_overscroll_ready(true, true));
            app.panel_wheel(&wheel(TouchPhase::Ended, 0.), cx);
            assert_eq!(
                app.tab_order.len(),
                2,
                "existing neighbor is navigated, not created"
            );
            assert_eq!(app.active_tab, 1);
        });
    }

    #[gpui::test]
    fn new_session_overscroll_plus_button_creates_and_clears_the_pull(cx: &mut TestAppContext) {
        let (view, cx) = mount(cx);
        view.update(cx, |app, cx| {
            assert!(app.swipe.feed(-180., 0., std::time::Instant::now()));
            cx.notify();
        });
        draw(cx);
        let plus = cx.debug_bounds("new-session-overscroll-plus").unwrap();
        cx.simulate_click(plus.center(), Modifiers::default());
        draw(cx);
        view.read_with(cx, |app, _| {
            assert_eq!(app.tab_order.len(), 2);
            assert!(!app.swipe.active);
            assert_eq!(app.swipe.dx, 0.);
        });
        assert!(cx.debug_bounds("new-session-overscroll-plus").is_none());
    }

    #[gpui::test]
    fn new_session_overscroll_short_or_cancelled_drag_does_not_create(cx: &mut TestAppContext) {
        let (view, cx) = mount(cx);
        for (dx, ending) in [(-180., TouchPhase::Ended), (-300., TouchPhase::Cancelled)] {
            for (phase, dx) in [
                (TouchPhase::Started, 0.),
                (TouchPhase::Moved, dx),
                (ending, 0.),
            ] {
                cx.simulate_event(wheel(phase, dx));
                draw(cx);
            }
            view.read_with(cx, |app, _| assert_eq!(app.tab_order.len(), 1));
            assert!(cx.debug_bounds("new-session-overscroll-plus").is_none());
        }
    }
}
