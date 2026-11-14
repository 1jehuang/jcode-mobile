#!/usr/bin/env bash
# Local-only AAB groundwork. This script does not publish or production-sign.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SDK="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-$HOME/Android/Sdk}}"
TOOLS="$SDK/build-tools/${ANDROID_BUILD_TOOLS:-35.0.0}"
OUT="$ROOT/target/android"
: "${BUNDLETOOL_JAR:?Set BUNDLETOOL_JAR to a locally installed bundletool jar}"
[[ -f "$BUNDLETOOL_JAR" ]] || { echo 'BUNDLETOOL_JAR does not exist' >&2; exit 1; }
# Reuse exactly the native libraries and service classes packaged into the APK.
ANDROID_PROFILE=release ANDROID_ABIS="${ANDROID_ABIS:-arm64-v8a x86_64}" "$ROOT/scripts/android-build.sh"
STAGE="$(mktemp -d "$OUT/bundle.XXXXXX")"
trap 'rm -rf "$STAGE"' EXIT
"$TOOLS/aapt2" convert --output-format proto -o "$STAGE/proto.apk" "$OUT/JcodeMobile.apk"
python3 "$ROOT/scripts/android-bundle.py" "$STAGE/proto.apk" "$STAGE/base.zip"
if [[ -z "${JAVA_HOME:-}" && -x "$HOME/Android/jdk-21/bin/java" ]]; then
    export JAVA_HOME="$HOME/Android/jdk-21"
fi
JAVA_BIN="${JAVA_HOME:+$JAVA_HOME/bin/}"
"${JAVA_BIN}java" -jar "$BUNDLETOOL_JAR" build-bundle --modules="$STAGE/base.zip" --output="$OUT/JcodeMobile-unsigned.aab" --overwrite
"${JAVA_BIN}java" -jar "$BUNDLETOOL_JAR" validate --bundle="$OUT/JcodeMobile-unsigned.aab"
echo "Unsigned local AAB: $OUT/JcodeMobile-unsigned.aab"
echo 'Not approved for publishing. Actual shared GPUI UI/device acceptance remains required.'
