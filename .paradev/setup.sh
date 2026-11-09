#!/usr/bin/env bash
# Paradev workspace setup: runs once in a new worktree.
# The app is a desktop binary (no dev server, no ports), so setup only fetches
# dependencies and copies the gitignored local files from the main checkout.
set -euo pipefail

ROOT="${SUPERSET_ROOT_PATH:-}"
WS="${SUPERSET_WORKSPACE_PATH:-$PWD}"
cd "$WS"

# 1. Rust dependencies (the C++ engine's CLAP / VST 3 SDKs are fetched by
#    CMake on the first cargo build).
cargo fetch --locked

# 2. Gitignored local files from the main checkout (reference screenshots and
#    the compiled mouse helper used by tools/compare and tools/reference).
if [ -n "$ROOT" ] && [ "$ROOT" != "$WS" ]; then
  for p in reference/screenshots tools/reference/bin; do
    if [ -e "$ROOT/$p" ] && [ ! -e "$WS/$p" ]; then
      mkdir -p "$(dirname "$WS/$p")"
      cp -R "$ROOT/$p" "$WS/$p"
      echo "copied $p"
    fi
  done

  # 3. Python venv for tools/compare (not relocatable, so rebuild it rather
  #    than copy). Only when the main checkout uses one. Best effort; a broken
  #    venv is removed so the next setup run retries it.
  if [ -d "$ROOT/tools/.venv" ] && command -v python3 >/dev/null \
     && ! tools/.venv/bin/python -c 'import numpy, PIL' 2>/dev/null; then
    rm -rf tools/.venv
    if python3 -m venv tools/.venv && tools/.venv/bin/pip install -q --disable-pip-version-check numpy pillow; then
      echo "created tools/.venv"
    else
      rm -rf tools/.venv
      echo "warning: could not create tools/.venv (fidelity compare tooling only)" >&2
    fi
  fi
fi
