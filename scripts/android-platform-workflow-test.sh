#!/usr/bin/env bash
# Build only by default. Root coordinator alone invokes --run on a disposable emulator.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SDK="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-$HOME/Android/Sdk}}"
export JAVA_HOME="${JAVA_HOME:-$HOME/Android/jdk-21}"
export PATH="$JAVA_HOME/bin:$PATH"
TOOLS="${ANDROID_BUILD_TOOLS:-$SDK/build-tools/35.0.0}"
PLATFORM="$SDK/platforms/android-36/android.jar"
OUT="${ANDROID_PLATFORM_WORKFLOW_OUT:-${JCODE_SCRATCH_DIR:-$ROOT/target}/android-platform-workflow-tests}"
KEYSTORE="${ANDROID_KEYSTORE:-$HOME/.android/jcode-debug.keystore}"
RUN=false
SERIAL=""
if [[ $# -gt 0 ]]; then
  [[ $# -eq 2 && "$1" == --run && "$2" == emulator-* ]] || { echo "Usage: $0 [--run emulator-SERIAL]" >&2; exit 2; }
  RUN=true; SERIAL="$2"
fi
for tool in aapt2 d8 zipalign apksigner; do [[ -x "$TOOLS/$tool" ]] || { echo "Missing $TOOLS/$tool" >&2; exit 1; }; done
[[ -f "$PLATFORM" && -x "$JAVA_HOME/bin/javac" && -f "$KEYSTORE" ]] || { echo 'Need SDK36, JDK21 and existing debug signing key' >&2; exit 1; }
mkdir -p "$OUT"
OUT="$(cd "$OUT" && pwd)"
STAGE="$(mktemp -d "$OUT/build.XXXXXX")"
ORIGINAL=""
TEST=org.jcode.mobile.platformworkflowtests
RUNNER="$TEST/.PlatformWorkflowInstrumentation"
cleanup() {
  local status=$?
  trap - EXIT
  if [[ -n "$ORIGINAL" ]]; then
    local action=revoke
    [[ "$ORIGINAL" == granted ]] && action=grant
    if ! timeout 15s "$ADB" -s "$SERIAL" shell pm "$action" "$TEST" android.permission.POST_NOTIFICATIONS; then
      echo 'FAIL: test-only notification permission restoration failed' >&2
      status=1
    elif ! run_phase probe "$OUT/restored.txt"; then
      echo 'FAIL: test-only notification permission restoration probe failed' >&2
      status=1
    elif ! grep -qx "INSTRUMENTATION_RESULT: notification_permission=$ORIGINAL" "$OUT/restored.txt"; then
      echo 'FAIL: test-only notification permission restoration mismatch' >&2
      status=1
    fi
  fi
  rm -rf "$STAGE"
  if $RUN && [[ "$status" -eq 0 ]]; then
    echo 'PASS: bounded isolated installed-production Java workflows and test permission restoration. Not full shared GPUI parity.'
  fi
  exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
mkdir -p "$STAGE/classes" "$STAGE/dex"
"$TOOLS/aapt2" link -o "$STAGE/base.apk" -I "$PLATFORM" --manifest "$ROOT/android/platform-workflow-tests/AndroidManifest.xml"
mapfile -t SOURCES < <(find "$ROOT/android/platform-workflow-tests/src" -name '*.java' -print | sort)
javac --release 8 -Xlint:all,-deprecation,-options -classpath "$PLATFORM" -d "$STAGE/classes" "${SOURCES[@]}"
mapfile -t CLASSES < <(find "$STAGE/classes" -name '*.class' -print | sort)
"$TOOLS/d8" --lib "$PLATFORM" --min-api 33 --output "$STAGE/dex" "${CLASSES[@]}"
cp "$STAGE/base.apk" "$STAGE/test.apk"
(cd "$STAGE/dex" && zip -q "$STAGE/test.apk" classes*.dex)
"$TOOLS/zipalign" -f 4 "$STAGE/test.apk" "$STAGE/aligned.apk"
export JCODE_ANDROID_STORE_PASS="${ANDROID_KEYSTORE_PASSWORD:-android}"
export JCODE_ANDROID_KEY_PASS="${ANDROID_KEY_PASSWORD:-$JCODE_ANDROID_STORE_PASS}"
APK="$OUT/platform-workflow-tests.apk"
"$TOOLS/apksigner" sign --ks "$KEYSTORE" --ks-key-alias "${ANDROID_KEY_ALIAS:-androiddebugkey}" --ks-pass env:JCODE_ANDROID_STORE_PASS --key-pass env:JCODE_ANDROID_KEY_PASS --out "$APK" "$STAGE/aligned.apk"
"$TOOLS/apksigner" verify "$APK"
echo "BUILT ONLY (not device-tested): $APK"
if ! $RUN; then exit 0; fi
ADB="$SDK/platform-tools/adb"
[[ "$("$ADB" -s "$SERIAL" shell getprop ro.kernel.qemu | tr -d '\r')" == 1 ]] || { echo 'Refusing non-emulator device' >&2; exit 1; }
API="$("$ADB" -s "$SERIAL" shell getprop ro.build.version.sdk | tr -d '\r')"
[[ "$API" =~ ^[0-9]+$ && "$API" -ge 33 ]] || { echo 'Requires API33+' >&2; exit 1; }
"$ADB" -s "$SERIAL" shell pm path org.jcode.mobile | grep -q '^package:' || { echo 'Install production APK first' >&2; exit 1; }
"$ADB" -s "$SERIAL" install -r -t "$APK"
HEADLINE="workflow-owned-$(cat /proc/sys/kernel/random/uuid)"
run_phase() {
  local phase="$1" log="$2"
  timeout 90s "$ADB" -s "$SERIAL" shell am instrument -w -r -e phase "$phase" -e headline "$HEADLINE" "$RUNNER" | tr -d '\r' | tee "$log" || return $?
  python3 "$ROOT/scripts/android-instrumentation-result.py" "$log"
}
run_phase probe "$OUT/original.txt"
ORIGINAL="$(awk -F= '/^INSTRUMENTATION_RESULT: notification_permission=(granted|denied)$/ {print $2}' "$OUT/original.txt")"
[[ "$ORIGINAL" == granted || "$ORIGINAL" == denied ]] || { ORIGINAL=""; echo 'Missing unique original permission state' >&2; exit 1; }
"$ADB" -s "$SERIAL" shell pm revoke "$TEST" android.permission.POST_NOTIFICATIONS
run_phase denied "$OUT/denied.txt"
"$ADB" -s "$SERIAL" shell pm grant "$TEST" android.permission.POST_NOTIFICATIONS
run_phase granted "$OUT/granted.txt"
echo 'Workflow phases passed. Final acceptance requires exit-trap permission restoration.'
