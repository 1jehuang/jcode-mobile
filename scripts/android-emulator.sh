#!/usr/bin/env bash
# Create/start an AVD and install the locally built Android app.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SDK="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-$HOME/Android/Sdk}}"
API="${ANDROID_EMULATOR_API:-36}"
TAG="${ANDROID_EMULATOR_TAG:-google_apis}"
ABI="${ANDROID_EMULATOR_ABI:-x86_64}"
SUFFIX=""
[[ "$TAG" != google_apis ]] && SUFFIX="-$TAG"
NAME="${ANDROID_AVD:-jcode-api-$API$SUFFIX}"
ADB="$SDK/platform-tools/adb"
AVDMANAGER="$SDK/cmdline-tools/latest/bin/avdmanager"
IMAGE="${ANDROID_EMULATOR_IMAGE:-system-images;android-$API;$TAG;$ABI}"
EXPECTED_PAGE_SIZE="${ANDROID_EMULATOR_PAGE_SIZE:-}"
if [[ "$TAG" == google_apis_ps16k && -z "$EXPECTED_PAGE_SIZE" ]]; then EXPECTED_PAGE_SIZE=16384; fi
[[ -z "$EXPECTED_PAGE_SIZE" || "$EXPECTED_PAGE_SIZE" == 4096 || "$EXPECTED_PAGE_SIZE" == 16384 ]] || { echo 'ANDROID_EMULATOR_PAGE_SIZE must be 4096 or 16384' >&2; exit 1; }
if [[ -z "${JAVA_HOME:-}" && -x "$HOME/Android/jdk-21/bin/java" ]]; then export JAVA_HOME="$HOME/Android/jdk-21"; fi
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
        # Detach the emulator from agent/tool process groups when supported.
        # nohup alone does not protect it from a tool-session SIGTERM cleanup.
        launcher=(nohup)
        if command -v setsid >/dev/null; then launcher+=(setsid --fork); fi
        "${launcher[@]}" "$SDK/emulator/emulator" -avd "$NAME" -port "$port" -no-snapshot -no-audio -gpu "${ANDROID_EMULATOR_GPU:-lavapipe}" "${flags[@]}" >"$ROOT/target/android/$NAME.log" 2>&1 </dev/null &
        echo "Started $NAME ($ANDROID_SERIAL), log: target/android/$NAME.log"
    fi
fi
# adb wait-for-device alone can hang forever after an emulator launch failure.
deadline=$((SECONDS + 240))
until [[ "$("$ADB" -s "$ANDROID_SERIAL" shell getprop sys.boot_completed 2>/dev/null | tr -d '\r')" == 1 ]]; do
    (( SECONDS < deadline )) || { echo 'Emulator did not boot within 240 seconds. Inspect the emulator log.' >&2; exit 1; }
    sleep 2
done
# Verify the runtime selected by this helper, not just a similarly named AVD.
actual_api="$("$ADB" -s "$ANDROID_SERIAL" shell getprop ro.build.version.sdk | tr -d '\r')"
[[ "$actual_api" == "$API" ]] || { echo "Expected Android API $API, device runs API $actual_api. Set ANDROID_EMULATOR_API explicitly." >&2; exit 1; }
if [[ -n "$EXPECTED_PAGE_SIZE" ]]; then
    actual_page_size="$("$ADB" -s "$ANDROID_SERIAL" shell getconf PAGE_SIZE | tr -d '\r')"
    [[ "$actual_page_size" == "$EXPECTED_PAGE_SIZE" ]] || { echo "Expected $EXPECTED_PAGE_SIZE-byte Android pages, got $actual_page_size" >&2; exit 1; }
fi
"$ADB" -s "$ANDROID_SERIAL" shell input keyevent KEYCODE_WAKEUP
"$ADB" -s "$ANDROID_SERIAL" shell wm dismiss-keyguard
"$ADB" -s "$ANDROID_SERIAL" install -r "$APK"
"$ADB" -s "$ANDROID_SERIAL" shell am start -W -n org.jcode.mobile/.JcodeActivity
echo "Running on $ANDROID_SERIAL. Smoke test: ANDROID_SERIAL=$ANDROID_SERIAL python3 scripts/android-smoke.py"
