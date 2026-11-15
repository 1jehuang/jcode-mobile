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

Validation: eight offline synthetic tests pass, covering ranking and nearest
rank percentiles, lifetime separation, malformed/unknown/private labels,
nonfinite values, overlapping batches, independent runs, plain-text parsing,
empty inputs and privacy-safe CLI errors.
