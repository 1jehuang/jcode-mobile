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

## Local inspection, November 14, 2026 Pacific

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
checkout (bounded search). Consequently there is no measured user-facing
bottleneck or before/after optimization result to claim from these inputs.
A new single-account structured export or existing device timing log is needed
for a real latency ranking. No production writes, deployment or inference
requests were performed.

Validation: eight offline synthetic tests pass, covering ranking and nearest
rank percentiles, lifetime separation, malformed/unknown/private labels,
nonfinite values, overlapping batches, independent runs, plain-text parsing,
empty inputs and privacy-safe CLI errors.
