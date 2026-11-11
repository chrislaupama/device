#!/usr/bin/env bash
# Decides which parts of CI a pull request needs, from the paths it changes.
#
#   git diff --name-only HEAD^1 HEAD | tools/ci/changes.sh
#
# Prints three lines, each `true` or `false`:
#
#   engine=  the standalone engine build (CMake, warnings as errors) and ctest
#   rust=    `cargo test` over the workspace (which also builds the engine through build.rs)
#   app=     the app's release build and its smoke tests (screenshot, self-test)
#
# A part is skipped only when every changed path is known not to reach it. Anything not
# listed below (Cargo.toml, Cargo.lock, the CI workflow, tools/ci/, a new top-level folder)
# runs everything. Pushes to main and scheduled runs do not call this: they run everything.
#
#   documentation (*.md, docs/, specs/, .agents/, LICENSE,
#     issue templates, dependabot.yml)              nothing
#   engine/tests/, engine/bench/, engine/tools/     engine     (build.rs builds none of them)
#   engine/ (anything else)                         engine, rust, app
#   crates/*/tests/, crates/*/benches/              rust       (not part of the app binary)
#   crates/ (anything else)                         rust, app
#   tools/perf/, tools/release/, tools/art/,
#     tools/agents/, tools/macos-bundle.sh,
#     .github/workflows/{perf,release}.yml,
#     ui-spike/                                     nothing    (their own workflows, or no CI)
#
# Test it with tools/ci/test-changes.sh.
set -euo pipefail

engine=false
rust=false
app=false
any=false
while IFS= read -r path || [ -n "$path" ]; do
    path="${path%$'\r'}"
    [ -n "$path" ] || continue
    any=true
    case "$path" in
        *.md | docs/* | specs/* | .agents/* | LICENSE* | .github/ISSUE_TEMPLATE/* | .github/dependabot.yml) ;;
        engine/tests/* | engine/bench/* | engine/tools/*) engine=true ;;
        engine/*) engine=true rust=true app=true ;;
        crates/*/tests/* | crates/*/benches/*) rust=true ;;
        crates/*) rust=true app=true ;;
        tools/perf/* | tools/release/* | tools/art/* | tools/agents/* | tools/macos-bundle.sh) ;;
        .github/workflows/perf.yml | .github/workflows/release.yml | ui-spike/*) ;;
        *) engine=true rust=true app=true ;;
    esac
done

# No list at all (the diff failed or was empty): run everything rather than nothing.
if [ "$any" = false ]; then
    engine=true rust=true app=true
fi

echo "engine=$engine"
echo "rust=$rust"
echo "app=$app"
