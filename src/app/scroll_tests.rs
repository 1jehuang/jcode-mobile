//! Regression coverage for following live tool calls without fighting input.
use super::*;
use gpui::{Modifiers, ScrollDelta, ScrollWheelEvent, TestAppContext, TouchPhase, point, size};

#[gpui::test]
fn live_tools_release_follow_on_small_upward_scroll(cx: &mut TestAppContext) {
    cx.update(init);
    let (view, cx) = cx.add_window_view(|window, cx| {
        let mut app = MobileApp::new_offline(SafeArea { top: 20., bottom: 0. }, window, cx);
        app.screen = Screen::Chat;
        app.device = Some((390., 844.));
        app.session.phase = ConnectionPhase::Connected;
        app.session.is_processing = true;
        app.session.transcript.push(TranscriptEntry {
            id: 1,
            role: Role::Assistant,
            text: "Streaming response\n\n".repeat(60),
            reasoning: String::new(),
            tool_calls: vec![jcode_mobile_core::ToolCall {
                id: "live-tool".into(),
                name: "bash".into(),
                input: r#"{"intent":"Inspect files"}"#.into(),
                output: String::new(),
                status: ToolStatus::Running,
            }],
            is_streaming: true,
            is_queued: false,
        });
        app
    });
    cx.simulate_resize(size(px(390.), px(844.)));
    cx.update(|window, cx| { window.draw(cx).clear(cx); });
    view.update(cx, |app, cx| {
        assert!(app.transcript_scroll.max_offset().y > px(100.));
        app.transcript_scroll.scroll_to_bottom();
        cx.notify();
    });
    cx.update(|window, cx| { window.draw(cx).clear(cx); });
    let at = view.read_with(cx, |app, _| app.transcript_scroll.bounds().center());
    cx.simulate_event(ScrollWheelEvent {
        position: at,
        delta: ScrollDelta::Pixels(point(px(0.), px(8.))),
        touch_phase: TouchPhase::Moved,
        modifiers: Modifiers::default(),
    });
    view.read_with(cx, |app, _| assert!(!app.follow, "small upward drag must release live follow"));
    cx.update(|window, cx| { window.draw(cx).clear(cx); });
    let before = view.read_with(cx, |app, _| app.transcript_scroll.offset());
    view.update(cx, |app, cx| {
        app.session.transcript[0].tool_calls[0].status = ToolStatus::Succeeded;
        app.session.transcript[0].text.push_str("\n\nNew streamed content");
        cx.notify();
    });
    cx.update(|window, cx| { window.draw(cx).clear(cx); });
    view.read_with(cx, |app, _| {
        assert!(!app.follow);
        assert_eq!(app.transcript_scroll.offset(), before, "tool updates must not pull the reader back down");
    });
}
