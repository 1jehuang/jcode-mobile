//! Drawn regression fixtures for synced transcripts, not just reducer assertions.
use super::*;
use gpui::{TestAppContext, point, size};
use jcode_mobile_core::SessionImage;
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
fn synced_structured_tool_expansion_starts_at_header_not_output_tail(cx: &mut TestAppContext) {
    use crate::app::navigation_coverage::{draw, tap};
    cx.update(init);
    let (view, cx) = cx.add_window_view(lifecycle_coverage::fixture_app);
    let mut history = snapshot();
    history.messages = serde_json::from_value(serde_json::json!([
        {"seq":0,"role":"user","text":"Inspect output"},
        {"seq":1,"role":"assistant","text":"","display_text":"",
         "tool_calls":[{"id":"reader-tool","name":"Read","input":{"path":"result.txt"}}]},
        {"seq":2,"role":"tool","text":"","display_text":"",
         "tool_results":[{"tool_use_id":"reader-tool","content":"Result line that wraps at narrow widths.\n".repeat(100)}]},
        {"seq":3,"role":"assistant","text":"Finished."}
    ])).unwrap();
    view.update(cx, |app, cx| {
        app.open_local_chat(history.session.clone(), None, cx);
        app.apply_local_poll(app.active_tab, Ok(history), None);
        cx.notify();
    });
    cx.simulate_resize(size(px(320.), px(844.)));
    cx.update(|window, _| window.set_emulate_touch_from_mouse(true));
    draw(cx);
    view.read_with(cx, |app, _| assert!(app.follow));
    tap(cx, "tool-reader-tool");
    draw(cx);
    let card = cx.debug_bounds("tool-reader-tool").unwrap();
    view.read_with(cx, |app, _| {
        assert!(!app.follow, "expanding a structured tool must enter reader mode just like legacy output");
        let viewport = app.transcript_scroll.bounds();
        assert!(card.top() >= viewport.top() && card.top() < viewport.bottom(),
            "the expanded tool header must remain visible instead of jumping to the last output line");
    });
}

#[gpui::test]
fn synced_expanded_tool_refreshes_failure_geometry_without_losing_disclosure(cx: &mut TestAppContext) {
    use crate::app::navigation_coverage::{draw, tap};
    cx.update(init);
    let (view, cx) = cx.add_window_view(lifecycle_coverage::fixture_app);
    let history = |failed: bool| {
        let mut history = snapshot();
        let mut messages = vec![serde_json::json!({
            "seq":7,"role":"assistant","text":"legacy summary","display_text":"Inspecting",
            "tool_calls":[{"id":"poll-card","name":"Read","input":{"path":"src/main.rs"}}]
        })];
        if failed {
            messages.push(serde_json::json!({
                "seq":9,"role":"tool","text":"legacy failure","display_text":"",
                "tool_results":[{"tool_use_id":"poll-card","content":"permission denied\n".repeat(12),"is_error":true}]
            }));
        }
        history.messages = serde_json::from_value(serde_json::Value::Array(messages)).unwrap();
        history
    };
    view.update(cx, |app, cx| {
        let initial = history(false);
        app.open_local_chat(initial.session.clone(), None, cx);
        app.apply_local_poll(app.active_tab, Ok(initial), None);
        cx.notify();
    });
    cx.simulate_resize(size(px(320.), px(844.)));
    cx.update(|window, _| window.set_emulate_touch_from_mouse(true));
    draw(cx);
    tap(cx, "tool-poll-card");
    draw(cx);
    let before = cx.debug_bounds("tool-poll-card").unwrap();
    let (id, key) = view.read_with(cx, |app, _| {
        let entry = &app.session.transcript[1];
        (entry.id, transcript_layout::key(app.active_tab, entry, 1, &app.expanded_tools, app.content_width()))
    });
    view.update(cx, |app, cx| {
        app.apply_local_poll(app.active_tab, Ok(history(true)), None);
        let entry = &app.session.transcript[1];
        assert_eq!(entry.id, id, "late results must keep the mixed-message tool phase identity");
        assert!(app.expanded_tools.contains("poll-card"));
        assert_ne!(key, transcript_layout::key(app.active_tab, entry, 1, &app.expanded_tools, app.content_width()));
        assert_eq!(entry.tool_calls[0].status, ToolStatus::Failed("permission denied\n".repeat(12)));
        assert_eq!(app.session.transcript.len(), 2, "result must update its card, not append legacy prose");
        cx.notify();
    });
    draw(cx);
    let after = cx.debug_bounds("tool-poll-card").unwrap();
    assert!(after.size.height > before.size.height, "expanded result must replace cached input geometry");
    let viewport = view.read_with(cx, |app, _| app.transcript_scroll.bounds());
    assert!(after.left() >= viewport.left() && after.right() <= viewport.right());
    tap(cx, "tool-poll-card");
    draw(cx);
    assert!(cx.debug_bounds("tool-poll-card").unwrap().size.height < before.size.height);
}

#[gpui::test]
fn synced_tool_result_image_renders_on_grouped_card_and_survives_identical_poll(cx: &mut TestAppContext) {
    use crate::app::navigation_coverage::draw;
    cx.update(init);
    let (view, cx) = cx.add_window_view(lifecycle_coverage::fixture_app);
    let pixels = image::RgbaImage::from_pixel(16, 8, image::Rgba([255, 0, 0, 255]));
    let mut bytes = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(pixels).write_to(&mut bytes, image::ImageFormat::Png).unwrap();
    let photo = SessionImage::new("image/png", bytes.into_inner());
    let (media_type, data) = photo.wire();
    let mut history = snapshot();
    history.messages = serde_json::from_value(serde_json::json!([
        {"seq":4,"role":"assistant","text":"legacy","display_text":"Drawing",
         "tool_calls":[{"id":"image-card","name":"image","input":null}]},
        {"seq":8,"role":"tool","text":"legacy result","display_text":"",
         "tool_results":[{"tool_use_id":"image-card","content":"generated"}],
         "images":[{"media_type":media_type,"data":data}]}
    ])).unwrap();
    let selector = Box::leak(format!("assistant-image-{}", photo.id).into_boxed_str());
    for poll in 0..2 {
        view.update(cx, |app, cx| {
            if poll == 0 { app.open_local_chat(history.session.clone(), None, cx); }
            app.apply_local_poll(app.active_tab, Ok(history.clone()), None);
            assert_eq!(app.session.transcript.len(), 2);
            let card = &app.session.transcript[1];
            assert_eq!(card.tool_calls[0].status, ToolStatus::Succeeded);
            assert_eq!(app.session.images_for(card.id), &[photo.clone()]);
            assert!(app.session.images_for(8).is_empty(), "result sequence is not a standalone image row");
            cx.notify();
        });
        cx.simulate_resize(size(px(390.), px(844.)));
        draw(cx);
        assert!(cx.debug_bounds("tool-image-card").is_some());
        let bounds = cx.debug_bounds(selector).expect("grouped result must render inline media");
        let viewport = view.read_with(cx, |app, _| app.transcript_scroll.bounds());
        assert!(bounds.size.height >= px(240.));
        assert!(bounds.left() >= viewport.left() && bounds.right() <= viewport.right());
        assert!(cx.debug_bounds("synced-output-8").is_none());
    }
}

#[gpui::test]
fn archived_structured_sync_uses_cloud_tool_cards_at_phone_widths(cx: &mut TestAppContext) {
    use crate::app::navigation_coverage::tap;
    cx.update(init);
    let (view, cx) = cx.add_window_view(lifecycle_coverage::fixture_app);
    view.update(cx, |app, cx| {
        let mut history = snapshot();
        history.messages = serde_json::from_value(serde_json::json!([
            {"seq":0,"role":"user","text":"Inspect the file"},
            {"seq":1,"role":"assistant","text":"Checking\n\n🔧 Read: path",
             "display_text":"Checking", "tool_calls":[
                 {"id":"synced-card","name":"Read_with_an_extremely_long_tool_name",
                  "input":{"file_path":"/workspace/long/".to_owned() + &"界🦀path".repeat(90)}}]},
            {"seq":2,"role":"tool","text":"legacy output", "display_text":"",
             "tool_results":[{"tool_use_id":"synced-card","content":"output line\n".repeat(15)}]},
            {"seq":3,"role":"assistant","text":"**Done**"}
        ])).unwrap();
        history.session.live = false;
        app.open_local_chat(history.session.clone(), None, cx);
        app.apply_local_poll(app.active_tab, Ok(history), None);
        assert!(!app.local_chat_ready(&app.session), "archived structured history is read-only");
        assert_eq!(app.session.transcript.len(), 4);
        assert_eq!(app.session.transcript[2].tool_calls[0].name, "Read_with_an_extremely_long_tool_name");
        assert!(app.session.transcript.iter().all(|e| e.role != Role::System));
        cx.notify();
    });
    for width in [320., 390., 430.] {
        view.update(cx, |app, cx| { app.device = Some((width, 844.)); cx.notify(); });
        cx.simulate_resize(size(px(width), px(844.)));
        cx.update(|window, cx| { window.set_emulate_touch_from_mouse(true); window.draw(cx).clear(cx); });
        let collapsed = cx.debug_bounds("tool-synced-card").expect("shared cloud tool control");
        let viewport = view.read_with(cx, |app, _| app.transcript_scroll.bounds());
        assert!(collapsed.left() >= viewport.left() && collapsed.right() <= viewport.right(),
            "tool card overflows {width}px phone: {collapsed:?} in {viewport:?}");
        assert!(cx.debug_bounds("synced-output-2").is_none(), "structured result must not render legacy System prose");
        tap(cx, "tool-synced-card");
        cx.update(|window, cx| { window.draw(cx).clear(cx); });
        assert!(cx.debug_bounds("tool-synced-card").unwrap().size.height > collapsed.size.height);
        tap(cx, "tool-synced-card");
        cx.update(|window, cx| { window.draw(cx).clear(cx); });
        assert_eq!(cx.debug_bounds("tool-synced-card").unwrap().size.height, collapsed.size.height);
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
