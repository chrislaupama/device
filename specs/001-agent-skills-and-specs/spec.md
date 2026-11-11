# Agent skills and spec-driven workflow

Status: Done · Spec: 001 · Issue: none

## Problem

Coding agents do much of the work on Device Studio. Each session rediscovers how the codebase is
changed: where a `Command` gets its inverse, which three places a built-in device must be
registered in, which review state to screenshot, which benchmark covers a change. What they learn
lives in long handoff notes or is lost, and mistakes repeat (a parameter reordered, a test that
allocates on the audio thread, a pull request without screenshots).

Larger features also start straight from code. Decisions that the maintainer would have wanted to
make (what a gesture does, what is saved, what is out of scope) are made implicitly and found in
review, when they are expensive to change.

## Behaviour

- A contributor or agent opening the repository finds `AGENTS.md` at the root. It says what the
  project is, the rules that are never bent, when to write a spec, and which skill to use for which
  kind of change.
- Codebase skills live in `.agents/skills/<name>/SKILL.md`, one folder per skill, plain Markdown
  with `name` and `description` front matter, readable by any agent that loads skills from the
  repository and by people.
- For a non-trivial change the work runs through four phases, each producing a file in
  `specs/<NNN>-<slug>/`: write the spec, plan it (plan and tasks), build it, verify it. The agent
  stops after the spec and after the plan for the maintainer's agreement.
- Bugs have their own short path: reproduce, failing test, fix, prove.
- `python3 tools/agents/check_skills.py` reports skills whose front matter is wrong or that name
  files which no longer exist.

## Requirements

- R1. Skills are written for this codebase and point at its real files, types and commands.
- R2. Skills don't duplicate long reference docs; they point at `docs/` and add the checklist.
- R3. The spec workflow has templates for spec, plan and tasks, and a principles file every plan is
  checked against.
- R4. Spec phases 1 and 2 end with an explicit stop for agreement.
- R5. Nothing in the repository names a specific agent product; the layout works for any agent.
- R6. All text is original.
- R7. Stale skill paths are detectable with one command.
- R8. The README tells contributors the workflow exists.

## Acceptance checks

- A1 (R1, R7): `python3 tools/agents/check_skills.py` passes and checks every backticked repository
  path in the skills and `AGENTS.md`; renaming a referenced file makes it fail.
- A2 (R2): `dstudio-builtin-device` defers to `docs/devices.md` for detail.
- A3 (R3, R4): `specs/_template/` has the three templates, `specs/principles.md` exists, and the
  `spec-write` and `spec-plan` skills end with a stop for agreement.
- A4 (R5): no agent product name appears in the added files or folder names.
- A5 (R8): README › Development mentions `AGENTS.md`, `specs/` and the skills.

## Out of scope

- Generated, per-agent copies of the skills or slash commands.
- Wiring the skill check into CI (it can join the CI workflow later).
- Moving existing device specs kept outside the repository into `specs/`.
- An issue-approval gate before work starts.

## Open questions

None.
