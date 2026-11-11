#!/usr/bin/env bash
# Packages Device Studio for macOS: one universal (Apple silicon + Intel) "Device Studio.app"
# in a .dmg with an Applications link.
#
# Usage: tools/release/package-macos.sh <version> <out dir>
#   Expects release builds of dstudio-app for both architectures:
#     cargo build --release --locked -p dstudio-app --target aarch64-apple-darwin
#     cargo build --release --locked -p dstudio-app --target x86_64-apple-darwin
#   (target dir: $CARGO_TARGET_DIR, default target/).
#
# Signing: with MACOS_SIGN_IDENTITY set (a "Developer ID Application" identity in the
# keychain) the app and the plugin scanner are signed with the hardened runtime and
# tools/release/macos-entitlements.plist; with APPLE_ID, APPLE_TEAM_ID and
# APPLE_APP_PASSWORD also set, the .dmg is notarised and stapled. Without an identity both
# are signed ad hoc: the app runs, but Gatekeeper asks the user to allow it the first time
# (right-click > Open, or System Settings > Privacy & Security).
set -euo pipefail
version="${1:?usage: package-macos.sh <version> <out dir>}"
out="${2:?usage: package-macos.sh <version> <out dir>}"
root="$(cd "$(dirname "$0")/../.." && pwd)"
target="${CARGO_TARGET_DIR:-$root/target}"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
mkdir -p "$out"

scanner_of() {  # the scanner build.rs built for one target triple
  ls -t "$target/$1"/release/build/dstudio-engine-*/out/build/dstudio_plugin_scanner 2>/dev/null | head -1
}
arm_scanner="$(scanner_of aarch64-apple-darwin)"
x86_scanner="$(scanner_of x86_64-apple-darwin)"
[ -n "$arm_scanner" ] && [ -n "$x86_scanner" ] || { echo "plugin scanner not built for both architectures" >&2; exit 1; }

lipo -create -output "$work/device-studio" \
  "$target/aarch64-apple-darwin/release/device-studio" "$target/x86_64-apple-darwin/release/device-studio"
lipo -create -output "$work/dstudio_plugin_scanner" "$arm_scanner" "$x86_scanner"
lipo -info "$work/device-studio" "$work/dstudio_plugin_scanner"

mkdir -p "$work/dmg"
DSTUDIO_APP_EXE="$work/device-studio" DSTUDIO_SCANNER_EXE="$work/dstudio_plugin_scanner" \
  bash "$root/tools/macos-bundle.sh" "$work/dmg" >/dev/null
app="$work/dmg/Device Studio.app"
# The bundle script reads the version from Cargo.toml; the tag was checked against it.
/usr/libexec/PlistBuddy -c "Print :CFBundleShortVersionString" "$app/Contents/Info.plist"

if [ -n "${MACOS_SIGN_IDENTITY:-}" ]; then
  sign=(codesign --force --timestamp --options runtime --entitlements "$root/tools/release/macos-entitlements.plist"
        --sign "$MACOS_SIGN_IDENTITY")
else
  echo "MACOS_SIGN_IDENTITY not set: signing ad hoc (not notarised)"
  sign=(codesign --force --sign -)
fi
# Inside out: the nested executable first, then the bundle (which seals it).
"${sign[@]}" "$app/Contents/MacOS/dstudio_plugin_scanner"
"${sign[@]}" "$app"
codesign --verify --strict --verbose=2 "$app"

ln -s /Applications "$work/dmg/Applications"
dmg="$out/DeviceStudio-$version-macos-universal.dmg"
rm -f "$dmg"
hdiutil create -volname "Device Studio $version" -srcfolder "$work/dmg" -fs HFS+ -format UDZO -ov "$dmg"

if [ -n "${MACOS_SIGN_IDENTITY:-}" ]; then
  codesign --force --timestamp --sign "$MACOS_SIGN_IDENTITY" "$dmg"
  if [ -n "${APPLE_ID:-}" ] && [ -n "${APPLE_TEAM_ID:-}" ] && [ -n "${APPLE_APP_PASSWORD:-}" ]; then
    xcrun notarytool submit "$dmg" --apple-id "$APPLE_ID" --team-id "$APPLE_TEAM_ID" \
      --password "$APPLE_APP_PASSWORD" --wait
    xcrun stapler staple "$dmg"
  else
    echo "APPLE_ID / APPLE_TEAM_ID / APPLE_APP_PASSWORD not set: signed but not notarised"
  fi
fi
echo "$dmg"
