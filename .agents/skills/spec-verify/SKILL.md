---
name: spec-verify
description: Check a built Device Studio spec against the code before its pull request - every requirement and acceptance check, the principles, docs, tests, screenshots and benchmark evidence - and mark it Done. Use after spec-build, and again after any review changes.
---

# Verify a spec

Building is done when every task is ticked; the work is done when this pass finds nothing.

## 1. Requirements and acceptance checks

Go through `spec.md`. For each acceptance check, run it (or find the test that runs it) and record
the result. For each requirement, point at the code and the check that proves it. Any requirement
without proof is a gap: fix it, or, if the maintainer agrees, move it to Out of scope with a line
saying why.

## 2. Principles

Re-read `specs/principles.md` against the actual diff (`git diff origin/main...HEAD`), not against
the plan. In particular: nothing new allocates or locks on the audio thread; every edit goes
through a `Command` with a working undo; old projects still load; ABI layout tests pass; no bare
`device` identifiers; nothing copied from another product.

## 3. The full local gate

```sh
cargo fmt --all --check && cargo clippy --workspace
cargo test --workspace
cd engine && cmake --preset default && cmake --build --preset default && ctest --preset default
cargo run --release -p dstudio-app -- --self-test --no-audio
```

The self-test prints `self-test: P / N checks passed`; P must equal N.

## 4. Evidence

Collect what the pull request needs (see `dstudio-pr`): before and after screenshots of the review
states named in the plan, before and after benchmark numbers with the machine noted.

## 5. Docs

Check that README.md, PROGRESS.md and `docs/` describe the new state. Read the README checklist in
README › Development › Keeping this README up to date.

## 6. Close the spec

Set `Status: Done` in `spec.md`. If anything changed during the build, make sure the spec and plan
now describe what was built. Then open the pull request with `dstudio-pr`, linking the spec folder.
