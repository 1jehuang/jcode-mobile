#!/usr/bin/env bash
# Random tap/swipe stress test on a booted iOS Simulator (macOS only).
# Usage: scripts/ios-monkey.sh <path/to/App.app> [seconds]
# Needs idb (brew install facebook/fb/idb-companion; pip install fb-idb).
# Exits non-zero if the app crashes or stops running. Crash logs go to $OUT.
set -uo pipefail
APP="$1"; DURATION="${2:-180}"
BUNDLE=sh.jcode.mobile.rust
OUT="${OUT:-monkey-out}"; mkdir -p "$OUT"
REPORTS="$HOME/Library/Logs/DiagnosticReports"

DEV=$(xcrun simctl list devices available -j | python3 -c '
import json,sys
d=json.load(sys.stdin)["devices"]
c=[x for r,l in d.items() if "iOS" in r for x in l if x["name"].startswith("iPhone")]
print(c[-1]["udid"])')
echo "simulator $DEV"
xcrun simctl boot "$DEV" 2>/dev/null || true
xcrun simctl bootstatus "$DEV" -b
touch "$OUT/.start"

xcrun simctl install "$DEV" "$APP"
xcrun simctl spawn "$DEV" log stream --level debug --predicate 'process == "jcode-mobile"' > "$OUT/app.log" 2>&1 &
LOGPID=$!
xcrun simctl launch "$DEV" "$BUNDLE"
sleep 5
idb connect "$DEV" >/dev/null

# Screen size in points.
read -r W H < <(idb describe --udid "$DEV" --json | python3 -c '
import json,sys; d=json.load(sys.stdin)["screen_dimensions"]
print(int(d["width_points"]), int(d["height_points"]))')
echo "screen ${W}x${H}"

alive() { xcrun simctl spawn "$DEV" launchctl list 2>/dev/null | grep -q "UIKitApplication:$BUNDLE"; }

r() { echo $(( RANDOM % $1 )); }
END=$(( SECONDS + DURATION )); n=0; fail=0
while [ $SECONDS -lt $END ]; do
  x=$(( $(r $((W-20))) + 10 )); y=$(( $(r $((H-60))) + 50 ))
  case $(r 10) in
    0|1|2|3|4) idb ui tap --udid "$DEV" "$x" "$y" ;;
    5|6) idb ui swipe --udid "$DEV" --duration 0.2 "$x" "$y" "$(( $(r $((W-20))) + 10 ))" "$(( $(r $((H-60))) + 50 ))" ;;
    7) idb ui swipe --udid "$DEV" --duration 0.15 5 "$y" "$(( W * 3 / 4 ))" "$y" ;;   # edge drag (drawer)
    8) idb ui swipe --udid "$DEV" --duration 0.8 "$x" "$y" "$x" "$(( $(r $((H-60))) + 50 ))" ;; # slow drag/scroll
    9) idb ui text --udid "$DEV" "monkey $(r 1000)" ;;
  esac >/dev/null 2>&1
  n=$((n+1))
  if [ $((n % 20)) -eq 0 ]; then
    if ! alive; then echo "app not running after $n events"; fail=1; break; fi
    echo "$n events, alive"
  fi
done
alive || { echo "app not running at end"; fail=1; }
xcrun simctl io "$DEV" screenshot "$OUT/final.png" >/dev/null 2>&1 || true
kill $LOGPID 2>/dev/null

# Crash reports newer than the run start (host + simulator report dirs).
for d in "$REPORTS" "$HOME/Library/Logs/CoreSimulator"; do
  find "$d" -newer "$OUT/.start" \( -name 'jcode-mobile*' -o -name '*.ips' -o -name '*.crash' \) 2>/dev/null \
    | while read -r f; do grep -qs jcode-mobile "$f" && cp "$f" "$OUT/"; done
done
if ls "$OUT"/*.ips "$OUT"/*.crash >/dev/null 2>&1; then echo "crash reports found:"; ls "$OUT"; fail=1; fi
echo "monkey: $n events in ${DURATION}s, fail=$fail"
exit $fail
