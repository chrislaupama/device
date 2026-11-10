//! Limiter (reference: the reference app's built-in "Limiter"). Parameter ids,
//! ranges, defaults, tapers and the panel arrangement are taken from the
//! reference as facts; the factory preset names and values are our own.

use super::*;
use crate::device::PluginKind;

const OFF_ON: &[&str] = &["Off", "On"];

pub static LIMITER: DeviceDef = DeviceDef {
    id: "limiter",
    name: "Limiter",
    kind: PluginKind::Effect,
    category: "Dynamics",
    purpose: "Keeps peaks under a fixed ceiling so a part or mix can be pushed louder without clipping.",
    params: &[
        ParamDef::new("ceiling", "Ceiling", -12.0, 0.0, -0.3, LINEAR, "dB", Format::Db).help("Highest level the output may reach."),
        ParamDef::new("release", "Release", 1.0, 1000.0, 100.0, Taper::Log, "ms", Format::Ms(0)).help("How fast the level comes back up after a peak."),
        ParamDef::new("lookahead", "Lookahead", 0.0, 20.0, 5.0, LINEAR, "ms", Format::Ms(1)).help("How far ahead the limiter sees peaks, so it can turn down smoothly before they arrive."),
        ParamDef::choice("true_peak", "True Peak", OFF_ON, 1).help("Also holds the ceiling between samples (dBTP), not just on them."),
        ParamDef::new("mix", "Mix", 0.0, 100.0, 100.0, LINEAR, "%", Format::Percent).help("Blend of limited and dry signal."),
        ParamDef::new("input_gain", "Input", -12.0, 24.0, 0.0, LINEAR, "dB", Format::Db).help("Gain into the limiter: more input means more limiting and a louder result."),
        ParamDef::new("output_gain", "Output", -12.0, 12.0, 0.0, LINEAR, "dB", Format::Db).help("Output level after the limiter."),
    ],
    layout: Layout {
        picker: &[],
        view: Some(View::Strip(Strip::new("ceiling").range(12.0, -30.0))),
        sliders: &[Control::Slider("output_gain")],
        clusters: &[
            &[Control::Knob("input_gain")],
            &[Control::Knob("ceiling"), Control::Knob("release")],
            &[Control::Knob("lookahead"), Control::Toggle("true_peak")],
            &[Control::Knob("mix")],
        ],
        faces: &[("ceiling", Face::BrickWall { ceiling: "ceiling" })],
        fixed_width: 0.0,
        min_height: 180.0,
        shown_when: &[],
        ..Layout::EMPTY
    },
    factory: &[
        FactoryPreset { name: "Ceiling Guard", values: &[("ceiling", -0.5), ("release", 150.0), ("lookahead", 4.0)] },
        FactoryPreset { name: "Hot Master", values: &[("ceiling", -0.8), ("release", 70.0), ("lookahead", 3.0), ("input_gain", 8.0)] },
        FactoryPreset { name: "Drum Peak Shave", values: &[("ceiling", -2.0), ("release", 25.0), ("lookahead", 1.5), ("input_gain", 5.0), ("true_peak", 0.0)] },
        FactoryPreset { name: "Smooth Bus", values: &[("ceiling", -1.5), ("release", 400.0), ("lookahead", 12.0), ("input_gain", 4.0)] },
        FactoryPreset { name: "Parallel Squash", values: &[("ceiling", -6.0), ("release", 60.0), ("lookahead", 2.0), ("input_gain", 14.0), ("mix", 40.0)] },
    ],
    sidechain: false,
    samples: 0,
};
