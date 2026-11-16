//! Headless GPUI geometry checks, not native pixel or shaping acceptance.
use super::*;
use gpui::{
    Context, Modifiers, Render, ScrollDelta, ScrollHandle, ScrollWheelEvent, TestAppContext,
    TouchPhase, Window, point,
};

struct Fixture {
    source: SharedString,
    width: f32,
    code_only: bool,
    scroll: ScrollHandle,
}

impl Render for Fixture {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::pure_black();
        let content = if self.code_only {
            code_block(&self.source, &theme, 0)
                .track_scroll(&self.scroll)
                .debug_selector(|| "code-pane".into())
                .into_any_element()
        } else {
            // Reasoning and answer coexist within one transcript row. Their
            // block-zero code panes must have independent persistent IDs.
            div()
                .w_full()
                .flex()
                .flex_col()
                .child(render_reasoning(&self.source, &theme))
                .child(render(&self.source, &theme))
                .into_any_element()
        };
        div().w(px(self.width)).whitespace_nowrap().child(
            div()
                .id("markdown-fixture")
                .w_full()
                .overflow_x_scroll()
                .when(!self.code_only, |d| d.track_scroll(&self.scroll))
                .child(content),
        )
    }
}

#[gpui::test]
fn long_indented_code_keeps_literal_lines_and_scrolls_to_the_end(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_, _| Fixture {
        source: format!(
            "{}{}\n  final_line",
            " ".repeat(64),
            "long_identifier".repeat(24)
        )
        .into(),
        width: 160.,
        code_only: true,
        scroll: ScrollHandle::new(),
    });
    for scale in [1., 2., 3.1] {
        let _settings = crate::system::settings_fixture(scale, false, false);
        for width in [160., 240., 320.] {
            view.update(cx, |view, cx| {
                view.width = width;
                cx.notify();
            });
            cx.update(|window, cx| window.draw(cx).clear(cx));
            let pane = cx.debug_bounds("code-pane").expect("code pane laid out");
            assert!(pane.size.width <= px(width));
            assert!(
                pane.size.height <= crate::system::fs(18.) * 2. + px(24.),
                "two literal lines must not become hundreds of indented soft wraps: {pane:?}"
            );
            let end = view.read_with(cx, |view, _| {
                assert!(
                    view.scroll.max_offset().x > px(width),
                    "long code must be horizontally reachable"
                );
                assert_eq!(view.scroll.max_offset().y, px(0.));
                view.scroll.set_offset(point(px(0.), px(0.)));
                -view.scroll.max_offset().x
            });
            cx.update(|window, cx| window.draw(cx).clear(cx));
            cx.simulate_event(ScrollWheelEvent {
                position: pane.center(),
                delta: ScrollDelta::Pixels(point(end, px(0.))),
                touch_phase: TouchPhase::Moved,
                modifiers: Modifiers::default(),
            });
            cx.update(|window, cx| window.draw(cx).clear(cx));
            view.read_with(cx, |view, _| {
                assert_eq!(
                    view.scroll.offset(),
                    point(end, px(0.)),
                    "a dispatched horizontal scroll must reach the code suffix"
                );
            });
        }
    }
}

#[gpui::test]
fn rich_prose_wraps_even_inside_a_nowrap_ancestor(cx: &mut TestAppContext) {
    let url = format!("https://example.test/{}", "longpath".repeat(20));
    let (view, cx) = cx.add_window_view(|_, _| Fixture {
        source: format!(
            "# **{url}**\n\n**{}** `{}界🦀`\n\n- {url}\n  - {url}\n999. {url}\n\n```\n    {url}\n```",
            "unbroken".repeat(24),
            "identifier".repeat(24)
        )
        .into(),
        width: 320.,
        code_only: false,
        scroll: ScrollHandle::new(),
    });
    for scale in [1., 2., 3.1] {
        let _settings = crate::system::settings_fixture(scale, false, false);
        let mut previous_height = px(0.);
        for width in [320., 240., 160.] {
            view.update(cx, |view, cx| {
                view.width = width;
                cx.notify();
            });
            cx.update(|window, cx| window.draw(cx).clear(cx));
            view.read_with(cx, |view, _| {
                assert_eq!(
                    view.scroll.max_offset().x,
                    px(0.),
                    "rich text must not widen the transcript"
                );
                let height = view.scroll.bounds().size.height;
                assert!(
                    height > crate::system::fs(21.) * 10.,
                    "long rich text must wrap, not silently clip"
                );
                assert!(
                    height >= previous_height,
                    "narrowing must retain all wrapped lines"
                );
                previous_height = height;
            });
        }
    }
}
