//! Microphone capture. All native objects stay on a dedicated recording worker.

use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};

#[derive(Default)]
struct Live {
    requested: AtomicBool,
    state: Mutex<LiveState>,
}
#[derive(Default)]
struct LiveState {
    snapshot: Option<Vec<u8>>,
    levels: std::collections::VecDeque<f32>,
    error: Option<String>,
}
impl Live {
    fn snapshot(&self) -> Option<Vec<u8>> {
        let result = self.state.try_lock().ok()?.snapshot.take();
        if result.is_none() {
            self.requested.store(true, Ordering::Release);
        }
        result
    }
    fn level(&self, level: f32) {
        if let Ok(mut state) = self.state.lock() {
            if state.levels.len() == 24 {
                state.levels.pop_front();
            }
            state.levels.push_back(if level.is_finite() {
                level.clamp(0.0, 1.0)
            } else {
                0.0
            });
        }
    }
    fn publish(&self, bytes: Option<Vec<u8>>) {
        if let Ok(mut state) = self.state.lock() {
            state.snapshot = bytes;
        }
    }
    fn complete(&self, result: &Result<Vec<u8>, String>) {
        if let Err(error) = result {
            if let Ok(mut state) = self.state.lock() {
                state.error = Some(error.clone());
            }
        }
    }
}

/// The sender lives only on the capture worker. A slow consumer must fail closed:
/// dropping audio would change the words heard by the transcription service.
#[derive(Default)]
struct PcmStream {
    sender: Option<tokio::sync::mpsc::Sender<Vec<i16>>>,
    pending: Vec<u8>,
}
impl PcmStream {
    fn new(sender: Option<tokio::sync::mpsc::Sender<Vec<i16>>>) -> Self {
        Self {
            sender,
            pending: Vec::new(),
        }
    }
    fn push(&mut self, mut pcm: &[u8]) -> Result<(), String> {
        if self.sender.is_none() {
            return Ok(());
        }
        while !pcm.is_empty() {
            let count = (1600 - self.pending.len()).min(pcm.len());
            self.pending.extend_from_slice(&pcm[..count]);
            pcm = &pcm[count..];
            if self.pending.len() == 1600 {
                self.send_pending()?;
            }
        }
        Ok(())
    }
    fn send_pending(&mut self) -> Result<(), String> {
        if self.pending.is_empty() {
            return Ok(());
        }
        let samples = self
            .pending
            .chunks_exact(2)
            .map(|s| i16::from_le_bytes([s[0], s[1]]))
            .collect();
        self.sender.as_ref().ok_or("PCM stream is closed")?.try_send(samples)
            .map_err(|error| match error {
                tokio::sync::mpsc::error::TrySendError::Full(_) =>
                    "Live transcription cannot keep up with microphone audio. Recording stopped to avoid losing speech.".to_owned(),
                tokio::sync::mpsc::error::TrySendError::Closed(_) =>
                    "Live transcription audio stream closed unexpectedly".to_owned(),
            })?;
        self.pending.clear();
        Ok(())
    }
    fn finish(&mut self) -> Result<(), String> {
        if self.pending.len() % 2 != 0 {
            return Err("Microphone returned an incomplete PCM sample".into());
        }
        self.send_pending()?;
        self.sender.take();
        Ok(())
    }
}

/// Incremental file reader for AVAudioRecorder's growing WAV. Chunk lengths may
/// remain placeholders until stop, so only the data offset and actual file size
/// are used. Header scans and each read allocation are bounded.
#[cfg(any(target_os = "ios", test))]
#[derive(Default)]
struct WavPcmTail {
    offset: Option<u64>,
}
#[cfg(any(target_os = "ios", test))]
impl WavPcmTail {
    fn poll(&mut self, path: &std::path::Path, stream: &mut PcmStream) -> Result<(), String> {
        use std::io::{Read, Seek, SeekFrom};
        if stream.sender.is_none() {
            return Ok(());
        }
        let mut file = std::fs::File::open(path)
            .map_err(|e| format!("Cannot read live microphone audio: {e}"))?;
        let size = file.metadata().map_err(|e| e.to_string())?.len();
        if size > 10 * 1024 * 1024 {
            return Err("Recorded audio exceeds the 10 MiB limit".into());
        }
        if self.offset.is_none() {
            self.offset = Self::data_offset(&mut file, size)?;
        }
        let Some(mut offset) = self.offset else {
            return Ok(());
        };
        if size < offset {
            return Err("Microphone audio file shrank during recording".into());
        }
        file.seek(SeekFrom::Start(offset))
            .map_err(|e| e.to_string())?;
        let mut buffer = [0u8; 1600];
        while offset < size {
            let count = (size - offset).min(buffer.len() as u64) as usize;
            file.read_exact(&mut buffer[..count])
                .map_err(|e| e.to_string())?;
            stream.push(&buffer[..count])?;
            offset += count as u64;
            self.offset = Some(offset);
        }
        Ok(())
    }

    fn data_offset(file: &mut std::fs::File, size: u64) -> Result<Option<u64>, String> {
        use std::io::{Read, Seek, SeekFrom};
        if size < 12 {
            return Ok(None);
        }
        let mut header = [0u8; 12];
        file.read_exact(&mut header).map_err(|e| e.to_string())?;
        if &header[..4] != b"RIFF" || &header[8..] != b"WAVE" {
            return Err("Microphone did not produce WAV audio".into());
        }
        let mut offset = 12u64;
        let mut valid_format = false;
        while offset + 8 <= size && offset <= 65536 {
            file.seek(SeekFrom::Start(offset))
                .map_err(|e| e.to_string())?;
            let mut chunk = [0u8; 8];
            file.read_exact(&mut chunk).map_err(|e| e.to_string())?;
            let len = u32::from_le_bytes(chunk[4..].try_into().unwrap()) as u64;
            if &chunk[..4] == b"data" {
                if !valid_format {
                    return Err("Microphone WAV format is not mono PCM16 at 16 kHz".into());
                }
                return Ok(Some(offset + 8));
            }
            if &chunk[..4] == b"fmt " {
                if len < 16 {
                    return Err("Microphone WAV format header is invalid".into());
                }
                if offset + 24 > size {
                    return Ok(None);
                }
                let mut format = [0u8; 16];
                file.read_exact(&mut format).map_err(|e| e.to_string())?;
                valid_format = u16::from_le_bytes(format[..2].try_into().unwrap()) == 1
                    && u16::from_le_bytes(format[2..4].try_into().unwrap()) == 1
                    && u32::from_le_bytes(format[4..8].try_into().unwrap()) == 16000
                    && u16::from_le_bytes(format[12..14].try_into().unwrap()) == 2
                    && u16::from_le_bytes(format[14..].try_into().unwrap()) == 16;
            }
            offset += 8 + len + len % 2;
        }
        if offset > 65536 {
            return Err("Microphone WAV header exceeds the 64 KiB limit".into());
        }
        Ok(None)
    }
}

fn pcm_level(pcm: &[u8]) -> f32 {
    let count = pcm.len() / 2;
    if count == 0 {
        return 0.0;
    }
    (pcm.chunks_exact(2)
        .map(|s| {
            let value = i16::from_le_bytes([s[0], s[1]]) as f64 / 32768.0;
            value * value
        })
        .sum::<f64>()
        / count as f64)
        .sqrt() as f32
}

#[derive(Default)]
struct PcmMeter {
    // At 16 kHz, 800 complete samples represent exactly 50ms. Preserve partial
    // windows across pipe reads so cadence never depends on subprocess buffering.
    window: Vec<u8>,
}
impl PcmMeter {
    fn push(&mut self, mut pcm: &[u8], live: &Live) {
        while !pcm.is_empty() {
            let count = (1600 - self.window.len()).min(pcm.len());
            self.window.extend_from_slice(&pcm[..count]);
            pcm = &pcm[count..];
            if self.window.len() == 1600 {
                live.level(pcm_level(&self.window));
                self.window.clear();
            }
        }
    }
}

// AVAudioRecorder's in-progress file can have placeholder chunk sizes. Walk chunks,
// not fixed offsets: native WAV files may include JUNK/FLLR before their data.
fn current_wav(mut bytes: Vec<u8>) -> Option<Vec<u8>> {
    if bytes.len() < 44
        || bytes.len() > 10 * 1024 * 1024
        || &bytes[..4] != b"RIFF"
        || &bytes[8..12] != b"WAVE"
    {
        return None;
    }
    let mut offset = 12usize;
    let mut alignment = None;
    while offset.checked_add(8)? <= bytes.len() {
        let size = u32::from_le_bytes(bytes[offset + 4..offset + 8].try_into().ok()?) as usize;
        if &bytes[offset..offset + 4] == b"fmt " {
            if size < 16 || offset.checked_add(8 + size)? > bytes.len() {
                return None;
            }
            if u16::from_le_bytes(bytes[offset + 8..offset + 10].try_into().ok()?) != 1 {
                return None;
            }
            alignment =
                Some(u16::from_le_bytes(bytes[offset + 20..offset + 22].try_into().ok()?) as usize);
        }
        if &bytes[offset..offset + 4] == b"data" {
            let alignment = alignment.filter(|a| *a > 0)?;
            let available = bytes.len() - offset - 8;
            let size = available / alignment * alignment;
            if size == 0 {
                return None;
            }
            bytes.truncate(offset + 8 + size);
            bytes[offset + 4..offset + 8].copy_from_slice(&(size as u32).to_le_bytes());
            let riff_size = (bytes.len() - 8) as u32;
            bytes[4..8].copy_from_slice(&riff_size.to_le_bytes());
            return Some(bytes);
        }
        offset = offset
            .checked_add(8)?
            .checked_add(size)?
            .checked_add(size % 2)?;
    }
    None
}

pub struct Recording {
    #[cfg(any(target_os = "ios", target_os = "linux"))]
    worker: Option<imp::Worker>,
}

impl Recording {
    /// Nonblocking capture/stream failure polling. Errors are retained until drop.
    pub fn error(&self) -> Option<String> {
        #[cfg(any(target_os = "ios", target_os = "linux"))]
        {
            self.worker
                .as_ref()?
                .live
                .state
                .try_lock()
                .ok()?
                .error
                .clone()
        }
        #[cfg(not(any(target_os = "ios", target_os = "linux")))]
        {
            None
        }
    }

    /// Stream mono 16 kHz signed PCM16 in 50ms chunks through a bounded channel.
    /// Call off the UI thread, just like `start`. Overflow stops capture with an
    /// error rather than silently dropping speech. Drop cancels without flushing.
    pub fn start_stream(pcm: tokio::sync::mpsc::Sender<Vec<i16>>) -> Result<Self, String> {
        Self::start_inner(Some(pcm))
    }
    /// Nonblocking demand/poll API. The first call requests a worker-built WAV;
    /// poll until Some, then request the next snapshot at the desired cadence.
    pub fn snapshot(&self) -> Option<Vec<u8>> {
        #[cfg(any(target_os = "ios", target_os = "linux"))]
        {
            self.worker.as_ref()?.live.snapshot()
        }
        #[cfg(not(any(target_os = "ios", target_os = "linux")))]
        {
            None
        }
    }

    /// Latest (at most 24) normalized microphone RMS amplitudes, oldest first.
    pub fn levels(&self) -> Vec<f32> {
        #[cfg(any(target_os = "ios", target_os = "linux"))]
        {
            self.worker
                .as_ref()
                .and_then(|w| w.live.state.try_lock().ok())
                .map(|s| s.levels.iter().copied().collect())
                .unwrap_or_default()
        }
        #[cfg(not(any(target_os = "ios", target_os = "linux")))]
        {
            Vec::new()
        }
    }
    /// Call off the UI thread: permission approval can take up to two minutes.
    pub fn start() -> Result<Self, String> {
        Self::start_inner(None)
    }

    fn start_inner(pcm: Option<tokio::sync::mpsc::Sender<Vec<i16>>>) -> Result<Self, String> {
        #[cfg(any(target_os = "ios", target_os = "linux"))]
        {
            Ok(Self {
                worker: Some(imp::start(pcm)?),
            })
        }
        #[cfg(not(any(target_os = "ios", target_os = "linux")))]
        {
            let _ = pcm;
            Err("Microphone recording is supported only on iOS and Linux, not on this desktop preview platform.".into())
        }
    }

    pub fn stop(mut self) -> Result<Vec<u8>, String> {
        #[cfg(any(target_os = "ios", target_os = "linux"))]
        {
            self.worker
                .take()
                .ok_or("Recording already stopped")?
                .finish(true)
        }
        #[cfg(not(any(target_os = "ios", target_os = "linux")))]
        {
            let _ = &mut self;
            Err("Microphone recording is supported only on iOS and Linux, not on this desktop preview platform.".into())
        }
    }
}

impl Drop for Recording {
    fn drop(&mut self) {
        #[cfg(any(target_os = "ios", target_os = "linux"))]
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
        pub(super) live: Arc<super::Live>,
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

    pub(super) fn start(
        pcm: Option<tokio::sync::mpsc::Sender<Vec<i16>>>,
    ) -> Result<Worker, String> {
        RESERVED
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| {
                "Another microphone recording is starting, recording, or cleaning up".to_owned()
            })?;
        let reservation = Reservation;
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let live = Arc::new(super::Live::default());
        let worker_live = live.clone();
        let (command, commands) = mpsc::channel();
        let notifications_command = command.clone();
        let join = thread::Builder::new()
            .name("voice-capture".into())
            .spawn(move || {
                let _reservation = reservation;
                let mut stream = super::PcmStream::new(pcm);
                let result = objc2::rc::autoreleasepool(|_| {
                    let mut tail = super::WavPcmTail::default();
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
                    let started = std::time::Instant::now();
                    let keep = loop {
                        match commands.recv_timeout(Duration::from_millis(50)) {
                            Ok(keep) => break keep,
                            Err(mpsc::RecvTimeoutError::Disconnected) => break false,
                            Err(mpsc::RecvTimeoutError::Timeout) => {}
                        }
                        if started.elapsed() >= Duration::from_secs(300) { break true; }
                        if let Some(recorder) = capture.recorder.as_ref() {
                            unsafe {
                                let _: () = msg_send![&**recorder, updateMeters];
                                let db: f32 = msg_send![&**recorder, averagePowerForChannel: 0usize];
                                worker_live.level(10.0f32.powf(db / 20.0));
                            }
                        }
                        tail.poll(&capture.path.0, &mut stream)?;
                        if worker_live.requested.swap(false, Ordering::AcqRel) {
                            let snapshot = File::open(&capture.path.0).ok().and_then(|file| {
                                let mut bytes = Vec::new();
                                file.take(MAX_BYTES + 1).read_to_end(&mut bytes).ok()?;
                                super::current_wav(bytes)
                            });
                            worker_live.publish(snapshot);
                        }
                    };
                    capture.stop();
                    if interrupted.load(Ordering::Acquire) {
                        return Err("Recording discarded because the app entered the background or audio was interrupted".into());
                    }
                    if !keep {
                        return Ok(Vec::new());
                    }
                    tail.poll(&capture.path.0, &mut stream)?;
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
                    stream.finish()?;
                    Ok(bytes)
                });
                worker_live.complete(&result);
                result
            })
            .map_err(|e| format!("Cannot start recording worker: {e}"))?;
        match ready_rx.recv() {
            Ok(Ok(())) => Ok(Worker {
                live,
                command,
                join,
            }),
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
                let _: () = msg_send![&**recorder, setMeteringEnabled: Bool::YES];
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

#[cfg(target_os = "linux")]
mod imp {
    use std::{
        io::Read,
        os::fd::AsRawFd,
        process::{Child, Command, Stdio},
        sync::{Arc, atomic::Ordering, mpsc},
        thread,
        time::{Duration, Instant},
    };

    const MAX_DURATION: Duration = Duration::from_secs(300);
    const MAX_PCM: usize = 10 * 1024 * 1024 - 44;

    pub(super) struct Worker {
        pub(super) live: Arc<super::Live>,
        command: mpsc::Sender<bool>,
        join: thread::JoinHandle<Result<Vec<u8>, String>>,
    }
    impl Worker {
        pub(super) fn cancel(self) {
            let _ = self.command.send(false);
            // Detach: killing and reaping the child must never block the UI.
        }
        pub(super) fn finish(self, keep: bool) -> Result<Vec<u8>, String> {
            let _ = self.command.send(keep);
            self.join
                .join()
                .map_err(|_| "Audio recording worker panicked".to_owned())?
        }
    }

    // Every path, including read errors and unwinding, kills and reaps the child.
    struct CaptureChild(Child);
    impl Drop for CaptureChild {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    pub(super) fn start(
        pcm: Option<tokio::sync::mpsc::Sender<Vec<i16>>>,
    ) -> Result<Worker, String> {
        let mut command = Command::new("parec");
        command.args([
            "--record",
            "--raw",
            "--format=s16le",
            "--rate=16000",
            "--channels=1",
        ]);
        start_command_stream(command, MAX_DURATION, MAX_PCM, pcm)
    }

    #[cfg(test)]
    fn start_command(
        process: Command,
        duration: Duration,
        max_pcm: usize,
    ) -> Result<Worker, String> {
        start_command_stream(process, duration, max_pcm, None)
    }

    fn start_command_stream(
        mut process: Command,
        duration: Duration,
        max_pcm: usize,
        sender: Option<tokio::sync::mpsc::Sender<Vec<i16>>>,
    ) -> Result<Worker, String> {
        let live = Arc::new(super::Live::default());
        let worker_live = live.clone();
        let (command, commands) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let join = thread::Builder::new().name("voice-capture".into()).spawn(move || {
            let mut stream = super::PcmStream::new(sender);
            let result = (|| {
            let child = process.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn();
            let mut child = match child {
                Ok(child) => CaptureChild(child),
                Err(error) => {
                    let _ = ready_tx.send(Err(format!("Cannot start parec microphone capture (install pulseaudio-utils and check your audio service): {error}")));
                    return Ok(Vec::new());
                }
            };
            let mut stdout = child.0.stdout.take().ok_or("parec stdout is unavailable")?;
            if ready_tx.send(Ok(())).is_err() { return Ok(Vec::new()); }
            let started = Instant::now();
            let mut pcm = Vec::new();
            let mut meter = super::PcmMeter::default();
            loop {
                match commands.try_recv() {
                    Ok(false) | Err(mpsc::TryRecvError::Disconnected) => return Ok(Vec::new()),
                    Ok(true) => break,
                    Err(mpsc::TryRecvError::Empty) => {}
                }
                if worker_live.requested.swap(false, Ordering::AcqRel) {
                    let complete = pcm.len() / 2 * 2;
                    worker_live.publish(wav(pcm[..complete].to_vec()).ok().and_then(super::current_wav));
                }
                if started.elapsed() >= duration { break; }
                if pcm.len() >= max_pcm { return Err("Recorded audio exceeds the 10 MiB limit".into()); }
                // poll makes silence cancellable without a blocked reader thread. There is
                // only one reader, so a readable pipe can safely be read without O_NONBLOCK.
                #[repr(C)]
                struct PollFd { fd: std::ffi::c_int, events: i16, revents: i16 }
                unsafe extern "C" {
                    fn poll(fds: *mut PollFd, nfds: usize, timeout: std::ffi::c_int) -> std::ffi::c_int;
                }
                let mut fd = PollFd { fd: stdout.as_raw_fd(), events: 1, revents: 0 };
                // SAFETY: fd points to one initialized pollfd for the lifetime of the call.
                let available = unsafe { poll(&mut fd, 1, 20) };
                if available < 0 {
                    let error = std::io::Error::last_os_error();
                    if error.kind() == std::io::ErrorKind::Interrupted { continue; }
                    return Err(format!("Cannot poll microphone audio: {error}"));
                }
                if available == 0 { continue; }
                let mut buffer = [0; 4096];
                let remaining = (max_pcm - pcm.len()).min(buffer.len());
                match stdout.read(&mut buffer[..remaining]) {
                    Ok(0) => {
                        let status = child.0.try_wait().map_err(|e| format!("Cannot inspect parec: {e}"))?;
                        return Err(format!("Microphone capture ended unexpectedly ({status:?}). Check the PulseAudio/PipeWire service and microphone permissions."));
                    }
                    Ok(count) => {
                        pcm.extend_from_slice(&buffer[..count]);
                        meter.push(&buffer[..count], &worker_live);
                        stream.push(&buffer[..count])?;
                    },
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(e) => return Err(format!("Cannot read microphone audio: {e}")),
                }
            }
            let bytes = wav(pcm)?;
            stream.finish()?;
            Ok(bytes)
            })();
            worker_live.complete(&result);
            result
        }).map_err(|e| format!("Cannot start recording worker: {e}"))?;
        match ready_rx.recv() {
            Ok(Ok(())) => Ok(Worker {
                live,
                command,
                join,
            }),
            Ok(Err(error)) => {
                let _ = join.join();
                Err(error)
            }
            Err(_) => {
                let _ = join.join();
                Err("Audio recording worker stopped during startup".to_owned())
            }
        }
    }

    fn wav(pcm: Vec<u8>) -> Result<Vec<u8>, String> {
        if pcm.is_empty() {
            return Err("Microphone produced no audio".into());
        }
        if pcm.len() > MAX_PCM {
            return Err("Recorded audio exceeds the 10 MiB limit".into());
        }
        if pcm.len() % 2 != 0 {
            return Err("Microphone returned an incomplete PCM sample".into());
        }
        let size = pcm.len() as u32;
        let mut bytes = Vec::with_capacity(44 + pcm.len());
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&(36 + size).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&16000u32.to_le_bytes());
        bytes.extend_from_slice(&32000u32.to_le_bytes());
        bytes.extend_from_slice(&2u16.to_le_bytes());
        bytes.extend_from_slice(&16u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&size.to_le_bytes());
        bytes.extend_from_slice(&pcm);
        Ok(bytes)
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn pcm_stream_frames_are_exact_across_odd_reads_and_flush_closes_sender() {
            let (sender, mut receiver) = tokio::sync::mpsc::channel(4);
            let mut stream = super::super::PcmStream::new(Some(sender));
            let samples: Vec<i16> = (0..1703).map(|n| n - 851).collect();
            let pcm: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
            for chunk in pcm.chunks(73) {
                stream.push(chunk).unwrap();
            }
            assert_eq!(receiver.try_recv().unwrap(), samples[..800]);
            assert_eq!(receiver.try_recv().unwrap(), samples[800..1600]);
            assert!(matches!(
                receiver.try_recv(),
                Err(tokio::sync::mpsc::error::TryRecvError::Empty)
            ));
            stream.finish().unwrap();
            assert_eq!(receiver.try_recv().unwrap(), samples[1600..]);
            assert!(matches!(
                receiver.try_recv(),
                Err(tokio::sync::mpsc::error::TryRecvError::Disconnected)
            ));
        }

        #[test]
        fn pcm_stream_overflow_and_disconnection_fail_instead_of_dropping_speech() {
            let (sender, mut receiver) = tokio::sync::mpsc::channel(1);
            let mut stream = super::super::PcmStream::new(Some(sender));
            stream.push(&[0; 1600]).unwrap();
            let started = Instant::now();
            assert!(stream.push(&[1; 1600]).unwrap_err().contains("keep up"));
            assert!(started.elapsed() < Duration::from_secs(1));
            assert_eq!(receiver.try_recv().unwrap(), vec![0; 800]);
            drop(receiver);
            let (sender, receiver) = tokio::sync::mpsc::channel(1);
            drop(receiver);
            let mut stream = super::super::PcmStream::new(Some(sender));
            assert!(stream.push(&[0; 1600]).unwrap_err().contains("closed"));
        }

        #[test]
        fn pcm_stream_cancel_discards_partial_and_rejects_incomplete_final_sample() {
            let (sender, mut receiver) = tokio::sync::mpsc::channel(1);
            let mut stream = super::super::PcmStream::new(Some(sender));
            stream.push(&[0, 64, 3]).unwrap();
            assert!(stream.finish().unwrap_err().contains("incomplete"));
            drop(stream);
            assert!(matches!(
                receiver.try_recv(),
                Err(tokio::sync::mpsc::error::TryRecvError::Disconnected)
            ));
        }

        #[test]
        fn linux_stream_publishes_real_frames_and_final_partial_without_retaining_sender() {
            let (sender, mut receiver) = tokio::sync::mpsc::channel(4);
            let mut process = Command::new("sh");
            process.args(["-c", "i=0; while [ $i -lt 803 ]; do printf '\\000\\100'; i=$((i+1)); done; exec sleep 30"]);
            let worker =
                start_command_stream(process, MAX_DURATION, MAX_PCM, Some(sender)).unwrap();
            let deadline = Instant::now() + Duration::from_secs(2);
            loop {
                if let Ok(samples) = receiver.try_recv() {
                    assert_eq!(samples, vec![16384; 800]);
                    break;
                }
                assert!(Instant::now() < deadline, "stream was not published");
                thread::sleep(Duration::from_millis(5));
            }
            // Let the final 3 samples reach the capture pipe reader.
            thread::sleep(Duration::from_millis(50));
            assert_eq!(
                worker.live.state.lock().unwrap().levels.back().copied(),
                Some(0.5)
            );
            assert_eq!(worker.finish(true).unwrap().len(), 44 + 1606);
            assert_eq!(receiver.try_recv().unwrap(), vec![16384; 3]);
            assert!(matches!(
                receiver.try_recv(),
                Err(tokio::sync::mpsc::error::TryRecvError::Disconnected)
            ));
        }

        #[test]
        fn linux_stream_overflow_stops_capture_and_publishes_error_before_closing() {
            let (sender, mut receiver) = tokio::sync::mpsc::channel(1);
            let mut process = Command::new("sh");
            process.args(["-c", "exec yes"]);
            let worker =
                start_command_stream(process, MAX_DURATION, MAX_PCM, Some(sender)).unwrap();
            let live = worker.live.clone();
            assert!(worker.join.join().unwrap().unwrap_err().contains("keep up"));
            assert!(
                live.state
                    .lock()
                    .unwrap()
                    .error
                    .as_ref()
                    .unwrap()
                    .contains("keep up")
            );
            assert_eq!(receiver.try_recv().unwrap().len(), 800);
            assert!(matches!(
                receiver.try_recv(),
                Err(tokio::sync::mpsc::error::TryRecvError::Disconnected)
            ));
        }

        #[test]
        fn linux_stream_cancel_does_not_flush_captured_partial_audio() {
            let (sender, mut receiver) = tokio::sync::mpsc::channel(1);
            let mut process = Command::new("sh");
            process.args(["-c", "printf '\\000\\100\\000\\300'; exec sleep 30"]);
            let worker =
                start_command_stream(process, MAX_DURATION, MAX_PCM, Some(sender)).unwrap();
            let deadline = Instant::now() + Duration::from_secs(2);
            // A valid snapshot proves the PCM was read before cancellation.
            loop {
                if worker.live.snapshot().is_some() {
                    break;
                }
                assert!(Instant::now() < deadline, "PCM was not captured");
                thread::sleep(Duration::from_millis(5));
            }
            let started = Instant::now();
            assert!(worker.finish(false).unwrap().is_empty());
            assert!(started.elapsed() < Duration::from_secs(2));
            assert!(matches!(
                receiver.try_recv(),
                Err(tokio::sync::mpsc::error::TryRecvError::Disconnected)
            ));
        }

        #[test]
        fn wav_tail_reads_only_new_pcm_and_handles_native_placeholder_headers() {
            use std::io::Write;
            static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            struct Temp(std::path::PathBuf);
            impl Drop for Temp {
                fn drop(&mut self) {
                    let _ = std::fs::remove_file(&self.0);
                }
            }
            let path = Temp(std::env::temp_dir().join(format!(
                "jcode-pcm-tail-{}-{}.wav",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            )));
            let mut file = std::fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&path.0)
                .unwrap();
            let original = wav(vec![0, 64, 0, 192]).unwrap();
            let mut native = original[..36].to_vec();
            native.extend_from_slice(b"JUNK\x02\0\0\0\0\0");
            native.extend_from_slice(&original[36..]);
            native[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
            native[50..54].copy_from_slice(&u32::MAX.to_le_bytes());
            let (sender, mut receiver) = tokio::sync::mpsc::channel(4);
            let mut stream = super::super::PcmStream::new(Some(sender));
            let mut tail = super::super::WavPcmTail::default();
            file.write_all(&native[..20]).unwrap();
            tail.poll(&path.0, &mut stream).unwrap();
            assert!(tail.offset.is_none());
            file.write_all(&native[20..57]).unwrap();
            tail.poll(&path.0, &mut stream).unwrap();
            assert_eq!(tail.offset, Some(57));
            tail.poll(&path.0, &mut stream).unwrap();
            file.write_all(&native[57..]).unwrap();
            tail.poll(&path.0, &mut stream).unwrap();
            stream.finish().unwrap();
            assert_eq!(receiver.try_recv().unwrap(), vec![16384, -16384]);
            assert!(receiver.try_recv().is_err());
        }

        #[test]
        fn live_snapshot_is_valid_and_capture_remains_cancellable() {
            let mut process = Command::new("sh");
            process.args(["-c", "printf '\\000\\100\\000\\300'; exec sleep 30"]);
            let worker = start_command(process, MAX_DURATION, MAX_PCM).unwrap();
            assert!(worker.live.snapshot().is_none());
            let deadline = Instant::now() + Duration::from_secs(2);
            let snapshot = loop {
                if let Some(bytes) = worker.live.snapshot() {
                    break bytes;
                }
                assert!(Instant::now() < deadline, "snapshot was not published");
                thread::sleep(Duration::from_millis(10));
            };
            assert_eq!(&snapshot[44..], &[0, 64, 0, 192]);
            assert_eq!(
                u32::from_le_bytes(snapshot[4..8].try_into().unwrap()) as usize,
                snapshot.len() - 8
            );
            assert!(worker.live.state.lock().unwrap().levels.is_empty());
            assert!(!worker.live.requested.load(Ordering::Acquire));
            assert!(worker.finish(false).unwrap().is_empty());
        }

        #[test]
        fn levels_are_bounded_and_snapshot_poll_does_not_wait_for_lock() {
            let live = super::super::Live::default();
            for _ in 0..100 {
                live.level(2.0);
            }
            live.level(f32::NAN);
            let state = live.state.lock().unwrap();
            assert_eq!(state.levels.len(), 24);
            assert!(state.levels.iter().all(|x| (0.0..=1.0).contains(x)));
            assert!(live.snapshot().is_none());
            assert_eq!(super::super::pcm_level(&[0, 0]), 0.0);
            assert_eq!(super::super::pcm_level(&[0, 128]), 1.0);
        }

        #[test]
        fn pcm_meter_preserves_50ms_windows_and_real_silence_across_odd_reads() {
            let live = super::super::Live::default();
            let mut meter = super::super::PcmMeter::default();
            let mut audio = [0u8, 64].repeat(800);
            audio.extend_from_slice(&[0; 1600]);
            // Deliberately split PCM samples and windows across arbitrary pipe reads.
            for part in audio.chunks(73) {
                meter.push(part, &live);
            }
            assert_eq!(
                live.state
                    .lock()
                    .unwrap()
                    .levels
                    .iter()
                    .copied()
                    .collect::<Vec<_>>(),
                vec![0.5, 0.0]
            );
            assert!(meter.window.is_empty());
            meter.push(&[0; 1599], &live);
            assert_eq!(live.state.lock().unwrap().levels.len(), 2);
            meter.push(&[0], &live);
            assert_eq!(live.state.lock().unwrap().levels.len(), 3);
            for _ in 0..30 {
                meter.push(&[0; 1600], &live);
            }
            let state = live.state.lock().unwrap();
            assert_eq!(state.levels.len(), 24);
            assert!(state.levels.iter().all(|x| *x == 0.0));
        }

        #[test]
        fn native_wav_placeholder_sizes_and_extra_chunks_are_repaired() {
            let original = wav(vec![0, 64, 0, 192]).unwrap();
            let mut native = original[..36].to_vec();
            native.extend_from_slice(b"JUNK\x02\0\0\0\0\0");
            native.extend_from_slice(&original[36..]);
            native[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
            native[50..54].copy_from_slice(&u32::MAX.to_le_bytes());
            native.push(123); // incomplete PCM sample must never reach the network
            let repaired = super::super::current_wav(native).unwrap();
            assert_eq!(repaired.len(), 58);
            assert_eq!(&repaired[50..54], &4u32.to_le_bytes());
            assert_eq!(&repaired[4..8], &50u32.to_le_bytes());
            assert!(super::super::current_wav(vec![0; 44]).is_none());
            assert!(super::super::current_wav(original[..44].to_vec()).is_none());
        }
        #[test]
        fn pcm_has_correct_wav_header() {
            let bytes = wav(vec![1, 2, 3, 4]).unwrap();
            assert_eq!(&bytes[..4], b"RIFF");
            assert_eq!(&bytes[4..8], &40u32.to_le_bytes());
            assert_eq!(&bytes[8..16], b"WAVEfmt ");
            assert_eq!(&bytes[20..24], &[1, 0, 1, 0]);
            assert_eq!(&bytes[24..28], &16000u32.to_le_bytes());
            assert_eq!(&bytes[28..32], &32000u32.to_le_bytes());
            assert_eq!(&bytes[32..36], &[2, 0, 16, 0]);
            assert_eq!(&bytes[36..44], &[100, 97, 116, 97, 4, 0, 0, 0]);
            assert_eq!(&bytes[44..], &[1, 2, 3, 4]);
            assert!(wav(vec![]).is_err());
            assert!(wav(vec![1]).is_err());
            assert!(wav(vec![0; MAX_PCM + 2]).is_err());
        }
        #[test]
        fn silent_child_cancellation_is_prompt_and_reaped() {
            let mut process = Command::new("sleep");
            process.arg("30");
            let worker = start_command(process, MAX_DURATION, MAX_PCM).unwrap();
            let start = Instant::now();
            assert!(worker.finish(false).unwrap().is_empty());
            assert!(start.elapsed() < Duration::from_secs(2));
        }
        #[test]
        fn fake_pcm_is_wrapped_after_duration_limit() {
            let mut process = Command::new("sh");
            process.args(["-c", "printf '\\001\\002\\003\\004'; exec sleep 30"]);
            let worker = start_command(process, Duration::from_millis(100), MAX_PCM).unwrap();
            // Wait for automatic completion, not a user stop, without using a microphone.
            let bytes = worker.join.join().unwrap().unwrap();
            assert_eq!(&bytes[44..], &[1, 2, 3, 4]);
        }
        #[test]
        fn fake_pcm_cannot_exceed_byte_limit() {
            let mut process = Command::new("sh");
            process.args(["-c", "exec yes"]);
            let worker = start_command(process, MAX_DURATION, 1024).unwrap();
            assert!(worker.join.join().unwrap().unwrap_err().contains("10 MiB"));
        }
        #[test]
        fn subprocess_failure_and_missing_executable_are_errors() {
            assert!(
                start_command(
                    Command::new("/nonexistent/jcode-parec"),
                    MAX_DURATION,
                    MAX_PCM
                )
                .is_err()
            );
            let worker = start_command(Command::new("false"), MAX_DURATION, MAX_PCM).unwrap();
            thread::sleep(Duration::from_millis(50));
            assert!(worker.finish(true).unwrap_err().contains("unexpectedly"));
        }
    }
}

#[cfg(all(test, not(any(target_os = "ios", target_os = "linux"))))]
mod tests {
    #[test]
    fn preview_cannot_record() {
        let error = super::Recording::start()
            .err()
            .expect("preview must not capture audio");
        assert!(error.contains("iOS") && error.contains("preview"));
    }
}
