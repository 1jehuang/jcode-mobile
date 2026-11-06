# Local performance instrumentation

`src/perf.rs` provides standard-library-only local instrumentation, wired into the application render and scrolling paths. Startup initializes the optional sink before input arrives. Add measurements for additional actions using:

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

The moving-scroll benchmark alternates old per-message spacers and coalesced
hidden-row runs in the same process, after warming a 500-message transcript:

```sh
justrust test benchmark_scrolling_spacer_coalescing -- --ignored --nocapture --test-threads=1
```

Measured November 6, 2026, run `20261106-031501346-1447924`, 60 frames per pass:

| Pass | Mean draw (ms) | P95 (ms) | Max (ms) |
| --- | ---: | ---: | ---: |
| Per-row spacers | 62.312 | 102.447 | 119.845 |
| Coalesced runs | 29.531 | 47.747 | 80.174 |
| Coalesced runs | 41.185 | 66.087 | 79.519 |
| Per-row spacers | 56.426 | 133.642 | 194.435 |

The combined mean dropped from 59.369 ms to 35.358 ms, about 40% less
draw-path elapsed time. These are debug/test-renderer measurements on a heavily
contended Linux machine, not iPhone frame times or an FPS claim. Hosted builds
were unavailable because the charge API returned HTTP 404, so justrust fell back
locally. Re-run under comparable conditions before extrapolating to devices.
Geometry regressions compare every materialized row and the full scroll range
against unwindowed rendering, including edits, resize and large jumps.

For subsequent optimization, reproduce the same action and transcript, inspect
`transcript.scroll_to_paint` first, then compare `app.build`, `transcript.build`
and `panel.preview_build` to locate expensive construction. A fast handler with
slow input-to-paint is not a fast interaction. Rank by p95 and over-budget count,
and confirm changes with the moving-scroll benchmark, not idle redraws alone.

Unit tests in `src/perf.rs` cover empty aggregation, count/mean/p95/max, strict budget boundaries, and bounded recent samples while retaining full-window statistics. They avoid mutating process environment, so tests can run concurrently. Run `justrust test perf::tests`. The module can also be copied into a temporary dependency-free library harness and tested with `justrust test --manifest-path <harness>/Cargo.toml` without compiling the UI application.
