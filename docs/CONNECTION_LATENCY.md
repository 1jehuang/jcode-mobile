# Connection latency

## Reproduce

Run `justrust run -p jcode-mobile-core --example connection_latency` with a
saved mobile account or the CLI account configuration. This live probe creates
one session and reattaches it for subsequent measurements. It prints elapsed
times, not credentials or target addresses.

The probe compares the original SDK connection, a cold mobile connection,
ten immediate warm connections, ten warm connections after a 500 ms pause,
and ten raw-master connections without a prepared channel. `hello` measures
the API handshake, `attach` measures session attachment, and `ready` includes
both. The pause and raw-master setup are excluded from individual request
timings and reported separately. Host provisioning is reported separately in the
progress timestamps. Subsecond readiness is the warm-connection target, not a
promise that a stopped VM can boot in under a second.

## Measured results

Live Linux-to-Cloud probe on November 5, 2026 (Pacific time), run
`20261105-220418009-3338703`:

| Path | Ready, including session attachment |
| --- | --- |
| Original SDK | 3.646 s |
| Mobile cold, VM already running | 3.089 s |
| Warm minimum | 329 ms |
| Warm median | 431 ms |
| Warm p95 / maximum, ten samples | 1.166 s |

Nine of ten warm connections were below one second. This is a substantial
improvement, not a guaranteed subsecond network SLA. A prior first connection
needed 29.7 s while the cloud host started. Cold VM startup and initial SSH
authentication remain outside the subsecond budget.

### Prepared API channel follow-up

Live Linux-to-Cloud probe on November 5, 2026 (Pacific time), run
`20261105-223117035-3520295`, comparing the previous warm path with the new
prepared channel in the same run:

| Path, ten samples each | Median ready | Maximum ready |
| --- | --- | --- |
| Raw authenticated master, no prepared channel | 506 ms | 1,091 ms |
| Prepared cache, immediate back-to-back reconnects | 247 ms | 713 ms |
| Prepared cache, 500 ms idle before each reconnect | 94 ms | 243 ms |

All twenty optimized warm samples were below one second. The idle pause is
not counted as connection time, but the probe explicitly reports it. The
immediate scenario includes cache misses rather than discarding them.
Prepared-cache API hello took 68–112 microseconds in the spaced scenario:
only the session attachment round trip remained on the foreground path.
The first mobile connection still took 3.533 s, and the original SDK took
3.276 s. This optimization targets reconnects, not VM boot or cold SSH.

A repeat run, `20261105-223253247-3533704`, measured 464 ms median ready
without a prepared channel, 184 ms for immediate reconnects and 136 ms after
the 500 ms pause. All ten spaced samples remained under 445 ms. One immediate
sample took 2.454 s, including 1.865 s waiting for session attachment. Across
both runs, 39 of 40 optimized warm samples were below one second. This is a
measured reduction in handshake cost, not a guarantee against network or
server-side attachment latency. No outliers are removed from the summaries.

## Design

- The first connection registers a fresh ephemeral key with the control plane
  and verifies the returned SSH host-key pins before authenticating.
- Further connections for the same account and API endpoint reuse that
  authenticated SSH master. They do not register another key, open another TCP
  connection, or repeat SSH authentication.
- Each tab gets its own API exec channel and client. Sharing the API client
  itself is unsafe because the bridge attaches one session per API channel.
- After a successful open, one independent, unattached API client is prepared
  in the background. The next caller takes exclusive ownership of it, avoiding
  SSH channel startup, bridge startup and API hello on the foreground path.
  A miss opens normally without waiting for the refill. Only one refill can
  run per cache entry, and no user session or message is created speculatively.
- Prepared clients older than 30 seconds or already known to be closed are
  discarded. Eviction/sign-out drops the spare, while a weak owner prevents
  a late background refill from resurrecting an evicted cache. A refill still
  in progress retains its SSH master for at most the existing two-second
  startup deadline. Active tabs are unaffected by cache eviction.
- Idle master caching expires after five minutes and is cleared on sign-out.
  Expiration does not interrupt active channels. No private SSH key is persisted.
- Failed warm connections fall back to authenticated cloud discovery. Concurrent
  cold connections are serialized to avoid duplicate key registration.
- Initial history, runtime information, model lists and session lists are fetched
  independently while messages and live events continue. Stale initial history
  cannot overwrite a transcript that has advanced.

## Build environment

On November 5, 2026 (Pacific time), hosted validation was attempted but the
billing endpoint returned HTTP 404 for `POST /internal/builds/charge`.
Justrust fell back to local builds. Run `20261105-215718465-3303289` confirms
`where local (remote failed in the last 10 minutes)`. No billing bypass was used.
