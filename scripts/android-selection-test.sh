#!/usr/bin/env bash
# Build-only by default. Root owns emulator selection and device scheduling.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="${ANDROID_SELECTION_TEST_OUT:-${JCODE_SCRATCH_DIR:-$ROOT/target}/android-selection-tests}"
RUN=false
SERIAL=""
if [[ $# -eq 2 && "$1" == --check-log ]]; then
  exec python3 "$ROOT/scripts/android-instrumentation-result.py" "$2"
fi
if [[ $# -gt 0 ]]; then
  [[ $# -eq 2 && "$1" == --run && "$2" == emulator-* ]] || { echo "Usage: $0 [--run emulator-SERIAL | --check-log FILE]" >&2; exit 2; }
  RUN=true; SERIAL="$2"
fi
# Root owns this shared manifest registration. Never patch it from this script.
grep -q 'android:name=".SelectionInstrumentation"' "$ROOT/android/tests/AndroidManifest.xml" || {
  echo 'Root must register .SelectionInstrumentation targeting org.jcode.mobile in android/tests/AndroidManifest.xml' >&2
  exit 1
}
mkdir -p "$OUT"
OUT="$(cd "$OUT" && pwd)"
python3 "$ROOT/scripts/android-instrumentation-result.py" --self-test
ANDROID_NATIVE_TEST_OUT="$OUT" "$ROOT/scripts/android-native-services-test.sh"
if ! $RUN; then echo 'Selection acceptance BUILT ONLY, not device-tested.'; exit 0; fi
SDK="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-$HOME/Android/Sdk}}"
ADB="$SDK/platform-tools/adb"
[[ "$("$ADB" -s "$SERIAL" shell getprop ro.kernel.qemu | tr -d '\r')" == 1 ]] || { echo 'Refusing non-emulator device' >&2; exit 1; }
API="$("$ADB" -s "$SERIAL" shell getprop ro.build.version.sdk | tr -d '\r')"
[[ "$API" =~ ^[0-9]+$ ]] && (( API >= 29 )) || { echo 'Public real-window inspection requires API29+. No skip/pass.' >&2; exit 1; }
"$ADB" -s "$SERIAL" shell pm path org.jcode.mobile | grep -q '^package:' || { echo 'Install real production debug APK first' >&2; exit 1; }
"$ADB" -s "$SERIAL" install -r -t "$OUT/native-services-tests.apk"
LOG="$OUT/selection.txt"
FAILED=0
if ! timeout 60s "$ADB" -s "$SERIAL" shell am instrument -w -r org.jcode.mobile.nativeservicestests/.SelectionInstrumentation | tee "$LOG"; then FAILED=1; fi
if ! python3 "$ROOT/scripts/android-instrumentation-result.py" "$LOG"; then FAILED=1; fi
if [[ $FAILED -ne 0 ]]; then
  echo 'FAIL or unavailable: selection acceptance not established. Inspect selection.txt. Interrupted/crashed runs may require manual clipboard restoration.' >&2
  exit 1
fi
echo 'PASS: real production selectable-text dialog, framework selection/Copy, scrolling, Back dismissal and live NativeActivity. Not long-press/drag-handle/toolbar gesture acceptance.'
