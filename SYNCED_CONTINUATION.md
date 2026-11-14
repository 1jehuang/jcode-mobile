# Synced continuation on mobile

## Two different destinations

A session under **On your computer** with `live: true` opens as a normal chat tab, still running on the computer. The phone does not SSH-attach that ID to the cloud machine and does not transfer the transcript. Live rows show **Live on computer** or **Working on computer**. A notification resolved to a live local session uses the same path.

Local sessions without fresh presence remain read-only in the synced browser. **Continue in cloud** is a separate, explicit transfer operation that creates a different cloud session. Older servers that omit `live` and `busy` are treated as not live. Existing cloud tabs retain their normal connection behavior.

## Mobile workflow

1. Sign in to the same account used by the computer and open the session drawer.
2. Tap a live computer session. Mobile opens or focuses its chat tab without creating a `Connection`, leaving other tabs and their drafts intact.
3. The tab fetches all transcript pages through account-authenticated `GET /v1/mobile/sessions/:id?after_seq=...`. Sequence numbers identify rows. Tool results appear as labeled system text.
4. Type in the normal composer and send. Mobile posts exactly one `POST /v1/mobile/commands` body with `kind: "message"`, the original `session_id`, and trimmed `text`. It never substitutes `transfer` or a cloud creation command.
5. The computer runner picks up the command. Mobile polls its command status and the synced transcript every two seconds after the previous poll completes. `queued` and `running` keep that tab's next send disabled. `done` releases it without navigation or requiring a new session ID. Terminal `failed` releases it and leaves an error visible.
6. Transcript snapshots replace existing rows, including rewritten assistant text at an existing sequence number. Other tabs continue to receive updates while parked.

`busy` is the computer's activity state, not a cloud connection state. A live busy session can accept a follow-up command. The computer decides when to execute it. Mobile does not offer a fake stop control or model/directory controls for this transport. Its composer identifies the destination as **On your computer**.

Notification replies become a draft, not an automatic command. They never replace an existing local draft. Voice auto-send also falls back to a draft for this transport. Sending is an explicit composer action.

## Safety and lifecycle

- Every API request uses the account's bearer credential and API base. No cloud host wake/attach request is needed for local chat.
- Poll and submission callbacks check the account identity, authentication generation, tab existence and local transport registration before applying results. Closing the tab or signing out prevents late callbacks from changing another session.
- Offline mode makes no polling or submission requests and preserves the draft. A failed transcript refresh leaves the previous transcript visible and disables sending until a successful refresh.
- Missing or expired presence disables sending. A later successful refresh with live presence restores it. Reconnect actions never fall back to cloud attach for a local tab.
- There is no automatic POST retry. A lost submission response may have reached the computer. Mobile preserves the draft and reports uncertain delivery, keeping sends disabled in that tab. Check the computer, then use **I checked the computer. Allow another message** if another submission is needed. This acknowledgement never sends anything by itself. This avoids silently duplicating work when the server has no client idempotency key.
- A known command ID continues to be polled through transient status-request failures. A mismatched session/kind or unknown command status is not interpreted as success.
- An accepted submission only clears the matching submitted draft. Text typed while the request is in flight, and unrelated tabs' drafts, stay intact.

## Implementation and tradeoffs

- `crates/jcode-mobile-core/src/mobile_sync.rs`: presence defaults, message enqueue, strict local command completion, paginated transcript reads.
- `src/app/synced_chat.rs`: per-tab local transport state, composer submission, guarded polling and transcript projection into `SessionState`.
- `src/app/synced_sessions.rs`: live row selection and resolved external target routing.
- `src/app.rs`: normal tab lifecycle/composer integration, local-only destination UI and cloud reconnect guard.

Polling currently re-reads complete paginated snapshots. This deliberately handles desktop transcript rewrites and compaction rather than assuming that a sequence number is immutable. It is eventual transcript synchronization, not token-level streaming. A future revisioned/delta protocol could reduce bandwidth without missing same-sequence updates. Polling pauses in app offline mode, but parked tabs otherwise keep polling until closed or authentication changes. Server-side presence expiry and command timeout remain authoritative.

## Validation

```sh
justrust check -p jcode-mobile
justrust test -p jcode-mobile-core mobile_sync
justrust test -p jcode-mobile synced_chat
justrust test -p jcode-mobile synced_sessions
```

Core HTTP tests run a local mock API and verify message enqueue, account headers, paginated transcript reads, queued/running/done status, no cloud/transfer endpoint, stale presence rejection, absent presence defaults, and session/kind validation. GPUI tests cover opening/reusing a local tab without a connection, preserving per-tab drafts, parked transcript refresh and rewrites, busy/offline presence transitions, command failure/uncertainty, authentication and closed-tab fences, and hiding cloud-only composer controls.

Manual cross-device acceptance: run and sync a computer session, open its live row on the phone, send a unique message, confirm the same computer session receives it and the answer appears on the phone, then stop the computer and confirm sending becomes unavailable without opening or billing a cloud session. Repeat while another phone tab has an unsent draft and while the computer is busy. Real device/background push delivery and deployed desktop/backend integration are outside the isolated mobile test harness.
