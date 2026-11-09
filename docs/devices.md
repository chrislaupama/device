# Built-in devices

Saine's own effects (Compressor, the modulation family: Tremolo, Auto-Panner,
Chorus, Flanger, Phaser, Ring Mod, Character Mod; more follow from the device specs) are
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
     automation entry).
   - `View::Field` is a plotted well (reference motion / response / sweep
     fields): name a curve model in `crates/saine-app/src/device_fields.rs`
     (traces, fill or band, ticks, caption value, live overlay), the caption,
     the parameters dragged on x and y, and the field's width; the card is
     at least field + sliders + padding wide. Field cards centre their dial
     row and space its knobs evenly. The live overlay follows the kernel's
     first readout as an LFO phase (0..1) while the transport plays.
   - Parameter extras: `preset_locked()` (presets never load or save it,
     e.g. a random seed), `reset_on_load()`, `Format::Freq` (reference Hz
     text) and `Format::SignedPercent`. Tempo-synced LFOs use the
     `DIVISIONS` / `MOD_NOTES` tables and `synced_hz`.
   - Linked writes: when one edit must move other parameters in the same
     undo step (Character Mod's circuit switch resets untouched artifacts),
     add an arm to `builtin::linked_writes`; the panel then commits one
     labelled `Command::Batch`.
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
3. **Custom view (optional).** For bodies the template cannot express (EQ
   curve, shaper curve, trance-gate grid), use `View::Custom("<name>")` and
   add an arm for `<name>` in `crates/saine-app/src/device_views.rs`
   (`custom_card` draws, `panel_field` handles pointer events). The card's
   `field` is a vector canvas: Rust builds SVG paths and text boxes in card
   coordinates, `FieldView` in `ui/device_panel.slint` paints them and sends
   every press, drag, release, hover, double-click, right-click and wheel back
   to Rust. Hit testing, gestures and menus live in Rust. A gesture sends live
   values to the engine and commits one undo step on release; Escape restores
   the values from before it. The card may still use `knobs` (`KnobCell`)
   for its dial row. The Trance Gate and the Shaper are the examples.
   - A kernel whose latency depends on its parameters overrides
     `BuiltinProcessor::latency_for` (main thread; the Shaper reports its
     pitch-shift latency only while that lane is on).
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
   defaults; render it with `--offscreen --screenshot <png>` (no window is
   shown) and compare it with the reference capture.

## Commands

~~~sh
cargo test --workspace                                   # core, engine wrapper, app unit tests
cd engine && cmake --build build && ./build/saine_engine_tests
./build/saine_device_bench                               # per-block cost, param updates, 32-track session
cargo run -p saine-app -- --state reference-compressor --screenshot /tmp/compressor.png
cargo run -p saine-app -- --self-test                    # includes the built-in device section
~~~
