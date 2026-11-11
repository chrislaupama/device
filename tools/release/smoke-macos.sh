#!/usr/bin/env bash
# Mounts a Device Studio .dmg, checks the plugin scanner is bundled and that the app starts and
# renders a screenshot. Usage: tools/release/smoke-macos.sh <dmg>
set -euo pipefail
dmg="${1:?usage: smoke-macos.sh <dmg>}"
mnt=$(hdiutil attach -nobrowse -readonly "$dmg" | sed -n 's#.*\(/Volumes/.*\)#\1#p' | tail -1)
[ -n "$mnt" ] || { echo "could not mount $dmg" >&2; exit 1; }
trap 'hdiutil detach "$mnt" >/dev/null' EXIT
bin="$mnt/Device Studio.app/Contents/MacOS"
test -x "$bin/dstudio_plugin_scanner"
shot="${RUNNER_TEMP:-/tmp}/device-studio-macos.png"
"$bin/device-studio" --state reference-default --screenshot "$shot"
test -s "$shot"
echo "ok: $shot"
