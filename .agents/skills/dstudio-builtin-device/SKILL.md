---
name: dstudio-builtin-device
description: Add or rework one of Device Studio's built-in effects or instruments - the DeviceDef in dstudio-core, the BuiltinProcessor kernel in engine/src/devices, the card or custom view in the app, presets, tests, benchmark row and review state. Use for any work on a built-in device, its parameters, DSP or panel.
---

# Built-in devices

Built-in devices are declarative: one `DeviceDef` in Rust describes the device, and the model, undo,
persistence, automation, modulation, engine state and panel all derive from it. The full reference
is `docs/devices.md`; read it before starting, and treat this skill as the checklist that makes sure
nothing in it is missed. Start from the closest existing device and mirror it (grep its id across
`crates/` and `engine/`).

## The parts

| Part | Where | Notes |
|---|---|---|
| Definition | `crates/dstudio-core/src/builtin/<id>.rs`, listed in `builtin::all()` (`crates/dstudio-core/src/builtin/mod.rs`) | parameters, layout, factory presets, behaviour |
| DSP kernel | `engine/src/devices/<id>.cpp`, registered in `kBuiltinRegistry` (`engine/src/devices/builtin.cpp`), added to `engine/CMakeLists.txt` | a `BuiltinProcessor` plus a `BuiltinDescriptor` |
| Standard card | derived by `crates/dstudio-app/src/device_panel.rs`, drawn by `crates/dstudio-app/ui/device_panel.slint` | nothing to write unless the layout needs a new control kind |
| Custom view | `View::Custom("<name>")`: `device_views.rs` (`custom_body`), `device_canvas.rs`, `panel_graph.rs`, `device_editors.rs` in `crates/dstudio-app/src/` | only when the standard card can't express it |
| Engine test | `engine/tests/devices/test_<id>.cpp` with `DeviceRig` / `DeviceSession` from `device_harness.hpp` | behaviour, not just "it runs" |
| Benchmark row | `engine/bench/bench_devices.cpp` (`dstudio_device_bench`) | numbers go in the pull request |

## Rules that bite

- **Parameter order is the engine order, and shipped parameters are append-only.** Never reorder,
  rename or remove one; append. Values are stored by id, so old projects keep loading.
- **Kernel and definition must agree** on parameter ids, order and defaults.
  `engine_registry_matches_core_definitions` (`crates/dstudio-engine/tests/builtin.rs`) fails when
  they don't.
- **`process` and `reset` are real-time** (see `dstudio-engine-abi`): no allocation, locks, I/O or
  exceptions. `prepare` may allocate. Parameters are constant within one `process` call; smoothing
  audible steps is the kernel's job (`dsp::Smoother`).
  `every_builtin_is_rt_safe_and_robust` runs every registered device under the allocation guard
  with hostile input and extreme values, so a new device is covered automatically once registered.
- **Reuse the shared DSP** before writing your own: `engine/src/devices/dsp.hpp` (smoothers, LFO
  clock, delay line, oversampler, state-variable filter), `dsp_dynamics.hpp`, `spectral.hpp`,
  `sampler_engine.hpp`, `eq_sections.hpp`.
- **Report latency** through `latency()` / `latency_for(params)` so delay compensation works.
- **Readouts** move away from their idle value as they grow, and declare `idle_readouts`.
- **Linked writes** (one edit moving several parameters) go through `builtin::linked_writes` so the
  panel commits one labelled `Command::Batch`.
- **Original work.** Ranges, defaults, units and tapers come from the device's agreed spec. Write
  descriptions, preset names and visuals in our own words and style; never copy another product's
  code, images, audio or preset files.
- Naming: the id is kebab-case; Rust and C++ symbols never use bare `device` as a prefix.

## Checklist

1. Definition, with every parameter's id, range, default, unit, taper and format; the `Layout`;
   factory presets.
2. Kernel and descriptor, registered, in CMake.
3. Custom view only if needed.
4. Tests: an engine test for the device's behaviour (a tone in, the expected change out; parameter
   extremes; latency if any), plus core tests for any new display format or parsing.
5. Benchmark row in `bench_devices.cpp`; run `dstudio_device_bench` before and after.
6. Review state: `--state reference-<id>` shows any built-in device at its defaults. Render it with
   `--offscreen --screenshot <png>` for the pull request.
7. Self-test: add the device where its family is added in `crates/dstudio-app/src/selftest.rs`.
8. Docs: the device list in `docs/devices.md`, README › Built-in devices (and the count), and
   PROGRESS.md.

```sh
cargo test --workspace
cd engine && cmake --preset default && cmake --build --preset default && ctest --preset default && ./build/dstudio_device_bench
cargo run --release -p dstudio-app -- --state reference-<id> --offscreen --screenshot /tmp/<id>.png
```
