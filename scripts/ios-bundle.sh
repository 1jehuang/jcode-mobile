#!/usr/bin/env bash
# Build jcode-mobile for a real iPhone from Linux and package an .ipa.
# Usage: scripts/ios-bundle.sh [--install]
set -euo pipefail
cd "$(dirname "$0")/.."

SDK="$HOME/.swiftpm/swift-sdks/darwin.artifactbundle/Developer/Platforms/iPhoneOS.platform/Developer/SDKs/iPhoneOS.sdk"
export RUSTUP_TOOLCHAIN=1.98.1 SDKROOT="$SDK"
export CARGO_TARGET_AARCH64_APPLE_IOS_LINKER=ios-clang CC_aarch64_apple_ios=ios-clang AR_aarch64_apple_ios=llvm-ar

cargo build --target aarch64-apple-ios --release

OUT=target/ios
APP="$OUT/Payload/JcodeMobile.app"
rm -rf "$OUT" && mkdir -p "$APP"
cp target/aarch64-apple-ios/release/jcode-mobile "$APP/jcode-mobile"
[ -d assets ] && cp -r assets "$APP/" || true

cat > "$APP/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleIdentifier</key><string>sh.jcode.mobile.rust</string>
  <key>CFBundleName</key><string>jcode (Rust)</string>
  <key>CFBundleDisplayName</key><string>jcode rs</string>
  <key>CFBundleExecutable</key><string>jcode-mobile</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>0.1.0</string>
  <key>CFBundleVersion</key><string>1</string>
  <key>CFBundleSupportedPlatforms</key><array><string>iPhoneOS</string></array>
  <key>MinimumOSVersion</key><string>15.0</string>
  <key>UIDeviceFamily</key><array><integer>1</integer></array>
  <key>UIRequiredDeviceCapabilities</key><array><string>arm64</string><string>metal</string></array>
  <key>UILaunchScreen</key><dict/>
  <key>UIStatusBarStyle</key><string>UIStatusBarStyleLightContent</string>
  <key>UIViewControllerBasedStatusBarAppearance</key><true/>
  <key>UISupportedInterfaceOrientations</key><array><string>UIInterfaceOrientationPortrait</string></array>
  <key>ITSAppUsesNonExemptEncryption</key><false/>
</dict></plist>
PLIST

(cd "$OUT" && zip -qr JcodeMobile.ipa Payload)
echo "built $OUT/JcodeMobile.ipa"

if [ "${1:-}" = "--install" ]; then
  xtool install "$OUT/JcodeMobile.ipa"
fi
