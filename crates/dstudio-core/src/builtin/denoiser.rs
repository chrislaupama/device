//! De-Noiser: parameter ids, ranges, defaults, the panel arrangement and the
//! factory presets.

use super::*;
use crate::device::PluginKind;

pub static DENOISER: DeviceDef = DeviceDef {
    id: "denoiser",
    name: "De-Noiser",
    kind: PluginKind::Effect,
    category: "Restoration",
    purpose: "Removes steady background noise such as hiss, air and fans, band by band across the spectrum.",
    params: &[
        ParamDef::new("threshold", "Threshold", -80.0, 0.0, -40.0, LINEAR, "dB", Format::Db)
            .help("Bands quieter than this are treated as noise."),
        ParamDef::new("reduction", "Reduction", -60.0, 0.0, -20.0, LINEAR, "dB", Format::Db)
            .help("How far noise bands are turned down: the floor sits this far below the threshold."),
        ParamDef::new("frequency_smoothing", "Frequency Smoothing", 0.0, 100.0, 50.0, LINEAR, "%", Format::Percent)
            .label("Smoothing")
            .help("Spreads each band's decision to its neighbours, which avoids watery artefacts."),
        ParamDef::new("attack", "Attack", 0.1, 50.0, 5.0, Taper::Log, "ms", Format::Plain(1))
            .help("How fast a band opens when it rises above the threshold."),
        ParamDef::new("release", "Release", 1.0, 500.0, 50.0, Taper::Log, "ms", Format::Plain(0))
            .help("How fast a band closes when it falls below the threshold."),
        ParamDef::new("mix", "Mix", 0.0, 100.0, 100.0, LINEAR, "%", Format::Percent).help("Blend of the processed and the dry signal."),
    ],
    layout: Layout {
        picker: &[],
        view: Some(View::Custom("denoiser-graph")),
        sliders: &[],
        clusters: &[
            &[Control::Knob("threshold"), Control::Knob("reduction")],
            &[Control::Knob("frequency_smoothing"), Control::Knob("attack"), Control::Knob("release")],
            &[Control::Knob("mix")],
        ],
        faces: &[],
        min_height: 176.0,
        shown_when: &[],
        fixed_width: 0.0,
        ..Layout::EMPTY
    },
    factory: &[
        FactoryPreset {
            name: "Room Tone",
            values: &[("threshold", -52.0), ("reduction", -12.0), ("frequency_smoothing", 60.0), ("release", 120.0)],
        },
        FactoryPreset {
            name: "Tape Hiss",
            values: &[("threshold", -46.0), ("reduction", -18.0), ("frequency_smoothing", 40.0), ("attack", 3.0), ("release", 80.0)],
        },
        FactoryPreset {
            name: "Fan and Air",
            values: &[("threshold", -38.0), ("reduction", -24.0), ("frequency_smoothing", 70.0), ("attack", 10.0), ("release", 200.0)],
        },
        FactoryPreset {
            name: "Dialogue Cleanup",
            values: &[("threshold", -42.0), ("reduction", -15.0), ("release", 150.0), ("mix", 90.0)],
        },
    ],
    sidechain: false,
    samples: 0,
};
