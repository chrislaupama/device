//! Transient Shaper (reference: the reference app's built-in "Transient Shaper"). Parameter ids,
//! ranges, defaults, tapers and the panel arrangement are taken from the
//! reference as facts; the factory preset names and values are our own.

use super::*;
use crate::device::PluginKind;

const OFF_ON: &[&str] = &["Off", "On"];

pub static TRANSIENT_SHAPER: DeviceDef = DeviceDef {
    id: "transient-shaper",
    name: "Transient Shaper",
    kind: PluginKind::Effect,
    category: "Dynamics",
    purpose: "Makes the start of each hit louder or softer and its tail longer or shorter, independent of level.",
    params: &[
        ParamDef::new("attack", "Attack", -100.0, 100.0, 0.0, LINEAR, "%", Format::SignedPercent).help("Boosts (+) or softens (−) the start of each hit."),
        ParamDef::new("sustain", "Sustain", -100.0, 100.0, 0.0, LINEAR, "%", Format::SignedPercent).help("Lengthens (+) or shortens (−) the body and tail after each hit."),
        ParamDef::new("sensitivity", "Sensitivity", 0.0, 100.0, 50.0, LINEAR, "%", Format::Percent).label("Sense").help("How strongly the detector reacts to changes in level."),
        ParamDef::new("output_gain", "Output Gain", -12.0, 12.0, 0.0, LINEAR, "dB", Format::Db).label("Output").help("Final level, after Mix."),
        ParamDef::new("mix", "Mix", 0.0, 100.0, 100.0, LINEAR, "%", Format::Percent).help("Blend of shaped and dry signal."),
        ParamDef::new("attack_time", "Attack Time", 3.0, 100.0, 30.0, Taper::Log, "ms", Format::Ms(0)).label("Atk Time").help("How long the start of a hit lasts, the part Attack changes; short values touch only the click."),
        ParamDef::new("sustain_time", "Sustain Time", 20.0, 2000.0, 2000.0, Taper::Log, "ms", Format::Ms(0)).label("Sus Time").max_text("Full").help("How long the Sustain change lasts after a hit; Full covers the whole tail."),
        ParamDef::choice("clip", "Clip", OFF_ON, 0).help("Soft-clips the output at 0 dBFS, after the output gain."),
    ],
    layout: Layout {
        picker: &[],
        view: Some(View::Custom("gain-scope")),
        sliders: &[Control::Slider("output_gain")],
        clusters: &[
            &[Control::Knob("attack"), Control::Knob("sustain")],
            &[Control::Knob("sensitivity")],
            &[Control::Knob("attack_time"), Control::Knob("sustain_time")],
            &[Control::Toggle("clip"), Control::Knob("mix")],
        ],
        faces: &[],
        fixed_width: 0.0,
        min_height: 199.0,
        shown_when: &[],
        ..Layout::EMPTY
    },
    factory: &[
        FactoryPreset { name: "Kick Click", values: &[("attack", 45.0), ("sustain", -10.0), ("sensitivity", 55.0), ("output_gain", -1.0), ("attack_time", 12.0)] },
        FactoryPreset { name: "Snare Crack", values: &[("attack", 55.0), ("sustain", 10.0), ("output_gain", -1.5), ("attack_time", 6.0), ("sustain_time", 400.0)] },
        FactoryPreset { name: "Dry Up Drums", values: &[("attack", 15.0), ("sustain", -55.0), ("output_gain", 2.5), ("sustain_time", 250.0)] },
        FactoryPreset { name: "Pull Back Room", values: &[("sustain", -40.0), ("sensitivity", 60.0), ("output_gain", 3.0)] },
        FactoryPreset { name: "Round Off Hits", values: &[("attack", -55.0), ("output_gain", 1.0), ("attack_time", 40.0)] },
        FactoryPreset { name: "Long Tails", values: &[("sustain", 60.0), ("sensitivity", 45.0), ("output_gain", -5.0)] },
        FactoryPreset { name: "Thick Toms", values: &[("attack", 30.0), ("sustain", 35.0), ("output_gain", -3.5), ("attack_time", 60.0), ("sustain_time", 600.0)] },
        FactoryPreset { name: "Parallel Snap", values: &[("attack", 70.0), ("sustain", -20.0), ("output_gain", -1.0), ("mix", 45.0), ("clip", 1.0)] },
        FactoryPreset { name: "Gentle Strum", values: &[("attack", -30.0), ("sustain", 15.0), ("attack_time", 10.0)] },
        FactoryPreset { name: "Hard Snap Clip", values: &[("attack", 80.0), ("sensitivity", 70.0), ("output_gain", 2.0), ("attack_time", 15.0), ("clip", 1.0)] },
    ],
    sidechain: false,
    samples: 0,
};
