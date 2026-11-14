#!/usr/bin/env bash
# Create/start an AVD and install the locally built Android app.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SDK="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-$HOME/Android/Sdk}}"
API="${ANDROID_EMULATOR_API:-35}"
NAME="${ANDROID_AVD:-jcode-api-$API}"
ADB="$SDK/platform-tools/adb"
AVDMANAGER="$SDK/cmdline-tools/latest/bin/avdmanager"
IMAGE="system-images;android-$API;google_apis;x86_64"
APK="${ANDROID_APK:-$ROOT/target/android/JcodeMobile.apk}"
[[ -x "$ADB" && -x "$SDK/emulator/emulator" ]] || { echo 'Install Android emulator/platform-tools first. See docs/ANDROID.md' >&2; exit 1; }
[[ -f "$APK" ]] || { echo 'Build the APK first: ./scripts/android-build.sh' >&2; exit 1; }
# Respect an explicitly selected emulator rather than booting another device.
if [[ -z "${ANDROID_SERIAL:-}" ]]; then
    if ! "$SDK/emulator/emulator" -list-avds | grep -Fxq "$NAME"; then
        printf 'no\n' | "$AVDMANAGER" create avd --name "$NAME" --package "$IMAGE" --device pixel_2 --force
    fi
    port="${ANDROID_EMULATOR_PORT:-5554}"
    export ANDROID_SERIAL="emulator-$port"
    if ! "$ADB" -s "$ANDROID_SERIAL" get-state >/dev/null 2>&1; then
        mkdir -p "$ROOT/target/android"
        flags=()
        [[ "${ANDROID_EMULATOR_HEADLESS:-0}" == 1 ]] && flags+=(-no-window)
        nohup "$SDK/emulator/emulator" -avd "$NAME" -port "$port" -no-snapshot -no-audio -gpu swiftshader_indirect "${flags[@]}" >"$ROOT/target/android/$NAME.log" 2>&1 </dev/null &
        echo "Started $NAME ($ANDROID_SERIAL), log: target/android/$NAME.log"
    fi
fi
# adb wait-for-device alone can hang forever after an emulator launch failure.
deadline=$((SECONDS + 240))
until [[ "$("$ADB" -s "$ANDROID_SERIAL" shell getprop sys.boot_completed 2>/dev/null | tr -d '\r')" == 1 ]]; do
    (( SECONDS < deadline )) || { echo 'Emulator did not boot within 240 seconds. Inspect the emulator log.' >&2; exit 1; }
    sleep 2
done
"$ADB" -s "$ANDROID_SERIAL" shell input keyevent KEYCODE_WAKEUP
"$ADB" -s "$ANDROID_SERIAL" shell wm dismiss-keyguard
"$ADB" -s "$ANDROID_SERIAL" install -r "$APK"
if [[ "${ANDROID_DEMO:-0}" == 1 ]]; then
    "$ADB" -s "$ANDROID_SERIAL" shell am start -W -n org.jcode.mobile/.MainActivity --es demo true
else
    "$ADB" -s "$ANDROID_SERIAL" shell am start -W -n org.jcode.mobile/.MainActivity
fi
echo "Running on $ANDROID_SERIAL. Smoke test: ANDROID_SERIAL=$ANDROID_SERIAL python3 scripts/android-smoke.py"
