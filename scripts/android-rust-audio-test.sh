#!/usr/bin/env bash
# Build-only unless the coordinator explicitly supplies an owned emulator serial.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
RUN=false
SERIAL=""
if [[ $# -gt 0 ]]; then
  [[ $# == 2 && "$1" == --run && "$2" == emulator-* ]] || { echo "Usage: $0 [--run emulator-SERIAL]" >&2; exit 2; }
  RUN=true
  SERIAL="$2"
fi
SDK="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-$HOME/Android/Sdk}}"
NDK="${ANDROID_NDK_HOME:-$SDK/ndk/27.2.12479018}"
TOOLS="$SDK/build-tools/${ANDROID_BUILD_TOOLS:-35.0.0}"
ADB="$SDK/platform-tools/adb"
READELF="$NDK/toolchains/llvm/prebuilt/linux-x86_64/bin/llvm-readelf"
OUT="${ANDROID_RUST_AUDIO_TEST_OUT:-$ROOT/target/android-rust-audio-tests}"
PACKAGE=org.jcode.mobile.rustaudiotests
HOOK=Java_org_jcode_mobile_rustaudiotests_RustAudioInstrumentation_nativeRun
bash "$ROOT/android/rust-audio-tests/build.sh"
if ! $RUN; then exit 0; fi
[[ -x "$ADB" && -x "$READELF" && -x "$TOOLS/aapt2" ]] || { echo 'Need adb, NDK readelf and aapt2' >&2; exit 1; }
mkdir -p "$OUT"
STAGE="$(mktemp -d "$OUT/preflight.XXXXXX")"
CLEANUP_PACKAGE=false
ACTIVE=""
cleanup() {
  local original_status=$? cleanup_failed=0
  trap - EXIT INT TERM
  if [[ -n "$ACTIVE" ]]; then
    # Stop the timeout supervisor, which forwards TERM to its command group.
    kill -TERM "$ACTIVE" 2>/dev/null || true
    wait "$ACTIVE" 2>/dev/null || true
  fi
  if $CLEANUP_PACKAGE; then
    timeout 10s "$ADB" -s "$SERIAL" shell am force-stop "$PACKAGE" || cleanup_failed=1
    timeout 10s "$ADB" -s "$SERIAL" shell pm revoke "$PACKAGE" android.permission.RECORD_AUDIO || cleanup_failed=1
  fi
  rm -rf "$STAGE" || cleanup_failed=1
  if [[ "$original_status" != 0 ]]; then exit "$original_status"; fi
  exit "$cleanup_failed"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
[[ "$(timeout 10s "$ADB" -s "$SERIAL" shell getprop ro.kernel.qemu | tr -d '\r')" == 1 ]] || { echo 'Refusing non-emulator' >&2; exit 1; }
# Read installed production APK only. Never install, launch or mutate its permissions.
APK_PATHS="$(timeout 10s "$ADB" -s "$SERIAL" shell pm path org.jcode.mobile | tr -d '\r')"
[[ "$APK_PATHS" == package:/* && "$APK_PATHS" != *$'\n'* ]] || { echo 'Require one installed production base APK' >&2; exit 1; }
timeout 20s "$ADB" -s "$SERIAL" pull "${APK_PATHS#package:}" "$STAGE/installed.apk" >/dev/null
"$TOOLS/aapt2" dump badging "$STAGE/installed.apk" > "$STAGE/badging.txt"
grep -Eq "^package: name='org[.]jcode[.]mobile' " "$STAGE/badging.txt" || { echo 'Wrong installed package' >&2; exit 1; }
grep -Fxq 'application-debuggable' "$STAGE/badging.txt" || { echo 'Installed production APK must be debug' >&2; exit 1; }
ABIS="$(timeout 10s "$ADB" -s "$SERIAL" shell getprop ro.product.cpu.abilist | tr -d '\r')"
python3 - "$STAGE/installed.apk" "$STAGE/library.so" "$ABIS" <<'PY'
import shutil, sys, zipfile
with zipfile.ZipFile(sys.argv[1]) as archive:
    for abi in sys.argv[3].split(','):
        if abi not in ('x86_64', 'arm64-v8a'):
            continue
        name = 'lib/' + abi + '/libjcode_mobile.so'
        if name not in archive.namelist():
            continue
        entry = archive.getinfo(name)
        if entry.file_size > 512 * 1024 * 1024:
            raise SystemExit('Native library exceeds preflight bound')
        with archive.open(entry) as source, open(sys.argv[2], 'wb') as output:
            shutil.copyfileobj(source, output)
        break
    else:
        raise SystemExit('Installed production APK lacks supported device ABI')
PY
"$READELF" --dyn-syms --wide "$STAGE/library.so" > "$STAGE/symbols.txt"
grep -Eq "GLOBAL.*DEFAULT[[:space:]]+[0-9]+[[:space:]]+$HOOK$" "$STAGE/symbols.txt" || { echo 'Installed debug APK lacks native Rust audio acceptance hook' >&2; exit 1; }
# Preflight has passed before any microphone grant or test package installation.
CLEANUP_PACKAGE=true
timeout 20s "$ADB" -s "$SERIAL" install -r -t "$OUT/rust-audio-tests.apk"
timeout 10s "$ADB" -s "$SERIAL" shell pm grant "$PACKAGE" android.permission.RECORD_AUDIO
LOG="$OUT/runtime-$SERIAL.txt"
# Background wait lets INT/TERM traps run immediately, including when only the
# runner PID is signaled. kill-after bounds cleanup of a stuck adb subprocess.
timeout --kill-after=2s 60s "$ADB" -s "$SERIAL" shell am instrument -w -r "$PACKAGE/.RustAudioInstrumentation" > "$LOG" 2>&1 &
ACTIVE=$!
STATUS=0
wait "$ACTIVE" || STATUS=$?
ACTIVE=""
cat "$LOG"
# A nonzero adb/timeout exit remains fatal even if its output contains PASS.
[[ "$STATUS" == 0 ]] || exit "$STATUS"
python3 "$ROOT/scripts/android-instrumentation-result.py" "$LOG"
echo 'PASS: isolated installed production Rust/JNI microphone scenarios. See README for remaining scope limits.'
