#!/usr/bin/env bash
# Paradev workspace teardown. Setup starts no services; this only stops any
# device-studio still running from this worktree's build (e.g. one launched from
# an agent terminal rather than the Run pane). Processes are matched by their
# resolved executable path, so relative launches count and the path is never
# treated as a pattern.
WS="$(cd "${SUPERSET_WORKSPACE_PATH:-$PWD}" 2>/dev/null && pwd -P)" || exit 0
for pid in $(pgrep -x device-studio 2>/dev/null); do
  exe=$(readlink "/proc/$pid/exe" 2>/dev/null) \
    || exe=$(lsof -a -p "$pid" -d txt -Fn 2>/dev/null | sed -n 's/^n//p' | head -1)
  case "$exe" in
    "$WS"/target/*) kill "$pid" 2>/dev/null && echo "stopped device-studio ($pid)" ;;
  esac
done
exit 0
