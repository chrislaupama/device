//! Tremolo (reference: the reference app's built-in "Tremolo"). Parameter ids, ranges,
//! defaults, tapers and the panel arrangement are taken from the reference
//! as facts; the factory presets (names and values) are our own.

use super::*;

const SHAPES: &[&str] = &["Sine", "Triangle", "Square", "Saw Down"];

pub static TREMOLO: DeviceDef = DeviceDef {
    id: "tremolo",
    name: "Tremolo",
    kind: PluginKind::Effect,
    category: "Modulation",
    purpose: "Moves the volume up and down with an LFO, from a slow swell to a hard rhythmic chop.",
    params: &[
        ParamDef::new("rate", "Rate", 0.1, 20.0, 5.0, Taper::Log, "Hz", Format::Freq).help("LFO speed while the timebase is Free."),
        ParamDef::new("depth", "Depth", 0.0, 100.0, 70.0, LINEAR, "%", Format::Percent).help("How far the volume dips at the bottom of each cycle."),
        ParamDef::choice("shape", "Shape", SHAPES, 0).help("LFO waveform: Sine and Triangle swell, Square chops, Saw Down ramps."),
        ParamDef::new("stereo_phase", "Stereo Phase", 0.0, 180.0, 0.0, LINEAR, "deg", Format::Plain(0))
            .label("Stereo")
            .help("Offset of the right channel's LFO; 180 deg makes the channels alternate."),
        ParamDef::new("mix", "Mix", 0.0, 100.0, 100.0, LINEAR, "%", Format::Percent).help("Blend of the modulated and the untouched signal."),
        ParamDef::choice("sync", "Sync", FREE_SYNC, 0).label("Timebase").help("Free runs at Rate in Hz; Sync follows the song tempo at Rate Sync."),
        ParamDef::choice("rate_sync", "Rate Sync", DIVISIONS, DIVISION_QUARTER).label("Rate").help("LFO cycle length as a note value while synced."),
    ],
    layout: Layout {
        picker: &[Control::Dropdown("shape"), Control::Gap, Control::Menu("sync")],
        view: Some(View::Field(Field::new("tremolo", "Modulation", &["rate", "rate_sync"], &["depth"], 441.0))),
        sliders: &[],
        clusters: &[&[Control::Swap(&["rate", "rate_sync"]), Control::Knob("depth")], &[Control::Knob("stereo_phase"), Control::Knob("mix")]],
        faces: &[],
        min_height: 202.0,
        fixed_width: 0.0,
        shown_when: &[("rate", "sync", &[0]), ("rate_sync", "sync", &[1])],
        ..Layout::EMPTY
    },
    factory: &[
        FactoryPreset { name: "Slow Throb", values: &[("depth", 50.0), ("shape", 1.0), ("sync", 1.0), ("rate_sync", 5.0)] },
        FactoryPreset { name: "Quarter Pump", values: &[("depth", 65.0), ("sync", 1.0), ("rate_sync", 8.0)] },
        FactoryPreset { name: "Eighth Chop", values: &[("depth", 95.0), ("shape", 2.0), ("sync", 1.0), ("rate_sync", 11.0)] },
        FactoryPreset { name: "Dotted Stutter", values: &[("depth", 80.0), ("shape", 2.0), ("sync", 1.0), ("rate_sync", 10.0)] },
        FactoryPreset { name: "Sixteenth Shimmer", values: &[("depth", 45.0), ("shape", 1.0), ("sync", 1.0), ("rate_sync", 14.0)] },
        FactoryPreset { name: "Ramp Duck", values: &[("depth", 85.0), ("shape", 3.0), ("sync", 1.0), ("rate_sync", 8.0)] },
        FactoryPreset { name: "Ping-Pong Pulse", values: &[("depth", 80.0), ("shape", 1.0), ("stereo_phase", 180.0), ("sync", 1.0), ("rate_sync", 11.0)] },
        FactoryPreset { name: "Combo Wobble", values: &[("rate", 6.2), ("depth", 50.0)] },
        FactoryPreset { name: "Wide Swirl", values: &[("rate", 3.5), ("depth", 60.0), ("stereo_phase", 120.0)] },
        FactoryPreset { name: "Rotor Blades", values: &[("rate", 14.0), ("depth", 100.0), ("shape", 2.0)] },
        FactoryPreset { name: "Parallel Flutter", values: &[("rate", 9.0), ("depth", 90.0), ("mix", 45.0)] },
    ],
    sidechain: false,
    hooks: Hooks::NONE,
};
