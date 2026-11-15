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
RUNNER_PID=''
TOUCH_DOWN=false
console_event() {
  local response
  response="$(adb emu event send "$@" | tr -d '\r')" || return 1
  printf '%s\n' "$response" >> "$OUT/talkback-console-events.txt"
  ! grep -q '^KO' <<< "$response" && grep -Fxq 'OK' <<< "$response"
}
release_touch() {
  console_event EV_ABS:ABS_MT_SLOT:0 EV_ABS:ABS_MT_TRACKING_ID:-1 EV_KEY:BTN_TOUCH:0 EV_SYN:0:0
}
restore() {
  local status=$? failed=0 key current found attempt
  trap - EXIT INT TERM
  set +e
  if $TOUCH_DOWN; then release_touch || failed=1; TOUCH_DOWN=false; fi
  if [[ -n "$RUNNER_PID" ]]; then
    kill "$RUNNER_PID" 2>/dev/null
    wait "$RUNNER_PID" 2>/dev/null
    # Stop only this disposable target's in-flight instrumentation on an aborted run.
    adb shell am force-stop org.jcode.mobile || failed=1
  fi
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
NONCE="$(python3 -c 'import secrets; print(secrets.token_hex(16))')"
timeout --signal=TERM --kill-after=5 100 "$ADB" -s "$SERIAL" shell am instrument -w -r \
  -e talkback_nonce "$NONCE" org.jcode.mobile.nativeservicestests/.TalkBackInstrumentation > "$LOG" 2>&1 &
RUNNER_PID=$!
COORDINATES=''
# Read only a unique, nonce-matching readiness record from this fresh instrumentation.
for attempt in {1..160}; do
  if COORDINATES="$(python3 - "$LOG" "$NONCE" <<'PY'
from pathlib import Path
import re
import sys
text = Path(sys.argv[1]).read_text(errors='replace')
records = re.findall(r'^INSTRUMENTATION_STATUS: talkback_ready=(.*)\r?$', text, re.MULTILINE)
if not records:
    sys.exit(1)
if len(records) != 1:
    sys.exit(2)
match = re.fullmatch(re.escape(sys.argv[2]) + r',([0-9]+),([0-9]+),([0-9]+),([0-9]+)', records[0].strip())
if not match:
    sys.exit(2)
x1, y1, x2, y2 = map(int, match.groups())
if not (0 <= x1 < x2 <= 20000 and 0 <= y1 == y2 <= 20000):
    sys.exit(2)
print(x1, y1, x2, y2)
PY
  )"; then break
  else
    parse_status=$?
    [[ $parse_status -eq 1 ]] || { cat "$LOG"; echo 'FAIL: invalid TalkBack readiness protocol' >&2; exit 1; }
  fi
  if ! kill -0 "$RUNNER_PID" 2>/dev/null; then break; fi
  sleep 0.25
done
[[ -n "$COORDINATES" ]] || { cat "$LOG"; echo 'FAIL: no genuine target-window readiness; resolve setup explicitly' >&2; exit 1; }
read -r X1 Y1 X2 Y2 <<< "$COORDINATES"
SWIPES=0
for gesture in {1..12}; do
  if ! kill -0 "$RUNNER_PID" 2>/dev/null; then break; fi
  # Emulator console hardware events enter InputReader and the accessibility
  # input filter. `adb shell input swipe` uses injection, bypassing that filter.
  TOUCH_DOWN=true
  console_event EV_ABS:ABS_MT_SLOT:0 "EV_ABS:ABS_MT_TRACKING_ID:$gesture" \
    "EV_ABS:ABS_MT_POSITION_X:$X1" "EV_ABS:ABS_MT_POSITION_Y:$Y1" EV_KEY:BTN_TOUCH:1 EV_SYN:0:0
  for point in {1..6}; do
    X=$((X1 + (X2 - X1) * point / 6))
    console_event "EV_ABS:ABS_MT_POSITION_X:$X" "EV_ABS:ABS_MT_POSITION_Y:$Y1" EV_SYN:0:0
    sleep 0.015
  done
  release_touch
  TOUCH_DOWN=false
  SWIPES=$((SWIPES + 1))
  sleep 1.5
done
runner_status=0
wait "$RUNNER_PID" || runner_status=$?
if [[ $runner_status -eq 0 ]]; then RUNNER_PID=''; fi
cat "$LOG"
[[ $runner_status -eq 0 && $SWIPES -gt 0 ]] || { echo 'FAIL: global gesture runner failed or injected no gestures' >&2; exit 1; }
python3 "$ROOT/scripts/android-instrumentation-result.py" "$LOG"
echo 'PASS: bounded installed TalkBack hardware-path traversal plus native semantic action. Password and spoken audio NOT established.'
