# Building Saine Studio

Everything you need to build, test and run Saine Studio from source on macOS, Windows and Linux. The [README](../README.md) has the short version.

## Supported platforms

The engine and the app build on macOS, Windows and Linux. Platform code lives
only in `engine/src/platform/`; everything else is portable.

| | macOS (arm64) | Windows (x64, MSVC) | Linux (x64, GCC) |
|---|---|---|---|
| Audio backend | Core Audio | WASAPI (shared mode) | PipeWire |
| Plugin formats | AU, VST3, CLAP | VST3, CLAP | VST3, CLAP |
| Plugin editor windows | yes | yes | not yet |
| Built and tested in CI | every run (self-hosted Mac) | last hosted run only | last hosted run only |
| Tested on real audio hardware | yes | **no** | **no** |

macOS is the primary platform. The WASAPI and PipeWire streaming code compiles
and its "no device" paths are tested, but it has never been run against a real
audio device. See [engine/README.md › Platforms](../engine/README.md#platforms)
for the full matrix, the WASAPI notes and plugin folders on each system.

## Building

Requirements on every system:

* Rust ≥ 1.90 (edition 2024), via [rustup](https://rustup.rs)
* CMake ≥ 3.25 and Ninja
* A C++23 compiler: Apple clang 21, MSVC 19.4x (Visual Studio 2022) or GCC 13
  have built it with warnings as errors
* Network access on the first configure, which fetches the pinned CLAP headers
  and VST 3 SDK

The Rust build compiles the engine itself (`crates/saine-engine/build.rs`), so
`cargo` alone is enough to build and run the app. The engine also builds
standalone with CMake for its own tests, benchmark and tools.

### macOS

```sh
brew install cmake ninja
cargo run --release -p saine-app             # the app
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
cargo run --release -p saine-app
cargo test --workspace

cd engine
cmake --preset default && cmake --build --preset default && ctest --preset default
```

ASIO is not supported (the ASIO SDK's licence keeps it out for now); audio goes
through WASAPI shared mode at the rate set in the Sound control panel.

### Linux

Debian/Ubuntu packages (the same list CI uses):

```sh
sudo apt-get install -y ninja-build cmake pkg-config \
  libpipewire-0.3-dev libasound2-dev \
  libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev libx11-dev libx11-xcb-dev \
  libxcursor-dev libxrandr-dev libxi-dev libfontconfig1-dev libfreetype-dev libudev-dev

cargo run --release -p saine-app
cargo test --workspace

cd engine
cmake --preset default && cmake --build --preset default && ctest --preset default
```

The X11, Wayland, xkbcommon, fontconfig and udev packages are for the app;
the engine itself needs only `pkg-config` and `libpipewire-0.3-dev`.

#### PipeWire notes

* Audio I/O on Linux goes through PipeWire streams, not JACK or ALSA directly.
  PipeWire is the default sound server on current mainstream distributions and
  also serves JACK clients; a native JACK backend is possible future work.
* Without a running PipeWire server the engine lists no devices and starting
  audio fails; device tests skip. Run the app with `--no-audio` in that case.
* A requested sample rate must be in the server's `clock.allowed-rates`, and
  the buffer size is a request for a quantum (`node.latency`); PipeWire may not
  honour either, and the device status reports what it actually chose.
* For a real-time audio thread, RTKit or PipeWire's RT module must be allowed
  on the system.
* To build without PipeWire (an engine with no device I/O): configure CMake
  with `-DSAINE_ENGINE_PIPEWIRE=OFF`, or set `SAINE_ENGINE_PIPEWIRE=OFF` in the
  environment for a cargo build.

Plugin editor windows are not implemented on Linux yet; opening one reports
that it is unsupported.

## Running the app

```sh
cargo run --release -p saine-app -- --help             # all options
cargo run --release -p saine-app -- --demo             # open with a demo project
cargo run --release -p saine-app -- --self-test --no-audio
```

Use release builds: debug builds of the app abort at exit on every OS (a known
issue in wgpu-core's debug-only lock tracing).

## Continuous integration

CI ([`.github/workflows/ci.yml`](../.github/workflows/ci.yml)) runs on every push
and pull request on a **self-hosted runner**: one Apple silicon Mac mini, so
the tests run on real hardware with a real audio device. Each run:

1. configures and builds the engine with warnings as errors, and runs ctest;
2. builds and tests the Rust workspace (`--locked`);
3. builds the app in release mode and runs `saine-studio --self-test
   --no-audio`, which must report every check passed;
4. uploads the self-test logs as an artifact.

Windows and Linux are **not built in CI** at the moment. Their steps stay in
the workflow, gated on `runner.os` (Linux: Xvfb + Mesa lavapipe screenshot and
self-test; Windows: WARP, informational), but they run only when the job lands
on a runner of that OS. The workflow has a single `runs-on: self-hosted` job
and no platform matrix, so adding a Windows or Linux runner alone would not
test every platform on each run: that runner could pick up the job instead of
the Mac. Covering all three needs runner labels per OS and a matrix or one job
per OS. The Windows and Linux results above come from the last run on
GitHub-hosted runners, before that matrix was dropped; later changes are not
compiled on those systems.
