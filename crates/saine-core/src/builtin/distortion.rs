//! Distortion (reference: the reference app's built-in "Distortion"). Parameter ids,
//! ranges, defaults, tapers and the panel arrangement are taken from the
//! reference as facts; the factory presets are our own.

use super::*;
use crate::device::PluginKind;

const MODES: &[&str] = &["Overdrive", "Fuzz", "Tube", "Hard Clip"];

pub static DISTORTION: DeviceDef = DeviceDef {
    id: "distortion",
    name: "Distortion",
    kind: PluginKind::Effect,
    category: "Distortion",
    purpose: "Drives the signal into a waveshaper for grit, warmth or fuzz, with a tone control and a dry/wet blend.",
    params: &[
        ParamDef::new("drive", "Drive", 0.0, 100.0, 30.0, LINEAR, "%", Format::Percent0).help("How hard the signal is pushed into the shaper."),
        ParamDef::new("tone", "Tone", 20.0, 20000.0, 4000.0, Taper::Log, "Hz", Format::Hz).help("Low-pass cutoff on the distorted signal: lower is darker."),
        ParamDef::new("mix", "Mix", 0.0, 100.0, 100.0, LINEAR, "%", Format::Percent0).help("Blend of distorted and dry signal."),
        ParamDef::new("output", "Gain", -24.0, 6.0, 0.0, LINEAR, "dB", Format::Db).help("Level of the distorted signal before the mix."),
        ParamDef::choice("mode", "Mode", MODES, 0).help("Shaper: Overdrive is smooth, Fuzz asymmetric and buzzy, Tube soft with even harmonics, Hard Clip flat-topped."),
        ParamDef::new("bias", "Bias", 0.0, 100.0, 0.0, LINEAR, "%", Format::Percent0).help("Offsets the signal into the shaper for asymmetric, even-harmonic distortion."),
        ParamDef::new("output_gain", "Output", -12.0, 12.0, 0.0, LINEAR, "dB", Format::Db).help("Output level."),
    ],
    layout: Layout {
        picker: &[Control::Menu("mode")],
        view: Some(View::Custom("wave")),
        sliders: &[Control::Slider("output_gain")],
        clusters: &[&[Control::Knob("drive"), Control::Knob("bias")], &[Control::Knob("tone")], &[Control::Knob("output"), Control::Knob("mix")]],
        faces: &[],
        min_height: 202.0,
        shown_when: &[],
        fixed_width: 0.0,
    },
    factory: &[
        FactoryPreset { name: "Edge of Breakup", values: &[("drive", 18.0), ("tone", 7500.0), ("output_gain", -2.5)] },
        FactoryPreset { name: "Velvet Tube", values: &[("drive", 28.0), ("tone", 8200.0), ("mode", 2.0), ("bias", 35.0), ("mix", 65.0), ("output_gain", -3.0)] },
        FactoryPreset { name: "Amp Stack", values: &[("drive", 52.0), ("tone", 4200.0), ("output", -8.0), ("output_gain", -1.5)] },
        FactoryPreset { name: "Clipped Snare", values: &[("drive", 42.0), ("tone", 10500.0), ("mode", 3.0), ("output", -6.0), ("mix", 70.0), ("output_gain", -2.0)] },
        FactoryPreset { name: "Low End Hair", values: &[("drive", 36.0), ("tone", 2600.0), ("mix", 40.0), ("output_gain", -3.5)] },
        FactoryPreset { name: "Octave Fuzz", values: &[("drive", 70.0), ("tone", 3800.0), ("mode", 1.0), ("bias", 25.0), ("output", -9.0), ("output_gain", -3.0)] },
        FactoryPreset { name: "Telephone Grit", values: &[("drive", 60.0), ("tone", 1500.0), ("mode", 3.0), ("output", -4.0), ("output_gain", -4.0)] },
        FactoryPreset { name: "Starved Tube", values: &[("drive", 80.0), ("tone", 5600.0), ("mode", 2.0), ("bias", 70.0), ("output", -7.0), ("output_gain", -2.0)] },
        FactoryPreset { name: "Parallel Heat", values: &[("drive", 85.0), ("tone", 6500.0), ("mix", 30.0), ("output", -6.0), ("output_gain", -1.0)] },
        FactoryPreset { name: "Wrecked", values: &[("drive", 100.0), ("tone", 9000.0), ("mode", 1.0), ("bias", 90.0), ("output", -12.0), ("output_gain", -3.0)] },
    ],
    sidechain: false,
};

/// Shaper of one mode (the drawn model; the engine kernel uses the same curves).
fn shape(mode: u32, x: f32, bias: f32) -> f32 {
    match mode {
        1 => {
            if x >= 0.0 {
                (2.0 * x).tanh()
            } else {
                (3.0 * x).atan() / std::f32::consts::FRAC_PI_2
            }
        }
        2 => {
            let asym = 0.5 * (2.0 * bias).tanh();
            x / (1.0 + x.abs()) - asym * x * x
        }
        3 => x.clamp(-1.0, 1.0),
        _ => x.tanh(),
    }
}

/// Static transfer of the distortion (the tone filter left out): input sample
/// -> output sample for plain parameter values (percent, dB, mode index).
pub fn distortion_transfer(x: f32, drive: f32, bias: f32, gain_db: f32, mix: f32, output_db: f32, mode: u32) -> f32 {
    let g = 1.0 + 10.0 * drive / 100.0;
    let b = bias / 100.0;
    let o = 0.5 * b;
    let wet = (shape(mode, x * g + o, b) - shape(mode, o, b)) * 10f32.powf(gain_db / 20.0);
    let m = mix / 100.0;
    (x * (1.0 - m) + wet * m) * 10f32.powf(output_db / 20.0)
}
