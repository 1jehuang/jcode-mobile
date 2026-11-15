# Installed production Rust microphone acceptance

This isolated test APK loads the exact `.so` and `AndroidServices` class from the installed **debug** production APK. It invokes the real `Recording` API, Android worker, JNI bridge, `AudioRecord`, snapshot/meter/stream and production WAV encoder. There is no fake microphone or success fallback. Empty/unavailable capture fails. The production NativeActivity/GPUI application is not launched, and this test UID has no INTERNET permission. PCM and WAV stay in native memory and are discarded. Only the native library is temporarily copied into the test UID's code cache, then deleted.

Build-only, safe for workers:

```sh
bash scripts/android-rust-audio-test.sh
PYTHONDONTWRITEBYTECODE=1 python3 scripts/android-rust-audio-test-test.py
justrust test --locked -p jcode-mobile --lib voice_capture
```

The coordinating device owner must first build and install an updated debug production APK using the existing `scripts/android-build.sh`. That script documents its NDK-only local-build exception. Do not run simultaneous packaging/build commands against the shared target directory.

## Coordinator-only runtime

On an unlocked, coordinator-owned disposable emulator, after the production debug APK is installed:

```sh
bash scripts/android-rust-audio-test.sh --run emulator-5554
```

The runner builds the isolated APK, verifies the explicit serial is an emulator, pulls the installed production APK read-only, requires its actual package identity and debuggable flag, selects a supported device ABI and requires the defined exported debug native hook before any test install or permission grant. It never changes production permissions or launches production. It installs/grants only `org.jcode.mobile.rustaudiotests`, captures the 60-second bounded instrumentation log at `target/android-rust-audio-tests/runtime-SERIAL.txt`, and applies the shared strict terminal parser. EXIT/INT/TERM cleanup force-stops and revokes microphone access only for the isolated package. Cleanup failure cannot hide the original failure, and turns an otherwise successful run into failure. An interrupted test's PCM stays in the terminated isolated process, not on disk.

`ANDROID_RUST_AUDIO_TEST_OUT` may override the output directory. The synthetic Python runner tests use only generated fake SDK executables in a temporary directory, never real adb. They cover build-only isolation, invalid serials, debug/hook/ABI/emulator preflight refusal before mutation, unique terminal result parsing, nonzero instrument exit despite PASS text, timeout status, cleanup failures and SIGINT/SIGTERM cleanup. Synthetic tests are not Android runtime acceptance.

Run separately on API35 and API36. Build-only success is not runtime acceptance. A missing debug native symbol, JNI exception, missing/empty stream, process crash, timeout, unexpected successful lifecycle stop or missing terminal output is a failure, not a skip/pass.

## Scenarios

0. Real nonempty worker capture, rejected duplicate Rust recording, on-demand snapshot with final-WAV prefix equality, finite bounded meter history, exact byte equality of all bounded-channel PCM frames with final production WAV, sender closed on successful stop.
1. Nonblocking `Recording` drop, sender closes, reservation and real AudioRecord release, then real recapture succeeds. This does not assert partial-frame cancellation because Android read boundaries may naturally be full frames.
2. Deliberately unconsumed capacity-one PCM channel forces real microphone backpressure. Worker publishes error, stop fails closed, and real restart succeeds.
3. Production service `onPause` while the Rust worker is recording causes a JNI read error, failed stop instead of a successful WAV, and background start rejection.
4. Same checks for production service `onDestroy`.

After every native scenario the Java harness checks that production `recorder` is null. All polls have three-second deadlines, with a 60-second coordinator timeout as the outer guard against unexpected blocking inside platform/JNI calls. Native assertions/panics become FAIL, never success.

## Scope limitations and isolation

The service is allocated with Unsafe to bypass its constructor and initialized through the real native bridge by the debug-only hook. Only the Activity and foreground flag are injected. The harness directly invokes production lifecycle service methods rather than proving actual app Activity-to-service lifecycle wiring. Native startup installs the account-storage adapter but never reads/writes accounts or invokes auth/transcription/network code. The native hook is gated by `target_os = "android"` and `debug_assertions`, so it is absent in release libraries.

This is not acceptance for GPUI record controls, actual app lifecycle wiring, permission UI, acoustic fidelity/physical microphones, audio focus/privacy switch, full 300-second/9.6 MB capture, transcription, auth or sending. Existing Java-only denied-permission acceptance remains separate. Host tests are algorithm checks, not live Android evidence.
