#!/usr/bin/env bash
# Runs one app self-test and checks its result (CI, macOS and Linux):
#
#   tools/ci/self-test.sh <name> <command...>
#   tools/ci/self-test.sh linux xvfb-run -a target/release/device-studio --self-test --no-audio
#
# The run's stdout and stderr go to selftest-<name>.txt and selftest-<name>-stderr.txt (CI
# uploads them). It passes when the app printed "self-test: P / N checks passed" with P = N
# (skipped checks are not counted in N) and exited with status 0. FAIL and SKIP lines are
# echoed either way, and the tail of stderr when it fails.
set -uo pipefail

name=$1
shift
out="selftest-$name.txt"
err="selftest-$name-stderr.txt"

start=$SECONDS
status=0
"$@" > "$out" 2> "$err" || status=$?
echo "self-test ($name): exit status $status after $((SECONDS - start)) s"
grep -E "^(FAIL|SKIP)" "$out" || true

fail() {
    echo "::error title=Self-test failed ($name)::$1"
    tail -30 "$err"
    exit 1
}

line=$(grep -E "^self-test: [0-9]+ / [0-9]+ checks passed" "$out") || fail "no self-test summary (exit status $status)"
echo "$line"
passed=$(echo "$line" | sed -E 's#^self-test: ([0-9]+) / ([0-9]+).*#\1#')
total=$(echo "$line" | sed -E 's#^self-test: ([0-9]+) / ([0-9]+).*#\2#')
[ "$passed" = "$total" ] || fail "$line"
[ "$status" -eq 0 ] || fail "every check passed, but the app exited with status $status"
