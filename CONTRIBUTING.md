# Contributing to Device Studio

Thanks for your interest. Device Studio is pre-alpha and moves quickly, so the most useful contributions right now are
bug reports with clear steps, testing on Windows and Linux audio hardware (which we have never run on), and small,
focused pull requests.

By taking part you agree to follow the [Code of Conduct](CODE_OF_CONDUCT.md). To report a security problem, follow
[SECURITY.md](SECURITY.md) instead of opening a public issue.

## Before you start

- **Bugs and ideas:** open an issue first using one of the templates. For anything larger than a small fix, wait for a
  reply before writing code, so we can agree on the approach and you don't spend time on something that won't be
  merged.
- **Read the docs:** [docs/architecture.md](docs/architecture.md) explains how the Rust app, the Rust core and the C++
  engine fit together, [docs/adr/](docs/adr/) records the main decisions, and [docs/devices.md](docs/devices.md)
  explains how a built-in device is defined and how to add one.

## Building and testing

[docs/building.md](docs/building.md) has the requirements and packages for macOS, Windows and Linux. In short:

```sh
cargo run --release -p dstudio-app -- --demo      # build the engine and the app, open a demo project
cargo test --workspace                            # core, FFI and end-to-end tests
cd engine && cmake --preset default && cmake --build --preset default && ctest --preset default
cargo run --release -p dstudio-app -- --self-test --no-audio   # drive the real UI through its checks
```

CI runs the tests and the self-test (not the interactive `--demo`) on macOS, Windows and Linux for every pull request.
Its runners have no audio device, so the self-test runs with `--no-audio` and anything that needs audio hardware has to
be tested by hand. On Windows the self-test step is informational
for now (`continue-on-error`), so a green Windows job does not mean the self-test passed: open that step's output to
check. The engine is built with warnings as errors on
clang, MSVC and GCC, so please build on more than one compiler if you can, or check the CI result.

## Pull requests

- **Keep each pull request to one change** and rebase it on the latest `main`.
- **Tests:** a bug fix comes with a few small tests that would have caught it. New behaviour comes with tests too. We
  prefer a handful of well-aimed tests to a large number of shallow ones.
- **Screenshots:** any visible change includes before and after screenshots in the description. The app can render
  itself: `cargo run --release -p dstudio-app -- --no-audio --state reference-default --screenshot out.png --exit-after-frames 5`.
- **Performance:** anything that could affect performance (especially the audio thread) includes benchmark results
  before and after. Extend the existing benchmarks (`engine/bench`, `crates/*/benches`) if none cover your change. The
  audio thread must never allocate, lock or wait.
- **Docs:** keep [README.md](README.md) and `docs/` accurate when your change affects what they describe (see README ›
  Development › Keeping this README up to date).
- **Original work only.** Do not paste code, artwork, icons, fonts, presets or samples from other products, and only add
  third-party libraries whose licences are compatible with ours. New third-party code is listed in
  [engine/third_party_licenses.md](engine/third_party_licenses.md) with its licence.

## Code style

- Rust: `cargo fmt` and `cargo clippy --workspace`.
- C++: C++23, follow the style of the surrounding code, no exceptions or allocation on the audio thread.
- Internal names use `dstudio` (crates, namespaces, `DSTUDIO_*` options). Do not use bare `device` as an identifier
  prefix: it already names effects and instruments throughout the code.
- Write comments that explain why, in plain sentences.

## Licence of contributions

Unless you say otherwise, anything you submit for inclusion is licensed under the same licence as the project (see
[LICENSE](LICENSE)).
