//! Tape Saturation: parameter ids, ranges, defaults, tapers, the panel
//! arrangement and the factory presets.

use super::*;
use crate::device::PluginKind;

const MODES: &[&str] = &["Reel-to-Reel", "Cassette", "VHS"];

pub static TAPE_SATURATION: DeviceDef = DeviceDef {
    id: "tapesaturation",
    name: "Tape Saturation",
    kind: PluginKind::Effect,
    category: "Distortion",
    purpose: "Colours the signal like analog tape: soft saturation, gentle compression, low-end warmth, rolled-off highs and optional hiss.",
    params: &[
        ParamDef::new("drive", "Drive", 0.0, 100.0, 30.0, LINEAR, "%", Format::Percent0)
            .help("Recording level into the tape: more drive, more saturation."),
        ParamDef::new("warmth", "Warmth", 0.0, 100.0, 40.0, LINEAR, "%", Format::Percent0).help("Low-end bump and softened highs."),
        ParamDef::new("compression", "Compression", 0.0, 100.0, 30.0, LINEAR, "%", Format::Percent0)
            .help("How much the tape squashes peaks."),
        ParamDef::new("noise", "Noise", -60.0, 0.0, -60.0, LINEAR, "dB", Format::DbOrInf).help("Tape hiss level; the minimum is off."),
        ParamDef::new("mix", "Mix", 0.0, 100.0, 100.0, LINEAR, "%", Format::Percent0).help("Blend of taped and dry signal."),
        ParamDef::choice("mode", "Mode", MODES, 0)
            .help("Machine: Reel-to-Reel is clean and wide, Cassette darker and softer, VHS narrow and lo-fi."),
        ParamDef::new("output_gain", "Output", -12.0, 12.0, 0.0, LINEAR, "dB", Format::Db).help("Output level."),
    ],
    layout: Layout {
        picker: &[Control::Menu("mode")],
        view: Some(View::Custom("wave")),
        sliders: &[Control::Slider("output_gain")],
        clusters: &[
            &[Control::Knob("drive"), Control::Knob("compression")],
            &[Control::Knob("warmth"), Control::Knob("noise")],
            &[Control::Knob("mix")],
        ],
        faces: &[],
        min_height: 202.0,
        shown_when: &[],
        fixed_width: 0.0,
        ..Layout::EMPTY
    },
    factory: &[
        FactoryPreset { name: "Light Print", values: &[("drive", 15.0), ("warmth", 30.0), ("compression", 15.0)] },
        FactoryPreset { name: "Bus Cohesion", values: &[("drive", 25.0), ("warmth", 45.0), ("compression", 40.0), ("output_gain", -1.0)] },
        FactoryPreset { name: "Vocal Sheen", values: &[("drive", 20.0), ("warmth", 25.0), ("compression", 35.0), ("mix", 75.0)] },
        FactoryPreset { name: "Thick Low End", values: &[("drive", 35.0), ("warmth", 80.0), ("compression", 30.0), ("output_gain", -1.5)] },
        FactoryPreset {
            name: "Kit Squash",
            values: &[("drive", 65.0), ("warmth", 50.0), ("compression", 70.0), ("mix", 60.0), ("output_gain", -3.0)],
        },
        FactoryPreset {
            name: "Overbiased Reel",
            values: &[("drive", 85.0), ("warmth", 55.0), ("compression", 55.0), ("output_gain", -4.0)],
        },
        FactoryPreset {
            name: "Mixtape",
            values: &[("drive", 40.0), ("warmth", 60.0), ("compression", 45.0), ("noise", -42.0), ("mode", 1.0), ("output_gain", -1.5)],
        },
        FactoryPreset {
            name: "Rental Tape",
            values: &[("drive", 50.0), ("warmth", 70.0), ("compression", 50.0), ("noise", -34.0), ("mode", 2.0), ("output_gain", -2.0)],
        },
    ],
    sidechain: false,
    samples: 0,
};

/// Drawn transfer of the tape stage: drive into the mode's saturation curve
/// with a small even-order term, then the compression ceiling, then mix and
/// output (warmth and noise are filters / additive and are not part of it).
pub fn tape_transfer(x: f32, drive: f32, compression: f32, mix: f32, output_db: f32, mode: u32) -> f32 {
    let d = drive / 100.0;
    let (g, k) = match mode {
        1 => (1.0 + 8.0 * d, 0.05),
        2 => (1.0 + 6.0 * d, 0.03),
        _ => (1.0 + 5.0 * d, 0.05),
    };
    let t = x * g;
    let s = match mode {
        1 => 0.9 * t.tanh() + 0.1 * (2.0 * t).atan() / std::f32::consts::FRAC_PI_2,
        2 => t / (1.0 + t.abs()),
        _ => t.tanh(),
    };
    let s = s + k * s * s;
    let ceiling = 1.0 - 0.5 * compression / 100.0;
    let (a, w) = (s.abs(), 0.15 * ceiling);
    let lo = ceiling - w;
    let limited = if a <= lo {
        a
    } else if a >= ceiling + w {
        ceiling
    } else {
        a - (a - lo).powi(2) / (4.0 * w)
    }
    .copysign(s);
    let m = mix / 100.0;
    (x * (1.0 - m) + limited * m) * 10f32.powf(output_db / 20.0)
}
