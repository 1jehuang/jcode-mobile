//! Drawn regression fixtures for synced transcripts, not just reducer assertions.
use super::*;
use gpui::{TestAppContext, point, size};
use jcode_mobile_core::mobile_sync::{SyncedSession, SyncedTranscript};

fn snapshot() -> SyncedTranscript {
    let session: SyncedSession = serde_json::from_value(serde_json::json!({
        "session_id": "render-local", "source": "local", "live": true,
        "busy": false, "title": "Synced rendering regression"
    }))
    .unwrap();
    let messages = (0..90).map(|seq| {
        let (role, text) = match seq % 4 {
            0 => ("user", format!("Question {seq}: {}", "A long prompt with Unicode 界🦀 and a file path /workspace/project/src/session.rs. ".repeat(3))),
            1 => ("assistant", format!("## Reply {seq}\n\nA paragraph with **bold** and `inline_code`.\n\n- A wrapped item with enough words to fill a narrow phone screen\n- Another item\n\n```rust\n{}\n```", "let long_identifier = \"a_very_long_unbroken_string_that_must_not_expand_the_phone_viewport\";")),
            2 => ("tool", format!("Tool output {seq}\n{}\n{}", "output line with spaces\n".repeat(8), "x".repeat(160))),
            _ => ("system", format!("System notification {seq}: {}", "A notification with a few wrapped words. ".repeat(4))),
        };
        serde_json::json!({"seq":seq,"role":role,"text":text})
    }).collect::<Vec<_>>();
    SyncedTranscript {
        session,
        messages: serde_json::from_value(serde_json::Value::Array(messages)).unwrap(),
    }
}

#[gpui::test]
fn newly_opened_synced_chat_follows_tail_without_changing_previous_reader(cx: &mut TestAppContext) {
    cx.update(init);
    let (view, cx) = cx.add_window_view(lifecycle_coverage::fixture_app);
    view.update(cx, |app, cx| {
        let previous = app.active_tab;
        app.follow = false;
        let history = snapshot();
        app.open_local_chat(history.session.clone(), None, cx);
        let local = app.active_tab;
        assert!(
            app.follow,
            "new synced chat must follow its latest messages, not inherit reader mode"
        );
        assert!(
            !app.parked
                .iter()
                .find(|tab| tab.id == previous)
                .unwrap()
                .follow
        );
        app.apply_local_poll(local, Ok(history), None);
        app.focus_tab(previous, cx);
        assert!(
            !app.follow,
            "previous tab keeps its reading position policy"
        );
        app.focus_tab(local, cx);
        assert!(app.follow);
        cx.notify();
    });
    cx.simulate_resize(size(px(390.), px(844.)));
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
    view.read_with(cx, |app, _| {
        assert_eq!(
            -app.transcript_scroll.offset().y,
            app.transcript_scroll.max_offset().y
        )
    });
    assert!(cx.debug_bounds("entry-89").is_some());
}

#[gpui::test]
fn synced_history_tail_and_reader_geometry_stay_stable_across_polls(cx: &mut TestAppContext) {
    cx.update(init);
    let (view, cx) = cx.add_window_view(lifecycle_coverage::fixture_app);
    view.update(cx, |app, cx| {
        let snapshot = snapshot();
        app.open_local_chat(snapshot.session.clone(), None, cx);
        app.apply_local_poll(app.active_tab, Ok(snapshot), None);
        cx.notify();
    });
    for width in [320., 390., 430.] {
        view.update(cx, |app, cx| {
            app.device = Some((width, 844.));
            app.follow = true;
            cx.notify();
        });
        cx.simulate_resize(size(px(width), px(844.)));
        for _ in 0..3 {
            cx.update(|window, cx| {
                window.draw(cx).clear(cx);
            });
        }
        view.read_with(cx, |app, _| {
            assert_eq!(
                -app.transcript_scroll.offset().y,
                app.transcript_scroll.max_offset().y
            );
            assert!(app.transcript_scroll.max_offset().y > px(844.));
        });
        let tail = cx
            .debug_bounds("entry-89")
            .expect("synced tail must be materialized");
        let viewport = view.read_with(cx, |app, _| app.transcript_scroll.bounds());
        assert!(tail.bottom() <= viewport.bottom() + px(1.));
        assert!(
            tail.left() >= viewport.left() && tail.right() <= viewport.right(),
            "row overflow at {width}: {tail:?} in {viewport:?}"
        );
        view.update(cx, |app, cx| {
            app.stream_motion.release(&app.transcript_scroll);
            app.follow = false;
            app.transcript_scroll.set_offset(point(px(0.), px(-400.)));
            cx.notify();
        });
        for _ in 0..2 {
            cx.update(|window, cx| {
                window.draw(cx).clear(cx);
            });
        }
        let before = view.read_with(cx, |app, _| app.transcript_scroll.offset());
        view.update(cx, |app, cx| {
            app.apply_local_poll(app.active_tab, Ok(snapshot()), None);
            cx.notify();
        });
        for _ in 0..2 {
            cx.update(|window, cx| {
                window.draw(cx).clear(cx);
            });
        }
        view.read_with(cx, |app, _| {
            assert_eq!(
                app.transcript_scroll.offset(),
                before,
                "an unchanged sync poll must not jump the reader at {width}"
            )
        });
        let rows = [
            "entry-0", "entry-1", "entry-2", "entry-3", "entry-4", "entry-5", "entry-6", "entry-7",
        ]
        .into_iter()
        .filter_map(|selector| cx.debug_bounds(selector))
        .collect::<Vec<_>>();
        assert!(!rows.is_empty());
        for pair in rows.windows(2) {
            assert!(
                pair[0].bottom() <= pair[1].top(),
                "synced rows overlap at {width}: {pair:?}"
            );
        }
    }
}

#[gpui::test]
fn synced_tool_output_is_compact_expandable_and_keeps_its_height_cache_coherent(
    cx: &mut TestAppContext,
) {
    use crate::app::navigation_coverage::tap;
    cx.update(init);
    let (view, cx) = cx.add_window_view(lifecycle_coverage::fixture_app);
    view.update(cx, |app, cx| {
        let mut history = snapshot();
        history.messages = serde_json::from_value(serde_json::json!([
            {"seq":0,"role":"user","text":"Inspect this file"},
            {"seq":2,"role":"tool","text":"result line with Unicode 界🦀\n".repeat(300)},
            {"seq":3,"role":"assistant","text":"The useful final answer remains visible."}
        ]))
        .unwrap();
        app.open_local_chat(history.session.clone(), None, cx);
        app.apply_local_poll(app.active_tab, Ok(history), None);
        cx.notify();
    });
    cx.simulate_resize(size(px(390.), px(844.)));
    cx.update(|window, cx| {
        window.set_emulate_touch_from_mouse(true);
        window.draw(cx).clear(cx);
    });
    let collapsed = cx
        .debug_bounds("synced-output-2")
        .expect("output disclosure");
    assert_eq!(collapsed.size.height, px(44.));
    assert!(cx.debug_bounds("synced-output-body-2").is_none());
    let (closed_key, raw) = view.read_with(cx, |app, _| {
        let entry = &app.session.transcript[1];
        (
            transcript_layout::key(
                app.active_tab,
                entry,
                1,
                &app.expanded_tools,
                app.content_width(),
            ),
            entry.text.clone(),
        )
    });
    tap(cx, "synced-output-2-toggle");
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
    assert!(cx.debug_bounds("synced-output-body-2").is_some());
    assert!(cx.debug_bounds("synced-output-2").unwrap().size.height > collapsed.size.height);
    view.read_with(cx, |app, _| {
        assert!(
            !app.follow,
            "opening output is an explicit reading action, not a jump to its tail"
        );
        let entry = &app.session.transcript[1];
        assert_eq!(
            entry.text, raw,
            "display bounding must not mutate synced content"
        );
        let open_key = transcript_layout::key(
            app.active_tab,
            entry,
            1,
            &app.expanded_tools,
            app.content_width(),
        );
        assert_ne!(
            closed_key, open_key,
            "expanded geometry must not reuse the collapsed spacer"
        );
    });
    view.update(cx, |app, cx| {
        app.follow = false;
        app.stream_motion.release(&app.transcript_scroll);
        app.transcript_scroll.set_offset(point(px(0.), px(0.)));
        cx.notify();
    });
    tap(cx, "synced-output-2-toggle");
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
    assert!(cx.debug_bounds("synced-output-body-2").is_none());
    assert_eq!(
        cx.debug_bounds("synced-output-2").unwrap().size.height,
        px(44.)
    );
    view.read_with(cx, |app, _| assert_eq!(app.session.transcript[1].text, raw));
}
