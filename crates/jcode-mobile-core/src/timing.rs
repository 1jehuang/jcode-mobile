//! Always-on, low-volume connection timing log.
//!
//! Every stage of opening a session (SSH handshake, auth, channel, API hello,
//! attach, first history) is logged through `log` at info level with a stable
//! `conn_timing` prefix. On iOS the GPUI logger forwards these to NSLog and
//! appends them to `<container>/Documents/gpui.log`, so timings from a real
//! device can be pulled after the fact. Set `JCODE_STARTUP_TRACE` to also get
//! them on stderr (desktop and the latency probe). Never logs addresses,
//! keys, or session content.
use std::time::Duration;

/// Process start, captured as early as `main` calls it.
pub fn process_start() -> std::time::Instant {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    *START.get_or_init(std::time::Instant::now)
}

/// A one-off connection event with no duration (disconnect reason, retry).
pub fn event(name: &str, detail: &str) {
    log::info!("conn_event {name} {detail}");
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if *ON.get_or_init(|| std::env::var_os("JCODE_STARTUP_TRACE").is_some()) {
        eprintln!("startup_trace {name} {detail}");
    }
}

/// Run `f` and log how long it took under `stage`.
pub fn timed<T>(stage_name: &str, f: impl FnOnce() -> T) -> T {
    let start = std::time::Instant::now();
    let out = f();
    stage(stage_name, start.elapsed());
    out
}

pub fn stage(stage: &str, elapsed: Duration) {
    let ms = elapsed.as_secs_f64() * 1000.0;
    log::info!("conn_timing {stage} {ms:.1}ms");
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if *ON.get_or_init(|| std::env::var_os("JCODE_STARTUP_TRACE").is_some()) {
        eprintln!("startup_trace {stage} +{elapsed:?}");
    }
}
