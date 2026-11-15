# Android local agent notification acceptance

## Scope and limitations

Production consumes the same serialized `AgentsState` as the iOS ActivityKit bridge. Android publishes a native group summary and at most six individually tappable session children. Titles prefer plan names, with state, detail, origin, bounded todo counts and optional elapsed timers. Each child uses the existing `jcode://session/<URI-encoded-id>` route, with no prompt or automatic submission. The shared resolver and signed-out behavior are unchanged.

This is a local Android platform equivalent for agent discoverability and individual session entry, not identical ActivityKit/Dynamic Island layout, custom clickable RemoteViews rows, voice assistant integration, or suspended/background push delivery. No FCM token, service, subscription, or network action is invented.

New notifications are private, local-only, ongoing and only-alert-once. Native group-alert suppression mutes both summary and children. A generic public fallback has no session title, identifier, details, actions or session PendingIntent. Lock-screen rendering still depends on the OS and user's privacy settings. Existing production channel settings are respected. The old `syncLive(String)` adapter remains unchanged for compatibility.

Only owned summary tag `agents`/ID 1 and child prefix `agents.session.`/ID 1 are canceled. Obsolete private rows are removed. Empty state clears owned notifications before permission/channel checks, including attempted tags not yet enumerated. Denied/blocked nonempty updates clear owned private state and return false for the existing bounded retry cache. No `cancelAll` or unrelated turn-notification changes occur. Full encoded IDs, not hashes, identify tags and session PendingIntents. Payload/text and remembered owned tags are bounded, with no persisted account JSON.

## Pure host and SDK validation

```sh
python3 scripts/android-agent-notifications-test.py
bash scripts/android-agent-notifications-test.sh
```

The Python suite executes actual production Java method/helper source with fake OS APIs from `AgentContract.java`. It covers bounds, Java hash collisions (`Aa`/`BB`), rank-independent identity, content updates, plans/todos/timers, URI encoding, private generic public versions, malformed/oversized inputs, denied/blocked/partial failures, owned-only cleanup and rapid teardown before notification enumeration sees an attempted post. This is synthetic algorithm evidence, not actual Android delivery.

The shell command builds/signs a separate instrumentation APK using SDK tools, without Gradle, adb or device access by default. SDK success is explicitly **BUILT ONLY**, not runtime PASS. No existing shared test manifest is modified.

## Coordinator-only actual OS validation

```sh
# Root alone runs this after installing the current production APK on an API33+ emulator.
bash scripts/android-agent-notifications-test.sh --run emulator-5556
```

The separate APK instruments **its own test package and UID**, never the production app. It has no INTERNET permission and never starts GPUI, nativeInit, account storage or server connections. It loads the installed production APK's unmodified `AndroidServices` through `DexClassLoader`, allocates only that helper without its nativeInit constructor, and supplies an isolated test Activity plus the bounded owned-tag set normally initialized by the constructor. Every actual adapter method remains production code. UID assertions reject the production UID.

The runner first probes original **test-only** notification permission, enabled state, and empty notification/channel namespace. It refuses existing test records/channels. It revokes only the isolated test permission for the denied phase, grants only that permission for the granted phase, then restores the original grant and enabled state and verifies both against a final probe. It never grants/revokes production permissions, changes app ops, or adjusts production notification settings. All phases, cleanup and restoration must succeed for terminal PASS.

The granted phase inspects real `NotificationManager` records and generic public versions, immutable distinct/stable OS PendingIntent identity for colliding IDs, actual child PendingIntent dispatch to the isolated Activity for both IDs and an escaped Unicode/special ID, plan/todo/timer content, six-child capacity, malformed rejection, replacement/empty teardown, and preservation of an unrelated synthetic sentinel. The denied phase checks false publication acknowledgement, true empty teardown, and no active owned records. The synthetic host suite additionally tests stale owned cleanup while permission is denied, since the OS may itself remove notifications on permission revocation.

Instrumentation `finally` first checks production-owned teardown while preserving any failure. It then cancels only owned test records/PendingIntents, removes its sentinel and newly owned test-only channel, verifies an empty namespace, and finishes the test Activity. The runner's EXIT trap independently invokes isolated fallback cleanup, restores original test permission, and probes restoration. Cleanup errors become failures and cannot turn a failed phase into PASS. SIGINT/SIGTERM run these traps. SIGKILL or device loss cannot guarantee cleanup and will not produce terminal PASS, and a later run refuses leftover test resources rather than silently overwriting them.

This validates installed Java adapters under an isolated UID plus real OS records and dispatch. It does **not** validate the shared Rust deep-link consumer, authenticated session resolution, actual notification-shade row gestures, lock-screen rendering/redaction on all devices, OS throttling behavior, assistant integration, or FCM/background updates. Those remain separate acceptance boundaries. A missing prerequisite, crash, timeout, cleanup failure, restoration mismatch or absent record is failure/unavailable, never a passing skip.

Logs (`original`, `denied`, `granted`, `cleanup`, `restored`) are stored under `ANDROID_AGENT_TEST_OUT` or the scratch `android-agent-notification-tests` directory. `scripts/android-instrumentation-result.py` must accept every terminal phase result, and the shell runner must exit zero after its restoration trap, before claiming actual-OS PASS.
