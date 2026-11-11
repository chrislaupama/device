# Plan: Agent skills and spec-driven workflow

Spec: [spec.md](spec.md)

## Approach

A small, repository-owned workflow rather than an external tool: four phase skills (`spec-write`,
`spec-plan`, `spec-build`, `spec-verify`) plus `bug-fix`, three Markdown templates, and a
principles file. Spec tooling packages exist that generate commands per agent; they add a
dependency, generated files for each agent and a vocabulary of their own, and this project needs
only the phases and the stops. The templates are short enough to fill by hand.

Codebase skills cover the areas where changes go wrong most often, one per layer or concern:
commands and persistence, the C ABI and audio thread, built-in devices, the UI, benchmarks, and
pull requests. Each is a checklist that names real files and points at the owning doc.

Skills live in `.agents/skills/`, a tool-neutral location, with `AGENTS.md` as the entry point. An
agent that looks elsewhere can be pointed at them locally (a symlink kept out of version control),
so the repository itself names no product.

## Changes by layer

- **Docs**: `AGENTS.md`; `specs/README.md`, `specs/principles.md`, `specs/_template/`; this spec as
  the worked example; README › Development gains a paragraph.
- **Tools**: `tools/agents/check_skills.py` (standard library only).
- No core, engine, ABI or app changes.

## Principles check

- 6 (original work): all text written for this repository.
- 8 (docs move with the code): README › Development updated; PROGRESS.md gets a status entry.
- 9 (naming): skills that are about this codebase use the `dstudio-` prefix, never bare `device`.
- The rest are not touched (no code changes).

## Tests and evidence

- Tests: the skill check run on the tree, and once with a deliberately broken path to see it fail.
- Benchmarks: no performance impact (documentation and a standalone script).
- Screenshots: nothing visible in the app.

## Risks

- Skills drift from the code. Mitigation: the path check, and `spec-build` telling agents to load
  the skill for the area they touch, so a wrong skill is noticed and fixed in the same change.
