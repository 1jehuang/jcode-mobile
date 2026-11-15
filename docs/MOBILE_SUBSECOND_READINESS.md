# Genuine mobile readiness and session channel experiment

## Target and non-goals

Target: an awake, reachable host should become usable in substantially less than
one second. Measure both `session_ready` (authenticated API plus session attach)
and `session_history` (fresh transcript delivered), independently. A cached
transcript, accepted queue entry, or shortened timeout is not proof of this goal.
A command's accepted acknowledgement is not its completed execution.

A host that must boot, a disconnected phone, and model inference have separate
physical/external dependencies. Do not promise a universal sub-second bound or
keep machines awake at additional cost without an explicit product decision.

## Existing architecture matters

Cloud chat already uses a persistent authenticated SSH master, independent API
channels, and pushed events. `cloud.rs` caches the master and `cloud_ssh.rs`
prepares an API channel. Replacing JSON framing cannot eliminate the cloud wake,
SSH handshake, or attach work that precedes that connection.

Synced local-computer chat follows a different path: authenticated HTTP command
submission plus independent two-second history and command-status poll loops in
`src/app/synced_chat.rs`. A persistent resumable event feed could remove that
polling delay. It must not change routing to a cloud computer or create another
conversation. The offline channel prototype studies this path only.

## Baseline acceptance, November 15, 2026

The protected, previously retrieved iPhone diagnostic export was checked without
network access. Five retained `session_ready` samples and five `session_history`
samples were available. Each milestone had three samples at or above one second.
Maximums were 20,507.26 ms and 21,628.50 ms respectively. **The baseline fails the
sub-second goal.** This export predates the new optimizations and cannot prove
their effect. Retention and sample selection do not establish a population SLA.

Reproduce the fixed-label gate on a fresh account-scoped diagnostics export:

```sh
python3 -B scripts/mobile_readiness_budget.py /protected/device-export.json \
  --budget-ms 1000 --minimum-samples 20
```

Exit 0 means every supplied required milestone is strictly below the budget and
has enough samples. Exit 1 means missing/slow samples. Exit 2 means invalid input.
The gate does not sum overlapping cumulative timings, accept cached UI markers,
or print identifiers or raw input. It checks these connection-attempt samples,
not launch-to-paint, command completion, or all network conditions.

## Candidate persistent-session contract

Build a versioned application envelope over an existing authenticated transport,
not custom encryption or a new networking stack. The experiment uses loopback
only and must never be exposed as an unauthenticated production endpoint.

A production candidate needs all of the following before routing real traffic:

- Authenticate account ownership on initial attach and every reconnect. Bind the
  stream and every command to the same authorized host and session.
- Resume by session, transcript epoch, and monotonically increasing sequence.
  A sequence gap, rewritten transcript, or expired replay window requires an
  explicit snapshot. Never concatenate an old prefix with an incompatible tail.
- Command idempotency keys retain their original payload and terminal result.
  Repeated IDs with different payloads fail. An uncertain delivery must not
  automatically issue a fresh command ID.
- Distinguish accepted, running, completed and failed. Only the actual local
  runner can prove completion. A transport acknowledgement cannot stand in for it.
- Bound frames, replay memory, outbound queue bytes, command retention and
  per-account connections. Never evict an idempotency record and silently
  re-execute an old command. Durable server receipts are needed across restart.
- Keep control acknowledgements from waiting behind bulk history. Logical
  channels alone do not prevent a large already-written TCP frame from blocking
  them. Chunk bulk traffic with bounded writes and scheduling, or use independent
  streams/connections where the transport supports it.
- Respect iOS suspension: reconnect on foreground, resume safely, never assume a
  background socket remains alive. Sign-out invalidates all completions and
  connection state. Keep existing HTTP fallback for older deployments.

## Startup scheduling improvement

Mobile commit `0f92008` moves the existing bounded, pinned SSH readiness probe
off the serial control-plane poll path. At most one speculative probe or unread
probe result exists process-wide. Cancellation abandons its result without
remembering the target or caching its connection. The 1200 ms SSH probe budget,
15-minute readiness allowance, key validation and account-scoped target rules
are unchanged. Control-plane requests never overlap one another.

The production-loop fixture used a server ready after 100 ms, one failed 1200 ms
probe and the configured 250 ms polling interval. Serial baseline measured
1450.287 ms; concurrent scheduling measured 250.148 ms, with exactly two requests
in each. A full-suite repeat measured 1450.302 ms and 250.212 ms. This removes a
specific avoidable wait, not the entire actual-device startup path.

During a prolonged boot, restoring the configured 250 ms cadence can issue more
metadata/console reads and runtime-observation writes than the accidentally
serialized 1450 ms cadence. No request overlap, extra speculative probe, or
ready-path EIC repetition is introduced, but the backend load tradeoff remains.
The four-request-per-second ceiling assumes an immediate response; real request
time lowers that rate. Do not claim identical request counts for every workload.

Hosted check passed (`20261114-223823953-149182`), six focused fixtures passed
(`20261114-224033021-166966`), and the full core library passed **145 tests, zero
failed, three ignored** (`20261114-224110144-173422`). These checks used hosted
SSH builds, not local compilation or production network access.

## Offline persistent channel result

Independent run:

```sh
python3 -B -W error scripts/session_channel_benchmark_test.py
python3 -B scripts/session_channel_benchmark.py --events 3 --interval-ms 200 --poll-ms 2000 --delay-ms 30
```

All 19 protocol tests passed. With three events becoming ready 200 ms apart,
a two-second poll interval and artificial 30 ms one-way message delay, polling
mean/max observation delay was **1662.638/1862.396 ms**, versus stream
**30.934/31.067 ms**. Polling used two requests and 389 application bytes, versus
one request and 333 bytes for the stream.

These are actual monotonic timings of an artificial socket-pair experiment, not
phone/network results. Both modes reuse their socket. Connection setup, TLS,
HTTP overhead, bandwidth/congestion and authentication are excluded. The result
demonstrates removal of polling delay only. Replay and deduplication are in-memory
and do not survive a process crash. This does **not** demonstrate that the real
21.6-second startup became 31 ms. No production endpoint uses this protocol.

## Backend recovery-only improvement

Backend source commit `60df7c6` removes a console-output read whose result was
unused for explicitly requested recovery with existing host-key pins. Its real
handler fixture preserves authentication, ownership, allowance, pins and EIC
authorization. Injected cost accounting changes 1215 ms to 315 ms by deleting
exactly one modeled 900 ms provider operation. Four dedicated fixture tests
passed independently. These are modeled costs, not elapsed AWS measurements.
Normal mobile startup does not request recovery, so this is **not** a measured
improvement to its 10.92-second readiness wait. Source changes are not deployed,
and the backend's divergent-source inventory guard remains mandatory.

## Real-device release gate

Use an installed release artifact containing the source changes. Capture cold
launch, warm open, long suspension, reconnect, Wi-Fi/cellular transition, large
history, and local-computer routing. Separate awake and stopped-host cohorts.
For each, record foreground-to-usable-session, fresh-history-render, submitted
command-to-local-receipt, and submitted-command-to-terminal-result. Do not send
inference or wake hosts merely to populate a benchmark without authorization.
No real-device improvement or new TestFlight upload is established by this file.
