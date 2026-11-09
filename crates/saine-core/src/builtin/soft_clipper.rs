//! Soft Clipper (reference: the reference app's built-in "Soft Clipper"). Parameter ids,
//! ranges, defaults, tapers and the panel arrangement are taken from the
//! reference as facts; the factory presets are our own.

use super::*;
use crate::device::PluginKind;

const SHAPES: &[&str] = &["Smooth", "Warm", "Hard"];
const OFF_ON: &[&str] = &["Off", "On"];

pub static SOFT_CLIPPER: DeviceDef = DeviceDef {
    id: "soft-clipper",
    name: "Soft Clipper",
    kind: PluginKind::Effect,
    category: "Distortion",
    purpose: "Rounds off peaks above a ceiling so a part gets louder without hard digital clipping.",
    params: &[
        ParamDef::new("input", "Input", -12.0, 24.0, 0.0, LINEAR, "dB", Format::Db).help("Gain into the clipper: more input, more clipping."),
        ParamDef::new("ceiling", "Ceiling", -12.0, 0.0, 0.0, LINEAR, "dB", Format::DbNoPlus).help("Highest level the clipper lets through."),
        ParamDef::new("softness", "Softness", 0.0, 100.0, 50.0, LINEAR, "%", Format::Percent0).help("How gradually the clipping sets in below the ceiling."),
        ParamDef::choice("shape", "Shape", SHAPES, 0).help("Curve: Smooth is symmetric, Warm rounds the negative half sooner (even harmonics), Hard is a tight polynomial knee."),
        ParamDef::choice("gain_match", "Gain Match", OFF_ON, 0).help("Compensates the input gain so the level stays the same while you push into the clipper."),
        ParamDef::new("mix", "Mix", 0.0, 100.0, 100.0, LINEAR, "%", Format::Percent0).help("Blend of clipped and dry signal."),
        ParamDef::new("output_gain", "Output", -12.0, 12.0, 0.0, LINEAR, "dB", Format::Db).help("Output level."),
    ],
    layout: Layout {
        picker: &[Control::Menu("shape"), Control::Toggle("gain_match")],
        view: Some(View::Custom("wave")),
        sliders: &[Control::Slider("output_gain")],
        clusters: &[&[Control::Knob("input"), Control::Knob("ceiling")], &[Control::Knob("softness")], &[Control::Knob("mix")]],
        faces: &[],
        min_height: 202.0,
        shown_when: &[],
        fixed_width: 0.0,
        ..Layout::EMPTY
    },
    factory: &[
        FactoryPreset { name: "Peak Shave", values: &[("input", 3.0), ("ceiling", -0.5), ("softness", 30.0)] },
        FactoryPreset { name: "Round Edges", values: &[("input", 5.0), ("softness", 85.0), ("output_gain", -2.5)] },
        FactoryPreset { name: "Transient Trim", values: &[("input", 8.0), ("ceiling", -4.0), ("softness", 15.0), ("shape", 2.0), ("output_gain", -2.0)] },
        FactoryPreset { name: "Even Glow", values: &[("input", 10.0), ("softness", 75.0), ("shape", 1.0), ("gain_match", 1.0)] },
        FactoryPreset { name: "Brickwall-ish", values: &[("input", 2.0), ("ceiling", -0.3), ("softness", 2.0), ("shape", 2.0)] },
        FactoryPreset { name: "Loud and Proud", values: &[("input", 7.0), ("ceiling", -0.8), ("softness", 40.0), ("gain_match", 0.0)] },
    ],
    sidechain: false,
    hooks: Hooks::NONE,
    samples: 0,
};

/// Clip curve of a shape for a normalised input u (1.0 = the ceiling) and
/// softness s in 0..1.
pub fn clip_curve(shape: u32, u: f32, s: f32) -> f32 {
    let k = 24.0 * (1.5f32 / 24.0).powf(s);
    let sat = |a: f32, k: f32| if a > 1.0 { 1.0 / (1.0 + a.powf(-k)).powf(1.0 / k) } else { a / (1.0 + a.powf(k)).powf(1.0 / k) };
    match shape {
        1 => {
            if u >= 0.0 {
                sat(u, k)
            } else {
                -sat(-u, (0.5 * k).max(1.2))
            }
        }
        2 => {
            let w = 0.01 + 0.49 * s;
            let a = u.abs();
            let y = if a <= 1.0 - w {
                a
            } else if a >= 1.0 + w {
                1.0
            } else {
                a - (a - (1.0 - w)).powi(2) / (4.0 * w)
            };
            y.copysign(u)
        }
        _ => sat(u.abs(), k).copysign(u),
    }
}

/// Drawn transfer: (x (1 - mix) + c clip(x a / c) m mix) o.
pub fn clipper_transfer(x: f32, input_db: f32, ceiling_db: f32, softness: f32, shape: u32, gain_match: bool, mix: f32, output_db: f32) -> f32 {
    let a = 10f32.powf(input_db / 20.0);
    let c = 10f32.powf(ceiling_db / 20.0);
    let m = if gain_match { 1.0 / a } else { 1.0 };
    let mx = mix / 100.0;
    (x * (1.0 - mx) + c * clip_curve(shape, x * a / c, softness / 100.0) * m * mx) * 10f32.powf(output_db / 20.0)
}

/// The same chain without clipping (the dashed "open" reference).
pub fn clipper_open(x: f32, input_db: f32, gain_match: bool, mix: f32, output_db: f32) -> f32 {
    let a = 10f32.powf(input_db / 20.0);
    let m = if gain_match { 1.0 / a } else { 1.0 };
    let mx = mix / 100.0;
    (x * (1.0 - mx) + x * a * m * mx) * 10f32.powf(output_db / 20.0)
}
