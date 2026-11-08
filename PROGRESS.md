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
1. Bundle Inter (OFL) to close most of the remaining text error; build the panel toggles (Rheo, mixer, editor), splitters,
   track rename/resize, native menu bar. Engine ABI requests: apply transport commands when the device is stopped,
   non-destructive meter reads, loop playback, metronome, input/recording.
2. Reference capture still missing: audio clip appearance (needs a local WAV dropped onto a track), hover/pressed
   states, animation timings, the rest of the shortcut bindings (scroll the Shortcuts page with mouse wheel events, not keys).
3. Engine: device selection, input/recording, worker-pool parallel track rendering, WASAPI/ASIO and PipeWire/JACK backends.
4. MIDI: model + piano roll + built-in instrument; plugin hosting (VST3/CLAP/AU) with an out-of-process scanner.
5. Assistant: provider adapter (outside core), Rheo-equivalent panel, highlight overlay.
6. Original branding and icons before any public release; the Söhne display face must not ship.
