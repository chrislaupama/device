---
name: spec-write
description: Start spec-driven work on Device by writing specs/NNN-slug/spec.md (problem, behaviour, requirements, acceptance checks, out of scope, open questions). Use when someone asks for a new feature, a behaviour change, cross-layer or performance work, or a format or C ABI change, before any code is written.
---

# Write a spec

The spec says what we are building and why, in terms the maintainer can agree to. It does not say
how; that is `spec-plan`. Read `specs/README.md` and `specs/principles.md` first, and the finished
example in `specs/001-agent-skills-and-specs/`.

## 1. Decide whether this needs a spec

Typos, doc edits, obvious small fixes and behaviour-free refactors don't: make the change. Bugs
start with `bug-fix`; when `bug-fix` sends a bug here because its cause or fix is not obvious, carry
on below. Everything else gets a spec. If unsure, ask, or write a short one.

## 2. Learn the area before writing

Read enough code to describe today's behaviour accurately and to know which layers are involved:
`docs/architecture.md` for the overall shape, `docs/devices.md` for built-in devices, the ADRs in
`docs/adr/` for decisions already made, and the relevant section of `PROGRESS.md`. Run the app
(`cargo run --release -p dstudio-app -- --demo`) or render a review state if the change is visible.
A spec built on a wrong picture of today wastes everyone's review.

## 3. Create the folder

Take the next free number: `ls specs | grep -E '^[0-9]{3}-' | sort | tail -1`. Create
`specs/<NNN>-<short-slug>/` and copy `specs/_template/spec.md` into it.

## 4. Fill it in

- **Problem**: what is missing or wrong, for whom. No solution words.
- **Behaviour**: what the user sees and does, step by step, using the app's real names for menus,
  panels and controls. Include the undo label, what is saved with the project, and what happens
  with old projects.
- **Requirements**: numbered (R1, R2...), each testable on its own. Cover edge cases the user can
  reach: empty selection, very long or very short clips, sample-rate changes, missing media, undo
  in the middle of a gesture.
- **Acceptance checks**: numbered (A1, A2...), each naming the requirements it covers and how it is
  checked (a core or engine test, a self-test step, a review-state screenshot, a benchmark number).
  Every requirement is covered by at least one check.
- **Out of scope**: what a reader might expect that this deliberately leaves out.
- **Open questions**: every decision that belongs to the maintainer. Don't guess these; list them,
  with your recommendation for each.

Write in plain sentences. Describe behaviour in our own words: a product we study for reference is
a source of facts, never of text, art or code (principle 6).

## 5. Stop for agreement

Set `Status: Draft`, show the maintainer the spec and the open questions, and wait. When the
questions are answered, fold the answers into the spec, empty the Open questions section, set
`Status: Agreed`, and move on to `spec-plan`. Don't plan or write code before then, unless you were
told to run the whole workflow without stopping.
