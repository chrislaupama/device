#!/usr/bin/env bash
# Signs "Device.app" and wraps it in a .dmg with an Applications link; notarises and
# staples the .dmg when credentials are given.
#
# Usage: tools/release/make-dmg.sh <app> <version> <out dir>
#
# Signing: with MACOS_SIGN_IDENTITY set (a "Developer ID Application" identity in the
# keychain) the app and the plugin scanner are signed with the hardened runtime and
# tools/release/macos-entitlements.plist. With APPLE_API_KEY_PATH (an App Store Connect API
# key, .p8), APPLE_API_KEY_ID and APPLE_API_ISSUER also set, the .dmg is notarised and
# stapled. Without an identity everything is signed ad hoc: the app runs, but Gatekeeper asks
# the user to allow it the first time (right-click > Open, or System Settings > Privacy &
# Security).
#
# REQUIRE_SIGNING=1 (set by the release workflow for tagged releases) makes a missing identity
# or notary key an error, so a release cannot quietly ship ad hoc signed.
set -euo pipefail
app="${1:?usage: make-dmg.sh <app> <version> <out dir>}"
version="${2:?usage: make-dmg.sh <app> <version> <out dir>}"
out="${3:?usage: make-dmg.sh <app> <version> <out dir>}"
if [ "${REQUIRE_SIGNING:-}" = 1 ]; then
  for v in MACOS_SIGN_IDENTITY APPLE_API_KEY_PATH APPLE_API_KEY_ID APPLE_API_ISSUER; do
    [ -n "${!v:-}" ] || { echo "REQUIRE_SIGNING=1 but $v is not set" >&2; exit 1; }
  done
fi
root="$(cd "$(dirname "$0")/../.." && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
mkdir -p "$out" "$work/dmg"
cp -R "$app" "$work/dmg/"
app="$work/dmg/$(basename "$app")"

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
dmg="$out/Device-$version-macos-universal.dmg"
rm -f "$dmg"
hdiutil create -volname "Device $version" -srcfolder "$work/dmg" -fs HFS+ -format UDZO -ov "$dmg"

if [ -n "${MACOS_SIGN_IDENTITY:-}" ]; then
  codesign --force --timestamp --sign "$MACOS_SIGN_IDENTITY" "$dmg"
  if [ -n "${APPLE_API_KEY_PATH:-}" ] && [ -n "${APPLE_API_KEY_ID:-}" ] && [ -n "${APPLE_API_ISSUER:-}" ]; then
    xcrun notarytool submit "$dmg" --key "$APPLE_API_KEY_PATH" --key-id "$APPLE_API_KEY_ID" \
      --issuer "$APPLE_API_ISSUER" --wait
    xcrun stapler staple "$dmg"
    spctl --assess --type open --context context:primary-signature --verbose=2 "$dmg"
  else
    echo "APPLE_API_KEY_PATH / APPLE_API_KEY_ID / APPLE_API_ISSUER not set: signed but not notarised"
  fi
fi
echo "$dmg"
