//! Authentication/settings interaction coverage with injected service completions.
//! No authentication provider, account file, network, or native browser is used.
use super::*;
use gpui::{
    Modifiers, PlatformInput, ScrollDelta, ScrollWheelEvent, TestAppContext, TouchEvent, TouchId,
    TouchPhase, VisualTestContext, point, size,
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

pub(super) fn account() -> Account {
    Account {
        api_key: "synthetic-coverage-key".into(),
        api_base: "https://offline.invalid".into(),
        email: Some("coverage@example.test".into()),
    }
}

pub(super) fn mount(cx: &mut TestAppContext) -> (Entity<MobileApp>, &mut VisualTestContext) {
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
        app.device = Some((390., 844.));
        app
    });
    cx.simulate_resize(size(px(390.), px(844.)));
    draw(cx);
    (view, cx)
}

pub(super) fn draw(cx: &mut VisualTestContext) {
    cx.update(|window, cx| window.draw(cx).clear(cx));
}

/// Scroll the real containing UI before tapping. A debug bound alone is not
/// evidence that a control is visible or physically reachable.
pub(super) fn tap(cx: &mut VisualTestContext, selector: &'static str) {
    for _ in 0..16 {
        draw(cx);
        let bounds = cx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("missing control {selector}"));
        let viewport = cx.update(|window, _| window.viewport_size());
        let center = bounds.center();
        if center.y > px(22.)
            && center.y < viewport.height - px(8.)
            && center.x > px(0.)
            && center.x < viewport.width
        {
            cx.update(|window, cx| {
                for phase in [TouchPhase::Started, TouchPhase::Ended] {
                    window.dispatch_event(
                        PlatformInput::Touch(TouchEvent {
                            id: TouchId(991),
                            phase,
                            position: center,
                            predicted_position: None,
                            force: None,
                        }),
                        cx,
                    );
                    window.draw(cx).clear(cx);
                }
            });
            return;
        }
        let dy = if center.y >= viewport.height - px(8.) {
            -240.
        } else {
            240.
        };
        cx.simulate_event(ScrollWheelEvent {
            position: point(viewport.width / 2., viewport.height / 2.),
            delta: ScrollDelta::Pixels(point(px(0.), px(dy))),
            touch_phase: TouchPhase::Moved,
            modifiers: Modifiers::default(),
        });
    }
    panic!("control {selector} is not reachable after scrolling");
}

#[gpui::test]
fn email_auth_outcomes_retry_cancel_and_stale_success_are_isolated(cx: &mut TestAppContext) {
    let (view, cx) = mount(cx);
    view.update(cx, |app, cx| {
        let first = app.begin_auth();
        app.complete_sign_in(first, Err("Fixture email error".into()), cx);
        assert!(!app.signing_in);
        assert_eq!(app.sign_in_error.as_deref(), Some("Fixture email error"));
        let second = app.begin_auth();
        assert!(app.sign_in_error.is_none());
        app.complete_sign_in(second, Ok(SignInOutcome::Sent("fixture-token".into())), cx);
        assert_eq!(app.login_token.as_deref(), Some("fixture-token"));
        assert!(app.focus_code);
    });
    draw(cx);
    tap(cx, "use-other-email");
    view.update(cx, |app, cx| {
        assert!(app.login_token.is_none());
        let canceled = app.begin_auth();
        app.invalidate_auth();
        app.complete_sign_in(canceled, Ok(SignInOutcome::Authenticated(account())), cx);
        assert!(app.account.is_none());
        assert!(matches!(app.screen, Screen::SignIn));
        let active = app.begin_auth();
        app.complete_sign_in(active, Err(String::new()), cx);
        assert!(!app.signing_in && app.sign_in_error.is_none());
        let active = app.begin_auth();
        app.complete_sign_in(active, Ok(SignInOutcome::Authenticated(account())), cx);
        assert!(matches!(app.screen, Screen::Chat));
        assert_eq!(app.account, Some(account()));
        assert!(app.connection.is_none() && app.login_token.is_none());
        assert!(app.code.read(cx).text().is_empty());
    });
}

#[gpui::test]
fn browser_cancel_old_failure_and_old_success_cannot_replace_new_login(cx: &mut TestAppContext) {
    let (view, cx) = mount(cx);
    view.update(cx, |app, cx| {
        let old = Arc::new(AtomicBool::new(false));
        app.browser_login = Some(old.clone());
        app.google_sign_in(cx); // The real cancel action remains local/offline.
        assert!(old.load(Ordering::Relaxed));
        assert!(!app.browser_request_current(&old));
        let current = Arc::new(AtomicBool::new(false));
        app.browser_login = Some(current.clone());
        app.complete_browser_sign_in(&old, Err("late error".into()), cx);
        app.complete_browser_sign_in(&old, Ok(account()), cx);
        assert!(app.browser_request_current(&current));
        assert!(app.account.is_none() && app.sign_in_error.is_none());
        app.complete_browser_sign_in(&current, Err("current error".into()), cx);
        assert!(app.browser_login.is_none());
        assert_eq!(app.sign_in_error.as_deref(), Some("current error"));
        let latest = Arc::new(AtomicBool::new(false));
        app.browser_login = Some(latest.clone());
        app.sign_out(cx);
        app.complete_browser_sign_in(&latest, Ok(account()), cx);
        assert!(app.account.is_none() && latest.load(Ordering::Relaxed));
    });
}

#[gpui::test]
fn logout_clears_transient_ui_focus_and_cross_account_pending_actions(cx: &mut TestAppContext) {
    let (view, cx) = mount(cx);
    cx.update(|window, cx| {
        view.update(cx, |app, cx| {
            app.account = Some(account());
            app.screen = Screen::Chat;
            app.model_picker = true;
            app.dir_picker = true;
            app.drawer_open = true;
            app.drawer_t = 1.;
            app.tab_menu = Some(app.active_tab);
            app.auto_send = Some("never send to another account".into());
            app.login_token = Some("old token".into());
            app.code
                .update(cx, |input, cx| input.set_text("654321", cx));
            window.focus(&app.composer.focus_handle(cx), cx);
            app.new_tab(cx);
            app.sign_out(cx);
            assert!(!app.model_picker && !app.dir_picker && !app.drawer_open);
            assert!(app.tab_menu.is_none() && app.auto_send.is_none() && app.parked.is_empty());
            assert!(!app.signing_in && app.browser_login.is_none() && app.login_token.is_none());
            assert!(app.code.read(cx).text().is_empty());
            assert_eq!(app.tab_order, [app.active_tab]);
        })
    });
    draw(cx);
    cx.update(|window, cx| {
        view.read_with(cx, |app, _| {
            assert!(
                !app.keyboard_up,
                "logout left a hidden focused field/keyboard"
            );
            assert!(window.focused(cx).is_none());
        })
    });
}

#[gpui::test]
fn pending_external_prompts_stay_owned_by_their_session_tabs(cx: &mut TestAppContext) {
    let (view, cx) = mount(cx);
    view.update(cx, |app, cx| {
        app.account = Some(account());
        app.auto_send = Some("first tab only".into());
        app.new_tab(cx);
        let second = app.active_tab;
        assert!(app.auto_send.is_none());
        app.auto_send = Some("second tab only".into());
        app.focus_tab(0, cx);
        assert_eq!(app.auto_send.as_deref(), Some("first tab only"));
        app.close_tab(0, cx);
        assert_eq!(app.active_tab, second);
        assert_eq!(app.auto_send.as_deref(), Some("second tab only"));
        app.sign_out(cx);
        assert!(app.auto_send.is_none());
    });
}

#[gpui::test]
fn settings_new_clear_delete_guard_and_signout_controls_are_hit_tested(cx: &mut TestAppContext) {
    let (view, cx) = mount(cx);
    view.update(cx, |app, cx| {
        app.account = Some(account());
        app.screen = Screen::Settings;
        cx.notify();
    });
    settle_settings_fixture(&view, cx);
    tap(cx, "new");
    view.update(cx, |app, cx| {
        assert!(matches!(app.screen, Screen::Chat));
        assert_eq!(app.tab_order.len(), 2);
        app.session.transcript.push(TranscriptEntry {
            id: 1,
            role: Role::User,
            text: "clear me".into(),
            reasoning: String::new(),
            tool_calls: vec![],
            is_streaming: false,
            is_queued: false,
        });
        app.screen = Screen::Settings;
        cx.notify();
    });
    settle_settings_fixture(&view, cx);
    tap(cx, "clear");
    view.update(cx, |app, cx| {
        assert!(app.session.transcript.is_empty());
        assert!(matches!(app.screen, Screen::Chat));
        app.screen = Screen::Settings;
        cx.notify();
    });
    settle_settings_fixture(&view, cx);
    tap(cx, "delete-account");
    tap(cx, "delete-account");
    view.read_with(cx, |app, _| {
        assert_eq!(
            app.delete_step, 0,
            "offline boundary must not arm real deletion"
        );
        assert!(app.account.is_some());
    });
    tap(cx, "sign-out");
    view.read_with(cx, |app, _| {
        assert!(matches!(app.screen, Screen::SignIn) && app.account.is_none())
    });
}

/// This test seeds the Settings route directly rather than testing its push
/// animation. The first draw initializes that presentation and its recorder.
/// Settle only its visual translation before locating real controls, otherwise
/// a partially visible button can move away between raw touch down and up.
/// The handlers, hit testing, and all resulting state assertions remain real.
fn settle_settings_fixture(view: &Entity<MobileApp>, cx: &mut VisualTestContext) {
    draw(cx);
    view.update(cx, |app, cx| {
        assert!(matches!(app.screen, Screen::Settings));
        app.feel.g.edge_x = 0.;
        cx.notify();
    });
    draw(cx);
}

#[gpui::test]
fn deferred_prompt_dispatch_waits_for_ready_session_and_preserves_live_draft(
    cx: &mut TestAppContext,
) {
    let (view, cx) = mount(cx);
    view.update(cx, |app, cx| {
        app.account = Some(account());
        app.composer.update(cx, |input, cx| {
            input.set_text("Keep typed draft 👩🏽‍💻", cx);
            input.select_all_text(cx);
        });
        app.auto_send = Some("External prompt".into());
        app.flush_pending_send(cx);
        assert_eq!(app.auto_send.as_deref(), Some("External prompt"));
        assert!(app.session.transcript.is_empty());
        app.session.phase = ConnectionPhase::Connected;
        app.flush_pending_send(cx);
        assert!(
            app.auto_send.is_some(),
            "phase alone is not session readiness"
        );
        app.session.session_id = Some("fixture-session".into());
        app.flush_pending_send(cx);
        assert!(app.auto_send.is_none());
        assert_eq!(app.composer.read(cx).text(), "Keep typed draft 👩🏽‍💻");
        assert_eq!(app.composer.read(cx).selected_text(), "Keep typed draft 👩🏽‍💻");
        assert!(
            app.session
                .transcript
                .iter()
                .any(|entry| entry.text == "External prompt")
        );
        let count = app.session.transcript.len();
        app.flush_pending_send(cx);
        assert_eq!(
            app.session.transcript.len(),
            count,
            "completion dispatched twice"
        );
        assert!(app.connection.is_none());
    });
}

#[gpui::test]
fn deletion_results_are_mocked_and_cannot_log_out_a_new_auth_session(cx: &mut TestAppContext) {
    let (view, cx) = mount(cx);
    view.update(cx, |app, cx| {
        let expected = account();
        app.adopt_account(expected.clone(), cx);
        let old_generation = app.auth_generation;
        app.delete_step = 2;
        app.complete_account_deletion(
            &expected,
            old_generation,
            Err("fixture deletion failed".into()),
            cx,
        );
        assert_eq!(app.delete_step, 0);
        assert_eq!(app.delete_error.as_deref(), Some("fixture deletion failed"));
        assert!(app.account.is_some());
        app.sign_out(cx);
        app.adopt_account(expected.clone(), cx);
        app.delete_step = 2;
        app.complete_account_deletion(&expected, old_generation, Ok(()), cx);
        assert!(
            app.account.is_some(),
            "old deletion logged out same-account new session"
        );
        assert_eq!(app.delete_step, 2);
        app.complete_account_deletion(&expected, app.auth_generation, Ok(()), cx);
        assert!(app.account.is_none());
        assert!(matches!(app.screen, Screen::SignIn));
    });
}
