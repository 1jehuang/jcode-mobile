//! Ephemeral voice capture. Never persisted, automatically discarded on navigation.
use super::*;
use crate::voice_capture::Recording;

#[derive(Default)]
pub(super) struct VoiceState {
    pub recording: Option<Recording>,
    pub busy: bool,
    generation: u64,
    error: Option<String>,
}
impl VoiceState {
    pub fn status(&self) -> Option<String> {
        if self.recording.is_some() {
            Some("Recording. Tap stop to transcribe, or cancel to discard.".into())
        } else if self.busy {
            Some("Preparing microphone or transcribing…".into())
        } else { self.error.clone() }
    }
}
impl MobileApp {
    /// Overscroll only starts capture. Repeated gestures must never stop it.
    pub(super) fn start_overscroll_voice(&mut self, cx: &mut Context<Self>) {
        if cfg!(target_os = "ios") && self.scene_allows_voice() {
            self.toggle_voice(cx);
        }
    }

    fn scene_allows_voice(&self) -> bool {
        self.screen == Screen::Chat && !self.offline && self.account.is_some()
            && !self.voice.busy && self.voice.recording.is_none()
            && !self.model_picker && self.tab_menu.is_none()
    }

    fn finish_dictation(&mut self, generation: u64, tab: u64, result: Result<String, String>, cx: &mut Context<Self>) {
        if self.voice.generation != generation || self.active_tab != tab { return; }
        self.voice.busy = false;
        match result {
            Ok(text) => {
                let draft = jcode_mobile_core::voice::append(self.composer.read(cx).text(), &text);
                self.composer.update(cx, |input, cx| input.set_text(draft, cx));
            }
            Err(error) => self.voice.error = Some(error),
        }
        cx.notify();
    }

    pub(super) fn cancel_voice(&mut self) {
        self.voice.generation = self.voice.generation.wrapping_add(1);
        self.voice.recording = None;
        self.voice.busy = false;
        self.voice.error = None;
    }

    pub(super) fn toggle_voice(&mut self, cx: &mut Context<Self>) {
        if self.offline { return; }
        if self.voice.busy { return; }
        let Some(account) = self.account.clone() else {
            self.voice.error = Some("Sign in to use voice transcription.".into());
            cx.notify();
            return;
        };
        self.voice.error = None;
        self.voice.busy = true;
        if self.voice.recording.is_none() {
            self.voice.generation = self.voice.generation.wrapping_add(1);
        }
        let generation = self.voice.generation;
        let tab = self.active_tab;
        if let Some(recording) = self.voice.recording.take() {
            let task = cx.background_spawn(async move {
                recording.stop().and_then(|wav| jcode_mobile_core::voice::transcribe(&account, wav))
            });
            cx.spawn(async move |this, cx| {
                let result = task.await;
                let _ = this.update(cx, |this, cx| {
                    this.finish_dictation(generation, tab, result, cx);
                });
            }).detach();
        } else {
            let task = cx.background_spawn(async move { Recording::start() });
            cx.spawn(async move |this, cx| {
                let result = task.await;
                let _ = this.update(cx, |this, cx| {
                    if this.voice.generation != generation || this.active_tab != tab { return; }
                    this.voice.busy = false;
                    match result {
                        Ok(recording) => this.voice.recording = Some(recording),
                        Err(error) => this.voice.error = Some(error),
                    }
                    cx.notify();
                });
                // Bound capture even when the user never taps stop.
                cx.background_executor().timer(Duration::from_secs(5 * 60)).await;
                let _ = this.update(cx, |this, cx| {
                    if this.voice.generation == generation && this.active_tab == tab && this.voice.recording.is_some() {
                        this.toggle_voice(cx);
                    }
                });
            }).detach();
        }
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;

    #[gpui::test]
    fn voice_results_preserve_typing_and_never_send(cx: &mut TestAppContext) {
        cx.update(init);
        let (view, cx) = cx.add_window_view(|window, cx| MobileApp::new_offline(SafeArea { top: 47., bottom: 34. }, window, cx));
        view.update(cx, |app, cx| {
            app.composer.update(cx, |input, cx| input.set_text("Typed meanwhile", cx));
            app.voice.busy = true;
            app.finish_dictation(app.voice.generation, app.active_tab, Ok("spoken text".into()), cx);
            assert_eq!(app.composer.read(cx).text(), "Typed meanwhile spoken text");
            assert!(app.session.transcript.is_empty());
            assert!(!app.voice.busy);
            app.finish_dictation(app.voice.generation, app.active_tab, Err("No speech".into()), cx);
            assert_eq!(app.composer.read(cx).text(), "Typed meanwhile spoken text");
            assert_eq!(app.voice.error.as_deref(), Some("No speech"));
        });
    }

    #[gpui::test]
    fn canceled_and_wrong_tab_results_are_discarded(cx: &mut TestAppContext) {
        cx.update(init);
        let (view, cx) = cx.add_window_view(|window, cx| MobileApp::new_offline(SafeArea { top: 47., bottom: 34. }, window, cx));
        view.update(cx, |app, cx| {
            let generation = app.voice.generation;
            app.cancel_voice();
            app.finish_dictation(generation, app.active_tab, Ok("stale".into()), cx);
            app.finish_dictation(app.voice.generation, app.active_tab + 1, Ok("wrong tab".into()), cx);
            app.toggle_voice(cx);
            assert!(app.composer.read(cx).text().is_empty());
            assert!(!app.voice.busy);
            assert!(app.voice.recording.is_none());
        });
    }
}
