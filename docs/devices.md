# Built-in devices

Saine's own effects (Compressor; the dynamics family: Limiter, Gate, Glue
Compressor, Transient Shaper, Mastering Limiter; the modulation family:
Tremolo, Auto-Panner, Chorus, Flanger, Phaser, Ring Mod, Character Mod; more
follow from the device specs) are
**declarative**: one definition in Rust describes a device, and the model,
undo, persistence, automation, engine and panel all derive from it. A device
is made of up to three parts:

1. a **definition** (`saine_core::builtin::DeviceDef`): parameters, panel layout, factory presets;
2. a **DSP kernel** (`saine::BuiltinProcessor` in `engine/src/devices/`);
3. optionally a **custom view** in the panel code, when the standard card body is not enough.

~~~mermaid
flowchart LR
    Def["DeviceDef<br/>crates/saine-core/src/builtin/&lt;id&gt;.rs"] --> Model["Device in the track chain<br/>params by id, undo, save"]
    Def --> Panel["Generic card<br/>src/device_panel.rs + ui/device_panel.slint"]
    Model --> Snap["Snapshot: values in definition order<br/>SE_PLUGIN_BUILTIN + se_builtin_state"]
    Snap --> Engine["BuiltinInstance → BuiltinProcessor<br/>engine/src/devices/&lt;id&gt;.cpp"]
    Engine -- readouts --> Panel
    Panel -- live drag values --> Engine
~~~

## How a built-in device lives in the app

- **Model.** A built-in device is an ordinary `Device` in a track's chain with
  `PluginFormat::Builtin` and `plugin.id` = the definition id. Its plain
  parameter values live in `Device::params` by parameter id; missing ids read
  as the default, unknown ids are dropped on `AddDevice`. Add / remove /
  reorder / bypass are the existing chain commands.
- **Edits.** `Command::SetDeviceParam` (validated and snapped by the
  definition; drags coalesce per parameter), `Command::SetDeviceParams`
  (preset loads, "Load Preset <name>") and `Command::SetDeviceSidechain`.
- **Automation.** `AutomationTarget::Builtin { device, device_type, param }`:
  lanes hold plain values in the parameter's unit; log parameters interpolate
  geometrically. `builtin::mod_targets` lists LFO / macro targets
  (`not_automatable` parameters are left out).
- **Engine.** The snapshot sends each built-in device with
  `format = SE_PLUGIN_BUILTIN`, `plugin_id` = the id, and an
  `se_builtin_state` blob (header + float32 values in definition order + the
  sidechain source track). Unlike plugin state it is applied on every publish.
  Lanes use `SE_AUTO_BUILTIN` with the parameter index. While a control is
  dragged the panel calls `se_builtin_set_param` (lock-free, applied at the
  next block) and commits one command on release. See the ABI block at the end
  of `engine/include/saine_engine.h`.
- **Panel.** `device_panel.rs` turns the definition's `Layout` into a
  `DeviceCard` (picker row, view row with slider column, dial row of knob
  clusters) and `ui/device_panel.slint` draws it. Lengths are the reference's
  100%-zoom card measurements times `device_panel::Z` (0.85, the zoom the rest
  of the UI is built at). Card width follows the reference rule:
  knobs × 62 + inner gaps × 10 + cluster gaps × 21 + 16.

## Modulation and chains

Every device gets the same modulation and container features; nothing here is
specific to one effect.

- **Sources.** An LFO (`lfo`) and a chain head (`device-rack`) are built-in
  devices with modulation outputs (`saine_core::modulation::outputs`: 1 and 8).
  A source owns its links in `Device::links` (`ModLink { slot, target, param,
  min, max }`); `param` is a built-in parameter id or a plugin parameter id
  (`ModParam`). A parameter takes at most three sources
  (`modulation::link_refusal` lists every rule). Edits are
  `AddModLink` / `RemoveModLink` / `SetModLink` (amount drags coalesce).
- **Maths** (in the target parameter's normalized 0..1 domain). LFO:
  `n = base + depth * (min + (max - min) * wave)`, wave 0..1, several LFOs add.
  Macro: `n = min + (max - min) * macro` (sets the value; min above max inverts).
  The result is clamped to 0..1.
- **Chains.** Members follow their head with `Device::chain = head`. The head's
  power switch bypasses every member (`modulation::effectively_enabled`, used by
  the snapshot); macros only reach members. Removing a head removes its members;
  moving it moves them. `collapsed` hides the members, `folded` folds any card.
- **Engine.** A source's state is an `se_builtin_state_v2` blob (the same
  header the samplers use): values, then `se_mod_route` records built by
  `modulation::routes` after the (empty) sample and slice tables. Every 32 frames
  (`SE_MOD_GRID_FRAMES`) the engine runs each enabled source, applies its routes
  to built-in parameters (as parameter events) or plugin parameters (as plugin
  events, the base restored when the last route goes), with no allocation on the
  audio thread. Plugin card knobs set values with `se_plugin_set_param`; a
  modulated parameter keeps that as its base (`se_mod_base`).
- **UI** (`src/device_mod.rs`). Right-click any control (built-in or plugin card)
  for Automate, Modulators (New LFO, the track's LFOs, Remove) and Macros inside a
  chain. Modulated knobs draw the reach of their sources and a dot at the live
  value. The LFO and chain head have custom bodies (`View::Custom("lfo" |
  "chain")`); their Map pill arms the source so the next control clicked becomes a
  target. Card headers: name = effect picker (swap in place), fold, device window
  (plugins), remove.
- **Plugin card.** A third-party plugin shows the parameters added with "+ Add"
  (`Device::card_params`, `SetCardParams`) plus any a source or automation moves,
  nine per page.

## Instruments and sample slots

The Sampler, Drum Rack and Pad Sampler added two general features to the
system. Any built-in device can use them.

- **Built-in instruments.** A definition with `kind: PluginKind::Instrument`
  goes in the track's instrument slot (`Command::SetInstrumentPlugin`, on
  instrument tracks only), not in the insert chain. The engine kernel sets
  `BuiltinDescriptor::instrument`. It then gets the track's notes as
  `BuiltinContext::events` and renders into silence. `se_builtin_note`
  plays a note on a live instrument right away, for pad and preview clicks.
  Velocity 0 releases the note.
- **Sample slots.** `DeviceDef::samples` is the number of audio files the
  device can hold. Each file is a `DeviceSample` in `Device::samples`: a
  slot, a project asset and optional slices (start, gain, pan, pitch).
  `Command::SetDeviceSample` loads, replaces or clears one slot, and slice
  edits use it too. Each of these is one undo step. `RemoveAsset` is refused
  while a device still plays the asset. The snapshot maps the sample assets to
  engine assets (`SessionSnapshot::device_assets`). The engine then gets the
  v2 state (`se_builtin_state_v2`): the v1 header, the values, then one
  `se_builtin_sample` per slot, followed by the slices. The kernel reads the
  samples as `BuiltinContext::samples`, which are bound on the control thread
  and never copied on the audio thread.
- **Shared sampler engine.** `engine/src/devices/sampler_engine.{hpp,cpp}`
  has the voice and voice pool, the sinc interpolator (band-limited when
  pitched up), envelopes, the filter, the LFO and the granular warp. All
  three samplers use it.
- **Panel.** `View::Custom("sampler" | "pad-sampler" | "drum-rack")` uses
  `src/sampler_panel.rs`. It has the waveform well with its flags, slices and
  playheads, the tabs with a picture canvas, and the pad grid. Files are loaded
  from the file picker or by dropping them on the window
  (`ImportTarget::DeviceSample`).

## Adding a device

1. **Definition.** Add `crates/saine-core/src/builtin/<id>.rs` with a
   `pub static <ID>: DeviceDef` and list it in `builtin::all()`.
   - Parameter order is the engine order. Never reorder or remove parameters
     of a shipped device; append new ones (old projects keep loading because
     values are stored by id).
   - Ids, ranges, defaults, units, tapers and the panel arrangement come from
     the device spec (`handoffs/device-specs/<device>.md` in the Captain drive).
     Those are facts; write descriptions in your own words. Never copy the
     reference's code, CSS, images, audio or preset files.
   - `Layout`: `picker` (segmented `Menu`s, `Segments` with their own
     labels, `Dropdown` pills for long lists, toggles, `Sidechain`, pills;
     `Gap` or the sidechain picker pushes what follows to the right),
     `view` (`DynamicsStrip`, `Field` or `Custom`), `sliders`,
     `clusters` of knobs (`Swap` holds alternatives in one cell, such as
     Rate / Rate Sync), `faces` (drawings inside a knob), `shown_when`
     (mode-dependent controls: hidden parameters get no control and no
     automation entry), `fixed_width` (card width in pt; 0 derives it from
     the dial row and view).
   - A card narrower than 240 pt hides the preset label and arrows and opens
     the preset menu from a chevron after the name. `Control::Toggle` on an
     Off/On choice draws a latch capsule in the dial row.
   - `View::Field` is a plotted well (reference motion / response / sweep
     fields): name a curve model in `crates/saine-app/src/device_fields.rs`
     (traces, fill or band, ticks, caption value, live overlay), the caption,
     the parameters dragged on x and y, and the field's width; the card is
     at least field + sliders + padding wide. Field cards centre their dial
     row and space its knobs evenly. The live overlay follows the kernel's
     first readout as an LFO phase (0..1) while the transport plays.
   - Parameter extras: `preset_locked()` (presets never load or save it,
     e.g. a random seed), `reset_on_load()`, `Format::Freq` (reference Hz
     text), `Format::SignedPercent` and `Format::DbCut` (a positive stored
     cut shown as −x.x dB). Tempo-synced LFOs use the `DIVISIONS` /
     `MOD_NOTES` tables and `synced_hz`.
   - Linked writes: when one edit must move other parameters in the same
     undo step (Character Mod's circuit switch resets untouched artifacts),
     add an arm to `builtin::linked_writes`; the panel then commits one
     labelled `Command::Batch`.
   - Dynamics additions: `Toggle` latches (a latch in a knob cell in the dial
     row; a right-aligned "<label> <choice>" pill in the picker row), detent
     ticks on discrete knobs with more than two choices, transfer faces
     (`SoftKneeCurve`, `BrickWall`, `Expander`, `BusVca`, `PushWall`), and an
     empty picker moves the view row up.
   - Views: `View::Strip(Strip::new("<line param>").range(top_db, floor_db).second("<param>"))`
     is the level / gain-reduction strip with a draggable dashed line, an
     optional solid second line (Alt+drag moves both by the same dB, one undo
     step) and its own plot range; `DynamicsStrip { threshold }` is shorthand
     for the 0..-36 dB strip. `View::Custom("gain-scope")` is the transient
     shaper's scrolling waveform + applied-gain scope.
   - Choice overlays: `ParamDef::overlays` lists, per choice, other
     parameters that choice writes (the mastering limiter's styles); they go
     through `linked_writes` like any other linked write. Also `max_text` (the
     top of the range reads e.g. "Full"), `Format::Ms(decimals)`, and
     `DeviceDef::sidechain_enable` (the parameter that follows the sidechain
     picker).
2. **DSP kernel.** Add `engine/src/devices/<id>.cpp`: a `BuiltinProcessor`
   (`prepare` may allocate; `reset` / `process` are real-time: no
   allocation, locks, I/O or exceptions) and a `BuiltinDescriptor` whose
   parameter ids are the definition's ids **in the same order**. Register it
   in `kBuiltinRegistry` (`engine/src/devices/builtin.cpp`), give the
   descriptor its `defaults` (checked against the definition by the Rust
   registry test through `se_builtin_desc::param_defaults`) and add the
   file to `engine/CMakeLists.txt`. `params` are plain values, constant
   within one `process` call (the instance splits blocks at automation
   events); smoothing audible steps is the kernel's job (`dsp::Smoother`).
   `engine/src/devices/dsp.hpp` also has `LfoClock` (free or locked to
   the song position through `BuiltinContext::offset`), `DelayLine`
   (Hermite) and `Oversampler2x` (report its latency from `latency()`).
   Readouts (meters, gain reduction) are written per block; give
   `idle_readouts` for the values before the first block. When automation
   splits a block, the engine keeps each readout's segment value farthest
   from its idle value, so readouts must move away from idle as they grow
   (a peak meter rises from -120 dB, gain reduction falls from 0 dB).
   A device has at most `kMaxReadouts` (64) readouts. Spectral helpers are in
   `engine/src/devices/spectral.hpp` (`RealFft`, and `SpectrumTap`: 48 log
   bands, 20 Hz–20 kHz, a full-scale sine reads 0 dB); `dsp.hpp` has the
   state-variable filter (`SvfCoeffs`, including `bandpass`). Report latency for
   PDC: `latency_for(params)` (the default returns the fixed `latency`), so a
   kernel whose delay depends on a setting (the mastering limiter's
   oversampling) reports the delay of the values it will run with. Shared
   dynamics building blocks (biquads, delay lines, sliding minimum, look-ahead
   gain, polyphase oversampler, 4x true-peak detector) are in
   `dsp_dynamics.hpp`.
3. **Custom view (optional).** For bodies the template cannot express (EQ
   curve, shaper curve, trance-gate grid), use `View::Custom("<name>")`, add
   an arm in `Controller::builtin_card` that fills the card's view data, and a
   Slint component in `ui/device_panel.slint` chosen by that data. Reuse
   `ParamDrag`, `KnobCell` and `SliderCol` for the gestures. Custom bodies
   return a `PanelCanvas` from `Controller::custom_body` (`device_views.rs`):
   paths, texts, grip pills and linear drag zones. When handles need
   non-linear mapping or hit-testing (several handles on one well, a live
   spectrum), set `pointer: true` and handle `Studio.panel-graph` in Rust,
   as the restoration devices do (`crates/saine-app/src/panel_graph.rs`: hit
   zones, spectrum ballistics from the readouts, one undo step per gesture).
4. **Tests.**
   - Engine: `engine/tests/devices/device_harness.hpp` gives `DeviceRig`
     (one instance, `set(id, value)`, `run` / `run_mono`, `readouts`),
     `DeviceSession` (a published engine session with tracks, devices and
     lanes) and test signals (`impulse`, `tone`, `noise`). Write
     `engine/tests/devices/test_<id>.cpp` for the device's behaviour.
     `every_builtin_is_rt_safe_and_robust` runs every registered device
     under the allocation guard with hostile input and extreme values
     automatically.
   - Rust: `engine_registry_matches_core_definitions`
     (`crates/saine-engine/tests/builtin.rs`) fails when the definition and
     the kernel disagree on parameter order or defaults. Add core tests for formatting and
     parsing of any new display format.
5. **Benchmark.** Add a row for the device to `engine/bench/bench_devices.cpp`
   (`saine_device_bench`, Release build) and put the numbers in the PR.
6. **Render.** `--state reference-<id>` shows any built-in device at its
   defaults. Render it with `--offscreen --screenshot <png>` (a hidden
   window, so it does not take over the screen) and compare it with the
   reference capture. Add the device to the app self-test (the dynamics,
   modulation and restoration sections in `src/selftest.rs` add each device
   through Add device › Built-in).

## Commands

~~~sh
cargo test --workspace                                   # core, engine wrapper, app unit tests
cd engine && cmake --build build && ./build/saine_engine_tests
./build/saine_device_bench                               # per-block cost, param updates, 32-track session
cargo run -p saine-app -- --state reference-limiter --offscreen --screenshot /tmp/limiter.png
cargo run -p saine-app -- --state reference-compressor --screenshot /tmp/compressor.png
cargo run -p saine-app -- --state reference-lfo --screenshot /tmp/lfo.png      # also reference-chain
cargo run -p saine-app -- --self-test                    # includes the built-in device section
~~~
