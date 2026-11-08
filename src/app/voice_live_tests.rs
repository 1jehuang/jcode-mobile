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
        pending: true,
        ..LiveDictation::default()
    };
}

fn preview(app: &mut MobileApp, text: &str, consumed: usize, cx: &mut Context<MobileApp>) {
    app.voice.live.pending = true;
    app.finish_voice_preview(
        app.voice.generation,
        app.active_tab,
        consumed,
        Ok(text.into()),
        cx,
    );
}

#[gpui::test]
fn partials_append_to_typed_base_and_merge_overlapping_speech(cx: &mut TestAppContext) {
    cx.update(init);
    let (view, cx) = cx.add_window_view(super::super::lifecycle_coverage::fixture_app);
    view.update(cx, |app, cx| {
        seed(app, "Typed draft", cx);
        preview(app, "hello world", 100, cx);
        assert_eq!(app.composer.read(cx).text(), "Typed draft hello world");
        assert!(!app.voice.live.pending);
        assert_eq!(app.voice.live.consumed, 100);
        preview(app, "world again", 200, cx);
        assert_eq!(
            app.composer.read(cx).text(),
            "Typed draft hello world again"
        );
        assert_eq!(app.voice.live.transcript, "hello world again");
        assert_eq!(app.voice.live.consumed, 200);
        assert!(app.session.transcript.is_empty());
    });
}

#[gpui::test]
fn preview_callbacks_ignore_wrong_generation_tab_and_final_processing(cx: &mut TestAppContext) {
    cx.update(init);
    let (view, cx) = cx.add_window_view(super::super::lifecycle_coverage::fixture_app);
    view.update(cx, |app, cx| {
        seed(app, "Untouched", cx);
        let generation = app.voice.generation;
        let tab = app.active_tab;
        app.finish_voice_preview(generation.wrapping_add(1), tab, 90, Ok("stale".into()), cx);
        app.finish_voice_preview(
            generation,
            tab.wrapping_add(1),
            90,
            Ok("wrong tab".into()),
            cx,
        );
        app.voice.busy = true;
        app.finish_voice_preview(generation, tab, 90, Ok("during final".into()), cx);
        app.voice.busy = false;
        app.voice.starting = true;
        app.finish_voice_preview(generation, tab, 90, Ok("during startup".into()), cx);
        app.voice.starting = false;
        assert_eq!(app.composer.read(cx).text(), "Untouched");
        assert!(app.voice.live.pending);
        assert_eq!(app.voice.live.consumed, 0);
        assert!(app.voice.live.transcript.is_empty());
        app.voice.live.pending = false;
        app.finish_voice_preview(generation, tab, 90, Ok("unsolicited".into()), cx);
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
        preview(app, "temporary speech", 100, cx);
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
        app.finish_voice_preview(generation, app.active_tab, 200, Ok("late".into()), cx);
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
        for finish in [false, true] {
            seed(app, "Typed", cx);
            preview(app, "partial", 100, cx);
            app.composer
                .update(cx, |input, cx| input.set_text("My deliberate edit", cx));
            preview(app, "partial continued", 200, cx);
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
        preview(app, "provisional words", 100, cx);
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
        preview(app, "unfinished voice", 100, cx);
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
fn preview_errors_advance_consumption_and_final_errors_keep_reviewable_speech(
    cx: &mut TestAppContext,
) {
    cx.update(init);
    let (view, cx) = cx.add_window_view(super::super::lifecycle_coverage::fixture_app);
    view.update(cx, |app, cx| {
        for final_result in [
            Err("fixture transcription failure".into()),
            Ok("   ".into()),
        ] {
            seed(app, "Original draft", cx);
            app.finish_voice_preview(
                app.voice.generation,
                app.active_tab,
                80,
                Err("preview unavailable".into()),
                cx,
            );
            assert_eq!(app.voice.live.consumed, 80);
            assert!(!app.voice.live.pending);
            assert_eq!(app.composer.read(cx).text(), "Original draft");
            preview(app, "temporary", 160, cx);
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
fn denied_preview_stops_capture_surface_and_keeps_review_draft(cx: &mut TestAppContext) {
    cx.update(init);
    let (view, cx) = cx.add_window_view(super::super::lifecycle_coverage::fixture_app);
    view.update(cx, |app, cx| {
        for error in [jcode_mobile_core::voice::ACCESS_DENIED, jcode_mobile_core::voice::SIGN_IN_REQUIRED] {
            seed(app, "Original draft", cx);
            preview(app, "recognized speech", 32_000, cx);
            app.voice.auto_send = true;
            app.voice.locked = true;
            app.voice.live.pending = true;
            app.finish_voice_preview(app.voice.generation, app.active_tab, 64_000, Err(error.into()), cx);
            assert_eq!(app.composer.read(cx).text(), "Original draft recognized speech");
            assert!(app.voice.recording.is_none());
            assert!(!app.voice.auto_send && !app.voice.locked && !app.voice.busy && !app.voice.live.pending);
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
