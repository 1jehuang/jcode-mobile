#!/usr/bin/env bash
# Fast device loop: build against local ~/gpui, install over USB, launch,
# wait SECS, then pull Documents/gpui.log (gpui_ios logger) + any crash.
set -euo pipefail
cd "$(dirname "$0")/.."
SECS="${SECS:-10}"
BID="${BID:-XTL-F1147F07.sh.jcode.mobile.rust}"  # xtool re-prefixes dev installs
export GPUI_IOS_NO_IDLE_LOCK=1  # dev builds keep the phone awake
GPUI_LOCAL=1 ./scripts/ios-bundle.sh >/dev/null 2>&1 || { GPUI_LOCAL=1 ./scripts/ios-bundle.sh 2>&1 | grep -E "^error" -A8 | head -60; exit 1; }
xtool install target/ios/JcodeMobile.ipa 2>&1 | grep -E "Error|Success" || true
pmd() { pymobiledevice3 "$@" 2>&1 | grep -v -E "WARNING|no-root userspace"; }
pmd apps afc --help >/dev/null 2>&1 || true
pmd developer dvt launch --kill-existing "$BID" | tail -1
sleep "$SECS"
OUT="${JCODE_SCRATCH_DIR:-/tmp}/ios-run"; mkdir -p "$OUT"
pmd apps pull "$BID" Documents/gpui.log "$OUT/gpui.log" >/dev/null || true
echo "--- gpui.log"; tail -n 60 "$OUT/gpui.log" 2>/dev/null || echo "(no log)"
