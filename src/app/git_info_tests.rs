use super::*;
use gpui::{Modifiers, TestAppContext, point, size};

#[gpui::test]
fn raw_touch_pull_opens_git_without_reconnecting_or_recording(cx: &mut TestAppContext) {
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
        app
    });
    cx.simulate_resize(size(px(390.), px(844.)));
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
    let start = cx.debug_bounds("git-pull-handle").unwrap().center();
    for (phase, dy) in [
        (gpui::TouchPhase::Started, 0.),
        (gpui::TouchPhase::Moved, 60.),
        (gpui::TouchPhase::Ended, 70.),
    ] {
        cx.update(|window, cx| {
            window.dispatch_event(
                gpui::PlatformInput::Touch(gpui::TouchEvent {
                    id: gpui::TouchId(1),
                    phase,
                    position: start + point(px(0.), px(dy)),
                    predicted_position: None,
                    force: None,
                }),
                cx,
            );
            window.draw(cx).clear(cx);
        });
    }
    view.read_with(cx, |app, _| {
        assert!(app.git_info.open);
        assert!(app.feel.g.refreshing.is_none());
        assert!(!app.voice.starting && !app.voice.busy && app.voice.recording.is_none());
    });
    assert!(cx.debug_bounds("git-info-panel").is_some());
}

#[gpui::test]
fn git_pull_threshold_cancel_and_session_invalidation(cx: &mut TestAppContext) {
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
        app.session.working_dir = Some("/project".into());
        app.device = Some((390., 844.));
        app
    });
    cx.simulate_resize(size(px(390.), px(844.)));
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
    let handle = cx.debug_bounds("git-pull-handle").unwrap();
    let start = handle.center();
    view.update(cx, |app, cx| {
        app.composer
            .update(cx, |input, cx| input.set_text("Keep my draft", cx));
        assert!(!app.git_down(point(px(10.), start.y)));
        assert!(app.git_down(start));
        app.git_move(start + point(px(0.), px(25.)), cx);
        app.git_up(false, cx);
        assert!(!app.git_info.open, "short pulls must not open");
        assert!(app.git_down(start));
        app.git_move(start + point(px(0.), px(70.)), cx);
        app.git_up(true, cx);
        assert!(!app.git_info.open, "cancelled pulls must not open");
        assert!(app.git_down(start));
        app.git_move(start + point(px(80.), px(50.)), cx);
        app.git_up(false, cx);
        assert!(!app.git_info.open, "horizontal swipes must not open");
        assert!(app.git_down(start));
        app.git_move(start + point(px(0.), px(70.)), cx);
        app.git_up(false, cx);
        assert!(app.git_info.open);
        assert!(
            !app.git_info.loading,
            "offline preview must never reach cloud"
        );
        assert_eq!(app.composer.read(cx).text(), "Keep my draft");
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
    assert!(cx.debug_bounds("git-info-panel").is_some());
    let close = cx.debug_bounds("close-git").unwrap();
    cx.simulate_click(close.center(), Modifiers::default());
    view.read_with(cx, |app, _| assert!(!app.git_info.open));
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
    cx.simulate_click(start, Modifiers::default());
    view.read_with(cx, |app, _| {
        assert!(app.git_info.open, "tap alternative should work")
    });
    view.update(cx, |app, cx| {
        app.session.working_dir = Some("/different-project".into());
        cx.notify();
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
    view.read_with(cx, |app, _| {
        assert!(
            !app.git_info.open,
            "session changes must dismiss stale Git details"
        );
        assert!(app.git_info.snapshot.is_none());
    });
}

#[gpui::test]
fn git_handle_stays_top_right_and_composer_has_no_microphone(cx: &mut TestAppContext) {
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
        app
    });
    for width in [320., 390., 430.] {
        view.update(cx, |app, cx| {
            app.device = Some((width, 844.));
            cx.notify();
        });
        cx.simulate_resize(size(px(width), px(844.)));
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });
        let handle = cx.debug_bounds("git-pull-handle").unwrap();
        let composer = cx.debug_bounds("chat-composer").unwrap();
        assert!(handle.left() > px(width / 2.));
        assert!(handle.right() <= px(width));
        assert!(handle.size.height >= px(44.));
        assert!(handle.bottom() < composer.top());
        assert!(cx.debug_bounds("microphone").is_none());
    }
}
