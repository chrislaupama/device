#!/usr/bin/env bash
# Checks tools/ci/changes.sh: which CI parts each kind of change runs.
set -euo pipefail
cd "$(dirname "$0")"

fails=0
check() {
    local want=$1
    shift
    local got
    got=$(printf '%s\n' "$@" | ./changes.sh | tr '\n' ' ')
    if [ "$got" != "$want " ]; then
        echo "FAIL: $* -> $got (want $want)"
        fails=$((fails + 1))
    fi
}

all="engine=true rust=true app=true"
none="engine=false rust=false app=false"

check "$none" README.md docs/building.md specs/003-midi/spec.md .agents/skills/x/SKILL.md LICENSE
check "$none" tools/perf/bench_gate.py tools/release/make-dmg.sh .github/workflows/release.yml .github/workflows/security.yml ui-spike/src/main.rs
check "engine=true rust=false app=false" engine/tests/test_pdc.cpp engine/bench/bench_main.cpp
check "$all" engine/src/engine.cpp
check "$all" engine/include/dstudio_engine.h README.md
check "$all" engine/CMakeLists.txt
check "engine=false rust=true app=false" crates/dstudio-core/tests/core.rs crates/dstudio-engine/tests/pdc.rs
check "engine=false rust=true app=true" crates/dstudio-app/src/main.rs
check "engine=false rust=true app=true" crates/dstudio-app/ui/main.slint docs/x.md
check "engine=false rust=true app=true" crates/dstudio-engine/build.rs
# Build configuration, dependencies and CI itself run everything.
check "$all" Cargo.lock
check "$all" Cargo.toml
check "$all" .github/workflows/ci.yml
check "$all" tools/ci/headless-pipewire.sh
check "$all" some-new-folder/file.txt
# An empty list (a failed diff) runs everything.
check "$all" ""

if [ "$fails" -ne 0 ]; then
    echo "$fails check(s) failed"
    exit 1
fi
echo "changes.sh: all checks passed"
