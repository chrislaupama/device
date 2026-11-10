#!/usr/bin/env bash
# Paradev workspace setup: runs once in a new worktree.
# The app is a desktop binary (no dev server, no ports), so setup only fetches
# dependencies.
set -euo pipefail

ROOT="${SUPERSET_ROOT_PATH:-}"
WS="${SUPERSET_WORKSPACE_PATH:-$PWD}"
cd "$WS"

# 1. Rust dependencies (the C++ engine's CLAP / VST 3 SDKs are fetched by
#    CMake on the first cargo build).
cargo fetch --locked
