//! Verify feedback at real action boundaries without hardware or network I/O.
use super::*;
use crate::haptics::{capture, Haptic};
use gpui::TestAppContext;

fn touch(cx: &mut gpui::VisualTestContext, phase: gpui::TouchPhase, position: gpui::Point<gpui::Pixels>) {
    cx.update(|window, cx| {
        window.dispatch_event(gpui::PlatformInput::Touch(gpui::TouchEvent {
            id: gpui::TouchId(44), phase, position, predicted_position: None, force: Some(0.5),
        }), cx);
        window.draw(cx).clear(cx);
    });
}

#[gpui::test]
fn held_tab_haptics_fire_once_per_threshold_not_every_move(cx: &mut TestAppContext) {
    use gpui::{TouchPhase, point};
    cx.update(init);
    let (view, cx) = cx.add_window_view(|window, cx| {
        MobileApp::new_offline(SafeArea { top: 20., bottom: 0. }, window, cx)
    });
    view.update(cx, |app, cx| {
        app.screen = Screen::Chat;
        app.account = Some(Account { given_name: None, name: None, api_key: "fixture".into(), api_base: "http://127.0.0.1:9".into(), email: None });
        app.new_tab(cx);
        app.focus_tab(0, cx);
    });
    cx.update(|window, cx| { window.draw(cx).clear(cx); });
    let at = cx.debug_bounds("tab-0").unwrap().center();
    assert!(capture(|| touch(cx, TouchPhase::Started, at)).1.is_empty());
    let (_, events) = capture(|| {
        cx.run_until_parked();
        cx.executor().advance_clock(Duration::from_millis(501));
        cx.run_until_parked();
    });
    assert_eq!(events, [Haptic::Medium], "arming a hold must have one distinct bump");
    assert_eq!(capture(|| touch(cx, TouchPhase::Moved, at + point(px(50.), px(0.)))).1, [Haptic::Selection]);
    assert!(capture(|| touch(cx, TouchPhase::Moved, at + point(px(55.), px(0.)))).1.is_empty());
    assert!(capture(|| touch(cx, TouchPhase::Ended, at + point(px(55.), px(0.)))).1.is_empty());
}

#[gpui::test]
fn haptics_follow_successful_actions_and_leave_noops_silent(cx: &mut TestAppContext) {
    cx.update(init);
    let (view, cx) = cx.add_window_view(|window, cx| {
        MobileApp::new_offline(SafeArea { top: 20., bottom: 0. }, window, cx)
    });
    view.update(cx, |app, cx| {
        app.account = None;
        let (_, events) = capture(|| {
            app.new_tab(cx);
            app.focus_tab(u64::MAX, cx);
            app.close_tab(app.active_tab, cx);
            app.cancel(cx);
            app.send_draft(cx);
            app.sign_in_step(cx);
            app.google_sign_in(cx);
            app.delete_account_tap(cx);
        });
        assert!(events.is_empty(), "no-op/offline actions must be silent: {events:?}");
        app.account = Some(Account {
            given_name: None,
            name: None,
            api_key: "offline-fixture".into(),
            api_base: "http://127.0.0.1:9".into(),
            email: Some("haptics@example.test".into()),
        });
        let first = app.active_tab;
        assert_eq!(capture(|| app.new_tab(cx)).1, [Haptic::Light]);
        let second = app.active_tab;
        assert_eq!(capture(|| app.focus_tab(first, cx)).1, [Haptic::Selection]);
        assert!(capture(|| app.focus_tab(first, cx)).1.is_empty());
        let tabs = app.tab_order.clone();
        assert!(capture(|| app.close_tab(u64::MAX, cx)).1.is_empty());
        assert_eq!(app.tab_order, tabs, "invalid close must not remove a tab");
        assert_eq!(capture(|| app.close_tab(second, cx)).1, [Haptic::Rigid]);
        app.session.is_processing = true;
        assert_eq!(capture(|| app.cancel(cx)).1, [Haptic::Medium]);
        app.session.is_processing = false;
        app.session.phase = ConnectionPhase::Connected;
        app.session.session_id = Some("offline-session".into());
        app.composer.update(cx, |input, cx| input.set_text("Hello", cx));
        assert_eq!(capture(|| app.send_draft(cx)).1, [Haptic::Light]);
        assert!(app.composer.read(cx).text().is_empty());
        assert!(capture(|| app.send_draft(cx)).1.is_empty());
        assert_eq!(capture(|| app.sign_out(cx)).1, [Haptic::Rigid]);
        assert!(capture(|| app.sign_out(cx)).1.is_empty());
    });
}

#[gpui::test]
fn model_choice_only_emits_selection_when_changed(cx: &mut TestAppContext) {
    cx.update(init);
    let (view, cx) = cx.add_window_view(|window, cx| {
        MobileApp::new_offline(SafeArea { top: 20., bottom: 0. }, window, cx)
    });
    view.update(cx, |app, cx| {
        app.session.model_name = Some("current-model".into());
        app.model_picker = true;
        assert!(capture(|| app.pick_model("current-model".into(), cx)).1.is_empty());
        assert!(!app.model_picker);
        assert_eq!(capture(|| app.pick_model("new-model".into(), cx)).1, [Haptic::Selection]);
    });
}

#[test]
fn turn_haptics_only_fire_at_completion_and_distinguish_errors() {
    let mut session = SessionState::default();
    assert!(capture(|| haptics::turn_transition(false, &session)).1.is_empty());
    session.is_processing = true;
    assert!(capture(|| haptics::turn_transition(true, &session)).1.is_empty());
    session.is_processing = false;
    assert_eq!(capture(|| haptics::turn_transition(true, &session)).1, [Haptic::Success]);
    session.error_banner = Some("Request failed".into());
    assert_eq!(capture(|| haptics::turn_transition(true, &session)).1, [Haptic::Error]);
    session.error_banner = None;
    session.phase = ConnectionPhase::Failed { reason: "Disconnected".into() };
    assert_eq!(capture(|| haptics::turn_transition(true, &session)).1, [Haptic::Error]);
    assert!(capture(|| haptics::turn_transition(false, &session)).1.is_empty());
}
