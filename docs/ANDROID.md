# Android development and emulator validation

Android uses a small native Java interface backed by `jcode-mobile-android`, a JNI adapter around the same Rust connection and session reducer used on iOS. The pinned GPUI revision has no Android platform backend, so the iOS GPUI UI itself is not cross-compiled to Android. No WebView or duplicated network protocol is used.

## Requirements

The scripts currently support a Linux x86_64 build host:

- Rust with the `aarch64-linux-android` and `x86_64-linux-android` targets.
- `justrust` for Rust builds and checks.
- A full JDK (Java 17 or newer), including `javac`, `jar`, and `keytool`.
- Android command-line tools, SDK platform 35, build tools 35.0.0, NDK 27.2.12479018.
- `zip`, `unzip`, Python 3, and KVM access for the emulator.
- Android emulator images for API 30 and API 35, `google_apis;x86_64`.

Android Studio and Gradle are not needed. Install Google's command-line tools under `$ANDROID_HOME/cmdline-tools/latest`, then:

```sh
export ANDROID_HOME="$HOME/Android/Sdk"
export PATH="$ANDROID_HOME/cmdline-tools/latest/bin:$ANDROID_HOME/platform-tools:$PATH"
sdkmanager --licenses
sdkmanager 'platform-tools' 'platforms;android-35' 'build-tools;35.0.0' \
  'ndk;27.2.12479018' 'emulator' \
  'system-images;android-30;google_apis;x86_64' \
  'system-images;android-35;google_apis;x86_64'
rustup target add aarch64-linux-android x86_64-linux-android
```

Point `JAVA_HOME` at a full JDK and add `$JAVA_HOME/bin` to `PATH` if only a JRE is installed. This machine's isolated development JDK is under `~/Android/jdk-21`.

## Build and run

```sh
./scripts/android-build.sh
# target/android/JcodeMobile.apk contains arm64-v8a and x86_64.

# Faster emulator-only iteration:
ANDROID_ABIS=x86_64 ./scripts/android-build.sh

# Boot, wait for Android, install and open the app:
ANDROID_DEMO=1 ./scripts/android-emulator.sh
# Headless:
ANDROID_EMULATOR_HEADLESS=1 ANDROID_DEMO=1 ./scripts/android-emulator.sh
```

The default build is debug and signed with a development key generated in `~/.android/jcode-debug.keystore`, never in the repository. Native libraries are uncompressed, APK-aligned to 16 KiB, and linked with 16 KiB page alignment. Minimum Android API is 26, target API is 35. An ARM64 build is not evidence of ARM64 device runtime validation.

The cross-build uses `justrust --local` with an explicit NDK reason because hosted workers do not have the Android toolchain installed. Host-side Rust checks and tests continue to use hosted builds. If your local policy deliberately disables all local compilation, the script respects it. For this unsupported hosted target only, use the per-command override `JUSTRUST_LOCAL=on ./scripts/android-build.sh` rather than changing persistent policy.

For a release-profile APK, set `ANDROID_PROFILE=release`. This does **not** imply production signing. Supply `ANDROID_KEYSTORE`, `ANDROID_KEY_ALIAS`, `ANDROID_KEYSTORE_PASSWORD`, and optionally `ANDROID_KEY_PASSWORD` to sign with your own key. Do not commit those values or a keystore. This is APK development tooling, not a Play Store AAB publishing pipeline.

## Connections and credential handling

- **Jcode Cloud:** use **Cloud sign in**, enter your email and the verification code, then connect. This reuses the shared cloud client and pure-Rust SSH transport. Sign-in state is intentionally memory-only in this first Android implementation, so force-stopping the app requires signing in again. Logging out clears the in-memory account and connection.
- **Advanced direct server:** use **Settings** and a `tcp://host:port` endpoint for a securely forwarded harness API socket. This is newline-delimited harness API, **not** the legacy `/ws` gateway. There is no raw TCP transport encryption or authentication, so never expose this socket publicly. Use a trusted SSH tunnel or local emulator fixture. Nonempty gateway tokens are rejected rather than silently ignored.
- The endpoint is stored in app-private preferences. Tokens and cloud credentials are not stored in plaintext preferences, backups, bundles, or logs. Android backups are disabled.
- **Demo:** explicitly labeled offline. It exercises the packaged Rust JNI library but never contacts a server or runs a model.

The UI supports session lists, switching sessions, streamed chat, history, model selection, generation cancellation, foreground liveness probing, and disconnect. iOS-specific push notifications, extensions, native sharing, photo/voice capture, and full iOS visual parity are not implemented by this Android shell. It is not a GPUI Android backend.

## Repeatable emulator tests

Start a local protocol fixture. It runs no model and never touches a real user account:

```sh
python3 crates/jcode-mobile-android/scripts/harness_fixture.py --port 17643
```

In another terminal, build and launch an emulator, then run the real JNI integration tests and UI tests:

```sh
ANDROID_ABIS=x86_64 ./scripts/android-build.sh
ANDROID_EMULATOR_HEADLESS=1 ./scripts/android-emulator.sh
export ANDROID_SERIAL=emulator-5554
# Emulator host alias. adb reverse can alternatively map to 127.0.0.1.
export ANDROID_TEST_ENDPOINT=tcp://10.0.2.2:17643
./scripts/android-test.sh
python3 scripts/android-smoke.py --endpoint "$ANDROID_TEST_ENDPOINT"
```

Repeat on an older Android image:

```sh
ANDROID_EMULATOR_API=30 ANDROID_EMULATOR_PORT=5556 \
  ANDROID_EMULATOR_HEADLESS=1 ./scripts/android-emulator.sh
ANDROID_SERIAL=emulator-5556 ANDROID_TEST_ENDPOINT=tcp://10.0.2.2:17643 \
  ./scripts/android-test.sh
ANDROID_SERIAL=emulator-5556 python3 scripts/android-smoke.py \
  --endpoint tcp://10.0.2.2:17643
```

Both test helpers refuse physical devices. Instrumentation verifies actual native library loading, invalid commands/endpoints, safe token rejection, Unicode/emoji/multiline round trips, explicit offline demo, real socket handshake/history/session list, model selection, message streaming, session switching, wake and disconnect. The UI script exercises the installed app through accessibility and input events, including visible software keyboard handling, blank-email validation without sending mail, unsent drafts across background/resume and rotation, and larger font settings. It restores the emulator's original rotation, font, and keyboard settings even when a check fails.

For additional cancellation and recovery coverage, start independent fixture instances:

```sh
python3 crates/jcode-mobile-android/scripts/harness_fixture.py --port 17644 --delay 0.6
python3 crates/jcode-mobile-android/scripts/harness_fixture.py --port 17645 --reject-handshake
python3 crates/jcode-mobile-android/scripts/harness_fixture.py --port 17646 --disconnect-once-after-send
```

Pass `ANDROID_TEST_SLOW_ENDPOINT=tcp://10.0.2.2:17644`, `ANDROID_TEST_REJECT_ENDPOINT=tcp://10.0.2.2:17645`, and `ANDROID_TEST_RECONNECT_ENDPOINT=tcp://10.0.2.2:17646` alongside `ANDROID_TEST_ENDPOINT` to `android-test.sh`. These add actual partial-stream cancellation, bounded handshake failure, and same-session recovery without duplicated sends. Restart the once-disconnecting fixture before each emulator run so the fault is actually injected again.

Results, screenshots, accessibility dumps, and logs are saved under `target/android/`. Instrumentation uses a separate test APK with the same development signer, not a test entry point exposed in the shipping application.

Shared-code checks:

```sh
justrust check -p jcode-mobile-android
justrust test -p jcode-mobile-android
justrust clippy -p jcode-mobile-android --all-targets --no-deps -- -D warnings
justrust test -p jcode-mobile-core
```

The Rust bridge tests include a local fake cloud control-plane sign-in flow and the actual SDK protocol fixture. They do not send verification emails or validate a live paid cloud account. Emulator validation cannot establish real-device GPU, OEM keyboard, camera/microphone, Play distribution, battery/background restrictions, or push behavior. See the dated validation report for observed results and remaining gaps.
