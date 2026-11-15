# Mobile latency optimization, November 15, 2026 UTC

## Priorities from actual logs

The protected own-account diagnostics export contained 20 retained batches from iPhone16,2, iOS 26.6.1, build 1794694215. The slowest paired fresh-history milestone was 21.63 seconds, including 20.34 seconds to connect. Within that same attempt the control-plane stage took 10.92 seconds and SSH handshake 4.08 seconds. Cached transcript content appeared near 14 milliseconds. UI history reduction itself peaked at 0.20 milliseconds in the initial export.

See `MOBILE_LATENCY_REPORT.md` for sample counts, chronology, retention limitations and privacy boundaries. These are sparse overlapping milestones, not additive stage percentiles or a claim that the screen remained blank. A later 476-second render milestone spans a long-lived open epoch and is not a verified 476-second connection request.

## Changes

### Reuse control-plane HTTP transports

Shared SDK commit `110919586` retains up to eight ureq transport agents by client name and timeout. Previously every SDK request discarded its connection pool, including consecutive host-readiness polls. Authorization stays per request, origins remain independently pooled, and timeouts, TLS validation and SSH pinning are unchanged.

Controlled loopback benchmark, eight real SDK requests with 100 ms artificial setup delay per accepted TCP connection:

| Path | Connections | Elapsed |
| --- | ---: | ---: |
| Fresh transport |8|809.943ms|
| Reused transport |1|102.934ms|
| Reused transport repeat |1|102.746ms|
| Fresh transport repeat |8|810.253ms|

This demonstrates removed repeated connection setup, not measured iPhone/WAN/cloud-boot improvement. Full SDK library validation: 120 passed, 0 failed, 4 ignored. Exact receipts and security regressions are recorded in the shared repository at `docs/benchmarks/mobile-control-plane-pooling-2026-11-15.md`.

### Do not queue command acknowledgements behind history

Mobile commit `1afc920` separates command-status polling from full paginated transcript refresh. A slow or blocked transcript request no longer prevents an independent terminal command receipt from clearing the existing pending-send guard. History remains a full snapshot to retain rewrite semantics. Authentication/account/tab/poll generations and the expected command ID fence late completions. Polling performs GETs only, never a repeated message POST.

The controlled HTTP fixture serves two history pages with 200 ms delay per page. The before path waits for both before reading command status. Measured acknowledgement latency was **407.235 ms serial versus 1.345 ms independent**, with the same artificial 400 ms history delay. The new path can apply the command receipt while history is still gated. Assertions check causal ordering and exact state, not a machine-dependent timing ratio. Follow-up `05a939b` adds production-task acceptance for retained newer image drafts and stale auth/account/tab/poll completions. These tests passed in the full integrated app run below.

### Avoid repeating an already failed speculative SSH attempt

Mobile commit `ca995a8` shares a failed prewarm result only with an overlapping foreground connection for the same account and API base. Previously the foreground opener could wait for speculative authentication to fail and immediately repeat that direct attempt before falling back to the control plane. The result is consumed once, is not a negative cache, and does not suppress a later independent retry or a different account's probe. Host pinning and timeout budgets are unchanged.

An offline serialized-probe fixture uses 80 ms simulated authentication failures: the old ordering took 160.19 ms and two probes, while shared failure took 80.11 ms and one probe. Exact probe counts are asserted, not a timing threshold. This establishes removed duplicate work in that overlap case, not that the uninstrumented 2.01-second gap in the phone timeline had this cause.

### Measure startup stages separately

Mobile commit `33114e4` adds fixed timings for cold-connect lock wait, control-plane HTTP requests, direct probes during wake polling, poll sleep and SSH authentication alone. Previously cumulative authentication timing included handshake, and control-plane readiness combined request/probe/sleep time. Commit `da3c9b0` adds all five fixed labels to the privacy-reviewed diagnostic upload vocabulary. These measurements are needed to determine the remaining long wait without weakening security or arbitrarily shortening deadlines.

Hosted startup validation: cloud tests 11 passed, one live-cloud test intentionally ignored (`20261114-215728297-4044102`), SSH fixture tests 11 passed (`20261114-215109703-3970435`), diagnostics 10 passed (`20261114-215410052-4023917`).

### Retire completed or abandoned UI timing epochs

Commit `d4c011a` clears the initial-open marker when empty history completes, a tab is parked/closed, the screen is hidden, or the connection retries/fails/disconnects. History-application timings are emitted only for active, auth-owned callbacks. A user composing the first message minutes after opening an empty chat must not produce a minutes-long connection latency. Six Rust regressions cover empty history (including the real event pump), one-shot consumption, hidden screens, retry/failure and tab parking. Nine offline report/privacy tests pass, including the five new precise startup labels. This is measurement correctness, not a measured latency reduction. All six Rust tests passed in the integrated app suite.

## Acceptance and remaining boundary

All measurements above are offline/loopback or read-only diagnostics. No benchmark created a production session, submitted inference, woke a cloud host or changed production credentials. No production/device before-and-after timeline is available for this code yet. Changes must be compiled into the mobile app before they affect an installed TestFlight build. Do not claim the observed 21.63-second fresh-history milestone has been eliminated.

Final workspace acceptance initially encountered an unrelated concurrent media-test lifetime error, then confirmed stale hosted inputs: the compiler used old binary modules, a different GPUI dependency and a voice source lacking a method present in the working checkout. Disabling in-flight build joining did not fix the stale mirror. Source-mirror repair is owned by the concurrent justrust maintainer lane.

At 06:15 UTC, the supported hosted SSH transport with fresh source reconciliation recovered current-source library acceptance: `JUSTRUST_HOSTED_NATIVE=off JUSTRUST_REMOTE_RESUME=0 JUSTRUST_BUILDD=0 justrust check -p jcode-mobile --lib`, receipt `20261114-221405437-4190849`. It performed a fresh source sync and checked the actual working GPUI/library graph. This is a per-command hosted transport selection, not a local battery-policy override, billing/tenant bypass, global configuration change or claim that the underlying native-mirror race is fixed.

Final integrated acceptance completed at 06:20 UTC with the same per-command hosted transport selection:

- `justrust check --workspace --tests`: passed, receipt `20261114-221556178-6128`.
- `justrust test -p jcode-mobile-core --lib -- --test-threads=1`: **139 passed, 0 failed, 3 ignored**, receipt `20261114-221612970-8850`.
- `justrust test -p jcode-mobile --lib -- --test-threads=1`: **467 passed, 0 failed, 11 ignored**, receipt `20261114-221715263-6127`. Includes actual gated production poll tasks, stale completion fences, photo ownership and all six UI-open epoch regressions.
- Offline report/privacy tests: **9 passed**.

The working checkout also includes concurrent generated-media and lifecycle work and local GPUI path overrides. This validates that integrated working graph, not a separately signed/pinned TestFlight artifact. No new TestFlight upload was performed for this latency task.
