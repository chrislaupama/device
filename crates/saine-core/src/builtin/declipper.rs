//! De-Clipper (reference: the reference app's built-in "De-Clipper"). Parameter ids,
//! ranges, defaults and the panel arrangement are taken from the reference as
//! facts; factory preset names and values are our own.

use super::*;
use crate::device::PluginKind;

const FACTORS: &[&str] = &["2x", "4x", "8x"];

pub static DECLIPPER: DeviceDef = DeviceDef {
    id: "declipper",
    name: "De-Clipper",
    kind: PluginKind::Effect,
    category: "Restoration",
    purpose: "Rebuilds peaks that were flattened by clipping and keeps the repaired signal under a ceiling.",
    params: &[
        ParamDef::new("threshold", "Threshold", -6.0, 0.0, -1.0, LINEAR, "dB", Format::Db).help("Clip detection level: samples at or above it count as clipped. Set it at or just below where the audio was flattened."),
        ParamDef::new("ceiling", "Ceiling", -6.0, 0.0, -0.3, LINEAR, "dB", Format::Db).help("The output never passes this level; rebuilt passages are turned down to fit."),
        ParamDef::choice("oversampling", "Oversampling", FACTORS, 1).help("Resolution of the true-peak check that holds the output under the ceiling."),
        ParamDef::new("mix", "Mix", 0.0, 100.0, 100.0, LINEAR, "%", Format::Percent).help("Blend of the repaired and the original signal."),
    ],
    layout: Layout {
        picker: &[Control::Menu("oversampling")],
        view: Some(View::Custom("declipper-wave")),
        sliders: &[],
        clusters: &[&[Control::Knob("threshold"), Control::Knob("ceiling")], &[Control::Knob("mix")]],
        faces: &[],
        min_height: 202.0,
        shown_when: &[],
        fixed_width: 244.0,
        ..Layout::EMPTY
    },
    factory: &[
        FactoryPreset { name: "Gentle Repair", values: &[("threshold", -1.5), ("mix", 85.0)] },
        FactoryPreset { name: "Restore Peaks", values: &[("threshold", -2.5), ("ceiling", -1.0)] },
        FactoryPreset { name: "Live Recording", values: &[("threshold", -3.5), ("ceiling", -0.5), ("oversampling", 2.0)] },
        FactoryPreset { name: "Crushed Master", values: &[("threshold", -5.0), ("ceiling", -1.0), ("oversampling", 2.0)] },
    ],
    sidechain: false,
    samples: 0,
};
