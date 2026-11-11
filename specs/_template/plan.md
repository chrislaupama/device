# Plan: <Feature name>

Spec: [spec.md](spec.md)

## Approach

<The design in a few paragraphs. Name the alternatives considered and why this one wins.>

## Changes by layer

<Leave out the layers this does not touch.>

- **Core** (`crates/dstudio-core`): model fields, new or changed `Command`s (validation, inverse,
  coalescing, label), persistence and migration, snapshot.
- **Engine** (`engine/`): render graph, devices, threading. What runs on the audio thread and how
  data gets there.
- **C ABI** (`engine/include/dstudio_engine.h`, `crates/dstudio-engine`): new functions or structs,
  layout tests.
- **App** (`crates/dstudio-app`): panels, gestures, menus, review states, self-test steps.
- **Docs**: README sections, PROGRESS.md, `docs/` pages that change.

## Principles check

<Go through [principles.md](../principles.md) and note each one this work touches and how it is
kept: real-time safety, one mutation path, projects keep loading, the ABI boundary,
cross-platform, original work, evidence, docs, naming.>

## Tests and evidence

- Tests: <the few that pin the behaviour, and where they live>
- Benchmarks: <which benchmark covers the cost, or the one to add; or "no performance impact" and why>
- Screenshots: <the `--state` review states for before and after; or "nothing visible">

## Risks

<What could go wrong, and how the plan catches it early.>
