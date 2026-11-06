//! Offline wheel ownership regressions. Capture is seeded, never started natively.
use super::*;
use crate::app::{MobileApp, SafeArea, init};
use crate::voice_gesture::{Action as VoiceAction, State as VoiceGestureState};
use gpui::{Modifiers, ScrollDelta, TestAppContext, point};
use std::time::Duration;

fn wheel(dy: f32, touch_phase: TouchPhase) -> ScrollWheelEvent {
    ScrollWheelEvent {
        position: point(px(100.), px(200.)),
        delta: ScrollDelta::Pixels(point(px(0.), px(dy))),
        touch_phase,
        modifiers: Modifiers::default(),
    }
}

fn seed_startup(app: &mut MobileApp, locked: bool) {
    app.screen = Screen::Chat;
    let actions = app
        .feel
        .voice_gesture
        .feed(if locked { -240. } else { -80. }, false);
    assert_eq!(actions.first(), Some(&VoiceAction::Start));
    app.feel.voice_wheel_at = Some(Instant::now());
    app.voice.starting = true;
    app.voice.busy = true;
    app.voice.auto_send = true;
    app.voice.locked = locked;
}

#[gpui::test]
fn reverse_to_stop_during_startup_consumes_tail_until_ended(cx: &mut TestAppContext) {
    cx.update(init);
    let (view, cx) = cx.add_window_view(|window, cx| {
        MobileApp::new_offline(
            SafeArea {
                top: 20.,
                bottom: 0.,
            },
            window,
            cx,
        )
    });
    view.update(cx, |app, cx| {
        seed_startup(app, false);
        app.composer
            .update(cx, |input, cx| input.set_text("Unrelated draft", cx));
        assert!(app.feel_voice_wheel(&wheel(80., TouchPhase::Moved), cx));
        assert_eq!(app.feel.voice_gesture.state, VoiceGestureState::Finished);
        assert!(
            app.voice
                .status()
                .unwrap()
                .contains("Finishing microphone startup")
        );
        assert!(app.voice.starting && app.voice.busy && app.voice.auto_send);
        assert!(app.feel_voice_wheel(&wheel(-240., TouchPhase::Moved), cx));
        assert_eq!(app.feel.voice_gesture.state, VoiceGestureState::Finished);
        assert!(!app.voice.locked, "tail must not restart or lock capture");
        assert!(app.feel_voice_wheel(&wheel(0., TouchPhase::Ended), cx));
        assert_eq!(app.feel.voice_gesture.state, VoiceGestureState::Idle);
        assert!(app.feel.voice_wheel_at.is_none());
        assert!(app.feel.voice_pan_ended);
        assert!(!app.feel_voice_wheel(&wheel(-240., TouchPhase::Moved), cx));
        assert_eq!(app.feel.voice_gesture.state, VoiceGestureState::Idle);
        assert!(app.session.transcript.is_empty());
        assert_eq!(app.composer.read(cx).text(), "Unrelated draft");
    });
}

#[gpui::test]
fn cancelled_startup_discards_capture_without_transcript(cx: &mut TestAppContext) {
    cx.update(init);
    let (view, cx) = cx.add_window_view(|window, cx| {
        MobileApp::new_offline(
            SafeArea {
                top: 20.,
                bottom: 0.,
            },
            window,
            cx,
        )
    });
    view.update(cx, |app, cx| {
        seed_startup(app, false);
        assert!(app.feel_voice_wheel(&wheel(0., TouchPhase::Cancelled), cx));
        assert!(
            !app.voice.busy && !app.voice.starting && !app.voice.auto_send && !app.voice.locked
        );
        assert!(app.voice.recording.is_none());
        assert!(app.voice.status().is_none());
        assert_eq!(app.feel.voice_gesture.state, VoiceGestureState::Idle);
        assert!(app.feel.voice_wheel_at.is_none());
        assert!(app.feel.voice_pan_ended);
        assert!(!app.feel_voice_wheel(&wheel(-240., TouchPhase::Moved), cx));
        assert!(app.session.transcript.is_empty());
        assert!(app.composer.read(cx).text().is_empty());
    });
}

#[gpui::test]
fn locked_release_keeps_capture_but_releases_wheel_ownership(cx: &mut TestAppContext) {
    cx.update(init);
    let (view, cx) = cx.add_window_view(|window, cx| {
        MobileApp::new_offline(
            SafeArea {
                top: 20.,
                bottom: 0.,
            },
            window,
            cx,
        )
    });
    view.update(cx, |app, cx| {
        seed_startup(app, true);
        assert!(app.feel_voice_wheel(&wheel(0., TouchPhase::Ended), cx));
        assert_eq!(app.feel.voice_gesture.state, VoiceGestureState::Locked);
        assert!(app.voice.locked && app.voice.starting && app.voice.busy);
        assert!(app.voice.status().unwrap().contains("Preparing microphone"));
        assert!(app.feel.voice_wheel_at.is_none());
        assert!(app.feel.voice_pan_ended);
        assert!(!app.feel_voice_wheel(&wheel(-80., TouchPhase::Moved), cx));
        assert!(!app.feel_voice_wheel(&wheel(80., TouchPhase::Started), cx));
        assert_eq!(app.feel.voice_gesture.state, VoiceGestureState::Locked);
        assert!(app.feel.voice_wheel_at.is_none());
        assert!(app.session.transcript.is_empty());
    });
}

#[gpui::test]
fn explicitly_phased_held_voice_does_not_idle_timeout(cx: &mut TestAppContext) {
    cx.update(init);
    let (view, cx) = cx.add_window_view(|window, cx| {
        MobileApp::new_offline(
            SafeArea {
                top: 20.,
                bottom: 0.,
            },
            window,
            cx,
        )
    });
    view.update(cx, |app, cx| {
        seed_startup(app, false);
        assert!(app.feel_voice_wheel(&wheel(0., TouchPhase::Started), cx));
        assert!(app.feel.voice_phased);
        assert!(!app.feel.voice_pan_ended);
        app.feel.voice_wheel_at = Some(Instant::now() - Duration::from_secs(2));
    });
    // Render invokes feel_wrap's actual wheel fallback, not a synthetic timer.
    cx.update(|window, cx| window.draw(cx).clear(cx));
    view.read_with(cx, |app, _| {
        assert_eq!(app.feel.voice_gesture.state, VoiceGestureState::Recording);
        assert!(app.feel.voice_wheel_at.is_some());
        assert!(app.voice.starting && app.voice.busy);
        assert!(app.voice.status().unwrap().contains("Preparing microphone"));
        assert!(app.session.transcript.is_empty());
    });
}

#[gpui::test]
fn navigation_discards_pending_quick_transcription_after_release(cx: &mut TestAppContext) {
    cx.update(init);
    let (view, cx) = cx.add_window_view(|window, cx| {
        MobileApp::new_offline(SafeArea { top: 20., bottom: 0. }, window, cx)
    });
    view.update(cx, |app, cx| {
        seed_startup(app, false);
        assert!(app.feel_voice_wheel(&wheel(0., TouchPhase::Ended), cx));
        assert_eq!(app.feel.voice_gesture.state, VoiceGestureState::Idle);
        assert!(app.voice.auto_send && app.voice.busy);
        app.screen = Screen::Settings;
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    view.read_with(cx, |app, _| {
        assert!(!app.voice.auto_send && !app.voice.busy && !app.voice.starting);
        assert!(app.session.transcript.is_empty());
    });
}
