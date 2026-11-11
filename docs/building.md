# Building Device Studio

Everything you need to build, test and run Device Studio from source on macOS, Windows and Linux. The [README](../README.md) has the short version.

## Supported platforms

The engine and the app build on macOS, Windows and Linux. Platform code lives
only in `engine/src/platform/`; everything else is portable.

| | macOS (arm64) | Windows (x64, MSVC) | Linux (x64, GCC) |
|---|---|---|---|
| Audio backend | Core Audio | WASAPI (shared mode) | PipeWire |
| Plugin formats | AU, VST3, CLAP | VST3, CLAP | VST3, CLAP |
| Plugin editor windows | yes | yes | yes (X11; XWayland on Wayland) |
| Built and tested in CI | every run | every run | every run |
| Audio streams run in CI | no (hosted runner, no audio device) | no (no audio endpoint) | yes, a headless PipeWire server |
| Tested on real audio hardware | yes | **no** | **no** |

macOS is the primary platform. On Linux, CI runs a real PipeWire server with a
null sink and a loopback, so the PipeWire streams, duplex recording and the
app's audio self-test run on every change; only physical sound cards are
missing. The WASAPI streaming code compiles and its "no device" paths are
tested, but it has never been run against an audio device. See [engine/README.md › Platforms](../engine/README.md#platforms)
for the full matrix, the WASAPI notes and plugin folders on each system.

## Building

Requirements on every system:

* Rust ≥ 1.90 (edition 2024), via [rustup](https://rustup.rs)
* CMake ≥ 3.25 and Ninja
* A C++23 compiler: Apple clang 21, MSVC 19.4x (Visual Studio 2022) or GCC 13
  have built it with warnings as errors
* Network access on the first configure, which fetches the pinned CLAP headers
  and VST 3 SDK

The Rust build compiles the engine itself (`crates/dstudio-engine/build.rs`), so
`cargo` alone is enough to build and run the app. The engine also builds
standalone with CMake for its own tests, benchmark and tools.

### macOS

```sh
brew install cmake ninja
cargo run --release -p dstudio-app             # the app
cargo test --workspace                       # core, FFI and end-to-end tests

cd engine
cmake --preset default && cmake --build --preset default && ctest --preset default
```

The Xcode command line tools provide Apple clang.

### Windows

Install Visual Studio 2022 with the C++ x64 tools, CMake and Ninja, then run
everything from an **x64 Native Tools / developer command prompt** so the MSVC
environment is set:

```bat
cargo run --release -p dstudio-app
cargo test --workspace

cd engine
cmake --preset default && cmake --build --preset default && ctest --preset default
```

ASIO is not supported (the ASIO SDK's licence keeps it out for now); audio goes
through WASAPI shared mode at the rate set in the Sound control panel.

### Linux

Debian/Ubuntu packages (the same list CI uses, on Ubuntu 24.04):

```sh
sudo apt-get install -y ninja-build cmake pkg-config \
  libpipewire-0.3-dev libasound2-dev \
  libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev libx11-dev libx11-xcb-dev \
  libxcursor-dev libxrandr-dev libxi-dev libfontconfig1-dev libfreetype-dev libudev-dev

cargo run --release -p dstudio-app
cargo test --workspace

cd engine
cmake --preset default && cmake --build --preset default && ctest --preset default
```

CMake must be 3.25 or newer. Ubuntu 24.04 and Debian 13 ship one; on Ubuntu 22.04 (CMake 3.22) install a newer one
first, for example `pip install --user cmake` or Kitware's APT repository (apt.kitware.com), and check `cmake --version`.

The X11, Wayland, xkbcommon, fontconfig and udev packages are for the app;
the engine itself needs only `pkg-config` and `libpipewire-0.3-dev`.

#### PipeWire notes

* Audio I/O on Linux goes through PipeWire streams, not JACK or ALSA directly.
  PipeWire is the default sound server on current mainstream distributions and
  also serves JACK clients; a native JACK backend is possible future work.
* Without a running PipeWire server the engine lists no devices and starting
  audio fails; device tests skip. Run the app with `--no-audio` in that case.
* No sound card (a container, a headless box)? `eval "$(tools/ci/headless-pipewire.sh)"`
  starts PipeWire with a null sink and a loopback source, as CI does. Then
  `DSTUDIO_TEST_LOOPBACK_INPUT=dstudio-loop-source ctest --test-dir engine/build`
  also runs the duplex loopback test, and the app runs with audio.
* Input and output streams share one `node.group`, so PipeWire drives them from
  one clock (resampling the other device when they are different hardware), and
  recordings stay sample-aligned. After a glitch that shifts the input by a
  cycle, the input ring re-syncs within about 64 cycles.
* A requested sample rate must be in the server's `clock.allowed-rates`, and
  the buffer size is a request for a quantum (`node.latency`); PipeWire may not
  honour either, and the device status reports what it actually chose.
* For a real-time audio thread, RTKit or PipeWire's RT module must be allowed
  on the system.
* To build without PipeWire (an engine with no device I/O): configure CMake
  with `-DDSTUDIO_ENGINE_PIPEWIRE=OFF`, or set `DSTUDIO_ENGINE_PIPEWIRE=OFF` in the
  environment for a cargo build.

Plugin editor windows on Linux use X11, which Wayland desktops provide
through XWayland. The build needs the libX11 headers (`libx11-dev`); the
engine loads libX11 itself when the first plugin window opens, so it is not
linked and a system without it only loses plugin windows. To build without it, configure
CMake with `-DDSTUDIO_ENGINE_X11=OFF` or set `DSTUDIO_ENGINE_X11=OFF` for a cargo
build; opening a plugin window then reports that it is unsupported. The
editor test (`plugin_editor_window_x11_open_resize_close`) needs a display:
run it under `xvfb-run` on a headless machine.

## Running the app

```sh
cargo run --release -p dstudio-app -- --help             # all options
cargo run --release -p dstudio-app -- --demo             # open with a demo project
cargo run --release -p dstudio-app -- --self-test --no-audio
```

Use release builds: debug builds of the app abort at exit on every OS (a known
issue in wgpu-core's debug-only lock tracing).

## Continuous integration

CI ([`.github/workflows/ci.yml`](../.github/workflows/ci.yml)) runs on every push
to main and every pull request, including pull requests from forks. It has one
job per system, all on GitHub-hosted runners (`macos-latest`, `windows-latest`,
`ubuntu-latest`). Each job:

1. configures and builds the engine with warnings as errors, and runs ctest;
2. builds and tests the Rust workspace (`--locked`);
3. builds the app in release mode and runs `device-studio --self-test
   --no-audio`, which must report every check passed (Linux under Xvfb with
   Mesa lavapipe; Windows under WARP, informational for now);
4. uploads the screenshots and self-test logs as an artifact.

The hosted runners have no audio device and no GPU. On macOS and Windows the
engine's device tests skip there. The Linux job starts a headless PipeWire
server (`tools/ci/headless-pipewire.sh`), so the device tests, the PipeWire
loopback test and `device-studio --self-test-audio` run against real PipeWire
streams. Tests against real audio hardware (Core Audio duplex, the
acoustic loopback test) are run by hand on a Mac. The loopback test plays
clicks through the speakers and records them with the microphone, so it is
ignored by default and gated on an environment variable:

```sh
DSTUDIO_TEST_REAL_INPUT=1 cargo test -p dstudio-engine --test real_loopback -- --ignored --nocapture --test-threads=1
```

The workflow never uses a self-hosted runner, has no secrets and gives its
token read-only access, so running it on a stranger's pull request is safe.

### Performance checks

[`.github/workflows/perf.yml`](../.github/workflows/perf.yml) runs on pull
requests that touch `engine/`, `crates/dstudio-core/`, `Cargo.lock`,
`tools/perf/` or the workflow itself, and on
pushes to `main`. It builds every engine benchmark (`engine/bench/`) and the
core's Rust benchmarks (`crates/dstudio-core/benches/`) twice on the same
runner, at the base commit and with the change, and compares them row by row
with [`tools/perf/bench_gate.py`](../tools/perf/bench_gate.py):

- **Instructions** (Linux, the gate): each benchmark runs under Valgrind's
  Callgrind, which starts a new count every time the benchmark prints a line,
  so every table row gets the CPU instructions (all threads) spent producing
  it. Counts repeat almost exactly between runs, so a row more than **1 %**
  above the base fails the check.
- **Time** (Linux, macOS, Windows, a warning only): the benchmarks run
  natively, base and head interleaved for three rounds, fastest round per
  row. A row more than **15 %** slower is re-timed and reported as a warning:
  hosted runners are too noisy for a timing gate, but this catches what
  instruction counts cannot (cache misses, denormals, platform code). If the
  comparison itself fails or times out (a flaky runner, or a benchmark that
  will not run on one OS) that is a warning too. A failed setup step, such as
  installing the build tools, still fails the job.

The comparison tables are on each run's summary page. A regression that is
worth it is accepted with the `perf-regression-accepted` label on the pull
request: the check passes and the report still lists it. A benchmark that
cannot be measured with the change (it crashes, or Callgrind records nothing)
fails the check even with the label. New benchmarks and new
rows are picked up without changes to the script, as long as they print tables
(a row is a line ending in numbers, under a header naming the columns).

Run the same comparison locally (instructions need Linux and Valgrind):

```sh
python3 tools/perf/bench_gate.py all --base origin/main              # instructions + time
python3 tools/perf/bench_gate.py all --base origin/main --mode time  # time only (macOS, Windows)
```

### Release packages

[`.github/workflows/release.yml`](../.github/workflows/release.yml) builds the
installable packages:

| System | Package |
|---|---|
| macOS 13+ | `DeviceStudio-<version>-macos-universal.dmg` (Apple silicon + Intel) |
| Windows x64 | `DeviceStudio-<version>-windows-x64-setup.exe` (installer) and a portable `.zip` |
| Linux x86_64, aarch64 | `DeviceStudio-<version>-linux-<arch>.tar.gz` (run `install.sh`) |

Each package carries the plugin scanner next to the app, where the app looks
for it first, and is smoke-tested on its runner (installed, then started to
render a screenshot). Pushing a tag `vX.Y.Z` builds them from `main` and
creates a **draft** GitHub release with generated notes; the tag must match the
`version` in the root `Cargo.toml`. A manual run (Actions › Release › Run
workflow) or a pull request that changes the packaging builds the packages as
run artifacts without releasing anything.

Only release tags are signed with a Developer ID and notarised, in a separate
`macos-sign` job that runs in the protected `release` environment; its secrets
(listed at the top of the workflow) are environment secrets, which pull
requests, manual runs and every other job cannot read. Notarisation uses an
App Store Connect API key, not an Apple ID password. Before adding the secrets,
protect the environment (Settings › Environments › `release`: a required
reviewer, deployments from `v*` tags only) and restrict who can create `v*`
tags with a tag ruleset: a signing run then waits for the maintainer's
approval. A tagged release without the secrets fails rather than ship
unsigned, unless the repository variable `ALLOW_UNSIGNED_MACOS` is `true`.
Every other build is signed ad hoc, and macOS asks the user to allow it on
first launch. The Windows packages are not signed yet,
so SmartScreen warns on first run. The packaging scripts are in
[`tools/release/`](../tools/release/).
