# PROGRESS: Device Studio

Living handoff document for future agent sessions. Read this first, then `docs/architecture.md`.

**Keep the README current.** `README.md` is the public front page. Any change that adds, finishes or removes a
feature, device, platform or build step, or visibly changes the screens it shows, updates the README (and
`docs/building.md`, re-rendered `docs/assets/` screenshots) in the same PR. The checklist is in README.md ›
Development › Keeping this README up to date.

## Current milestone

**Phase 1 (inspection + architecture) and the first slice of Phase 3 (functional audio).**

## Repository map

| Path | What | Owner role |
|---|---|---|
| `engine/` | C++23 real-time engine, C ABI `engine/include/dstudio_engine.h` | Audio engineering |
| `crates/dstudio-core` | Model, commands, undo/redo, persistence, snapshots | App architecture |
| `crates/dstudio-engine` | Safe Rust FFI wrapper + end-to-end tests | App architecture |
| `crates/dstudio-app` | Desktop app (Slint shell + wgpu surfaces) | UI engineering |
| `ui-spike/` | Slint ↔ wgpu interop spike (standalone workspace) | UI engineering |
| `docs/` | Architecture spec, ADRs | Lead |

## How to build and test

```bash
export PATH=$HOME/.cargo/bin:$PATH
cmake --preset default -S engine && cmake --build engine/build && ctest --test-dir engine/build   # engine
cargo test --workspace                                                                          # core + FFI + e2e
```

Windows: from an x64 MSVC developer prompt with Ninja; Linux: needs `pkg-config` and
`libpipewire-0.3-dev` (plus the X11/Wayland/xkbcommon/fontconfig/udev dev packages for the app;
see `.github/workflows/ci.yml`). Platform status: `engine/README.md` › Platforms.

## Status log

### 2026-11-08: session 1

**Implemented and verified**
- Engine (C++23): 34/34 ctest (Release and ASan/UBSan), zero allocations on the audio path, Core Audio playback
  (manual tool), bench: 128 tracks × 4 clips = 12.4 µs per 256-frame block (≈430× real-time, one thread, M4).
- Core (Rust): model, validated invertible commands, undo/redo with coalescing, persistence, snapshots,
  assistant layer (Assist/Teach), context-aware keymap: 28 tests.
- FFI + end-to-end: 6 tests (ABI layout, multitrack summing with sample-accurate placement, mute/solo/gain/pan,
  seek/stop/trim, peaks, export).
- UI rendering decision: Slint 1.18.1 + wgpu 30.0.1, offscreen texture via `Image::try_from` (ADR 0001):
  ≈0.15 ms CPU/frame for 8,192 clips at a locked 60 fps.

- App (`crates/dstudio-app`, binary `device-studio`): app shell + wgpu timeline with real waveforms,
  wired to core/engine (transport, mixer controls, import, clip move/trim, undo/redo, save/open).
  `--self-test` 21/21;
  60 fps with ≈0.06 ms of our render CPU per frame.
  Run: `cargo run --release -p dstudio-app -- --demo`.

**Known gaps / next steps**
1. Note preview/audition, velocity lane, clip loop, synth bypass + modulation/macros/effects; SIMD unison;
   splitters, track rename/resize, native menu bar.
   Engine: input/recording, loop crossfade, tempo map for the metronome.
2. Engine: device selection, input/recording, worker-pool parallel track rendering, WASAPI/ASIO and PipeWire/JACK backends.
3. MIDI: model + piano roll + built-in instrument; plugin hosting (VST3/CLAP/AU) with an out-of-process scanner.
4. Assistant: provider adapter (outside core), assistant panel, highlight overlay.
5. Original branding and icons before any public release; the Söhne display face must not ship.

### 2026-11-08: session 1, round 2 (engine fixes + fonts)
- Engine ABI v1 additions (append-only): `se_meters_peek` (non-destructive ~50 ms window), `se_transport_set_loop`
  (sample-accurate wrap), `se_metronome_set` (+ `se_metronome_desc`); transport commands now apply immediately
  when no device runs (race-free ownership hand-off; see engine/README.md). 46/46 ctest in default and ASan;
  bench unchanged (128 tracks ≈12.4 µs/block).
- App: Inter 4.1 bundled (OFL, `crates/dstudio-app/ui/fonts/`), sizes ×0.91 after a sweep.
  Loop (⌘L, button, `SetLoop` undoable) and metronome wired; offline-seek workaround removed; meters use peek.
  `--self-test` 26/26 offline and with Core Audio.

### 2026-11-08: session 1, round 3 (mixer + assistant panels)
- Mixer (header toggle / ⇧⌘X): per-track strips with real fader, pan, stereo `peek_meters`, M/S, peak hold;
  Master strip; add-track column.
- Assistant panel (header toggle / ⇧⌘A): Assist/Teach over `dstudio_core::assistant`, one undo step per turn,
  highlight overlay (`src/highlight.rs`), Anthropic Messages API provider (`src/assistant_provider.rs`, key from
  `ANTHROPIC_API_KEY` only, model `DSTUDIO_ASSISTANT_MODEL`, default `claude-opus-5-5`). **No live API call has been
  exercised yet** (no key in the dev environment); the pipeline is covered by the labelled TEST provider
  `--assistant-fixture`.
- Self-test 42/42 offline and with Core Audio; workspace tests 49 pass.

### 2026-11-08: session 1, round 4 (MIDI + piano roll)
- Core: MIDI clips/notes, commands (AddMidiClip/Remove/Move/Resize/SetMidiNotes), seeded note transforms, note
  snapshots in engine frames. Engine ABI additions `se_session_publish_v2` (+ note/track v2 structs); original
  16-voice polyBLEP synth with sample-accurate events, republish-safe voices: 60/60 ctest (default + ASan),
  pitch within ±0.5 Hz. Bench: 32 MIDI tracks × 8 voices ≈ 396 µs per 256-frame block (unoptimised).
- App: MIDI clips on the timeline, piano roll (wgpu) with editing, toolbar transforms, footer, context menu.
  Self-test 60/60 offline and with Core Audio, incl. a check that notes drawn in the piano roll sound on the
  instrument track.
- Requested core API: RenameClip command, clip loop fields; engine: note audition call.

### 2026-11-08: session 1, round 5 (synth Devices panel)
- Core: `SynthParams` contract + 6 original factory presets, `Command::SetSynthParams`, `Document::seal_coalescing`.
- Engine: `se_session_publish_v3` with `se_synth_params`; every parameter drives the DSP (polyBLEP/BLAMP oscillators,
  shape morph/fold, cross-FM, sub/noise, TPT SVF LP/HP/BP, ADSRs, LFO, mono legato/glide, unison) with 10 ms
  smoothing; 76/76 ctest (default + ASan). Bench 32 tracks: default patch 573 µs/block, heavy patch 3489 µs (65 % of
  deadline).
- App: Synth device card, preset browser, 6 real pages, live displays derived from
  the engine formulas; self-test 76/76 offline and with Core Audio, incl. a rendered-audio check that a panel cutoff
  change reaches the engine.

### 2026-11-08: session 1, round 6 (recording + input)
- Core: Track arm/monitor/input + commands; ADR 0002.
- Engine: device enumeration/config (private aggregate device for separate mic/speaker clocks), per-track input
  routing + monitoring, disk recording via SPSC rings + writer thread, latency-compensated takes, input-injected
  offline render. 94/94 ctest (default + ASan). Input opens ONLY when a device is configured by name/default.
- App: R/I buttons, In popup, record transport, live recording clip, Settings › Audio panel with saved preferences
  (`~/Library/Application Support/Device Studio/preferences.json`). Self-test 99/99 both modes (recording checks via
  injected input). The session rate is fixed at 48 kHz; the Audio page says so when a different device rate is picked.
- Real microphone verified (2026-11-08, with the user's permission): MacBook Air mic via the private aggregate
  device, 3 s take, 0 overruns, real room signal. An acoustic loopback test (metronome → speakers → mic,
  `DSTUDIO_TEST_REAL_INPUT=1 cargo test -p dstudio-engine --test real_loopback -- --ignored --nocapture`) found the
  latency compensation 18.5 ms too early (an extra IO buffer per side); fixed, residual now +2.7..6.2 ms late.
  Next: per-device manual recording offset (calibration) in Settings › Audio.

### 2026-11-08: session 1, round 7 (plugin hosting)
- Core: device chains (`Track.devices`, `Track.instrument_plugin`), commands, base64 plugin state,
  `Document::sync_device_states`; ADR 0003.
- Engine: AU/VST3 (SDK v3.8.1, MIT)/CLAP (1.2.10, MIT) hosting, publish v5, out-of-process scanner with cache and
  blocklist, sample-accurate instrument plugins, state save/restore, native editor windows; own test plugins
  (VST3/CLAP gain, CLAP sine, crashing CLAP). 111/111 ctest (default + ASan). Scan on the dev Mac: 68 plugins.
- App: browser Devices tab from the real catalog, device cards (power, editor, remove, reorder), Add device
  menus, Inst selector, Settings › Plugins with rescan, plugin state synced on save. Self-test 120/120 both modes.
- Screenshot states only show Apple's built-in AUs; never commit names of the user's installed plugins.

### 2026-11-08: session 1, round 8 (plugin delay compensation)
- Engine (ADR 0004): per-track alignment delays, whole-mix delay L, bypass keeps timing, monitored input skips the
  compensation delay, metronome/export/recording compensated, runtime latency changes via plugin_idle, 2^17-frame
  cap, global toggle (off = bit-identical to before). 125/125 ctest (default + ASan); bench L=0 unchanged.
- App: playhead and Bar/Time readouts show the audible position; Settings › Audio has a compensation switch
  (preference `pdc_off`) and a plugin-latency readout. Self-test 125/125 both modes, incl. an offline check that a
  4800-frame plugin puts the readout 100 ms behind the transport.
- Not run: acoustic loopback with plugins (`acoustic_loopback_latency_with_pdc`, opens the mic).

### 2026-11-08: session 1, round 9 (automation)
- Core: automation lanes (Volume dB, Pan, 20 synth params, plugin params; master volume), curve contract,
  `SetAutomationLane`; ADR 0005.
- Engine: publish v6 with lanes; evaluation bit-identical to core (135k random samples); volume/pan per ≤32
  samples (per sample near silence), synth at 16-frame control rate, plugin params as sample-offset events
  (VST3/CLAP/AU); PDC-aligned per stage incl. loop wraps; editor gesture capture for all formats. 139/139 ctest
  (default + ASan). Bench: 32 tracks × 4 lanes ≈ 0.6 % of the deadline.
- App: lanes toggle, header automation row with power + parameter picker, gold overlay lanes with labels,
  point/curve/marquee/freehand editing, Automation Arm touch-write, controls follow automation. Self-test 146/146
  offline, 145/145 (+1 offline-only skip) with Core Audio.

### 2026-11-08: session 1, round 10 (Windows and Linux port, branch `port/windows-linux`)
- CI green on macos-15, windows-2025 (MSVC /W4 /WX) and ubuntu-24.04 (GCC -Werror): engine build,
  145 ctest (139 + 5 FrameRing + 1 Win32 editor test; AU and device tests skip off macOS with a
  reason, CTest "Skipped"), `cargo build/test --workspace` (120 tests) on all three.
- Engine: `platform/os.{hpp,cpp}` (UTF-8 paths/_wfopen, env, ChildProcess over
  CreateProcessW/posix_spawn), scanner ported (per-OS folders, NTSTATUS crash detection), VST3/CLAP
  loading via LoadLibraryExW/dlopen with the VST3 bundle layout, canonical cross-platform VST3 class
  id strings, test plugins built as Windows/Linux modules.
- Audio: WASAPI shared/event/float32 backend (separate capture client + drift-tolerant
  `FrameRing`, MMCSS) and PipeWire backend (playback/capture streams). **Neither has run on real
  audio hardware** (CI runners have none); ASIO is future work (licence).
- Editors: Windows HWND host (tested in CI with `DStudioTestGui.clap`); Linux returns
  `SE_ERR_UNSUPPORTED_FORMAT` (X11 embedding not written).
- App: builds on all three; macOS-only winit hook stays cfg-gated; per-OS plugin folders. Off macOS
  the self-test reports its Audio Unit devices section as skipped.
- macOS-visible changes: two hardware-/compiler-dependent tests relaxed (PDC golden hashes only on
  Apple clang/arm64; output latency plausibility instead of `>= 512`), ABI header comment fixed
  (no buffer term in latency), `on_platform_main_thread` unchanged on macOS.
- CI now runs on the self-hosted Mac mini only (as on main, 30a5ccb); the hosted Windows/Linux matrix
  was dropped. The Windows/Linux results above are from hosted run 42464468427 (`6cecd4f`); later
  changes are not compiled on those systems until a self-hosted Windows/Linux runner is added.

### 2026-11-08: Windows and Linux back in CI

- CI is a three-job matrix again: macOS stays on the self-hosted Mac mini, Windows and Linux run on
  GitHub-hosted `windows-latest` / `ubuntu-latest` (engine `-Werror`/`/W4 /WX`, ctest, Rust
  workspace build + tests, app self-test). All three pass.
- Fix found by the Linux job: an import into the "selected track" was resolved when the import
  finished, so when imports finished out of order (the demo's second, new-track import first) the
  file landed on the newly selected track. The target track is now resolved when the import starts.
- Windows app self-test: 125/125 pass; the process still exits 0x87A afterwards (step stays
  informational, as before).

### 2026-11-10: step sequencer, part 1: pattern clips in core and engine (SAIN-37)

- Core (`steps.rs`): a pattern clip is a MIDI clip with a step grid (`MidiClip.pattern`): rate 1/1 to 1/32T,
  8 to 64 steps, swing, and Velocity, Offset, Repeat and Tie per cell. Step edits bake the notes
  (`Command::SetStepPattern`); piano-roll edits read only the changed rows back into the grid; `None` converts
  the clip to plain MIDI (its repeats become notes). The snapshot repeats the pattern over the clip. Old project
  files load unchanged.
- Engine: patterns play as ordinary notes; a new ctest checks that dense repeats republished while playing
  don't allocate, and bench_render has a Pattern table. `cargo bench -p dstudio-core --bench steps` times bake,
  read-back and snapshot builds.

### 2026-11-10: minimap pointer hide and Devices minimap (SAIN-33)

- Header minimap drags (window and zoom grabs) hide the pointer and put it back on the grabbed spot on release
  (`src/pointer.rs`: NSCursor + winit warp; self-test records the requests).
- Devices minimap at the right of the Clip / Devices strip (`src/devminimap.rs`, `DevMinimap` in `ui/app.slint`):
  tiles with glyphs, chain frames, window, jump / drag / wheel, Esc restores. Not done: the hover name.

### 2026-11-10: grid size and snap, clip-editor rulers, minimap pictures and motion (SAIN-33)

- Timeline toolbar snap control (`src/snap.rs`): the readout toggles snapping, the chevron opens Off / Auto / 1 Bar ...
  1/32 / triplet sizes. Auto follows the ruler step for the zoom (two beats at the default zoom), a
  fixed size overrides it when finer (doubled to 6 px), Cmd at the press drags off the grid.
- Piano roll and audio clip editor rulers and grids use the same `RulerSteps` rules (bar labels by power-of-two
  steps, ticks, 16 / 8 px sub and beat lines).
- Devices minimap tiles show pictures we draw from each device's own layout (`Art`), and fade in (300 ms) and glide
  (spring) when the chain changes; the strip redraws only while they run. `--drag-bench devmm`
  benchmarks it; `--state review-snap-menu` shows the menu.

### 2026-11-10: step sequencer, gaps closed (SAIN-37)

- Paging: the grid shows 16 or 32 steps at a time (by width), with page tabs and a Follow
  playhead button at the right of the toolbar. A MIDI clip longer than 64 steps now converts with its own step
  count (up to `STEP_LIMIT`, 4096) instead of being refused; Length and Double still stop at 64.
- While the transport plays inside the clip, the playing step's column has a white ring and the cells it hits
  flash (230 ms, Steps lane).
- Toolbar tooltips (Double, Clear, the lane tabs, Follow playhead) with a 1 s hover delay; Clear is
  disabled when the pattern has no cells.
- Value lanes: empty steps dark, Velocity filled from the bottom on a lighter base.
### 2026-11-10: pattern library, Genre list and Shape tab (SAIN-37, PR B)

- Genre now lists Any and 21 genres. The ten new ones come after the old eleven, because saved
  clips store the index. Each new genre has its own feel (scale, grid, swing, density, octave, progressions) and
  its own drum grooves, including two new grooves, Dembow and Warehouse pulse. Drum swing now follows the
  genre's feel, which gives the same result as before for the old genres. The Genre menu shows 12 rows and
  scrolls to the active one.
- Shape tab (`dstudio_core::shape`, `composer.rs`): Octave, Voices, Density, Length, Velocity, Nudge, Repeat and
  Arp reshape what the clip plays. The unshaped notes stay in `Composer::base`. Each part, and the kept
  notes, keep their own Shape. A slider drag is one undo step ("Shape Change"). If you edit notes after they
  were shaped, they play as edited under a neutral Shape. The other rack rows show disabled with
  tips.
- Review states: `--state review-patterns-3` (Shape on a lead) and `review-patterns-4` (the Genre menu).
- Next: Shape option A, the full rack with drawn curves and the remaining transforms.

### 2026-11-10: Shape rack, full (SAIN-37, PR C)

- `dstudio_core::shape` now has all 16 controls: Octave, Voices, Density, Length, Velocity, Vary, Nudge, Repeat,
  Air, Sub, Spread, Tension, Inversion, Arp, and the arp's Pattern and Octaves. Each one has a base value and a
  drawn curve (one offset per beat). Shapes saved before this change load unchanged.
- `shape_rack.rs` holds the rack rows (click a label to choose the curve to edit). The piano roll toolbar has
  Shape (draw), the Pencil, Steps and Ramp brushes, and Reset. The curves are drawn into the piano roll's wgpu
  draw list. A stroke or a slider drag is one undo step.
- Review states: `review-patterns-5` (Velocity and Density curves) and `review-patterns-6` (Air, a Tension
  curve, and the arp).

### 2026-11-11: Clip renders through plugins (CHR-38)
- Consolidate's sibling renders (Bounce to new track, Export audio) now render tracks with VST3, AU and CLAP plugins
  and instrument plugins. The UI thread prepares the instances in the private engine, one per tick, from the live
  instances' current state; the worker renders; the engine is dropped on the UI thread. New engine entry points
  `se_plugin_prepare`, `se_plugin_save_state`, `se_plugin_set_detached` (engine/README.md "Plugins in offline renders").
- The render honours plugin delay compensation (renders L more frames, drops the first L) and the project tempo.
- Self-test section "clip render plugin", ctest `offline_*`, `tests/clip_render.rs`, `dstudio_offline_plugin_bench`.

### 2026-11-09: Renamed to Device Studio (user-facing)

- Window title, binary (`device-studio`), built-in device vendor, preferences folder
  (`Device Studio` / `device-studio`), recording temp folder, CLAP host name, Core Audio aggregate and
  PipeWire stream names, assistant prompt and README/docs now say Device Studio.
- Projects save as `Name.dsproj/` with format `device-studio-project`, by default in `~/Documents/Device Studio`.
  (Opening projects from before the rename and moving the old support and projects folders over were supported
  at first, then removed on 2026-11-11: open old projects in an earlier build and use Save As with a `.dsproj`
  name, and move any projects kept in the old projects folder into `~/Documents/Device Studio` or point
  `DSTUDIO_PROJECTS_DIR` at it.)
  The macOS bundle id is `com.devicestudio.app` (microphone permission is asked again once).
- Internal names use `dstudio` (`dstudio-*` crates, `dstudio::` namespace, `dstudio_engine.h`,
  `DSTUDIO_*` CMake options and env vars, `DStudioTest*` test plugins, `studio.dstudio.test.*` ids).
  Not `device`: it already names the effects/instruments concept. The GitHub repo is `chrislaupama/device`.

### 2026-11-10: spec-driven workflow and agent skills

- `specs/` holds specs for non-trivial work (`specs/README.md`: write, plan, build, verify, with a stop for agreement
  after the spec and after the plan), templates in `specs/_template/`, and `specs/principles.md`. Spec 001 is the
  worked example.
- `AGENTS.md` is the entry point for coding agents; codebase skills live in `.agents/skills/` (commands, C ABI and
  audio thread, built-in devices, UI, benchmarks, pull requests, plus the spec phases and bug fixing).
  `python3 tools/agents/check_skills.py` fails when a skill names a file that no longer exists.

### 2026-11-11: return tracks and sends, part 1: core (spec 002)

- Spec `specs/002-return-tracks-and-sends/` (spec, plan, tasks). Return tracks are `TrackKind::Return` in
  `Project::tracks`, always after the ordinary tracks (at most 16); `Track::sends` (`TrackSend`: return, level 0..+6 dB,
  pre/post fader) and `Track::output` (a return, or the master). Commands `SetSends` (level drags coalesce per track)
  and `SetTrackOutput`; removing a return removes the sends to it and resets outputs to it in one undo step. Clips,
  input, arm, monitor and instrument plugins are refused on returns. Old projects load unchanged; damaged routing is
  repaired on load. The snapshot carries the routing; single-track renders stay dry.
- Not yet: the engine ignores the routing (returns publish as empty tracks) and the app has no UI for it. Next: engine
  (spec 002 T4-T8), then the UI (T9).

### 2026-11-11: track rename

- Double-click a track's name in its header, in the Devices tab's inspector or in the Clip tab's title to rename it
  (`TrackName` in `ui/track_header.slint`, `Controller::rename_track`): Enter or clicking away commits, Escape
  cancels, surrounding spaces are dropped and an empty name keeps the old one. `Command::RenameTrack` already existed.
- The field commits to the track it opened on, also when the selection moves first, and a press in the lanes or a
  clip editor commits it (`Controller::press_epoch` moves the focus back to the shortcuts). The name's accessible
  default action opens the field, and ⌘R (`track.rename`, a track selected and no clip) opens the selected track's
  header field, scrolled into view (a row taller than the lanes is centred).
- The open field mirrors its text to `Controller::rename_draft`; when the bottom panel holding it goes away (tab
  switch, mixer, selection cleared) `push_models` commits the draft, since the field never sees its focus loss, and
  gives the keyboard back to the shortcuts. A field records its draft as soon as it opens, so an untouched one counts.
- Rename and draft calls carry the `doc_epoch` the field opened in (`Studio.doc-epoch`): when another project
  replaces the open one (Open, Live Set import) the field closes and a late commit from it renames nothing.
- Undo of a rename is `Command::RestoreTrackName`, which puts the old name back exactly (an imported name over the
  128-character limit included). It is never deserialized, so the assistant cannot send one.
- Self-test section `track_rename_steps` drives the inspector name with real double-clicks and keys.

### 2026-11-11: piano roll velocity lane

- The velocity button in the piano-roll toolbar opens a 68-unit lane under the note grid (the grid gives up that
  height). Each note has a stem from the lane floor to its velocity (1 to 127). Dragging a stem moves the velocity
  of that note, or of the whole selection when the note is selected; a press on empty lane paints the stems the
  pointer sweeps across (the selection only, when there is one). Each gesture is one "Note Velocity" undo step.
- Code: `pianoroll.rs` (`VEL_H`, `vel_y`/`vel_at`, `PrDrag::Velocity`, `pr_vel_pointer`, `paint_velocity`, the
  `vel_lane` draw). Review state `review-pr-velocity`; self-test section "velocity"; unit test
  `velocity_lane_maps_and_paints`.

### 2026-11-11: MIDI input, engine side (spec 003)

- Spec `specs/003-midi-input-and-recording/` (spec, plan, tasks). This first part is the engine and the C ABI; the
  app side (R/I on instrument tracks, Settings › MIDI inputs, recorded MIDI clips) follows.
- Engine: MIDI input backends (Core MIDI, Windows `midiIn*`, ALSA sequencer, stub) behind `MidiBackend`;
  `MidiInputHub` (per-port SPSC queues, normalisation, held-note release on close); routing in `render_block` to
  armed/monitoring instrument tracks, else the focus track; `LiveSynth` plays the built-in Synth live; capture ring
  with latency compensation and loop folding. `se_midi_*` in the header, `Engine::midi_inputs` and friends in Rust.
- Core: `SetTrackArm` / `SetTrackMonitor` accept instrument tracks.
- Tests: `engine/tests/test_midi_input.cpp`, `crates/dstudio-engine/tests/midi_input.rs`; bench: "MIDI input"
  table in `dstudio_engine_bench`.
- Not verified on hardware yet: Core MIDI with a real keyboard (Mac mini), WinMM (CI build only).

