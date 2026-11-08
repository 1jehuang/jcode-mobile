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

pub fn stage(stage: &str, elapsed: Duration) {
    let ms = elapsed.as_secs_f64() * 1000.0;
    log::info!("conn_timing {stage} {ms:.1}ms");
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if *ON.get_or_init(|| std::env::var_os("JCODE_STARTUP_TRACE").is_some()) {
        eprintln!("startup_trace {stage} +{elapsed:?}");
    }
}
