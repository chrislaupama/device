# Plan: Automate any device control, mute and device on/off

Spec: [spec.md](spec.md)

## Approach

One shared control menu (`Controller::mod_menu`) already offered "Automate <parameter>" for built-in device knobs.
The work routes every remaining control's right press to it (or to the Synth's own menu, which gains the same row),
and adds a small `strip_menu` for the channel controls. Mute and device on/off become lane targets rather than a
separate mechanism, so drawing, touch-write, undo, saving and export all apply to them unchanged. Steps are a lane
flag rather than a new evaluation function, so the existing core/engine contract (one `value_at`) stays the single
definition.

## Changes by layer

- **Core:** `AutomationTarget::Mute` and `DeviceOn { device, name }`; `AutomationTarget::stepped()` (switches and
  built-in choice parameters); `value_at` holds values on stepped lanes; `SetAutomationLane` checks the device is on
  the track; import remaps the device id; bounce keeps mute lanes.
- **Engine:** `AutoLane::step`, `SE_AUTO_MUTE` (per-sample gain before the fader), `SE_AUTO_DEVICE_ON` (per block,
  replaces `enabled` for an insert or the instrument plugin), PDC bypass delay line for slots with an on/off lane.
- **C ABI:** `se_auto_lane_desc._pad` becomes `flags` (same layout, 0 keeps the old meaning); new enums.
- **App:** right-press wiring in `device_panel.slint`, `synth.slint`, the strip widgets; `mod_menu` power entry,
  `synth_pill_menu`, `strip_menu`; picker entries; unit labels; point readout; M and power follow active lanes.
- **Docs:** README feature list, ADR 0005, engine README, PROGRESS.md.

## Principles check

- Real-time safety: the new per-block lookups are index reads into lanes built at publish time; nothing allocates on
  the audio thread.
- One mutation path: every write is `SetAutomationLane`.
- Projects keep loading: new enum variants only; old projects have none.
- ABI boundary: the only struct change reuses padding, checked by the layout tests on both sides.

## Tests and evidence

- Tests: see the spec's acceptance checks.
- Benchmarks: `dstudio_engine_bench` before and after (Automation table gains a 7-lane row).
- Screenshots: Synth right-click menu and a Synth cutoff lane label, before and after.

## Risks

- A static mute that a lane overrides could confuse: the M button shows the lane's state, and clicking explains.
