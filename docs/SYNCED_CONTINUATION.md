# Continue a synced local session in cloud

Open the session drawer. The **Synced from your computers** section loads the
signed-in account's local synced sessions independently of the live SSH session
list. Search matches title/session ID and computer host. Local entries show their
latest synced text preview and remain read-only.

1. Tap **Continue in cloud** on a local entry.
2. Read the relocation notice and enter what Jcode should do next.
3. Tap **Continue in cloud** again (or submit the text field).
4. The app sends `POST /mobile/commands` relative to the account's API base with
   `{kind:"transfer", session_id:<original local ID>, text:<next message>}`.
5. Pending/running state prevents duplicate submissions. Command status is polled
   until completion. A successful result opens the new cloud session in a tab if
   the continuation drawer is still active. If the user has moved elsewhere, an
   **Open cloud continuation** link is shown next time instead of stealing focus.

The original local transcript stays intact. This copies only synced visible text,
not files, running processes, tools, hidden reasoning, or provider credentials.
The runner adds relocation context before the user's next message.

The refresh action reconciles pending command state, including after restarting
the app. Account changes clear all synced UI state. Async responses from a
different account or API base are ignored. Network errors during polling retain
the pending command. An uncertain enqueue response is reconciled with the command
list before a retry is allowed.

## Validation

- `justrust check -p jcode-mobile -p jcode-mobile-core`
- `justrust test --lib -p jcode-mobile-core`
- `justrust test -p jcode-mobile --bin jcode-mobile`

Core tests exercise real HTTP against an in-process mock server, including the
bearer header, API paths, explicit transfer payload, result-session navigation,
subscription errors, and local-only eligibility. The GPUI gesture stress test
renders local, pending, and completed transfer fixtures. Pure account-guard tests
cover logout and API-base/key mismatches.

For end-to-end acceptance after the backend migration/Worker and compatible
runner are deployed: sync a local transcript, continue from the mobile drawer,
verify the new cloud conversation includes its visible history and relocation
context, and verify the original local session is unchanged. Also close the
drawer during continuation and confirm completion does not steal focus.

No backend deployment, remote database migration, mobile publication, or live
account transfer was performed as part of this change.
