# ADR 0005: Parameter automation

Status: accepted (2026-11-08). Engine playback, plugin parameters and editor-change capture implemented (see Implementation notes); write (automation arm) is the app's.

## Reference
Automation is drawn over each track's lane (a line in the parameter's colour, value label; Volume is gold). Each track header has an
"Enable automation" power toggle and an "Automation parameter" picker ("Track › Volume", device categories),
and a "Show N automated lanes" disclosure. The transport has "Automation Arm: knob moves write automation while
playing". A Draw mode (B) pencil exists. See reference/ui-inventory.md §12. Point-editing gestures and the
write behaviour were not observed (U).

## Decision
- **Model (dstudio-core `automation.rs`):** `Track.automation: Vec<AutomationLane>` and `Project.master_automation`
  (volume only). A lane has a `target` (Volume dB −60..+6, Pan −1..1, Synth{param} in SynthParams units,
  Plugin{device, param} normalized), `enabled`, and `points: Vec<AutoPoint{time ticks, value, curve −1..1}>`.
- **Evaluation contract:** hold the first/last value outside the points; between points
  `v = lerp(a, b, shape(t, a.curve))`. `shape` is defined in core: power curves `t^(1+3c)` or
  `1−(1−t)^(1−3c)`. Interpolation is geometric for frequency-like synth parameters (cutoff, LFO rate).
  A vertical step at equal times takes the later point.
- **Editing:** a single invertible command, `SetAutomationLane{track: Option<TrackId>, target, lane: Option<_>}`, does
  create, replace and delete; one gesture is one undo step.
- **Playback (engine):** an enabled lane with points overrides the static value. Volume/pan are evaluated
  per sample (or ≤32-sample ramps) without zipper noise. Synth parameters run at the synth's control rate.
  Plugin parameters are sent as sample-offset parameter events (VST3 IParameterChanges, CLAP param events,
  AU scheduled parameters) at ≤64-frame resolution plus every point.
- **Write (automation arm):** while armed and playing, control moves (track volume/pan, synth knobs, and plugin
  editor changes via `take_plugin_param_changes`) are recorded into the lane in "touch" style. Points are
  thinned and committed as one undo step when the gesture ends.
- **Plugin parameters** are listed via `plugin_params`; a removed device's lanes stay but are inactive.

## Implementation notes (engine)

Details: engine/README.md › Automation.
- **ABI:** `se_session_publish_v6` (additive; `SE_ABI_VERSION` stays 1): `se_track_desc_v6` = v5 + `lanes`,
  `se_session_desc_v6` + `master_lanes`; `se_auto_lane_desc {target, logarithmic, synth_param (SynthParam::ALL
  index), param_id, device_id, points}`, `se_auto_point {i64 frame, f32 value, f32 curve}`. `Engine::publish` sends
  every `LaneSnapshot`. Plugin parameters: `se_plugin_params`, `se_plugin_param_value`, `se_plugin_param_text`,
  `se_plugin_take_param_changes` behind the Rust `plugin_params`, `plugin_param_value`, `plugin_param_text`,
  `take_plugin_param_changes` (signatures unchanged). `se_automation_eval` exposes the engine evaluation for tests.
- **Evaluation:** a literal float port of `value_at` with FP contraction off; 135 522 values from 200 random
  lanes are bit-identical to Rust (test requirement 1e-5 relative).
- **Replace, not multiply:** an active lane replaces the static gain / pan / synth field / plugin value; removing
  or disabling it restores the static value at the next publish.
- **Rates:** volume and pan per sample (exact at each point and at least every 32 samples, linear gain ramps in
  between, per-sample evaluation when leaving/entering the −60 dB silence); synth parameters every 16 timeline
  frames through the synth's ~10 ms smoothing; plugin parameters as sample-offset events at each processed
  segment start, every 64 timeline frames and every point (≤ 64 points per lane and segment).
- **Delay compensation:** each stage evaluates its lanes at the timeline frame its audio was read at (fader and
  master: `L`; insert k: the instrument plugin's latency plus slots 0..k−1, plus `L − lat_i` on monitoring
  tracks; synth / instrument plugin: 0), through a short history of rendered playback segments so this also holds
  for the samples right after a loop wrap. With PDC off nothing is compensated and lanes follow the transport.
- **Editor changes:** one lock-free MPMC queue fed by VST3 `IComponentHandler` begin/perform/endEdit, CLAP output
  events from `process()` and `params.flush()` (flushed by `se_plugin_idle` when nothing processes), and an AU
  `AUEventListener` on a dispatch queue; gestures are separate begin/end records. Stamped with the transport frame
  (sample-accurate for CLAP `process()` events). Automation itself is never reported.
- **Not done in the engine:** pushing automated values back to plugin editors (VST3 `setParamNormalized` from
  the RT values, AU listener notifications); per-voice/note automation; AU v3 `AUParameterTree` paths.
