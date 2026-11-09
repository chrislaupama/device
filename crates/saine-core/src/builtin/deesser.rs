//! De-Esser (reference: the reference app's built-in "De-Esser"). Parameter ids, ranges,
//! defaults and the panel arrangement are taken from the reference as facts;
//! factory preset names and values are our own.

use super::*;
use crate::device::PluginKind;

const OFF_ON: &[&str] = &["Off", "On"];

pub static DEESSER: DeviceDef = DeviceDef {
    id: "deesser",
    name: "De-Esser",
    kind: PluginKind::Effect,
    category: "Restoration",
    purpose: "Turns down harsh s, sh and t sounds: a band around the frequency is cut only while it is louder than the threshold.",
    params: &[
        ParamDef::new("frequency", "Frequency", 2000.0, 16000.0, 7000.0, Taper::Log, "Hz", Format::Hz).help("Centre of the band that is watched and cut."),
        ParamDef::new("bandwidth", "Bandwidth", 0.1, 4.0, 1.0, LINEAR, "", Format::Plain(2)).help("Q of the band: higher values make it narrower."),
        ParamDef::new("threshold", "Threshold", -40.0, 0.0, -36.0, LINEAR, "dB", Format::Db).help("Band level above which the cut starts."),
        ParamDef::new("reduction", "Reduction", 0.0, 30.0, 8.0, LINEAR, "dB", Format::DbCut).help("Largest cut applied to the band."),
        ParamDef::choice("listen", "Listen", OFF_ON, 0).reset_on_load().help("Plays only the watched band, to find the sibilance."),
        ParamDef::new("mix", "Mix", 0.0, 100.0, 100.0, LINEAR, "%", Format::Percent).help("Blend of the processed and the dry signal."),
    ],
    layout: Layout {
        picker: &[],
        view: Some(View::Custom("deesser-graph")),
        sliders: &[],
        clusters: &[
            &[Control::Knob("frequency"), Control::Knob("bandwidth")],
            &[Control::Knob("threshold"), Control::Knob("reduction")],
            &[Control::Toggle("listen")],
            &[Control::Knob("mix")],
        ],
        faces: &[],
        min_height: 176.0,
        shown_when: &[],
        fixed_width: 0.0,
        ..Layout::EMPTY
    },
    factory: &[
        FactoryPreset { name: "Soft Sibilance", values: &[("frequency", 6500.0), ("bandwidth", 1.1), ("threshold", -30.0), ("reduction", 5.0)] },
        FactoryPreset { name: "Low Voice", values: &[("frequency", 4800.0), ("bandwidth", 0.9), ("threshold", -34.0), ("reduction", 10.0)] },
        FactoryPreset { name: "Bright Voice", values: &[("frequency", 8200.0), ("bandwidth", 0.8), ("threshold", -35.0), ("reduction", 10.0)] },
        FactoryPreset { name: "Podcast Speech", values: &[("frequency", 6000.0), ("bandwidth", 0.7), ("threshold", -32.0), ("reduction", 7.0), ("mix", 90.0)] },
        FactoryPreset { name: "Hi-Hat Tamer", values: &[("frequency", 11000.0), ("bandwidth", 0.6), ("threshold", -38.0), ("reduction", 7.0)] },
    ],
    sidechain: false,
    samples: 0,
    hooks: crate::builtin::Hooks::NONE,
};
