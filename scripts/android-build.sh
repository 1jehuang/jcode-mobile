#!/usr/bin/env bash
# SDK-only Android packaging. No Android Studio or Gradle installation required.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
SDK="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-$HOME/Android/Sdk}}"
NDK="${ANDROID_NDK_HOME:-$SDK/ndk/27.2.12479018}"
TOOLS="$SDK/build-tools/${ANDROID_BUILD_TOOLS:-35.0.0}"
PLATFORM="$SDK/platforms/android-${ANDROID_API:-35}/android.jar"
OUT="$ROOT/target/android"
ABIS="${ANDROID_ABIS:-arm64-v8a x86_64}"
PROFILE="${ANDROID_PROFILE:-debug}"
# Reuse the existing SDK-local JDK when no system compiler is on PATH.
if [[ -z "${JAVA_HOME:-}" && -x "$HOME/Android/jdk-21/bin/javac" ]]; then
    export JAVA_HOME="$HOME/Android/jdk-21"
fi
JAVA_BIN="${JAVA_HOME:+$JAVA_HOME/bin/}"
[[ "$PROFILE" == debug || "$PROFILE" == release ]] || { echo 'ANDROID_PROFILE must be debug or release' >&2; exit 1; }
for tool in "${JAVA_BIN}javac" "${JAVA_BIN}jar" "${JAVA_BIN}keytool" zip justrust; do
    command -v "$tool" >/dev/null || { echo "Missing required tool: $tool" >&2; exit 1; }
done
for tool in aapt2 d8 zipalign apksigner; do
    [[ -x "$TOOLS/$tool" ]] || { echo "Missing $TOOLS/$tool. See docs/ANDROID.md" >&2; exit 1; }
done
[[ -f "$PLATFORM" ]] || { echo "Missing Android platform: $PLATFORM" >&2; exit 1; }
[[ -d "$NDK/toolchains/llvm/prebuilt/linux-x86_64/bin" ]] || { echo "Missing Linux Android NDK: $NDK" >&2; exit 1; }
mkdir -p "$OUT"
# Each packaging run gets an empty staging directory, never stale classes/ABIs.
STAGE="$(mktemp -d "$OUT/stage.XXXXXX")"
trap 'rm -rf "$STAGE"' EXIT
mkdir -p "$STAGE/classes" "$STAGE/dex" "$STAGE/apk/lib" "$STAGE/generated"
for abi in $ABIS; do
    case "$abi" in
        arm64-v8a) target=aarch64-linux-android; clang=aarch64-linux-android26-clang ;;
        x86_64) target=x86_64-linux-android; clang=x86_64-linux-android26-clang ;;
        *) echo "Unsupported ABI: $abi" >&2; exit 1 ;;
    esac
    target_env="${target^^}"
    target_env="${target_env//-/_}"
    export "CARGO_TARGET_${target_env}_LINKER=$NDK/toolchains/llvm/prebuilt/linux-x86_64/bin/$clang"
    export "CC_${target//-/_}=$NDK/toolchains/llvm/prebuilt/linux-x86_64/bin/$clang"
    export "AR_${target//-/_}=$NDK/toolchains/llvm/prebuilt/linux-x86_64/bin/llvm-ar"
    export "CARGO_TARGET_${target_env}_RUSTFLAGS=-C link-arg=-fuse-ld=lld -C link-arg=-Wl,-z,max-page-size=16384"
    args=()
    [[ "$PROFILE" == release ]] && args+=(--release)
    # Hosted workers do not have this machine's Android NDK/toolchain. The
    # native Linux checks/tests still use hosted justrust by default.
    # Host linker acceleration must not replace the NDK linker, particularly
    # on ARM64 where Android requires Cortex-A53 workaround flags.
    JUSTRUST_LINKER=off CARGO_TARGET_DIR="$ROOT/target" justrust build --local="Android cross-compilation requires the locally installed NDK" -p jcode-mobile --lib --target "$target" "${args[@]}"
    mkdir -p "$STAGE/apk/lib/$abi"
    cp "$ROOT/target/$target/$PROFILE/libjcode_mobile.so" "$STAGE/apk/lib/$abi/"
    "$NDK/toolchains/llvm/prebuilt/linux-x86_64/bin/llvm-strip" --strip-unneeded "$STAGE/apk/lib/$abi/libjcode_mobile.so"
    # The shared app must export NativeActivity glue, never the removed JNI UI adapter.
    "$NDK/toolchains/llvm/prebuilt/linux-x86_64/bin/llvm-readelf" --dyn-syms --wide "$STAGE/apk/lib/$abi/libjcode_mobile.so" > "$STAGE/symbols-$abi.txt"
    grep -Eq 'GLOBAL.*DEFAULT.*ANativeActivity_onCreate$' "$STAGE/symbols-$abi.txt" || {
        echo "Missing ANativeActivity_onCreate in shared GPUI app for $abi" >&2; exit 1;
    }
    while read -r alignment; do
        (( alignment >= 16384 )) || { echo "Native library for $abi is not 16 KiB aligned. Check RUSTFLAGS overrides." >&2; exit 1; }
    done < <("$NDK/toolchains/llvm/prebuilt/linux-x86_64/bin/llvm-readelf" --program-headers "$STAGE/apk/lib/$abi/libjcode_mobile.so" | awk '$1 == "LOAD" { print $NF }')
done
# Compile only platform service helpers. All UI is rendered by shared GPUI.
resource_args=()
if [[ -d android/app/src/main/res ]] && find android/app/src/main/res -type f -print -quit | grep -q .; then
    "$TOOLS/aapt2" compile --dir android/app/src/main/res -o "$STAGE/resources.zip"
    resource_args+=("$STAGE/resources.zip")
fi
"$TOOLS/aapt2" link -I "$PLATFORM" --manifest android/app/src/main/AndroidManifest.xml --java "$STAGE/generated" -o "$STAGE/base.apk" "${resource_args[@]}"
find android/app/src/main/java "$STAGE/generated" -name '*.java' > "$STAGE/sources.txt"
"${JAVA_BIN}javac" --release 8 -classpath "$PLATFORM" -d "$STAGE/classes" @"$STAGE/sources.txt"
"${JAVA_BIN}jar" cf "$STAGE/classes.jar" -C "$STAGE/classes" .
"$TOOLS/d8" --min-api 26 --lib "$PLATFORM" --output "$STAGE/dex" "$STAGE/classes.jar"
cp "$STAGE/base.apk" "$STAGE/unsigned.apk"
cp "$STAGE/dex/classes.dex" "$STAGE/apk/"
(cd "$STAGE/apk" && zip -q -0 -r "$STAGE/unsigned.apk" classes.dex lib)
"$TOOLS/zipalign" -P 16 -f 4 "$STAGE/unsigned.apk" "$STAGE/aligned.apk"
KEYSTORE="${ANDROID_KEYSTORE:-$HOME/.android/jcode-debug.keystore}"
if [[ -z "${ANDROID_KEYSTORE:-}" && ! -f "$KEYSTORE" ]]; then
    mkdir -p "$(dirname "$KEYSTORE")"
    "${JAVA_BIN}keytool" -genkeypair -keystore "$KEYSTORE" -storepass android -keypass android -alias androiddebugkey -dname 'CN=Android Debug,O=Android,C=US' -keyalg RSA -keysize 2048 -validity 10000 >/dev/null
    chmod 600 "$KEYSTORE"
fi
# The built-in key is development-only. Production signing must be explicit.
export JCODE_ANDROID_STORE_PASS="${ANDROID_KEYSTORE_PASSWORD:-android}"
export JCODE_ANDROID_KEY_PASS="${ANDROID_KEY_PASSWORD:-$JCODE_ANDROID_STORE_PASS}"
"$TOOLS/apksigner" sign --ks "$KEYSTORE" --ks-key-alias "${ANDROID_KEY_ALIAS:-androiddebugkey}" --ks-pass env:JCODE_ANDROID_STORE_PASS --key-pass env:JCODE_ANDROID_KEY_PASS --out "$OUT/JcodeMobile.apk" "$STAGE/aligned.apk"
"$TOOLS/apksigner" verify --verbose "$OUT/JcodeMobile.apk"
"$TOOLS/zipalign" -c -P 16 4 "$OUT/JcodeMobile.apk"
echo "APK: $OUT/JcodeMobile.apk ($ABIS, $PROFILE)"
