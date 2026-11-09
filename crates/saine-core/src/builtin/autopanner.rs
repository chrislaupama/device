//! Auto-Panner (reference: the reference app's built-in "Auto-Panner"). Parameter ids,
//! ranges, defaults, tapers and the panel arrangement are taken from the
//! reference as facts; the factory presets (names and values) are our own.

use super::*;

const SHAPES: &[&str] = &["Sine", "Triangle", "Square", "Random"];

pub static AUTOPANNER: DeviceDef = DeviceDef {
    id: "autopanner",
    name: "Auto-Panner",
    kind: PluginKind::Effect,
    category: "Modulation",
    purpose: "Moves the sound between the left and right speakers with an LFO.",
    params: &[
        ParamDef::new("rate", "Rate", 0.1, 20.0, 2.0, Taper::Log, "Hz", Format::Freq).help("LFO speed while the timebase is Free."),
        ParamDef::new("depth", "Depth", 0.0, 100.0, 80.0, LINEAR, "%", Format::Percent).help("How far the sound travels from the centre; 100% reaches the speakers."),
        ParamDef::choice("shape", "Shape", SHAPES, 0).help("Path of the movement: smooth (Sine, Triangle), a jump between sides (Square) or a new random spot each cycle (Random)."),
        ParamDef::new("phase", "Phase", 0.0, 360.0, 0.0, LINEAR, "deg", Format::Plain(0)).help("Start point of the LFO cycle (shifts the movement against the beat)."),
        ParamDef::new("mix", "Mix", 0.0, 100.0, 100.0, LINEAR, "%", Format::Percent).help("Blend of the panned and the untouched signal."),
        ParamDef::choice("sync", "Sync", FREE_SYNC, 1).label("Timebase").help("Free runs at Rate in Hz; Sync follows the song tempo at Rate Sync."),
        ParamDef::choice("rate_sync", "Rate Sync", DIVISIONS, DIVISION_QUARTER).label("Rate").help("LFO cycle length as a note value while synced."),
    ],
    layout: Layout {
        picker: &[Control::Dropdown("shape"), Control::Gap, Control::Menu("sync")],
        view: Some(View::Field(Field::new("autopanner", "", &["rate", "rate_sync"], &["depth"], 401.0))),
        sliders: &[],
        clusters: &[&[Control::Swap(&["rate", "rate_sync"]), Control::Knob("depth")], &[Control::Knob("phase"), Control::Knob("mix")]],
        faces: &[],
        min_height: 204.0,
        shown_when: &[("rate", "sync", &[0]), ("rate_sync", "sync", &[1])],
    },
    factory: &[
        FactoryPreset { name: "Gentle Sway", values: &[("depth", 40.0), ("rate_sync", 5.0)] },
        FactoryPreset { name: "Quarter Swing", values: &[("depth", 75.0)] },
        FactoryPreset { name: "Eighth Bounce", values: &[("depth", 85.0), ("shape", 1.0), ("rate_sync", 11.0)] },
        FactoryPreset { name: "Side Switch", values: &[("depth", 90.0), ("shape", 2.0), ("rate_sync", 8.0)] },
        FactoryPreset { name: "Triplet Roll", values: &[("depth", 65.0), ("rate_sync", 12.0)] },
        FactoryPreset { name: "Bar Wander", values: &[("depth", 55.0), ("phase", 90.0), ("rate_sync", 3.0)] },
        FactoryPreset { name: "Dotted Weave", values: &[("depth", 60.0), ("shape", 1.0), ("rate_sync", 10.0)] },
        FactoryPreset { name: "Scatter Shot", values: &[("depth", 80.0), ("shape", 3.0), ("rate_sync", 11.0)] },
        FactoryPreset { name: "Slow Orbit", values: &[("rate", 0.25), ("depth", 90.0), ("sync", 0.0)] },
        FactoryPreset { name: "Fast Spin", values: &[("rate", 5.2), ("depth", 60.0), ("sync", 0.0)] },
    ],
    sidechain: false,
};
