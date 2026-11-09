//! De-Hummer (reference: the reference app's built-in "De-Hummer"). Parameter ids,
//! ranges, defaults and the panel arrangement are taken from the reference as
//! facts; factory preset names and values are our own.

use super::*;
use crate::device::PluginKind;

pub static DEHUMMER: DeviceDef = DeviceDef {
    id: "dehummer",
    name: "De-Hummer",
    kind: PluginKind::Effect,
    category: "Restoration",
    purpose: "Notches out mains hum at 50 or 60 Hz and its harmonics.",
    params: &[
        ParamDef::new("frequency", "Frequency", 40.0, 70.0, 60.0, LINEAR, "Hz", Format::Plain(1)).help("Fundamental of the hum (50 Hz in most of the world, 60 Hz in the Americas)."),
        ParamDef::new("num_harmonics", "Harmonics", 1.0, 8.0, 4.0, LINEAR, "", Format::Plain(0)).step(1.0).help("How many notches: the fundamental and its multiples."),
        ParamDef::new("notch_width", "Q", 0.5, 10.0, 5.0, LINEAR, "", Format::Plain(1)).help("Sharpness of each notch: higher is narrower."),
        ParamDef::new("depth", "Depth", -60.0, 0.0, -30.0, LINEAR, "dB", Format::Db).help("Cut at the centre of each notch."),
        ParamDef::new("mix", "Mix", 0.0, 100.0, 100.0, LINEAR, "%", Format::Percent).help("Blend of the processed and the dry signal."),
    ],
    layout: Layout {
        picker: &[],
        view: Some(View::Custom("dehummer-graph")),
        sliders: &[],
        clusters: &[
            &[Control::Knob("frequency"), Control::Knob("num_harmonics")],
            &[Control::Knob("notch_width"), Control::Knob("depth")],
            &[Control::Knob("mix")],
        ],
        faces: &[],
        min_height: 176.0,
        shown_when: &[],
        fixed_width: 0.0,
    },
    factory: &[
        FactoryPreset { name: "Mains 50 Hz", values: &[("frequency", 50.0), ("num_harmonics", 4.0), ("notch_width", 6.0), ("depth", -40.0)] },
        FactoryPreset { name: "Mains 60 Hz", values: &[("frequency", 60.0), ("num_harmonics", 4.0), ("notch_width", 6.0), ("depth", -40.0)] },
        FactoryPreset { name: "Amp Buzz 50 Hz", values: &[("frequency", 50.0), ("num_harmonics", 7.0), ("notch_width", 3.0), ("depth", -50.0)] },
        FactoryPreset { name: "Amp Buzz 60 Hz", values: &[("frequency", 60.0), ("num_harmonics", 7.0), ("notch_width", 3.0), ("depth", -50.0)] },
        FactoryPreset { name: "Fundamental Only", values: &[("num_harmonics", 1.0), ("notch_width", 9.0), ("depth", -45.0)] },
    ],
    sidechain: false,
};
