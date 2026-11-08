//! Regression coverage for following live tool calls without fighting input.
use super::*;
use gpui::{Modifiers, ScrollDelta, ScrollWheelEvent, TestAppContext, TouchPhase, point, size};

/// Check the actual first painted layout, without pumping follow frames or
/// sleeping for the old easing clock to catch up. Network updates can be lone
/// events and orb frames must not be needed to reveal their tail.
#[gpui::test]
fn followed_stream_and_thinking_transitions_anchor_in_the_same_layout(cx: &mut TestAppContext) {
    cx.update(init);
    let (view, cx) = cx.add_window_view(|window, cx| {
        let mut app = MobileApp::new_offline(SafeArea { top: 20., bottom: 0. }, window, cx);
        app.screen = Screen::Chat;
        app.device = Some((390., 844.));
        app.session.phase = ConnectionPhase::Connected;
        app.session.is_processing = true;
        app.session.is_reasoning = true;
        app.session.transcript.push(TranscriptEntry {
            id: 1, role: Role::Assistant,
            text: "Initial response\n\n".repeat(60),
            reasoning: String::new(), tool_calls: Vec::new(),
            is_streaming: true, is_queued: false,
        });
        app
    });
    cx.simulate_resize(size(px(390.), px(844.)));
    cx.update(|window, cx| { window.draw(cx).clear(cx); });
    let activity_height = cx.debug_bounds("transcript-activity").unwrap().size.height;
    for step in 0..8 {
        view.update(cx, |app, cx| {
            app.session.is_reasoning = step % 2 == 0;
            app.session.transcript[0].tool_calls = if step % 3 == 1 {
                vec![jcode_mobile_core::ToolCall {
                    id: "transition-tool".into(), name: "read".into(),
                    input: r#"{"path":"fixture.rs"}"#.into(),
                    output: String::new(), status: ToolStatus::Running,
                }]
            } else { Vec::new() };
            app.session.transcript[0].text.push_str("\n\nA streamed paragraph with enough words to wrap on a phone.");
            cx.notify();
        });
        cx.update(|window, cx| { window.draw(cx).clear(cx); });
        view.read_with(cx, |app, _| {
            assert!(app.transcript_scroll.max_offset().y > px(100.));
            assert_eq!(-app.transcript_scroll.offset().y, app.transcript_scroll.max_offset().y,
                "streamed update must be pinned in its first painted layout");
        });
        let activity = cx.debug_bounds("transcript-activity").unwrap();
        assert_eq!(activity.size.height, activity_height, "orb state must not change row height");
        let viewport = view.read_with(cx, |app, _| app.transcript_scroll.bounds());
        assert!(activity.bottom() <= viewport.bottom(), "thinking row hidden below viewport");
        assert!(cx.debug_bounds("jump").is_none());
        let before = view.read_with(cx, |app, _| app.transcript_scroll.offset());
        view.update(cx, |_, cx| cx.notify());
        cx.update(|window, cx| { window.draw(cx).clear(cx); });
        view.read_with(cx, |app, _| assert_eq!(app.transcript_scroll.offset(), before,
            "an unrelated redraw must not continue settling the transcript"));
    }
    view.update(cx, |app, cx| {
        app.session.is_reasoning = false;
        app.session.is_processing = false;
        cx.notify();
    });
    cx.update(|window, cx| { window.draw(cx).clear(cx); });
    assert!(cx.debug_bounds("transcript-activity").is_none());
    view.read_with(cx, |app, _| assert_eq!(-app.transcript_scroll.offset().y,
        app.transcript_scroll.max_offset().y, "removing the activity footer must remain pinned"));
    // Viewport changes (including keyboard presentation) use the new maximum,
    // not a delayed chase against the pre-keyboard geometry.
    cx.simulate_resize(size(px(390.), px(544.)));
    cx.update(|window, cx| { window.draw(cx).clear(cx); });
    view.read_with(cx, |app, _| assert_eq!(-app.transcript_scroll.offset().y,
        app.transcript_scroll.max_offset().y));
}

#[gpui::test]
fn tail_anchor_materializes_cached_bottom_on_first_jump_and_append(cx: &mut TestAppContext) {
    cx.update(init);
    let (view, cx) = cx.add_window_view(|window, cx| {
        let mut app = MobileApp::new_offline(SafeArea { top: 20., bottom: 0. }, window, cx);
        app.screen = Screen::Chat;
        app.device = Some((390., 844.));
        app.session.phase = ConnectionPhase::Connected;
        app.follow = false;
        for id in 0..40 {
            app.session.transcript.push(TranscriptEntry {
                id, role: Role::Assistant, text: format!("Message {id}\n\n{}", "Measured content\n\n".repeat(3)),
                reasoning: String::new(), tool_calls: Vec::new(),
                is_streaming: false, is_queued: false,
            });
        }
        app
    });
    cx.simulate_resize(size(px(390.), px(844.)));
    // Measure history at the top, then exercise a jump beyond overscan.
    cx.update(|window, cx| { window.draw(cx).clear(cx); });
    view.update(cx, |app, cx| { app.follow = true; cx.notify(); });
    cx.update(|window, cx| { window.draw(cx).clear(cx); });
    assert!(cx.debug_bounds("entry-39").is_some(), "cached destination must not be a spacer");
    view.read_with(cx, |app, _| {
        assert!(app.rendered_transcript_rows < 20, "tail anchoring must retain windowing");
        assert_eq!(-app.transcript_scroll.offset().y, app.transcript_scroll.max_offset().y);
    });
    view.update(cx, |app, cx| {
        app.transcript_scroll.set_offset(point(px(0.), px(0.)));
        app.session.transcript.push(TranscriptEntry {
            id: 40, role: Role::Assistant, text: "New short tail".into(),
            reasoning: String::new(), tool_calls: Vec::new(),
            is_streaming: true, is_queued: false,
        });
        cx.notify();
    });
    cx.update(|window, cx| { window.draw(cx).clear(cx); });
    for selector in ["entry-38", "entry-39", "entry-40"] {
        assert!(cx.debug_bounds(selector).is_some(), "visible {selector} must not become a spacer after append");
    }
    view.read_with(cx, |app, _| assert_eq!(-app.transcript_scroll.offset().y, app.transcript_scroll.max_offset().y));
}

#[gpui::test]
fn phone_sign_in_offers_apple_and_google_before_email_code(cx: &mut TestAppContext) {
    cx.update(init);
    let (view, cx) = cx.add_window_view(|window, cx| {
        let mut app = MobileApp::new_offline(SafeArea { top: 20., bottom: 0. }, window, cx);
        app.screen = Screen::SignIn;
        app.device = Some((390., 844.));
        app
    });
    cx.simulate_resize(size(px(390.), px(844.)));
    cx.update(|window, cx| { window.draw(cx).clear(cx); });
    let apple = cx.debug_bounds("sign-in-with-apple").expect("Apple sign-in must be offered");
    let google = cx.debug_bounds("google").expect("Google must accompany Apple on phones");
    assert!(apple.bottom() <= google.top(), "provider buttons must not overlap");
    assert!(google.left() >= px(0.) && google.right() <= px(390.));
    view.update(cx, |app, cx| {
        app.login_token = Some("fixture".into());
        cx.notify();
    });
    cx.update(|window, cx| { window.draw(cx).clear(cx); });
    assert!(cx.debug_bounds("sign-in-with-apple").is_none());
    assert!(cx.debug_bounds("google").is_none());
}

#[gpui::test]
fn attached_cloud_history_and_footer_stay_inside_phone(cx: &mut TestAppContext) {
    cx.update(init);
    let (view, cx) = cx.add_window_view(|window, cx| {
        let mut app = MobileApp::new_offline(SafeArea { top: 20., bottom: 0. }, window, cx);
        app.screen = Screen::Chat;
        app.session.phase = ConnectionPhase::Connected;
        app.session.session_id = Some("cloud-render-fixture".into());
        app.session.model_name = Some("gpt-6.1".into());
        app.session.provider_name = Some("anthropic".into());
        app.session.reasoning_effort = Some("medium".into());
        app.session.account_label = Some("a-very-long-account-label@example.test".into());
        app.session.working_dir = Some(format!("/home/cloud/projects/{}", "long-repository-name".repeat(5)));
        app.session.transcript.push(TranscriptEntry {
            id: 1, role: Role::Assistant,
            text: format!("The **answer** wraps normally.\n\n```rust\n{}\n```\n\nFinal paragraph.", "long_identifier_".repeat(80)),
            reasoning: "**Adding TODO initialization**\n\n**Preparing type-coercion coverage**".into(),
            tool_calls: Vec::new(), is_streaming: false, is_queued: false,
        });
        app
    });
    for width in [320., 390., 430.] {
        view.update(cx, |app, cx| { app.device = Some((width, 844.)); cx.notify(); });
        cx.simulate_resize(size(px(width), px(844.)));
        for _ in 0..3 { cx.update(|window, cx| { window.draw(cx).clear(cx); }); }
        let composer = cx.debug_bounds("chat-composer").unwrap();
        let metadata = cx.debug_bounds("chat-metadata").unwrap();
        assert!(composer.left() >= px(0.) && composer.right() <= px(width), "composer overflow: {composer:?}");
        assert!(metadata.left() >= composer.left() && metadata.right() <= composer.right());
        for id in ["pill-model", "pill-effort", "pill-account", "location"] {
            let bounds = cx.debug_bounds(id).unwrap();
            assert!(bounds.left() >= metadata.left() && bounds.right() <= metadata.right(), "{id} clipped at {width}: {bounds:?} in {metadata:?}");
            assert!(bounds.top() >= metadata.top() && bounds.bottom() <= metadata.bottom());
        }
        #[cfg(not(target_os = "ios"))]
        assert!(cx.debug_bounds("microphone").is_none(), "unsupported desktop microphone must not be offered");
        view.read_with(cx, |app, _| {
            assert_eq!(app.transcript_scroll.max_offset().x, px(0.), "code must not widen the transcript");
            assert!(app.transcript_scroll.bounds().bottom() <= composer.top(), "transcript overlaps composer");
        });
    }
}

#[gpui::test]
fn drawer_footer_is_compact_account_only_and_opens_settings(cx: &mut TestAppContext) {
    cx.update(init);
    let (view, cx) = cx.add_window_view(|window, cx| {
        let mut app = MobileApp::new_offline(SafeArea { top: 20., bottom: 0. }, window, cx);
        app.screen = Screen::Chat;
        app.device = Some((390., 844.));
        app.drawer_open = true;
        app.drawer_t = 1.;
        app
    });
    cx.simulate_resize(size(px(390.), px(844.)));
    cx.update(|window, cx| { window.draw(cx).clear(cx); });
    let account = cx.debug_bounds("drawer-account").unwrap();
    let sessions = cx.debug_bounds("drawer-sessions").unwrap();
    assert_eq!(account.size.height, px(44.), "keep a compact, touch-friendly account row");
    assert!(account.top() - sessions.bottom() <= px(4.), "no action bar between list and account");
    assert!(cx.debug_bounds("drawer-new-thread").is_none());
    assert!(cx.debug_bounds("drawer-refresh").is_none());
    cx.simulate_click(account.center(), Modifiers::default());
    view.read_with(cx, |app, _| {
        assert!(matches!(app.screen, Screen::Settings));
        assert!(!app.drawer_open);
    });
}

#[gpui::test]
fn drawer_scroll_does_not_move_transcript_or_release_follow(cx: &mut TestAppContext) {
    cx.update(init);
    let (view, cx) = cx.add_window_view(|window, cx| {
        let mut app = MobileApp::new_offline(SafeArea { top: 20., bottom: 0. }, window, cx);
        app.screen = Screen::Chat;
        app.device = Some((390., 844.));
        app.session.transcript.push(TranscriptEntry {
            id: 1,
            role: Role::Assistant,
            text: "Long transcript\n\n".repeat(100),
            reasoning: String::new(),
            tool_calls: Vec::new(),
            is_streaming: false,
            is_queued: false,
        });
        app.drawer_open = true;
        app.drawer_t = 1.;
        app
    });
    cx.simulate_resize(size(px(390.), px(844.)));
    cx.update(|window, cx| { window.draw(cx).clear(cx); });
    view.update(cx, |app, cx| {
        assert!(app.transcript_scroll.max_offset().y > px(100.));
        app.transcript_scroll.scroll_to_bottom();
        app.follow = true;
        cx.notify();
    });
    cx.update(|window, cx| { window.draw(cx).clear(cx); });
    let before = view.read_with(cx, |app, _| app.transcript_scroll.offset());
    // Empty list (at its boundary), pinned search/actions, and the scrim all
    // belong to the modal drawer, not the transcript behind it.
    for (x, y) in [(100., 400.), (100., 36.), (100., 760.), (370., 400.)] {
        for delta in [8., -8.] {
            for touch_phase in [TouchPhase::Moved, TouchPhase::Ended, TouchPhase::Cancelled] {
                cx.simulate_event(ScrollWheelEvent {
                    position: point(px(x), px(y)),
                    delta: ScrollDelta::Pixels(point(px(0.), px(delta))),
                    touch_phase,
                    modifiers: Modifiers::default(),
                });
                cx.update(|window, cx| { window.draw(cx).clear(cx); });
                view.read_with(cx, |app, _| {
                    assert!(app.follow, "drawer input must not release transcript follow");
                    assert_eq!(app.transcript_scroll.offset(), before, "drawer input must not scroll transcript");
                });
            }
        }
    }
}

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
