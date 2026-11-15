# Installed production Rust microphone acceptance

This isolated test APK loads the exact `.so` and `AndroidServices` class from the installed **debug** production APK. It invokes the real `Recording` API, Android worker, JNI bridge, `AudioRecord`, snapshot/meter/stream and production WAV encoder. There is no fake microphone or success fallback. Empty/unavailable capture fails. The production NativeActivity/GPUI application is not launched, and this test UID has no INTERNET permission. PCM and WAV stay in native memory and are discarded. Only the native library is temporarily copied into the test UID's code cache, then deleted.

Build-only, safe for workers:

```sh
bash android/rust-audio-tests/build.sh
justrust test --locked -p jcode-mobile --lib voice_capture
```

The coordinating device owner must first build and install an updated debug production APK using the existing `scripts/android-build.sh`. That script documents its NDK-only local-build exception. Do not run simultaneous packaging/build commands against the shared target directory.

## Coordinator-only runtime

On an unlocked, coordinator-owned disposable emulator, after the production debug APK is installed:

```sh
set -euo pipefail
SERIAL=emulator-5554
ADB="${ANDROID_HOME:-$HOME/Android/Sdk}/platform-tools/adb"
PACKAGE=org.jcode.mobile.rustaudiotests
LOG="target/android-rust-audio-tests/runtime-$SERIAL.txt"
[[ "$SERIAL" == emulator-* ]]
[[ "$("$ADB" -s "$SERIAL" shell getprop ro.kernel.qemu | tr -d '\r')" == 1 ]]
# Always clean only the isolated package, including crashes and timeouts.
trap '"$ADB" -s "$SERIAL" shell am force-stop "$PACKAGE"; "$ADB" -s "$SERIAL" shell pm revoke "$PACKAGE" android.permission.RECORD_AUDIO' EXIT
"$ADB" -s "$SERIAL" install -r -t target/android-rust-audio-tests/rust-audio-tests.apk
"$ADB" -s "$SERIAL" shell pm grant "$PACKAGE" android.permission.RECORD_AUDIO
timeout 60s "$ADB" -s "$SERIAL" shell am instrument -w -r "$PACKAGE/.RustAudioInstrumentation" | tee "$LOG"
python3 scripts/android-instrumentation-result.py "$LOG"
```

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
