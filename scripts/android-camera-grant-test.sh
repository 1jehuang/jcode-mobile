#!/usr/bin/env bash
# Build-only by default. Coordinator alone invokes --run on an emulator.
# Instrumentation restarts the target app. Never use during another device task.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SDK="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-$HOME/Android/Sdk}}"
OUT="${ANDROID_NATIVE_TEST_OUT:-${JCODE_SCRATCH_DIR:-$ROOT/target}/android-native-services-tests}"
RUN=false
SERIAL=""
if [[ $# -gt 0 ]]; then
  [[ $# -eq 2 && "$1" == --run && "$2" == emulator-* ]] || { echo "Usage: $0 [--run emulator-SERIAL]" >&2; exit 2; }
  RUN=true; SERIAL="$2"
fi
"$ROOT/scripts/android-native-services-test.sh"
if ! $RUN; then echo 'Camera permission instrumentation BUILT ONLY, not device-tested.'; exit 0; fi
ADB="$SDK/platform-tools/adb"
[[ "$("$ADB" -s "$SERIAL" shell getprop ro.kernel.qemu | tr -d '\r')" == 1 ]] || { echo 'Refusing non-emulator device' >&2; exit 1; }
"$ADB" -s "$SERIAL" shell pm path org.jcode.mobile | grep -q '^package:' || { echo 'Install updated production debug APK first' >&2; exit 1; }
"$ADB" -s "$SERIAL" install -r -t "$OUT/native-services-tests.apk"
URI=""
CLEANED=false
cleanup() {
  if [[ -n "$URI" ]] && ! $CLEANED; then
    "$ADB" -s "$SERIAL" shell am instrument -w -r -e phase cleanup -e cleanupOnly true -e uri "$URI" org.jcode.mobile.nativeservicestests/.CameraOwnerInstrumentation > "$OUT/camera-emergency-cleanup.txt" 2>&1 || true
    echo "Emergency fixture cleanup attempted, inspect $OUT/camera-emergency-cleanup.txt" >&2
  fi
}
trap cleanup EXIT
run() {
  local log="$1" runner="$2"; shift 2
  "$ADB" -s "$SERIAL" shell am instrument -w -r "$@" "org.jcode.mobile.nativeservicestests/.$runner" | tee "$OUT/$log.txt"
  grep -q '^INSTRUMENTATION_RESULT: result=PASS' "$OUT/$log.txt" && grep -q '^INSTRUMENTATION_CODE: -1' "$OUT/$log.txt"
}
run camera-setup CameraOwnerInstrumentation -e phase setup
URI="$(awk -F'uri=' '/^INSTRUMENTATION_RESULT: uri=/{print $2}' "$OUT/camera-setup.txt" | tr -d '\r')"
[[ "$URI" =~ ^content://org\.jcode\.mobile\.attachments/camera-[0-9]+\.jpg$ ]] || { echo 'Missing or unsafe production test output URI' >&2; exit 1; }
run camera-external CameraExternalInstrumentation -e phase granted -e uri "$URI"
run camera-cleanup CameraOwnerInstrumentation -e phase cleanup -e uri "$URI"
CLEANED=true
run camera-revoked CameraExternalInstrumentation -e phase revoked -e uri "$URI"
echo 'PASS: actual distinct-UID provider JPEG write/read, exact grants, sibling/traversal denial, production decode/cleanup, revoked read/write denial. This is not a real camera UI acceptance claim.'
