#!/usr/bin/env bash
# Wraps the release build in "Saine Studio.app", so macOS names the app menu, About, Hide and
# Quit "Saine Studio" instead of the executable's name.
# Usage: tools/macos-bundle.sh [output dir]   (default: $CARGO_TARGET_DIR/release or target/release)
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
release="${CARGO_TARGET_DIR:-$root/target}/release"
out="${1:-$release}"
exe="$release/saine-studio"
[ -x "$exe" ] || { echo "no $exe: run cargo build --release -p saine-app first" >&2; exit 1; }
version="$(sed -n 's/^version = "\(.*\)"/\1/p' "$root/Cargo.toml" | head -1)"
app="$out/Saine Studio.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS"
cp "$exe" "$app/Contents/MacOS/saine-studio"
cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>Saine Studio</string>
    <key>CFBundleDisplayName</key><string>Saine Studio</string>
    <key>CFBundleIdentifier</key><string>com.saine.studio</string>
    <key>CFBundleExecutable</key><string>saine-studio</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>$version</string>
    <key>CFBundleVersion</key><string>$version</string>
    <key>LSMinimumSystemVersion</key><string>13.0</string>
    <key>NSHighResolutionCapable</key><true/>
    <key>NSMicrophoneUsageDescription</key><string>Saine Studio records from your audio inputs.</string>
</dict>
</plist>
PLIST
echo "$app"
