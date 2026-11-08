//! Seeded, offline render regressions for finger-owned voice displacement.
//! No test starts capture, dispatches audio actions, or contacts a server.
use super::*;
use crate::app::init;
use crate::voice_gesture::{Action as VoiceAction, LOCK_REST, State as VoiceGestureState};
use gpui::TestAppContext;
use gpui::{Modifiers, ScrollDelta, point};
use crate::app::{Role, TranscriptEntry};
use std::time::Duration;

fn close(actual: f32, expected: f32) {
    assert!((actual - expected).abs() < 0.01, "{actual} != {expected}");
}

fn settle(app: &mut MobileApp, expected: f32) {
    let mut moving = true;
    for _ in 0..180 {
        moving = app.step_voice_motion(1. / 60., false);
        if !moving { break; }
    }
    assert!(!moving, "voice motion must stop requesting frames at rest");
    close(app.feel.voice_gesture.pull, expected);
    assert!(!app.step_voice_motion(1. / 60., false));
    assert!(app.offline && app.voice.recording.is_none());
}

#[gpui::test]
fn voice_native_cancel_discards_capture_without_snapping_chat(cx: &mut TestAppContext) {
    cx.update(init);
    let (view, cx) = cx.add_window_view(super::super::lifecycle_coverage::fixture_app);
    view.update(cx, |app, cx| {
        app.feel.voice_gesture.feed(-80., false);
        app.feel.voice_wheel_at = Some(Instant::now());
        app.voice.starting = true;
        app.voice.busy = true;
        app.voice.auto_send = true;
        let before = app.feel.voice_gesture.pull;
        let cancel = ScrollWheelEvent {
            position: point(px(100.), px(200.)),
            delta: ScrollDelta::Pixels(point(px(0.), px(0.))),
            touch_phase: TouchPhase::Cancelled,
            modifiers: Modifiers::default(),
        };
        assert!(app.feel_voice_wheel(&cancel, cx));
        close(app.feel.voice_gesture.pull, before);
        assert!(!app.feel.voice_gesture.held());
        assert_eq!(app.feel.voice_gesture.state, VoiceGestureState::Idle);
        assert!(!app.voice.starting && !app.voice.busy && !app.voice.auto_send);
        assert!(app.feel.voice_wheel_at.is_none());
        settle(app, 0.);
    });
}

#[cfg(any(target_os = "ios", target_os = "linux"))]
#[gpui::test]
fn voice_bottom_handoff_uses_only_same_event_residual_travel(cx: &mut TestAppContext) {
    cx.update(init);
    let (view, cx) = cx.add_window_view(super::super::lifecycle_coverage::fixture_app);
    cx.simulate_resize(gpui::size(px(390.), px(844.)));
    view.update(cx, |app, cx| {
        app.follow = false;
        app.session.transcript = (0..40).map(|index| TranscriptEntry {
            id: index, role: Role::Assistant,
            text: format!("Offline transcript message {index}"),
            reasoning: String::new(), tool_calls: Vec::new(),
            is_streaming: false, is_queued: false,
        }).collect();
        cx.notify();
    });
    for _ in 0..3 {
        cx.update(|window, cx| { window.refresh(); window.draw(cx).clear(cx); });
    }
    view.update(cx, |app, cx| {
        assert!(app.offline && app.voice.recording.is_none());
        let max = app.transcript_scroll.max_offset().y;
        assert!(max > px(20.), "fixture must really scroll");
        app.follow = false;
        app.transcript_scroll.set_offset(point(px(0.), -max + px(20.)));
        let at = app.feel.g.transcript.unwrap().center();
        let wheel = |dy| ScrollWheelEvent {
            position: at,
            delta: ScrollDelta::Pixels(point(px(0.), px(dy))),
            touch_phase: TouchPhase::Moved,
            modifiers: Modifiers::default(),
        };
        // Temporarily enable eligibility, but residual travel stays far below
        // START so no capture action can be generated or native mic accessed.
        app.offline = false;
        assert!(!app.voice_wheel_eligible(&wheel(-10.)));
        assert!(!app.feel_voice_wheel(&wheel(-10.), cx));
        assert_eq!(app.transcript_scroll.offset().y, -max + px(20.));
        assert_eq!(app.feel.voice_gesture.state, VoiceGestureState::Idle);
        assert!(app.voice_wheel_eligible(&wheel(-30.)));
        assert!(app.feel_voice_wheel(&wheel(-30.), cx));
        app.offline = true;
        assert_eq!(app.feel.voice_gesture.state, VoiceGestureState::Pulling);
        close(app.feel.voice_gesture.pull, 10. / (1. + 10. / 160.));
        assert_eq!(app.transcript_scroll.offset().y, -max);
        assert!(app.follow);
        assert!(!app.voice.starting && !app.voice.busy && app.voice.recording.is_none());
    });
}

#[gpui::test]
fn voice_motion_is_painted_in_same_frame_and_last_frame_disappears(cx: &mut TestAppContext) {
    cx.update(init);
    let (view, cx) = cx.add_window_view(super::super::lifecycle_coverage::fixture_app);
    cx.simulate_resize(gpui::size(px(390.), px(844.)));
    for _ in 0..3 {
        cx.update(|window, cx| { window.refresh(); window.draw(cx).clear(cx); });
    }
    let baseline = cx.debug_bounds("chat-composer").unwrap();
    let before = view.update(cx, |app, cx| {
        assert!(app.offline && app.voice.recording.is_none());
        app.feel.voice_gesture.feed(-80., false);
        let before = app.feel.voice_gesture.pull;
        app.feel.voice_gesture.release();
        // Do not route Finish: a fake pending capture must not keep the reveal up.
        app.voice.starting = false;
        app.voice.busy = false;
        app.voice.auto_send = false;
        app.feel.voice_frame_at = Some(Instant::now() - Duration::from_millis(16));
        cx.notify();
        before
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let painted = f32::from(cx.debug_bounds("voice-control").unwrap().size.height);
    view.read_with(cx, |app, _| {
        assert!(app.feel.voice_gesture.pull < before, "draw must advance the spring");
        assert!((painted - app.feel.voice_gesture.pull).abs() <= 1.,
            "render must use this frame's motion, not the previous displacement");
    });
    assert_eq!(cx.debug_bounds("chat-composer").unwrap().size, baseline.size);
    view.update(cx, |app, cx| { settle(app, 0.); cx.notify(); });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("voice-overscroll-mic").is_none(), "no stale final mic frame");
    assert!(cx.debug_bounds("voice-control").is_none());
    assert_eq!(cx.debug_bounds("chat-composer"), Some(baseline));
}

#[gpui::test]
fn held_voice_startup_and_flag_changes_never_move_finger(cx: &mut TestAppContext) {
    cx.update(init);
    let (view, cx) = cx.add_window_view(super::super::lifecycle_coverage::fixture_app);
    view.update(cx, |app, _| {
        app.feel.voice_gesture.feed(-120., false);
        let held = app.feel.voice_gesture.pull;
        assert!((64.0..70.0).contains(&held));
        app.voice.starting = true;
        app.voice.busy = true;
        app.voice.auto_send = true;
        for (starting, busy) in [(true, true), (false, true), (false, false)] {
            app.voice.starting = starting;
            app.voice.busy = busy;
            assert!(!app.step_voice_motion(1. / 60., false));
            close(app.feel.voice_gesture.pull, held);
            assert!(app.feel.voice_gesture.held());
        }
        assert!(app.offline && app.voice.recording.is_none());
    });
}

#[gpui::test]
fn released_voice_processing_completion_and_cancel_are_continuous(cx: &mut TestAppContext) {
    cx.update(init);
    let (view, cx) = cx.add_window_view(super::super::lifecycle_coverage::fixture_app);
    cx.simulate_resize(gpui::size(px(390.), px(844.)));
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let baseline = cx.debug_bounds("chat-composer").unwrap();
    let input_size = cx.debug_bounds("composer-input").unwrap().size;
    for cancel in [false, true] {
        view.update(cx, |app, cx| {
            app.feel.voice_gesture.feed(-120., false);
            let held = app.feel.voice_gesture.pull;
            app.feel.voice_gesture.release();
            close(app.feel.voice_gesture.pull, held);
            app.voice.busy = true;
            app.voice.auto_send = true;
            app.step_voice_motion(1. / 60., false);
            close(app.feel.voice_gesture.pull, held); // Retarget primes, never jumps.
            settle(app, crate::voice_visual::PROCESSING_REST);
            cx.notify();
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert_eq!(cx.debug_bounds("chat-composer").unwrap().size, baseline.size);
        assert_eq!(cx.debug_bounds("composer-input").unwrap().size, input_size);
        view.update(cx, |app, cx| {
            let before = app.feel.voice_gesture.pull;
            if cancel {
                app.cancel_voice_animated(cx);
            } else {
                app.voice.busy = false;
                app.voice.auto_send = false;
            }
            close(app.feel.voice_gesture.pull, before);
            app.step_voice_motion(1. / 60., false);
            close(app.feel.voice_gesture.pull, before);
            settle(app, 0.);
            cx.notify();
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert_eq!(cx.debug_bounds("chat-composer"), Some(baseline));
        assert!(cx.debug_bounds("voice-overscroll-mic").is_none());
    }
}

#[gpui::test]
fn reduced_motion_tracks_held_finger_then_snaps_released_rest(cx: &mut TestAppContext) {
    cx.update(init);
    let (view, cx) = cx.add_window_view(super::super::lifecycle_coverage::fixture_app);
    view.update(cx, |app, _| {
        app.voice.starting = true;
        app.voice.busy = true;
        app.voice.auto_send = true;
        app.feel.voice_gesture.feed(-120., false);
        let first = app.feel.voice_gesture.pull;
        assert!(!app.step_voice_motion(1. / 60., true));
        close(app.feel.voice_gesture.pull, first);
        app.feel.voice_gesture.feed(-40., false);
        let second = app.feel.voice_gesture.pull;
        assert!(second > first);
        assert!(!app.step_voice_motion(1. / 60., true));
        close(app.feel.voice_gesture.pull, second);
        app.feel.voice_gesture.release();
        assert!(!app.step_voice_motion(1. / 60., true));
        close(app.feel.voice_gesture.pull, crate::voice_visual::PROCESSING_REST);
        app.voice.starting = false;
        app.voice.busy = false;
        app.voice.auto_send = false;
        assert!(!app.step_voice_motion(1. / 60., true));
        close(app.feel.voice_gesture.pull, 0.);
        assert!(!app.step_voice_motion(1. / 60., true));
        // A released lock has a different rest from pending transcription.
        app.feel.voice_gesture.feed(-400., false);
        app.voice.starting = true;
        app.voice.busy = true;
        app.voice.auto_send = true;
        app.voice.locked = false;
        let locked_pull = app.feel.voice_gesture.pull;
        assert!(!app.step_voice_motion(1. / 60., true));
        close(app.feel.voice_gesture.pull, locked_pull);
        assert_eq!(app.feel.voice_gesture.release(), vec![VoiceAction::Lock]);
        app.voice.locked = true;
        assert!(!app.step_voice_motion(1. / 60., true));
        close(app.feel.voice_gesture.pull, LOCK_REST);
        assert!(!app.step_voice_motion(1. / 60., true));
        assert!(app.offline && app.voice.recording.is_none());
    });
}

#[gpui::test]
fn held_lock_renders_beyond_old_cap_then_settles_with_mic_inside_reveal(cx: &mut TestAppContext) {
    cx.update(init);
    let (view, cx) = cx.add_window_view(super::super::lifecycle_coverage::fixture_app);
    cx.simulate_resize(gpui::size(px(390.), px(844.)));
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let baseline_size = cx.debug_bounds("chat-composer").unwrap().size;
    view.update(cx, |app, cx| {
        assert_eq!(app.feel.voice_gesture.feed(-400., false), vec![VoiceAction::Start]);
        assert_eq!(app.feel.voice_gesture.state, VoiceGestureState::Recording);
        assert!(app.feel.voice_gesture.pull > 100.);
        let first = app.feel.voice_gesture.pull;
        assert!(app.feel.voice_gesture.feed(-80., false).is_empty());
        assert!(app.feel.voice_gesture.pull > first, "deep pull does not freeze finger travel");
        app.voice.starting = true;
        app.voice.busy = true;
        app.voice.auto_send = true;
        app.voice.locked = false;
        assert!(!app.step_voice_motion(1. / 60., false));
        cx.notify();
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let height = f32::from(cx.debug_bounds("voice-control").unwrap().size.height);
    assert!(height > 100., "held reveal must not retain the old 96px clip");
    view.read_with(cx, |app, _| assert!((height - app.feel.voice_gesture.pull).abs() <= 1.));
    assert_eq!(cx.debug_bounds("chat-composer").unwrap().size, baseline_size);
    view.update(cx, |app, cx| {
        let before = app.feel.voice_gesture.pull;
        assert!(!app.voice.locked);
        assert_eq!(app.feel.voice_gesture.release(), vec![VoiceAction::Lock]);
        app.voice.locked = true;
        assert_eq!(app.feel.voice_gesture.state, VoiceGestureState::Locked);
        close(app.feel.voice_gesture.pull, before);
        settle(app, LOCK_REST);
        cx.notify();
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let reveal = cx.debug_bounds("voice-control").unwrap();
    let mic = cx.debug_bounds("voice-overscroll-mic").unwrap();
    assert!((f32::from(reveal.size.height) - LOCK_REST).abs() <= 1.);
    assert!(mic.top() >= reveal.top() && mic.bottom() <= reveal.bottom());
    assert!(mic.left() >= reveal.left() && mic.right() <= reveal.right());
    assert_eq!(cx.debug_bounds("chat-composer").unwrap().size, baseline_size);
    view.update(cx, |app, _| {
        assert!(!app.step_voice_motion(1. / 60., false), "locked rest must not redraw forever");
    });
}
