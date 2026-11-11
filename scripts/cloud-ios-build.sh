#!/usr/bin/env bash
# Run ON the managed Linux VM. Builds and signs an IPA, never uploads it.
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT="$(pwd -P)"
STATE="$HOME/.local/share/jcode-mobile-build"
SDK="$HOME/.swiftpm/swift-sdks/darwin.artifactbundle"
SWIFT="$HOME/.local/share/swiftly/toolchains/6.4.1"
SIGN="$HOME/.appstoreconnect/signing"

for file in "$SDK/swift-sdk.json" "$SWIFT/usr/bin/swift" \
  "$HOME/.rustup/toolchains/1.98.1-x86_64-unknown-linux-gnu/bin/rustc" \
  "$STATE/bin/justrust" "$STATE/bin/rustup" "$STATE/bin/ios-clang" "$STATE/bin/rcodesign" \
  "$SIGN/dist.p12" "$SIGN/appstore.mobileprovision" \
  "$SIGN/live.mobileprovision" "$SIGN/share.mobileprovision"; do
  test -e "$file" || { printf 'Missing prerequisite: %s\n' "$file" >&2; exit 1; }
done
mkdir -p "$STATE/cargo" "$STATE/state/.appstoreconnect/builds"
# Path dependency `../jcode` resolves to /jcode inside the container.
JCODE_SRC="${JCODE_SRC:-$(cd "$ROOT/../jcode" 2>/dev/null && pwd -P)}"
test -f "$JCODE_SRC/crates/jcode-voice/Cargo.toml" || { printf 'Missing jcode sibling: %s\n' "$JCODE_SRC" >&2; exit 1; }
# Send only the Dockerfile as build context, not the repository or credentials.
sudo -n docker build -t jcode-mobile-ios:ubuntu24 - < scripts/cloud-ios.Dockerfile
sudo -n docker run --rm --user "$(id -u):$(id -g)" \
  -e "BUILD_NUMBER=${BUILD_NUMBER:-$(date +%s)}" \
  -v "$ROOT:/workspace" \
  -v "$JCODE_SRC:/jcode:ro" \
  -v "$STATE/state:/build-home" \
  -v "$STATE/bin:/build-home/.local/bin:ro" \
  -v "$STATE/cargo:/build-home/.cargo" \
  -v "$HOME/.rustup:/build-home/.rustup:ro" \
  -v "$SDK:/build-home/.swiftpm/swift-sdks/darwin.artifactbundle:ro" \
  -v "$SWIFT:/build-home/.local/share/swiftly/toolchains/6.4.1:ro" \
  -v "$SIGN:/build-home/.appstoreconnect/signing:ro" \
  jcode-mobile-ios:ubuntu24 "$@"
