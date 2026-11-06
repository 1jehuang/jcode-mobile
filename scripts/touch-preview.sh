#!/usr/bin/env bash
# Native Wayland phone preview. No prompts, compositor startup, or login changes.
set -euo pipefail
cd "$(dirname "$0")/.."

usage() {
  cat <<'HELP'
Usage: scripts/touch-preview.sh [--dry-run] [justrust run options...]

Requires Linux, an existing Wayland session, and justrust on PATH.
Uses ~/gpui by default. Set GPUI_DIR=/path/to/gpui to choose a checkout,
or GPUI_LOCAL=0 to use the pinned dependencies instead.
--dry-run validates prerequisites and prints the command without running it.
Other arguments are passed to justrust run (for example --release).
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
if [[ "${GPUI_LOCAL:-1}" != 0 ]]; then
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
COMMAND=(justrust run --package jcode-mobile --bin jcode-mobile "${PATCH[@]}" "$@")
if (( DRY_RUN )); then
  printf 'unset DISPLAY\n'
  printf '%q ' "${COMMAND[@]}"
  printf '\n'
else
  exec "${COMMAND[@]}"
fi
