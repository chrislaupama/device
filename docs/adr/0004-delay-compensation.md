# ADR 0004: Plugin delay compensation (PDC)

Status: accepted (2026-11-08). Implemented in the engine (see "Implementation notes"); the app is not wired yet.

## Context
ADR 0003 hosts AU, VST3 and CLAP plugins and reports their latency (`plugin_latency`), but nothing
compensated it: a track with a look-ahead limiter (one installed third-party limiter reports 3115 frames) plays
~65 ms late against the other tracks, the metronome and recordings. VST3 `restartComponent(kLatencyChanged)` and
CLAP `request_restart` / `latency.changed` were ignored, so a latency change at runtime was not even seen.

## Decision

### 1. Model: align tracks, delay the whole mix
- Per track, `lat_i` = sum of the reported latencies of its insert devices + its instrument plugin's latency.
  **Bypassed devices count.** A bypassed slot whose plugin has latency `x` is replaced by an `x`-frame delay line
  (the plugin is not processed), so toggling bypass never shifts a track in time. A device that failed to load
  counts 0 (it is a pass-through).
- `L = max_i lat_i` over all tracks, **muted and soloed-out tracks included**, so mute/solo never shift timing.
- Each track is delayed by `L - lat_i` after its chain, before gain/pan/the master sum. All tracks line up and the
  whole mix is `L` frames late relative to the transport position. `Engine::audible_position()` tells the UI what
  is being heard (see API).
- Within a track, clips on an instrument track with an instrument plugin of latency `li` are delayed by `li` so
  they line up with the (latent) instrument output before the chain.

### 2. Metronome
Clicks are generated on the transport grid exactly as before, then delayed by `L` through their own (mono) delay
line, so beat `k` is heard together with the music of beat `k`.

### 3. Input monitoring
A track that monitors input (`monitor` on and an input set) must not add `L - lat_i` to its input path. For those
tracks the delay is moved in front of the chain and applied **only to the playback source** (clips + built-in
synth / instrument plugin output); the input is added after that delay, and the sum then goes through the
track's own plugins. So the input is heard with only `lat_i` (+ device latency), while playback content is still
compensated (`(L - lat_i) + lat_i = L`). For time-invariant plugins pre-chain and post-chain delays are equivalent;
plugins that read the transport (tempo-synced effects) see their playback content `L - lat_i` frames later than
the position they are told while the track monitors. Toggling monitoring moves the line and causes a one-time
discontinuity of up to `L - lat_i` frames on that track.

### 4. Recording
The performer plays along with audio that is late by `L` + output latency, and their input arrives another input
latency later. Takes are therefore placed `start_frame = position - (input latency + output latency) - L`
(injected input: device part 0, only `L`). If `L` changes while recording, the takes end and new takes start at
the new position (like a seek), so every take stays aligned.

### 5. Export
`se_export_wav` renders `[start, end + L)` with private delay lines and drops the first `L` frames, so timeline
frame `t` is file frame `t - start`. The metronome is not exported (unchanged).

### 6. Dynamics
- **When the plan changes.** The plan (`L`, per-track delays, bypass delays, metronome delay) is part of the
  immutable render graph. It is computed on publish, and re-computed when a plugin's latency changes or the switch
  is toggled: the current graph is cloned with the new plan and installed with the normal atomic swap (same plugin
  instances, synths and delay lines; meters keep their values).
- **Latency changes.** CLAP `request_restart` and VST3 `restartComponent(kLatencyChanged)` set a flag on the
  instance (any thread). `se_plugin_idle` (main thread; also called inside `se_session_publish_v5`):
  1. for each flagged instance: suspend it (the RT side bypasses it: effects pass audio through, instruments are
     silent; a Dekker handshake waits for a `process()` already in flight), restart it (CLAP: `stop_processing`
     → `deactivate` → `activate`, `start_processing` again on the next block; VST3: `setProcessing(false)` →
     `setActive(false)` → `setActive(true)` → `setProcessing(true)`), resume;
  2. re-read **every** live instance's latency (also catches AU `kAudioUnitProperty_Latency` changes and plugins
     that change latency without asking for a restart);
  3. if any value changed, re-plan.
  The app must call `plugin_idle()` regularly on the main thread (10-60 Hz) for latency changes to take effect.
- **Delay lines** are allocated on the main thread when a plan is installed, sized to the next power of two
  `>= delay + max block`, and carried over to the next graph by key (track id / device id), like synth state, so
  republishes do not interrupt them. A line is reused when big enough; otherwise a new zeroed line replaces it.
  When a track's delay value changes, the line's content is reinterpreted with the new delay (a one-time jump,
  no allocation on the RT side). **Cap: 2^17 frames (131072, ~2.7 s at 48 kHz) per delay**; larger values are
  clamped and `clamped` is reported (those tracks stay late by the excess). Memory is ~2 MB per line at the cap,
  only for lines that are needed.
- **Transport.** Seek and stop **flush** every line (O(1): a flush generation bumped by the RT owner, each line
  marks everything older as silence on next use; no memset). Pause does **not** flush: the last `L` frames ring
  out while paused. Loop wraps keep the lines running, so after a wrap the end of the loop is heard, then its
  start (the output is the playback history delayed by `L`). Plugins' own internal buffers are not flushed on seek
  (they are opaque); their stale tail comes out within the first `lat_i` frames after a seek, before the
  compensated content.
- **Idle tracks.** A line whose pending output has drained only advances an index (no memory traffic). A track
  that is muted or soloed out drops its lines' contents (it restarts silent when it becomes audible again, so
  the first content after unmuting is heard aligned). While stopped, tracks keep rendering until their lines
  have drained.
- **Real time.** No allocation, locks or I/O on the audio thread (tested with the allocation tracker while
  latencies change and the switch toggles). Each plugin `process()` call is wrapped in two atomics for the
  suspension handshake.

### 7. Switch
`set_delay_compensation(bool)`, **on by default**. Off, the plan is empty and the engine renders bit-identically
to the engine before PDC (also with latent plugins, recording, metronome and export; tested against hashes
produced with the pre-PDC library). With PDC on and `L = 0` nothing changes either. The plan's `lat_i` values
are still reported while off.

## API
C ABI (append-only, `SE_ABI_VERSION` stays 1): `SE_PDC_MAX_DELAY_FRAMES`, `se_pdc_set_enabled`,
`se_pdc_is_enabled`, `se_pdc_report` (16 bytes) + `se_pdc_track` (16 bytes) + `se_pdc_report_get`,
`se_transport_audible_position`. Rust (`dstudio_engine::Engine`): `set_delay_compensation(bool)`,
`delay_compensation_enabled()`, `latency_report() -> LatencyReport { total_frames, tracks: Vec<(track_id, lat_i)>,
clamped }`, `audible_position() -> i64`, plus `PDC_MAX_DELAY_FRAMES`.

`audible_position()` = transport position − applied `L` − output device latency (`DeviceStatus::
output_latency_frames`, the same Core Audio device + safety offset + stream latency used for recording; 0 offline
or with the device stopped), while playing; the transport position while paused or stopped. Block granularity.

## Alternatives considered
- **Time-shifted rendering instead of delay lines** (render clips at `position - d`, no memory): free for clips,
  but wrong across loop wraps (after a wrap it would play material from before the loop start instead of the loop
  end) and impossible for plugin outputs and live input. Rejected.
- **Pre-chain delay for every track**: one code path, but tempo-synced plugins would see content late relative to
  the transport they are told. Kept only for monitoring tracks, where it is required.
- **Preallocating every line at the cap**: ~2 MB per track even with `L = 0`. Rejected in favour of sizing lines
  per plan on the main thread.
- **Flushing on pause / keeping on seek**: a pause flush would cut the tail mid-note; keeping lines on seek would
  play pre-seek audio at the new position. Flush on seek/stop, ring out on pause.

## Implementation notes (engine, 2026-11-08)
Details in engine/README.md "Plugin delay compensation".
- `src/delay_line.hpp` (power-of-two ring, memcpy in/out, lazy flush, idle tracking); `PdcPlan` in
  `RenderGraph`; `Engine::plan_pdc` / `replan_pdc_locked`; delay lines in `render_block` and the metronome in
  `Engine::process`; `PluginInstance::run/suspend/restart`; `PluginHost::service_latencies`.
- All graph installs (every publish version, re-plans) now happen under the plugin host mutex, so a re-plan can
  never overwrite a concurrent publish.
- Test plugins `DStudioTestLatency.clap` / `.vst3` (delay by N, report N; N from state, parameter 0 changes it at
  runtime through the format's restart protocol).
- **Not verified:** real-device acoustic alignment with plugins (`real_loopback.rs`
  `acoustic_loopback_latency_with_pdc` exists but was not run: it opens the microphone); third-party plugins
  that change latency at runtime; the clip-delay path for instrument plugins *with* latency (no latent test
  instrument; the code path is shared with the tested lines).
