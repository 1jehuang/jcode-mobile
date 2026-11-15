#!/usr/bin/env bash
# Build-only by default. Root coordinator alone may request an emulator run.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SDK="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-$HOME/Android/Sdk}}"
export JAVA_HOME="${JAVA_HOME:-$HOME/Android/jdk-21}"
export PATH="$JAVA_HOME/bin:$PATH"
TOOLS="${ANDROID_BUILD_TOOLS:-$SDK/build-tools/35.0.0}"
PLATFORM="$SDK/platforms/android-36/android.jar"
OUT="${ANDROID_AUDIO_TEST_OUT:-${JCODE_SCRATCH_DIR:-$ROOT/target}/android-audio-tests}"
KEYSTORE="${ANDROID_KEYSTORE:-$HOME/.android/jcode-debug.keystore}"
PACKAGE=org.jcode.mobile.audiotests
RUN=false
SERIAL=""
if [[ $# -eq 2 && "$1" == --check-log ]]; then
  exec python3 "$ROOT/scripts/android-instrumentation-result.py" "$2"
fi
if [[ $# -gt 0 ]]; then
  [[ $# -eq 2 && "$1" == --run && "$2" == emulator-* ]] || { echo "Usage: $0 [--run emulator-SERIAL | --check-log FILE]" >&2; exit 2; }
  RUN=true; SERIAL="$2"
fi
for tool in aapt2 d8 zipalign apksigner; do [[ -x "$TOOLS/$tool" ]] || { echo "Missing $TOOLS/$tool" >&2; exit 1; }; done
[[ -f "$PLATFORM" && -x "$JAVA_HOME/bin/javac" && -f "$KEYSTORE" ]] || { echo 'Need SDK36, JDK21 and existing debug signing key' >&2; exit 1; }
"$JAVA_HOME/bin/javac" -version 2>&1 | grep -Eq '^javac 21([ .]|$)' || { echo 'JDK21 required' >&2; exit 1; }
python3 "$ROOT/scripts/android-instrumentation-result.py" --self-test
mkdir -p "$OUT"
OUT="$(cd "$OUT" && pwd)"
STAGE="$(mktemp -d "$OUT/build.XXXXXX")"
CLEANUP_TEST_PACKAGE=false
cleanup() {
  local original_status=$? cleanup_failed=0
  trap - EXIT INT TERM
  if $CLEANUP_TEST_PACKAGE; then
    # Only this dedicated test package. Never production UID/data/permissions.
    timeout 10s "$ADB" -s "$SERIAL" shell am force-stop "$PACKAGE" || cleanup_failed=1
    timeout 10s "$ADB" -s "$SERIAL" shell pm revoke "$PACKAGE" android.permission.RECORD_AUDIO || cleanup_failed=1
    if [[ $cleanup_failed -ne 0 ]]; then echo "WARN: best-effort audio test-package cleanup failed on $SERIAL" >&2; fi
  fi
  rm -rf "$STAGE" || cleanup_failed=1
  # Preserve any original build/run/signal failure. Cleanup cannot hide it.
  if [[ $original_status -ne 0 ]]; then exit "$original_status"; fi
  exit "$cleanup_failed"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
mkdir -p "$STAGE/classes" "$STAGE/dex"
"$TOOLS/aapt2" link -o "$STAGE/base.apk" -I "$PLATFORM" --manifest "$ROOT/android/audio-tests/AndroidManifest.xml"
mapfile -t SOURCES < <(find "$ROOT/android/audio-tests/src" -name '*.java' -print | sort)
"$JAVA_HOME/bin/javac" --release 8 -Xlint:all,-deprecation,-options -classpath "$PLATFORM" -d "$STAGE/classes" "${SOURCES[@]}"
mapfile -t CLASSES < <(find "$STAGE/classes" -name '*.class' -print | sort)
"$TOOLS/d8" --lib "$PLATFORM" --min-api 26 --output "$STAGE/dex" "${CLASSES[@]}"
cp "$STAGE/base.apk" "$STAGE/test.apk"
(cd "$STAGE/dex" && zip -q "$STAGE/test.apk" classes*.dex)
"$TOOLS/zipalign" -f 4 "$STAGE/test.apk" "$STAGE/aligned.apk"
export JCODE_ANDROID_STORE_PASS="${ANDROID_KEYSTORE_PASSWORD:-android}"
export JCODE_ANDROID_KEY_PASS="${ANDROID_KEY_PASSWORD:-$JCODE_ANDROID_STORE_PASS}"
APK="$OUT/audio-tests.apk"
"$TOOLS/apksigner" sign --ks "$KEYSTORE" --ks-key-alias "${ANDROID_KEY_ALIAS:-androiddebugkey}" --ks-pass env:JCODE_ANDROID_STORE_PASS --key-pass env:JCODE_ANDROID_KEY_PASS --out "$APK" "$STAGE/aligned.apk"
"$TOOLS/apksigner" verify "$APK"
echo "BUILT ONLY (not device-tested): $APK"
if ! $RUN; then exit 0; fi
ADB="$SDK/platform-tools/adb"
[[ "$("$ADB" -s "$SERIAL" shell getprop ro.kernel.qemu | tr -d '\r')" == 1 ]] || { echo 'Refusing non-emulator device' >&2; exit 1; }
"$ADB" -s "$SERIAL" shell pm path org.jcode.mobile | grep -q '^package:' || { echo 'Install production debug APK first' >&2; exit 1; }
# Operates only on the dedicated test package, never production permissions/data.
CLEANUP_TEST_PACKAGE=true
"$ADB" -s "$SERIAL" install -r -t "$APK"
# Older package-manager shells do not expose this command. Probe advertised
# capability, not an arbitrary failed mutation. An advertised command failure
# remains fatal, and instrumentation still verifies the real no-rationale state.
PM_HELP_STATUS=0
PM_HELP="$(timeout 10s "$ADB" -s "$SERIAL" shell pm help | tr -d '\r')" || PM_HELP_STATUS=$?
# API30's package shell can emit valid help with status 255. Accept that
# specific compatibility case only with the expected header and body. This
# exception never applies to installs, permission mutations or instrumentation.
if [[ "$PM_HELP_STATUS" != 0 && "$PM_HELP_STATUS" != 255 ]]; then
  echo "Package-manager help failed with status $PM_HELP_STATUS" >&2
  exit "$PM_HELP_STATUS"
fi
if ! grep -Fxq 'Package manager (package) commands:' <<< "$PM_HELP" ||
   ! grep -Eq '^[[:space:]]+[^[:space:]]' <<< "$PM_HELP"; then
  echo 'Package-manager help missing expected exact header or nonempty body' >&2
  exit 1
fi
CAN_CLEAR_PERMISSION_FLAGS=false
if grep -Eq '^[[:space:]]*clear-permission-flags([[:space:]]|$)' <<< "$PM_HELP"; then
  CAN_CLEAR_PERMISSION_FLAGS=true
fi
FAILED=0
for MODE in denied granted; do
  if [[ "$MODE" == denied ]]; then
    "$ADB" -s "$SERIAL" shell pm revoke "$PACKAGE" android.permission.RECORD_AUDIO
    if $CAN_CLEAR_PERMISSION_FLAGS; then
      "$ADB" -s "$SERIAL" shell pm clear-permission-flags "$PACKAGE" android.permission.RECORD_AUDIO user-set user-fixed
    else
      echo 'Package shell lacks clear-permission-flags. Denied instrumentation must verify actual no-rationale state; no bypass.'
    fi
  else
    "$ADB" -s "$SERIAL" shell pm grant "$PACKAGE" android.permission.RECORD_AUDIO
  fi
  LOG="$OUT/audio-$MODE.txt"
  if ! timeout 45s "$ADB" -s "$SERIAL" shell am instrument -w -r -e mode "$MODE" "$PACKAGE/.AudioInstrumentation" | tee "$LOG"; then FAILED=1; fi
  if ! python3 "$ROOT/scripts/android-instrumentation-result.py" "$LOG"; then FAILED=1; fi
done
# EXIT always releases the test permission and stops the test package, including
# set -e failures and INT/TERM. No recorded PCM is persisted.
if [[ $FAILED -ne 0 ]]; then echo 'FAIL or unavailable: inspect audio logs. No audio parity claim.' >&2; exit 1; fi
echo 'PASS: isolated installed production Java AudioRecord/PCM/stop and denied checks only. Rust WAV, JNI cancellation, voice UI and acoustic fidelity remain untested.'
