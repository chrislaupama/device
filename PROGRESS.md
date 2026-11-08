# PROGRESS: Saine Studio

Living handoff document for future agent sessions. Read this first, then `docs/architecture.md`.

## Current milestone

**Phase 1 (inspection + architecture) and the first slice of Phase 3 (functional audio).**

## Repository map

| Path | What | Owner role |
|---|---|---|
| `reference/` | Reference audit, UI inventory, design tokens, interactions, parity matrix, research | Reference research |
| `reference/screenshots/` | Captures of the reference app at 1440×820 (git-ignored: third-party art and personal library data) | Reference research |
| `reference/data/` | Raw accessibility dumps (geometry source of truth) | Reference research |
| `tools/reference/` | `ui.sh` (pin/click/snap), `ax_dump.js` (AX tree), `mouse.swift` (CGEvent pointer), `sample.py`/`scan.py`/`montage.py` (pixel tools) | Reference research |
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
1. Synth parameters + Devices panel, note preview, velocity lane, clip loop; splitters, track rename/resize, native menu bar.
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
