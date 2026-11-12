# Tasks: Channel strip trim, width and mono

Plan: [plan.md](plan.md)

- [x] T1. Engine: `se_track_strip`, `se_session_publish_v10`, `apply_strips`, `kernel_strip` — verify: `ctest -R strip`
- [x] T2. Core: model fields, three commands, snapshot, render plan — verify: `cargo test -p dstudio-core --test strip`
- [x] T3. Rust FFI and publish — verify: `cargo test -p dstudio-engine` (layout test)
- [x] T4. App: strip knobs and switch, controller, self-test steps, review state — verify: self-test
- [x] T5. Docs: README, PROGRESS.md, engine/README.md — verify: read them against the change
- [x] T6. Evidence: screenshots and benchmark numbers for the pull request
