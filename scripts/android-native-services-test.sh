#!/usr/bin/env bash
# Build by default. Only the coordinator should invoke --run SERIAL, on an emulator.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SDK="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-$HOME/Android/Sdk}}"
export JAVA_HOME="${JAVA_HOME:-$HOME/Android/jdk-21}"
export PATH="$JAVA_HOME/bin:$PATH"
TOOLS="${ANDROID_BUILD_TOOLS:-$SDK/build-tools/35.0.0}"
PLATFORM="$SDK/platforms/android-36/android.jar"
OUT="${ANDROID_NATIVE_TEST_OUT:-${JCODE_SCRATCH_DIR:-$ROOT/target}/android-native-services-tests}"
KEYSTORE="${ANDROID_KEYSTORE:-$HOME/.android/jcode-debug.keystore}"
RUN=false
SERIAL=""
if [[ $# -gt 0 ]]; then
  [[ $# -eq 2 && "$1" == --run && "$2" == emulator-* ]] || { echo "Usage: $0 [--run emulator-SERIAL]" >&2; exit 2; }
  RUN=true; SERIAL="$2"
fi
for tool in aapt2 d8 zipalign apksigner; do [[ -x "$TOOLS/$tool" ]] || { echo "Missing $TOOLS/$tool" >&2; exit 1; }; done
[[ -f "$PLATFORM" && -x "$JAVA_HOME/bin/javac" && -f "$KEYSTORE" ]] || { echo "Need SDK36, JDK21, and existing target debug signing key: $KEYSTORE" >&2; exit 1; }
mkdir -p "$OUT"
OUT="$(cd "$OUT" && pwd)"
STAGE="$(mktemp -d "$OUT/build.XXXXXX")"
trap 'rm -rf "$STAGE"' EXIT
mkdir -p "$STAGE/classes" "$STAGE/dex"
"$TOOLS/aapt2" link -o "$STAGE/base.apk" -I "$PLATFORM" --manifest "$ROOT/android/tests/AndroidManifest.xml"
mapfile -t SOURCES < <(find "$ROOT/android/tests/src" -name '*.java' -print | sort)
javac --release 8 -Xlint:all,-deprecation,-options -classpath "$PLATFORM" -d "$STAGE/classes" "${SOURCES[@]}"
mapfile -t CLASSES < <(find "$STAGE/classes" -name '*.class' -print | sort)
"$TOOLS/d8" --lib "$PLATFORM" --min-api 26 --output "$STAGE/dex" "${CLASSES[@]}"
cp "$STAGE/base.apk" "$STAGE/test.apk"
(cd "$STAGE/dex" && zip -q "$STAGE/test.apk" classes*.dex)
"$TOOLS/zipalign" -f 4 "$STAGE/test.apk" "$STAGE/aligned.apk"
export JCODE_ANDROID_STORE_PASS="${ANDROID_KEYSTORE_PASSWORD:-android}"
export JCODE_ANDROID_KEY_PASS="${ANDROID_KEY_PASSWORD:-$JCODE_ANDROID_STORE_PASS}"
APK="$OUT/native-services-tests.apk"
"$TOOLS/apksigner" sign --ks "$KEYSTORE" --ks-key-alias "${ANDROID_KEY_ALIAS:-androiddebugkey}" --ks-pass env:JCODE_ANDROID_STORE_PASS --key-pass env:JCODE_ANDROID_KEY_PASS --out "$APK" "$STAGE/aligned.apk"
"$TOOLS/apksigner" verify "$APK"
echo "BUILT ONLY (not device-tested): $APK"
if ! $RUN; then exit 0; fi
ADB="$SDK/platform-tools/adb"
[[ "$("$ADB" -s "$SERIAL" shell getprop ro.kernel.qemu | tr -d '\r')" == 1 ]] || { echo 'Refusing non-emulator device' >&2; exit 1; }
"$ADB" -s "$SERIAL" shell pm path org.jcode.mobile | grep -q '^package:' || { echo 'Install real org.jcode.mobile debug APK first' >&2; exit 1; }
"$ADB" -s "$SERIAL" install -r -t "$APK"
FAILED=0
for runner in StorageInstrumentation NativeInstrumentation; do
  LOG="$OUT/$runner.txt"
  if ! "$ADB" -s "$SERIAL" shell am instrument -w -r "org.jcode.mobile.nativeservicestests/.$runner" | tee "$LOG"; then FAILED=1; fi
  if ! grep -q '^INSTRUMENTATION_RESULT: result=PASS' "$LOG" || ! grep -q '^INSTRUMENTATION_CODE: -1' "$LOG"; then FAILED=1; fi
done
if [[ $FAILED -ne 0 ]]; then echo "FAIL or UNTESTABLE: inspect $OUT/*Instrumentation.txt. No parity claim." >&2; exit 1; fi
echo 'PASS: bounded native-service checks only. Not full platform parity or hardware acceptance.'
