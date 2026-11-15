#!/usr/bin/env bash
# Build-only by default. Coordinator alone runs the isolated-UID OS phases.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SDK="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-$HOME/Android/Sdk}}"
export JAVA_HOME="${JAVA_HOME:-$HOME/Android/jdk-21}"
TOOLS="$SDK/build-tools/${ANDROID_BUILD_TOOLS:-35.0.0}"
PLATFORM="$SDK/platforms/android-${ANDROID_API:-36}/android.jar"
OUT="${ANDROID_AGENT_TEST_OUT:-${JCODE_SCRATCH_DIR:-$ROOT/target}/android-agent-notification-tests}"
RUN=false
SERIAL=""
if [[ $# -eq 2 && "$1" == --check-log ]]; then
    exec python3 "$ROOT/scripts/android-instrumentation-result.py" "$2"
fi
if [[ $# -gt 0 ]]; then
    [[ $# -eq 2 && "$1" == --run && "$2" == emulator-* ]] || { echo "Usage: $0 [--run emulator-SERIAL | --check-log FILE]" >&2; exit 2; }
    RUN=true; SERIAL="$2"
fi
for tool in aapt2 d8 zipalign apksigner; do [[ -x "$TOOLS/$tool" ]] || { echo "Missing $TOOLS/$tool" >&2; exit 1; }; done
[[ -f "$PLATFORM" && -x "$JAVA_HOME/bin/javac" ]] || { echo 'Android SDK and JDK required' >&2; exit 1; }
KEYSTORE="${ANDROID_KEYSTORE:-$HOME/.android/jcode-debug.keystore}"
[[ -f "$KEYSTORE" ]] || { echo 'Existing debug signing key required, no key generated' >&2; exit 1; }
mkdir -p "$OUT"
OUT="$(cd "$OUT" && pwd)"
STAGE="$(mktemp -d "$OUT/build.XXXXXX")"
TEST=org.jcode.mobile.agentnotificationtests
RUNNER="$TEST/.AgentNotificationsInstrumentation"
ORIGINAL=""
ORIGINAL_ENABLED=""
OWNS_NAMESPACE=false
run_phase() {
    local phase="$1" log="$2"
    timeout 90s "$ADB" -s "$SERIAL" shell am instrument -w -r -e phase "$phase" -e owns_test_namespace true "$RUNNER" | tr -d '\r' | tee "$log" || return $?
    python3 "$ROOT/scripts/android-instrumentation-result.py" "$log"
}
cleanup() {
    local status=$?
    trap - EXIT
    if $OWNS_NAMESPACE; then
        if ! run_phase cleanup "$OUT/cleanup.txt"; then echo 'FAIL: isolated test resource cleanup failed' >&2; status=1; fi
    fi
    if [[ -n "$ORIGINAL" ]]; then
        local action=revoke
        [[ "$ORIGINAL" == granted ]] && action=grant
        if ! timeout 15s "$ADB" -s "$SERIAL" shell pm "$action" "$TEST" android.permission.POST_NOTIFICATIONS; then
            echo 'FAIL: isolated notification permission restoration failed' >&2; status=1
        elif ! run_phase probe "$OUT/restored.txt"; then
            echo 'FAIL: isolated restoration probe failed' >&2; status=1
        elif ! grep -qx "INSTRUMENTATION_RESULT: notification_permission=$ORIGINAL" "$OUT/restored.txt" \
                || ! grep -qx "INSTRUMENTATION_RESULT: notifications_enabled=$ORIGINAL_ENABLED" "$OUT/restored.txt" \
                || ! grep -qx 'INSTRUMENTATION_RESULT: namespace_clean=true' "$OUT/restored.txt"; then
            echo 'FAIL: isolated grant/enabled state or cleanup restoration mismatch' >&2; status=1
        fi
    fi
    rm -rf "$STAGE"
    if $RUN && [[ $status -eq 0 ]]; then
        echo 'PASS: installed production Java adapter under isolated UID, real OS agent records/child dispatch, denied/granted phases, automatic owned cleanup and original TEST permission/enabled-state restoration. Not full shared GPUI, lock-screen or FCM parity.'
    fi
    exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
mkdir -p "$STAGE/classes" "$STAGE/dex"
"$TOOLS/aapt2" link --debug-mode -I "$PLATFORM" --manifest "$ROOT/android/agent-notification-tests/AndroidManifest.xml" -o "$STAGE/base.apk"
mapfile -t SOURCES < <(find "$ROOT/android/agent-notification-tests/src" -name '*.java' -print | sort)
"$JAVA_HOME/bin/javac" --release 8 -Xlint:all,-deprecation,-options -classpath "$PLATFORM" -d "$STAGE/classes" "${SOURCES[@]}"
"$JAVA_HOME/bin/jar" cf "$STAGE/classes.jar" -C "$STAGE/classes" .
"$TOOLS/d8" --min-api 33 --lib "$PLATFORM" --output "$STAGE/dex" "$STAGE/classes.jar"
cp "$STAGE/base.apk" "$STAGE/test.apk"
(cd "$STAGE/dex" && zip -q "$STAGE/test.apk" classes*.dex)
"$TOOLS/zipalign" -f 4 "$STAGE/test.apk" "$STAGE/aligned.apk"
export JCODE_ANDROID_STORE_PASS="${ANDROID_KEYSTORE_PASSWORD:-android}"
export JCODE_ANDROID_KEY_PASS="${ANDROID_KEY_PASSWORD:-$JCODE_ANDROID_STORE_PASS}"
APK="$OUT/agent-notification-tests.apk"
"$TOOLS/apksigner" sign --ks "$KEYSTORE" --ks-key-alias "${ANDROID_KEY_ALIAS:-androiddebugkey}" --ks-pass env:JCODE_ANDROID_STORE_PASS --key-pass env:JCODE_ANDROID_KEY_PASS --out "$APK" "$STAGE/aligned.apk"
"$TOOLS/apksigner" verify "$APK"
echo "BUILT ONLY, not device-tested: $APK"
if ! $RUN; then exit 0; fi
ADB="$SDK/platform-tools/adb"
[[ "$("$ADB" -s "$SERIAL" shell getprop ro.kernel.qemu | tr -d '\r')" == 1 ]] || { echo 'Refusing non-emulator device' >&2; exit 1; }
API="$("$ADB" -s "$SERIAL" shell getprop ro.build.version.sdk | tr -d '\r')"
[[ "$API" =~ ^[0-9]+$ && "$API" -ge 33 ]] || { echo 'Requires API33+' >&2; exit 1; }
"$ADB" -s "$SERIAL" shell pm path org.jcode.mobile | grep -q '^package:' || { echo 'Install current real production APK first' >&2; exit 1; }
"$ADB" -s "$SERIAL" install -r -t "$APK"
run_phase probe "$OUT/original.txt"
grep -qx 'INSTRUMENTATION_RESULT: namespace_clean=true' "$OUT/original.txt" || { echo 'Refusing pre-existing isolated test notifications/channel' >&2; exit 1; }
ORIGINAL="$(awk -F= '/^INSTRUMENTATION_RESULT: notification_permission=(granted|denied)$/ {print $2}' "$OUT/original.txt")"
ORIGINAL_ENABLED="$(awk -F= '/^INSTRUMENTATION_RESULT: notifications_enabled=(true|false)$/ {print $2}' "$OUT/original.txt")"
[[ "$ORIGINAL" == granted || "$ORIGINAL" == denied ]] && [[ "$ORIGINAL_ENABLED" == true || "$ORIGINAL_ENABLED" == false ]] || { ORIGINAL=""; echo 'Missing unique original isolated permission/enabled states' >&2; exit 1; }
OWNS_NAMESPACE=true
"$ADB" -s "$SERIAL" shell pm revoke "$TEST" android.permission.POST_NOTIFICATIONS
run_phase denied "$OUT/denied.txt"
"$ADB" -s "$SERIAL" shell pm grant "$TEST" android.permission.POST_NOTIFICATIONS
run_phase granted "$OUT/granted.txt"
echo 'Isolated phases passed. Terminal acceptance still requires automatic cleanup and original permission/enabled-state restoration in EXIT trap.'
