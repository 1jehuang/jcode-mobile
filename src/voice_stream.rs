//! Native microphone PCM streamed through the same Nari transport as desktop.
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
    pub fn start() -> Result<Self, String> {
        let key = jcode_voice::nari_api_key().ok_or_else(|| "Nari voice is not configured. Add NARI_API_KEY to your Jcode nari.env configuration.".to_string())?;
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
                        let session = jcode_voice::NariSession::connect(&key, thread_cancel)
                            .await
                            .map_err(|e| e.to_string())?;
                        session
                            .run(rx, |event| {
                                if let jcode_voice::NariEvent::Transcript(text) = event {
                                    if let Ok(mut events) = thread_events.lock() {
                                        events.transcript = Some(text);
                                    }
                                }
                            })
                            .await
                            .map_err(|e| e.to_string())
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
