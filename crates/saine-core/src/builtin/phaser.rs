//! Phaser (reference: the reference app's built-in "Phaser"). Parameter ids, ranges,
//! defaults, tapers and the panel arrangement are taken from the reference
//! as facts; the factory presets (names and values) are our own.

use super::*;

pub static PHASER: DeviceDef = DeviceDef {
    id: "phaser",
    name: "Phaser",
    kind: PluginKind::Effect,
    category: "Modulation",
    purpose: "Sweeps a few soft notches up and down the spectrum with a chain of all-pass filters: a swirling, liquid tone.",
    params: &[
        ParamDef::new("rate", "Rate", 0.01, 10.0, 0.5, Taper::Log, "Hz", Format::Freq).help("Speed of the sweep while the timebase is Free."),
        ParamDef::new("depth", "Depth", 0.0, 100.0, 70.0, LINEAR, "%", Format::Percent).help("Range of the sweep; 100% covers 200 Hz to 8 kHz."),
        ParamDef::new("feedback", "Feedback", -95.0, 95.0, 40.0, LINEAR, "%", Format::Percent).help("Resonance of the notches' edges; negative values invert it."),
        ParamDef::new("mix", "Mix", 0.0, 100.0, 50.0, LINEAR, "%", Format::Percent).help("Balance of the dry signal and the phased copy; 50% gives the deepest notches."),
        ParamDef::choice("stages", "Stages", &["2", "4", "6", "12"], 1).help("All-pass stages: every two make one notch."),
        ParamDef::new("width", "Width", 0.0, 100.0, 50.0, LINEAR, "%", Format::Percent).help("Offset between the left and right sweep."),
        ParamDef::choice("sync", "Sync", FREE_SYNC, 0).label("Timebase").help("Free runs at Rate in Hz; Sync follows the song tempo at Rate Sync."),
        ParamDef::choice("rate_sync", "Rate Sync", DIVISIONS, DIVISION_QUARTER).label("Rate").help("Sweep length as a note value while synced."),
        ParamDef::new("output_gain", "Output", -12.0, 12.0, 0.0, LINEAR, "dB", Format::Db).help("Final level after Mix; it still trims when Mix is at 0%."),
    ],
    layout: Layout {
        picker: &[Control::Menu("stages"), Control::Gap, Control::Menu("sync")],
        view: Some(View::Field(Field::new("phaser", "First notch", &["rate", "rate_sync"], &["feedback"], 289.0))),
        sliders: &[Control::Slider("output_gain")],
        clusters: &[
            &[Control::Swap(&["rate", "rate_sync"]), Control::Knob("depth")],
            &[Control::Knob("feedback"), Control::Knob("width"), Control::Knob("mix")],
        ],
        faces: &[],
        min_height: 202.0,
        fixed_width: 0.0,
        shown_when: &[("rate", "sync", &[0]), ("rate_sync", "sync", &[1])],
    },
    factory: &[
        FactoryPreset { name: "Four-Pole Swirl", values: &[("rate", 0.45), ("depth", 75.0), ("feedback", 45.0), ("width", 30.0), ("output_gain", -1.0)] },
        FactoryPreset { name: "Quick Funk", values: &[("rate", 2.2), ("depth", 55.0), ("feedback", 30.0), ("width", 15.0)] },
        FactoryPreset { name: "Two-Stage Wobble", values: &[("rate", 1.8), ("depth", 60.0), ("feedback", 20.0), ("stages", 0.0)] },
        FactoryPreset { name: "Resonant Six", values: &[("rate", 0.3), ("depth", 85.0), ("feedback", 80.0), ("stages", 2.0), ("output_gain", -1.6)] },
        FactoryPreset { name: "Negative Hollow", values: &[("rate", 0.4), ("depth", 75.0), ("feedback", -60.0), ("stages", 2.0), ("output_gain", -1.0)] },
        FactoryPreset { name: "Twelve Deep", values: &[("rate", 0.2), ("depth", 90.0), ("feedback", 60.0), ("stages", 3.0), ("output_gain", -1.2)] },
        FactoryPreset { name: "Glacial Sweep", values: &[("rate", 0.05), ("depth", 100.0), ("feedback", 55.0), ("stages", 3.0), ("width", 80.0), ("output_gain", -1.4)] },
        FactoryPreset { name: "Wide Stereo Sweep", values: &[("rate", 0.25), ("depth", 70.0), ("feedback", 25.0), ("mix", 40.0), ("width", 100.0), ("output_gain", 0.8)] },
        FactoryPreset { name: "Half-Note Phase", values: &[("depth", 80.0), ("feedback", 50.0), ("stages", 2.0), ("sync", 1.0), ("rate_sync", 5.0), ("output_gain", -1.0)] },
        FactoryPreset { name: "Sixteenth Ripple", values: &[("depth", 40.0), ("feedback", 25.0), ("sync", 1.0), ("rate_sync", 14.0), ("output_gain", 1.0)] },
    ],
    sidechain: false,
    samples: 0,
};
