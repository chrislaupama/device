# Specs

Non-trivial work on Device Studio is spec-driven: we agree on what to build and how before writing
the code, and the spec ships in the same pull request as the change. A spec is short. Its job is to
make the decisions visible while they are still cheap to change, and to leave a record of why the
code is the way it is.

## When to write one

Write a spec for a new feature, a change to how something behaves, work that crosses layers (core,
engine, app), performance work, a file-format or C ABI change, or a bug whose cause or fix is not
obvious. Skip it for typos, doc edits, small fixes with an obvious cause, and pure refactors with
no change in behaviour. When unsure, write the spec: a one-page spec costs less than a rewrite.

## The phases

| Phase | Writes | Gate |
|---|---|---|
| 1. Write | `spec.md`: the problem, who it is for, behaviour as the user sees it, requirements, acceptance checks, out of scope, open questions | Open questions answered, spec agreed |
| 2. Plan | `plan.md`: the design layer by layer, the principles check, risks; `tasks.md`: ordered tasks, each with how it is verified | Plan agreed |
| 3. Build | the code, tests, benchmarks and docs, ticking off `tasks.md` | Every task ticked |
| 4. Verify | a pass over every requirement and acceptance check, with the evidence for the pull request | Every check passes; spec status Done |

Agreement is the point of phases 1 and 2. An agent stops after writing `spec.md`, and again after
`plan.md`, and waits for the maintainer, unless it was told to run straight through.

If building shows the spec or plan was wrong, change them first (and say so in the pull request),
then the code. A spec that no longer matches the code is worse than none.

Agent skills cover each phase (`spec-write`, `spec-plan`, `spec-build`, `spec-verify`, and
`bug-fix` for bugs); see [AGENTS.md](../AGENTS.md). You do not need an agent: copying the templates
and filling them in by hand is the same workflow.

## Layout

```
specs/
  principles.md          the rules every plan is checked against
  _template/             spec.md, plan.md, tasks.md to copy
  001-agent-skills-and-specs/
    spec.md
    plan.md
    tasks.md
```

Folders are numbered in order of creation (`NNN-short-slug`, three digits, the next free number).
The status line at the top of `spec.md` is one of Draft, Agreed, Building, Done or Dropped. A
Dropped spec stays, with a line saying why. Extra notes (research, measurements, a format sketch)
can sit next to the three files when the plan needs them.

[001-agent-skills-and-specs](001-agent-skills-and-specs/) is a complete example: the spec for this
workflow itself.
