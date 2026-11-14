//! Real app frames driven by protocol events and a deterministic reveal clock.
use super::*;
use gpui::{Modifiers, ScrollDelta, ScrollWheelEvent, TestAppContext, TouchPhase, point, size};
use jcode_mobile_core::ApiEvent;
use std::time::{Duration, Instant};

fn event(app: &mut MobileApp, event: ApiEvent) {
    app.session.apply(ConnectionOutput::Event(event));
}

fn text(value: &str) -> ApiEvent {
    ApiEvent::TextDelta { session_id: "stream-fixture".into(), text: value.into(), message_id: None }
}

fn reasoning(value: &str) -> ApiEvent {
    ApiEvent::ReasoningDelta { session_id: "stream-fixture".into(), text: value.into() }
}

fn fixture(window: &mut Window, cx: &mut Context<MobileApp>, now: Instant) -> MobileApp {
    let mut app = MobileApp::new_offline(SafeArea { top: 20., bottom: 0. }, window, cx);
    app.screen = Screen::Chat;
    app.device = Some((390., 844.));
    app.session.phase = ConnectionPhase::Connected;
    app.animate_stream_in_tests = true;
    app.stream_test_now = Some(now);
    app
}

#[gpui::test]
fn actual_events_paint_reasoning_text_tools_and_resumed_reasoning_in_order(cx: &mut TestAppContext) {
    cx.update(init);
    let now = Instant::now();
    let (view, cx) = cx.add_window_view(|window, cx| fixture(window, cx, now));
    cx.simulate_resize(size(px(390.), px(844.)));
    for value in [reasoning("First thought\n"), text("Answer before tool\n"),
        ApiEvent::ToolStart { session_id: "stream-fixture".into(), call_id: "ordered".into(), name: "bash".into() },
        reasoning("Second thought\n"), text("Final answer\n")]
    {
        view.update(cx, |app, cx| { event(app, value); cx.notify(); });
        cx.update(|window, cx| { window.draw(cx).clear(cx); });
    }
    let ids = view.read_with(cx, |app, _| {
        assert_eq!(app.session.transcript.len(), 5);
        assert_eq!(app.session.transcript[0].reasoning, "First thought\n");
        assert_eq!(app.session.transcript[1].text, "Answer before tool\n");
        assert_eq!(app.session.transcript[3].reasoning, "Second thought\n");
        app.session.transcript.iter().map(|e| e.id).collect::<Vec<_>>()
    });
    assert_eq!(ids, vec![1, 2, 3, 4, 5]);
    let rows: Vec<_> = ["entry-1", "entry-2", "entry-3", "entry-4", "entry-5"]
        .into_iter().map(|selector| cx.debug_bounds(selector).unwrap()).collect();
    for pair in rows.windows(2) {
        assert!(pair[0].bottom() <= pair[1].top(), "phase rows reordered or overlap: {pair:?}");
    }
    let tool = cx.debug_bounds("tool-ordered").unwrap();
    assert!(tool.top() >= rows[1].bottom());
    assert!(tool.bottom() <= rows[3].top());
    view.update(cx, |app, cx| {
        event(app, ApiEvent::ToolDone { session_id: "stream-fixture".into(), call_id: "ordered".into(), name: "bash".into(), output: "done".into(), error: None });
        cx.notify();
    });
    cx.update(|window, cx| { window.draw(cx).clear(cx); });
    assert_eq!(cx.debug_bounds("tool-ordered").unwrap(), tool, "late tool completion must not move its phase");
}

#[gpui::test]
fn fixed_clock_holds_partial_text_then_releases_a_whole_block(cx: &mut TestAppContext) {
    cx.update(init);
    let now = Instant::now();
    let raw = "Complete paragraph.\n\nUnfinished next paragraph with Unicode 界🦀";
    let (view, cx) = cx.add_window_view(|window, cx| {
        let mut app = fixture(window, cx, now);
        event(&mut app, text(raw));
        app
    });
    cx.simulate_resize(size(px(390.), px(844.)));
    cx.update(|window, cx| { window.draw(cx).clear(cx); });
    view.update(cx, |app, _| {
        let frame = app.stream_presentation.prepare(app.active_tab, &app.session, now, false);
        let entry = &app.session.transcript[0];
        let reduced = crate::system::reduce_motion();
        assert_eq!(&entry.text[..frame.rows[&entry.id].text_len],
            if reduced { raw } else { "Complete paragraph.\n\n" });
        assert_eq!(entry.text, raw, "presentation must never truncate reducer state");
        assert_eq!(frame.animating, !reduced);
        if reduced { assert_eq!(frame.rows[&entry.id].text_fade.amount, 0.); }
    });
    let partial_height = cx.debug_bounds("entry-1").unwrap().size.height;
    for (millis, full) in [(159, false), (160, true), (380, true)] {
        let at = now + Duration::from_millis(millis);
        view.update(cx, |app, cx| { app.stream_test_now = Some(at); cx.notify(); });
        cx.update(|window, cx| { window.draw(cx).clear(cx); });
        view.update(cx, |app, _| {
            let frame = app.stream_presentation.prepare(app.active_tab, &app.session, at, false);
            let entry = &app.session.transcript[0];
            let row = frame.rows[&entry.id];
            assert_eq!(row.text_len == raw.len(), full || crate::system::reduce_motion(), "idle release at {millis}ms");
            assert!(raw.is_char_boundary(row.text_len));
            if millis == 380 { assert!(!frame.animating); assert_eq!(row.text_fade.amount, 0.); }
        });
        let painted_height = cx.debug_bounds("entry-1").unwrap().size.height;
        if crate::system::reduce_motion() {
            assert_eq!(painted_height, partial_height, "instant text must already occupy its final layout");
        } else if full {
            assert!(painted_height > partial_height, "released block must reach actual app layout");
        } else {
            assert_eq!(painted_height, partial_height, "partial block must not paint early");
        }
    }
}

#[gpui::test]
fn completion_and_instant_presentation_flush_unreleased_content(cx: &mut TestAppContext) {
    cx.update(init);
    let now = Instant::now();
    let raw = "Still unfinished but completion must show every received character 界";
    let (view, cx) = cx.add_window_view(|window, cx| {
        let mut app = fixture(window, cx, now);
        event(&mut app, text(raw));
        app
    });
    cx.simulate_resize(size(px(390.), px(844.)));
    cx.update(|window, cx| { window.draw(cx).clear(cx); });
    view.update(cx, |app, _| {
        let frame = app.stream_presentation.prepare(app.active_tab, &app.session, now, false);
        let row = frame.rows[&app.session.transcript[0].id];
        assert_eq!(row.text_len, if crate::system::reduce_motion() { raw.len() } else { 0 });
        if crate::system::reduce_motion() {
            assert_eq!(row.text_fade.amount, 0.);
            assert!(!frame.animating);
            assert_eq!(app.stream_motion.shift(), px(0.));
        }
    });
    view.update(cx, |app, cx| {
        event(app, ApiEvent::TextDone { session_id: "stream-fixture".into(), message_id: None });
        cx.notify();
    });
    cx.update(|window, cx| { window.draw(cx).clear(cx); });
    let settled = cx.debug_bounds("entry-1").unwrap();
    assert!(settled.size.height > px(20.), "completion must paint unreleased text immediately");
    view.update(cx, |app, _| {
        let frame = app.stream_presentation.prepare(app.active_tab, &app.session, now, false);
        assert!(frame.rows.is_empty());
        assert!(!frame.animating);
        assert_eq!(app.session.transcript[0].text, raw);
        event(app, text(raw));
        // Exercise the actual render bypass. Accessibility uses this same
        // instant path, without changing a process-global setting in tests.
        app.animate_stream_in_tests = false;
    });
    view.update(cx, |_, cx| cx.notify());
    cx.update(|window, cx| { window.draw(cx).clear(cx); });
    assert_eq!(cx.debug_bounds("entry-2").unwrap().size.height, settled.size.height,
        "instant rendering must paint the same complete text as completion");
    view.update(cx, |app, _| {
        let instant = app.stream_presentation.prepare(app.active_tab, &app.session, now, true);
        let live = app.session.transcript.last().unwrap();
        assert_eq!(instant.rows[&live.id].text_len, live.text.len());
        assert_eq!(instant.rows[&live.id].text_fade.amount, 0.);
        assert!(!instant.animating);
    });
    view.update(cx, |app, cx| {
        event(app, ApiEvent::TurnDone { session_id: "stream-fixture".into(), pending_soft_interrupts: None });
        cx.notify();
    });
    cx.update(|window, cx| { window.draw(cx).clear(cx); });
    assert!(cx.debug_bounds("transcript-activity").is_none(), "completed turn removes the orb");
    assert_eq!(cx.debug_bounds("entry-2").unwrap().size.height, settled.size.height);
}

#[gpui::test]
fn upward_input_hands_off_positive_stream_shift_and_new_chunks_do_not_follow(cx: &mut TestAppContext) {
    cx.update(init);
    let now = Instant::now();
    let (view, cx) = cx.add_window_view(|window, cx| {
        let mut app = fixture(window, cx, now);
        event(&mut app, text(&"Existing transcript paragraph.\n\n".repeat(30)));
        event(&mut app, ApiEvent::TextDone { session_id: "stream-fixture".into(), message_id: None });
        app
    });
    cx.simulate_resize(size(px(390.), px(844.)));
    cx.update(|window, cx| { window.draw(cx).clear(cx); });
    view.update(cx, |app, cx| {
        app.stream_test_now = Some(now + Duration::from_millis(16));
        event(app, text(&"New complete paragraph grows the transcript.\n\n".repeat(4)));
        cx.notify();
    });
    cx.update(|window, cx| { window.draw(cx).clear(cx); });
    let (offset, shift, at) = view.read_with(cx, |app, _| {
        assert!(app.follow);
        let shift = app.stream_motion.shift();
        if crate::system::reduce_motion() {
            assert_eq!(shift, px(0.), "instant stream growth has no displacement to transfer");
            assert_eq!(-app.transcript_scroll.offset().y, app.transcript_scroll.max_offset().y);
        } else {
            assert!(shift > px(0.), "fixture must interrupt a positive visual displacement");
        }
        (app.transcript_scroll.offset(), shift, app.transcript_scroll.bounds().center())
    });
    cx.simulate_event(ScrollWheelEvent {
        position: at, delta: ScrollDelta::Pixels(point(px(0.), px(8.))),
        touch_phase: TouchPhase::Moved, modifiers: Modifiers::default(),
    });
    view.read_with(cx, |app, _| {
        assert!(!app.follow, "upward input releases following immediately");
        assert_eq!(app.transcript_scroll.offset().y, (offset.y + shift + px(8.)).min(px(0.)),
            "transfer displayed position before applying manual wheel delta");
    });
    cx.update(|window, cx| { window.draw(cx).clear(cx); });
    let released = view.read_with(cx, |app, _| app.transcript_scroll.offset());
    view.update(cx, |app, cx| {
        app.stream_test_now = Some(now + Duration::from_millis(500));
        event(app, text("A later complete chunk must leave the reader alone.\n\n"));
        cx.notify();
    });
    cx.update(|window, cx| { window.draw(cx).clear(cx); });
    view.read_with(cx, |app, _| {
        assert!(!app.follow);
        assert_eq!(app.stream_motion.shift(), px(0.));
        assert_eq!(app.transcript_scroll.offset(), released);
    });
}

#[gpui::test]
fn phone_widths_and_keyboard_resize_keep_windowed_stream_tail_materialized(cx: &mut TestAppContext) {
    cx.update(init);
    let now = Instant::now();
    let (view, cx) = cx.add_window_view(|window, cx| {
        let mut app = fixture(window, cx, now);
        event(&mut app, ApiEvent::History {
            session_id: "stream-fixture".into(), images: vec![],
            messages: (0..80).map(|n| serde_json::from_value(serde_json::json!({
                "role": "assistant",
                "content": format!("History entry {n}.\n\n{}", "Windowed history paragraph.\n\n".repeat(4)),
            })).unwrap()).collect(),
        });
        app
    });
    for (step, width) in [320., 390., 430.].into_iter().enumerate() {
        let start = now + Duration::from_secs(step as u64 * 2);
        view.update(cx, |app, cx| {
            app.device = Some((width, 844.));
            app.stream_test_now = Some(start);
            cx.notify();
        });
        cx.simulate_resize(size(px(width), px(844.)));
        cx.update(|window, cx| { window.draw(cx).clear(cx); });
        view.update(cx, |app, cx| {
            app.stream_test_now = Some(start + Duration::from_millis(16));
            event(app, text("Phone-sized streamed paragraph with wrapping words.\n\n"));
            cx.notify();
        });
        cx.update(|window, cx| { window.draw(cx).clear(cx); });
        view.read_with(cx, |app, _| {
            if crate::system::reduce_motion() {
                assert_eq!(app.stream_motion.shift(), px(0.), "resize must not introduce accessibility motion at {width}");
            } else {
                assert!(app.stream_motion.shift() > px(0.), "fixture must resize during active motion at {width}");
            }
            assert!(app.rendered_transcript_rows < 25, "history must remain windowed");
            assert_eq!(-app.transcript_scroll.offset().y, app.transcript_scroll.max_offset().y);
            assert_eq!(app.transcript_scroll.max_offset().x, px(0.));
        });
        assert!(cx.debug_bounds("entry-81").is_some(), "new live tail may not be culled at {width}");
        // A keyboard changes the real viewport while the reveal/motion clocks
        // remain deterministic. Native layout must publish the NEW maximum.
        view.update(cx, |app, cx| {
            app.device = Some((width, 544.));
            app.stream_test_now = Some(start + Duration::from_millis(32));
            cx.notify();
        });
        cx.simulate_resize(size(px(width), px(544.)));
        cx.update(|window, cx| { window.draw(cx).clear(cx); });
        let activity = cx.debug_bounds("transcript-activity").unwrap();
        let composer = cx.debug_bounds("chat-composer").unwrap();
        view.read_with(cx, |app, _| {
            assert_eq!(-app.transcript_scroll.offset().y, app.transcript_scroll.max_offset().y);
            assert!(activity.bottom() <= app.transcript_scroll.bounds().bottom());
            assert!(activity.bottom() <= composer.top(), "keyboard must not hide the activity row");
            assert_eq!(app.transcript_scroll.max_offset().x, px(0.));
        });
        view.update(cx, |app, cx| {
            app.stream_test_now = Some(start + Duration::from_millis(500));
            cx.notify();
        });
        cx.update(|window, cx| { window.draw(cx).clear(cx); });
        let tail = cx.debug_bounds("entry-81").unwrap();
        view.read_with(cx, |app, _| {
            assert_eq!(app.stream_motion.shift(), px(0.));
            assert!(tail.bottom() <= app.transcript_scroll.bounds().bottom());
            assert!(tail.right() <= px(width), "stream tail must remain inside phone width");
        });
    }
}

#[gpui::test]
fn actual_tab_switch_and_reset_do_not_replay_or_leak_live_prefixes(cx: &mut TestAppContext) {
    cx.update(init);
    let now = Instant::now();
    let first = "First tab unfinished output";
    let second = "Second tab unfinished output";
    let (view, cx) = cx.add_window_view(|window, cx| {
        let mut app = fixture(window, cx, now);
        event(&mut app, text(first));
        app
    });
    cx.simulate_resize(size(px(390.), px(844.)));
    cx.update(|window, cx| { window.draw(cx).clear(cx); });
    view.update(cx, |app, cx| {
        app.park_active(cx);
        app.active_tab = 1;
        app.tab_order.push(1);
        app.session.phase = ConnectionPhase::Connected;
        event(app, text(second));
        cx.notify();
    });
    cx.update(|window, cx| { window.draw(cx).clear(cx); });
    let second_bounds = cx.debug_bounds("entry-1").unwrap();
    assert!(second_bounds.size.height > px(20.));
    view.update(cx, |app, _| {
        let frame = app.stream_presentation.prepare(app.active_tab, &app.session, now, false);
        assert_eq!(frame.rows[&1].text_len, second.len(), "same entry ID in another tab must restore fully");
        assert!(!frame.animating);
        assert_eq!(app.stream_motion.shift(), px(0.));
    });
    view.update(cx, |app, cx| app.focus_tab(0, cx));
    cx.update(|window, cx| { window.draw(cx).clear(cx); });
    assert!(cx.debug_bounds("entry-1").unwrap().size.height > px(20.));
    view.update(cx, |app, _| {
        assert_eq!(app.session.transcript[0].text, first);
        let restored = app.stream_presentation.prepare(app.active_tab, &app.session, now, false);
        assert_eq!(restored.rows[&1].text_len, first.len());
        assert!(!restored.animating);
    });
    view.update(cx, |app, cx| {
        app.session.apply_intent(LocalIntent::Reset);
        cx.notify();
    });
    cx.update(|window, cx| { window.draw(cx).clear(cx); });
    assert!(cx.debug_bounds("entry-1").is_none());
    assert!(cx.debug_bounds("transcript-activity").is_none());
    view.update(cx, |app, _| {
        let frame = app.stream_presentation.prepare(app.active_tab, &app.session, now, false);
        assert!(frame.rows.is_empty());
        assert!(!frame.animating);
        assert_eq!(app.stream_motion.shift(), px(0.));
        assert_eq!(app.transcript_scroll.max_offset().y, px(0.));
    });
}

#[gpui::test]
fn replacement_and_cancellation_flush_the_actual_row_without_hidden_tail(cx: &mut TestAppContext) {
    cx.update(init);
    let now = Instant::now();
    let (view, cx) = cx.add_window_view(|window, cx| {
        let mut app = fixture(window, cx, now);
        event(&mut app, text("Original complete paragraph.\n\nOld unfinished suffix"));
        app
    });
    cx.simulate_resize(size(px(390.), px(844.)));
    cx.update(|window, cx| { window.draw(cx).clear(cx); });
    let replaced = "Replacement with Unicode 界🦀 and an unfinished ending";
    view.update(cx, |app, cx| {
        app.stream_test_now = Some(now + Duration::from_millis(16));
        event(app, ApiEvent::TextReplace { session_id: "stream-fixture".into(), text: replaced.into(), message_id: None });
        cx.notify();
    });
    cx.update(|window, cx| { window.draw(cx).clear(cx); });
    view.update(cx, |app, _| {
        let frame = app.stream_presentation.prepare(app.active_tab, &app.session, now + Duration::from_millis(16), false);
        assert_eq!(app.session.transcript[0].text, replaced);
        let row = frame.rows[&1];
        assert_eq!(row.text_len, if crate::system::reduce_motion() { replaced.len() } else { 0 },
            "replacement must use its own boundary, instantly complete under Reduce Motion");
        if crate::system::reduce_motion() {
            assert_eq!(row.text_fade.amount, 0.);
            assert!(!frame.animating);
        }
    });
    view.update(cx, |app, cx| {
        event(app, ApiEvent::SessionStatus { session_id: "stream-fixture".into(), status: "cancelled".into(), pending_soft_interrupts: None });
        cx.notify();
    });
    cx.update(|window, cx| { window.draw(cx).clear(cx); });
    assert!(cx.debug_bounds("transcript-activity").is_none());
    let row = cx.debug_bounds("entry-1").unwrap();
    assert!(row.size.height > px(20.), "cancellation must paint full replacement, not its held prefix");
    view.update(cx, |app, _| {
        let frame = app.stream_presentation.prepare(app.active_tab, &app.session, now + Duration::from_millis(16), false);
        assert!(frame.rows.is_empty());
        assert!(!frame.animating);
        assert_eq!(app.session.transcript[0].text, replaced);
        assert!(!app.session.transcript[0].is_streaming);
        assert!(row.bottom() <= app.transcript_scroll.bounds().bottom());
    });
}
