#!/usr/bin/env bash
# Deterministic headless widget tests. Never launches the application binary.
set -euo pipefail
cd "$(dirname "$0")/.."

usage() {
  cat <<'HELP'
Usage: scripts/mobile-fuzz.sh [--dry-run] [--matrix] [justrust test options...] [-- test options...]

Runs the whole jcode-mobile binary test suite, headlessly and serialized.
Cargo dependency resolution is offline. Hosted justrust transport is unaffected.
--matrix runs dark/light x normal/reduced motion x 1.0/1.6 text scale,
with one independent test process per configuration. Default: dark/0/1.0.
--dry-run validates local GPUI paths and prints commands without building.
Use GPUI_DIR (default ~/gpui), or GPUI_LOCAL=0 for pinned dependencies.
Pass filters and build options such as --release directly to justrust test.
Test-thread overrides are rejected because existing tests share global state.
See docs/MOBILE_FUZZING.md for replay controls and coverage limits.
HELP
}
fail() { printf 'mobile-fuzz: %s\n' "$*" >&2; exit 1; }
DRY_RUN=0
MATRIX=0
ARGS=()
TEST_ARGS=()
IN_TEST_ARGS=0
for arg in "$@"; do
  if (( IN_TEST_ARGS )); then
    case "$arg" in
      --test-threads|--test-threads=*) fail 'test threads are fixed at 1.' ;;
    esac
    TEST_ARGS+=("$arg")
  else
    case "$arg" in
      --help|-h) usage; exit 0 ;;
      --dry-run) DRY_RUN=1 ;;
      --matrix) MATRIX=1 ;;
      --) IN_TEST_ARGS=1 ;;
      --test-threads|--test-threads=*) fail 'put test options after --; threads are fixed at 1.' ;;
      *) ARGS+=("$arg") ;;
    esac
  fi
done
[[ "$(uname -s)" == Linux ]] || fail 'this headless launcher requires Linux.'
if (( ! DRY_RUN )); then
  command -v justrust >/dev/null 2>&1 || fail 'justrust is not on PATH.'
fi

PATCH=()
if [[ "${GPUI_LOCAL:-1}" != 0 ]]; then
  G="${GPUI_DIR:-$HOME/gpui}"
  [[ -d "$G" ]] || fail "GPUI checkout not found: $G (set GPUI_DIR or GPUI_LOCAL=0)."
  G="$(cd "$G" && pwd -P)"
  [[ "$G" != *"'"* && "$G" != *$'\n'* && "$G" != *$'\r'* ]] || fail 'GPUI path cannot contain apostrophes or newlines.'
  for c in gpui gpui_platform gpui_linux gpui_wgpu gpui_macros gpui_shared_string gpui_util; do
    [[ -f "$G/crates/$c/Cargo.toml" ]] || fail "missing local crate: $G/crates/$c"
  done
  # Match touch-preview's complete type-identity patch family, including
  # other-target crates Cargo resolves and crates.io consumers such as orbs.
  for manifest in "$G"/crates/gpui/Cargo.toml "$G"/crates/gpui_*/Cargo.toml; do
    [[ -f "$manifest" ]] || continue
    crate_dir="${manifest%/Cargo.toml}"
    c="${crate_dir##*/}"
    PATCH+=(--config "patch.'https://github.com/1jehuang/gpui'.$c.path='$crate_dir'")
  done
  PATCH+=(--config "patch.crates-io.gpui.path='$G/crates/gpui'")
  [[ -f "$G/vendor/cgl/Cargo.toml" ]] || fail "missing local crate: $G/vendor/cgl"
  PATCH+=(--config "patch.'https://github.com/1jehuang/gpui'.cgl.path='$G/vendor/cgl'")
  PATCH+=(--config "patch.crates-io.cgl.path='$G/vendor/cgl'")
fi

COMMAND=(justrust test --offline --package jcode-mobile --bin jcode-mobile "${PATCH[@]}" "${ARGS[@]}" -- "${TEST_ARGS[@]}" --test-threads=1)
run_case() {
  local appearance="$1" motion="$2" scale="$3"
  local invocation=(env -u DISPLAY -u WAYLAND_DISPLAY ZED_HEADLESS=1 RUST_TEST_THREADS=1
    "JCODE_MOBILE_APPEARANCE=$appearance" "JCODE_MOBILE_REDUCE_MOTION=$motion"
    "JCODE_MOBILE_TEXT_SCALE=$scale" "${COMMAND[@]}")
  printf '# mobile-fuzz appearance=%s reduce-motion=%s text-scale=%s\n' "$appearance" "$motion" "$scale"
  if (( DRY_RUN )); then
    printf '%q ' "${invocation[@]}"
    printf '\n'
  else
    "${invocation[@]}"
  fi
}
if (( MATRIX )); then
  for appearance in dark light; do
    for motion in 0 1; do
      for scale in 1.0 1.6; do
        run_case "$appearance" "$motion" "$scale"
      done
    done
  done
else
  run_case dark 0 1.0
fi
