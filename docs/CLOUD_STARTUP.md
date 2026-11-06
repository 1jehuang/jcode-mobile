# Cloud startup latency, November 6, 2026

## Why a four-second estimate was not a launch-time guarantee

The mobile connection has separate control-plane wake/key registration, TCP/SSH authentication, remote API process startup/hello, and session create/attach stages. The previous two-second deadline covered channel open, remote exec, and API hello together. If a waking or loaded machine took longer, the app discarded successful SSH authentication, retried cloud setup, and added exponential reconnect backoff. With six retries, the backoff alone can add 61 seconds (1 + 2 + 4 + 8 + 16 + 30), excluding all network and startup work.

An in-memory warm master and preopened API channel help later connections in the same process. They do not survive iOS terminating the app. The five-minute master cache and 30-second spare lifetime are unchanged. Neither a warm benchmark nor a Linux cold connection establishes release-iPhone launch time.

## Fix

- Cache successfully authenticated SSH before opening the API channel. An API hello timeout no longer discards that authentication.
- Preserve a live master on an API-level error so the normal retry can open another independent channel without repeating cloud host/key setup.
- Allow 15 seconds total for foreground remote channel/exec/API hello startup. This is an allowance, not a predicted wait or a guaranteed completion time.
- Separately bound SSH channel establishment to two seconds. A peer that cannot acknowledge a new channel is treated as disconnected and goes back through the authenticated control plane, rather than consuming the longer process-start budget on every retry.
- Keep speculative background spare creation bounded to two seconds. An abandoned refill must not retain an evicted master for 15 seconds.
- Distinguish checking the cloud machine, connecting SSH, and opening the session in visible progress messages.

Host-key pinning, ephemeral key registration, credential/endpoint scoping, separate session channels, request timeouts, and sign-out cache clearing remain intact.

## Measurements

Opt-in live-account receipt `20261106-033127969-1685970`, Linux debug core, one cold and one warm pass against the installed real cloud binary:

| Cumulative milestone | Cold | Warm |
| --- | ---: | ---: |
| Control plane returned host | 1332 ms | reused |
| SSH authenticated | 2481 ms | reused |
| API hello complete | 2830 ms | <1 ms |
| Ping complete | 2917 ms | 300 ms |
| List one session complete | 3019 ms | 387 ms |
| Attach existing session complete | 3253 ms | 486 ms |

The benchmark performs ping and list before attach to expose functioning API milestones. Thus the attach totals include those extra calls, unlike the normal direct-attach path. It does not create conversations or submit model requests. This is a single diagnostic sample, not a before/after distribution, device benchmark, force-quit relaunch guarantee, or measured speedup. The host was reachable, not necessarily booting from a stopped state.

Run locally because saved account credentials must remain on the device executing the test:

```sh
justrust test --local -p jcode-mobile-core benchmark_real_cloud_startup -- --ignored --nocapture --test-threads=1
```

The ignored test emits stage timings but no credentials or conversation content. Do not run it concurrently with other tests that alter the process-global cloud cache.

## Regression coverage and remaining acceptance

SSH fixtures cover a hello delayed beyond the former two-second deadline with exactly one authentication, a short timed-out API probe followed by a successful channel on the same master, an authenticated but unresponsive peer, independent session channels, spare reuse/expiry, and eviction without interrupting active clients. Full core receipt `20261106-033101912-1678968` passed 59 tests before the final stale-peer regression was added. Final focused receipt `20261106-033425130-1727736` passed all 21 cloud-related tests, including the stale-peer and slow-hello regressions. Core check receipt `20261106-033424535-1727558` also passed. Hosted builds had recently failed, so justrust's automatic local fallback was used for these receipts.

Still required before claiming an iPhone startup SLA: capture a release build's real launch-to-attached timing on the phone, including force-quit launch, long suspension, network switching, poor cellular service, stopped cloud host, and large session history. These changes fix identified retry amplification. They do not prove the cause of every long wait reported on a previously shipped build.
