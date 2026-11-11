#!/usr/bin/env bash
# Builds the universal (Apple silicon + Intel) "Device Studio.app" for macOS, unsigned apart
# from the linker's ad hoc signature. tools/release/make-dmg.sh signs it and wraps it in a .dmg.
#
# Usage: tools/release/package-macos.sh <out dir>   (writes "<out dir>/Device Studio.app")
#   Expects release builds of dstudio-app for both architectures:
#     cargo build --release --locked -p dstudio-app --target aarch64-apple-darwin
#     cargo build --release --locked -p dstudio-app --target x86_64-apple-darwin
#   (target dir: $CARGO_TARGET_DIR, default target/).
set -euo pipefail
out="${1:?usage: package-macos.sh <out dir>}"
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

rm -rf "$out/Device Studio.app"
DSTUDIO_APP_EXE="$work/device-studio" DSTUDIO_SCANNER_EXE="$work/dstudio_plugin_scanner" \
  bash "$root/tools/macos-bundle.sh" "$out"
