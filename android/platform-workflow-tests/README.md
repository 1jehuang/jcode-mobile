# Isolated installed-production platform workflows

This **separate** test APK registers
`org.jcode.mobile.platformworkflowtests/.PlatformWorkflowInstrumentation`, targeting
its own `org.jcode.mobile.platformworkflowtests` package, not the user's app.
No shared manifest, existing runner, Rust or production Java file is modified.

Build only, no device access:

```sh
scripts/android-platform-workflow-test.sh
```

Coordinator only, disposable explicitly selected API33+ emulator with the actual
`org.jcode.mobile` APK already installed:

```sh
scripts/android-platform-workflow-test.sh --run emulator-5554
```

SDK36, JDK21 and build-tools 35.0.0 defaults match the existing native harness.
Output defaults to `$JCODE_SCRATCH_DIR/android-platform-workflow-tests`, otherwise
`target/android-platform-workflow-tests`. Override with
`ANDROID_PLATFORM_WORKFLOW_OUT`. Existing SDK, signing key and Java overrides
match `scripts/android-native-services-test.sh`. Matching production signatures
are not needed because this instrumentation targets only its own package.
The built APK is `platform-workflow-tests.apk`.

## What executes

The test asserts its UID differs from the installed production app, loads
**that APK's** `AndroidServices` with `DexClassLoader`, bypasses only constructor
JNI registration with `Unsafe.allocateInstance`, and supplies a real launched
test Activity. It does not copy, replace, or mock the adapter methods or system
services. The separate Activity has no native library, account, network or model
code. Reflection restrictions fail the run, never skip to a pass.

* The `denied` phase creates two dynamic shortcuts using production
  `indexSessions`, queries the actual OS records, validates a session containing
  slash/query/Unicode characters survives URI encoding, dispatches the stored
  Intent to the actual test Activity, replaces the label/list to rename one and
  delete the other, and exercises empty-list clearing. This uses two publications
  and one clear, not a quota-exhaustion loop. Existing test dynamic/pinned shortcuts
  cause refusal before any replacement. Cleanup removes only the two owned IDs.
* In that phase, revoked test notification permission must produce a false
  production `syncLive` acknowledgement and no active notification. Empty
  cancellation must succeed even while denied.
* The coordinator runner grants **only the test package's** notification
  permission and invokes the `granted` phase with the **identical generated
  headline**. Production `syncLive` must acknowledge, and the actual
  `StatusBarNotification` must contain that text, the ongoing flag and the expected
  channel. Its real PendingIntent must deliver MAIN once to the test Activity.
  Production empty cancellation must remove the active notification.
* Permission revocation is performed between instrumentation processes, never
  inside the process whose UID could be killed by Android on revocation. A probe
  records the original test-package permission. An EXIT trap restores and probes
  that state even after failure. Neither global settings nor production app
  permission, data, shortcuts or notifications are changed. No blanket
  `cancelAll`, package clear, uninstall, launcher reset or throttling reset occurs.
  Test channel creation is allowed only if absent, then deleted during cleanup.

Every workflow phase must have a unique successful terminal result accepted by
`scripts/android-instrumentation-result.py`. Shell pipeline failure also fails.
Phases have a 90-second host deadline and asynchronous assertions a 5-second
monotonic deadline. Logs are `original.txt`, `denied.txt`, `granted.txt`, and
`restored.txt`. Process/host crashes can still leave test-owned state. Refusal on
retry is deliberate. Only the coordinator may uninstall this **test package** to
recover, never clear production app data.

## Exact evidence limits

Compilation is not device execution. This harness has been SDK36/JDK21 compiled,
not run by its author, because the root coordinator owns all adb/emulator access.
A PASS is real OS adapter workflow evidence under an isolated test UID, **not**
full shared GPUI acceptance. In particular:

* Stored shortcut Intent dispatch is not a launcher UI long-press/tap or pinning
  gesture. The endpoint is deliberately the isolated test Activity, not the
  production Rust deep-link parser. No real production session is opened.
* Denied/granted phases execute the same Java adapter content in separate
  processes. They do not prove shared Rust acknowledgement-cache invalidation,
  the five-second scheduled retry, or in-process unchanged-state recovery.
* This does not exhaust shortcut quota, reset system throttling, or establish
  failure recovery after Android's rate-limit interval.
* It deliberately does not fake JNI settings callbacks or emit a foreground
  settings PASS. The shared app's theme/font/reduce-motion invalidation still
  needs actual GPUI runtime observation while resumed, with root-owned settings
  snapshot/restoration. Isolated Java readbacks cannot establish that result.
* No notification shade gesture, background reply, TalkBack, physical-device
  feedback, auth, account, backend, FCM, model or network workflow is claimed.
