//! Chorus (reference: the reference app's built-in "Chorus"). Parameter ids, ranges,
//! defaults, tapers and the panel arrangement are taken from the reference
//! as facts; the factory presets (names and values) are our own.

use super::*;

pub static CHORUS: DeviceDef = DeviceDef {
    id: "chorus",
    name: "Chorus",
    kind: PluginKind::Effect,
    category: "Modulation",
    purpose: "Thickens and widens a sound with slightly detuned, moving copies of it.",
    params: &[
        ParamDef::new("rate", "Rate", 0.1, 10.0, 0.5, Taper::Log, "Hz", Format::Freq).help("Speed of the delay movement while the timebase is Free."),
        ParamDef::new("depth", "Depth", 0.0, 8.0, 4.0, LINEAR, "ms", Format::Time).help("How far the copies' delay moves: more depth, more detune."),
        ParamDef::new("mix", "Mix", 0.0, 100.0, 50.0, LINEAR, "%", Format::Percent).help("Balance of the dry signal and the chorus voices."),
        ParamDef::choice("voices", "Voices", &["1", "2", "3", "4"], 0).help("Number of moving copies, spread evenly over the LFO cycle."),
        ParamDef::new("width", "Width", 0.0, 100.0, 100.0, LINEAR, "%", Format::Percent).help("Offset between the left and right movement; 0% is mono-safe."),
        ParamDef::new("feedback", "Feedback", 0.0, 90.0, 0.0, LINEAR, "%", Format::Percent).help("Feeds the voices back into the delay for a resonant, metallic edge."),
        ParamDef::choice("sync", "Sync", FREE_SYNC, 0).label("Timebase").help("Free runs at Rate in Hz; Sync follows the song tempo at Rate Sync."),
        ParamDef::choice("rate_sync", "Rate Sync", DIVISIONS, DIVISION_QUARTER).label("Rate").help("LFO cycle length as a note value while synced."),
        ParamDef::new("output_gain", "Output", -12.0, 12.0, 0.0, LINEAR, "dB", Format::Db).help("Final level after Mix; it still trims when Mix is at 0%."),
    ],
    layout: Layout {
        picker: &[Control::Menu("voices"), Control::Gap, Control::Menu("sync")],
        view: Some(View::Field(Field::new("chorus", "First notch", &["rate", "rate_sync"], &["feedback"], 289.0))),
        sliders: &[Control::Slider("output_gain")],
        clusters: &[
            &[Control::Swap(&["rate", "rate_sync"]), Control::Knob("depth")],
            &[Control::Knob("feedback"), Control::Knob("width"), Control::Knob("mix")],
        ],
        faces: &[],
        min_height: 202.0,
        fixed_width: 0.0,
        shown_when: &[("rate", "sync", &[0]), ("rate_sync", "sync", &[1])],
        ..Layout::EMPTY
    },
    factory: &[
        FactoryPreset { name: "Light Double", values: &[("rate", 0.35), ("depth", 1.8), ("mix", 30.0), ("voices", 1.0), ("output_gain", 0.3)] },
        FactoryPreset { name: "Classic Stereo", values: &[("rate", 0.6), ("depth", 3.5), ("mix", 50.0)] },
        FactoryPreset { name: "Thick Quad", values: &[("rate", 0.5), ("depth", 6.0), ("mix", 45.0), ("voices", 3.0), ("output_gain", 0.6)] },
        FactoryPreset { name: "Detuned Section", values: &[("rate", 0.9), ("depth", 5.0), ("mix", 60.0), ("voices", 2.0), ("output_gain", 0.5)] },
        FactoryPreset { name: "Slow Wide Pad", values: &[("rate", 0.18), ("depth", 7.0), ("mix", 55.0), ("voices", 2.0)] },
        FactoryPreset { name: "Glassy Keys", values: &[("rate", 1.2), ("depth", 2.2), ("mix", 40.0), ("feedback", 15.0), ("width", 85.0)] },
        FactoryPreset { name: "Resonant Chorus", values: &[("rate", 0.7), ("depth", 2.5), ("mix", 40.0), ("feedback", 45.0), ("output_gain", -0.8)] },
        FactoryPreset { name: "Centered Bass", values: &[("rate", 0.45), ("depth", 3.0), ("mix", 35.0), ("width", 0.0), ("output_gain", -0.5)] },
        FactoryPreset { name: "Vibrato Wash", values: &[("rate", 4.8), ("depth", 1.5), ("mix", 100.0), ("width", 0.0)] },
        FactoryPreset { name: "Half-Note Swirl", values: &[("depth", 4.5), ("mix", 50.0), ("feedback", 20.0), ("sync", 1.0), ("rate_sync", 5.0)] },
    ],
    sidechain: false,
    samples: 0,
};
