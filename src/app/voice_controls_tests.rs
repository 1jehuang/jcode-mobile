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
fn pulling_voice_mic_reveals_below_input_without_reflow(cx: &mut TestAppContext) {
    cx.update(init);
    let (view, cx) = cx.add_window_view(super::super::lifecycle_coverage::fixture_app);
    cx.simulate_resize(gpui::size(px(390.), px(844.)));
    // Gesture bounds are recorded a frame after layout, including after resize.
    for _ in 0..3 {
        cx.update(|window, cx| {
            window.refresh();
            window.draw(cx).clear(cx);
        });
    }
    let transcript = view.read_with(cx, |app, _| {
        assert!(app.offline, "this regression must never start native capture");
        assert_eq!(app.feel.voice_gesture.state, VoiceGestureState::Idle);
        app.feel.g.transcript.expect("chat transcript must be laid out")
    });
    assert!(cx.debug_bounds("voice-overscroll-semicircle").is_none());
    assert!(cx.debug_bounds("voice-overscroll-mic").is_none());

    let composer = cx.debug_bounds("chat-composer").expect("input line");
    let mut previous_height = px(0.);
    for delta in [-20., -10.] {
        view.update(cx, |app, cx| {
            // Feed the recognizer directly, bypassing offline input eligibility.
            assert!(app.feel.voice_gesture.feed(delta, false).is_empty());
            assert_eq!(app.feel.voice_gesture.state, VoiceGestureState::Pulling);
            assert!(app.feel.voice_gesture.pull < crate::voice_gesture::START);
            cx.notify();
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert!(cx.debug_bounds("voice-overscroll-semicircle").is_none());
        let reveal = cx.debug_bounds("voice-control").expect("overscroll reveal");
        let mic = cx.debug_bounds("voice-overscroll-mic").expect("bare mic");
        let moved_composer = cx.debug_bounds("chat-composer").unwrap();
        assert!(reveal.size.height > previous_height, "pull must uncover more space");
        assert!(reveal.size.height <= px(crate::voice_visual::HEIGHT));
        assert_eq!(mic.size, gpui::size(px(20.), px(26.)));
        assert!(reveal.top() >= moved_composer.bottom() - px(1.),
            "mic reveal belongs below the whole input line, not inside the transcript");
        assert!((f32::from(moved_composer.bottom() - (composer.bottom() - reveal.size.height))).abs() <= 1.);
        assert_eq!(moved_composer.size, composer.size);
        previous_height = reveal.size.height;
        // Let the recorded bounds catch up, so a reflow cannot hide for a frame.
        cx.update(|window, cx| {
            window.refresh();
            window.draw(cx).clear(cx);
        });
        view.read_with(cx, |app, _| {
            let moved = app.feel.g.transcript.unwrap();
            assert_eq!(moved.size, transcript.size, "overscroll must not reflow the transcript");
            assert!((f32::from(moved.top() - (transcript.top() - previous_height))).abs() <= 1.,
                "transcript and input line must travel upward together");
            assert!(!app.voice.starting && !app.voice.busy);
            assert!(app.voice.recording.is_none());
        });
    }
    view.update(cx, |app, cx| {
        assert!(app.feel.voice_gesture.release().is_empty());
        app.feel.voice_gesture.snap_to_target();
        cx.notify();
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("voice-overscroll-mic").is_none());
    assert_eq!(cx.debug_bounds("chat-composer"), Some(composer));
}

#[cfg(any(target_os = "ios", target_os = "linux"))]
#[gpui::test]
fn supported_preview_accepts_only_deliberate_bottom_overscroll(cx: &mut TestAppContext) {
    cx.update(init);
    let (view, cx) = cx.add_window_view(super::super::lifecycle_coverage::fixture_app);
    view.update(cx, |app, cx| {
        // Eligibility only. Never start a microphone or contact the fixture account.
        app.offline = false;
        app.feel.g.transcript = Some(gpui::Bounds::new(
            point(px(0.), px(0.)), gpui::size(px(390.), px(600.)),
        ));
        assert!(app.voice_wheel_eligible(&wheel(-80., TouchPhase::Moved)));
        assert!(!app.voice_wheel_eligible(&wheel(80., TouchPhase::Moved)));
        assert!(!app.voice_wheel_eligible(&wheel(-80., TouchPhase::Ended)));
        assert!(!app.voice_wheel_eligible(&wheel(-80., TouchPhase::Cancelled)));
        let mut outside = wheel(-80., TouchPhase::Moved);
        outside.position = point(px(500.), px(200.));
        assert!(!app.voice_wheel_eligible(&outside));
        let mut horizontal = wheel(-80., TouchPhase::Moved);
        horizontal.delta = ScrollDelta::Pixels(point(px(200.), px(-80.)));
        assert!(!app.voice_wheel_eligible(&horizontal));
        app.drawer_open = true;
        assert!(!app.voice_wheel_eligible(&wheel(-80., TouchPhase::Moved)));
        app.drawer_open = false;
        app.feel.voice_pan_ended = true;
        assert!(!app.voice_wheel_eligible(&wheel(-80., TouchPhase::Moved)));
        app.offline = true;
        app.cancel_voice(cx);
        cx.notify();
    });
}

#[gpui::test]
fn voice_phases_reuse_controls_without_resizing_chat(cx: &mut TestAppContext) {
    cx.update(init);
    let (view, cx) = cx.add_window_view(super::super::lifecycle_coverage::fixture_app);
    cx.simulate_resize(gpui::size(px(390.), px(844.)));
    view.update(cx, |app, cx| {
        assert!(app.offline, "seed state only, never capture or network");
        app.composer.update(cx, |input, cx| input.set_text("Keep this draft", cx));
        cx.notify();
    });
    for _ in 0..3 {
        cx.update(|window, cx| { window.refresh(); window.draw(cx).clear(cx); });
    }
    let composer = cx.debug_bounds("chat-composer").unwrap();
    let input = cx.debug_bounds("composer-input").unwrap();
    let metadata = cx.debug_bounds("chat-metadata").unwrap();
    let send = cx.debug_bounds("send").expect("existing composer send button");
    let transcript = view.read_with(cx, |app, _| app.feel.g.transcript.unwrap());

    for phase in ["pulling", "preparing", "locked preparing", "transcribing", "cancelled"] {
        view.update(cx, |app, cx| {
            match phase {
                "pulling" => {
                    assert!(app.feel.voice_gesture.feed(-20., false).is_empty());
                    assert_eq!(app.feel.voice_gesture.state, VoiceGestureState::Pulling);
                }
                "preparing" => {
                    app.feel.voice_gesture.reset();
                    seed_startup(app, false);
                }
                "locked preparing" => {
                    app.feel.voice_gesture.reset();
                    seed_startup(app, true);
                    app.feel.voice_gesture.release();
                    app.feel.voice_wheel_at = None;
                }
                "transcribing" => {
                    app.feel.voice_gesture.reset();
                    app.feel.voice_wheel_at = None;
                    app.voice.starting = false;
                    app.voice.busy = true;
                    app.voice.auto_send = true;
                    app.voice.locked = false;
                }
                "cancelled" => {} // Click the rendered replacement send control below.
                _ => unreachable!(),
            }
            cx.notify();
        });
        if phase == "cancelled" {
            let cancel = cx.debug_bounds("cancel-dictation").expect("transcription cancel button");
            cx.simulate_click(cancel.center(), Modifiers::default());
        }
        // Recorded transcript bounds lag layout by a frame. Check settled sizes,
        // not absolute y: the pull and lock reveal intentionally move the chat.
        for _ in 0..3 {
            cx.update(|window, cx| { window.refresh(); window.draw(cx).clear(cx); });
        }
        let current = cx.debug_bounds("chat-composer").unwrap();
        assert_eq!(current.size, composer.size, "{phase}: no status rows or cards may grow composer");
        assert_eq!(cx.debug_bounds("composer-input").unwrap().size, input.size, "{phase}");
        assert_eq!(cx.debug_bounds("chat-metadata").unwrap().size, metadata.size, "{phase}");
        view.read_with(cx, |app, cx| {
            assert!(app.offline);
            assert!(app.voice.recording.is_none(), "no real capture in {phase}");
            assert_eq!(app.feel.g.transcript.unwrap().size, transcript.size, "{phase}: no transcript reflow or status card");
            assert!(app.session.transcript.is_empty());
            assert_eq!(app.composer.read(cx).text(), "Keep this draft");
            if phase == "cancelled" {
                assert!(!app.voice.starting && !app.voice.busy && !app.voice.auto_send && !app.voice.locked);
                assert!(app.voice.status().is_none());
                assert_eq!(app.feel.voice_gesture.state, VoiceGestureState::Idle);
                assert!(app.feel.voice_wheel_at.is_none());
            }
        });
        if matches!(phase, "preparing" | "locked preparing" | "transcribing") {
            let cancel = cx.debug_bounds("cancel-dictation").expect("reuse send button for cancellation");
            assert_eq!(cancel.size, send.size, "{phase}: cancel replaces existing round send control");
            assert!(cancel.left() >= current.left() && cancel.right() <= current.right());
            assert!(cancel.top() >= current.top() && cancel.bottom() <= current.bottom());
            assert!(cx.debug_bounds("send").is_none(), "cancel must replace, not supplement, send");
        } else {
            assert!(cx.debug_bounds("cancel-dictation").is_none());
            assert_eq!(cx.debug_bounds("send").unwrap().size, send.size);
        }
        if phase == "locked preparing" {
            let stop = cx.debug_bounds("stop-dictation").expect("locked MIC reveal is the stop control");
            assert!(stop.top() >= current.bottom() - px(1.), "stop belongs below composer, not in a new composer row");
            assert!(cx.debug_bounds("voice-overscroll-mic").is_some());
        }
    }
}

#[gpui::test]
fn locked_voice_has_a_rendered_stop_and_send_control(cx: &mut TestAppContext) {
    cx.update(init);
    let (view, cx) = cx.add_window_view(super::super::lifecycle_coverage::fixture_app);
    cx.simulate_resize(gpui::size(px(390.), px(844.)));
    view.update(cx, |app, cx| {
        seed_startup(app, true);
        app.feel.voice_gesture.release();
        app.feel.voice_wheel_at = None;
        app.composer.update(cx, |input, cx| input.set_text("Keep this draft", cx));
        cx.notify();
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let stop = cx.debug_bounds("stop-dictation").expect("locked recording needs a stop control");
    cx.simulate_click(stop.center(), Modifiers::default());
    view.read_with(cx, |app, cx| {
        assert_eq!(app.feel.voice_gesture.state, VoiceGestureState::Idle);
        assert!(!app.voice.locked);
        assert!(app.voice.starting && app.voice.auto_send);
        assert!(app.voice.status().unwrap().contains("Finishing microphone startup"));
        assert_eq!(app.composer.read(cx).text(), "Keep this draft");
        assert!(app.session.transcript.is_empty());
    });
}

#[cfg(any(target_os = "ios", target_os = "linux"))]
#[gpui::test]
fn native_touch_pan_enters_voice_and_release_finishes_mocked_capture(cx: &mut TestAppContext) {
    cx.update(init);
    let (view, cx) = cx.add_window_view(super::super::lifecycle_coverage::fixture_app);
    cx.simulate_resize(gpui::size(px(390.), px(844.)));
    // Recorded gesture bounds lag layout by one frame after resizing.
    for _ in 0..3 {
        cx.update(|window, cx| { window.refresh(); window.draw(cx).clear(cx); });
    }
    let at = view.read_with(cx, |app, _| app.feel.g.transcript.unwrap().center());
    view.update(cx, |app, _| app.offline = false);
    let dispatch = |cx: &mut gpui::VisualTestContext, phase, position| {
        cx.update(|window, cx| {
            window.dispatch_event(gpui::PlatformInput::Touch(gpui::TouchEvent {
                id: gpui::TouchId(701), phase, position,
                predicted_position: None, force: None,
            }), cx);
            window.draw(cx).clear(cx);
        });
    };
    dispatch(cx, TouchPhase::Started, at);
    let moved = at - point(px(0.), px(20.));
    dispatch(cx, TouchPhase::Moved, moved);
    view.update(cx, |app, _| {
        assert_eq!(app.feel.voice_gesture.state, VoiceGestureState::Pulling,
            "at={at:?}, transcript={:?}, ended={}, phased={}, tracked={}, eligible={}, offset={:?}, max={:?}, offline={}, screen={:?}, drawer={}, git={}",
            app.feel.g.transcript, app.feel.voice_pan_ended, app.feel.voice_phased,
            app.feel.voice_wheel_at.is_some(), app.scene_allows_voice(),
            app.transcript_scroll.offset(), app.transcript_scroll.max_offset(),
            app.offline, app.screen == Screen::Chat, app.drawer_open, app.git_info.open);
        assert!(app.feel.voice_phased && app.feel.voice_wheel_at.is_some());
        // The real touchscreen pan has acquired voice routing below the start
        // threshold. Seed capture now, so tests never access real audio/network.
        app.offline = true;
        seed_startup(app, false);
    });
    dispatch(cx, TouchPhase::Ended, moved);
    view.read_with(cx, |app, _| {
        assert_eq!(app.feel.voice_gesture.state, VoiceGestureState::Idle);
        assert!(app.feel.voice_pan_ended && app.feel.voice_wheel_at.is_none());
        assert!(app.voice.status().unwrap().contains("Finishing microphone startup"));
        assert!(app.session.transcript.is_empty());
    });
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
