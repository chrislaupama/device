---
name: spec-plan
description: Turn an agreed Device spec into plan.md (design layer by layer, principles check, tests and evidence, risks) and an ordered tasks.md. Use after spec-write, once spec.md is Agreed.
---

# Plan a spec

Input: `specs/<NNN>-<slug>/spec.md` with `Status: Agreed`. Output: `plan.md` and `tasks.md` next to
it, from `specs/_template/`.

## 1. Design layer by layer

Work top to bottom through the layers the spec touches and write down, concretely:

- **Core** (`crates/dstudio-core`): model fields; each new or changed `Command` with its validation,
  inverse, coalescing key and undo label; persistence (a schema bump and a `persist::migrate` step
  if the saved shape changes); what the snapshot sends to the engine. The `dstudio-command` skill
  has the checklist.
- **Engine** (`engine/`): what is computed off the audio thread, what runs in the callback, and how
  data crosses (snapshot publish, transport queue, lock-free parameter write, readouts, analysis
  frames).
- **C ABI**: new functions or structs in `engine/include/dstudio_engine.h`, their mirror in
  `crates/dstudio-engine/src/ffi.rs`, the layout test. See `dstudio-engine-abi`.
- **Built-in devices**: follow `dstudio-builtin-device`.
- **App** (`crates/dstudio-app`): panels and Slint components, gestures, menus, the review state that
  shows the result, self-test steps. See `dstudio-ui`.
- **Docs**: which README sections, PROGRESS.md entries and `docs/` pages change.

Name the files and types you will touch. Grep for the closest existing example and say which one
you are following: consistency with the code around it beats a cleverer design.

Note the alternatives you rejected and why, in a sentence each.

## 2. Principles check

Go through `specs/principles.md` one by one. For each principle the work touches, say how the plan
keeps it. If one can't be kept, the plan changes or the question goes to the maintainer (principle
10); it is never waived.

## 3. Tests and evidence

- Tests: the few, well-aimed tests that pin the behaviour (core tests in
  `crates/dstudio-core/tests/`, engine tests in `engine/tests/`, end-to-end in
  `crates/dstudio-engine/tests/`, UI flows in the app self-test). Don't plan a test per function.
- Benchmarks: the existing one that covers the cost, or the one to add (`dstudio-bench`). If there
  is truly no performance impact, say why.
- Screenshots: the `--state` review states for before and after, or "nothing visible".

## 4. Write tasks.md

Order tasks so the tree builds and the tests pass after each one: usually core, then ABI, then
engine, then app, then docs and evidence. Each task is small enough to review on its own and names
how it is verified (a command, a test name, a screenshot). Include the docs and evidence tasks.

## 5. Stop for agreement

Show the maintainer the plan, highlighting risky or debatable choices. When agreed, set the spec to
`Status: Building` and continue with `spec-build`.
