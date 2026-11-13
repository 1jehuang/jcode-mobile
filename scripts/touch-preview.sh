#!/usr/bin/env bash
# Native Wayland phone preview. No prompts, compositor startup, or login changes.
set -euo pipefail
cd "$(dirname "$0")/.."

usage() {
  cat <<'HELP'
Usage: scripts/touch-preview.sh [--dry-run] [justrust build options...] [-- app arguments...]

Requires Linux, an existing Wayland session, and justrust on PATH.
Uses the pinned GPUI dependencies by default. Set GPUI_LOCAL=1 to use
~/gpui, or GPUI_DIR=/path/to/gpui to choose a checkout with GPUI_LOCAL=1.
--dry-run validates prerequisites and prints the command without running it.
Other arguments are passed to justrust build (release is the default).
Arguments after -- are passed to the app. Hosted builds work on battery.
See docs/TOUCH_PREVIEW.md for touch limitations and preview settings.
HELP
}
fail() { printf 'touch-preview: %s\n' "$*" >&2; exit 1; }

DRY_RUN=0
case "${1:-}" in
  --help|-h) usage; exit 0 ;;
  --dry-run) DRY_RUN=1; shift ;;
esac
[[ "$(uname -s)" == Linux ]] || fail 'this launcher requires Linux.'
[[ -n "${WAYLAND_DISPLAY:-}" ]] || fail 'WAYLAND_DISPLAY is unset. Run from a Wayland desktop session.'
[[ -z "${ZED_HEADLESS+x}" ]] || fail 'unset ZED_HEADLESS to enable the native Wayland backend.'
case "$WAYLAND_DISPLAY" in
  /*) SOCKET="$WAYLAND_DISPLAY" ;;
  *) [[ -n "${XDG_RUNTIME_DIR:-}" ]] || fail 'XDG_RUNTIME_DIR is required for a relative WAYLAND_DISPLAY.'
     SOCKET="$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY" ;;
esac
[[ -S "$SOCKET" ]] || fail "Wayland socket not found: $SOCKET"
command -v justrust >/dev/null 2>&1 || fail 'justrust is not on PATH.'

PATCH=()
if [[ "${GPUI_LOCAL:-0}" != 0 ]]; then
  G="${GPUI_DIR:-$HOME/gpui}"
  [[ -d "$G" ]] || fail "GPUI checkout not found: $G (set GPUI_DIR or GPUI_LOCAL=0)."
  G="$(cd "$G" && pwd -P)"
  [[ "$G" != *"'"* ]] || fail "GPUI checkout path cannot contain an apostrophe (Cargo TOML override)."
  for c in gpui gpui_platform gpui_linux gpui_wgpu gpui_macros gpui_shared_string gpui_util; do
    [[ -f "$G/crates/$c/Cargo.toml" ]] || fail "missing local crate: $G/crates/$c"
  done
  # Patch the whole GPUI family, including other-target crates Cargo resolves.
  # Shared string/util and platform crates must use the same GPUI type identity.
  for manifest in "$G"/crates/gpui/Cargo.toml "$G"/crates/gpui_*/Cargo.toml; do
    [[ -f "$manifest" ]] || continue
    crate_dir="${manifest%/Cargo.toml}"
    c="${crate_dir##*/}"
    PATCH+=(--config "patch.'https://github.com/1jehuang/gpui'.$c.path='$crate_dir'")
  done
  # gpui-thinking-orbs uses crates.io GPUI. Keep it on this same instance.
  PATCH+=(--config "patch.crates-io.gpui.path='$G/crates/gpui'")
  # The app also patches crates.io cgl to the GPUI repository.
  [[ -f "$G/vendor/cgl/Cargo.toml" ]] || fail "missing local crate: $G/vendor/cgl"
  PATCH+=(--config "patch.'https://github.com/1jehuang/gpui'.cgl.path='$G/vendor/cgl'")
  PATCH+=(--config "patch.crates-io.cgl.path='$G/vendor/cgl'")
fi

# WAYLAND_DISPLAY takes precedence in GPUI. Remove X11 fallback explicitly.
unset DISPLAY
# `run` always compiles locally, which fails under the battery policy.
# Release builds can run hosted and download the executable instead.
BUILD_ARGS=()
APP_ARGS=()
TARGET_DIR="${CARGO_TARGET_DIR:-$PWD/target}"
while (( $# )); do
  case "$1" in
    --) shift; APP_ARGS=("$@"); break ;;
    --release) shift ;;
    --target-dir)
      (( $# >= 2 )) || fail 'missing value for --target-dir'
      TARGET_DIR="$2"; BUILD_ARGS+=("$1" "$2"); shift 2 ;;
    --target-dir=*) TARGET_DIR="${1#*=}"; BUILD_ARGS+=("$1"); shift ;;
    --profile|--profile=*|--target|--target=*)
      fail 'the native preview requires the host release profile' ;;
    *) BUILD_ARGS+=("$1"); shift ;;
  esac
done
BINARY="$TARGET_DIR/release/jcode-mobile"
COMMAND=(justrust build --release --package jcode-mobile --bin jcode-mobile "${PATCH[@]}" "${BUILD_ARGS[@]}")
if (( DRY_RUN )); then
  printf 'unset DISPLAY\n'
  printf '%q ' "${COMMAND[@]}"
  printf '\n'
  printf '%q ' "$BINARY" "${APP_ARGS[@]}"
  printf '\n'
else
  "${COMMAND[@]}"
  [[ -x "$BINARY" ]] || fail "build succeeded but executable not found: $BINARY"
  exec "$BINARY" "${APP_ARGS[@]}"
fi
