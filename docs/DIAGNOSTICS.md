# Automatic mobile diagnostics

The mobile app now records privacy-safe diagnostics independently of a cloud
session connection. A failed SSH session does not prevent HTTPS diagnostic
uploads. Existing panic-report uploads continue unchanged.

## Captured data

- Startup milestones and time to first render.
- Direct SSH attempts, handshake/authentication, channel opening and API hello.
- Cloud control-plane wait, cached-master validation, session attachment/fork,
  initial history and metadata requests.
- End-to-end time from opening a session to applying and rendering its history.
- Connection failures, retries, dropped streams and foreground liveness probes.
- Fixed error-kind classifications, connection start/stop/wake, iOS foreground,
  background and memory-warning notifications.
- Request duration by fixed operation type, without request arguments or replies.
- Slow UI frame-build durations, rather than per-frame traffic.
- App version/build, OS/architecture and, on iOS, OS release and iPhone product
  model. No serial number, advertising identifier or permanent device ID.
- Random per-process run ID, per-batch UUID, event sequence number, wall-clock
  timestamp and monotonic process uptime. Retries reuse an immutable batch ID.

Only explicitly allowlisted event names and numeric measurements enter the
uploader. Raw log lines and event detail strings stay on the device. This
excludes prompts, messages, reasoning, tool input/output, repository/file paths,
SSH addresses/keys, tokens and raw server error messages. The authenticated
backend attributes diagnostics to the signed-in account, so this is not
anonymous telemetry.

## Delivery and resource bounds

`diagnostics::start()` starts a background worker during normal application
startup, before UI initialization. Producers use a bounded nonblocking channel.
The worker coalesces events and persists them atomically before sending
`POST /v1/mobile/diagnostics` with the existing account bearer credential.
Credentials are never stored in the diagnostic payload or queue.

- Up to 64 events per batch, 32 queued batches and a 512 KiB queue file.
- Local queue entries expire after seven days. Oldest batches are evicted when
  capacity is reached. This is best-effort diagnostics, not lossless auditing.
- Uploads begin after about ten seconds. Queued batches drain at no more than
  one per second when connectivity returns.
- Network failures, authorization failures, an unavailable rollout endpoint,
  rate limits and server errors retain batches and back off up to five minutes.
- Invalid/oversized batches are discarded rather than retried forever.
- Account changes/sign-out clear the previous account's queue.
- No idle disk writes. Disk and HTTP operations never run on the UI thread.
- Events are normally persisted within one second. Abrupt termination or iOS
  suspension can lose the most recent in-memory events. Delivery resumes on
  subsequent execution, not through an iOS background-service entitlement.

The queue lives next to the existing panic report as `diagnostics-queue.json`.
Demo modes do not enable uploads. Set `JCODE_MOBILE_DIAGNOSTICS_OFF=1` to disable
the worker during local development. Unit tests do not start it automatically.

## Validation and rollout

Core tests cover label privacy, durable replay, bounds/retention, account
isolation, immutable retry IDs and authenticated HTTP status handling. The
backend has its own validation, authentication, deduplication and storage tests.
Physical-device performance and delivery still need validation with a newly
installed iOS build.

The additive backend migration and endpoint were deployed on November 13, 2026
as Worker version `8d2aea83-11c6-4c47-b00b-55fb08aa04bf`, at 100% traffic.
Live smoke checks verified authentication, rejection of forbidden fields,
successful ingestion, immutable retry deduplication and exactly one stored
synthetic batch. Existing bindings and live configuration were preserved.
Backend regression results: 610 passed, one skipped, zero failed.

Existing installed app builds do not upload connection diagnostics. A new mobile
build is still required. Historical phone-local timings cannot be reconstructed
by deploying the endpoint alone. UIKit-specific compilation and physical-device
delivery/performance acceptance remain release checks, not claims made by the
Linux core tests.

The backend source and exact migration/read-only operator query contract are in
`solosystems-backend/workers/api/MOBILE_DIAGNOSTICS.md`. Retrieval is scoped to an
account through the existing read-only operations credential, not a public log
download endpoint. This lets us inspect slow connection stages without access
to a user's phone or conversation.
