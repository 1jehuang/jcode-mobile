# Android validation, November 14, 2026

## Delivered artifact

- `target/android/JcodeMobile.apk`, debug-signed universal APK, 35,832,452 bytes.
- SHA-256: `2ee215048d03a983336619582308915bcb75ba390ac8570feca96f6300ce10b0`.
- Native ABIs: `arm64-v8a` and `x86_64`. Minimum API 26, target API 35.
- Both native targets cross-built using the installed Android NDK through justified local `justrust` builds. NDK LLD used explicitly, not the experimental host linker.
- APK signature verification passed with v2 and v3 signatures. `zipalign -c -P 16 4` passed. Build script verifies native ELF LOAD alignment of at least 16 KiB for both ABIs.
- Source includes Android client identity, accessibility/insets fixes, unsupported-token rejection, and disabled persistent Android cloud SSH device keys.

This is a native Java Android interface around the shared Rust core and SDK. It is not an Android GPUI backend or a complete iOS UI port. Build and connection instructions are in `docs/ANDROID.md`.

## Actual emulator acceptance

Both emulators used KVM on this Linux host and ran the actual packaged x86_64 JNI library. No physical phone was used.

| Environment | Serial | Final instrumentation | Final black-box UI |
| --- | --- | --- | --- |
| Android 11, API 30, Google APIs x86_64 | emulator-5556 | Passed, 352 assertions | Passed, 14 checks, 67.3 seconds |
| Android 15, API 35, Google APIs x86_64 | emulator-5554 | Passed, 309 assertions | Passed, 14 checks, 71.4 seconds |

Instrumentation assertion counts vary because bounded wait loops validate repeated snapshots. They are not counts of independent test scenarios.

The final native instrumentation enabled all four fixture endpoints: normal protocol, delayed streaming, rejected handshake, and disconnect-once-after-send. It verified:

- Actual JNI loading, JSON error handling, invalid endpoints, safe unsupported-token rejection, and reset/logout.
- Offline demo through Rust, including exact Unicode, emoji, and multiline round trips.
- Real Android TCP connection using the real SDK protocol and shared session reducer, session/history loading, model selection, fresh message streaming, session switching, wake, and disconnect.
- Cancellation after a partial streamed reply, bounded handshake failure, and reconnect to the same session without duplicating the submitted message.
- Invalid cloud email and missing-login errors without sending real verification emails.

The 14 UI checks per emulator cover accessible launch/composer, disabled disconnected send, blank-email validation, explicit native demo, actually visible software keyboard, send accessibility above the keyboard, native demo reply, unsent drafts across background/resume and rotation, large fonts, real socket history and streamed messages, disconnect, and absence of app JVM/native fatal crashes in the captured run.

Final commands used `scripts/android-test.sh` with `ANDROID_TEST_ENDPOINT`, `ANDROID_TEST_SLOW_ENDPOINT`, `ANDROID_TEST_REJECT_ENDPOINT`, and `ANDROID_TEST_RECONNECT_ENDPOINT`. UI used `scripts/android-smoke.py --endpoint ...`. Each UI run used its own fresh fixture history. Each reconnect run used a fresh once-disconnecting fixture.

Evidence is retained locally:

- `target/android/instrumentation-emulator-5556.txt`
- `target/android/instrumentation-emulator-5554.txt`
- `target/android/smoke-emulator-5556/result.json`
- `target/android/smoke-emulator-5554/result.json`
- Screenshots, accessibility dumps, and app-filtered logs in those smoke directories.

## Rust and tooling checks

Earlier hosted validation passed all 9 Android bridge tests and scoped Clippy with warnings denied. Those tests include actual SDK TCP fixtures, fake local cloud email start/verify HTTP, account credential non-disclosure, logout races, handshake failure, and reconnect history without resent messages. Earlier hosted shared-core regression validation passed 126 tests with 3 ignored.

Final combined hosted regression passed after the credential-storage and Android client-identity changes: `justrust test -p jcode-mobile-core -p jcode-mobile-android`, **138 passed, 0 failed, 3 ignored**. This includes the fail-closed Android credential-store test. Hosted run `20261114-145759910-3517787` completed in 527.2 seconds. Earlier retries hit hosted broken-pipe failures, but the final run succeeded on hosted without a local host-test fallback. Both final Android cross-builds and emulator acceptance runs include the security changes.

The hosted report identified shared build-slot contention, dependency invalidations, and a 123-second existing crash-hook test. These explain the slow final regression loop, not an Android functional failure. No unrelated crate restructuring was performed.

ShellCheck passed for all three Android shell scripts. `git diff --check` passed. The Python smoke helper was syntax checked.

## Problems found and corrected

- Pinned GPUI has no Android backend: implemented a separate native UI sharing the existing Rust networking/state implementation.
- No Android SDK/JDK initially: installed isolated SDK, NDK, system images and a full JDK without replacing system Java.
- Host experimental linker interception: explicitly selected NDK LLD for Android.
- Polling repeatedly changed accessibility state: stopped no-op UI mutations and serialized JNI work without disabling idle controls.
- Android 15 edge-to-edge insets erased content spacing: separated outer system-inset container from padded inner content.
- Unsupported gateway token suggested false authentication: removed the field and rejected nonempty tokens at the JNI boundary.
- Generic desktop cloud device-key persistence was unsafe for Android: disabled persistent shortcut and permanent remote authorization. Normal pinned ephemeral SSH remains.
- Initial partial-stream assertion depended on trailing whitespace and message order: changed the test to find the actual streaming assistant fragment.
- Reusing accumulated fixture history hid the initial history marker below the accessibility viewport: final UI runs used independent clean fixture instances, documented as a test precondition.

## Explicit remaining limits

- No live cloud account, actual verification email, paid server wake, or production SSH login was exercised. Fake-control-plane tests are not production account acceptance.
- ARM64 builds successfully, but runtime tests were x86_64 emulators only. No physical-device or ARM64 runtime claim is made.
- Networking fixtures implement real SDK transport and events but run no live model. Streamed fixture replies are deliberately labeled as local test content.
- Photo/voice capture, push notifications, extensions, native sharing, and full iOS visual parity are not implemented in this Android shell.
- API 26 is the declared minimum, not an emulator-tested version. Only API 30 and 35 were exercised.
- No Play Store AAB/publishing or production signing validation. The APK is development-signed.
- Emulator tests cannot establish OEM keyboard behavior, real GPU quirks, battery/background restrictions, camera/microphone behavior, or push delivery.

All emulator and fixture processes started for this validation were stopped after acceptance. APK and test evidence remain available on disk.
