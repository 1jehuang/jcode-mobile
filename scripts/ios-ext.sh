#!/usr/bin/env bash
# Cross-compile the Swift iOS extensions (ios-ext/) from Linux and lay them
# out inside an .app bundle:
#   Frameworks/JcodeBridge.framework  (Live Activity bridge, dlopened by Rust)
#   PlugIns/JcodeLive.appex           (WidgetKit: Live Activity + Dynamic Island)
#   PlugIns/JcodeShare.appex          (Share extension -> jcode://new?text=)
# Usage: scripts/ios-ext.sh <path/to/App.app> <app bundle id> <build number>
set -euo pipefail
cd "$(dirname "$0")/.."
APP="$1"; APP_ID="$2"; BUILD_NUMBER="$3"

(cd ios-ext && env -u SDKROOT swift build --build-system native --swift-sdk arm64-apple-ios -c release 2>&1 \
  | grep -vE "lld: warning: This version of lld|deprecated and will be removed") || exit 1
BIN=ios-ext/.build/arm64-apple-ios/release

plist_common() { # $1 bundle id, $2 executable, $3 display name, $4 package type
  cat <<PLIST
  <key>CFBundleIdentifier</key><string>$1</string>
  <key>CFBundleExecutable</key><string>$2</string>
  <key>CFBundleName</key><string>$2</string>
  <key>CFBundleDisplayName</key><string>$3</string>
  <key>CFBundlePackageType</key><string>$4</string>
  <key>CFBundleShortVersionString</key><string>1.0</string>
  <key>CFBundleVersion</key><string>$BUILD_NUMBER</string>
  <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
  <key>CFBundleDevelopmentRegion</key><string>en</string>
  <key>CFBundleSupportedPlatforms</key><array><string>iPhoneOS</string></array>
  <key>UIDeviceFamily</key><array><integer>1</integer></array>
  <key>UIRequiredDeviceCapabilities</key><array><string>arm64</string></array>
  <key>DTPlatformName</key><string>iphoneos</string>
  <key>DTPlatformVersion</key><string>27.1</string>
  <key>DTSDKName</key><string>iphoneos27.1</string>
PLIST
}
HEAD='<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>'

# Bridge framework. The dylib's install name is @rpath/libJcodeBridge.dylib;
# Rust dlopens it by path, so the install name does not matter.
FW="$APP/Frameworks/JcodeBridge.framework"
mkdir -p "$FW"
cp "$BIN/libJcodeBridge.dylib" "$FW/JcodeBridge"
{ echo "$HEAD"; plist_common "$APP_ID.bridge" JcodeBridge JcodeBridge FMWK
  echo '  <key>MinimumOSVersion</key><string>15.0</string>'; echo '</dict></plist>'; } > "$FW/Info.plist"

# Widget extension.
LIVE="$APP/PlugIns/JcodeLive.appex"
mkdir -p "$LIVE"
cp "$BIN/JcodeLive" "$LIVE/JcodeLive"
{ echo "$HEAD"; plist_common "$APP_ID.live" JcodeLive "jcode" XPC!
  cat <<'PLIST'
  <key>MinimumOSVersion</key><string>16.2</string>
  <key>NSExtension</key><dict>
    <key>NSExtensionPointIdentifier</key><string>com.apple.widgetkit-extension</string>
  </dict>
</dict></plist>
PLIST
} > "$LIVE/Info.plist"

# Share extension.
SHARE="$APP/PlugIns/JcodeShare.appex"
mkdir -p "$SHARE"
cp "$BIN/JcodeShare" "$SHARE/JcodeShare"
{ echo "$HEAD"; plist_common "$APP_ID.share" JcodeShare "jcode" XPC!
  cat <<'PLIST'
  <key>MinimumOSVersion</key><string>15.0</string>
  <key>NSExtension</key><dict>
    <key>NSExtensionPointIdentifier</key><string>com.apple.share-services</string>
    <key>NSExtensionPrincipalClass</key><string>ShareViewController</string>
    <key>NSExtensionAttributes</key><dict>
      <key>NSExtensionActivationRule</key><dict>
        <key>NSExtensionActivationSupportsText</key><true/>
        <key>NSExtensionActivationSupportsWebURLWithMaxCount</key><integer>1</integer>
        <key>NSExtensionActivationSupportsWebPageWithMaxCount</key><integer>1</integer>
      </dict>
    </dict>
  </dict>
</dict></plist>
PLIST
} > "$SHARE/Info.plist"

echo "ios-ext: bundled JcodeBridge.framework, JcodeLive.appex, JcodeShare.appex"
