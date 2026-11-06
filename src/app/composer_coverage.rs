//! Composer and transcript controls exercised through the offline phone UI.
//! Assertions distinguish optimistic local intent from server acknowledgement.
use super::navigation_coverage::{draw, mount, tap};
use super::*;
use gpui::TestAppContext;

fn entry(id: u64, text: &str) -> TranscriptEntry {
    TranscriptEntry {
        id,
        role: Role::Assistant,
        text: text.into(),
        reasoning: String::new(),
        tool_calls: vec![],
        is_streaming: false,
        is_queued: false,
    }
}

#[gpui::test]
fn header_new_session_is_inert_without_account_and_parks_existing_draft(cx: &mut TestAppContext) {
    let (view, cx) = mount(cx);
    view.update(cx, |app, cx| {
        app.account = None;
        app.composer
            .update(cx, |input, cx| input.set_text("Keep header draft 👩🏽‍💻", cx));
        cx.notify();
    });
    tap(cx, "new-session");
    view.read_with(cx, |app, cx| {
        assert_eq!(app.tab_order.len(), 1);
        assert_eq!(app.composer.read(cx).text(), "Keep header draft 👩🏽‍💻");
    });
    view.update(cx, |app, cx| {
        app.account = Some(super::account_coverage::account());
        cx.notify();
    });
    tap(cx, "new-session");
    view.read_with(cx, |app, cx| {
        assert_eq!(app.tab_order.len(), 2);
        assert_eq!(app.active_tab, 1);
        assert!(app.composer.read(cx).text().is_empty());
        assert!(app.connection.is_none());
    });
    tap(cx, "tab-0");
    view.read_with(cx, |app, cx| {
        assert_eq!(app.composer.read(cx).text(), "Keep header draft 👩🏽‍💻");
    });
}

#[gpui::test]
fn composer_send_disabled_for_disconnected_missing_session_and_blank_input(
    cx: &mut TestAppContext,
) {
    let (view, cx) = mount(cx);
    for (phase, session_id, draft) in [
        (
            ConnectionPhase::Disconnected,
            Some("fixture"),
            "Keep disconnected draft",
        ),
        (
            ConnectionPhase::Connected,
            None,
            "Keep missing-session draft",
        ),
        (ConnectionPhase::Connected, Some("fixture"), "   "),
    ] {
        view.update(cx, |app, cx| {
            app.session.phase = phase;
            app.session.session_id = session_id.map(str::to_owned);
            app.composer
                .update(cx, |input, cx| input.set_text(draft, cx));
            cx.notify();
        });
        let (_, feedback) = haptics::capture(|| tap(cx, "send"));
        assert!(
            feedback.is_empty(),
            "disabled send must not acknowledge an action"
        );
        view.read_with(cx, |app, cx| {
            assert_eq!(app.composer.read(cx).text(), draft);
            assert!(app.session.transcript.is_empty());
            assert!(!app.session.is_processing);
            assert!(app.session.pending_interrupts.is_empty());
            assert!(app.connection.is_none());
        });
    }
}

#[gpui::test]
fn composer_send_records_local_message_then_queues_processing_followup(cx: &mut TestAppContext) {
    let (view, cx) = mount(cx);
    view.update(cx, |app, cx| {
        app.session.phase = ConnectionPhase::Connected;
        app.session.session_id = Some("offline-send-fixture".into());
        app.session.error_banner = Some("Old error".into());
        app.composer.update(cx, |input, cx| {
            input.set_text("  First local message  ", cx)
        });
        cx.notify();
    });
    let (_, feedback) = haptics::capture(|| tap(cx, "send"));
    assert_eq!(feedback, [Haptic::Light]);
    view.read_with(cx, |app, cx| {
        assert_eq!(app.composer.read(cx).text(), "");
        assert_eq!(app.session.transcript.len(), 1);
        let sent = &app.session.transcript[0];
        assert_eq!(sent.role, Role::User);
        assert_eq!(sent.text, "First local message");
        assert!(!sent.is_queued);
        assert!(app.session.is_processing && app.follow);
        assert!(app.session.error_banner.is_none());
        assert!(app.session.pending_interrupts.is_empty());
        assert!(
            app.connection.is_none(),
            "local intent must not create transport"
        );
    });
    view.update(cx, |app, cx| {
        app.composer.update(cx, |input, cx| {
            input.set_text("  Next local instruction  ", cx)
        });
        cx.notify();
    });
    draw(cx);
    assert!(
        cx.debug_bounds("stop").is_none(),
        "a processing draft offers send, not stop"
    );
    tap(cx, "send");
    view.read_with(cx, |app, cx| {
        assert_eq!(app.composer.read(cx).text(), "");
        assert_eq!(app.session.transcript.len(), 2);
        assert_eq!(app.session.transcript[1].text, "Next local instruction");
        assert!(app.session.transcript[1].is_queued);
        assert_eq!(app.session.pending_interrupts, ["Next local instruction"]);
        assert!(
            app.session.is_processing,
            "queueing is not a completion acknowledgement"
        );
        assert!(app.connection.is_none());
    });
    draw(cx);
    assert!(
        cx.debug_bounds("stop").is_some(),
        "clearing a processing draft reveals stop"
    );
}

#[gpui::test]
fn composer_stop_requires_ready_session_and_does_not_fake_cancel_ack(cx: &mut TestAppContext) {
    let (view, cx) = mount(cx);
    for (phase, session_id, ready) in [
        (ConnectionPhase::Disconnected, Some("fixture"), false),
        (ConnectionPhase::Connected, None, false),
        (ConnectionPhase::Connected, Some("fixture"), true),
    ] {
        view.update(cx, |app, cx| {
            app.session.phase = phase;
            app.session.session_id = session_id.map(str::to_owned);
            app.session.is_processing = true;
            app.session.pending_interrupts = vec!["Queued fixture".into()];
            app.composer.update(cx, |input, cx| input.set_text("", cx));
            cx.notify();
        });
        let (_, feedback) = haptics::capture(|| tap(cx, "stop"));
        if ready {
            assert_eq!(feedback, [Haptic::Medium]);
        } else {
            assert!(feedback.is_empty());
        }
        view.read_with(cx, |app, _| {
            assert!(
                app.session.is_processing,
                "stop is only a request until the server acknowledges it"
            );
            assert_eq!(app.session.pending_interrupts, ["Queued fixture"]);
            assert!(app.connection.is_none());
        });
    }
    view.update(cx, |app, cx| {
        app.session.is_processing = false;
        cx.notify();
    });
    draw(cx);
    assert!(
        cx.debug_bounds("stop").is_none(),
        "idle sessions do not expose stop"
    );
    assert!(cx.debug_bounds("send").is_some());
}

#[gpui::test]
fn transcript_tool_control_expands_and_collapses_visible_output(cx: &mut TestAppContext) {
    let (view, cx) = mount(cx);
    view.update(cx, |app, cx| {
        let mut message = entry(10, "Fixture tool result");
        message.tool_calls.push(jcode_mobile_core::ToolCall {
            id: "composer-fixture".into(),
            name: "Read".into(),
            input: "{\"path\":\"fixture.rs\"}".into(),
            output: "First output line\nSecond output line\nThird output line".into(),
            status: ToolStatus::Succeeded,
        });
        app.session.transcript.push(message);
        cx.notify();
    });
    draw(cx);
    let collapsed = cx.debug_bounds("tool-composer-fixture").unwrap();
    tap(cx, "tool-composer-fixture");
    draw(cx);
    let expanded = cx.debug_bounds("tool-composer-fixture").unwrap();
    assert!(
        expanded.size.height > collapsed.size.height,
        "expansion must reveal actual output layout"
    );
    view.read_with(cx, |app, _| {
        assert!(app.expanded_tools.contains("composer-fixture"))
    });
    tap(cx, "tool-composer-fixture");
    draw(cx);
    let restored = cx.debug_bounds("tool-composer-fixture").unwrap();
    assert_eq!(restored.size.height, collapsed.size.height);
    view.read_with(cx, |app, _| {
        assert!(!app.expanded_tools.contains("composer-fixture"))
    });
}

#[gpui::test]
fn transcript_jump_control_restores_follow_without_discarding_draft(cx: &mut TestAppContext) {
    let (view, cx) = mount(cx);
    view.update(cx, |app, cx| {
        app.session.transcript.push(entry(
            20,
            &"Long fixture transcript paragraph.\n\n".repeat(100),
        ));
        app.follow = false;
        app.composer
            .update(cx, |input, cx| input.set_text("Keep while jumping", cx));
        cx.notify();
    });
    draw(cx);
    view.update(cx, |app, cx| {
        assert!(app.transcript_scroll.max_offset().y > px(100.));
        app.transcript_scroll
            .set_offset(gpui::point(px(0.), px(0.)));
        app.follow = false;
        cx.notify();
    });
    tap(cx, "jump");
    view.read_with(cx, |app, cx| {
        assert!(
            app.follow,
            "jump requests animated following, not synchronous final position"
        );
        assert_eq!(app.composer.read(cx).text(), "Keep while jumping");
        assert_eq!(app.session.transcript.len(), 1);
    });
}

#[gpui::test]
fn transcript_and_empty_error_banners_dismiss_safely_offline(cx: &mut TestAppContext) {
    let (view, cx) = mount(cx);
    for with_history in [false, true] {
        view.update(cx, |app, cx| {
            app.session.transcript = if with_history {
                vec![entry(30, "Preserved history")]
            } else {
                vec![]
            };
            app.session.error_banner = Some("Synthetic connection failure".into());
            app.session.phase = ConnectionPhase::Failed {
                reason: "Offline fixture".into(),
            };
            app.session.session_id = Some("offline-banner-fixture".into());
            app.composer.update(cx, |input, cx| {
                input.set_text("Keep while reconnecting", cx)
            });
            cx.notify();
        });
        tap(cx, if with_history { "err" } else { "err-empty" });
        view.read_with(cx, |app, cx| {
            assert!(app.session.error_banner.is_none());
            assert_eq!(app.composer.read(cx).text(), "Keep while reconnecting");
            assert_eq!(app.session.transcript.len(), usize::from(with_history));
            assert_eq!(
                app.session.session_id.as_deref(),
                Some("offline-banner-fixture")
            );
            assert!(
                app.connection.is_none(),
                "banner reconnect must respect offline transport boundary"
            );
        });
        draw(cx);
        assert!(
            cx.debug_bounds(if with_history { "err" } else { "err-empty" })
                .is_none()
        );
    }
}

#[gpui::test]
fn metadata_picker_actions_preserve_each_tabs_unsent_draft(cx: &mut TestAppContext) {
    let (view, cx) = mount(cx);
    view.update(cx, |app, cx| {
        app.composer
            .update(cx, |input, cx| input.set_text("First tab draft", cx));
        app.new_tab(cx);
        app.session.working_dir = Some("/second-repo".into());
        app.session.model_name = Some("fixture-model".into());
        app.session.available_models = vec!["fixture-model".into()];
        app.composer
            .update(cx, |input, cx| input.set_text("Second tab draft", cx));
        cx.notify();
    });
    for selector in ["pill-model", "pill-account"] {
        tap(cx, selector);
        draw(cx);
        view.update(cx, |app, cx| {
            assert!(
                app.model_picker,
                "metadata control {selector} must open the picker"
            );
            app.feel.g.sheet_y = 0.;
            app.feel.g.edge_x = 0.;
            cx.notify();
        });
        tap(cx, "model-row-bare-0");
        view.read_with(cx, |app, cx| {
            assert!(!app.model_picker);
            assert_eq!(app.composer.read(cx).text(), "Second tab draft");
            assert!(app.session.transcript.is_empty());
        });
    }
    tap(cx, "location");
    tap(cx, "dir-dismiss");
    tap(cx, "tab-0");
    view.read_with(cx, |app, cx| {
        assert_eq!(app.composer.read(cx).text(), "First tab draft")
    });
    tap(cx, "location");
    tap(cx, "dir-dismiss");
    tap(cx, "tab-1");
    view.read_with(cx, |app, cx| {
        assert_eq!(app.composer.read(cx).text(), "Second tab draft");
        assert_eq!(app.session.working_dir.as_deref(), Some("/second-repo"));
        assert!(app.session.transcript.is_empty());
        assert!(app.connection.is_none());
    });
}
