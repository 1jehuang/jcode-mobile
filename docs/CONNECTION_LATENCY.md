# Connection latency

## Reproduce

Run `justrust run -p jcode-mobile-core --example connection_latency` with a
saved mobile account or the CLI account configuration. This live probe creates
one session and reattaches it for subsequent measurements. It prints elapsed
times, not credentials or target addresses.

The probe compares the original SDK connection, a cold mobile connection,
and ten warm mobile connections. `hello` measures the API handshake and `ready`
includes session attachment. Host provisioning is reported separately in the
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

## Design

- The first connection registers a fresh ephemeral key with the control plane
  and verifies the returned SSH host-key pins before authenticating.
- Further connections for the same account and API endpoint reuse that
  authenticated SSH master. They do not register another key, open another TCP
  connection, or repeat SSH authentication.
- Each tab gets its own API exec channel and client. Sharing the API client
  itself is unsafe because the bridge attaches one session per API channel.
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
