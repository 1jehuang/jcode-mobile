# Real NativeActivity service instrumentation

Build only, without contacting any device:

```sh
scripts/android-native-services-test.sh
```

The coordinator alone may run on a disposable emulator that already has the real,
matching-signed `org.jcode.mobile` debug APK installed:

```sh
scripts/android-native-services-test.sh --run emulator-5554
```

The script never selects an implicit device, starts an emulator, resets app data,
or uses a physical device. It requires an explicit `emulator-*` serial and checks
`ro.kernel.qemu=1` before installing/running tests. JDK defaults to
`~/Android/jdk-21`, platform to SDK36, build tools to 35.0.0, signing key to
`~/.android/jcode-debug.keystore`. `ANDROID_KEYSTORE`, `ANDROID_KEY_ALIAS`,
`ANDROID_KEYSTORE_PASSWORD`, `ANDROID_KEY_PASSWORD`, `ANDROID_BUILD_TOOLS`, and
`ANDROID_NATIVE_TEST_OUT` can override those defaults. The signing certificate
must match the target debug APK for cross-package instrumentation.

The APK package is `org.jcode.mobile.nativeservicestests`, with two runners:

* `.StorageInstrumentation` targets **the test package**, asserts its Linux UID
  differs from the target UID, and loads `AndroidServices` from the installed
  production APK using `DexClassLoader`. It bypasses only the constructor's JNI
  service registration using reflective `Unsafe.allocateInstance`, then injects
  an Activity wrapped around the test package context. Production methods execute
  unchanged, against real Android Keystore and test-private no-backup storage.
  The hardcoded account alias is therefore created/deleted only under the test
  UID. Preexisting test alias/files cause a refusal, not destructive replacement.
  Assertions cover decrypt roundtrip, non-exportability, encrypted envelope,
  randomized ciphertext, absence of the plaintext marker in the isolated
  no-backup directory, authenticated corruption rejection, and key/file clear.
  Reflection restrictions or any setup failure are failures, never skips/passes.
* `.NativeInstrumentation` targets **`org.jcode.mobile`**, launches its actual
  `JcodeActivity extends NativeActivity` with the shared Rust `todos` debug demo,
  and uses its real clipboard and lifecycle adapters. Host-discovery assertions
  call the production helper against the actual `NativeActivity$NativeContentView`
  and verify generic containers/IME are rejected and SurfaceView fallback works.
  Host installation is limited to `android.R.id.content`, never system decor.
  Clipboard contents are
  saved and restored even when subsequent field tests fail. It invokes real
  pause/resume callbacks and checks service state and recorder release, but
  does not claim OS-driven background/process recreation coverage.
  For IME it finds the production NativeContentView's semantic provider, focuses an
  empty non-password native editable demo field, and calls the real
  `GpuiInput.onCreateInputConnection`. Compose, replacement, commit, deletion,
  and finish must succeed. Native semantic text must match after every edit.
  The Java connection's local Editable is **not** accepted as evidence. Native
  field content is restored via the semantic set-text action and re-observed.

Each runner returns `result=PASS` only after its required assertions and cleanup.
Missing native semantics or an editable demo field produces `UNTESTABLE` in a
failure, and the script exits nonzero. It still runs both runners and saves their
outputs under `ANDROID_NATIVE_TEST_OUT` (default scratch/target location).
The script does not uninstall either package. If a crashed storage run leaves
isolated test data, the coordinator may uninstall **only the test package** before
retrying. Do not clear or uninstall the user's target app to repair a test.

## Scope and remaining acceptance

These are bounded adapter tests, not all-platform parity. Compilation alone is
not emulator execution. Native editable-field observability depends on shared
GPUI semantic snapshots and may currently be unavailable. Full OS-driven
background/foreground, rotation and process-death restoration, actual IME app
behavior/Unicode selection, TalkBack, hardware-backed key guarantees, whole-device
plaintext forensics/backups, notification delivery/replies, sharing destinations,
photo pickers, camera, microphone capture, haptics and hardware behavior are not
established by these tests. No email, network send, camera or microphone capture
is performed. The debug demo route disables startup diagnostics/prewarming and
crash upload in the current source. The emulator should be disposable and offline
if independent prevention of all network traffic is required.
