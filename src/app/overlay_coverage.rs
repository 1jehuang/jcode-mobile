//! Offline overlay semantic inventory. Raw touch and clipboard use GPUI's test platform only.
use super::*;
use crate::app::{SafeArea, init};
use gpui::{PlatformInput, TestAppContext, TouchEvent, TouchId, VisualTestContext, point, size};
use jcode_mobile_core::TranscriptEntry;
use std::time::Duration;

const DRAFT: &str = "Preserve composer 👩🏽‍💻 e\u{301}";
const ANSWER: &str = "Answer 👩🏽‍💻 e\u{301}\nsecond line";

fn entry(id: u64, role: Role, text: &str) -> TranscriptEntry {
    TranscriptEntry {
        id,
        role,
        text: text.into(),
        reasoning: String::new(),
        tool_calls: vec![],
        is_streaming: false,
        is_queued: false,
    }
}
fn setup(cx: &mut TestAppContext) -> (Entity<MobileApp>, &mut VisualTestContext) {
    cx.update(init);
    let (view, cx) = cx.add_window_view(|window, cx| {
        let mut app = MobileApp::new_offline(
            SafeArea {
                top: 20.,
                bottom: 0.,
            },
            window,
            cx,
        );
        app.screen = Screen::Chat;
        app.device = Some((390., 844.));
        app.session.transcript = vec![
            entry(1, Role::Assistant, "No prompt yet"),
            entry(2, Role::User, "First prompt"),
            entry(3, Role::Assistant, ANSWER),
            entry(4, Role::User, "Second prompt"),
        ];
        app.composer
            .update(cx, |input, cx| input.set_text(DRAFT, cx));
        app
    });
    cx.simulate_resize(size(px(390.), px(844.)));
    draw(cx);
    (view, cx)
}
fn draw(cx: &mut VisualTestContext) {
    cx.update(|window, cx| window.draw(cx).clear(cx));
}
fn touch(cx: &mut VisualTestContext, phase: TouchPhase, at: Point<Pixels>) {
    cx.update(|window, cx| {
        window.dispatch_event(
            PlatformInput::Touch(TouchEvent {
                id: TouchId(72),
                phase,
                position: at,
                predicted_position: None,
                force: Some(0.5),
            }),
            cx,
        );
        window.draw(cx).clear(cx);
    });
}
fn tap(cx: &mut VisualTestContext, selector: &'static str) {
    let at = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("missing {selector}"))
        .center();
    touch(cx, TouchPhase::Started, at);
    touch(cx, TouchPhase::Ended, at);
}
fn open_menu(view: &Entity<MobileApp>, cx: &mut VisualTestContext, id: u64) {
    view.update(cx, |app, cx| {
        app.feel.menu = Some(MsgMenu {
            entry: id,
            at: point(px(160.), px(240.)),
            closing: false,
        });
        app.feel.menu_anim = 1.;
        cx.notify();
    });
    draw(cx);
}
fn settle_keyboard(view: &Entity<MobileApp>, cx: &mut VisualTestContext) {
    draw(cx);
    view.update(cx, |app, cx| {
        app.inset_anim = None;
        cx.notify();
    });
    draw(cx);
}

#[gpui::test]
fn retry_eligibility_uses_preceding_prompt_and_connection_state(cx: &mut TestAppContext) {
    let (view, cx) = setup(cx);
    view.update(cx, |app, _| {
        assert_eq!(app.retry_text(1), None);
        assert_eq!(app.retry_text(999), None);
        assert_eq!(app.retry_text(2).as_deref(), Some("First prompt"));
        assert_eq!(app.retry_text(3).as_deref(), Some("First prompt"));
        assert_eq!(app.retry_text(4).as_deref(), Some("Second prompt"));
        assert_eq!(app.entry_text(3).as_deref(), Some(ANSWER));
        for phase in [
            ConnectionPhase::Disconnected,
            ConnectionPhase::Connecting,
            ConnectionPhase::Reconnecting { attempt: 1 },
            ConnectionPhase::Failed {
                reason: "fixture".into(),
            },
        ] {
            app.session.phase = phase;
            assert!(!app.can_retry_entry(3));
        }
        app.session.phase = ConnectionPhase::Connected;
        assert!(app.can_retry_entry(3));
        assert!(!app.can_retry_entry(1));
        assert!(!app.can_retry_entry(999));
        app.session.is_processing = true;
        assert!(!app.can_retry_entry(3));
    });
}

#[gpui::test]
fn raw_message_hold_copy_and_cancel_preserve_transcript_and_draft(cx: &mut TestAppContext) {
    let (view, cx) = setup(cx);
    draw(cx);
    let at = view.read_with(cx, |app, _| {
        app.feel
            .rec
            .borrow()
            .hits
            .iter()
            .find(|(target, _)| *target == Target::Message(3))
            .expect("message hit")
            .1
            .center()
    });
    touch(cx, TouchPhase::Started, at);
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_millis(501));
    cx.run_until_parked();
    draw(cx);
    touch(cx, TouchPhase::Ended, at);
    view.read_with(cx, |app, _| {
        assert_eq!(app.feel.menu.as_ref().map(|m| m.entry), Some(3))
    });
    tap(cx, "msg-copy");
    cx.update(|_, cx| {
        assert_eq!(
            cx.read_from_clipboard().and_then(|item| item.text()),
            Some(ANSWER.into())
        )
    });
    view.read_with(cx, |app, cx| {
        assert!(app.feel.menu.as_ref().unwrap().closing);
        assert_eq!(app.session.transcript.len(), 4);
        assert_eq!(app.composer.read(cx).text(), DRAFT);
    });
    view.update(cx, |app, cx| {
        app.feel.menu = None;
        app.feel.menu_anim = 0.;
        cx.notify();
    });
    draw(cx);
    touch(cx, TouchPhase::Started, at);
    touch(cx, TouchPhase::Cancelled, at);
    view.read_with(cx, |app, _| {
        assert!(app.feel.menu.is_none());
        assert_eq!(app.session.transcript.len(), 4);
        assert!(!app.session.is_processing);
    });
}

#[gpui::test]
fn selection_is_read_only_copyable_and_done_releases_focus(cx: &mut TestAppContext) {
    let (view, cx) = setup(cx);
    open_menu(&view, cx, 3);
    tap(cx, "msg-select");
    let input = view.read_with(cx, |app, cx| {
        let input = app.feel.selection.clone().expect("selection overlay");
        assert!(input.read(cx).read_only);
        assert_eq!(input.read(cx).selected_text(), ANSWER);
        assert!(!app.scene().menus);
        input
    });
    cx.update(|window, cx| {
        input.update(cx, |input, cx| {
            input.sim_insert("must not replace", window, cx);
            input.sim_backspace(window, cx);
        })
    });
    assert_eq!(
        input.read_with(cx, |input, _| input.text().to_owned()),
        ANSWER
    );
    tap(cx, "selection-all");
    tap(cx, "selection-copy");
    cx.update(|_, cx| {
        assert_eq!(
            cx.read_from_clipboard().and_then(|item| item.text()),
            Some(ANSWER.into())
        )
    });
    // The selection modal covers the Git pull handle still recorded by the
    // chat underneath. Raw touches over the handle must reach the modal and
    // never claim the Git gesture.
    let git = cx.debug_bounds("git-pull-handle").unwrap();
    touch(cx, TouchPhase::Started, git.center());
    touch(cx, TouchPhase::Ended, git.center());
    view.read_with(cx, |app, _| {
        assert!(app.feel.selection.is_some(), "touch over handle stays in the modal");
        assert!(!app.git_info.open, "selection must not activate Git behind it");
    });
    tap(cx, "selection-done");
    cx.update(|window, cx| {
        view.read_with(cx, |app, cx| {
            assert!(app.feel.selection.is_none());
            assert!(
                !app.git_info.open,
                "selection Done must not activate Git behind it"
            );
            assert!(!input.read(cx).is_focused(window));
            assert_eq!(app.composer.read(cx).text(), DRAFT);
            assert_eq!(app.session.transcript[2].text, ANSWER);
        })
    });
    assert!(cx.debug_bounds("selection-done").is_none());
}

#[gpui::test]
fn raw_disabled_retry_is_inert_and_enabled_retry_resends_correct_prompt(cx: &mut TestAppContext) {
    let (view, cx) = setup(cx);
    open_menu(&view, cx, 3);
    tap(cx, "msg-retry");
    view.read_with(cx, |app, _| {
        assert_eq!(app.session.transcript.len(), 4);
        assert!(!app.session.is_processing);
    });
    view.update(cx, |app, cx| {
        app.session.phase = ConnectionPhase::Connected;
        cx.notify();
    });
    open_menu(&view, cx, 3);
    tap(cx, "msg-retry");
    view.read_with(cx, |app, cx| {
        assert_eq!(app.session.transcript.last().unwrap().text, "First prompt");
        assert!(app.session.is_processing);
        assert!(app.feel.menu.as_ref().unwrap().closing);
        assert_eq!(app.composer.read(cx).text(), DRAFT);
        assert!(app.connection.is_none());
    });
}

#[gpui::test]
fn rename_save_cancel_empty_unicode_and_settled_keyboard_viewports(cx: &mut TestAppContext) {
    let (view, cx) = setup(cx);
    for (width, height) in [(320., 568.), (430., 700.), (390., 844.)] {
        view.update(cx, |app, cx| {
            app.device = Some((width, height));
            app.session.session_title = Some("Original".into());
            cx.notify();
        });
        cx.simulate_resize(size(px(width), px(height)));
        for (text, save) in [
            ("Not saved", false),
            ("  工作 👩🏽‍💻 e\u{301}  ", true),
            (" \n\t ", true),
        ] {
            view.update(cx, |app, cx| {
                app.feel_rename_tab(app.active_tab, cx);
                app.feel
                    .rename
                    .as_ref()
                    .unwrap()
                    .1
                    .update(cx, |input, cx| input.set_text(text, cx));
            });
            settle_keyboard(&view, cx);
            let before = view.read_with(cx, |app, _| app.session.session_title.clone());
            let visible_bottom = cx.update(|window, _| window.viewport_size().height)
                - if std::env::var("JCODE_MOBILE_KEYBOARD").ok().as_deref() == Some("0") {
                    px(0.)
                } else {
                    px(crate::sim_keyboard::HEIGHT)
                };
            for selector in ["rename-save", "rename-cancel"] {
                let bounds = cx.debug_bounds(selector).expect("rename action");
                assert!(bounds.left() >= px(0.) && bounds.right() <= px(width));
                assert!(
                    bounds.top() >= px(0.) && bounds.bottom() <= visible_bottom,
                    "{selector} clipped at {width}x{height}: {bounds:?}, bottom={visible_bottom:?}"
                );
            }
            tap(cx, if save { "rename-save" } else { "rename-cancel" });
            view.read_with(cx, |app, cx| {
                assert!(app.feel.rename.is_none());
                assert!(!app.feel.focus_rename);
                let expected = if save {
                    (!text.trim().is_empty()).then(|| text.trim().to_string())
                } else {
                    before
                };
                assert_eq!(app.session.session_title, expected);
                assert_eq!(app.composer.read(cx).text(), DRAFT);
            });
            assert!(cx.debug_bounds("rename-card").is_none());
        }
    }
}

#[gpui::test]
fn rename_transition_discards_selection_menu_and_pending_gesture(cx: &mut TestAppContext) {
    let (view, cx) = setup(cx);
    open_menu(&view, cx, 3);
    tap(cx, "msg-select");
    view.update(cx, |app, cx| {
        assert!(app.feel.selection.is_some());
        app.feel_rename_tab(app.active_tab, cx);
        assert!(app.feel.selection.is_none());
        assert!(app.feel.menu.is_none());
        assert!(app.tab_menu.is_none());
        assert!(!app.scene().menus);
    });
    settle_keyboard(&view, cx);
    tap(cx, "rename-cancel");
    view.read_with(cx, |app, _| assert!(app.scene().menus));
    open_menu(&view, cx, 3);
    let at = point(px(10.), px(100.));
    touch(cx, TouchPhase::Started, at);
    touch(cx, TouchPhase::Ended, at);
    view.read_with(cx, |app, _| {
        assert!(app.feel.menu.as_ref().unwrap().closing);
        assert_eq!(app.session.transcript.len(), 4);
    });
}

#[gpui::test]
fn rename_submit_commits_once_and_releases_editor_focus(cx: &mut TestAppContext) {
    let (view, cx) = setup(cx);
    view.update(cx, |app, cx| app.feel_rename_tab(app.active_tab, cx));
    settle_keyboard(&view, cx);
    let input = view.read_with(cx, |app, _| app.feel.rename.as_ref().unwrap().1.clone());
    input.update(cx, |input, cx| {
        input.set_text("  Submit 工作 👋  ", cx);
        input.sim_submit(cx);
    });
    cx.run_until_parked();
    draw(cx);
    draw(cx);
    cx.update(|window, cx| {
        view.read_with(cx, |app, cx| {
            assert_eq!(app.session.session_title.as_deref(), Some("Submit 工作 👋"));
            assert!(app.feel.rename.is_none());
            assert!(!input.read(cx).is_focused(window));
            assert_eq!(app.composer.read(cx).text(), DRAFT);
            assert!(!app.session.is_processing);
        })
    });
    // A queued duplicate event from the now detached editor is harmless.
    input.update(cx, |input, cx| input.sim_submit(cx));
    cx.run_until_parked();
    view.read_with(cx, |app, _| {
        assert_eq!(app.session.session_title.as_deref(), Some("Submit 工作 👋"))
    });
}

#[gpui::test]
fn stale_enabled_retry_handler_rechecks_processing_before_sending(cx: &mut TestAppContext) {
    let (view, cx) = setup(cx);
    view.update(cx, |app, _| app.session.phase = ConnectionPhase::Connected);
    open_menu(&view, cx, 3);
    let at = cx.debug_bounds("msg-retry").unwrap().center();
    // Change eligibility without notifying or drawing: the installed listener
    // still comes from the enabled menu frame.
    cx.update(|window, cx| {
        view.update(cx, |app, _| app.session.is_processing = true);
        for phase in [TouchPhase::Started, TouchPhase::Ended] {
            window.dispatch_event(
                PlatformInput::Touch(TouchEvent {
                    id: TouchId(73),
                    phase,
                    position: at,
                    predicted_position: None,
                    force: None,
                }),
                cx,
            );
        }
    });
    view.read_with(cx, |app, _| {
        assert_eq!(app.session.transcript.len(), 4);
        assert!(app.session.is_processing);
    });
}
