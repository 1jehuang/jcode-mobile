//! Seeded callbacks only: these tests never start recording or upload audio.
use super::*;
use gpui::TestAppContext;

fn seed(app: &mut MobileApp, text: &str, cx: &mut Context<MobileApp>) {
    assert!(app.offline);
    assert!(app.voice.recording.is_none());
    app.composer
        .update(cx, |input, cx| input.set_text(text.to_string(), cx));
    app.voice.busy = false;
    app.voice.starting = false;
    app.voice.live = LiveDictation {
        original: Some(app.composer.read(cx).save_draft()),
        base: text.to_string(),
        shown: Some(text.to_string()),
        ..LiveDictation::default()
    };
}

fn revision(app: &mut MobileApp, text: &str, cx: &mut Context<MobileApp>) {
    app.apply_voice_revision(app.voice.generation, app.active_tab, text.into(), cx);
}

#[gpui::test]
fn cancelling_pending_finalization_signals_the_network_worker(cx: &mut TestAppContext) {
    cx.update(init);
    let (view, cx) = cx.add_window_view(super::super::lifecycle_coverage::fixture_app);
    view.update(cx, |app, cx| {
        seed(app, "Original draft", cx);
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        app.voice.cancel = Some(cancel.clone());
        app.voice.busy = true;
        app.cancel_voice(cx);
        assert!(cancel.load(std::sync::atomic::Ordering::SeqCst));
        assert!(app.voice.cancel.is_none());
        assert!(!app.voice.busy);
        assert_eq!(app.composer.read(cx).text(), "Original draft");
    });
}

#[gpui::test]
fn aggregate_revisions_replace_correct_and_retract_speech(cx: &mut TestAppContext) {
    cx.update(init);
    let (view, cx) = cx.add_window_view(super::super::lifecycle_coverage::fixture_app);
    view.update(cx, |app, cx| {
        seed(app, "Typed draft", cx);
        revision(app, "hello world", cx);
        assert_eq!(app.composer.read(cx).text(), "Typed draft hello world");
        revision(app, "hello corrected world", cx);
        assert_eq!(
            app.composer.read(cx).text(),
            "Typed draft hello corrected world"
        );
        assert_eq!(app.voice.live.transcript, "hello corrected world");
        revision(app, "hello", cx);
        assert_eq!(app.composer.read(cx).text(), "Typed draft hello");
        revision(app, "", cx);
        assert_eq!(app.composer.read(cx).text(), "Typed draft");
        assert!(app.voice.live.transcript.is_empty());
        assert!(app.session.transcript.is_empty());
    });
}

#[gpui::test]
fn revisions_ignore_wrong_generation_tab_and_final_processing(cx: &mut TestAppContext) {
    cx.update(init);
    let (view, cx) = cx.add_window_view(super::super::lifecycle_coverage::fixture_app);
    view.update(cx, |app, cx| {
        seed(app, "Untouched", cx);
        let generation = app.voice.generation;
        let tab = app.active_tab;
        app.apply_voice_revision(generation.wrapping_add(1), tab, "stale".into(), cx);
        app.apply_voice_revision(generation, tab.wrapping_add(1), "wrong tab".into(), cx);
        app.voice.busy = true;
        app.apply_voice_revision(generation, tab, "during final".into(), cx);
        app.voice.busy = false;
        app.voice.starting = true;
        app.apply_voice_revision(generation, tab, "during startup".into(), cx);
        app.voice.starting = false;
        assert_eq!(app.composer.read(cx).text(), "Untouched");
        assert!(app.voice.live.transcript.is_empty());
        app.voice.live.original = None;
        app.apply_voice_revision(generation, tab, "unsolicited".into(), cx);
        assert_eq!(app.composer.read(cx).text(), "Untouched");
    });
}

#[gpui::test]
fn cancellation_restores_saved_text_and_selection_and_rejects_late_preview(
    cx: &mut TestAppContext,
) {
    cx.update(init);
    let (view, cx) = cx.add_window_view(super::super::lifecycle_coverage::fixture_app);
    view.update(cx, |app, cx| {
        seed(app, "Original 日本語 draft", cx);
        app.composer
            .update(cx, |input, cx| input.select_all_text(cx));
        let original = app.composer.read(cx).save_draft();
        app.voice.live.original = Some(original.clone());
        let generation = app.voice.generation;
        revision(app, "temporary speech", cx);
        assert_eq!(app.composer.read(cx).selected_text(), "");
        app.cancel_voice(cx);
        assert_eq!(app.composer.read(cx).text(), "Original 日本語 draft");
        assert_eq!(
            app.composer.read(cx).selected_text(),
            "Original 日本語 draft"
        );
        let restored_text = app.composer.read(cx).text().to_string();
        app.composer
            .update(cx, |input, cx| input.restore_draft(original, cx));
        assert_eq!(app.composer.read(cx).text(), restored_text);
        app.apply_voice_revision(generation, app.active_tab, "late".into(), cx);
        assert_eq!(app.composer.read(cx).text(), restored_text);
        assert!(app.voice.live.original.is_none());
        assert!(app.voice.live.levels.is_empty());
    });
}

#[gpui::test]
fn user_edits_after_partial_are_never_overwritten_by_preview_cancel_or_final(
    cx: &mut TestAppContext,
) {
    cx.update(init);
    let (view, cx) = cx.add_window_view(super::super::lifecycle_coverage::fixture_app);
    view.update(cx, |app, cx| {
        for (finish, aggregate) in [
            (false, "partial continued"),
            (true, "partial continued"),
            (true, ""),
        ] {
            seed(app, "Typed", cx);
            revision(app, "partial", cx);
            app.composer
                .update(cx, |input, cx| input.set_text("My deliberate edit", cx));
            revision(app, aggregate, cx);
            assert_eq!(app.composer.read(cx).text(), "My deliberate edit");
            if finish {
                app.voice.busy = true;
                app.finish_dictation(
                    app.voice.generation,
                    app.active_tab,
                    Ok("authoritative speech".into()),
                    cx,
                );
                assert!(
                    app.voice
                        .error
                        .as_deref()
                        .unwrap()
                        .contains("edited voice draft was kept")
                );
            } else {
                app.cancel_voice(cx);
            }
            assert_eq!(app.composer.read(cx).text(), "My deliberate edit");
            assert!(app.session.transcript.is_empty());
        }
    });
}

#[gpui::test]
fn authoritative_final_replaces_preview_instead_of_duplicating_it(cx: &mut TestAppContext) {
    cx.update(init);
    let (view, cx) = cx.add_window_view(super::super::lifecycle_coverage::fixture_app);
    view.update(cx, |app, cx| {
        seed(app, "Typed draft", cx);
        revision(app, "provisional words", cx);
        app.voice.busy = true;
        app.finish_dictation(
            app.voice.generation,
            app.active_tab,
            Ok("correct final words".into()),
            cx,
        );
        assert_eq!(
            app.composer.read(cx).text(),
            "Typed draft correct final words"
        );
        assert!(app.voice.live.transcript.is_empty());
        assert!(app.voice.live.shown.is_none());
        assert!(!app.voice.busy);
        assert!(app.session.transcript.is_empty());
    });
}

#[gpui::test]
fn auto_send_sends_only_authoritative_voice_and_restores_typed_draft(cx: &mut TestAppContext) {
    cx.update(init);
    let (view, cx) = cx.add_window_view(super::super::lifecycle_coverage::fixture_app);
    view.update(cx, |app, cx| {
        app.session.phase = ConnectionPhase::Connected;
        app.session.session_id = Some("offline-live-voice-fixture".into());
        seed(app, "Never send my typed draft", cx);
        revision(app, "unfinished voice", cx);
        app.voice.auto_send = true;
        app.voice.busy = true;
        app.finish_dictation(
            app.voice.generation,
            app.active_tab,
            Ok("  final voice only  ".into()),
            cx,
        );
        assert_eq!(app.composer.read(cx).text(), "Never send my typed draft");
        assert_eq!(
            app.session.transcript.last().unwrap().text,
            "final voice only"
        );
        assert_eq!(app.session.transcript.len(), 1);
        assert!(!app.voice.auto_send);
        assert!(app.offline && app.voice.recording.is_none());
    });
}

#[gpui::test]
fn stream_final_errors_keep_reviewable_speech(cx: &mut TestAppContext) {
    cx.update(init);
    let (view, cx) = cx.add_window_view(super::super::lifecycle_coverage::fixture_app);
    view.update(cx, |app, cx| {
        for final_result in [
            Err("fixture transcription failure".into()),
            Ok("   ".into()),
        ] {
            seed(app, "Original draft", cx);
            assert_eq!(app.composer.read(cx).text(), "Original draft");
            revision(app, "temporary", cx);
            app.voice.busy = true;
            app.finish_dictation(app.voice.generation, app.active_tab, final_result, cx);
            assert_eq!(app.composer.read(cx).text(), "Original draft temporary");
            assert!(app.voice.has_error());
            assert!(!app.voice.busy);
            assert!(app.session.transcript.is_empty());
        }
    });
}

#[gpui::test]
fn stream_failure_stops_capture_surface_and_keeps_review_draft(cx: &mut TestAppContext) {
    cx.update(init);
    let (view, cx) = cx.add_window_view(super::super::lifecycle_coverage::fixture_app);
    view.update(cx, |app, cx| {
        for error in [
            "Nari rejected the voice session",
            "Nari credits are exhausted",
        ] {
            seed(app, "Original draft", cx);
            revision(app, "recognized speech", cx);
            app.voice.auto_send = true;
            app.voice.locked = true;
            app.finish_live_voice(app.voice.generation, app.active_tab, Err(error.into()), cx);
            assert_eq!(
                app.composer.read(cx).text(),
                "Original draft recognized speech"
            );
            assert!(app.voice.recording.is_none());
            assert!(!app.voice.auto_send && !app.voice.locked && !app.voice.busy);
            assert!(app.voice.error.as_deref().unwrap().starts_with(error));
            assert!(app.session.transcript.is_empty());
            assert!(!app.poll_live_voice(app.voice.generation, app.active_tab, cx));
        }
    });
}

#[gpui::test]
fn final_error_without_preview_preserves_original_draft(cx: &mut TestAppContext) {
    cx.update(init);
    let (view, cx) = cx.add_window_view(super::super::lifecycle_coverage::fixture_app);
    view.update(cx, |app, cx| {
        seed(app, "Untouched original", cx);
        app.voice.busy = true;
        app.voice.auto_send = true;
        app.finish_dictation(
            app.voice.generation,
            app.active_tab,
            Err("final unavailable".into()),
            cx,
        );
        assert_eq!(app.composer.read(cx).text(), "Untouched original");
        assert_eq!(app.voice.error.as_deref(), Some("final unavailable"));
        assert!(app.session.transcript.is_empty());
        assert!(!app.voice.busy && !app.voice.auto_send);
    });
}

#[gpui::test]
fn mocked_signal_renders_beside_mic_without_resizing_composer(cx: &mut TestAppContext) {
    cx.update(init);
    let (view, cx) = cx.add_window_view(super::super::lifecycle_coverage::fixture_app);
    cx.simulate_resize(gpui::size(px(390.), px(844.)));
    view.update(cx, |app, cx| {
        seed(app, "Keep draft", cx);
        app.feel.voice_gesture.feed(-80., false);
        app.voice.starting = true;
        app.voice.busy = true;
        app.voice.auto_send = true;
        app.feel.voice_wheel_at = Some(std::time::Instant::now());
        cx.notify();
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let composer = cx.debug_bounds("chat-composer").unwrap();
    let reveal = cx.debug_bounds("voice-control").unwrap();
    for levels in [vec![0.; 24], vec![0.25, 0.5, 1.]] {
        view.update(cx, |app, cx| {
            app.voice.live.levels = levels.clone();
            cx.notify();
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let waveform = cx.debug_bounds("voice-live-waveform").unwrap();
        let mic = cx.debug_bounds("voice-overscroll-mic").unwrap();
        assert_eq!(
            waveform.size,
            gpui::size(px(crate::voice_visual::LIVE_WIDTH), px(26.))
        );
        assert!(waveform.left() < mic.left() && waveform.right() > mic.right());
        assert!(waveform.left() >= reveal.left() && waveform.right() <= reveal.right());
        assert_eq!(cx.debug_bounds("chat-composer"), Some(composer));
        assert_eq!(cx.debug_bounds("voice-control"), Some(reveal));
    }
    view.read_with(cx, |app, cx| {
        assert!(app.offline && app.voice.recording.is_none());
        assert_eq!(app.composer.read(cx).text(), "Keep draft");
    });
}
