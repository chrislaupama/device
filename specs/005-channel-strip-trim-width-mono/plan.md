# Plan: Channel strip trim, width and mono

Spec: [spec.md](spec.md)

## Approach

Trim, width and mono collapse into one 2x2 matrix per track at publish time:

```
M = (L + R) / 2, S = (L - R) / 2, w = mono ? 0 : width, t = trim (linear)
L' = t (M + w S) = a L + b R        a = t (1 + w) / 2
R' = t (M - w S) = b L + a R        b = t (1 - w) / 2
```

so the audio thread runs one in-place kernel (`kernel_strip`) over the track's buffer, and only for tracks whose
matrix is not the identity (`TrackNode::strip_on`). At unity a = 1 and b = 0 exactly, so a neutral track is not
touched at all. The stage sits right after the insert chain (and the delay-compensation line), where pre-fader
sends tap, so the fader, pan, post-fader sends and meters all follow it unchanged.

Alternative: trim before the insert chain (gain into the effects). Rejected for now: it would need a second stage
and a different place for width; one stage before the fader matches "before the fader" in the issue and keeps the
render loop simple.

## Changes by layer

- **Core**: `Track::trim_db` (saved as `trim`), `Track::width`, `Track::mono`, all `serde(default)` and skipped when
  neutral. Commands `SetTrackTrim`, `SetTrackWidth` (coalescing keys `trim:<id>`, `width:<id>`), `SetTrackMono`.
  `TrackSnapshot` gets linear `trim`, `width`, `mono`. `render_plan` neutralises them for Material and Bounce.
  Assistant summaries for the three commands.
- **C ABI**: `se_track_strip { track_id, trim, width, mono }` and `se_session_publish_v10` = v9 plus a strip list
  (only tracks that are not neutral are listed). Layout asserts in `c_api.cpp` and `ffi.rs`.
- **Engine**: `apply_strips` validates and fills `TrackNode::strip_a/strip_b/strip_on`; `kernel_strip` in
  `render_block` after the chain.
- **App**: `Studio.sel-trim/-text`, `sel-width/-text`, `sel-mono`, callbacks `track-trim`, `track-width`,
  `track-mono`, `strip-gesture-end`; `Controller::set_track_trim` / `set_track_width` hold coalescing until release.
- **Docs**: README Mixer bullet, PROGRESS.md entry, engine/README.md signal path.

## Principles check

- Real-time: the kernel is a plain loop over existing buffers; covered by an `RtAllocGuard` test.
- One mutation path: three `Command`s with exact inverses.
- Projects keep loading: defaults, no schema bump.
- ABI: appended struct and entry point; v1..v9 publish exactly as before.

## Tests and evidence

- Tests: `engine/tests/test_strip.cpp`, `crates/dstudio-core/tests/strip.rs`, self-test steps.
- Benchmarks: `dstudio_engine_bench` "Channel strip" table.
- Screenshots: `review-strip-trim` on main and on the branch.

## Risks

- Width above 100 % can raise peaks (side doubled): expected, the meters show it.
