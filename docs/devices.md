# Built-in devices

Saine's own effects (Compressor first; more follow from the device specs) are
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
- **Engine.** A source's state is an `se_builtin_state_v2` blob: values, then
  `se_mod_route` records built by `modulation::routes`. Every 32 frames
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
   - `Layout`: `picker` (menus / toggles / `Sidechain` / pills; the
     sidechain picker and what follows align right), `view`
     (`DynamicsStrip` or `Custom`), `sliders`, `clusters` of knobs,
     `faces` (drawings inside a knob), `shown_when` (mode-dependent controls).
2. **DSP kernel.** Add `engine/src/devices/<id>.cpp`: a `BuiltinProcessor`
   (`prepare` may allocate; `reset` / `process` are real-time: no
   allocation, locks, I/O or exceptions) and a `BuiltinDescriptor` whose
   parameter ids are the definition's ids **in the same order**. Register it
   in `kBuiltinRegistry` (`engine/src/devices/builtin.cpp`) and add the
   file to `engine/CMakeLists.txt`. `params` are plain values, constant
   within one `process` call (the instance splits blocks at automation
   events); smoothing audible steps is the kernel's job (`dsp::Smoother`).
   Readouts (meters, gain reduction) are written per block; give
   `idle_readouts` for the values before the first block. When automation
   splits a block, the engine keeps each readout's segment value farthest
   from its idle value, so readouts must move away from idle as they grow
   (a peak meter rises from -120 dB, gain reduction falls from 0 dB).
3. **Custom view (optional).** For bodies the template cannot express (EQ
   curve, shaper curve, trance-gate grid), use `View::Custom("<name>")`, add
   an arm in `Controller::builtin_card` that fills the card's view data, and a
   Slint component in `ui/device_panel.slint` chosen by that data. Reuse
   `ParamDrag`, `KnobCell` and `SliderCol` for the gestures.
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
     the kernel disagree on parameter order. Add core tests for formatting and
     parsing of any new display format.
5. **Benchmark.** Add a row for the device to `engine/bench/bench_devices.cpp`
   (`saine_device_bench`, Release build) and put the numbers in the PR.
6. **Render.** Add a `--state` that shows the device at its defaults (see
   `reference-compressor` in `crates/saine-app/src/main.rs`), render it with
   `--screenshot` and compare it with the reference capture.

## Commands

~~~sh
cargo test --workspace                                   # core, engine wrapper, app unit tests
cd engine && cmake --build build && ./build/saine_engine_tests
./build/saine_device_bench                               # per-block cost, param updates, 32-track session
cargo run -p saine-app -- --state reference-compressor --screenshot /tmp/compressor.png
cargo run -p saine-app -- --state reference-lfo --screenshot /tmp/lfo.png      # also reference-chain
cargo run -p saine-app -- --self-test                    # includes the built-in device section
~~~
