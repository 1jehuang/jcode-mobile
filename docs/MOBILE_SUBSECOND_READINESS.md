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
`src/app/synced_chat.rs`. Desktop source already has an optional account-scoped
WebSocket command-wakeup hint (`mobile_runner_events.rs`), with HTTP claims
remaining authoritative and a five-second fallback. Reuse that mechanism rather
than introduce a second command authority. Its presence in source does not prove
that it is enabled in production. Transcript persistence also notifies the
uploader after a bounded 150 ms coalescing window; the 20-second sync interval is
reconciliation, not a fixed delay for every saved response. A persistent
resumable phone event feed could remove mobile polling delay. It must not change
routing to a cloud computer or create another conversation. The offline channel
prototype studies this path only.

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

Exit 0 means every supplied required milestone is strictly below the budget,
has enough samples, and there are no supplied `session_connect_failed` samples.
Exit 1 means missing/slow/failed samples. Exit 2 means invalid input.
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

The initial `0f92008` candidate was held after review found that a slow
subsequent control-plane request could still block an already successful SSH
probe. **Use the follow-up `7166e86` plus causal tests `e5b6e0a`, not the initial
commit alone.** Both request and probe now run as individually bounded workers;
whichever validated readiness path completes first can win. Cancellation drops
late results without adopting them into another account or session. A
control-plane error is deferred until an active pinned probe succeeds or fails.
The global request permit may temporarily serialize an inspection/retry behind
an already draining HTTP request, but never adopts that other caller's result.

Final hosted acceptance: **13 causal fixtures passed**, receipt
`20261114-225350753-289664`; **152 core tests passed, zero failed, three ignored**,
receipt `20261114-225437398-293650`. The gated tests cover both slow-path winners,
late HTTP errors, cancellation, permit cleanup and polling cadence without
fragile speed assertions. Final controlled scheduling measurements were
**1453.480 ms serial versus 273.823 ms concurrent**, with two requests each;
the full suite repeated at 1453.866 ms versus 274.176 ms. These are still not
real-device end-to-end measurements. The final integrated hosted
`justrust check --workspace --tests` also passed, receipt
`20261114-225538277-298731`. It compiled the current working dependency graph,
not a separately signed release artifact.

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

Normal-path follow-up backend `f4ebab3` supersedes rejected candidate
`25ea39b`: console output is read only after provider ownership verification,
then overlaps independent runtime accounting. Allowance/error precedence and
EIC authorization gates stay intact. Independent real-handler fixture execution
passed **6/6** recovery/normal tests. Under injected provider costs, normal
readiness changes **1190 to 1180 ms**; with deliberately slow 1200 ms accounting,
**2380 to 1480 ms**. These are modeled scheduling costs, not AWS measurements.
The worker's full suite passed 682 tests with one existing skip and retained the
mandatory inventory guard. Nothing was deployed. The rejected pre-ownership
read's 190 ms claim is withdrawn and must not be used as release evidence.

## Follow-up: provisioning contention

A cancelled connection previously used a blocking mutex acquisition behind any
other cold opener. That opener may be waiting for a sleeping host, so the
cancelled waiter could retain its connection worker until provisioning ended.
The acquisition now checks cancellation while contended, without cancelling the
active opener or bypassing provisioning serialization. Two causal fixtures
require cancellation to complete while the owner still holds the lock; their
watchdogs are deadlock protection, not latency benchmarks.

After final authenticated-master publication, the provisioning lock is also
released before independent API hello. A waiter already queued on that lock
can then recheck the shared authenticated master instead of waiting for the
first tab's API process startup. Host discovery/key registration remain
serialized. The remembered-direct fallback branch intentionally retains its
existing serialization because a failed API open there still falls back to
control-plane provisioning. These changes remove contention, not SSH network
round trips or VM boot time, and do not establish a phone speedup.

Contention changes are committed as `58e4d6c`. Combined hosted core acceptance
passed **156 tests, zero failed, four ignored**, receipt
`20261114-231911143-424070`. That also includes the pre-attach push subscription
regressions from `bec8a23` (typed assertion corrected in `08488a4`) for create,
resume and fork. Early attach pushes are no longer lost before subscribing,
which avoids missing fresh content and an unnecessary eight-second runtime
metadata fallback. This is not an eight-second improvement to every connection.

Fresh-history optimization `bf8e4f7` skips three reconciliation indexes when no
live/optimistic tail exists. The controlled index-only benchmark measured
**6.097 ms** of eliminated work for 250 messages of 4096 bytes and **95.971 ms**
for 250 messages of 65536 bytes. The no-tail branch returns empty indexes
without scanning transcript content. Tests retain the indexes when a live tail
needs reconciliation and verify unchanged fresh/repeated snapshot contents.
Benchmark receipt: `20261114-232052572-424068`. These are synthetic CPU savings,
not phone end-to-end timings or a solution to the observed 21.63-second wait.

Integrated hosted app acceptance ran 476 non-ignored tests: **475 passed** and
one coverage-ledger drift check failed, receipt `20261114-232246113-445178`.
Review confirmed the new text-input accessibility ID/SetValue wiring was absent
from the ledger. Only that row was updated with three existing accessibility
regressions and explicit native assistive-technology coverage limits. The
inventory and all three fail-closed guard fixtures then passed (**4/4**), receipt
`20261114-232628185-466563`, with no test binary rebuild. The entire app suite
was not rerun after this documentation-only correction. No guard was disabled.

At this point the remaining large observed costs are cloud readiness and SSH
network/authentication, not transcript hashing. Further device-facing tuning
needs a new installed-artifact stage breakdown. Do not infer that all remaining
costs are unavoidable, promise universal subsecond cold boot, reduce security,
or keep paid hosts awake merely to improve a benchmark.

## Follow-up: active cache and cached revalidation

Mobile `cffa7c3` makes the authenticated-master cache expire after five minutes
of inactivity rather than five minutes after authentication. Matching active
reuse renews the idle deadline, while expired entries cannot revive. Credential
and endpoint checks remain mandatory. A generation token prevents an old timer
from evicting a replacement or restoring signed-out state. Reuse creates no new
timer and no host wake request. A second cache hit after provisioning contention
now releases the provisioning lock before independent API hello, with a locked
replacement recheck before any fallback provisioning.

Mobile `2990f46` validates an existing prepared API channel with a bounded ping
rather than starting another remote API process. The existing two-second stale
socket budget remains. A stalled ping closes only its channel, not active sibling
sessions. Causal fixtures cover stalled, closed and reused channels. Five loopback
revalidations measured **7.21 ms reused versus 610.28 ms fresh**, with an injected
120 ms API hello cost. This is not an iPhone or AWS measurement.

Combined hosted core acceptance passed **162 tests, zero failed, four ignored**,
receipt `20261115-000826075-681885`, including cache-account/expiry fencing and
blocked-hello lock handoff. Integrated app compilation is checked separately.

Direct-auth follow-up `02d4d1e` also releases the provisioning guard before
remembered-direct API hello. A failed hello reacquires the guard and checks for
an authenticated replacement before control-plane fallback. It does not repeat
the direct attempt or provision in parallel. Deterministic channel-barrier
fixtures cover blocked hello, failure/replacement and serialized fallback.
Hosted full core acceptance passed **164 tests, zero failed, four ignored**,
receipt `20261115-001441684-703422`. These are contention guarantees, not a
measurement of SSH network or VM startup time.

Final integrated hosted `check --workspace --tests` passed. Full app acceptance
then passed **485 tests, zero failed, eleven ignored**, receipt
`20261115-001615956-710566`. This run includes the current concurrent platform
work as well as the startup changes. The offline device-budget gate also passed
all nine safety tests. These changes do not establish a new installed-device
end-to-end latency result.

## Real-device release gate

Use an installed release artifact containing the source changes. Capture cold
launch, warm open, long suspension, reconnect, Wi-Fi/cellular transition, large
history, and local-computer routing. Separate awake and stopped-host cohorts.
For each, record foreground-to-usable-session, fresh-history-render, submitted
command-to-local-receipt, and submitted-command-to-terminal-result. Do not send
inference or wake hosts merely to populate a benchmark without authorization.
No real-device improvement or new TestFlight upload is established by this file.
