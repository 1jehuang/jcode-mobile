//! Native microphone PCM streamed through the authenticated Jcode cloud Nari relay.
use jcode_mobile_core::cloud::Account;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::thread::JoinHandle;

#[derive(Default)]
struct Events {
    transcript: Option<String>,
    finished: Option<Result<String, String>>,
}

pub struct Recording {
    capture: Option<crate::voice_capture::Recording>,
    commit_guard: Option<tokio::sync::mpsc::Sender<Vec<i16>>>,
    events: Arc<Mutex<Events>>,
    cancel: Arc<AtomicBool>,
    worker: Option<JoinHandle<Result<String, String>>>,
}

impl Recording {
    /// Called on a background executor, never the UI thread.
    pub fn start(account: Account) -> Result<Self, String> {
        let relay = relay_url(&account.api_base)?;
        if account.api_key.is_empty() {
            return Err("Sign in to use voice transcription.".into());
        }
        // Twenty seconds of 50 ms frames bounds memory while network setup completes.
        let (tx, rx) = tokio::sync::mpsc::channel(400);
        // Keep EOF gated until stop confirms successful capture. On cancellation
        // or capture failure the cancel flag is raised before this guard drops.
        let commit_guard = tx.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let events = Arc::new(Mutex::new(Events::default()));
        let thread_cancel = cancel.clone();
        let thread_events = events.clone();
        let final_events = events.clone();
        let worker = std::thread::Builder::new()
            .name("mobile-nari".into())
            .spawn(move || {
                let result = (|| {
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|_| "Unable to start voice networking.".to_string())?;
                    runtime.block_on(async move {
                        let session = jcode_voice::NariSession::connect_relay(
                            &relay,
                            &account.api_key,
                            thread_cancel,
                        )
                        .await
                        .map_err(cloud_voice_error)?;
                        session
                            .run(rx, |event| {
                                if let jcode_voice::NariEvent::Transcript(text) = event {
                                    if let Ok(mut events) = thread_events.lock() {
                                        events.transcript = Some(text);
                                    }
                                }
                            })
                            .await
                            .map_err(cloud_voice_error)
                    })
                })();
                if let Ok(mut events) = final_events.lock() {
                    events.finished = Some(result.clone());
                }
                result
            })
            .map_err(|_| "Unable to start voice networking.".to_string())?;
        let capture = match crate::voice_capture::Recording::start_stream(tx) {
            Ok(capture) => capture,
            Err(error) => {
                cancel.store(true, Ordering::Release);
                return Err(error);
            }
        };
        // The event receiver needs the same bounded state as the network worker.
        // The worker owns a separate clone, so UI polling cannot block networking.
        Ok(Self {
            capture: Some(capture),
            commit_guard: Some(commit_guard),
            events,
            cancel,
            worker: Some(worker),
        })
    }

    pub fn levels(&self) -> Vec<f32> {
        self.capture
            .as_ref()
            .map(|c| c.levels())
            .unwrap_or_default()
    }
    pub fn cancel_handle(&self) -> Arc<AtomicBool> {
        self.cancel.clone()
    }
    pub fn poll_transcript(&self) -> Option<String> {
        self.events.lock().ok()?.transcript.take()
    }
    pub fn poll_finished(&self) -> Option<Result<String, String>> {
        if let Some(error) = self.capture.as_ref().and_then(|c| c.error()) {
            return Some(Err(error));
        }
        self.events.lock().ok()?.finished.take()
    }
    /// Capture must finish successfully before its sender EOF commits Nari audio.
    pub fn stop(mut self) -> Result<String, String> {
        if let Some(capture) = self.capture.take() {
            if let Err(error) = capture.stop() {
                self.cancel.store(true, Ordering::Release);
                return Err(error);
            }
        }
        self.commit_guard.take();
        self.worker
            .take()
            .ok_or_else(|| "Voice session has already finished.".to_string())?
            .join()
            .map_err(|_| "Voice networking stopped unexpectedly.".to_string())?
    }
}

/// Derive the relay only from the signed-in account's configured HTTPS API.
/// Never redirect an account bearer to a separately configured provider URL.
fn relay_url(api_base: &str) -> Result<String, String> {
    let invalid = || "The signed-in cloud voice endpoint is invalid.".to_string();
    let mut url = url::Url::parse(api_base).map_err(|_| invalid())?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(invalid());
    }
    let path = format!("{}/voice/realtime", url.path().trim_end_matches('/'));
    url.set_path(&path);
    url.set_scheme("wss").map_err(|_| invalid())?;
    Ok(url.into())
}

fn cloud_voice_error(error: jcode_voice::VoiceError) -> String {
    use jcode_voice::VoiceError;
    match error {
        VoiceError::NariNotConfigured | VoiceError::Http(401) => {
            "Cloud voice authorization failed. Check your Jcode sign-in and subscription.".into()
        }
        VoiceError::Http(403) | VoiceError::Unavailable => {
            "Cloud voice is unavailable for this Jcode account.".into()
        }
        VoiceError::NariCreditsExhausted | VoiceError::Http(402) => {
            "Cloud voice capacity is unavailable. Please try again later.".into()
        }
        VoiceError::NariRejected | VoiceError::Http(404 | 503) => {
            "Cloud voice is not ready. Please try again later.".into()
        }
        other => other.to_string(),
    }
}
impl Drop for Recording {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
        // Capture Drop and cancellation are nonblocking. Never join on the UI thread.
        self.capture.take();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cloud_errors_do_not_request_a_phone_provider_key() {
        for error in [
            jcode_voice::VoiceError::NariNotConfigured,
            jcode_voice::VoiceError::NariCreditsExhausted,
            jcode_voice::VoiceError::NariRejected,
            jcode_voice::VoiceError::Http(404),
            jcode_voice::VoiceError::Http(503),
        ] {
            let message = cloud_voice_error(error);
            assert!(message.starts_with("Cloud voice"));
            assert!(!message.contains("API key"));
            assert!(!message.contains("Add credits"));
        }
    }

    #[test]
    fn cloud_relay_is_derived_from_account_api_without_credential_overrides() {
        assert_eq!(
            relay_url("https://cloud.example/v1/").unwrap(),
            "wss://cloud.example/v1/voice/realtime"
        );
        for base in [
            "http://cloud.example/v1",
            "https://user:secret@cloud.example/v1",
            "https://cloud.example/v1?redirect=elsewhere",
            "https://cloud.example/v1#fragment",
            "not a URL",
        ] {
            assert!(relay_url(base).is_err(), "accepted {base}");
        }
    }

    fn fake() -> (Recording, tokio::sync::mpsc::Receiver<Vec<i16>>) {
        let (tx, rx) = tokio::sync::mpsc::channel(2);
        (
            Recording {
                capture: None,
                commit_guard: Some(tx),
                events: Arc::new(Mutex::new(Events::default())),
                cancel: Arc::new(AtomicBool::new(false)),
                worker: None,
            },
            rx,
        )
    }

    #[test]
    fn successful_stop_commits_only_after_guard_is_released() {
        let (mut recording, mut rx) = fake();
        assert!(matches!(
            rx.try_recv(),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty)
        ));
        let cancel = recording.cancel.clone();
        recording.worker = Some(std::thread::spawn(move || {
            assert!(rx.blocking_recv().is_none());
            assert!(!cancel.load(Ordering::Acquire));
            Ok("authoritative final".into())
        }));
        assert_eq!(recording.stop().unwrap(), "authoritative final");
    }

    #[test]
    fn cancellation_precedes_audio_eof_without_joining_network() {
        let (recording, mut rx) = fake();
        let cancel = recording.cancel.clone();
        drop(recording);
        assert!(rx.blocking_recv().is_none());
        assert!(cancel.load(Ordering::Acquire));
    }

    #[test]
    fn retained_cancel_handle_interrupts_background_finalization() {
        let (mut recording, mut rx) = fake();
        let cancel = recording.cancel_handle();
        let worker_cancel = cancel.clone();
        recording.worker = Some(std::thread::spawn(move || {
            assert!(rx.blocking_recv().is_none());
            while !worker_cancel.load(Ordering::Acquire) {
                std::thread::yield_now();
            }
            Err("Voice recording cancelled".into())
        }));
        let finish = std::thread::spawn(move || recording.stop());
        cancel.store(true, Ordering::Release);
        assert!(finish.join().unwrap().is_err());
    }

    #[test]
    fn latest_full_revision_is_coalesced_and_terminal_result_is_drained() {
        let (recording, _) = fake();
        {
            let mut state = recording.events.lock().unwrap();
            state.transcript = Some("first hypothesis".into());
            state.transcript = Some("corrected hypothesis".into());
            state.finished = Some(Err("Nari credits exhausted".into()));
        }
        assert_eq!(
            recording.poll_transcript().as_deref(),
            Some("corrected hypothesis")
        );
        assert!(recording.poll_transcript().is_none());
        assert!(recording.poll_finished().unwrap().is_err());
        assert!(recording.poll_finished().is_none());
    }
}
