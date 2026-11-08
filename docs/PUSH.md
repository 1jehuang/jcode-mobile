# Push notifications contract (jcode mobile)

Shared by: jcode-mobile (iOS app), jcode server (`~/jcode`), and the
subscription API Worker (`~/solosystems-managed-cloud/workers/api`).

## Flow

1. iOS app gets an APNs device token, then `POST /v1/push/devices` with the
   account API key (the same `jck_live_` key it signs in with).
2. The jcode server (cloud host or any machine signed in to a Jcode account,
   `JCODE_API_KEY` + `JCODE_API_BASE`) calls `POST /v1/push/send` when
   something needs the user. It never talks to Apple.
3. The Worker signs an ES256 JWT with the APNs .p8 key and POSTs to
   `https://api.push.apple.com/3/device/<token>` (or sandbox) for every device
   registered on that account. 410 / BadDeviceToken / Unregistered rows are
   deleted.

## Endpoints (all Bearer `jck_live_...`, JSON)

### POST /v1/push/devices
```json
{"token":"<hex>","environment":"production"|"sandbox","bundle_id":"sh.jcode.mobile.rust","device_name":"iPhone"}
```
Upsert by token (a token moves to the caller's account). `200 {"ok":true}`.
Machine-scoped keys (cloud machines) are rejected.

### DELETE /v1/push/devices
`{"token":"<hex>"}` -> `200 {"ok":true}`. Sign-out calls this.

### POST /v1/push/badge
`{"count":0}` -> resets the account's unread counter (app foregrounded).

### POST /v1/push/send
```json
{
  "kind": "turn_done" | "turn_error" | "needs_input" | "wake",
  "session_id": "session_...",
  "session_title": "Fix login bug",
  "body": "Short preview text",
  "turn_id": 42
}
```
Reply `200 {"sent":N,"devices":M}`. `devices:0` lets the server back off
(cache "no devices" for 10 minutes). Allowed for host keys too (add to the
machine-credential allowlist). Rate limit: 60 sends / account / minute.

## APNs payload built by the Worker

```json
{
  "aps": {
    "alert": {"title": "<session_title or 'jcode'>", "subtitle": "<kind label>", "body": "<body, max 180 chars>"},
    "sound": "default",
    "badge": <account unread count, incremented per send>,
    "thread-id": "<session_id>",
    "category": "JCODE_TURN" | "JCODE_INPUT",
    "interruption-level": "active" | "time-sensitive",
    "relevance-score": 0.0..1.0,
    "target-content-id": "<session_id>"
  },
  "session_id": "...", "kind": "...", "turn_id": 42
}
```

Kind mapping:

| kind        | subtitle          | category    | interruption   | relevance |
|-------------|-------------------|-------------|----------------|-----------|
| turn_done   | "Finished"        | JCODE_TURN  | active         | 0.6       |
| turn_error  | "Stopped with an error" | JCODE_TURN | active      | 0.8       |
| needs_input | "Needs your input"| JCODE_INPUT | time-sensitive | 1.0       |
| wake        | "Update"          | JCODE_TURN  | active         | 0.5       |

Each field has one job: title identifies the session, subtitle conveys the
event, and body supplies the result, failure, or question. The Worker removes
repeated leading session titles, app names, and status labels from the body
before applying its length limit. It does not remove matching words inside
substantive prose. Empty or status-only bodies receive a kind-specific prompt
to review the result, inspect/retry the failure, respond, or see the update.
Blank titles fall back to `jcode`, as do titles identical to the status subtitle.
Text is whitespace-normalized and Unicode-safe, with truncation preserving
complete emoji and combining-character sequences.

For example, `session_title: "Fix login"` and
`body: "Fix login: Finished: Fixed token refresh; 12 tests pass."` produce:

```text
Fix login
Finished
Fixed token refresh; 12 tests pass.
```

Headers: `apns-push-type: alert`, `apns-priority: 10`, `apns-topic: <bundle_id>`,
`apns-expiration: now+86400`, `apns-collapse-id: <session_id>:<turn_id>` (only
when turn_id present, max 64 bytes).

## Categories registered by the app

- `JCODE_TURN`: actions `REPLY` (text input, "Reply") and `OPEN` ("Open", foreground).
- `JCODE_INPUT`: actions `REPLY` (text input, "Answer") and `OPEN`.

Tapping opens the app on `session_id`. Reply sends the text to that session.
The app suppresses banners in the foreground only for the session on screen.

## Local fallback previews

When APNs is not live, the app can post a local banner while its connection
remains alive in the background. The preview uses the latest nonempty assistant
response after the current user message, never a previous turn's result. Queued
follow-ups do not hide the current result. A turn with no text response says
"Finished without a text response" instead of recycling stale text.

Local previews collapse whitespace, strip heading/list/code-emphasis layout,
and include identical lines only once. They are capped at 180 Unicode characters
and prefer whole-word truncation. Identical alerts already waiting to be posted
are coalesced, but an identical result from a later turn remains deliverable.

## Worker secrets

`APNS_KEY_P8` (PEM contents), `APNS_KEY_ID`, `APNS_TEAM_ID=TAS6ARKDN7`.
Without them `/v1/push/send` returns `503 push_not_configured`.

## Release validation

Validated on November 6, 2026:

- Mobile core: 47 library tests passed, including stale-turn prevention,
  queued-follow-up context, duplicate coalescing, and preview formatting.
- API Worker: 13 push tests and 415 total API tests passed.
- Wrangler 4.107.0 `deploy --dry-run` bundled the configured Worker successfully
  without uploading or changing remote resources.
- Eight isolated workerd runtime cases passed with the production
  `2025-01-01` compatibility date and `nodejs_compat` flag. Coverage includes
  all four event kinds, repeated labels/Markdown/lines, preserved caveats and
  negative values, session/turn routing metadata, and complete emoji clusters.

These checks do not establish delivery or appearance on a physical iPhone.
The local device probe could not connect to usbmuxd. The API checkout also
contains a broader managed-cloud branch, so a production rollout must review
the whole release rather than blindly deploy a notification-only change.
No production deployment or live test push was performed.

After an approved API rollout and installation of the updated mobile build,
use an owner's test account and paired iPhone to check:

1. Background/lock the phone and trigger completion, error, input-needed, and
   wake events in a clearly named test session. Check that title, subtitle,
   and body each add distinct information, including long and emoji previews.
2. Check foreground behavior: the visible session suppresses its banner,
   while an event for another session can still present one.
3. Tap a banner and verify the correct session opens. Use inline Reply/Answer
   and verify it reaches that session, not the currently selected tab.
4. Confirm local fallback and APNs do not both announce the same event, and
   that later turns with identical results are still deliverable.
5. Inspect grouped notifications and badge reset behavior after foregrounding.

Do not send test pushes to other users or use production secrets in test logs.

## Many agents (v2)

Users run many agents at once. Two rules follow:

1. One aggregate Live Activity for the whole account, not one per session.
   It shows counts plus the most urgent few sessions, and it is updated by
   APNs (`apns-push-type: liveactivity`) so it stays current with the app
   suspended or killed.
2. Alerts degrade instead of dropping. Bursts become quiet, grouped
   notifications rather than 429s.

### POST /v1/agents/status (jcode server, host or user key)

Full snapshot of one host's non-idle sessions. Idempotent. The server sends
it on any state change (debounced ~2 s) and as a heartbeat every 5 minutes
while any session is non-idle, then one final empty snapshot.

```json
{
  "host": "<stable host id, 1..64 [A-Za-z0-9_.:-]>",
  "sessions": [
    {"session_id": "session_...", "title": "Fix login", "state": "running" | "waiting" | "error", "detail": "bash", "since": 1762590000}
  ]
}
```

Max 200 sessions per snapshot, titles <= 80 chars, detail <= 60 chars.
Swarm workers owned by another session are included (they are agents too).
`state`: `running` (thinking, responding or tools), `waiting` (needs input),
`error` (last turn failed and nobody has replied yet). Idle sessions are omitted.
A host whose snapshot is older than 15 minutes counts as empty.
Reply `200 {"ok":true,"pushed":N}`.

### GET /v1/agents/status (app)

`200 {"state": <ContentState>}`, the same merged state that is pushed.

### POST /v1/push/live-tokens (app)

```json
{"token":"<hex>","kind":"update"|"start","environment":"production"|"sandbox","bundle_id":"sh.jcode.mobile.rust","device_token":"<alert token hex, optional>"}
```
`device_token` identifies the phone (falls back to the API key) so sign-out
via `DELETE /v1/push/devices` can also drop its live tokens.
`update` is the per-activity push token (`Activity.pushTokenUpdates`). The
Worker keeps the newest `update` token per device and drops older ones.
`start` is the iOS 17.2+ push-to-start token (`pushToStartTokenUpdates`).
`DELETE` with `{"token":"..."}` removes one. Sign-out deletes both kinds.

### Merged ContentState (JSON keys exactly as Swift Codable)

```json
{
  "running": 7, "waiting": 1, "errored": 0,
  "items": [{"id": "session_...", "title": "Fix login", "state": "waiting", "detail": ""}],
  "updatedAt": 1762590123
}
```
`items` has at most 3 entries, ordered waiting, then error, then running
(newest `since` first within a state). Times are Unix seconds (Int), never
`Date`, because ActivityKit's push decoder uses the default JSON decoder.
Attributes type: `JcodeAgentsAttributes` with `{"title":"jcode"}`.

### Live Activity pushes (Worker)

- Merge every fresh host snapshot for the account. If the ContentState
  (ignoring `updatedAt`) equals the last one pushed, push nothing.
- Topic `<bundle_id>.push-type.liveactivity`, `apns-push-type: liveactivity`.
- `apns-priority: 10` when `waiting` or `errored` rose, otherwise `5`.
- Payload `aps: {timestamp, event: "update", content-state, stale-date: now+1200}`.
  A rise in `waiting` also includes `alert: {title, body}` so the island expands.
- If nothing is active any more: `event: "end"` with `dismissal-date: now+900`.
- If agents are active, no update token is registered and a `start` token
  exists, send `event: "start"` with `attributes-type: "JcodeAgentsAttributes"`,
  `attributes: {"title":"jcode"}`, the content-state and an alert.
- On 410/BadDeviceToken, delete the token.

### Alert bursts (Worker `/v1/push/send`)

- The first 4 alerts per account per rolling 60 s are normal.
- Further alerts in the window are sent with `interruption-level: passive`,
  no sound and relevance 0.2, so they sit in the session's thread quietly.
  `needs_input` and `turn_error` are never demoted.
- The hard cap is 240 per minute. Beyond that the reply is 429.

### App

- The app runs one activity. When it starts the activity, it requests
  `pushType: .token` and reports each token. It seeds the activity from
  `GET /v1/agents/status` and overlays the live state of its own open tabs.
  It starts the activity when anything is active and ends it when the merged
  state is empty.
- The tap URL is `jcode://session/<id>` for the top item, and the item rows
  link to their own sessions on the Lock Screen.
