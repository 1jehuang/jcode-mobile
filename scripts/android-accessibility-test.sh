#!/usr/bin/env bash
# Compile service-only Java accessibility adapter and run native semantic tests.
# This never starts/stops emulators, changes accessibility settings, or claims
# TalkBack acceptance. Device/TalkBack validation belongs to the coordinator.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
GPUI="${GPUI_ANDROID_ROOT:-$ROOT/../gpui}"
SDK="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-$HOME/Android/Sdk}}"
JAVA="${JAVA_HOME:+$JAVA_HOME/bin/}javac"
PLATFORM="${ANDROID_A11Y_PLATFORM_JAR:-$SDK/platforms/android-35/android.jar}"
[[ -f "$PLATFORM" ]] || { echo "Missing Android SDK platform JAR: $PLATFORM" >&2; exit 1; }
command -v "$JAVA" >/dev/null || { echo 'Set JAVA_HOME or put javac on PATH' >&2; exit 1; }
SCRATCH="${JCODE_SCRATCH_DIR:-$ROOT/target}"
mkdir -p "$SCRATCH"
OUT="$(mktemp -d "$SCRATCH/android-accessibility-classes.XXXXXX")"
trap 'rm -rf "$OUT"' EXIT
"$JAVA" --release 8 -Xlint:all,-options -classpath "$PLATFORM" -d "$OUT" \
  "$ROOT/android/app/src/main/java/org/jcode/mobile/GpuiAccessibility.java"
echo 'Java accessibility service adapter: compiled (not device-tested)'
# Fail rather than reporting green if the backend owner has not registered tests.
python3 - "$GPUI/crates/gpui_android/src/lib.rs" <<'PY'
from pathlib import Path
import re
import sys
source = Path(sys.argv[1]).read_text()
match = re.search(r'#\[cfg\(any\(([^\]]+)\)\)\]\s*mod accessibility;', source)
if not match or not re.search(r'\btest\b', match.group(1)):
    raise SystemExit('Backend accessibility test module registration missing. Native test integration is blocked.')
PY
cd "$GPUI"
justrust test -p gpui_android --lib accessibility::tests -- --nocapture
echo 'Semantic adapter checks complete. Actual native app/TalkBack acceptance remains required.'
