use super::*;
use crate::transcript_notice::{for_entry, is_prompt, parse};
use gpui::{TestAppContext, size};

const DONE: &str = "**Background task** `bg123` · `验证 tests` (`bash`) · ✓ completed · 7.1s · exit 0\n\n```text\nfull output\n```\n\n_Full output:_ `bg action=\"output\" task_id=\"bg123\"`";

#[test]
fn transcript_notice_compact_token_badges_are_approximate_and_bounded() {
    use crate::transcript_notice::token_badge;
    assert_eq!(token_badge(0), "~0 tok");
    assert_eq!(token_badge(999), "~999 tok");
    assert_eq!(token_badge(12_345), "~12.3k tok");
    assert_eq!(token_badge(1_234_567), "~1.2m tok");
}

#[gpui::test]
fn transcript_notice_real_events_render_without_history_and_expand_independently(cx: &mut TestAppContext) {
    use crate::app::navigation_coverage::{draw, tap};
    use jcode_mobile_core::{ApiEvent, ConnectionOutput};
    let _settings = crate::system::settings_fixture(2., false, false);
    fn selector(value: String) -> &'static str { Box::leak(value.into_boxed_str()) }
    cx.update(init);
    let (view, cx) = cx.add_window_view(lifecycle_coverage::fixture_app);
    let ids = view.update(cx, |app, cx| {
        app.session.transcript.clear();
        for message in ["Context compacted\n\n**Complete report**", "**Background task** `unterminated"] {
            app.session.apply(ConnectionOutput::Event(ApiEvent::Compacted {
                session_id: "test".into(), message: message.into(),
            }));
        }
        let ids = app.session.notices.iter().map(|notice| notice.id).collect::<Vec<_>>();
        cx.notify();
        ids
    });
    cx.simulate_resize(size(px(280.), px(844.)));
    cx.update(|window, _| window.set_emulate_touch_from_mouse(true));
    draw(cx);
    for id in &ids {
        let header = cx.debug_bounds(selector(format!("live-notice-{id}"))).expect("event notice visible without transcript");
        let summary = cx.debug_bounds(selector(format!("live-notice-{id}-summary"))).unwrap();
        let badge = cx.debug_bounds(selector(format!("live-notice-{id}-tokens"))).unwrap();
        assert!(summary.size.width > px(40.), "summary retains space beside compact badge");
        assert!(badge.right() <= header.right(), "badge stays inside narrow row");
    }
    tap(cx, Box::leak(format!("live-notice-{}", ids[0]).into_boxed_str()));
    draw(cx);
    assert!(cx.debug_bounds(selector(format!("live-notice-{}-detail", ids[0]))).is_some());
    assert!(cx.debug_bounds(selector(format!("live-notice-{}-detail", ids[1]))).is_none());
    tap(cx, Box::leak(format!("live-notice-{}", ids[0]).into_boxed_str()));
    draw(cx);
    assert!(cx.debug_bounds(selector(format!("live-notice-{}-detail", ids[0]))).is_none());
    view.read_with(cx, |app, _| assert!(app.session.transcript.is_empty()));
}

fn entry(role: Role, text: &str) -> TranscriptEntry {
    TranscriptEntry {
        id: 7,
        role,
        text: text.into(),
        reasoning: String::new(),
        tool_calls: vec![],
        is_streaming: false,
        is_queued: false,
    }
}

#[test]
fn transcript_notice_machine_envelopes_preserve_complete_markdown() {
    for text in [
        DONE,
        "**Background task progress** `bg123` · `bash`\n\n[#####-------] 42% · Running tests (reported)",
        "**Background task stalled** `stall01` · `bash` · no output or progress for 300s (running 1234s total)",
        "DM from fox: **Ready**\n\nValidation:\n- tests pass",
        "Cross-swarm DM from fox (swarm 'a'): ready",
        "#parser from owl: hello\nall detail",
        "broadcast from fox: hello",
        "⚠️ File activity: /workspace/文件.rs - moss previously read this file: read lines 1-1000\nMore details",
        "Agent fox finished their work and is ready for more.\n\nLane plan:\ncomplete\n\nFinal response:\n**Passed**",
        "🐝 **Swarm await finished**\n\nMember statuses:\nfox completed",
    ] {
        let notice = parse(text, false).expect(text);
        assert_eq!(notice.detail, text);
        assert!(!notice.initially_expanded);
        assert!(!is_prompt(&entry(Role::User, text)));
        assert!(for_entry(&entry(Role::Assistant, text)).is_none());
    }
}

#[test]
fn transcript_notice_does_not_classify_quoted_or_arbitrary_user_prose() {
    for text in [
        "Please explain background task progress",
        "> DM from fox: hello",
        " DM from fox: hello",
        "DM from my boss: please review",
        "Agent fox stopped.",
        "prefix\n**Background task** `bg123` · `bash`",
        "```text\nDM from fox: hello\n```",
        "**Background task** `unterminated",
        "**Background task** `` · `bash`",
        "\u{200b}\u{200b}swarm-tldr:unknown\u{200b}\u{200b}summary\nbody",
        "<system-reminder>Quoted user reminder</system-reminder>",
    ] {
        assert!(parse(text, false).is_none(), "{text}");
        assert!(is_prompt(&entry(Role::User, text)));
    }
    assert!(parse("Ordinary system notification", true).is_some());
}

#[test]
fn transcript_notice_zero_width_swarm_roundtrip() {
    for (state, expanded) in [("collapsed", false), ("expanded", true)] {
        let text = format!(
            "\u{200b}\u{200b}swarm-tldr:{state}\u{200b}\u{200b}Tests passed\n## Validation\n\nFull report 🦀"
        );
        let notice = parse(&text, false).unwrap();
        assert_eq!(notice.title, "Tests passed");
        assert_eq!(notice.detail, "## Validation\n\nFull report 🦀");
        assert_eq!(notice.initially_expanded, expanded);
    }
}

#[test]
fn transcript_notice_estimates_are_not_provider_usage_and_failed_tasks_are_not_success() {
    let notice = parse("abcd🦀", true).unwrap();
    assert_eq!(notice.estimated_tokens(), 2);
    assert_eq!(parse("", true).unwrap().estimated_tokens(), 0);
    let failed = DONE.replace("✓ completed", "✗ failed");
    assert_eq!(parse(&failed, false).unwrap().glyph, "✗");
    let tldr_only = "\u{200b}\u{200b}swarm-tldr:collapsed\u{200b}\u{200b}Only summary";
    assert_eq!(parse(tldr_only, false).unwrap().detail, "Only summary");
}

#[gpui::test]
fn transcript_notice_native_live_rows_keep_images_and_use_same_disclosure(cx: &mut TestAppContext) {
    use crate::app::navigation_coverage::{draw, tap};
    cx.update(init);
    let (view, cx) = cx.add_window_view(lifecycle_coverage::fixture_app);
    view.update(cx, |app, cx| {
        app.session.transcript = vec![entry(
            Role::User,
            "DM from fox: Tests pass\n\n**Validation**\n\n- all good",
        )];
        app.session.images.insert(
            7,
            vec![jcode_mobile_core::SessionImage::new(
                "image/png",
                vec![1, 2, 3],
            )],
        );
        cx.notify();
    });
    cx.simulate_resize(size(px(320.), px(844.)));
    cx.update(|window, _| window.set_emulate_touch_from_mouse(true));
    draw(cx);
    assert!(cx.debug_bounds("notice-7").is_some());
    assert!(cx.debug_bounds("notice-images-7").is_some());
    let header = cx.debug_bounds("notice-7").unwrap();
    assert!(
        header.size.height <= crate::system::fs(28.),
        "collapsed activity is one line"
    );
    let before = view.read_with(cx, |app, _| {
        transcript_layout::key(
            app.active_tab,
            &app.session.transcript[0],
            0,
            &app.expanded_tools,
            app.content_width(),
        )
    });
    tap(cx, "notice-7");
    draw(cx);
    assert!(cx.debug_bounds("notice-detail-7").is_some());
    assert!(cx.debug_bounds("notice-images-7").is_some());
    view.read_with(cx, |app, _| {
        let after = transcript_layout::key(
            app.active_tab,
            &app.session.transcript[0],
            0,
            &app.expanded_tools,
            app.content_width(),
        );
        assert_ne!(
            before, after,
            "notice disclosure invalidates measured heights"
        );
        assert_eq!(
            app.session
                .transcript
                .iter()
                .filter(|e| is_prompt(e))
                .count(),
            0
        );
    });
    tap(cx, "notice-7");
    draw(cx);
    assert!(cx.debug_bounds("notice-detail-7").is_none());
}

#[gpui::test]
fn transcript_notice_native_expand_collapse_works_for_synced_history(cx: &mut TestAppContext) {
    use crate::app::navigation_coverage::{draw, tap};
    use jcode_mobile_core::mobile_sync::{SyncedSession, SyncedTranscript};
    cx.update(init);
    let (view, cx) = cx.add_window_view(lifecycle_coverage::fixture_app);
    let session: SyncedSession = serde_json::from_value(serde_json::json!({
        "session_id":"notice-render", "source":"local", "live":true, "busy":false
    }))
    .unwrap();
    let photo = jcode_mobile_core::SessionImage::new("image/png", vec![1, 2, 3]);
    let (media_type, data) = photo.wire();
    let history = SyncedTranscript {
        session,
        messages: serde_json::from_value(serde_json::json!([
            {"seq":7,"role":"user","text":DONE,"images":[{"media_type":media_type,"data":data}]}
        ]))
        .unwrap(),
    };
    view.update(cx, |app, cx| {
        app.open_local_chat(history.session.clone(), None, cx);
        app.apply_local_poll(app.active_tab, Ok(history), None);
        cx.notify();
    });
    cx.simulate_resize(size(px(320.), px(844.)));
    cx.update(|window, _| window.set_emulate_touch_from_mouse(true));
    draw(cx);
    assert!(cx.debug_bounds("notice-7").is_some());
    assert!(cx.debug_bounds("notice-tokens-7").is_some());
    assert!(cx.debug_bounds("notice-images-7").is_some());
    assert!(cx.debug_bounds("notice-detail-7").is_none());
    tap(cx, "notice-7");
    draw(cx);
    assert!(cx.debug_bounds("notice-detail-7").is_some());
    assert!(cx.debug_bounds("notice-images-7").is_some());
    view.read_with(cx, |app, _| {
        assert!(!app.follow);
        assert!(app.expanded_tools.contains("notice:7"));
    });
    tap(cx, "notice-7");
    draw(cx);
    assert!(cx.debug_bounds("notice-detail-7").is_none());
}
