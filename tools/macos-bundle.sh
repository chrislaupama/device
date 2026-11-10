#!/usr/bin/env bash
# Wraps the release build in "Device Studio.app", so macOS names the app menu, About, Hide and
# Quit "Device Studio" instead of the executable's name.
# Usage: tools/macos-bundle.sh [output dir]   (default: $CARGO_TARGET_DIR/release or target/release)
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
release="${CARGO_TARGET_DIR:-$root/target}/release"
out="${1:-$release}"
exe="$release/device-studio"
[ -x "$exe" ] || { echo "no $exe: run cargo build --release -p dstudio-app first" >&2; exit 1; }
version="$(sed -n 's/^version = "\(.*\)"/\1/p' "$root/Cargo.toml" | head -1)"
app="$out/Device Studio.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS"
cp "$exe" "$app/Contents/MacOS/device-studio"
cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>Device Studio</string>
    <key>CFBundleDisplayName</key><string>Device Studio</string>
    <key>CFBundleIdentifier</key><string>com.dstudio.studio</string>
    <key>CFBundleExecutable</key><string>device-studio</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>$version</string>
    <key>CFBundleVersion</key><string>$version</string>
    <key>LSMinimumSystemVersion</key><string>13.0</string>
    <key>NSHighResolutionCapable</key><true/>
    <key>NSMicrophoneUsageDescription</key><string>Device Studio records from your audio inputs.</string>
</dict>
</plist>
PLIST
echo "$app"
