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

The controlled HTTP fixture serves two history pages with 200 ms delay per page. The before path waits for both before reading command status. Measured acknowledgement latency was **407.235 ms serial versus 1.345 ms independent**, with the same artificial 400 ms history delay. The new path can apply the command receipt while history is still gated. Assertions check causal ordering and exact state, not a machine-dependent timing ratio. Follow-up `05a939b` adds production-task acceptance for retained newer image drafts and stale auth/account/tab/poll completions. The final integrated rerun is required before treating that follow-up as fully validated.

### Avoid repeating an already failed speculative SSH attempt

Mobile commit `ca995a8` shares a failed prewarm result only with an overlapping foreground connection for the same account and API base. Previously the foreground opener could wait for speculative authentication to fail and immediately repeat that direct attempt before falling back to the control plane. The result is consumed once, is not a negative cache, and does not suppress a later independent retry or a different account's probe. Host pinning and timeout budgets are unchanged.

An offline serialized-probe fixture uses 80 ms simulated authentication failures: the old ordering took 160.19 ms and two probes, while shared failure took 80.11 ms and one probe. Exact probe counts are asserted, not a timing threshold. This establishes removed duplicate work in that overlap case, not that the uninstrumented 2.01-second gap in the phone timeline had this cause.

### Measure startup stages separately

Mobile commit `33114e4` adds fixed timings for cold-connect lock wait, control-plane HTTP requests, direct probes during wake polling, poll sleep and SSH authentication alone. Previously cumulative authentication timing included handshake, and control-plane readiness combined request/probe/sleep time. Commit `da3c9b0` adds all five fixed labels to the privacy-reviewed diagnostic upload vocabulary. These measurements are needed to determine the remaining long wait without weakening security or arbitrarily shortening deadlines.

Hosted startup validation: cloud tests 11 passed, one live-cloud test intentionally ignored (`20261114-215728297-4044102`), SSH fixture tests 11 passed (`20261114-215109703-3970435`), diagnostics 10 passed (`20261114-215410052-4023917`).

## Acceptance and remaining boundary

All measurements above are offline/loopback or read-only diagnostics. No benchmark created a production session, submitted inference, woke a cloud host or changed production credentials. No production/device before-and-after timeline is available for this code yet. Changes must be compiled into the mobile app before they affect an installed TestFlight build. Do not claim the observed 21.63-second fresh-history milestone has been eliminated.
