# Mobile transcript request latency

The synced-session browser makes account-scoped HTTP requests independently of
SSH connection startup and transcript rendering. Opening its sidebar fetches the
session list and command list. Command polling also uses this transport.

## Connection reuse measurement, November 8, 2026

The previous implementation constructed a fresh `ureq::Agent` for every request,
discarding its connection pool. A real loopback fixture running the core's `list`
function measured three batches of eight requests before and after retaining a
shared agent:

| Controlled fixture | Connections per eight requests | Median batch elapsed |
| --- | ---: | ---: |
| Fresh agent per request | 8 | 315.039 ms |
| Shared transport agent | 1 | 97.462 ms |

The fixture injects **30 ms per accepted connection and 8 ms per response body**.
These are artificial delays, not measurements of the production API, APNs,
iPhone startup, rendering, or cellular-network performance. Reproduce with:

```sh
justrust test --remote -p jcode-mobile-core --lib benchmark_mobile_sync_http_pool -- --ignored --nocapture
```

Hosted receipts: baseline `20261108-004918773-1023948`, after
`20261108-005241843-1044478`. The fixture has no account credentials or external
network access. It checks actual socket counts instead of asserting timings.

## Correctness and timing boundaries

The shared agent contains no account credentials or transcript results. Each
request still sets its own Authorization header, and sockets are pooled by
origin. Regression tests switch account credentials on one connection and
alternate distinct origins to check routing and authorization isolation.
The 30-second request budget, user agent, server-error behavior, command-ID
validation, local-only transfer gate, and explicit user transfer action remain.

Content-free `[CLOUD_TIMING] stage=mobile_sync_request` records report `list`,
`commands`, `command`, or `transfer` operations and success/error duration. They
cover the HTTP request and JSON body read, not UI scheduling, typed response
reduction or paint. They never include account keys, URLs, IDs or bodies. Raw
core stderr records have no wall-clock prefix, so use the shared Jcode latency
report without a time filter unless a capture system adds compatible timestamps.

Hosted validation: all **85 core tests passed**, three opt-in tests ignored,
receipt `20261108-005418434-1054197`. Strict Clippy currently reports unrelated
existing findings in `voice.rs`, `session.rs`, and `todos.rs`, not in these files.

This change is source-level until a new mobile app is built and installed. A
Linux core fixture cannot establish release-iPhone latency. Device acceptance
must include cold launch, resumed app, account changes, poor networks, and real
sidebar refresh/command polling. Separate cloud wake and SSH startup work is
recorded in `CLOUD_STARTUP.md`.
