//! Compressor: parameter ids, ranges, defaults, tapers, the panel arrangement
//! and the factory presets.

use super::*;
use crate::device::PluginKind;

const STYLES: &[&str] = &["Clean", "Opto", "Punch", "Smooth"];
const OFF_ON: &[&str] = &["Off", "On"];

pub static COMPRESSOR: DeviceDef = DeviceDef {
    id: "compressor",
    name: "Compressor",
    kind: PluginKind::Effect,
    category: "Dynamics",
    purpose: "Evens out level: it holds the loudest peaks back so quieter parts come up, making a part steadier, denser or punchier.",
    params: &[
        ParamDef::new("threshold", "Threshold", -60.0, 0.0, -24.0, LINEAR, "dB", Format::Db).help("Level above which gain reduction starts."),
        ParamDef::new("ratio", "Ratio", 1.0, 20.0, 4.0, Taper::Log, ":1", Format::Ratio).help("How strongly level above the threshold is reduced; 1:1 is off."),
        ParamDef::new("attack", "Attack", 1.0, 1000.0, 10.0, Taper::Log, "ms", Format::Time).help("How fast reduction sets in."),
        ParamDef::new("release", "Release", 10.0, 3000.0, 100.0, Taper::Log, "ms", Format::Time).help("How fast reduction lets go."),
        ParamDef::new("knee", "Knee", 0.0, 40.0, 6.0, LINEAR, "dB", Format::Plain(1)).help("Width of the soft transition around the threshold."),
        ParamDef::new("makeup_gain", "Gain", -12.0, 24.0, 0.0, LINEAR, "dB", Format::Db).help("Make-up gain after compression."),
        ParamDef::new("mix", "Mix", 0.0, 100.0, 100.0, LINEAR, "%", Format::Percent).help("Blend of compressed and dry signal (parallel compression)."),
        ParamDef::choice("style", "Style", STYLES, 0).label("Circuit").help("Detector character: Clean is transparent, Opto smooth and program dependent, Punch fast with an emphasised attack, Smooth slow and gentle."),
        ParamDef::choice("sidechainEnable", "Sidechain Enable", OFF_ON, 0).not_automatable().help("Set while a sidechain source is picked."),
        ParamDef::new("output_gain", "Output", -12.0, 12.0, 0.0, LINEAR, "dB", Format::Db).help("Output level."),
        ParamDef::new("sidechain_gain", "Sidechain Gain", -24.0, 24.0, 0.0, LINEAR, "dB", Format::Db)
            .label("Gain")
            .help("Scales the sidechain signal into the detector only; works while a sidechain source is picked."),
    ],
    layout: Layout {
        picker: &[Control::Menu("style"), Control::Sidechain, Control::Pill("sidechain_gain")],
        view: Some(View::DynamicsStrip { threshold: "threshold" }),
        sliders: &[Control::Slider("output_gain")],
        clusters: &[
            &[Control::Knob("threshold"), Control::Knob("ratio")],
            &[Control::Knob("attack"), Control::Knob("release"), Control::Knob("knee"), Control::Knob("makeup_gain"), Control::Knob("mix")],
        ],
        faces: &[("ratio", Face::SoftKneeCurve { threshold: "threshold", ratio: "ratio", knee: "knee" })],
        min_height: 217.0,
        shown_when: &[],
        ..Layout::EMPTY
    },
    factory: &[
        FactoryPreset { name: "Gentle Leveler", values: &[("threshold", -18.0), ("ratio", 2.0), ("attack", 20.0), ("release", 250.0), ("knee", 12.0), ("makeup_gain", 3.0), ("style", 3.0)] },
        FactoryPreset { name: "Vocal Rider", values: &[("threshold", -22.0), ("ratio", 3.0), ("attack", 8.0), ("release", 160.0), ("knee", 8.0), ("makeup_gain", 4.0), ("style", 1.0)] },
        FactoryPreset { name: "Drum Snap", values: &[("threshold", -20.0), ("ratio", 5.0), ("attack", 25.0), ("release", 80.0), ("knee", 3.0), ("makeup_gain", 3.0), ("style", 2.0)] },
        FactoryPreset { name: "Bass Hold", values: &[("threshold", -26.0), ("ratio", 4.0), ("attack", 12.0), ("release", 180.0), ("knee", 6.0), ("makeup_gain", 4.5)] },
        FactoryPreset { name: "Parallel Crush", values: &[("threshold", -40.0), ("ratio", 12.0), ("attack", 2.0), ("release", 120.0), ("knee", 0.0), ("makeup_gain", 12.0), ("mix", 35.0), ("style", 2.0)] },
        FactoryPreset { name: "Bus Glue", values: &[("threshold", -14.0), ("ratio", 1.8), ("attack", 30.0), ("release", 400.0), ("knee", 16.0), ("makeup_gain", 1.5), ("style", 3.0)] },
    ],
    sidechain: true,
    samples: 0,
};
