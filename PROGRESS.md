# PROGRESS: Saine Studio

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
| `reference/` | Reference audit, UI inventory, design tokens, interactions, parity matrix, research | Reference research |
| `reference/screenshots/` | Captures of the reference app at 1440×820 (git-ignored: third-party art and personal library data) | Reference research |
| `reference/data/` | Raw accessibility dumps (geometry source of truth) | Reference research |
| `reference/bundle-contents.md` | What is in the reference app (`dist/` web UI, resources), where each feature lives, rules for reading it (understand, never copy) | Reference research |
| `tools/reference/` | `ui.sh` (pin/click/snap), `ax_dump.js` (AX tree), `mouse.swift` (CGEvent pointer), `sample.py`/`scan.py`/`montage.py` (pixel tools), `dist.py` (search the reference app's `dist/`) | Reference research |
| `engine/` | C++23 real-time engine, C ABI `engine/include/saine_engine.h` | Audio engineering |
| `crates/saine-core` | Model, commands, undo/redo, persistence, snapshots | App architecture |
| `crates/saine-engine` | Safe Rust FFI wrapper + end-to-end tests | App architecture |
| `crates/saine-app` | Desktop app (Slint shell + wgpu surfaces) | UI engineering |
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

## Reference-inspection rules (learned the hard way)

- Window pinned at (0,33), 1440×820: `tools/reference/ui.sh pin`.
- Use AX clicks only for buttons/toggles at coordinates taken from `ax_dump.js`. Use
  `tools/reference/bin/mouse` (build: `xcrun swiftc -O tools/reference/mouse.swift -o tools/reference/bin/mouse`)
  for lanes, clips, hover, drag and right-click.
- **Never send keystrokes or clicks into Settings › Shortcuts.** A stray click once re-bound
  "Consolidate clips". It was reverted with Edit › Undo Shortcut Change.
- Don't send Rheo prompts (consumes the user's AI credits). Don't open or save the user's projects. Work in the
  unsaved "Untitled" project.

## Status log

### 2026-11-08: session 1

**Implemented and verified**
- Reference baseline: the reference app identity, architecture inference (native C++ host + WKWebView UI), AX-measured
  geometry of the main window, colour tokens, all browser tabs, Rheo panel, mixer, musical typing, 9 Settings pages,
  menus, shortcut map, piano roll, clip context menu, instrument device panel (`reference/`).
- Engine (C++23): 34/34 ctest (Release and ASan/UBSan), zero allocations on the audio path, Core Audio playback
  (manual tool), bench: 128 tracks × 4 clips = 12.4 µs per 256-frame block (≈430× real-time, one thread, M4).
- Core (Rust): model, validated invertible commands, undo/redo with coalescing, persistence, snapshots,
  assistant layer (Assist/Teach), context-aware keymap: 28 tests.
- FFI + end-to-end: 6 tests (ABI layout, multitrack summing with sample-accurate placement, mute/solo/gain/pan,
  seek/stop/trim, peaks, export).
- UI rendering decision: Slint 1.18.1 + wgpu 30.0.1, offscreen texture via `Image::try_from` (ADR 0001):
  ≈0.15 ms CPU/frame for 8,192 clips at a locked 60 fps.

- App (`crates/saine-app`, binary `saine-studio`): reference-geometry shell + wgpu timeline with real waveforms,
  wired to core/engine (transport, mixer controls, import, clip move/trim, undo/redo, save/open).
  `--self-test` 21/21; full-window screenshot MAE 1.96/255 vs reference (`crates/saine-app/FIDELITY.md`);
  60 fps with ≈0.06 ms of our render CPU per frame.
  Run: `cargo run --release -p saine-app -- --demo`.

**Known gaps / next steps**
1. Note preview/audition, velocity lane, clip loop, synth bypass + modulation/macros/effects; SIMD unison;
   splitters, track rename/resize, native menu bar.
   Engine: input/recording, loop crossfade, tempo map for the metronome.
2. Reference capture still missing: audio clip appearance (needs a local WAV dropped onto a track), hover/pressed
   states, animation timings, the rest of the shortcut bindings (scroll the Shortcuts page with mouse wheel events, not keys).
3. Engine: device selection, input/recording, worker-pool parallel track rendering, WASAPI/ASIO and PipeWire/JACK backends.
4. MIDI: model + piano roll + built-in instrument; plugin hosting (VST3/CLAP/AU) with an out-of-process scanner.
5. Assistant: provider adapter (outside core), Rheo-equivalent panel, highlight overlay.
6. Original branding and icons before any public release; the Söhne display face must not ship.

### 2026-11-08: session 1, round 2 (engine fixes + fonts)
- Engine ABI v1 additions (append-only): `se_meters_peek` (non-destructive ~50 ms window), `se_transport_set_loop`
  (sample-accurate wrap), `se_metronome_set` (+ `se_metronome_desc`); transport commands now apply immediately
  when no device runs (race-free ownership hand-off; see engine/README.md). 46/46 ctest in default and ASan;
  bench unchanged (128 tracks ≈12.4 µs/block).
- App: Inter 4.1 bundled (OFL, `crates/saine-app/ui/fonts/`), sizes ×0.91 after a sweep → full-window MAE 1.96 → 1.65.
  Loop (⌘L, button, `SetLoop` undoable) and metronome wired; offline-seek workaround removed; meters use peek.
  `--self-test` 26/26 offline and with Core Audio.

### 2026-11-08: session 1, round 3 (mixer + assistant panels)
- Mixer (header toggle / ⇧⌘X): per-track strips with real fader, pan, stereo `peek_meters`, M/S, peak hold;
  Master strip; add-track column. Fidelity full MAE 1.46 (`--state reference-mixer`).
- Assistant panel (header toggle / ⇧⌘A): Assist/Teach over `saine_core::assistant`, one undo step per turn,
  highlight overlay (`src/highlight.rs`), Anthropic Messages API provider (`src/assistant_provider.rs`, key from
  `ANTHROPIC_API_KEY` only, model `SAINE_ASSISTANT_MODEL`, default `claude-opus-5-5`). **No live API call has been
  exercised yet** (no key in the dev environment); the pipeline is covered by the labelled TEST provider
  `--assistant-fixture`. Fidelity full MAE 1.87 (`--state reference-assistant`, transcript masked).
- Self-test 42/42 offline and with Core Audio; workspace tests 49 pass; default state unchanged (1.65).
- Privacy: the reference Rheo AX dump had the owner's chat history; transcript strings are redacted in
  `reference/data/ax-rheo.tsv`. Never commit reference chat content.

### 2026-11-08: session 1, round 4 (MIDI + piano roll)
- Core: MIDI clips/notes, commands (AddMidiClip/Remove/Move/Resize/SetMidiNotes), seeded note transforms, note
  snapshots in engine frames. Engine ABI additions `se_session_publish_v2` (+ note/track v2 structs); original
  16-voice polyBLEP synth with sample-accurate events, republish-safe voices: 60/60 ctest (default + ASan),
  pitch within ±0.5 Hz. Bench: 32 MIDI tracks × 8 voices ≈ 396 µs per 256-frame block (unoptimised).
- App: MIDI clips on the timeline, piano roll (wgpu) with editing, toolbar transforms, footer, context menu.
  Self-test 60/60 offline and with Core Audio, incl. a check that notes drawn in the piano roll sound on the
  instrument track. Fidelity: midi-clip 1.45, piano roll 2.16; earlier states unchanged.
- Requested core API: RenameClip command, clip loop fields; engine: note audition call.

### 2026-11-08: session 1, round 5 (synth Devices panel)
- Core: `SynthParams` contract + 6 original factory presets, `Command::SetSynthParams`, `Document::seal_coalescing`.
- Engine: `se_session_publish_v3` with `se_synth_params`; every parameter drives the DSP (polyBLEP/BLAMP oscillators,
  shape morph/fold, cross-FM, sub/noise, TPT SVF LP/HP/BP, ADSRs, LFO, mono legato/glide, unison) with 10 ms
  smoothing; 76/76 ctest (default + ASan). Bench 32 tracks: default patch 573 µs/block, heavy patch 3489 µs (65 % of
  deadline).
- App: Synth device card at reference geometry (MAE 2.41), preset browser, 6 real pages, live displays derived from
  the engine formulas; self-test 76/76 offline and with Core Audio, incl. a rendered-audio check that a panel cutoff
  change reaches the engine.

### 2026-11-08: session 1, round 6 (recording + input)
- Core: Track arm/monitor/input + commands; ADR 0002.
- Engine: device enumeration/config (private aggregate device for separate mic/speaker clocks), per-track input
  routing + monitoring, disk recording via SPSC rings + writer thread, latency-compensated takes, input-injected
  offline render. 94/94 ctest (default + ASan). Input opens ONLY when a device is configured by name/default.
- App: R/I buttons, In popup, record transport, live recording clip, Settings › Audio panel with saved preferences
  (`~/Library/Application Support/Saine Studio/preferences.json`). Self-test 99/99 both modes (recording checks via
  injected input). The session rate is fixed at 48 kHz; the Audio page says so when a different device rate is picked.
- Real microphone verified (2026-11-08, with the user's permission): MacBook Air mic via the private aggregate
  device, 3 s take, 0 overruns, real room signal. An acoustic loopback test (metronome → speakers → mic,
  `SAINE_TEST_REAL_INPUT=1 cargo test -p saine-engine --test real_loopback -- --ignored --nocapture`) found the
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
  offline, 145/145 (+1 offline-only skip) with Core Audio. Fidelity `reference-automation` 1.94.

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
- Editors: Windows HWND host (tested in CI with `SaineTestGui.clap`); Linux returns
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
  files load unchanged. Findings SQ-1..SQ-7 in `reference/research-findings.md`.
- Engine: patterns play as ordinary notes; a new ctest checks that dense repeats republished while playing
  don't allocate, and bench_render has a Pattern table. `cargo bench -p saine-core --bench steps` times bake,
  read-back and snapshot builds.

### 2026-11-10: minimap pointer hide and Devices minimap (SAIN-33)

- Header minimap drags (window and zoom grabs) hide the pointer and put it back on the grabbed spot on release
  (`src/pointer.rs`: NSCursor + winit warp; self-test records the requests). Findings S68.
- Devices minimap at the right of the Clip / Devices strip (`src/devminimap.rs`, `DevMinimap` in `ui/app.slint`):
  tiles with glyphs, chain frames, window, jump / drag / wheel, Esc restores. Findings S69, S70. Not done: tile
  pictures, glide / fade animations, hover name.

### 2026-11-10: step sequencer, gaps closed (SAIN-37)

- Paging as in the reference: the grid shows 16 or 32 steps at a time (by width), with page tabs and a Follow
  playhead button at the right of the toolbar. A MIDI clip longer than 64 steps now converts with its own step
  count (up to `STEP_LIMIT`, 4096) instead of being refused; Length and Double still stop at 64.
- While the transport plays inside the clip, the playing step's column has a white ring and the cells it hits
  flash (230 ms, Steps lane).
- Toolbar tooltips (Double, Clear, the lane tabs, Follow playhead) with the reference's 1 s hover delay; Clear is
  disabled when the pattern has no cells.
- Value lanes use the reference's palette: empty steps dark, Velocity filled from the bottom on a lighter base.
  Findings SQ-8..SQ-11.
### 2026-11-10: pattern library, Genre list and Shape tab (SAIN-37, PR B)

- Genre now lists Any and the reference's 21 genres. The ten new ones come after the old eleven, because saved
  clips store the index. Each new genre has its own feel (scale, grid, swing, density, octave, progressions) and
  its own drum grooves, including two new grooves, Dembow and Warehouse pulse. Drum swing now follows the
  genre's feel, which gives the same result as before for the old genres. The Genre menu shows 12 rows and
  scrolls to the active one.
- Shape tab (`saine_core::shape`, `composer.rs`): Octave, Voices, Density, Length, Velocity, Nudge, Repeat and
  Arp reshape what the clip plays. The unshaped notes stay in `Composer::base`. Each part, and the kept
  notes, keep their own Shape. A slider drag is one undo step ("Shape Change"). If you edit notes after they
  were shaped, they play as edited under a neutral Shape. The other rack rows show disabled with the
  reference's tips. Tab rules are in SQ-13. Findings are SQ-12..SQ-15 in
  `reference/findings/step-sequencer.md`.
- Review states: `--state review-patterns-3` (Shape on a lead) and `review-patterns-4` (the Genre menu).
- Next: Shape option A, the full rack with drawn curves and the remaining transforms.
