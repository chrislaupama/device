# Channel strip trim, width and mono

Status: Done · Spec: 005 · Issue: #32

## Problem

The channel strip in the bottom panel (first tab) shows **Trim**, **Width** and **Mono** next to Pan, but only Pan
works. The Trim and Width knobs are fixed at "0.0 dB" and "100%" and the Mono switch does nothing, because a track has
no trim, width or mono in the model and the engine has nowhere to apply them. Anyone gain-staging a loud recording,
narrowing a wide pad, or checking a part in mono has to reach for an effect instead.

## Behaviour

- **Trim** is a bipolar knob, -24 to +24 dB, centred at 0.0 dB. It reads "+3.5 dB" / "−6.0 dB" and moves in 0.1 dB
  steps.
- **Width** is a knob from 0 % (mono) through 100 % (unchanged, the knob points straight up) to 200 % (side signal
  doubled). It reads "100%".
- **Mono** is a switch that folds the track to mono (the same as 0 % width) whatever the width knob says; the width
  knob dims while it is on. The text under it reads "On" / "Off".
- Double-clicking a knob resets it (0.0 dB, 100%). Keyboard and screen-reader users get increment / decrement actions.
- Each knob drag is one undo step, labelled "Trim Change" or "Width Change", however long the knob is held still;
  the switch is "Mono Change".
- The values are saved with the project. Projects saved before this load with 0.0 dB, 100 % and mono off.
- Return tracks have the same controls.
- Where in the signal path: after the insert chain, before pre-fader sends, the fader and pan. So trim and width
  shape what pre-fader sends carry, and the fader still sets the final level.

## Requirements

- R1. Each track has a trim (dB, -24..24), a width (0..2, 1 = unchanged) and a mono flag, saved in the project with
  defaults that old projects load as.
- R2. Undoable commands set each one; out-of-range values are refused and change nothing; a knob drag is one undo
  step.
- R3. The engine applies trim, mid/side width and the mono fold after the insert chain and before pre-fader sends
  and the fader, allocation-free on the audio thread. A track at 0 dB, 100 % and mono off renders bit-identically to
  before and costs nothing extra.
- R4. The channel strip's Trim, Width and Mono controls show and edit the selected track's values, return tracks
  included.
- R5. Track renders that keep the track (bounce in place) and material renders are taken before trim, width and
  mono; stems keep them.

## Acceptance checks

- A1 (R3): engine tests: +6 dB trim raises the level by 6 dB; width 0 gives identical L and R; width 2 doubles the
  side; mono gives the L/R average whatever the width; unity is bit-identical to no strip; bad values are refused;
  rendering allocates nothing (`engine/tests/test_strip.cpp`).
- A2 (R1, R2): core tests: apply / undo / redo and labels, validation leaves the project unchanged, a held drag is one
  step, save and reopen keeps the values, old projects load neutral, the snapshot carries linear trim
  (`crates/dstudio-core/tests/strip.rs`).
- A3 (R4): self-test drives the knobs and the switch through the strip and checks model, undo and text; before and
  after screenshots of the strip (review state `review-strip-trim`).
- A4 (R3): `dstudio_engine_bench` "Channel strip" table: 32 tracks with and without trim + width / trim + mono.

## Out of scope

- Automation lanes for trim and width (the issue marks them optional; volume and pan stay the automatable strip
  controls).
- Trim, width and mono on the master strip and in the mixer panel's strips.
- Carrying trim, width and mono into the Ableton Live set export.

## Open questions

None.
