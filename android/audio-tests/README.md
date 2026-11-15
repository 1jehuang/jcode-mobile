# Isolated installed-production audio acceptance

Build only, without adb or a device:

```sh
JAVA_HOME="$HOME/Android/jdk-21" scripts/android-audio-test.sh
```

Coordinator-only emulator validation, after installing the production debug APK and unlocking the emulator:

```sh
JAVA_HOME="$HOME/Android/jdk-21" scripts/android-audio-test.sh --run emulator-5554
```

Replace `emulator-5554` with the coordinator-owned emulator serial. The script refuses non-emulators. It installs only `org.jcode.mobile.audiotests`, changes only that test package's microphone permission, runs separate denied/granted instrumentations with a 45-second host timeout, and applies `scripts/android-instrumentation-result.py` to each saved log. Missing, duplicated, crashed, timed-out or failed terminal output cannot pass. Build-only output is not runtime acceptance.

The APK has no INTERNET permission and never launches the production NativeActivity, loads its native library, invokes native voice capture, saves PCM, or sends audio. A visible test Activity provides a real foreground UID. A DexClassLoader loads AndroidServices from the installed production APK. Unsafe allocation skips only its nativeInit constructor, then reflection injects the actual test Activity and initial foreground flag. No audio method or AudioRecord is replaced.

## Observables

* Denied RECORD_AUDIO plus a test-private prior-request preference exercises the production no-rationale `-2` branch. There must be no recorder and `audioRead` must throw IOException. The preference is restored. This does not exercise the first-request dialog or user denial workflow.
* Granted mode requires a real initialized, recording AudioRecord at mono PCM16 16000 Hz. Duplicate start must return `-1` without replacing it.
* Each capture must produce at least 3200 bytes within 2.5 seconds through production nonblocking `audioRead`, with at most 1600 bytes per chunk and even sample alignment. Empty/unavailable input fails. Sample values are neither synthesized nor asserted nonzero: emulator silence is valid PCM, not acoustic fidelity proof.
* Separate start/read cycles terminate through production `audioStop`, `onPause`, and `onDestroy`. Each must clear the recorder, release the real AudioRecord, and cause subsequent reads to throw IOException. Stop is idempotent, restart works, and pause clears foreground state and rejects a new start. These directly exercise Java cancellation/release primitives, not UI lifecycle delivery or Rust cancellation races.

## Explicit limits

Production WAV encoding and bounded worker accumulation are in Rust `src/voice_capture.rs`, not AndroidServices. There is no callable Java acceptance hook for them. This test deliberately does not duplicate an encoder or claim a test-generated WAV proves production success. JNI integration, Rust WAV bytes/limits, Rust worker cancellation, GPUI recording controls, real Activity-to-service lifecycle wiring, audio focus/privacy-switch behavior, acoustic correctness and voice-service sending remain untested. Run the production Rust unit tests and add a coordinated native acceptance hook if end-to-end WAV acceptance is required.

At implementation time only SDK36/JDK21 compilation, dexing, alignment, signature verification and strict-parser self-tests were run. Actual emulator validation belongs to the root coordinator.
