//! Ephemeral voice capture. Never persisted, automatically discarded on navigation.
use super::*;
use crate::voice_stream::{MicGate, OpenOnDrop, Recording};
use std::collections::VecDeque;

/// A pulled voice message that finished while the finger kept going. It
/// finalizes in the background, in order, so the next pull can record at once.
struct Segment {
    id: u64,
    tab: u64,
    /// Live preview text, kept for review if finalization fails.
    partial: String,
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    result: Option<Result<String, String>>,
}

#[derive(Default)]
struct LiveDictation {
    original: Option<crate::text_input::InputDraft>,
    base: String,
    shown: Option<String>,
    transcript: String,
    levels: Vec<f32>,
}

#[derive(Default)]
pub(super) struct VoiceState {
    pub recording: Option<Recording>,
    pub busy: bool,
    pub starting: bool,
    pub locked: bool,
    pub auto_send: bool,
    pub began: Option<std::time::Instant>,
    pub(super) stop_requested: bool,
    generation: u64,
    cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    error: Option<String>,
    live: LiveDictation,
    segments: VecDeque<Segment>,
    next_segment: u64,
    /// Opens once the most recent segment released the microphone.
    mic_gate: Option<MicGate>,
}
impl VoiceState {
    /// Finished voice messages still transcribing in the background.
    pub fn sending(&self) -> bool {
        !self.segments.is_empty()
    }

    pub fn status(&self) -> Option<String> {
        if self.starting {
            Some(
                if self.stop_requested {
                    "Finishing microphone startup…"
                } else {
                    "Preparing microphone…"
                }
                .into(),
            )
        } else if self.recording.is_some() && self.locked {
            Some("Mic locked on. Tap Stop and send, or cancel to discard.".into())
        } else if self.recording.is_some() && self.auto_send {
            Some(
                "Recording. Move back down to finish, or release while deeply pulled to lock."
                    .into(),
            )
        } else if self.recording.is_some() {
            Some("Recording. Tap stop to transcribe, or cancel to discard.".into())
        } else if self.busy {
            Some(
                if self.auto_send {
                    "Transcribing, then sending voice message…"
                } else {
                    "Transcribing…"
                }
                .into(),
            )
        } else if self.sending() {
            Some("Transcribing, then sending voice message…".into())
        } else {
            self.error.clone()
        }
    }

    pub fn has_error(&self) -> bool {
        self.error.is_some()
    }

    pub(super) fn levels(&self) -> &[f32] {
        &self.live.levels
    }

    #[cfg(test)]
    pub(super) fn seed_live_preview_for_test(&mut self, original: crate::text_input::InputDraft, text: &str) {
        self.live = LiveDictation {
            original: Some(original),
            base: String::new(),
            shown: Some(String::new()),
            transcript: text.into(),
            levels: Vec::new(),
        };
    }
}
impl MobileApp {
    /// Remove only our own provisional text. User edits and IME state are never
    /// rolled back if they changed the input after a partial result was painted.
    fn restore_voice_draft(&mut self, cx: &mut Context<Self>) -> bool {
        let owned = self
            .voice
            .live
            .shown
            .as_deref()
            .is_some_and(|shown| self.composer.read(cx).text() == shown);
        if let Some(draft) = self.voice.live.original.take() {
            if owned {
                self.composer
                    .update(cx, |input, cx| input.restore_draft(draft, cx));
            }
        }
        owned
    }

    fn apply_voice_revision(
        &mut self,
        generation: u64,
        tab: u64,
        text: String,
        cx: &mut Context<Self>,
    ) {
        if self.voice.generation != generation
            || self.active_tab != tab
            || self.voice.live.original.is_none()
            || self.voice.busy
            || self.voice.starting
        {
            return;
        }
        // Nari publishes the complete aggregate, including corrections and
        // retractions. Never overlap-merge it with an older hypothesis.
        self.voice.live.transcript = text;
        if self
            .voice
            .live
            .shown
            .as_deref()
            .is_some_and(|shown| self.composer.read(cx).text() == shown)
        {
            let text = if self.voice.live.transcript.is_empty() {
                self.voice.live.base.clone()
            } else {
                jcode_mobile_core::voice::append(&self.voice.live.base, &self.voice.live.transcript)
            };
            self.composer
                .update(cx, |input, cx| input.set_text(text.clone(), cx));
            self.voice.live.shown = Some(text);
        }
        cx.notify();
    }

    fn finish_live_voice(
        &mut self,
        generation: u64,
        tab: u64,
        result: Result<String, String>,
        cx: &mut Context<Self>,
    ) {
        if self.voice.generation != generation || self.active_tab != tab {
            return;
        }
        self.voice.recording = None;
        self.feel.voice_gesture.dismiss();
        self.feel.voice_wheel_at = None;
        self.feel.voice_pan_ended = true;
        self.finish_dictation(generation, tab, result, cx);
    }

    fn poll_live_voice(&mut self, generation: u64, tab: u64, cx: &mut Context<Self>) -> bool {
        if self.voice.generation != generation || self.active_tab != tab {
            return false;
        }
        let Some(recording) = self.voice.recording.as_ref() else {
            return false;
        };
        let levels = recording.levels();
        let revision = recording.poll_transcript();
        let finished = recording.poll_finished();
        if levels != self.voice.live.levels {
            self.voice.live.levels = levels;
            cx.notify();
        }
        if let Some(text) = revision {
            self.apply_voice_revision(generation, tab, text, cx);
        }
        if let Some(result) = finished {
            self.finish_live_voice(generation, tab, result, cx);
            return false;
        }
        true
    }

    fn start_live_voice(&mut self, cx: &mut Context<Self>) {
        let generation = self.voice.generation;
        let tab = self.active_tab;
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(50))
                    .await;
                if !this
                    .update(cx, |this, cx| this.poll_live_voice(generation, tab, cx))
                    .unwrap_or(false)
                {
                    break;
                }
            }
        })
        .detach();
    }

    /// Start while the finger is still held, not after release.
    pub(super) fn start_overscroll_voice(&mut self, cx: &mut Context<Self>) {
        // Pulled back out before the microphone was ready: keep that capture.
        if self.voice.starting && self.voice.stop_requested {
            self.voice.stop_requested = false;
            cx.notify();
            return;
        }
        if cfg!(any(target_os = "ios", target_os = "linux")) && self.scene_allows_voice() {
            self.begin_voice(true, cx);
        }
    }

    pub(super) fn scene_allows_voice(&self) -> bool {
        self.screen == Screen::Chat
            && !self.offline
            && self.account.is_some()
            && !self.voice.busy
            && self.voice.recording.is_none()
            && !self.voice.starting
            && !self.drawer_open
            && !self.map.visible()
            && !self.git_info.open
            && !self.model_picker
            && !self.dir_picker
            && self.tab_menu.is_none()
    }

    fn finish_dictation(
        &mut self,
        generation: u64,
        tab: u64,
        result: Result<String, String>,
        cx: &mut Context<Self>,
    ) {
        if self.voice.generation != generation || self.active_tab != tab {
            return;
        }
        self.feel_voice_motion_changed();
        self.voice.busy = false;
        self.voice.starting = false;
        self.voice.locked = false;
        self.voice.stop_requested = false;
        self.voice.began = None;
        let auto_send = std::mem::take(&mut self.voice.auto_send);
        self.voice.cancel = None;
        let partial = self.voice.live.transcript.clone();
        let had_preview = !partial.is_empty();
        let had_live_draft = self.voice.live.original.is_some();
        let owned_preview = self.restore_voice_draft(cx);
        self.voice.live = LiveDictation::default();
        match result {
            Ok(text) if !text.trim().is_empty() => {
                // Voice sends only its own transcript. Never send or clear a draft
                // that the user was typing while the service was working.
                if !(auto_send && self.send_voice_text(&text)) {
                    if had_live_draft && !owned_preview {
                        self.voice.error = Some(
                            "Your edited voice draft was kept. Review it before sending.".into(),
                        );
                        cx.notify();
                        return;
                    }
                    let draft =
                        jcode_mobile_core::voice::append(self.composer.read(cx).text(), &text);
                    self.composer
                        .update(cx, |input, cx| input.set_text(draft, cx));
                    if auto_send {
                        self.voice.error = Some(
                            "Connection changed. Voice text saved to your draft, not sent.".into(),
                        );
                    }
                }
            }
            result => {
                if had_preview && owned_preview {
                    let draft =
                        jcode_mobile_core::voice::append(self.composer.read(cx).text(), &partial);
                    self.composer
                        .update(cx, |input, cx| input.set_text(draft, cx));
                }
                self.voice.error = Some(match result {
                    Err(error) if had_preview => format!("{error} Live voice draft kept. Review before sending."),
                    Err(error) => error,
                    _ if had_preview => "Final transcription was empty. Live voice draft kept. Review before sending.".into(),
                    _ => "No speech detected. Nothing was sent.".into(),
                });
            }
        }
        cx.notify();
    }

    /// Send a voice transcript as its own message. False when not connected.
    fn send_voice_text(&mut self, text: &str) -> bool {
        // Local chat uses the command queue, not the SSH protocol. Keep voice
        // auto-send results as drafts for explicit composer submission.
        if self.is_local_chat() { return false; }
        if self.session.phase != ConnectionPhase::Connected || self.session.session_id.is_none() {
            return false;
        }
        let text = text.trim().to_string();
        if self.session.is_processing {
            self.session
                .apply_intent(LocalIntent::UserQueuedInterrupt(text.clone()));
            self.send(|id| Request::SoftInterrupt {
                id,
                content: text,
                urgent: false,
            });
        } else {
            self.session
                .apply_intent(LocalIntent::UserSentMessage(text.clone()));
            self.send(|id| Request::Message { id, content: text });
        }
        self.follow = true;
        self.feel_sent();
        true
    }

    /// Hand a finished pulled message to the background and free the gesture,
    /// so the same held finger can immediately record the next one.
    fn queue_voice_segment(&mut self, mut recording: Recording, cx: &mut Context<Self>) {
        self.feel_voice_motion_changed();
        let partial = self.voice.live.transcript.clone();
        let owned = self.restore_voice_draft(cx);
        self.voice.live = LiveDictation::default();
        let id = self.voice.next_segment;
        self.voice.next_segment = id.wrapping_add(1);
        let gate = MicGate::default();
        self.voice.mic_gate = Some(gate.clone());
        self.voice.segments.push_back(Segment {
            id,
            tab: self.active_tab,
            partial: if owned { partial } else { String::new() },
            cancel: recording.cancel_handle(),
            result: None,
        });
        // Invalidate live polling and the capture timeout of this recording.
        self.voice.generation = self.voice.generation.wrapping_add(1);
        self.voice.cancel = None;
        self.voice.busy = false;
        self.voice.starting = false;
        self.voice.locked = false;
        self.voice.auto_send = false;
        self.voice.stop_requested = false;
        self.voice.began = None;
        let task = cx.background_spawn(async move {
            let mic = {
                let _open = OpenOnDrop(gate);
                recording.release_microphone()
            };
            mic?;
            recording.stop()
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| this.voice_segment_done(id, result, cx));
        })
        .detach();
        cx.notify();
    }

    /// Deliver finished segments strictly in the order they were spoken.
    fn voice_segment_done(&mut self, id: u64, result: Result<String, String>, cx: &mut Context<Self>) {
        let Some(segment) = self.voice.segments.iter_mut().find(|s| s.id == id) else {
            return;
        };
        segment.result = Some(result);
        while self.voice.segments.front().is_some_and(|s| s.result.is_some()) {
            let Some(segment) = self.voice.segments.pop_front() else { break };
            if segment.tab == self.active_tab {
                self.deliver_voice_segment(segment, cx);
            }
        }
        self.feel_voice_motion_changed();
        cx.notify();
    }

    fn deliver_voice_segment(&mut self, segment: Segment, cx: &mut Context<Self>) {
        let append = |this: &mut Self, text: &str, cx: &mut Context<Self>| {
            let draft = jcode_mobile_core::voice::append(this.composer.read(cx).text(), text);
            this.composer.update(cx, |input, cx| input.set_text(draft, cx));
        };
        match segment.result {
            Some(Ok(text)) if !text.trim().is_empty() => {
                // Same as a single pulled message: auto-send wins when connected.
                if !self.send_voice_text(&text) {
                    append(self, text.trim(), cx);
                    self.voice.error =
                        Some("Connection changed. Voice text saved to your draft, not sent.".into());
                }
            }
            result => {
                let kept = !segment.partial.is_empty();
                if kept {
                    append(self, &segment.partial, cx);
                }
                self.voice.error = Some(match result {
                    Some(Err(error)) if kept => format!("{error} Live voice draft kept. Review before sending."),
                    Some(Err(error)) => error,
                    _ if kept => "Final transcription was empty. Live voice draft kept. Review before sending.".into(),
                    _ => "No speech detected. Nothing was sent.".into(),
                });
            }
        }
    }

    pub(super) fn cancel_voice(&mut self, cx: &mut Context<Self>) {
        if let Some(cancel) = self.voice.cancel.take() {
            cancel.store(true, std::sync::atomic::Ordering::SeqCst);
        }
        for segment in self.voice.segments.drain(..) {
            segment.cancel.store(true, std::sync::atomic::Ordering::SeqCst);
        }
        self.restore_voice_draft(cx);
        self.voice.live = LiveDictation::default();
        self.voice.generation = self.voice.generation.wrapping_add(1);
        self.voice.recording = None;
        self.voice.busy = false;
        self.voice.starting = false;
        self.voice.locked = false;
        self.voice.auto_send = false;
        self.voice.stop_requested = false;
        self.voice.began = None;
        self.voice.error = None;
        self.feel.voice_gesture.reset();
        self.feel.voice_wheel_at = None;
        self.feel.voice_pan_ended = true;
    }

    /// User cancellation clears capture immediately while the mic returns gently.
    /// Navigation continues to use cancel_voice, which discards the whole surface.
    pub(super) fn cancel_voice_animated(&mut self, cx: &mut Context<Self>) {
        let mut gesture = std::mem::take(&mut self.feel.voice_gesture);
        self.cancel_voice(cx);
        gesture.dismiss();
        self.feel.voice_gesture = gesture;
        self.feel_voice_motion_changed();
    }

    /// A release during permission/startup is remembered and handled exactly once.
    pub(super) fn finish_overscroll_voice(&mut self, cx: &mut Context<Self>) {
        self.feel_voice_motion_changed();
        self.voice.locked = false;
        if self.voice.starting {
            self.voice.stop_requested = true;
        } else if self.voice.recording.is_some() {
            self.transcribe_voice(cx);
        }
        cx.notify();
    }

    pub(super) fn lock_overscroll_voice(&mut self, cx: &mut Context<Self>) {
        if self.voice.starting || self.voice.recording.is_some() {
            self.voice.locked = true;
            cx.notify();
        }
    }

    pub(super) fn toggle_voice(&mut self, cx: &mut Context<Self>) {
        if self.offline || self.voice.busy {
            return;
        }
        if self.voice.recording.is_some() {
            self.feel.voice_gesture.dismiss();
            self.feel.voice_wheel_at = None;
            self.transcribe_voice(cx);
        } else {
            self.begin_voice(false, cx);
        }
    }

    fn transcribe_voice(&mut self, cx: &mut Context<Self>) {
        if self.account.is_none() {
            self.cancel_voice(cx);
            cx.notify();
            return;
        }
        let Some(recording) = self.voice.recording.take() else {
            return;
        };
        if self.voice.auto_send {
            self.queue_voice_segment(recording, cx);
            return;
        }
        self.feel_voice_motion_changed();
        self.voice.busy = true;
        self.voice.starting = false;
        self.voice.locked = false;
        self.voice.stop_requested = false;
        let generation = self.voice.generation;
        let tab = self.active_tab;
        let task = cx.background_spawn(async move { recording.stop() });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.finish_dictation(generation, tab, result, cx)
            });
        })
        .detach();
        cx.notify();
    }

    fn voice_started(
        &mut self,
        generation: u64,
        tab: u64,
        result: Result<Recording, String>,
        cx: &mut Context<Self>,
    ) {
        if self.voice.generation != generation || self.active_tab != tab {
            return;
        }
        self.feel_voice_motion_changed();
        self.voice.busy = false;
        self.voice.starting = false;
        match result {
            Ok(recording) => {
                self.voice.cancel = Some(recording.cancel_handle());
                self.voice.recording = Some(recording);
                if self.voice.stop_requested {
                    self.transcribe_voice(cx);
                } else {
                    self.start_live_voice(cx);
                }
            }
            Err(error) => {
                self.voice.error = Some(error);
                self.voice.locked = false;
                self.voice.auto_send = false;
                self.voice.began = None;
                self.voice.stop_requested = false;
                // Keep the gesture consumed through release, so failure cannot
                // turn the rest of this drag into a second recording.
                if self.feel.voice_wheel_at.is_some() {
                    self.feel.voice_gesture.abort();
                } else {
                    self.feel.voice_gesture.dismiss();
                }
            }
        }
        cx.notify();
    }

    fn begin_voice(&mut self, auto_send: bool, cx: &mut Context<Self>) {
        let Some(account) = self.account.clone() else {
            self.voice.error = Some("Sign in to use voice transcription.".into());
            cx.notify();
            return;
        };
        self.voice.error = None;
        self.voice.busy = true;
        self.voice.starting = true;
        self.voice.stop_requested = false;
        self.voice.auto_send = auto_send;
        self.voice.locked = false;
        self.voice.began = Some(std::time::Instant::now());
        self.voice.generation = self.voice.generation.wrapping_add(1);
        self.voice.live = LiveDictation {
            original: Some(self.composer.read(cx).save_draft()),
            base: self.composer.read(cx).text().to_string(),
            shown: Some(self.composer.read(cx).text().to_string()),
            ..LiveDictation::default()
        };
        let generation = self.voice.generation;
        let tab = self.active_tab;
        let gate = self.voice.mic_gate.clone();
        let task = cx.background_spawn(async move {
            // A just-finished pulled message may still be releasing the mic.
            if let Some(gate) = gate {
                gate.wait(Duration::from_secs(3));
            }
            Recording::start(account)
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.voice_started(generation, tab, result, cx)
            });
            // Bound capture even when the user never taps stop.
            cx.background_executor()
                .timer(Duration::from_secs(5 * 60))
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.voice.generation == generation
                    && this.active_tab == tab
                    && this.voice.recording.is_some()
                {
                    this.feel.voice_gesture.dismiss();
                    this.transcribe_voice(cx);
                }
            });
        })
        .detach();
        cx.notify();
    }
}

#[cfg(test)]
#[path = "voice_live_tests.rs"]
mod live_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;

    #[gpui::test]
    fn mocked_startup_failure_release_lock_and_retry_states(cx: &mut TestAppContext) {
        cx.update(init);
        let (view, cx) = cx.add_window_view(super::super::lifecycle_coverage::fixture_app);
        view.update(cx, |app, cx| {
            // Mock startup only. Recording::start/stop and transcription are never called.
            app.voice.starting = true;
            app.voice.busy = true;
            app.voice.auto_send = true;
            app.voice.began = Some(std::time::Instant::now());
            assert_eq!(app.voice.status().as_deref(), Some("Preparing microphone…"));
            app.finish_overscroll_voice(cx);
            assert_eq!(
                app.voice.status().as_deref(),
                Some("Finishing microphone startup…")
            );
            app.lock_overscroll_voice(cx);
            assert!(app.voice.locked);
            app.voice_started(
                app.voice.generation,
                app.active_tab,
                Err("permission denied fixture".into()),
                cx,
            );
            assert!(app.voice.has_error());
            assert!(!app.voice.starting && !app.voice.busy && !app.voice.locked);
            assert!(!app.voice.auto_send && !app.voice.stop_requested);
            assert!(app.voice.began.is_none() && app.voice.recording.is_none());
            app.cancel_voice(cx);
            assert!(app.voice.status().is_none());
            // Stop without a recording is a no-op, not a network transcription.
            app.transcribe_voice(cx);
            assert!(!app.voice.busy);
            app.account = None;
            app.voice.busy = true;
            app.transcribe_voice(cx);
            assert!(!app.voice.busy && !app.voice.has_error());
        });
    }

    #[gpui::test]
    fn mocked_callbacks_after_tab_roundtrip_and_signout_are_stale(cx: &mut TestAppContext) {
        cx.update(init);
        let (view, cx) = cx.add_window_view(super::super::lifecycle_coverage::fixture_app);
        view.update(cx, |app, cx| {
            let tab = app.active_tab;
            let generation = app.voice.generation;
            app.voice.starting = true;
            app.voice.busy = true;
            app.new_tab(cx);
            app.focus_tab(tab, cx);
            app.composer
                .update(cx, |input, cx| input.set_text("retained draft", cx));
            app.voice_started(generation, tab, Err("stale startup".into()), cx);
            app.finish_dictation(generation, tab, Ok("stale transcript".into()), cx);
            assert_eq!(app.composer.read(cx).text(), "retained draft");
            assert!(!app.voice.has_error() && !app.voice.busy);
            let generation = app.voice.generation;
            app.voice.busy = true;
            app.voice.auto_send = true;
            app.sign_out(cx);
            app.finish_dictation(generation, tab, Ok("after logout".into()), cx);
            app.voice_started(generation, tab, Err("after logout".into()), cx);
            assert!(app.composer.read(cx).text().is_empty());
            assert!(app.session.transcript.is_empty());
            assert!(!app.voice.has_error() && !app.voice.busy);
        });
    }

    #[gpui::test]
    fn voice_results_preserve_typing_and_never_send(cx: &mut TestAppContext) {
        cx.update(init);
        let (view, cx) = cx.add_window_view(|window, cx| {
            MobileApp::new_offline(
                SafeArea {
                    top: 47.,
                    bottom: 34.,
                },
                window,
                cx,
            )
        });
        view.update(cx, |app, cx| {
            app.composer
                .update(cx, |input, cx| input.set_text("Typed meanwhile", cx));
            app.voice.busy = true;
            app.finish_dictation(
                app.voice.generation,
                app.active_tab,
                Ok("spoken text".into()),
                cx,
            );
            assert_eq!(app.composer.read(cx).text(), "Typed meanwhile spoken text");
            assert!(app.session.transcript.is_empty());
            assert!(!app.voice.busy);
            app.finish_dictation(
                app.voice.generation,
                app.active_tab,
                Err("No speech".into()),
                cx,
            );
            assert_eq!(app.composer.read(cx).text(), "Typed meanwhile spoken text");
            assert_eq!(app.voice.error.as_deref(), Some("No speech"));
        });
    }

    #[gpui::test]
    fn canceled_and_wrong_tab_results_are_discarded(cx: &mut TestAppContext) {
        cx.update(init);
        let (view, cx) = cx.add_window_view(|window, cx| {
            MobileApp::new_offline(
                SafeArea {
                    top: 47.,
                    bottom: 34.,
                },
                window,
                cx,
            )
        });
        view.update(cx, |app, cx| {
            let generation = app.voice.generation;
            app.cancel_voice(cx);
            app.finish_dictation(generation, app.active_tab, Ok("stale".into()), cx);
            app.finish_dictation(
                app.voice.generation,
                app.active_tab + 1,
                Ok("wrong tab".into()),
                cx,
            );
            app.toggle_voice(cx);
            assert!(app.composer.read(cx).text().is_empty());
            assert!(!app.voice.busy);
            assert!(app.voice.recording.is_none());
        });
    }

    #[gpui::test]
    fn quick_voice_send_preserves_unrelated_draft_and_queues_while_processing(
        cx: &mut TestAppContext,
    ) {
        cx.update(init);
        let (view, cx) = cx.add_window_view(|window, cx| {
            MobileApp::new_offline(
                SafeArea {
                    top: 47.,
                    bottom: 34.,
                },
                window,
                cx,
            )
        });
        view.update(cx, |app, cx| {
            app.session.phase = ConnectionPhase::Connected;
            app.session.session_id = Some("voice-fixture".into());
            app.composer
                .update(cx, |input, cx| input.set_text("Do not send this draft", cx));
            app.voice.auto_send = true;
            app.finish_dictation(
                app.voice.generation,
                app.active_tab,
                Ok("  quick voice  ".into()),
                cx,
            );
            assert_eq!(app.session.transcript.last().unwrap().text, "quick voice");
            assert_eq!(app.composer.read(cx).text(), "Do not send this draft");
            app.voice.auto_send = true;
            app.finish_dictation(
                app.voice.generation,
                app.active_tab,
                Ok("follow up".into()),
                cx,
            );
            assert!(app.session.transcript.last().unwrap().is_queued);
            assert_eq!(app.composer.read(cx).text(), "Do not send this draft");
        });
    }

    #[gpui::test]
    fn release_during_startup_and_cancellation_do_not_restart_or_send(cx: &mut TestAppContext) {
        cx.update(init);
        let (view, cx) = cx.add_window_view(|window, cx| {
            MobileApp::new_offline(
                SafeArea {
                    top: 47.,
                    bottom: 34.,
                },
                window,
                cx,
            )
        });
        view.update(cx, |app, cx| {
            app.voice.starting = true;
            app.voice.busy = true;
            app.voice.auto_send = true;
            app.finish_overscroll_voice(cx);
            assert!(app.voice.stop_requested);
            app.lock_overscroll_voice(cx);
            let generation = app.voice.generation;
            app.cancel_voice(cx);
            app.voice_started(
                generation,
                app.active_tab,
                Err("late permission failure".into()),
                cx,
            );
            assert!(!app.voice.busy && !app.voice.locked && !app.voice.auto_send);
            assert!(app.voice.error.is_none());
            assert!(app.session.transcript.is_empty());
        });
    }

    #[gpui::test]
    fn disconnected_or_empty_quick_result_never_sends(cx: &mut TestAppContext) {
        cx.update(init);
        let (view, cx) = cx.add_window_view(|window, cx| {
            MobileApp::new_offline(
                SafeArea {
                    top: 47.,
                    bottom: 34.,
                },
                window,
                cx,
            )
        });
        view.update(cx, |app, cx| {
            app.voice.auto_send = true;
            app.finish_dictation(
                app.voice.generation,
                app.active_tab,
                Ok("saved voice".into()),
                cx,
            );
            assert_eq!(app.composer.read(cx).text(), "saved voice");
            assert!(app.session.transcript.is_empty());
            app.voice.auto_send = true;
            app.finish_dictation(app.voice.generation, app.active_tab, Ok("  ".into()), cx);
            assert!(app.session.transcript.is_empty());
            assert_eq!(app.composer.read(cx).text(), "saved voice");
        });
    }
}
