//! Flanger (reference: the reference app's built-in "Flanger"). Parameter ids, ranges,
//! defaults, tapers and the panel arrangement are taken from the reference
//! as facts; the factory presets (names and values) are our own.

use super::*;

pub static FLANGER: DeviceDef = DeviceDef {
    id: "flanger",
    name: "Flanger",
    kind: PluginKind::Effect,
    category: "Modulation",
    purpose: "Sweeps a comb of notches through the sound with a very short moving delay: the jet-plane whoosh.",
    params: &[
        ParamDef::new("rate", "Rate", 0.01, 10.0, 0.3, Taper::Log, "Hz", Format::Freq).help("Speed of the sweep while the timebase is Free."),
        ParamDef::new("depth", "Depth", 0.0, 10.0, 3.0, LINEAR, "ms", Format::Time).help("Range of the delay sweep."),
        ParamDef::new("feedback", "Feedback", -90.0, 90.0, 60.0, LINEAR, "%", Format::Percent).help("Resonance of the comb; negative values move the peaks to odd positions for a hollow tone."),
        ParamDef::new("mix", "Mix", 0.0, 100.0, 50.0, LINEAR, "%", Format::Percent).help("Balance of the dry signal and the swept copy."),
        ParamDef::new("width", "Width", 0.0, 100.0, 50.0, LINEAR, "%", Format::Percent).help("Offset between the left and right sweep."),
        ParamDef::choice("mode", "Mode", &["Classic", "Jet", "Thru-Zero"], 0).help("Classic tape-style flanging, a brighter more resonant Jet, or Thru-Zero where the copy passes the dry signal for a deep cancellation."),
        ParamDef::choice("sync", "Sync", FREE_SYNC, 0).label("Timebase").help("Free runs at Rate in Hz; Sync follows the song tempo at Rate Sync."),
        ParamDef::choice("rate_sync", "Rate Sync", DIVISIONS, DIVISION_QUARTER).label("Rate").help("Sweep length as a note value while synced."),
        ParamDef::new("output_gain", "Output", -12.0, 12.0, 0.0, LINEAR, "dB", Format::Db).help("Final level after Mix; it still trims when Mix is at 0%."),
    ],
    layout: Layout {
        picker: &[Control::Dropdown("mode"), Control::Gap, Control::Menu("sync")],
        view: Some(View::Field(Field::new("flanger", "First notch", &["rate", "rate_sync"], &["feedback"], 300.0))),
        sliders: &[Control::Slider("output_gain")],
        clusters: &[
            &[Control::Swap(&["rate", "rate_sync"]), Control::Knob("depth")],
            &[Control::Knob("feedback"), Control::Knob("width"), Control::Knob("mix")],
        ],
        faces: &[],
        min_height: 202.0,
        shown_when: &[("rate", "sync", &[0]), ("rate_sync", "sync", &[1])],
        fixed_width: 0.0,
    },
    factory: &[
        FactoryPreset { name: "Light Flange", values: &[("rate", 0.25), ("depth", 2.0), ("feedback", 25.0), ("mix", 35.0), ("output_gain", -0.3)] },
        FactoryPreset { name: "Airplane Sweep", values: &[("rate", 0.12), ("depth", 7.0), ("feedback", 75.0), ("mode", 1.0), ("output_gain", -1.5)] },
        FactoryPreset { name: "Tight Comb", values: &[("rate", 0.2), ("depth", 1.0), ("feedback", 80.0), ("width", 30.0), ("output_gain", -2.5)] },
        FactoryPreset { name: "Inverted Hollow", values: &[("rate", 0.3), ("depth", 3.5), ("feedback", -65.0), ("output_gain", -1.5)] },
        FactoryPreset { name: "Wide Swoosh", values: &[("rate", 0.1), ("depth", 6.0), ("feedback", 55.0), ("width", 100.0), ("output_gain", -0.6)] },
        FactoryPreset { name: "Zero Crossing", values: &[("rate", 0.12), ("depth", 3.0), ("feedback", 20.0), ("mix", 55.0), ("mode", 2.0)] },
        FactoryPreset { name: "Metal Tube", values: &[("rate", 0.05), ("depth", 0.6), ("feedback", 88.0), ("output_gain", -3.0)] },
        FactoryPreset { name: "Fast Warble", values: &[("rate", 3.0), ("depth", 1.5), ("feedback", 30.0), ("mix", 45.0)] },
        FactoryPreset { name: "Two-Bar Rise", values: &[("depth", 5.0), ("feedback", 60.0), ("sync", 1.0), ("rate_sync", 2.0), ("output_gain", -1.2)] },
        FactoryPreset { name: "Quarter Whoosh", values: &[("depth", 2.5), ("feedback", 45.0), ("width", 70.0), ("sync", 1.0), ("rate_sync", 8.0), ("output_gain", -0.8)] },
    ],
    sidechain: false,
};
