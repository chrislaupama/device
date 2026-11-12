---
name: bug-fix
description: Fix a Device bug the project's way - reproduce it, write a small failing test, find the root cause, fix it, prove the test now passes. Use for any bug report or broken feature; escalates to a spec when the cause or fix is not obvious.
---

# Fix a bug

## 1. Reproduce

Turn the report into exact steps, on `origin/main`. Use the cheapest path that shows it: a core
test, an engine test, the app with `--demo` or a `--state` review state, or the self-test. Note the
platform; Linux GCC and Windows MSVC catch mistakes macOS clang lets through. If you can't
reproduce it, say so with what you tried, rather than guessing a fix.

## 2. Pin it with a failing test

Write the smallest test that fails because of the bug, at the lowest layer that shows it:

- model, command or persistence logic: `crates/dstudio-core/tests/`
- engine behaviour or DSP: `engine/tests/` (devices: `engine/tests/devices/`, with `DeviceRig`)
- Rust and engine together: `crates/dstudio-engine/tests/`
- a UI flow: a step in the app self-test (`crates/dstudio-app/src/selftest.rs`)

Run it and watch it fail for the right reason. A few sharp tests beat many shallow ones; add the
neighbouring cases the same root cause could break, and stop there.

## 3. Find the cause

Fix the cause, not the symptom. If the cause turns out to be a design problem, or the fix crosses
layers or changes behaviour users rely on, stop and write a spec (`spec-write`) instead; put what
you learned in its Problem section.

## 4. Fix and prove

Make the fix, run the new test (it passes), then the fast checks for the touched area and the full
gate from `spec-verify` § 3 before the pull request. If the bug was a performance regression, show
the benchmark before and after (`dstudio-bench`), and add a benchmark row if none caught it.

## 5. Pull request

Follow `dstudio-pr`. The description says how to reproduce the bug, what the cause was, and which
test now guards it. Visible bugs get before and after screenshots.
