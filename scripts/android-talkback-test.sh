#!/usr/bin/env bash
# Build only unless root/coordinator explicitly supplies a disposable emulator serial.
# Real installed TalkBack, never a UiAutomation replacement service. No audio claim.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="${ANDROID_TALKBACK_TEST_OUT:-${JCODE_SCRATCH_DIR:-$ROOT/target}/android-talkback-tests}"
RUN=false
SERIAL=""
if [[ $# -gt 0 ]]; then
  [[ $# -eq 2 && "$1" == --run && "$2" == emulator-* ]] || { echo "Usage: $0 [--run emulator-SERIAL]" >&2; exit 2; }
  RUN=true; SERIAL="$2"
fi
mkdir -p "$OUT"
OUT="$(cd "$OUT" && pwd)"
ANDROID_NATIVE_TEST_OUT="$OUT" "$ROOT/scripts/android-native-services-test.sh"
if ! $RUN; then exit 0; fi
SDK="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-$HOME/Android/Sdk}}"
ADB="$SDK/platform-tools/adb"
adb() { "$ADB" -s "$SERIAL" "$@"; }
[[ "$(adb shell getprop ro.kernel.qemu | tr -d '\r')" == 1 ]] || { echo 'Refusing non-emulator device' >&2; exit 1; }
adb shell pm path org.jcode.mobile | grep -q '^package:'
adb shell pm path com.google.android.marvin.talkback | grep -q '^package:'
SERVICE='com.google.android.marvin.talkback/com.google.android.marvin.talkback.TalkBackService'
# No device changes occur before the state journal and EXIT trap are ready.
KEYS=(enabled_accessibility_services touch_exploration_enabled touch_exploration_granted_accessibility_services accessibility_enabled)
declare -A VALUES PRESENT
SNAPSHOT="$(adb shell settings --user current list secure | tr -d '\r')"
for key in "${KEYS[@]}"; do
  PRESENT[$key]=false; VALUES[$key]=''
  while IFS= read -r line; do
    if [[ "$line" == "$key="* ]]; then PRESENT[$key]=true; VALUES[$key]="${line#*=}"; break; fi
  done <<< "$SNAPSHOT"
  # These settings contain component names or booleans only. Refuse unsafe shell quoting.
  [[ "${VALUES[$key]}" != *"'"* && "${VALUES[$key]}" != *$'\n'* ]] || { echo "Unsafe setting value: $key" >&2; exit 1; }
done
unset SNAPSHOT
STATE="$OUT/talkback-settings-before.txt"
(umask 077; : > "$STATE"; for key in "${KEYS[@]}"; do printf '%s\t%s\t%s\n' "$key" "${PRESENT[$key]}" "${VALUES[$key]}"; done > "$STATE")
restore() {
  local status=$? failed=0 key current found attempt
  trap - EXIT INT TERM
  set +e
  for key in "${KEYS[@]}"; do
    if [[ "${PRESENT[$key]}" == true ]]; then
      adb shell settings --user current put secure "$key" "'${VALUES[$key]}'" || failed=1
    else
      adb shell settings --user current delete secure "$key" || failed=1
    fi
  done
  # The accessibility manager may asynchronously update touch-exploration state.
  for attempt in {1..20}; do
    current="$(adb shell settings --user current list secure | tr -d '\r')" || failed=1
    found=true
    for key in "${KEYS[@]}"; do
      if [[ "${PRESENT[$key]}" == true ]]; then
        grep -Fxq "$key=${VALUES[$key]}" <<< "$current" || found=false
      elif grep -q "^$key=" <<< "$current"; then found=false; fi
    done
    if $found; then break; fi
    sleep 0.25
  done
  if ! $found || [[ $failed -ne 0 ]]; then
    echo "FAIL: accessibility settings not restored. Recovery journal: $STATE, device: $SERIAL" >&2
    status=1
  else
    echo 'RESTORED: original enabled services, accessibility and touch-exploration settings'
  fi
  exit "$status"
}
trap restore EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
adb install -r -t "$OUT/native-services-tests.apk"
services="${VALUES[enabled_accessibility_services]}"
if [[ ":$services:" != *":$SERVICE:"* ]]; then services="${services:+$services:}$SERVICE"; fi
adb shell settings --user current put secure enabled_accessibility_services "'$services'"
adb shell settings --user current put secure accessibility_enabled 1
# Service binding is asynchronous. Java still requires the real enabled package and touch exploration.
sleep 3
LOG="$OUT/TalkBackInstrumentation.txt"
timeout --signal=TERM 120 "$ADB" -s "$SERIAL" shell am instrument -w -r \
  org.jcode.mobile.nativeservicestests/.TalkBackInstrumentation | tee "$LOG"
python3 "$ROOT/scripts/android-instrumentation-result.py" "$LOG"
echo 'PASS: bounded installed TalkBack traversal plus native semantic action. Password and spoken audio NOT established.'
