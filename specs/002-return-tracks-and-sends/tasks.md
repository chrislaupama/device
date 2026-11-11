# Tasks: Return tracks and sends

Plan: [plan.md](plan.md)

Ordered so the tree builds and the tests pass after every task. Each task says how it is verified.
Tick a task only when its verification passed.

- [x] T1. Core model: `TrackKind::Return`, `Send`, `Track::sends`, `Track::output`, helpers —
  verify: `cargo test -p dstudio-core`
- [x] T2. Core commands: `AddTrack`/`RemoveTrack` rules, `SetSends`, `SetTrackOutput`, refusals on
  returns, labels and coalescing — verify: `cargo test -p dstudio-core --test returns`
- [x] T3. Core snapshot fields and persistence round trip — verify: `cargo test -p dstudio-core`
- [x] T4. Engine graph, ABI v9, validation and order — verify: `ctest -R routing`
- [x] T5. Engine rendering: buses, sends, outputs, mute/solo — verify: `ctest -R routing`
- [x] T6. Engine delay compensation across buses — verify: `ctest -R "routing|pdc"`
- [x] T7. Rust wrapper `publish` on v9, layout tests, e2e test — verify: `cargo test -p dstudio-engine`
- [x] T8. Benchmark "Returns" table — verify: `dstudio_render_bench` numbers in the PR
- [x] T9. App: Returns pill and rows, return strips, Sends section, Sends tab, Out menu, Add
  track entries — verify: self-test, review states
- [x] T10. Docs: README, PROGRESS.md, engine/README.md, ADR 0004 note — verify: read them against
  the change
- [ ] T11. Evidence: screenshots and benchmark numbers for the pull requests — verify: `spec-verify`
