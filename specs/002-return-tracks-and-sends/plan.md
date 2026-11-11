# Plan: Return tracks and sends

Spec: [spec.md](spec.md)

## Approach

Return tracks live in `Project::tracks` as `TrackKind::Return`, after every ordinary track. Keeping
them in the one track list means every per-track command (volume, pan, mute, solo, rename, devices,
automation) and every per-track engine path (chain, fader, meters, delay compensation) works on
them unchanged; the new code is the routing between tracks. The alternative, a separate
`Project::returns` list, would have needed a second lookup in every command and a second set of
indices in the app, for no gain.

Routing is a two-level graph in this spec: ordinary tracks feed returns (sends, or their Out), and
returns feed the master. The engine is written for a general acyclic graph of buses (processing
order from a topological sort, per-edge delay lines), so group tracks and return-to-return sends
can come later by relaxing validation, not by rewriting the renderer.

Delay compensation extends ADR 0004 from "align every track at the master" to "align every input
of every bus". For each track `t`, `lat_t` is its chain latency as today. A return `r` receives at
`in_r = max(lat_s)` over the tracks `s` that feed it (0 when none) and reaches the master at
`in_r + lat_r`. The master's `L = max` of `lat_t` over ordinary tracks and `in_r + lat_r` over
returns. Each track is delayed after its chain by `d_t = min(dest_in) - lat_t` over its
destinations (`L` for the master, `in_r` for a return), so it is aligned for its earliest
destination; each edge whose destination receives later gets its own delay line of
`dest_in - lat_t - d_t`. A return is delayed by `L - in_r - lat_r`. Without latent plugins on
returns every edge delay is 0 and no extra line exists, so the common case costs nothing. Edge
delays run after the fader, so fader and send-tap timing for automation is unchanged; a monitored
input skips only the track delay `d_t`, as today, not the edge delays (documented).

## Changes by layer

- **Core** (`crates/dstudio-core`):
  - `TrackKind::Return`; `Track::sends: Vec<Send>` (`Send { target, level, pre_fader }`) and
    `Track::output: Option<TrackId>` (None = Master), both `#[serde(default)]` and skipped when
    empty, so no schema bump.
  - `AddTrack` keeps ordinary tracks before returns (`index: None` inserts before the first return)
    and validates returns (empty, no input or instrument, at most `MAX_RETURNS` = 16).
    `RemoveTrack` of a return also removes the sends to it and resets outputs to it; its inverse is
    a `Batch` that restores the track, then the sends and outputs.
  - New commands: `SetSends { track, sends }` (whole list; labels by what changed is the caller's
    `Batch` label: Add Send, Remove Send, Send Pre-Fader; coalesce key `sends:{track}` so a level
    drag is one step, label Send Level Change) and `SetTrackOutput { track, output }` (Output
    Change).
  - Refusals on returns: `AddClip`, `MoveClip`, `AddMidiClip`, `MoveMidiClip`, `SetTrackArm`,
    `SetTrackMonitor`, `SetTrackInput`, `SetInstrumentPlugin`.
  - Snapshot: `TrackSnapshot::{is_return, output, sends}` (levels linear).
  - Helpers: `Project::returns()`, `return_letter(index)`, `next_return_name()`.
- **Engine** (`engine/`):
  - `RenderGraph`: `TrackNode::{is_return, bus, out_bus, send_begin, send_count, send_live}`,
    `RenderGraph::sends`, `bus_count`. `plan_sidechains` becomes `plan_order`: routing edges are
    fixed, sidechain edges are cut on a cycle as before.
  - `RenderContext`: `SE_MAX_RETURNS` stereo bus buffers and one stereo send scratch, preallocated.
  - `render_block`: buses cleared per block; a return's source is its bus; after the chain and
    track delay, pre-fader sends tap the track buffer, then the fader, then post-fader sends and the
    output go to the master or a bus, each through its edge delay when it has one. Mute/solo
    collapse adds `send_live` next to `audible`.
  - `plan_pdc`: per-bus arrival as above; `PdcPlan` gains edge lines (one per send, one per track
    output), carried across republishes by (track id, destination id) like the other lines.
- **C ABI** (`engine/include/dstudio_engine.h`, `crates/dstudio-engine`): append-only
  `se_session_publish_v9` = v8 plus `se_track_route { track_id, output_track_id, is_return }` and
  `se_send_desc { track_id, return_track_id, gain, pre_fader }`; `SE_MAX_RETURNS 16`. Validation
  refuses unknown ids, a send or output to a non-return, sends from returns, a return routed to a
  return, duplicate sends, gains outside 0..4, clips, notes or input on a return, more than 16
  returns. `Engine::publish` calls v9; layout tests for both structs.
- **App** (`crates/dstudio-app`): Returns pill and pinned return rows (timeline), return strips and
  the Sends section (mixer), Sends tab and Out menu (channel strip), "Return track" in the Add
  track menus, review states and self-test steps. The track list code skips returns where it means
  "ordinary tracks" (numbering, record, import targets).
- **Docs**: README Features, PROGRESS, `engine/README.md` (routing section), ADR 0004 note on bus
  alignment.

## Principles check

- Real-time safety: buses and scratch are preallocated in `RenderContext`; edge lines are sized on
  the main thread like every PDC line; the RT tracker test covers a routed session.
- One mutation path: all edits are commands; the pill is view state, not project state.
- Projects keep loading: new fields default; no schema bump; a load test of an old project.
- ABI boundary: append-only v9 entry point; Rust layout tests.
- Cross-platform: no platform code.
- Original work: names, layout and art are ours.
- Evidence: screenshots before/after for the UI PR; bench numbers for the engine PR.
- Docs: listed above.
- Naming: `dstudio`, "return", "send", "bus"; no bare "device".

## Tests and evidence

- Tests: core `tests/returns.rs`; engine `tests/test_routing.cpp` (levels, pre/post, solo/mute,
  outputs, export parity, PDC alignment with `DStudioTestLatency.clap`, RT safety); Rust e2e in
  `crates/dstudio-engine/tests`.
- Benchmarks: `bench_render` "Returns" table (32 tracks, 2 sends each, 2 returns) next to the same
  session without sends.
- Screenshots: `--state review-returns` (timeline rows + mixer with sends open) and
  `review-returns-strip` (channel strip Sends tab), before on main and after.

## Risks

- App code that iterates `project.tracks` meaning "timeline tracks with clips": caught by making
  the clip/record/input commands refuse returns, and by self-test steps on a project with returns.
- Sidechain plus routing cycles in the order sort: a test with a sidechain reading a return.
- Work in parallel threads on the channel strip and engine: kept to small pull requests rebased on
  main.

Pull requests: (1) spec and core, (2) engine, ABI and Rust wrapper, (3) app UI.
