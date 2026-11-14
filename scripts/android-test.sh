#!/usr/bin/env bash
# Legacy widget instrumentation was deleted. Capture the native GPUI app instead.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
exec python3 "$ROOT/scripts/android-smoke.py" "$@"
