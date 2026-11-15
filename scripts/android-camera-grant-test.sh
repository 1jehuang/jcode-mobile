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
# Do NOT start instrumentation in the recipient package after granting a URI:
# instrumentation startup force-stops that package, clearing temporary grants.
# One owner session launches a real cross-UID Activity recipient with ClipData
# grants, keeps it alive across production cleanup, and receives Binder results.
LOG="$OUT/camera-grant-roundtrip.txt"
"$ADB" -s "$SERIAL" shell am instrument -w -r org.jcode.mobile.nativeservicestests/.CameraOwnerInstrumentation | tee "$LOG"
grep -q '^INSTRUMENTATION_RESULT: result=PASS' "$LOG" && grep -q '^INSTRUMENTATION_CODE: -1' "$LOG"
echo 'PASS: live distinct-UID Activity JPEG write/read, exact grants, sibling/traversal denial, production decode/cleanup, explicit revoked read/write denial. Not a real camera UI acceptance claim.'
