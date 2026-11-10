# Building Saine Studio

Everything you need to build, test and run Saine Studio from source on macOS, Windows and Linux. The [README](../README.md) has the short version.

## Supported platforms

The engine and the app build on macOS, Windows and Linux. Platform code lives
only in `engine/src/platform/`; everything else is portable.

| | macOS (arm64) | Windows (x64, MSVC) | Linux (x64, GCC) |
|---|---|---|---|
| Audio backend | Core Audio | WASAPI (shared mode) | PipeWire |
| Plugin formats | AU, VST3, CLAP | VST3, CLAP | VST3, CLAP |
| Plugin editor windows | yes | yes | yes (X11; XWayland on Wayland) |
| Built and tested in CI | every run (self-hosted Mac) | every run (hosted runner) | every run (hosted runner) |
| Audio streams run in CI | yes, real device | no (no audio endpoint) | yes, a headless PipeWire server |
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

Debian/Ubuntu packages (the same list CI uses, on Ubuntu 24.04):

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
  `SAINE_TEST_LOOPBACK_INPUT=saine-loop-source ctest --test-dir engine/build`
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
  with `-DSAINE_ENGINE_PIPEWIRE=OFF`, or set `SAINE_ENGINE_PIPEWIRE=OFF` in the
  environment for a cargo build.

Plugin editor windows on Linux use X11 (libX11, from `libx11-dev`), which
Wayland desktops provide through XWayland. To build without it, configure
CMake with `-DSAINE_ENGINE_X11=OFF` or set `SAINE_ENGINE_X11=OFF` for a cargo
build; opening a plugin window then reports that it is unsupported. The
editor test (`plugin_editor_window_x11_open_resize_close`) needs a display:
run it under `xvfb-run` on a headless machine.

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
and pull request, one job per OS: macOS on a **self-hosted runner** (one Apple
silicon Mac mini, with a real audio device), Windows and Linux on GitHub-hosted
runners (`windows-latest`, `ubuntu-latest`). Each job:

1. configures and builds the engine with warnings as errors, and runs ctest;
2. builds and tests the Rust workspace (`--locked`);
3. builds the app in release mode and runs `saine-studio --self-test
   --no-audio`, which must report every check passed (on Windows this step is
   informational);
4. uploads the screenshots and self-test logs as an artifact.

Linux also starts a headless PipeWire graph first
([`tools/ci/headless-pipewire.sh`](../tools/ci/headless-pipewire.sh)): the
engine's device tests then open real PipeWire streams, the duplex loopback test
records what the engine plays and checks every click lands on its timeline
position, and the app runs its self-test a second time with audio
(`--self-test-audio`). The hosted Windows runner has no audio endpoint, so the
WASAPI device tests skip there.
