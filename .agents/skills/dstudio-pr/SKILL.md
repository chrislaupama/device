---
name: dstudio-pr
description: Open a Device Studio pull request the project's way - one change per PR rebased on main, the local gate passing, the PR template filled with before/after screenshots and benchmark numbers, README and docs updated, then CI green and every review finding fixed. Use whenever asked to open, prepare or finish a pull request.
---

# Pull requests

A pull request is reviewed by reading its description, so the description carries the proof.

## 1. Scope

One change per pull request, on a branch from the latest `main`. Spec-driven work includes its
`specs/<NNN>-<slug>/` folder, with the spec marked Done (`spec-verify`).

## 2. The local gate

```sh
cargo fmt --all --check && cargo clippy --workspace
cargo test --workspace
cd engine && cmake --preset default && cmake --build --preset default && ctest --preset default
cargo run --release -p dstudio-app -- --self-test --no-audio
```

## 3. Evidence

| The change... | The description includes |
|---|---|
| changes anything visible (a panel, card, menu, layout, drawing) | before and after screenshots of the same review state, same size and zoom (`dstudio-ui`) |
| could affect performance (render path, DSP, threading, allocation, snapshot or edit cost, frame time, startup) | before and after benchmark numbers with the machine (`dstudio-bench`) |
| fixes a bug | how to reproduce it, the cause, and the test that now guards it (`bug-fix`) |
| none of these (docs, CI, a refactor with no runtime effect) | a sentence saying so |

Screenshots, recordings and other media for the description never go on the pull request's own
branch, so they never land on `main`. Commit them to the `pr-screenshots` branch (an orphan branch
holding only media, one folder per change, never merged), push it, and link each file by that
commit's hash: `https://raw.githubusercontent.com/chrislaupama/device/<commit>/<folder>/<file>`.
Only add commits on top of `pr-screenshots`: never delete, rebase or force-push it, because its
history is what keeps the hash links working. `docs/pr-media.md` has the
commands, folder and file names, formats and sizes.

## 4. Docs

Tick the template's Docs checklist honestly: README.md (features, devices and their count, status,
platforms, build steps, screenshots), PROGRESS.md, and the `docs/` pages the change affects. README
› Development › Keeping this README up to date says which section covers what.

## 5. Description

Fill in `.github/pull_request_template.md`: What (what changed and why, linking the spec),
Screenshots, Benchmarks, Docs. Write it for a reviewer who hasn't seen the work. Commit messages
say what and why in plain sentences; no tool attribution lines or trailers in commits or the
description.

## 6. Until it is done

A pull request is done when CI is green on macOS, Windows and Linux and every review finding,
including automated reviewers, is fixed and resolved. A failing check is fixed at its cause, never
by skipping or weakening a test. If a finding is wrong, explain why in the review thread rather
than ignoring it.
