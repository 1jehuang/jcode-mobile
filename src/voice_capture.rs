//! Microphone capture. All native objects stay on a dedicated recording worker.

pub struct Recording {
    #[cfg(target_os = "ios")]
    worker: Option<imp::Worker>,
}

impl Recording {
    /// Call off the UI thread: permission approval can take up to two minutes.
    pub fn start() -> Result<Self, String> {
        #[cfg(target_os = "ios")]
        {
            Ok(Self {
                worker: Some(imp::start()?),
            })
        }
        #[cfg(not(target_os = "ios"))]
        {
            Err("Microphone recording is supported only on iOS, not in the desktop preview.".into())
        }
    }

    pub fn stop(mut self) -> Result<Vec<u8>, String> {
        #[cfg(target_os = "ios")]
        {
            self.worker
                .take()
                .ok_or("Recording already stopped")?
                .finish(true)
        }
        #[cfg(not(target_os = "ios"))]
        {
            let _ = &mut self;
            Err("Microphone recording is supported only on iOS, not in the desktop preview.".into())
        }
    }
}

impl Drop for Recording {
    fn drop(&mut self) {
        #[cfg(target_os = "ios")]
        if let Some(worker) = self.worker.take() {
            worker.cancel();
        }
    }
}

#[cfg(target_os = "ios")]
mod imp {
    use objc2::{
        msg_send,
        rc::{Allocated, Retained},
        runtime::{AnyClass, AnyObject, Bool},
    };
    use objc2_foundation::NSString;
    use std::{
        fs::{self, File, OpenOptions},
        io::Read,
        path::PathBuf,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
            mpsc,
        },
        thread,
        time::Duration,
    };

    #[link(name = "AVFoundation", kind = "framework")]
    unsafe extern "C" {
        static AVAudioSessionInterruptionNotification: *mut AnyObject;
    }
    #[link(name = "UIKit", kind = "framework")]
    unsafe extern "C" {
        static UIApplicationDidEnterBackgroundNotification: *mut AnyObject;
    }

    const MAX_BYTES: u64 = 10 * 1024 * 1024;
    static RESERVED: AtomicBool = AtomicBool::new(false);
    struct Reservation;
    impl Drop for Reservation {
        fn drop(&mut self) {
            RESERVED.store(false, Ordering::Release);
        }
    }
    pub(super) struct Worker {
        command: mpsc::Sender<bool>,
        join: thread::JoinHandle<Result<Vec<u8>, String>>,
    }
    impl Worker {
        pub(super) fn cancel(self) {
            let _ = self.command.send(false);
            // Dropping JoinHandle detaches. Cleanup runs on the worker, never the UI thread.
        }
        pub(super) fn finish(self, keep: bool) -> Result<Vec<u8>, String> {
            let _ = self.command.send(keep);
            self.join
                .join()
                .map_err(|_| "Audio recording worker panicked".to_owned())?
        }
    }

    pub(super) fn start() -> Result<Worker, String> {
        RESERVED
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| {
                "Another microphone recording is starting, recording, or cleaning up".to_owned()
            })?;
        let reservation = Reservation;
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let (command, commands) = mpsc::channel();
        let notifications_command = command.clone();
        let join = thread::Builder::new()
            .name("voice-capture".into())
            .spawn(move || {
                let _reservation = reservation;
                objc2::rc::autoreleasepool(|_| {
                    let interrupted = Arc::new(AtomicBool::new(false));
                    let _observers = match unsafe { Observers::new(notifications_command, interrupted.clone()) } {
                        Ok(observers) => observers,
                        Err(error) => { let _ = ready_tx.send(Err(error)); return Ok(Vec::new()); }
                    };
                    let capture = unsafe { Capture::start(&interrupted) };
                    let mut capture = match capture {
                        Ok(capture) => capture,
                        Err(error) => {
                            let _ = ready_tx.send(Err(error));
                            return Ok(Vec::new());
                        }
                    };
                    if interrupted.load(Ordering::Acquire) {
                        let _ = ready_tx.send(Err("Recording interrupted during setup".into()));
                        return Ok(Vec::new());
                    }
                    if ready_tx.send(Ok(())).is_err() {
                        return Ok(Vec::new());
                    }
                    // Native recordForDuration also enforces this limit independently.
                    let keep = match commands.recv_timeout(Duration::from_secs(300)) {
                        Ok(keep) => keep,
                        Err(mpsc::RecvTimeoutError::Timeout) => true,
                        Err(_) => false,
                    };
                    capture.stop();
                    if interrupted.load(Ordering::Acquire) {
                        return Err("Recording discarded because the app entered the background or audio was interrupted".into());
                    }
                    if !keep {
                        return Ok(Vec::new());
                    }
                    let file = File::open(&capture.path.0)
                        .map_err(|e| format!("Cannot open recorded audio: {e}"))?;
                    if file.metadata().map_err(|e| e.to_string())?.len() > MAX_BYTES {
                        return Err("Recorded audio exceeds the 10 MiB limit".into());
                    }
                    let mut bytes = Vec::new();
                    file.take(MAX_BYTES + 1)
                        .read_to_end(&mut bytes)
                        .map_err(|e| format!("Cannot read recorded audio: {e}"))?;
                    if bytes.len() as u64 > MAX_BYTES {
                        return Err("Recorded audio exceeds the 10 MiB limit".into());
                    }
                    if bytes.len() < 44 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
                        return Err("Recording did not produce a valid WAV file".into());
                    }
                    Ok(bytes)
                })
            })
            .map_err(|e| format!("Cannot start recording worker: {e}"))?;
        match ready_rx.recv() {
            Ok(Ok(())) => Ok(Worker { command, join }),
            Ok(Err(error)) => {
                let _ = join.join();
                Err(error)
            }
            Err(_) => {
                let _ = join.join();
                Err("Audio recording worker stopped during startup".into())
            }
        }
    }

    // Notification callbacks only signal the worker. They never access native recorder objects.
    struct Observers {
        center: Retained<AnyObject>,
        tokens: Vec<Retained<AnyObject>>,
    }
    impl Observers {
        unsafe fn new(
            command: mpsc::Sender<bool>,
            interrupted: Arc<AtomicBool>,
        ) -> Result<Self, String> {
            unsafe {
                let center: Retained<AnyObject> =
                    msg_send![class(c"NSNotificationCenter")?, defaultCenter];
                let mut observers = Self {
                    center,
                    tokens: Vec::new(),
                };
                for name in [
                    AVAudioSessionInterruptionNotification,
                    UIApplicationDidEnterBackgroundNotification,
                ] {
                    let command = command.clone();
                    let interrupted = interrupted.clone();
                    let callback = block2::RcBlock::new(move |_: *mut AnyObject| {
                        interrupted.store(true, Ordering::Release);
                        let _ = command.send(false);
                    });
                    let nil: *mut AnyObject = std::ptr::null_mut();
                    let token: Option<Retained<AnyObject>> = msg_send![&*observers.center, addObserverForName: name, object: nil, queue: nil, usingBlock: &*callback];
                    observers
                        .tokens
                        .push(token.ok_or("Cannot monitor recording interruptions")?);
                }
                Ok(observers)
            }
        }
    }
    impl Drop for Observers {
        fn drop(&mut self) {
            for token in &self.tokens {
                unsafe {
                    let _: () = msg_send![&*self.center, removeObserver: &**token];
                }
            }
        }
    }

    struct TempPath(PathBuf);
    impl Drop for TempPath {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
        }
    }
    struct Capture {
        recorder: Option<Retained<AnyObject>>,
        session: Retained<AnyObject>,
        active: bool,
        path: TempPath,
    }
    impl Capture {
        // SAFETY: called only on the recording worker inside an autorelease pool.
        unsafe fn start(interrupted: &AtomicBool) -> Result<Self, String> {
            unsafe {
                let session_cls = class(c"AVAudioSession")?;
                let session: Retained<AnyObject> = msg_send![session_cls, sharedInstance];
                // Avoid the OS terminating the app when its usage description is missing.
                let bundle: *mut AnyObject = msg_send![class(c"NSBundle")?, mainBundle];
                let usage: *mut AnyObject = msg_send![bundle, objectForInfoDictionaryKey: &*NSString::from_str("NSMicrophoneUsageDescription")];
                if usage.is_null() {
                    return Err(
                        "iOS requires NSMicrophoneUsageDescription in Info.plist to record audio"
                            .into(),
                    );
                }
                let (tx, rx) = mpsc::sync_channel(1);
                let permission = block2::RcBlock::new(move |allowed: Bool| {
                    let _ = tx.send(allowed.as_bool());
                });
                let _: () = msg_send![&*session, requestRecordPermission: &*permission];
                if !rx
                    .recv_timeout(Duration::from_secs(120))
                    .map_err(|_| "Microphone permission request timed out".to_owned())?
                {
                    return Err("Microphone access denied. Enable it in iOS Settings > Privacy & Security > Microphone.".into());
                }
                if interrupted.load(Ordering::Acquire) {
                    return Err("Recording canceled while waiting for microphone permission".into());
                }
                let uuid: *mut AnyObject = msg_send![class(c"NSUUID")?, UUID];
                let uuid: Retained<NSString> = msg_send![uuid, UUIDString];
                let path = std::env::temp_dir().join(format!("jcode-voice-{uuid}.wav"));
                // Reserve a unique, private path before handing it to AVAudioRecorder.
                use std::os::unix::fs::OpenOptionsExt;
                OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .open(&path)
                    .map_err(|e| format!("Cannot create temporary audio file: {e}"))?;
                // Only remove paths we successfully created, never a colliding file.
                let path = TempPath(path);
                let mut capture = Self {
                    recorder: None,
                    session,
                    active: false,
                    path,
                };
                let mut error: *mut AnyObject = std::ptr::null_mut();
                let category = NSString::from_str("AVAudioSessionCategoryRecord");
                let ok: Bool =
                    msg_send![&*capture.session, setCategory: &*category, error: &mut error];
                if !ok.as_bool() {
                    return Err(native_error("Cannot configure microphone session", error));
                }
                error = std::ptr::null_mut();
                let ok: Bool =
                    msg_send![&*capture.session, setActive: Bool::YES, error: &mut error];
                if !ok.as_bool() {
                    return Err(native_error("Cannot activate microphone session", error));
                }
                capture.active = true;
                let settings: Retained<AnyObject> = msg_send![class(c"NSMutableDictionary")?, new];
                for (key, value) in [
                    ("AVFormatIDKey", 0x6c70636du32),
                    ("AVNumberOfChannelsKey", 1),
                    ("AVLinearPCMBitDepthKey", 16),
                ] {
                    let number: *mut AnyObject =
                        msg_send![class(c"NSNumber")?, numberWithUnsignedInt: value];
                    let _: () =
                        msg_send![&*settings, setObject: number, forKey: &*NSString::from_str(key)];
                }
                let rate: *mut AnyObject =
                    msg_send![class(c"NSNumber")?, numberWithDouble: 16000.0f64];
                let _: () = msg_send![&*settings, setObject: rate, forKey: &*NSString::from_str("AVSampleRateKey")];
                for key in [
                    "AVLinearPCMIsFloatKey",
                    "AVLinearPCMIsBigEndianKey",
                    "AVLinearPCMIsNonInterleaved",
                ] {
                    let no: *mut AnyObject =
                        msg_send![class(c"NSNumber")?, numberWithBool: Bool::NO];
                    let _: () =
                        msg_send![&*settings, setObject: no, forKey: &*NSString::from_str(key)];
                }
                let path = capture
                    .path
                    .0
                    .to_str()
                    .ok_or("Temporary audio path is not UTF-8")?;
                let url: *mut AnyObject =
                    msg_send![class(c"NSURL")?, fileURLWithPath: &*NSString::from_str(path)];
                let allocated: Allocated<AnyObject> = msg_send![class(c"AVAudioRecorder")?, alloc];
                error = std::ptr::null_mut();
                capture.recorder =
                    msg_send![allocated, initWithURL: url, settings: &*settings, error: &mut error];
                let recorder = capture
                    .recorder
                    .as_ref()
                    .ok_or_else(|| native_error("Cannot create audio recorder", error))?;
                if interrupted.load(Ordering::Acquire) {
                    return Err("Recording interrupted during setup".into());
                }
                let ok: Bool = msg_send![&**recorder, recordForDuration: 300.0f64];
                if !ok.as_bool() {
                    return Err("iOS could not start microphone recording".into());
                }
                Ok(capture)
            }
        }
        fn stop(&mut self) {
            unsafe {
                if let Some(recorder) = self.recorder.take() {
                    let _: () = msg_send![&*recorder, stop];
                }
                if self.active {
                    let mut error: *mut AnyObject = std::ptr::null_mut();
                    let _: Bool = msg_send![&*self.session, setActive: Bool::NO, withOptions: 1usize, error: &mut error];
                    self.active = false;
                }
            }
        }
    }
    impl Drop for Capture {
        fn drop(&mut self) {
            self.stop();
        }
    }
    fn class(name: &std::ffi::CStr) -> Result<&'static AnyClass, String> {
        AnyClass::get(name)
            .ok_or_else(|| format!("iOS audio class {} is unavailable", name.to_string_lossy()))
    }
    unsafe fn native_error(context: &str, error: *mut AnyObject) -> String {
        if error.is_null() {
            return context.into();
        }
        let description: Retained<NSString> = unsafe { msg_send![error, localizedDescription] };
        format!("{context}: {description}")
    }
}

#[cfg(all(test, not(target_os = "ios")))]
mod tests {
    #[test]
    fn preview_cannot_record() {
        let error = super::Recording::start()
            .err()
            .expect("preview must not capture audio");
        assert!(error.contains("iOS") && error.contains("preview"));
    }
}
