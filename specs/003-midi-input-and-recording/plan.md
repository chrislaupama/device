# Plan: MIDI input and recording

Spec: [spec.md](spec.md)

## Approach

MIDI comes in on a platform thread, crosses to the audio thread through lock-free queues, and is
routed there, per block, to instrument tracks using flags the session snapshot already carries
(`arm`, `monitor`, `instrument`). The audio thread also writes what it delivered, with timeline
positions, into a capture ring that the app drains while recording. Turning captured events into
notes and clips happens in Rust, where clips, undo and the timeline live.

Alternatives considered:

- **Recording in the engine** (like audio takes, with a writer thread and take files). MIDI takes
  are tiny and become project data (notes), not media files, so the engine only needs to report
  positions; building clips in core keeps the logic testable without the engine and keeps one
  mutation path.
- **Routing in the app** (the app reads MIDI and calls `se_preview_note`). That adds a frame of UI
  latency (16 ms or more) and depends on the UI thread being responsive. Routing on the audio
  thread costs a few comparisons per track per block.
- **Feeding the built-in Synth's timeline renderer with fake "now" notes.** The built-in Synth
  (`engine/src/synth.cpp`) renders notes from a sorted list on the timeline and has no live note
  entry point. Rather than change its internals, each instrument track that uses it gets a second
  `Synth` driven through the same public `render()` interface with its own free-running clock: a
  key press becomes a note that starts "now" and ends at its key release. Live notes therefore
  never steal sequenced voices, and the Synth code is unchanged.

## Changes by layer

- **Engine** (`engine/`)
  - `src/platform/midi_backend.hpp` and one backend per platform: `coremidi_input.cpp` (Core MIDI,
    MIDI 1.0 protocol port), `winmm_midi_input.cpp` (Windows multimedia `midiIn*`),
    `alsa_midi_input.cpp` (ALSA sequencer, one reader thread), and a stub. They list ports (id,
    name) and open/close them, calling back with raw 1 to 3 byte channel messages. CMake option
    `DSTUDIO_ENGINE_ALSA_MIDI` (default ON on Linux; `libasound2-dev` is already a documented
    dependency).
  - `src/midi_input.hpp/.cpp`: `MidiInputHub`. Fixed slots (16 device ports plus one injected
    source), each with a preallocated SPSC queue (1024 events). The audio thread drains all slots
    at block start (at most 256 events a block), normalises messages (channel 1, note-on velocity
    0 becomes note-off), drops note-offs for notes that are not held (closing a port floods
    note-offs safely), counts drops, and keeps per-slot activity counters for the UI.
  - Per-track live state `TrackMidi` (held-note bitmap plus, for the built-in Synth, a
    `LiveSynth`) kept across republishes by track id, exactly like `synth_refs`.
  - `render_block`: for each instrument track decides whether it receives input (armed or
    monitoring tracks if any exist, else the focus track), delivers the block's events (prepended
    to the plugin or built-in instrument's event list, or played on the `LiveSynth`), and sends
    note-offs for held notes when a track stops receiving.
  - `Engine::process`: drains the hub once per callback, and while capture is on and the
    transport plays, writes note events to a capture ring (4096 events, RT producer, main thread
    consumer) at `position - (output latency + delay compensation)`, folded back into the loop
    after a wrap.
- **C ABI** (`engine/include/dstudio_engine.h`, `crates/dstudio-engine`): `se_midi_input_info`,
  `se_midi_inputs_list`, `se_midi_inputs_open`, `se_midi_inject`, `se_midi_set_focus`,
  `se_midi_capture_enable`, `se_midi_capture_read` (`se_midi_captured`), `se_midi_capture_wrapped`
  (a real loop wrap, never a seek), `se_midi_stats_get`.
  Additive; `SE_ABI_VERSION` stays 1. Rust mirror, layout tests and safe wrappers.
- **Core** (`crates/dstudio-core`): `SetTrackArm` / `SetTrackMonitor` accept instrument tracks.
  New `midi_record` module: pure function from captured events (frames) plus pass info (start,
  stop, loop) to MIDI clips per track, unit tested. No schema change.
- **App** (`crates/dstudio-app`): R and I on instrument tracks; record start/stop drive MIDI
  capture alongside audio, one "Record" batch; live "Recording" clip with notes; no focus track
  (only armed or monitoring tracks play, Q1); Settings › MIDI "MIDI inputs" list with switches and activity lights, and the
  `midi.disabled` preference; inputs rescanned every two seconds; self-test steps with injected
  MIDI; a `--state` review state for the MIDI page.
- **Docs**: README (features, status), PROGRESS.md, `docs/architecture.md` (MIDI input thread),
  `engine/README.md` (MIDI input section), `docs/building.md` (ALSA option).

## Principles check

1. Real-time safety: queues and states are allocated off the audio thread; the RT path only pops,
   copies and compares. `RtAllocGuard` test over a block with input.
2. One mutation path: arm, monitor and the recorded clips are `Command`s; capture data is not
   project state.
3. Projects keep loading: no new fields; `arm`/`monitor` already persist.
4. ABI boundary: new `se_midi_*` functions only, mirrored by hand with layout tests.
5. Cross-platform: three backends in `engine/src/platform/`, stub elsewhere; CI builds all three.
   Core MIDI is checked by hand on the Mac mini with a real keyboard.
6. Original work: all new code.
7. Evidence: engine and core tests, self-test, Settings › MIDI screenshot, bench numbers.
8. Docs: listed above.
9. Naming: `MidiInputHub`, `TrackMidi`, `LiveSynth`, `se_midi_*`; no bare "device".

## Tests and evidence

- Tests: `engine/tests/test_midi_input.cpp` (routing rules, built-in Synth, built-in instrument,
  CLAP test instrument, held-note release, no allocation, drops, capture positions and loop fold);
  `crates/dstudio-engine/tests/midi_input.rs` (Rust wrapper end to end); core `midi_record` unit
  tests; self-test steps for arm/monitor and a recorded pass.
- Benchmarks: `dstudio_engine_bench` before and after (no input: no change expected), plus a case
  with input notes routed to a Synth track.
- Screenshots: Settings › MIDI before and after; an armed instrument track header before and after.

## Risks

- Core MIDI and WinMM cannot be built or run in the Linux container: CI builds them; the Mac mini
  checks Core MIDI with a real keyboard. WinMM is checked by CI build only until someone tries a
  Windows machine.
- Block-start timing: jitter up to one buffer. Documented; sub-block timestamps are follow-up work.
