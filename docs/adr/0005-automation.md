# ADR 0005: Parameter automation

Status: accepted (2026-11-08). Implementation in progress.

## Reference
Automation is drawn over each track's lane (gold line, value label). Each track header has an
"Enable automation" power toggle and an "Automation parameter" picker ("Track › Volume", device categories),
and a "Show N automated lanes" disclosure. The transport has "Automation Arm: knob moves write automation while
playing". A Draw mode (B) pencil exists. See reference/ui-inventory.md §12. Point-editing gestures and the
write behaviour were not observed (U).

## Decision
- **Model (saine-core `automation.rs`):** `Track.automation: Vec<AutomationLane>` and `Project.master_automation`
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
