#!/usr/bin/env bash
# Build-only. adb/emulator commands belong exclusively to the coordinator.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
SDK="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-$HOME/Android/Sdk}}"
JAVA_HOME="${JAVA_HOME:-$HOME/Android/jdk-21}"
TOOLS="$SDK/build-tools/${ANDROID_BUILD_TOOLS:-35.0.0}"
PLATFORM="$SDK/platforms/android-36/android.jar"
OUT="${ANDROID_RUST_AUDIO_TEST_OUT:-$ROOT/target/android-rust-audio-tests}"
KEYSTORE="${ANDROID_KEYSTORE:-$HOME/.android/jcode-debug.keystore}"
[[ $# == 0 ]] || { echo 'Build-only, no arguments accepted' >&2; exit 2; }
mkdir -p "$OUT"
OUT="$(cd "$OUT" && pwd)"
STAGE="$(mktemp -d "$OUT/build.XXXXXX")"
trap 'rm -rf "$STAGE"' EXIT
mkdir -p "$STAGE/classes" "$STAGE/dex"
"$TOOLS/aapt2" link -o "$STAGE/base.apk" -I "$PLATFORM" --manifest "$ROOT/android/rust-audio-tests/AndroidManifest.xml"
mapfile -t SOURCES < <(find "$ROOT/android/rust-audio-tests/src" -name '*.java' -print | sort)
"$JAVA_HOME/bin/javac" --release 8 -Xlint:all,-deprecation,-options -classpath "$PLATFORM" -d "$STAGE/classes" "${SOURCES[@]}"
mapfile -t CLASSES < <(find "$STAGE/classes" -name '*.class' -print | sort)
"$TOOLS/d8" --lib "$PLATFORM" --min-api 26 --output "$STAGE/dex" "${CLASSES[@]}"
cp "$STAGE/base.apk" "$STAGE/test.apk"
(cd "$STAGE/dex" && zip -q "$STAGE/test.apk" classes*.dex)
"$TOOLS/zipalign" -f 4 "$STAGE/test.apk" "$STAGE/aligned.apk"
export JCODE_ANDROID_STORE_PASS="${ANDROID_KEYSTORE_PASSWORD:-android}"
export JCODE_ANDROID_KEY_PASS="${ANDROID_KEY_PASSWORD:-$JCODE_ANDROID_STORE_PASS}"
APK="$OUT/rust-audio-tests.apk"
"$TOOLS/apksigner" sign --ks "$KEYSTORE" --ks-key-alias "${ANDROID_KEY_ALIAS:-androiddebugkey}" --ks-pass env:JCODE_ANDROID_STORE_PASS --key-pass env:JCODE_ANDROID_KEY_PASS --out "$APK" "$STAGE/aligned.apk"
"$TOOLS/apksigner" verify "$APK"
echo "BUILT ONLY, no native/device acceptance: $APK"
