# Local performance instrumentation

`src/perf.rs` is a standalone, standard-library-only module. It does not wire itself into the application. The application owner must declare `mod perf;` and add measurements at the desired call sites.

```rust
let _span = crate::perf::span("chat.render");
// Measured synchronous work. Keep the guard alive until this work completes.

crate::perf::record("chat.layout", elapsed);
```

Public API:

- `pub fn span(action: &'static str) -> Span`: returns an RAII guard. Dropping it records elapsed time, including on early return or unwinding.
- `pub fn start() -> Option<std::time::Instant>`: returns a timestamp only when enabled, for measuring across separate callbacks without a disabled-path clock read.
- `pub fn record(action: &'static str, duration: std::time::Duration)`: records an already measured duration.

The application uses `app.build`, `transcript.build`, `transcript.entry_build`, `panel.preview_build`, `panel.scroll_input`, `transcript.scroll_input`, and `transcript.scroll_to_paint`. The last action measures input-to-CPU-paint latency only, not display presentation.

## Enable and retrieve output

Set environment variables **before the first measurement**:

```sh
JCODE_MOBILE_PERF=1 ./target/release/jcode-mobile 2>performance.log
# Optional additional append-only output file, while still writing stderr:
JCODE_MOBILE_PERF=1 JCODE_MOBILE_PERF_LOG=./performance-extra.log ./target/release/jcode-mobile
```

No logger initialization is required. Only the exact opt-in value `1` enables collection. Setting the file variable alone does not enable it. Configuration is cached once for the process lifetime. Disabled calls perform no clock reads, locking, file access, or background-thread startup after the initial configuration check.

When enabled, a background worker emits nonempty aggregate windows approximately every five seconds, even when recording subsequently becomes idle. There is no per-frame logging. If worker startup fails, instrumentation is disabled with a diagnostic. The worker is not joined at shutdown, so the final partial window can be lost. No public flush API is provided. File-open or write failures report a path-free diagnostic on stderr and preserve stderr output. Output failures do not panic.

Each line starts with `jcode_mobile_perf` and contains `action`, actual `window_s`, `count`, `mean_ms`, `p95_ms`, `max_ms`, `over_budget`, `budget_ms`, and `samples`. Statistics reset after each emitted window. `count`, mean, max, and over-budget count cover **all** accepted measurements in that window. `over_budget` counts durations strictly greater than **16.670 ms**, a useful reference budget rather than proof of a dropped frame.

P95 is nearest-rank over the **latest at most 256 samples per action**, not an unbiased population estimate or an exact percentile of the entire window. Memory/cardinality is bounded to the first 64 distinct action labels for the process lifetime. Further unknown labels are ignored. Existing labels continue to work across windows. Labels are truncated to 80 characters and restricted to identifier characters in output to prevent log injection.

## Interpretation, privacy, and overhead

These are **CPU-side elapsed wall-clock spans**, not CPU execution-time accounting, GPU timings, presentation timestamps, FPS, or end-to-end frame latency. Waiting, scheduling delays, and blocking inside a span count toward its duration. A span around render-tree construction does not measure GPU completion. Overlapping or nested spans may double-count work. Avoid holding a guard across unrelated asynchronous work unless that complete elapsed interval is explicitly what you intend to measure.

Use fixed developer-defined action identifiers only. Never put message text, prompts, tool arguments/results, credentials, file paths, user identifiers, or any other private content in labels. The module collects no payloads and sends nothing over the network. Static string labels alone do not guarantee privacy, so call sites must honor this contract. Local stderr redirection and optional files remain under the caller's control. Log files have no rotation or size limit, and use normal process file permissions/umask.

Enabled recording uses a nonblocking attempt to acquire a process-wide mutex and stores bounded samples. Samples are dropped when the mutex is contended or poisoned, so counts describe accepted measurements rather than all attempted measurements. Only the background worker performs periodic formatting and local output. Output holds that mutex, so slow stderr or file sinks can lose samples, but periodic I/O never blocks the recording thread. First-use opt-in initialization does open the optional file and start the worker synchronously. Opt in for focused local investigations, compare equivalent workloads, and disable instrumentation for baseline measurements. The module intentionally does not claim to be a zero-overhead profiler.

## Tests

Unit tests in `src/perf.rs` cover empty aggregation, count/mean/p95/max, strict budget boundaries, and bounded recent samples while retaining full-window statistics. They avoid mutating process environment, so tests can run concurrently. Once the application declares the module, run `justrust test perf::tests`. The standalone module can also be copied into a temporary dependency-free library harness and tested with `justrust test --manifest-path <harness>/Cargo.toml` without compiling the UI application.
