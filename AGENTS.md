# Working on Device Studio with a coding agent

This file is for coding agents (and the people driving them). Humans start with
[README.md](README.md) › Development, [PROGRESS.md](PROGRESS.md) and
[docs/architecture.md](docs/architecture.md); everything below points back at those.

Device Studio is a cross-platform DAW: a Rust app (`crates/dstudio-app`, Slint + wgpu) over a Rust
application core (`crates/dstudio-core`: model, commands, undo, persistence, snapshots) that drives a
C++23 real-time engine (`engine/`) through a hand-written C ABI (`engine/include/dstudio_engine.h`,
wrapped by `crates/dstudio-engine`).

## Rules that are never bent

[specs/principles.md](specs/principles.md) lists them with their sources. The short version:

- The audio thread never allocates, locks, blocks, does I/O, logs or calls Rust.
- Every edit to a project is a `dstudio_core::Command`: validated, invertible, serialisable.
- Old projects keep loading. Built-in device parameters are append-only.
- Original work only: no code, art, presets or text copied from other products.
- Internal names use `dstudio`, never bare `device` (it already names effects and instruments).
- The engine builds with warnings as errors on clang, MSVC and GCC.

## Spec first for anything non-trivial

A feature, a behaviour change, anything that crosses layers (core, engine, app), performance work
or a hard bug starts with a spec, not with code. Specs live in `specs/<NNN>-<slug>/` and ship in the
same pull request as the change. The workflow, and when to skip it, is in
[specs/README.md](specs/README.md). The phases have skills:

1. `spec-write`: `spec.md`, what and why, with acceptance checks. Stop for agreement.
2. `spec-plan`: `plan.md` and `tasks.md`, how, layer by layer. Stop for agreement.
3. `spec-build`: work through `tasks.md` in order, ticking tasks off.
4. `spec-verify`: check every requirement and acceptance check against the code, collect the PR
   evidence, mark the spec Done.

Bugs go through `bug-fix` (reproduce, failing test, fix, verify). Typos, doc edits and small,
obvious fixes skip the spec and go straight to the change.

## Codebase skills

The skills in `.agents/skills/` encode how this codebase is changed. Prefer them to improvising.

| Skill | Use it when |
|---|---|
| `dstudio-command` | adding or changing an edit: a `Command`, its undo, coalescing, persistence, snapshot |
| `dstudio-engine-abi` | anything crossing the C ABI, or touching the audio thread |
| `dstudio-builtin-device` | adding or reworking a built-in effect or instrument |
| `dstudio-ui` | Slint panels, wgpu surfaces, review states, screenshots, the app self-test |
| `dstudio-bench` | measuring performance before and after a change |
| `dstudio-pr` | opening a pull request: evidence, docs, CI, review findings |

Skills are plain Markdown with a short front matter (`name`, `description`). After editing one,
run `python3 tools/agents/check_skills.py`: it checks the front matter and that every repository
path a skill names still exists, so skills don't silently rot as the code moves.

## Before any pull request

```sh
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cd engine && cmake --preset default && cmake --build --preset default && ctest --preset default
cargo run --release -p dstudio-app -- --self-test --no-audio
```

Then follow `dstudio-pr`. Keep [README.md](README.md) and `docs/` accurate in the same pull request
(README › Development › Keeping this README up to date).
