---
name: spec-build
description: Implement an agreed Device Studio spec by working through its tasks.md in order, verifying and ticking each task. Use after spec-plan, when plan.md is agreed and the spec is Building.
---

# Build a spec

Input: `specs/<NNN>-<slug>/` with `spec.md` at `Status: Building`, `plan.md` and `tasks.md`.

## Loop

For each unticked task in `tasks.md`, in order:

1. Re-read the task, the parts of `plan.md` it implements and the requirements it serves.
2. Load the codebase skill for the area (`dstudio-command`, `dstudio-engine-abi`,
   `dstudio-builtin-device`, `dstudio-ui`, `dstudio-bench`) and follow it.
3. Make the change, matching the style of the code around it. Comments explain why, in plain
   sentences.
4. Run the task's verification. Also run the fast checks for what you touched:
   - Rust: `cargo test -p <crate>` (and `cargo clippy -p <crate>`).
   - Engine: `cmake --build --preset default && ctest --preset default` from `engine/`.
   - App: render the review state, or run the relevant self-test section.
5. Tick the task (`- [x]`) only when its verification passed. Commit at sensible points with
   messages that say what and why.

## When reality disagrees with the plan

Stop and fix the documents first: update `plan.md` (or `spec.md`, if behaviour changes) and
`tasks.md`, and tell the maintainer when the change is more than a detail. Never let the code
drift silently from the spec.

## Don't

- Don't weaken a test, skip a check or relax a principle to get green.
- Don't widen scope beyond the spec. Note ideas for later in the pull request instead.
- Don't leave the audio thread to chance: anything new in the callback goes under the allocation
  guard (`dstudio-engine-abi`).

When every task is ticked, continue with `spec-verify`.
