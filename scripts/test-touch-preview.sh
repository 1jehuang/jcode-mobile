#!/usr/bin/env bash
# Self-contained launcher regression tests. No compositor, GPUI, or Rust build needed.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
LAUNCHER="$ROOT/scripts/touch-preview.sh"
TMP="$(mktemp -d "${TMPDIR:-/tmp}/touch-preview-test.XXXXXXXX")"
TMP="$(cd "$TMP" && pwd -P)"
SOCKET_PID=''
cleanup() {
  if [[ -n "$SOCKET_PID" ]]; then
    kill "$SOCKET_PID" 2>/dev/null || true
    wait "$SOCKET_PID" 2>/dev/null || true
  fi
  rm -rf "$TMP"
}
trap cleanup EXIT
fail() { printf 'FAIL: %s\n' "$*" >&2; exit 1; }
mkdir -p "$TMP/bin" "$TMP/runtime" "$TMP/target"
export XDG_RUNTIME_DIR="$TMP/runtime" WAYLAND_DISPLAY=wayland-test
export CARGO_TARGET_DIR="$TMP/target"
unset GPUI_LOCAL GPUI_DIR
export TEST_BUILD_LOG="$TMP/build.args" TEST_APP_LOG="$TMP/app.args"
export TEST_EVENTS="$TMP/events" TEST_APP_TEMPLATE="$TMP/fake-app"
export PATH="$TMP/bin:$PATH"
unset ZED_HEADLESS
# A real Unix-domain socket is required by the launcher's -S prerequisite check.
python3 - "$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY" <<'PY' &
import signal
import socket
import sys
sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
sock.bind(sys.argv[1])
sock.listen(1)
signal.pause()
PY
SOCKET_PID=$!
for ((attempt = 0; attempt < 100; attempt++)); do
  [[ -S "$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY" ]] && break
  kill -0 "$SOCKET_PID" 2>/dev/null || fail 'fake Wayland socket process exited'
  sleep 0.02
done
[[ -S "$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY" ]] || fail 'fake Wayland socket did not become ready'
cat > "$TMP/bin/justrust" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\0' "$@" > "$TEST_BUILD_LOG"
printf 'build\n' >> "$TEST_EVENTS"
case "${TEST_BUILD_MODE:-success}" in
  failure) exit 23 ;;
  missing) exit 0 ;;
  success)
    mkdir -p "$CARGO_TARGET_DIR/release"
    cp "$TEST_APP_TEMPLATE" "$CARGO_TARGET_DIR/release/jcode-mobile"
    chmod +x "$CARGO_TARGET_DIR/release/jcode-mobile"
    ;;
  *) exit 99 ;;
esac
SH
cat > "$TEST_APP_TEMPLATE" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\0' "$@" > "$TEST_APP_LOG"
printf 'app\n' >> "$TEST_EVENTS"
[[ -z "${DISPLAY+x}" ]] || exit 91
SH
chmod +x "$TMP/bin/justrust" "$TEST_APP_TEMPLATE"
reset_case() {
  rm -f "$TEST_BUILD_LOG" "$TEST_APP_LOG" "$TEST_EVENTS" "$CARGO_TARGET_DIR/release/jcode-mobile"
  export TEST_BUILD_MODE=success DISPLAY=:fake-x11
}
assert_build_args() {
  printf '%s\0' build --release --package jcode-mobile --bin jcode-mobile > "$TMP/expected-build"
  cmp -s "$TMP/expected-build" "$TEST_BUILD_LOG" || fail 'unexpected justrust build arguments'
}

reset_case
bash "$LAUNCHER" --dry-run -- --runtime-flag 'two words' > "$TMP/output" 2>&1 || {
  cat "$TMP/output" >&2
  fail 'dry-run failed'
}
[[ ! -e "$TEST_BUILD_LOG" && ! -e "$TEST_APP_LOG" ]] || fail 'dry-run executed build or app'
grep -q 'justrust build' "$TMP/output" || fail 'dry-run did not show build command'
printf 'PASS: dry-run does not build or launch\n'

reset_case
bash "$LAUNCHER" -- --runtime-flag 'two words' '' '--literal=value' > "$TMP/output" 2>&1 || {
  cat "$TMP/output" >&2
  fail 'successful build and launch failed'
}
assert_build_args
printf '%s\0' --runtime-flag 'two words' '' '--literal=value' > "$TMP/expected-app"
cmp -s "$TMP/expected-app" "$TEST_APP_LOG" || fail 'runtime arguments were not preserved after --'
printf 'build\napp\n' > "$TMP/expected-events"
cmp -s "$TMP/expected-events" "$TEST_EVENTS" || fail 'app did not launch after build'
printf 'PASS: builds release binary then launches with exact runtime arguments\n'

reset_case
export TEST_BUILD_MODE=failure
mkdir -p "$CARGO_TARGET_DIR/release"
cp "$TEST_APP_TEMPLATE" "$CARGO_TARGET_DIR/release/jcode-mobile"
chmod +x "$CARGO_TARGET_DIR/release/jcode-mobile"
if bash "$LAUNCHER" > "$TMP/output" 2>&1; then
  fail 'build failure unexpectedly succeeded'
fi
assert_build_args
[[ ! -e "$TEST_APP_LOG" ]] || fail 'app launched after build failure'
printf 'PASS: build failure prevents app launch\n'

reset_case
export TEST_BUILD_MODE=missing
if bash "$LAUNCHER" > "$TMP/output" 2>&1; then
  fail 'missing artifact unexpectedly succeeded'
fi
assert_build_args
[[ ! -e "$TEST_APP_LOG" ]] || fail 'app launched despite missing artifact'
grep -Fq "$CARGO_TARGET_DIR/release/jcode-mobile" "$TMP/output" || fail 'missing artifact diagnostic does not identify expected executable'
grep -Eiq 'not found|missing|not executable|did not|no executable' "$TMP/output" || fail 'missing artifact diagnostic is unclear'
printf 'PASS: missing artifact fails with a clear executable diagnostic\n'
printf 'All touch-preview launcher regression tests passed.\n'
