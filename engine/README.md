# Saine Studio audio engine (`engine/`)

C++23 real-time audio engine behind the C ABI in `include/saine_engine.h`.
The Rust core links the static library `saine_engine`. The header is the
contract. `SE_ABI_VERSION` is still 1: the first milestone's structs and
signatures are unchanged, and later features are **appended** at the end of
the header (`se_meters_peek`, `se_transport_set_loop`, `se_metronome_desc` +
`se_metronome_set`, for MIDI `se_note_desc`, `se_track_desc_v2`,
`se_session_desc_v2` + `se_session_publish_v2`, and for synth parameters
`se_synth_params`, `se_track_desc_v3`, `se_session_desc_v3` +
`se_session_publish_v3`, `se_synth_params_default`, and for input and
recording `se_track_desc_v4`, `se_session_desc_v4` + `se_session_publish_v4`,
`se_device_list`, `se_device_configure`, `se_device_status_get`,
`se_input_set_muted`, `se_record_start/stop/get_take/state/peaks`,
`se_record_stats_get`, `se_render_offline_with_input`).

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

./build/saine_engine_bench [blocks=4000] [clips_per_track=4] [features=0]  # 1 = loop + metronome on; also prints a MIDI table
./build/play_sine_or_wav [file.wav] [seconds=1.0]   # real default output device, -24 dBFS
./build/record_check                                # list devices (enumeration only, opens nothing)
SAINE_TEST_REAL_INPUT=1 ./build/record_check [s=2]  # MANUAL: opens the real default input (macOS mic prompt!)
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
| `src/synth.{hpp,cpp}` | Built-in polyphonic instrument (`Synth`, one per instrument track) |
| `src/spsc_queue.hpp` | Lock-free SPSC ring for transport commands |
| `src/wav.{hpp,cpp}` | Hand-written RIFF/WAVE reader, streaming float32 writer |
| `src/resampler.{hpp,cpp}` | Kaiser-windowed sinc resampler (import time) |
| `src/rt_util.hpp` | Flush-to-zero guard, clock, atomic max |
| `src/platform/` | `AudioDevice` interface + backends (Core Audio real; WASAPI/ASIO, PipeWire/JACK stubs) |
| `tests/` | Self-contained harness (`test_harness.hpp`), tests, allocation-tracking `operator new` |
| `bench/bench_render.cpp` | Offline render benchmark |
| `src/recorder.{hpp,cpp}` | Recording: RT capture into per-track SPSC rings, WAV writer thread, live takes/peaks |
| `src/test_hooks.hpp` | Test-only hooks (fake device factory, writer stall, ring size); not public ABI |
| `tools/play_sine_or_wav.cpp` | Manual check through the real default output device |
| `tools/record_check.cpp` | Device listing; with `SAINE_TEST_REAL_INPUT=1` a 2 s real-input recording (manual only) |

## Design

### Data flow

```
Rust core (non-RT)                                  audio thread (RT)
------------------                                  -----------------
se_asset_load_file ─ decode WAV ─ resample ─► asset registry (shared_ptr<const Asset>)
se_session_publish(_v2/_v3) ─ copy + validate + build ─► RenderGraph ──atomic ptr swap──► process()
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
* **Instrument tracks** (`se_session_publish_v2`, `instrument != 0`): the
  track's notes render through its `Synth` (mono), summed with the track's
  audio clips, then the same gain/pan/mute/solo/meters as audio. Notes are
  validated (pitch 0..127, velocity 1..127, `notes` non-NULL when
  `note_count > 0` → else `SE_ERR_INVALID_ARG`), zero-length notes dropped,
  and sorted by (start, pitch). Notes on non-instrument tracks are validated
  but ignored. `se_session_publish` builds the same graph with no
  instruments, `se_session_publish_v2` the same with default synth
  parameters (both convert to v3 internally).
* **Synth parameters** (`se_session_publish_v3`): each track may point to an
  `se_synth_params` (NULL = defaults = `SynthParams::default()` in
  saine-core). They are copied into the graph (`RenderGraph::synth_params`,
  reserved bytes zeroed), so they change atomically with notes and mix.
  Validation: enums in range, every float finite and inside the documented
  range (same ranges as `SynthParams::validate`) → else `SE_ERR_INVALID_ARG`.

### Threading model

| Thread | What it may do |
|---|---|
| Audio thread (Core Audio IO proc) **or** the thread inside `se_render_offline(_with_input)` | `Engine::process()` only |
| Recording writer thread (one per recording) | drains the recorder's rings into WAV files |
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

### Built-in instrument (MIDI + `se_synth_params`)

An original subtractive voice (`src/synth.hpp`); it does not model any
commercial instrument. Every Devices-panel parameter maps onto it:

| Parameter | DSP |
|---|---|
| `oscN_wave` | Saw: polyBLEP falling ramp. Square: polyBLEP pulse, DC removed. Triangle: polyBLAMP (sine-aligned; ~13 dB less aliasing than naive). Sine: odd degree-9 polynomial (harmonics < −100 dB). |
| `oscN_shape` | Square: pulse width 50 % → 5 %. Saw ↔ triangle: linear morph toward the other shape (phases aligned so the fundamental never cancels). Sine: wavefolder `sin(π/2 · 5·shape · sin θ)` normalised to ±1 (continuous to a pure sine at 0; odd harmonics only; not band-limited). |
| `oscN_octave/semitones/fine_cents` | Offsets added to the note pitch; octave/semitone switch at the next control tick, fine is smoothed. |
| `oscN_level` | Linear mix gains (smoothed, ramped per sample). |
| `cross_fm` | Linear FM osc2 → osc1, per sample: `inc1 · (1 + 0.9 · cross_fm · osc2)`, clamped to [0.02·inc1, 0.45]; so the deviation is at most ±90 % of the carrier frequency (not through-zero; band-limiting uses the instantaneous increment). |
| `sub_level` | Sine one octave below osc1's pitch (incl. glide/LFO, without FM/unison). |
| `noise_level` | White noise (xorshift32, seeded per note from (start, pitch), so export == live). |
| `filter_mode` | TPT/ZDF state-variable filter, 12 dB/oct: LP, HP, BP (BP normalised to unity gain at the centre). Stable for any coefficient change. |
| `filter_cutoff_hz` | Cutoff at C4 when key tracking is on: `fc = cutoff · 2^(key_track·(pitch−60)/12 + 6·env_amount·fenv + lfo·lfo_to_cutoff_oct)`, clamped to 10 Hz .. 0.45 × rate. |
| `filter_resonance` | Q = 0.707 · (25/0.707)^res: 0 = Butterworth, 0.5 ≈ 4.2 (+12.5 dB), 1 = 25 (+28 dB, below self-oscillation). |
| `filter_key_track` | Octaves of cutoff per octave of pitch (uses the glided pitch, not the LFO). |
| `filter_env_amount` | ±1 → ±6 octaves × filter envelope. |
| `amp_*`, `filter_*` ADSR | Linear attack over `attack`; exponential decay (distance to sustain falls 60 dB over `decay`); sustain; exponential release (−60 dB over `release`). Amp env per sample; filter env per control tick (≤ 0.33 ms late on stage changes). Voice freed at −90 dB in release, or in decay with sustain 0. |
| `lfo_*` | Sine/triangle/square/saw (all start at 0 or +1 and rise), `rate_hz`, to pitch (cents), cutoff (octaves) and amplitude (tremolo: gain between `1 − depth` and 1). Output de-clicked by a ~1 ms one-pole. **Per voice, phase 0 at each fresh note-on** (deterministic and identical in export); a mono legato note keeps the running LFO. |
| `polyphony` | 1..16 note voices; see below. |
| `glide` | Portamento, linear in pitch, covering any interval in `glide` seconds. Works in mono **and** poly: a fresh voice glides from the previous note-on's pitch (cleared by transport jumps, so the first note after a seek/loop wrap doesn't glide). |
| `unison`, `unison_detune_cents` | 1..4 stacks of (osc1 + osc2) **inside one voice** (they do not use voice slots or count against polyphony), detuned evenly over ±detune/2, start phases spread, level normalised by 1/√n (so RMS stays about the same; mono output, no stereo spread). |
| `output` | Linear post-filter gain (smoothed). |

Fixed: per-voice gain = velocity/127 × 0.25 (`kVoiceGain`), equal
temperament, A4 = MIDI 69 = 440 Hz (C4 = 60). The defaults (saw −5 ct
level 0.6 + square +5 ct level 0.5, LP 2093 Hz at C4 tracked 1:1, amp ADSR
5 ms / 200 ms / 0.6 / 250 ms) sound close to the previous fixed patch. Both
oscillators of stack 0 start at a phase where the waveform is non-zero (osc2
half a cycle after osc1), so the note-on frame itself is non-zero.

**Control rate.** Each voice recomputes pitch increments, filter
coefficients, LFO, filter envelope, glide and the smoothed parameters every
16 samples of its own lifetime (a per-voice grid from its note-on), and
ramps its gains linearly per sample inside the tick (value = start + slope ×
position-in-tick). So the output is still bit-identical for any block
partition and does not depend on where in the timeline a note starts
(tested). Transcendental work (`exp2`, `tan`) is skipped while its inputs
are unchanged.

**Parameter changes (republish).** A new graph seq makes the RT side apply
the track's parameters (a `memcmp` skips identical ones). Continuous
parameters are approached per voice by a one-pole at control rate (time
constant 10 ms, snapped when within 1e-6), so a knob sweep published as many
snapshots does not zipper or click (tested: the largest sample step during
0.2 ↔ 2.0 output and 300 Hz ↔ 8 kHz cutoff jumps every 5 ms equals the
steady signal's own). New voices start at the target values. Discrete
parameters (waveforms, filter mode, octave/semitone, unison count, LFO wave)
switch at each voice's next control tick (a filter mode switch can click).
A lower `polyphony` releases the oldest held voices (normal release).

**Polyphony and voices.** `polyphony` note voices per track (max 16). A note
beyond the limit steals the oldest note voice (note-on order): it fades out
linearly over 5 ms in its slot while the new note takes a free slot (24
slots = 16 + 8 for fades; only if all 24 are busy is the quietest fading
voice overwritten).

**Mono (`polyphony` = 1), last-note priority.** A note-on while the voice is
*held* (notes overlap) is legato: no envelope retrigger, velocity unchanged,
the pitch glides (or jumps at the note-on frame when `glide` = 0). A
note-on while the voice is *releasing* (e.g. touching notes: the note-off
and note-on fall on the same frame) retriggers both envelopes' attacks from
their current level (no click) and takes the new velocity; the pitch still
glides from where it is. Otherwise a fresh voice starts. The voice follows
the newest note's end; there is no return to an earlier still-held note.
Simultaneous notes collapse to one voice (the highest pitch of a chord,
since notes are sorted by (start, pitch)).

**State across republishes.** `Synth` objects are allocated on the non-RT
side, preallocate every voice, and are owned by `shared_ptr`s in each graph
(`RenderGraph::synth_refs`, raw pointers in `synths` for the RT path).
`Engine::install_graph` (under `control_mutex_`) gives each instrument track
the `Synth` of the same `track_id` in the currently published graph, else a
new one. The RT side is the only thread that mutates a live `Synth`; a
`Synth` whose track disappeared is freed together with the last retired
graph that holds it, i.e. by the existing reclamation protocol, never on the
audio thread. When the RT side sees a new graph seq, held voices are
re-matched by `(start, pitch)`: a match keeps sounding and follows the note's
new `end` (if that is already behind the playhead, it releases at the next
segment start); no match → normal release. So editing elsewhere, or
lengthening/shortening a sounding note, never retriggers or cuts it.

**Events.** `Synth::render` splits each render segment at every note-on
(`start`) and note-off (`end`) frame, so both are sample-accurate and the
output is bit-identical for any block partition (tested). Note-ons are found
by binary search for `start ∈ [segment)`, so no cursor has to survive
seeks.

**Transport jumps.** `rt_synth_gen_` is bumped on seek, stop, pause and every
loop wrap; a `Synth` that sees a new generation (or a timeline position
other than where its last segment ended) fades every sounding voice over
5 ms. Notes that start exactly at the new position play; notes already in
progress there are **not** retriggered (simple, deterministic). While paused
or stopped, `render_block` still renders those fade tails (nothing else), so
pausing mid-note does not click.

**Mute.** Synths of muted / non-soloed tracks keep running (output not
mixed), so unmuting mid-note continues the note exactly as if it had never
been muted. This costs CPU for muted instrument tracks.

### Export

`se_export_wav` renders `[start, end)` linearly with its private transport:
it ignores the live loop and metronome (tested by
`export_ignores_loop_and_metronome`). Instrument tracks render through
private `Synth` objects created for the export, so exporting never touches
live voices (tested: live output with an export in the middle is identical
to an uninterrupted render, and the exported file matches it bit for bit).

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
   that the render drains; and across 1500 renders of 8 instrument tracks ×
   200 notes with a loop, seeks and pause/play, while another thread
   republishes four different note sets as fast as it can; and across 1200
   renders of 8 instrument tracks whose `se_synth_params` (waveforms, filter
   mode, unison, polyphony incl. mono + glide, FM, LFO, ...) are republished
   every 3 blocks by the render thread and continuously by another thread).
   Synth parameter changes are applied on the RT side without allocation
   (plain struct copy + coefficient math). Also run ad hoc under
   ThreadSanitizer (MIDI, synth, RT, transport and concurrency tests clean).
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

## Input, monitoring and recording

(ADR: `docs/adr/0002-recording.md`.)

### Devices (`se_device_list`, `se_device_configure`, `se_device_status_get`)

Enumeration reads Core Audio properties only (UID, name, input/output
channel counts from the stream configuration, the standard nominal rates
inside `AvailableNominalSampleRates`, default input/output); it never opens
a device. `se_device_configure` validates first, again without opening
anything: unknown UID → `SE_ERR_NOT_FOUND`; a device without the needed
channels, an unsupported rate, a buffer outside 16..4096 or the device's
`BufferFrameSizeRange`, or an input combined with a rate other than the
engine rate → `SE_ERR_INVALID_ARG`. Stopped: the setup is stored for the
next `se_device_start`. Running: the device is stopped and restarted in
place (the engine keeps RT ownership across the restart, so no offline
render can slip in, and queued transport commands wait for the new stream).

**Input is opened lazily.** A fresh engine (and `se_device_start` without a
prior configure that enabled input) opens the output only, exactly as
before. An input is opened only after `se_device_configure` with
`input_enabled` (Rust: `DeviceConfig { input: Some(uid) }`, or
`Some(DeviceConfig::DEFAULT_INPUT)` = `""` for the system default input;
`input: None` = output only). If the output opens but the input
does not (microphone permission denied/restricted, which we read via
`AVCaptureDevice authorizationStatusForMediaType:` through the ObjC runtime
without prompting; aggregate creation failure; device gone; rate mismatch),
the device restarts **output only**, `se_device_status.input_open = 0`,
`input_error` explains, and a running reconfigure returns `SE_ERR_DEVICE`
with that message. Output keeps working.

### Duplex on macOS: private aggregate device (and why)

Input and output on different Core Audio devices (MacBook microphone +
speakers) run on independent clocks. We create a **private, non-persistent
aggregate device** (`kAudioAggregateDeviceIsPrivateKey = 1`, visible only to
this process, destroyed on stop and by the system if we crash) whose
sub-device list is [output, input], main (clock) sub-device = the output,
and **drift compensation on the input sub-device** (high quality). One
`HALOutput` AUHAL runs on it with input enabled; inside the output render
callback we pull input bus 1 with `AudioUnitRender` into preallocated
buffers, so each IO cycle delivers matching input and output frames. If
input and output are the same device, the AUHAL runs on it directly (no
aggregate).

Chosen over a separate input AUHAL + lock-free ring + adaptive resampler
because: the HAL's drift compensation is what Logic/GarageBand rely on and
is maintained by Apple; it keeps a single IO thread and a single callback,
so input and output are inherently sample-aligned per cycle (no ring fill
level to servo, no extra buffering latency, no second RT thread); and the
HAL reports the aggregate's latency and safety offsets for compensation.
Costs: aggregate creation takes up to a few hundred ms, and AUHAL does not
resample on its input side, so with an input the hardware is switched to the
engine rate (devices that cannot run at it are rejected by validation).
Output-only setups keep `DefaultOutput` (follows the system default) or a
`HALOutput` bound to the named device, with the unit's converter if the
device rate differs from the engine rate.

### Routing and monitoring (`se_track_desc_v4`)

Per track: `input_mode` (none / mono / stereo) and `input_channel`. Mono
feeds one device channel to both sides of the track, then the pan law;
stereo feeds `channel, channel+1` (pan = balance). Missing channels read as
zeros. `render_block()` looks at input only if the graph has a routed track
(`RenderGraph::uses_input`), so sessions without input cost one branch.

* `monitor` on: the input is summed with the track's clips/notes and goes
  through the track's gain, pan, mute, solo, meters and the master, **also
  while the transport is stopped**.
* **Armed, not monitored ("meter input when armed")**: the input is not
  heard, but the track meters (`se_meters_read` and `se_meters_peek`) show
  the raw input peak per side, **pre-fader**, so levels can be set before
  and during recording. (Armed and monitored: the normal post-fader meter.)
* `se_input_set_muted(1)` replaces every input channel with zeros
  (monitoring, input meters and recordings; recordings keep their timing).

### Recording

```
audio thread (process, per segment)            writer thread (~every 5 ms)
-----------------------------------            ---------------------------
rec = rec_live_ (loaded once per block)        pop markers; read each ring up
for each armed track:                            to its head; at a marker:
  new take?  -> push {start, ring index}         finish the WAV, open the next
             to the track's marker SPSC        write frames, update live takes
  ring full? -> drop block, count overrun,       and 64-frame min/max peaks
             end the take                         (mutex, never seen by RT)
  else       -> interleave into float SPSC
             ring, publish head (release)
```

`se_record_start(dir)` takes the armed tracks with an input from the
published graph (fixed for the take), allocates one ring per track (2 s at
the engine rate, power of two), starts the writer thread, publishes the
recorder to the RT side and queues Play (no-op if playing). From the next
block, every playing segment is copied raw (pre-fader, pre-mute) into the
rings. **RT argument:** the audio thread only does `memcpy`-style copies into
preallocated memory, two SPSC pushes with acquire/release atomics, and
relaxed counter increments: no allocation (tested under the allocation
tracker with 8 armed+monitored tracks, seeks, loop wraps and input mute), no
locks, no I/O, no waiting. A full ring drops that block for that track and
counts it (`se_record_stats`); the take ends and the next block that fits
starts a new take at its own timeline position, so takes stay sample-aligned
and the audio thread never blocks on the disk.

**Takes and transport.** A take is a run of contiguous playback. A seek or
stop (or any jump) ends it; the next captured block starts a new take at
the new position. Pause + play at the same position continues the take.
**Recording while looping writes linearly** (no take lanes): the loop wrap
does not end the take, the file just keeps growing, and the take is placed
from its start frame (so it extends past the loop end on the timeline).
`se_record_stop` leaves the transport as it is.

**Stop.** `rec_live_ = null` (seq_cst), then wait while the RT side's
`rec_in_use_` flag is set (it is set before loading `rec_live_` at block
start and cleared at block end: a Dekker pair, so either the block sees null
or the stopping thread waits for that block). Only the non-RT stopping
thread waits. Then the writer drains everything left (also when stalled),
patches every WAV header (RIFF/data/fact sizes) and exits. Failed writes are
counted in `write_errors` and those takes are left out of the result.

**Latency compensation.** In a duplex cycle the input sample delivered with
output frame P was captured when the listener heard frame P − (input latency
+ output latency). So `start_frame = position − (in + out)`, with each side =
`kAudioDevicePropertyLatency + kAudioDevicePropertySafetyOffset +` the first
stream's `kAudioStreamPropertyLatency` (no IO-buffer term: an acoustic loopback test on a MacBook Air showed the buffer term over-compensated by ~18.5 ms; without it recordings land +2.7..6.2 ms late, consistent with speaker→mic travel and onset detection; see crates/saine-engine/tests/real_loopback.rs), read from the running
(aggregate) device; for an aggregate each side is the max of what the
aggregate and the sub-device report. Converted to engine frames. Injected
input (`se_render_offline_with_input`) has compensation 0.

**Live view.** `se_record_state` lists every take (track, start, frames as
written by the writer, so ≤ ~5 ms behind); `se_record_peaks` gives min/max
pairs of a track's newest take at 64-frame resolution. Both take the
recorder mutex for a copy and never touch the RT side.

**Files.** `rec-<YYYYmmdd-HHMMSS>-<n>-t<track>-<take>.wav` in `dir`
(created if missing), 32-bit float at the engine rate, mono or stereo; an
existing name gets a `_2`, `_3`, ... suffix.

### Tests (no real input is ever opened)

`tests/test_recording.cpp` uses injected input or a fake `AudioDevice`
(`test_hooks.hpp`): mono/stereo routing through the pan law (bit-exact),
monitoring while stopped, monitor off, input + clip sum, armed-not-monitored
meters equal to the raw input peak with silent output, input mute, track
mute/solo; recording a ramp across odd block sizes (127/1/333/1000/64/255,
bit-exact file, exact `start_frame`, live state and peaks), two tracks on
different channels (mono + stereo), seek → new take, loop → one linear take,
pause/play continuity, error cases, writer-stall overruns (exact counts,
both takes aligned and valid after stop), zero allocations while recording
and monitoring 8 tracks, the fake device path (default start never enables
input, input failure keeps output running and reports the error, take
`start_frame` compensated by input + output latency), configure validation
against real Core Audio (no start), and a real output-only named-device
start + running reconfigure. Rust: `crates/saine-engine/tests/recording.rs`
(Document → snapshot → injected input → take → `load_file` → "Record" Batch →
playback renders the same samples).

### Verification status

Not verified on hardware by the engine agent, by design (it would trigger
the microphone prompt): opening a real input, the aggregate device and its
drift compensation, AUHAL input pull, and the latency values Core Audio
reports for a real duplex setup. Run `SAINE_TEST_REAL_INPUT=1
./build/record_check` by hand to check level and reported latency; a
loopback measurement (play a click, record it) is the next step to confirm
the compensation is sample-accurate on real devices.

## Placeholders and known gaps

* **Windows** (`src/platform/wasapi_device.cpp`): stub, `se_device_start` →
  `SE_ERR_DEVICE` ("TODO: WASAPI/ASIO"). Also: `fopen` is not UTF-8 aware on
  Windows (needs `_wfopen`).
* **Linux** (`src/platform/pipewire_device.cpp`): stub, `SE_ERR_DEVICE`
  ("TODO: PipeWire/JACK").
* Core Audio: no handling of default-device changes, device hot-unplug or
  externally triggered rate changes while running (the stream just keeps
  whatever Core Audio does). **The duplex/aggregate input path has not been
  run against real hardware by the engine agent** (see "Input, monitoring and
  recording" > Verification).
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
* MIDI: no sustain pedal / pitch bend / CC / mod wheel, no velocity →
  filter routing, no per-note pan or stereo unison spread (the synth is
  mono). Removing a track or switching it from instrument to audio cuts its
  voices without a fade. A mid-note seek never retriggers the note (no
  "chase"). The sine wavefolder is not band-limited (aliases at high fold
  and pitch); the filter has no drive/saturation and does not
  self-oscillate. Cost is about 9 ns per voice-sample for the default patch
  and ~50 ns for a 4-voice-unison FM patch on Apple silicon (bench below);
  muted instrument tracks still cost CPU. Unison stacks are scalar (SIMD
  across the 4 stacks is the obvious next optimisation).

## Benchmark (Apple silicon, Release, 256-frame blocks @ 48 kHz, deadline 5333 µs)

| Case | 8 tracks | 32 tracks | 128 tracks |
|---|---|---|---|
| Audio, 4 clips/track (avg µs/block) | 1.0 | 3.2 | 12.3 |
| Audio, 4 clips/track, after the input/recording change (same machine, HEAD baseline in parentheses) | 1.06 (1.09) | 3.36 (3.37) | 12.6 (12.7) |
| MIDI, 8 held voices + 1 retriggered note/track, previous fixed synth | 98 | 396 | 1594 |
| MIDI, same notes, `se_synth_params` defaults | 140 | 573 | 2246 |
| MIDI, same notes, heavy patch (unison 4, cross FM, folded sine, triangle morph, sub, noise, resonant BP + filter env, LFO → pitch/cutoff/amp) | 870 | 3489 | 13782 |

Input (8 armed + monitored tracks, injected 8-channel input, no clips):
1.33 µs/block with recording off; 5.9 µs/block avg (p99 11 µs, max 32 µs)
with recording on, i.e. ~0.1 % of the deadline for 8 recorded tracks. The
recording case pauses (untimed) every 256 blocks so the writer keeps up,
because offline rendering runs ~1000x faster than real time and would
otherwise only exercise the overrun path. The extra cost is the interleaved
copy into the 8 rings while the writer thread reads them on another core.

(Default patch, 32 tracks: ≈ 1.45× the previous synth, ~11 % of the block
deadline. The heavy patch runs 8 oscillators per voice: 32 tracks × 8 voices
fit in ~65 % of the deadline; 128 such tracks do not run in real time.)
