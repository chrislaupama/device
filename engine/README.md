# Saine Studio audio engine (`engine/`)

C++23 real-time audio engine behind the C ABI in `include/saine_engine.h`.
The Rust core links the static library `saine_engine`. The header is the
contract. `SE_ABI_VERSION` is still 1: the first milestone's structs and
signatures are unchanged, and later features are **appended** at the end of
the header (`se_meters_peek`, `se_transport_set_loop`, `se_metronome_desc` +
`se_metronome_set`).

## Build, test, bench

Requirements: CMake ≥ 3.25, Ninja, a C++23 compiler (Apple clang 21 tested).

```sh
cd engine
cmake --preset default            # Release, Ninja, -Werror  -> engine/build
cmake --build --preset default
ctest --preset default            # every TEST_CASE is its own CTest test

cmake --preset asan               # Debug + -fsanitize=address,undefined -> engine/build-asan
cmake --build --preset asan
ctest --preset asan

./build/saine_engine_bench [blocks=4000] [clips_per_track=4] [features=0]  # 1 = loop + metronome on
./build/play_sine_or_wav [file.wav] [seconds=1.0]   # real default output device, -24 dBFS
```

CMake options: `SAINE_ENGINE_BUILD_TESTS|BENCH|TOOLS` (default ON when top
level), `SAINE_ENGINE_SANITIZE` (ASan+UBSan), `SAINE_ENGINE_WERROR`.

### Linking from Rust (`cmake` crate)

The `cmake` crate builds and runs the `install` target, which produces
`<out>/lib/libsaine_engine.a` and `<out>/include/saine_engine.h`. In `build.rs`:

```rust
let dst = cmake::Config::new("../../engine")
    .define("SAINE_ENGINE_BUILD_TESTS", "OFF")
    .define("SAINE_ENGINE_BUILD_BENCH", "OFF")
    .define("SAINE_ENGINE_BUILD_TOOLS", "OFF")
    .build();
println!("cargo:rustc-link-search=native={}/lib", dst.display());
println!("cargo:rustc-link-lib=static=saine_engine");
// macOS
println!("cargo:rustc-link-lib=c++");
println!("cargo:rustc-link-lib=framework=AudioToolbox");
println!("cargo:rustc-link-lib=framework=CoreAudio");
println!("cargo:rustc-link-lib=framework=CoreFoundation");
```

The library is built with `-fPIC`, so it can go into a cdylib as well.

## Layout

| Path | Contents |
|---|---|
| `include/saine_engine.h` | C ABI (v1; additions appended at the end) |
| `src/c_api.cpp` | `extern "C"` wrappers, exception barrier, thread-local `se_last_error` |
| `src/engine.{hpp,cpp}` | `Engine`: assets, graph publication/reclamation, transport, meters, stats, RT `process()` |
| `src/render_graph.{hpp,cpp}` | Immutable render graph built from a session snapshot; pan law |
| `src/processor.{hpp,cpp}` | `render_block()` (the one DSP path) and SIMD-friendly kernels |
| `src/spsc_queue.hpp` | Lock-free SPSC ring for transport commands |
| `src/wav.{hpp,cpp}` | Hand-written RIFF/WAVE reader, streaming float32 writer |
| `src/resampler.{hpp,cpp}` | Kaiser-windowed sinc resampler (import time) |
| `src/rt_util.hpp` | Flush-to-zero guard, clock, atomic max |
| `src/platform/` | `AudioDevice` interface + backends (Core Audio real; WASAPI/ASIO, PipeWire/JACK stubs) |
| `tests/` | Self-contained harness (`test_harness.hpp`), tests, allocation-tracking `operator new` |
| `bench/bench_render.cpp` | Offline render benchmark |
| `tools/play_sine_or_wav.cpp` | Manual check through the real default output device |

## Design

### Data flow

```
Rust core (non-RT)                                  audio thread (RT)
------------------                                  -----------------
se_asset_load_file ─ decode WAV ─ resample ─► asset registry (shared_ptr<const Asset>)
se_session_publish ─ copy + validate + build ─► RenderGraph ──atomic ptr swap──► process()
se_transport_* / set_loop / metronome_set
                   ─ mutex(producers) ─► SPSC queue ─┬─────────── drained at block start (device/offline render)
                                                     └─ applied inline when nobody owns the RT path
se_meters_read / se_meters_peek / se_stats_get ◄── atomics ◄───── written per block / segment
```

### Render graph

`se_session_publish` copies the description into an immutable `RenderGraph`
on the calling thread. Everything is resolved up front: mute/solo collapses
into one `audible` flag, track gain × pan law becomes two per-channel gains,
clips are clamped to their asset's length (clips past the asset end or with
zero length are dropped) and sorted by start, and raw sample pointers are
cached. The graph holds `shared_ptr` references to every asset it uses.
Validation errors: negative `source_offset`/`length`, non-finite gains/pan,
NULL arrays with non-zero counts → `SE_ERR_INVALID_ARG`; unknown asset id →
`SE_ERR_NOT_FOUND`. The published graph is replaced atomically: the audio
thread sees either the old or the new graph for a whole block, never a mix.

Semantics:

* **Clip**: plays asset frames `[source_offset, source_offset + length)` at
  timeline frames `[timeline_start, timeline_start + length)`, scaled by
  `gain`. Overlapping clips on a track sum. Mono assets feed both channels.
* **Pan**: constant power, `θ = (pan + 1)·π/4`, `L = cos θ`, `R = sin θ`
  (centre = −3.01 dB each side, `L² + R² = 1`, hard pans are exact 1/0).
  For stereo material it acts as a balance control (L gain on the L channel).
* **Mute/solo**: if any track is soloed, only soloed tracks are audible.
  Mute always wins (a soloed and muted track is silent).
* **Master**: sum of tracks × `master_gain`. No limiter/clipper — output can
  exceed ±1.0.

### Threading model

| Thread | What it may do |
|---|---|
| Audio thread (Core Audio IO proc) **or** the thread inside `se_render_offline` | `Engine::process()` only |
| Any non-RT thread | everything else in the ABI |

Exactly one "RT owner" at a time: an atomic owner flag (`rt_owner_`:
none / device / offline / control) makes `se_render_offline` fail with
`SE_ERR_STATE` while the device runs (and vice versa; a second concurrent
offline render also gets `SE_ERR_STATE`). Only the owner touches the RT
transport state and pops the command queue. The fourth value, *control*, is
a short claim a non-RT thread takes to apply transport commands inline (see
Transport); device start and offline render yield until it is released
instead of failing.
`se_render_offline` runs the identical `process()` the device callback runs
(command drain, graph acquire, chunked `render_block`, meters, stats).
`se_export_wav` uses the same `render_block()` DSP with a private transport,
scratch buffers and a `shared_ptr` to the current graph, so it never touches
live playback/meters/stats and may run while the device is playing.

Mutexes exist only on the non-RT side: `assets_mutex_` (registry),
`control_mutex_` (current/retired graphs), `command_mutex_` (serialises
transport producers onto the SPSC queue), `device_mutex_` (start/stop).

### Graph reclamation (no frees on the audio thread)

The audio thread uses a single hazard slot `rt_active_seq_`:

```
RT block start: active = BUSY;  g = live_graph (seq_cst);  active = g->seq
RT block end:   active = IDLE
```

A retired graph `R` (already swapped out) is freed only when
`active == IDLE` or (`active != BUSY` and `active > R.seq`). The RT side is
wait-free; the non-RT side checks in `se_session_publish`, `se_asset_release`
and engine destruction (`Engine::collect_garbage`). A graph that cannot be
freed yet just stays on the retired list until a later check. Graph
destruction drops asset references, so asset memory is also released only on
non-RT threads.

### Assets

`shared_ptr<const Asset>` (planar float at engine rate). The registry holds
one reference, every graph that uses the asset holds another.
`se_asset_release` removes the id from the registry (later snapshots that
reference it fail with `SE_ERR_NOT_FOUND`) but audio already published keeps
playing until the graph using it is replaced and reclaimed.

### Transport

Commands (`play`, `pause`, `stop` = pause + position 0, `seek`, and
`se_transport_set_loop` / `se_metronome_set`) all go through one 256-entry
lock-free SPSC queue (producers serialised by `command_mutex_`). Positions are
`int64` frames and may be negative (pre-roll renders silence).

* **Device running**: commands are applied, in order, at the start of the
  next callback (block boundary, not sample-accurate). A full queue returns
  `SE_ERR_STATE`. `se_transport_get` shows the state as of the last block.
* **No device running** (and no `se_render_offline` in progress): the
  command is applied synchronously on the caller's thread before the call
  returns, so `se_transport_get` reflects it immediately, without rendering.
* **During a concurrent `se_render_offline`** on another thread: queued;
  applied at that render's next block start or when it returns.

How it works and why it is race-free (`Engine::push_command`,
`acquire_rt_owner`, `release_rt_owner`):

1. A producer always pushes onto the queue first, then issues a seq_cst
   fence and tries to claim the RT path (`CAS none → control`). On success
   it drains the whole queue (so earlier queued commands keep their order),
   publishes the transport state and releases. Being the single owner, it is
   the only consumer of the SPSC queue and the only writer of the RT state.
2. If the device or an offline render owns the path the CAS fails and the
   command stays queued for that owner. If another thread holds the brief
   control claim, the producer yields and retries, so with no device and no
   offline render the command is always applied before the call returns.
3. Every owner releases with `store(none); seq_cst fence; if queue not
   empty → claim and drain` (looping). Producer (`push; fence; CAS`) and
   releaser (`store none; fence; check empty`) form a store-buffering pair:
   with both fences seq_cst, at least one of them sees the other's write, so
   either the producer's CAS succeeds or the releaser sees the command. A
   command can never be stranded in the queue with no owner left to apply
   it. `se_device_stop` releases this way after the device has stopped,
   so commands queued after the last callback are applied by the stop call.
4. All ownership transitions are seq_cst RMWs/stores on `rt_owner_`, which
   gives happens-before between successive owners for the plain RT fields
   (position, loop, metronome, click voices). Device start/stop are still
   serialised by `device_mutex_`.

Tested by `offline_transport_no_stranded_commands_under_concurrent_render`
(producer vs. offline renderer), `transport_commands_across_device_start_stop`
(producer vs. start/stop cycles on the real device; skipped without one),
under ASan/UBSan and, ad hoc, ThreadSanitizer.

### Loop

`se_transport_set_loop(enabled, start, end)` (requires `start < end` when
enabling). While playing with the loop on and the playhead before `end`,
`process()` splits its segments at `end`; when a segment reaches `end` the
playhead jumps to `start`, so the wrap is sample-accurate anywhere inside a
device/offline block, and loops shorter than a block wrap repeatedly. A
playhead before `start` plays into the loop and then wraps; a playhead at or
after `end` plays linearly. `render_block()` is stateless (clips are
addressed by timeline frame), so the segment after the wrap renders clip
offsets for `start` exactly and nothing stale carries over. (The audio jump
itself is not crossfaded.) Disabling mid-play continues linearly.

### Metronome

`se_metronome_set(&{enabled, gain, frames_per_beat, beats_per_bar})`. The
two click shapes (15 ms sine, 1.2 kHz downbeat / 800 Hz beat, 3 ms
exponential decay, 2 ms raised-cosine tail fade, first sample non-zero) are
computed once in the `Engine` constructor; setting the metronome only queues
parameters. While playing, beat `k` starts at timeline frame
`llround(k · frames_per_beat)` (beat 0 at frame 0, negative beats in
pre-roll); `k mod beats_per_bar == 0` is a downbeat. Onsets come from the
timeline beat grid of each rendered segment (so they are sample-accurate in
any block partition and follow the loop: a beat sounds whenever the playhead
crosses its frame). Up to 8 click *voices* carry tails across segments in
playback time, so a click that started in the previous block, or just before
a loop wrap or a seek, rings out instead of being cut (the oldest voice is
stolen if more overlap). Voices are cleared on pause/stop/disable. The click
is mixed into the master **after master gain and after metering**, so it is
independent of the mix level, does not show on meters, and is never in
`se_export_wav` (which calls `render_block()` directly).

### Export

`se_export_wav` renders `[start, end)` linearly with its private transport:
it ignores the live loop and metronome (tested by
`export_ignores_loop_and_metronome`).

### Blocks, meters, stats

* Scratch buffers are preallocated to `max_block_size` (1..16384). A device
  or offline request larger than that is processed in `max_block_size`
  chunks inside one `process()` call; output is bit-identical for any block
  partitioning (tested).
* Meters: master + one per track (post fader/pan), stored as atomics and
  raised with a lock-free max. `se_meters_read` returns master first, then
  tracks in session order, and **resets** each peak it reads (peak-hold since
  last read; decay/ballistics are left to the UI). Track meters belong to the
  current graph, so they restart at 0 when a new snapshot is published.
* `se_meters_peek` is the non-destructive variant (same ordering): the max
  peak over a sliding window of 5 buckets × 10 ms of *rendered* frames
  (preallocated atomic rings, master in the engine, tracks in the graph's
  `TrackMeter`). Each render segment's peak goes into the bucket it ends in;
  buckets entered are zeroed by the RT side. So values are stable however
  fast the UI polls, decay to 0 after 40–50 ms of rendered silence (paused
  output counts), and hold while nothing renders. One block longer than the
  window counts as a single bucket. Neither call disturbs the other. Both
  meter kinds exclude the metronome.
* Stats are recorded per `process()` call (device callbacks and offline
  renders alike): count, last/avg/max µs, the block deadline
  (`frames / sample_rate`) and the number of calls that exceeded it.
* FTZ/DAZ is enabled for the duration of each `process()` call.

### SIMD

The mixing kernels are plain loops over `__restrict` pointers that clang
auto-vectorises (verified with `-Rpass=loop-vectorize`: width 4, interleaved
2–4 on arm64). The max-abs peak reduction has explicit NEON and SSE2 paths,
because `std::max` semantics prevent auto-vectorisation without fast-math.

### Multithreading (future)

Not in this milestone: all tracks render on the audio thread. Tracks are
independent until the master sum and the graph is a flat read-only array, so
a worker pool can later render disjoint track ranges into per-worker scratch
buffers (allocated with the graph or the context, never on the RT thread) and
the audio thread then sums them. Buses/sends will need a precomputed
topological order in the graph; see the comment in `render_graph.hpp`.

## Real-time safety rules

Inside `Engine::process()` / `render_block()` and the device callback:

1. No heap allocation or deallocation (enforced by a test that replaces
   global `operator new/delete` and counts calls made inside an RT scope:
   zero allowed across 650 renders with seeks/pauses/stops, chunked blocks,
   32 tracks × 4 clips, and a graph swap; and across 1500 renders with loop
   wraps, metronome onsets/tails/voice stealing, while other threads call
   `se_meters_peek`/`se_meters_read` and push loop/metronome/seek commands
   that the render drains).
2. No locks, no waiting inside `process()`: only atomics, the SPSC pop, and a
   CAS loop for meter max that only retries if a reader raced it. (The
   `se_render_offline` *wrapper* may yield while a non-RT thread holds the
   brief transport "control" claim; the device callback never waits.)
3. No I/O, logging, or exceptions; everything is `noexcept`.
4. Never free a graph or asset; only the non-RT side reclaims.
5. No first-touch thread-local storage (`se_last_error` is only written on
   failure paths, never by a successful `se_render_offline`).

## WAV support

Reader (hand-written RIFF parser): `WAVE_FORMAT_PCM` 8-bit unsigned and
16/24/32-bit signed, `WAVE_FORMAT_IEEE_FLOAT` 32/64-bit, and
`WAVE_FORMAT_EXTENSIBLE` around either (e.g. 24-bit in 32-bit containers).
Odd-sized chunks/padding and unknown chunks are handled. A `data` chunk that
claims more bytes than the file holds is clamped. **Mono and stereo only:
files with more than 2 channels are rejected** with
`SE_ERR_UNSUPPORTED_FORMAT` (a real downmix needs a channel-layout policy).
RF64/BW64 and compressed formats are rejected.

Writer: streaming `WAVE_FORMAT_IEEE_FLOAT` 32-bit with a `fact` chunk
(round trip is bit-exact, tested). Outputs over 4 GiB are refused.

## Resampling

Assets are converted to the engine rate on import with a Kaiser-windowed sinc
(β = 9, 32 zero crossings per side, 512-phase table with linear interpolation
between phases, cutoff 0.94 × the lower Nyquist). Measured in tests:
44.1 k → 48 k keeps a 1 kHz sine at the right frequency (±0.01 Hz) and
amplitude (±0.05 dB) with error below −66 dBFS against an ideal sine;
96 k → 48 k rejects a 30 kHz tone (alias at 18 kHz) by more than 80 dB.
Output length is `llround(frames × dst / src)`, time-aligned with no added
latency. It is offline-quality but unoptimised (scalar, about 70 taps per
output sample).

## Sample rate

`se_config.sample_rate` fixes the engine rate for the engine's lifetime
(assets are resampled to it). `0` means "use the default output device's
nominal rate" (falls back to 48 kHz). If the hardware runs at another rate,
the Core Audio output unit's built-in converter resamples.
`se_engine_sample_rate` reports the engine rate.

## Placeholders and known gaps

* **Windows** (`src/platform/wasapi_device.cpp`): stub, `se_device_start` →
  `SE_ERR_DEVICE` ("TODO: WASAPI/ASIO"). Also: `fopen` is not UTF-8 aware on
  Windows (needs `_wfopen`).
* **Linux** (`src/platform/pipewire_device.cpp`): stub, `SE_ERR_DEVICE`
  ("TODO: PipeWire/JACK").
* Core Audio: default output device only, no device selection, no handling
  of default-device changes or device rate switching, no input.
* No janitor thread: retired graphs are reclaimed on the next publish /
  asset release / destroy. With the device running and no edits, the
  previously retired graph (and any released assets it holds) stays alive
  until the next such call.
* Multithreaded track rendering, buses/sends, automation, plugins, and
  sample-accurate transport commands while the device runs (they apply at
  block boundaries; the loop wrap itself is sample-accurate) are not
  implemented.
* Loop wraps are hard cuts (no crossfade). Tempo changes are not modelled:
  the metronome grid is a constant `frames_per_beat` from frame 0.
* `se_meters_peek` track windows restart at 0 when a snapshot is published
  (same as `se_meters_read`).
* The resampler is scalar; large imports at a different rate take noticeably
  longer than same-rate imports.
* Output channels: only `output_channels == 2` is accepted.
