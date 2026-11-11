# Building Device Studio

Everything you need to build, test and run Device Studio from source on macOS, Windows and Linux. The [README](../README.md) has the short version.

## Supported platforms

The engine and the app build on macOS, Windows and Linux. Platform code lives
only in `engine/src/platform/`; everything else is portable.

| | macOS (arm64) | Windows (x64, MSVC) | Linux (x64, GCC) |
|---|---|---|---|
| Audio backend | Core Audio | WASAPI (shared mode) | PipeWire |
| Plugin formats | AU, VST3, CLAP | VST3, CLAP | VST3, CLAP |
| Plugin editor windows | yes | yes | not yet |
| Built and tested in CI | every run | every run | every run |
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
* A requested sample rate must be in the server's `clock.allowed-rates`, and
  the buffer size is a request for a quantum (`node.latency`); PipeWire may not
  honour either, and the device status reports what it actually chose.
* For a real-time audio thread, RTKit or PipeWire's RT module must be allowed
  on the system.
* To build without PipeWire (an engine with no device I/O): configure CMake
  with `-DDSTUDIO_ENGINE_PIPEWIRE=OFF`, or set `DSTUDIO_ENGINE_PIPEWIRE=OFF` in the
  environment for a cargo build.

Plugin editor windows are not implemented on Linux yet; opening one reports
that it is unsupported.

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

The hosted runners have no audio device and no GPU, so the engine's device
tests skip there. Tests against real audio hardware (Core Audio duplex, the
acoustic loopback test) are run by hand on a Mac. The loopback test plays
clicks through the speakers and records them with the microphone, so it is
ignored by default and gated on an environment variable:

```sh
DSTUDIO_TEST_REAL_INPUT=1 cargo test -p dstudio-engine --test real_loopback -- --ignored --nocapture
```

The workflow never uses a self-hosted runner, has no secrets and gives its
token read-only access, so running it on a stranger's pull request is safe.
