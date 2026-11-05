#!/usr/bin/env bash
# Build jcode-mobile for a real iPhone from Linux and package an .ipa.
# Usage: scripts/ios-bundle.sh [--install]
set -euo pipefail
cd "$(dirname "$0")/.."

SDK="$HOME/.swiftpm/swift-sdks/darwin.artifactbundle/Developer/Platforms/iPhoneOS.platform/Developer/SDKs/iPhoneOS.sdk"
export RUSTUP_TOOLCHAIN=1.98.1 SDKROOT="$SDK"
export CARGO_TARGET_AARCH64_APPLE_IOS_LINKER=ios-clang CC_aarch64_apple_ios=ios-clang AR_aarch64_apple_ios=llvm-ar

# GPUI_LOCAL=1 builds against the ~/gpui checkout instead of the pinned rev.
PATCH=()
if [ -n "${GPUI_LOCAL:-}" ]; then
  G="$HOME/gpui/crates"
  for c in gpui gpui_platform gpui_ios gpui_apple gpui_macros; do
    [ -d "$G/$c" ] && PATCH+=(--config "patch.'https://github.com/1jehuang/gpui'.$c.path='$G/$c'")
  done
  # gpui-thinking-orbs pulls gpui through [patch.crates-io]; point it at the same checkout.
  PATCH+=(--config "patch.crates-io.gpui.path='$G/gpui'")
fi
cargo build --target aarch64-apple-ios --release "${PATCH[@]}"

OUT=target/ios
APP="$OUT/Payload/JcodeMobile.app"
rm -rf "$OUT" && mkdir -p "$APP"
cp target/aarch64-apple-ios/release/jcode-mobile "$APP/jcode-mobile"
[ -d assets ] && cp -r assets "$APP/" || true
cp ios-res/Assets.car ios-res/AppIcon60x60@2x.png ios-res/PrivacyInfo.xcprivacy "$APP/"

BUILD_NUMBER="${BUILD_NUMBER:-$(date +%s)}"
cat > "$APP/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleIdentifier</key><string>sh.jcode.mobile.rust</string>
  <key>CFBundleName</key><string>jcode (Rust)</string>
  <key>CFBundleDisplayName</key><string>jcode rs</string>
  <key>CFBundleExecutable</key><string>jcode-mobile</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>0.1.0</string>
  <key>CFBundleVersion</key><string>$BUILD_NUMBER</string>
  <key>CFBundleSupportedPlatforms</key><array><string>iPhoneOS</string></array>
  <key>MinimumOSVersion</key><string>15.0</string>
  <key>UIDeviceFamily</key><array><integer>1</integer><integer>2</integer></array>
  <key>UIRequiredDeviceCapabilities</key><array><string>arm64</string><string>metal</string></array>
  <key>UILaunchScreen</key><dict/>
  <key>ITSAppUsesNonExemptEncryption</key><false/>
  <key>DTPlatformName</key><string>iphoneos</string>
  <key>DTPlatformVersion</key><string>27.1</string>
  <key>DTPlatformBuild</key><string>24A94232</string>
  <key>DTSDKName</key><string>iphoneos27.1</string>
  <key>DTSDKBuild</key><string>24A94232</string>
  <key>DTXcode</key><string>2710</string>
  <key>DTXcodeBuild</key><string>27A9275</string>
  <key>DTCompiler</key><string>com.apple.compilers.llvm.clang.1_0</string>
  <key>BuildMachineOSBuild</key><string>25A354</string>
  <key>CFBundleIcons</key><dict><key>CFBundlePrimaryIcon</key><dict>
    <key>CFBundleIconFiles</key><array><string>AppIcon60x60</string></array>
    <key>CFBundleIconName</key><string>AppIcon</string>
  </dict></dict>
  <key>UIStatusBarStyle</key><string>UIStatusBarStyleDefault</string>
  <key>UIViewControllerBasedStatusBarAppearance</key><true/>
  <key>CFBundleURLTypes</key><array><dict>
    <key>CFBundleURLName</key><string>sh.jcode.mobile</string>
    <key>CFBundleURLSchemes</key><array><string>jcode</string></array>
  </dict></array>
  <key>NSSupportsLiveActivities</key><true/>
  <key>UIApplicationShortcutItems</key><array>
    <dict><key>UIApplicationShortcutItemType</key><string>jcode://new</string>
      <key>UIApplicationShortcutItemTitle</key><string>New session</string>
      <key>UIApplicationShortcutItemIconSymbolName</key><string>plus.bubble</string></dict>
  </array>
  <key>UISupportedInterfaceOrientations</key><array><string>UIInterfaceOrientationPortrait</string></array>
  <key>UISupportedInterfaceOrientations~ipad</key><array>
    <string>UIInterfaceOrientationPortrait</string><string>UIInterfaceOrientationPortraitUpsideDown</string>
    <string>UIInterfaceOrientationLandscapeLeft</string><string>UIInterfaceOrientationLandscapeRight</string>
  </array>
  <key>ITSAppUsesNonExemptEncryption</key><false/>
</dict></plist>
PLIST

# Swift extensions: Live Activity widget, share extension, ActivityKit bridge.
# NO_IOS_EXT=1 skips them (faster Rust-only iteration).
[ -z "${NO_IOS_EXT:-}" ] && scripts/ios-ext.sh "$APP" sh.jcode.mobile.rust "$BUILD_NUMBER"

SIGN="$HOME/.appstoreconnect/signing"
if [ "${1:-}" = "--testflight" ]; then
  cp "$SIGN/appstore.mobileprovision" "$APP/embedded.mobileprovision"
  cat > "$OUT/entitlements.plist" <<'ENT'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>application-identifier</key><string>TAS6ARKDN7.sh.jcode.mobile.rust</string>
  <key>com.apple.developer.team-identifier</key><string>TAS6ARKDN7</string>
  <key>get-task-allow</key><false/>
  <key>beta-reports-active</key><true/>
  <key>aps-environment</key><string>production</string>
  <key>com.apple.developer.applesignin</key><array><string>Default</string></array>
</dict></plist>
ENT
  # Extensions: own profile + entitlements (scripts/asc_extensions.py makes the profiles).
  EXT_SIGN=()
  for ext in live share; do
    name=$([ $ext = live ] && echo JcodeLive || echo JcodeShare)
    [ -d "$APP/PlugIns/$name.appex" ] || continue
    cp "$SIGN/$ext.mobileprovision" "$APP/PlugIns/$name.appex/embedded.mobileprovision"
    cat > "$OUT/$ext.entitlements.plist" <<ENT
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>application-identifier</key><string>TAS6ARKDN7.sh.jcode.mobile.rust.$ext</string>
  <key>com.apple.developer.team-identifier</key><string>TAS6ARKDN7</string>
  <key>get-task-allow</key><false/>
  <key>beta-reports-active</key><true/>
</dict></plist>
ENT
    EXT_SIGN+=(--entitlements-xml-file "PlugIns/$name.appex:$OUT/$ext.entitlements.plist")
  done
  rcodesign sign --p12-file "$SIGN/dist.p12" --p12-password jcode \
    --entitlements-xml-file "$OUT/entitlements.plist" "${EXT_SIGN[@]}" "$APP"
fi

(cd "$OUT" && python3 -c 'import shutil; shutil.make_archive("JcodeMobile", "zip", ".", "Payload")' && mv JcodeMobile.zip JcodeMobile.ipa)
echo "built $OUT/JcodeMobile.ipa"

if [ "${1:-}" = "--install" ]; then
  xtool install "$OUT/JcodeMobile.ipa"
fi
