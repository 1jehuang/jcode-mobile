# Offline mobile latency report

`scripts/mobile_latency_report.py` reads a local diagnostics queue or a saved
**single-account** read-only operations response and emits only fixed stage
labels, counts and duration statistics. It performs no network calls, changes
no input files and never prints raw input, account/run/batch identifiers,
release metadata, credentials, paths, error detail or conversation content.
Do not combine exports from different accounts. Do not copy raw logs into
issues or paste credentials into a shell command.

```sh
python3 scripts/mobile_latency_report.py /path/to/diagnostics-queue.json
python3 scripts/mobile_latency_report.py /path/to/account-scoped-export.json
python3 scripts/mobile_latency_report.py --log /path/to/gpui.log
python3 scripts/mobile_latency_report_test.py
```

Inputs are bounded to 16 MiB. JSON input must contain `batches`, matching both
the durable local queue and `GET /internal/ops/mobile-diagnostics` response.
The operator endpoint requires the existing read-only operations token and
an explicit account filter. See `docs/DIAGNOSTICS.md` and the backend's
`workers/api/MOBILE_DIAGNOSTICS.md` for that retrieval contract. An ordinary
account credential cannot retrieve backend diagnostics. This tool deliberately
does not discover credentials or implement remote retrieval.

## Interpretation

- `latency` is ranked by descending p95, then max, then fixed label.
- Each row has `count`, `p50_ms`, `p95_ms`, `max_ms`. Percentiles use nearest
  rank, with no interpolation. For a single sample p50, p95 and max coincide.
- `lifetime` is separate. `session_lost_after` measures how long the connection
  task existed before loss, not how long a request waited.
- Some latency labels are cumulative milestones, such as `session_ready` and
  `ui_open_to_transcript_render`. They overlap stage measurements. Do not sum
  rows or interpret the sum as total user wait. `app_first_render` is elapsed
  from process start, not frame-build work alone.
- JSON retry/overlap samples are deduplicated by run and event sequence when
  both are present. Separate runs remain independent. Local plain-text logs
  lack stable event identities and cannot be deduplicated reliably.
- `--log` accepts only `conn_timing` lines with an exact numeric millisecond
  suffix and a fixed instrumentation label. It ignores event details, unrelated
  log lines and `startup_trace` copies to avoid double counting the same log.
- Unknown names, malformed durations, negative/nonfinite durations, booleans
  and durations over seven days are excluded. `ignored_samples` counts these
  excluded timing samples. Unknown names are never echoed. Updating client
  instrumentation requires explicitly reviewing this tool's fixed label set.
- A capped backend export (20 newest batches), queued offline events, sparse
  samples and slow-frame-only instrumentation are not population-wide latency
  distributions. Missing samples do not mean an operation was fast.

## Initial local inspection, November 14, 2026 Pacific

Both diagnostic contracts were read. No operations-token environment variable
or backend `.env`/`.dev.vars` file was found in the inspected locations, so no
backend request was attempted and no account was guessed. The existing local
mobile diagnostics queue was read only through the aggregate report. It
contained **zero timing samples**, yielding empty latency and lifetime lists
and zero ignored samples. No account metadata or raw queue content was emitted.

The existing Android emulator logs contain emulator boot measurements of
39,414 ms and 26,110 ms, not mobile app connection or transcript latency. These
are excluded from the report. No `gpui.log` or saved latency/startup report was
found under the inspected mobile checkout, local mobile data directory or iOS
checkout (bounded search).
The initial local-only pass did not establish a user-facing bottleneck. The
subsequent account-scoped backend read below provides actual duration evidence.
No production writes, deployment or inference requests were performed.

## Actual account-scoped backend inspection

The follow-up read used existing configured Wrangler access to perform only a
D1 `SELECT`. Ownership was resolved internally from the SHA-256 hash of the
API key in the existing protected mobile account record, matching exactly one
active account-scoped `keys` row. The diagnostic query selected payloads only
where their `account_id` matched that key's account, restricted retention to
14 days, ordered newest first and capped the read at 20 batches. No account
identifier, key, key hash, raw payload or cross-account data was printed.
`SUBSCRIPTIONS_OPS_TOKEN` was not substituted for `OPS_INTERNAL_TOKEN`.

Twenty retained batches were retrieved. Their event window was
**November 14, 2026 22:54:44 UTC through November 15, 2026 05:23:32 UTC**.
The aggregate-only report gave these largest measured durations (milliseconds,
rounded to two decimal places):

| Fixed stage | Count | p50 | p95 / max |
| --- | ---: | ---: | ---: |
| session_history | 5 | 1915.52 | 21628.50 |
| session_ready | 5 | 1535.18 | 20507.26 |
| session_connect | 5 | 1293.19 | 20337.42 |
| control_plane_ready | 3 | 6304.64 | 10919.57 |
| ssh_authenticated | 5 | 1002.85 | 5085.71 |
| ssh_handshake | 6 | 500.05 | 4084.36 |
| ui_open_to_history | 6 | 21.38 | 2608.05 |
| direct_ssh_failed | 2 | 1502.46 | 1502.77 |
| api_hello | 8 | 345.28 | 1119.05 |
| initial_history | 5 | 394.20 | 1065.34 |
| ui_open_to_transcript_render | 3 | 30.39 | 647.18 |
| ui_history_apply | 6 | 0.04 | 0.20 |

These small samples have nearest-rank p95 equal to max. Connection open,
ready and history timings share the same per-attempt start and are overlapping
milestones. SSH authenticated includes the handshake from the same SSH start,
not authentication alone. Do not sum these durations or subtract unpaired
percentiles to estimate stage costs.

The largest ranked real wait is **21.63 seconds to session history**. The
largest session-connect sample was **20.34 seconds**. Control-plane readiness
reached **10.92 seconds** and SSH
handshake **4.08 seconds**. These are measured candidates for connection-path
optimization, not evidence that every slow sample had the same cause. UI
history application itself peaked at **0.20 ms**, which does not support
blaming history reduction for these observed multi-second waits. The UI
open-to-history and session-history clocks have different origins and may
include cached history, so they are not interchangeable.

The report excluded five unknown/invalid duration samples without emitting
their raw labels. No lifetime samples were present in this export. A capped
read is not a complete account history. No before/after speedup is claimed by
this reporting-only change. No production writes, deployment, synthetic
uploads or inference requests occurred.

## Paired slow-attempt timeline and later snapshot

A later account-scoped SELECT at approximately 05:53 UTC was necessary because
the earlier query's raw payloads were memory-only. It saved an own-account
export outside Git under private scratch with mode `0600`, for subsequent
**offline** analysis. No additional database reads are needed for that snapshot.
The saved export includes run/batch IDs and must remain protected, even though
its events are structured and contain no conversation content or credentials.
Only aggregate output should be shared.

This later newest-20-batch window spans November 14 22:59:10 UTC to November 15
05:52:53 UTC. All 20 batches report `ios aarch64 26.6.1 iPhone16,2`, build
`1794694215`. Counts are batches, not users or distinct devices.

The slowest session-history sample was paired with chronological events in
the **same run and attempt**, using its event uptime minus its cumulative
duration as the attempt origin. Uptime is integer milliseconds, so relative
times and subtraction have about millisecond precision:

| Event | Attempt-relative end (ms) | Recorded duration (ms) |
| --- | ---: | ---: |
| ui_history_apply | 13.50 | 0.12 |
| ui_open_to_history | 14.50 | 21.38 |
| ui_open_to_transcript_render | 14.50 | 21.96 |
| direct_ssh_failed | 3514.50 | 1502.46 |
| control_plane_ready | 14435.50 | 10919.57 |
| ssh_handshake | 18520.50 | 4084.36 |
| ssh_authenticated | 19522.50 | 5085.71 |
| channel_opened | 19732.50 | 208.84 |
| api_hello | 20335.50 | 602.41 |
| session_connect | 20337.50 | 20337.42 |
| session_attach | 20505.50 | 167.91 |
| session_ready | 20506.50 | 20507.26 |
| initial_models | 20929.50 | 421.90 |
| initial_sessions | 21008.50 | 500.27 |
| initial_history | 21573.50 | 1065.34 |
| session_history | 21628.50 | 21628.50 |

This paired timeline places the failed direct SSH attempt's start near
**2012 ms after the attempt origin**, followed by its 1502 ms failure and the
10,920 ms control-plane wait. It establishes a preceding approximately
**2.01-second gap**, but does not identify the cause. Prewarm-lock waiting is
a hypothesis for code inspection, not an instrumented fact. Handshake and
successful authentication then finish near 18.52 and 19.52 seconds. Cached
history was already applied near 14 ms, so the 21.63-second fresh-history
milestone is not proof that the transcript was blank for that whole interval.

The later snapshot also contains a new **476,186.30 ms**
`ui_open_to_transcript_render` milestone at November 15 05:49:49.227 UTC.
Its inferred start aligns within one millisecond with `app_first_render` in
that run. Before this milestone, a `session_lost_after` lifetime of
**389,798.18 ms** was recorded, followed by another failed direct SSH attempt.
SSH/control-plane and attach-failure events occur after the UI milestone.
Thus this unusually long value spans a long-lived UI/open epoch and must not
be presented as a measured 476-second connection request or verified blank
screen. Missing events in the capped export prevent reconstructing the full
interval. This is an instrumentation interpretation concern to investigate,
not a measured latency improvement. The original table above remains the
**earlier snapshot**, not an assertion about all subsequently arriving events.

Validation: eight offline synthetic tests pass, covering ranking and nearest
rank percentiles, lifetime separation, malformed/unknown/private labels,
nonfinite values, overlapping batches, independent runs, plain-text parsing,
empty inputs and privacy-safe CLI errors.
