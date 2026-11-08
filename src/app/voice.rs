//! Ephemeral voice capture. Never persisted, automatically discarded on navigation.
use super::*;
use crate::voice_capture::Recording;

#[derive(Default)]
struct LiveDictation {
    original: Option<crate::text_input::InputDraft>,
    base: String,
    shown: Option<String>,
    transcript: String,
    consumed: usize,
    pending: bool,
    next_preview: Option<std::time::Instant>,
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
    stop_requested: bool,
    generation: u64,
    error: Option<String>,
    live: LiveDictation,
}
impl VoiceState {
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
            Some("Recording. Return to Stop or release to send. Pull farther to lock.".into())
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
}
impl MobileApp {
    /// Remove only our own provisional text. User edits and IME state are never
    /// rolled back if they changed the input after a partial result was painted.
    fn restore_voice_draft(&mut self, cx: &mut Context<Self>) -> bool {
        let owned = self.voice.live.shown.as_deref()
            .is_some_and(|shown| self.composer.read(cx).text() == shown);
        if let Some(draft) = self.voice.live.original.take() {
            if owned {
                self.composer.update(cx, |input, cx| input.restore_draft(draft, cx));
            }
        }
        owned
    }

    fn finish_voice_preview(&mut self, generation: u64, tab: u64, consumed: usize,
        result: Result<String, String>, cx: &mut Context<Self>) {
        if self.voice.generation != generation || self.active_tab != tab
            || !self.voice.live.pending || self.voice.busy || self.voice.starting { return; }
        if result.as_ref().err().is_some_and(|error| matches!(error.as_str(),
            jcode_mobile_core::voice::ACCESS_DENIED | jcode_mobile_core::voice::SIGN_IN_REQUIRED)) {
            // Permanent access failures must not leave a live-looking mic that
            // silently retries forever. Keep any recognized speech for review.
            self.voice.recording = None;
            self.feel.voice_gesture.dismiss();
            self.feel.voice_wheel_at = None;
            self.feel.voice_pan_ended = true;
            self.finish_dictation(generation, tab, result, cx);
            return;
        }
        self.voice.live.pending = false;
        // Transient failures never prevent capture or the authoritative final
        // pass. Move past silence too, rather than repeatedly uploading it.
        self.voice.live.consumed = consumed;
        if let Ok(text) = result {
            self.voice.live.transcript = jcode_mobile_core::voice::merge_preview(
                &self.voice.live.transcript, &text);
            if self.voice.live.shown.as_deref()
                .is_some_and(|shown| self.composer.read(cx).text() == shown) {
                let text = jcode_mobile_core::voice::append(
                    &self.voice.live.base, &self.voice.live.transcript);
                self.composer.update(cx, |input, cx| input.set_text(text.clone(), cx));
                self.voice.live.shown = Some(text);
            }
        }
        cx.notify();
    }

    fn poll_live_voice(&mut self, generation: u64, tab: u64, cx: &mut Context<Self>) -> bool {
        if self.voice.generation != generation || self.active_tab != tab { return false; }
        let Some(recording) = self.voice.recording.as_ref() else { return false; };
        let levels = recording.levels();
        if levels != self.voice.live.levels {
            self.voice.live.levels = levels;
            cx.notify();
        }
        let now = std::time::Instant::now();
        if self.voice.live.pending || self.voice.live.next_preview.is_some_and(|at| now < at) {
            return true;
        }
        let Some(wav) = recording.snapshot() else { return true; };
        self.voice.live.next_preview = Some(now + Duration::from_millis(1500));
        let Some((wav, consumed)) = jcode_mobile_core::voice::preview_chunk(
            &wav, self.voice.live.consumed) else { return true; };
        let Some(account) = self.account.clone() else { return false; };
        self.voice.live.pending = true;
        let task = cx.background_spawn(async move {
            jcode_mobile_core::voice::transcribe_preview(&account, wav)
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.finish_voice_preview(generation, tab, consumed, result, cx);
            });
        }).detach();
        true
    }

    fn start_live_voice(&mut self, cx: &mut Context<Self>) {
        let generation = self.voice.generation;
        let tab = self.active_tab;
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(50)).await;
                if !this.update(cx, |this, cx| this.poll_live_voice(generation, tab, cx))
                    .unwrap_or(false) { break; }
            }
        }).detach();
    }

    /// Start while the finger is still held, not after release.
    pub(super) fn start_overscroll_voice(&mut self, cx: &mut Context<Self>) {
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
        let partial = self.voice.live.transcript.clone();
        let had_preview = !partial.is_empty();
        let owned_preview = self.restore_voice_draft(cx);
        self.voice.live = LiveDictation::default();
        match result {
            Ok(text) if !text.trim().is_empty() => {
                // Voice sends only its own transcript. Never send or clear a draft
                // that the user was typing while the service was working.
                if auto_send
                    && self.session.phase == ConnectionPhase::Connected
                    && self.session.session_id.is_some()
                {
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
                } else {
                    if had_preview && !owned_preview {
                        self.voice.error = Some("Your edited voice draft was kept. Review it before sending.".into());
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
                    let draft = jcode_mobile_core::voice::append(self.composer.read(cx).text(), &partial);
                    self.composer.update(cx, |input, cx| input.set_text(draft, cx));
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

    pub(super) fn cancel_voice(&mut self, cx: &mut Context<Self>) {
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
        let Some(account) = self.account.clone() else {
            self.cancel_voice(cx);
            cx.notify();
            return;
        };
        let Some(recording) = self.voice.recording.take() else {
            return;
        };
        self.feel_voice_motion_changed();
        self.voice.busy = true;
        self.voice.starting = false;
        self.voice.locked = false;
        self.voice.stop_requested = false;
        let generation = self.voice.generation;
        let tab = self.active_tab;
        let task = cx.background_spawn(async move {
            recording
                .stop()
                .and_then(|wav| jcode_mobile_core::voice::transcribe(&account, wav))
        });
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
        if self.account.is_none() {
            self.voice.error = Some("Sign in to use voice transcription.".into());
            cx.notify();
            return;
        }
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
            next_preview: Some(std::time::Instant::now() + Duration::from_millis(1200)),
            ..LiveDictation::default()
        };
        let generation = self.voice.generation;
        let tab = self.active_tab;
        let task = cx.background_spawn(async move { Recording::start() });
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
