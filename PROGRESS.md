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

(see the milestone report at the bottom; updated each session)
