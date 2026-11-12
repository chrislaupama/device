---
name: dstudio-bench
description: Measure Device's performance before and after a change - which engine, core or app benchmark covers what, how to run a fair comparison, when to add a benchmark, and how to report the numbers in a pull request. Use for any change that could affect CPU, memory, latency, frame time or load time.
---

# Benchmarks

Every change that could affect performance comes with before and after numbers, and the aim is
always to make things faster, never to find out later that they got slower. If no benchmark covers
the change, add one in the same pull request.

## What covers what

| Area | Benchmark | Run |
|---|---|---|
| Whole-session render (tracks, clips, MIDI, plugins, delay compensation) | `engine/bench/bench_render.cpp` | `./build/dstudio_engine_bench [blocks] [clips_per_track] [features]` |
| Built-in devices, modulation, parameter updates | `engine/bench/bench_devices.cpp` | `./build/dstudio_device_bench [blocks]` |
| Time-stretch / warp | `engine/bench/bench_warp.cpp` | `./build/dstudio_warp_bench` |
| Clip loops | `engine/bench/bench_clip_loop.cpp` | `./build/dstudio_clip_loop_bench` |
| Audio file decoding | `engine/bench/bench_decode.cpp` | `./build/dstudio_decode_bench` |
| Offline rendering through plugins | `engine/bench/bench_offline_plugins.cpp` | `./build/dstudio_offline_plugin_bench` |
| Plugin scan cache | `engine/bench/bench_scan_cache.cpp` | `./build/dstudio_scan_cache_bench` |
| Snapshot and edit cost in the core | `crates/dstudio-core/benches/` (`warp`, `steps`, `library`, `clip_loop`) | `cargo bench -p dstudio-core --bench <name>` |
| App frame time | `device-studio` options | `--frame-stats`, `--render-bench N`, `--drag-bench <what>` |

Engine benchmarks run from `engine/` after a Release build (`cmake --preset default && cmake
--build --preset default`); never quote numbers from a Debug or sanitizer build. Each file's header
comment says exactly what it measures.

## A fair comparison

1. Build the base: a worktree of `origin/main` (`git worktree add ../dstudio-base origin/main`),
   Release.
2. Build the branch, Release, same compiler and flags.
3. Run both on the same machine, back to back, mains power, nothing heavy in the background. Run
   each at least three times and take the median; say if the spread is large.
4. Report the machine (CPU, OS) with the numbers.

## Adding a benchmark

- Extend the closest existing one before writing a new executable: a new row in
  `bench_devices.cpp` for a device, a new case in `bench_render.cpp` for a render-path feature.
- Measure what the user pays for: per-block cost at 256 frames and 48 kHz for the audio path (as
  microseconds per block and as a share of the block deadline), per-edit cost for commands and
  snapshots, per-frame cost for drawing.
- Keep the file's header comment current: what it measures, its arguments, its units.
- A new engine benchmark executable goes in `engine/CMakeLists.txt` next to the others; a new core
  benchmark gets a `[[bench]]` entry with `harness = false` in `crates/dstudio-core/Cargo.toml`.

## Reporting

In the pull request's Benchmarks section, a small table: case, before, after, change, with the
machine and the command used. Call out anything that got slower and why it is acceptable. If the
render numbers in `engine/README.md` move noticeably, update them, and README › Under the hood if
its headline number changes.
