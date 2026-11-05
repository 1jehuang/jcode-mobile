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

Headers: `apns-push-type: alert`, `apns-priority: 10`, `apns-topic: <bundle_id>`,
`apns-expiration: now+86400`, `apns-collapse-id: <session_id>:<turn_id>` (only
when turn_id present, max 64 bytes).

## Categories registered by the app

- `JCODE_TURN`: actions `REPLY` (text input, "Reply") and `OPEN` ("Open", foreground).
- `JCODE_INPUT`: actions `REPLY` (text input, "Answer") and `OPEN`.

Tapping opens the app on `session_id`. Reply sends the text to that session.
The app suppresses banners in the foreground only for the session on screen.

## Worker secrets

`APNS_KEY_P8` (PEM contents), `APNS_KEY_ID`, `APNS_TEAM_ID=TAS6ARKDN7`.
Without them `/v1/push/send` returns `503 push_not_configured`.
