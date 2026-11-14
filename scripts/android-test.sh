#!/usr/bin/env bash
# Package and run dependency-free instrumentation against the actual JNI APK.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SDK="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-$HOME/Android/Sdk}}"
TOOLS="$SDK/build-tools/${ANDROID_BUILD_TOOLS:-35.0.0}"
PLATFORM="$SDK/platforms/android-${ANDROID_API:-35}/android.jar"
OUT="$ROOT/target/android"
ADB="$SDK/platform-tools/adb"
JAVA_BIN="${JAVA_HOME:+$JAVA_HOME/bin/}"
[[ -n "${ANDROID_SERIAL:-}" ]] || { echo 'Set ANDROID_SERIAL to an emulator, e.g. emulator-5554' >&2; exit 1; }
[[ "$("$ADB" -s "$ANDROID_SERIAL" shell getprop ro.kernel.qemu | tr -d '\r')" == 1 ]] || { echo 'This test helper only runs against emulators.' >&2; exit 1; }
STAGE="$(mktemp -d "$OUT/tests.XXXXXX")"
trap 'rm -rf "$STAGE"' EXIT
mkdir -p "$STAGE/classes" "$STAGE/dex"
"$TOOLS/aapt2" link -I "$PLATFORM" --manifest "$ROOT/android/tests/AndroidManifest.xml" -o "$STAGE/base.apk"
find "$ROOT/android/tests/src" -name '*.java' > "$STAGE/sources.txt"
"${JAVA_BIN}javac" --release 8 -classpath "$PLATFORM" -d "$STAGE/classes" @"$STAGE/sources.txt"
"${JAVA_BIN}jar" cf "$STAGE/classes.jar" -C "$STAGE/classes" .
"$TOOLS/d8" --min-api 26 --lib "$PLATFORM" --output "$STAGE/dex" "$STAGE/classes.jar"
cp "$STAGE/base.apk" "$STAGE/test.apk"
(cd "$STAGE/dex" && zip -q -0 "$STAGE/test.apk" classes.dex)
"$TOOLS/zipalign" -f 4 "$STAGE/test.apk" "$STAGE/aligned.apk"
export JCODE_ANDROID_STORE_PASS="${ANDROID_KEYSTORE_PASSWORD:-android}"
export JCODE_ANDROID_KEY_PASS="${ANDROID_KEY_PASSWORD:-$JCODE_ANDROID_STORE_PASS}"
"$TOOLS/apksigner" sign --ks "${ANDROID_KEYSTORE:-$HOME/.android/jcode-debug.keystore}" --ks-key-alias "${ANDROID_KEY_ALIAS:-androiddebugkey}" --ks-pass env:JCODE_ANDROID_STORE_PASS --key-pass env:JCODE_ANDROID_KEY_PASS --out "$OUT/JcodeMobileTests.apk" "$STAGE/aligned.apk"
"$ADB" -s "$ANDROID_SERIAL" install -r "$OUT/JcodeMobileTests.apk"
args=()
[[ -n "${ANDROID_TEST_ENDPOINT:-}" ]] && args+=(-e endpoint "$ANDROID_TEST_ENDPOINT")
[[ -n "${ANDROID_TEST_REJECT_ENDPOINT:-}" ]] && args+=(-e reject_endpoint "$ANDROID_TEST_REJECT_ENDPOINT")
[[ -n "${ANDROID_TEST_RECONNECT_ENDPOINT:-}" ]] && args+=(-e reconnect_endpoint "$ANDROID_TEST_RECONNECT_ENDPOINT")
[[ -n "${ANDROID_TEST_SLOW_ENDPOINT:-}" ]] && args+=(-e slow_endpoint "$ANDROID_TEST_SLOW_ENDPOINT")
timeout 120 "$ADB" -s "$ANDROID_SERIAL" shell am instrument -w "${args[@]}" org.jcode.mobile.tests/org.jcode.mobile.tests.SmokeInstrumentation | tee "$OUT/instrumentation-$ANDROID_SERIAL.txt"
grep -q JCODE_TESTS_PASSED "$OUT/instrumentation-$ANDROID_SERIAL.txt"
