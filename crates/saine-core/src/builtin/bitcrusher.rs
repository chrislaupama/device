//! Bit Crusher (reference: the reference app's built-in "Bit Crusher"). Parameter ids,
//! ranges, defaults, tapers and the panel arrangement are taken from the
//! reference as facts; the factory presets are our own.

use super::*;
use crate::device::PluginKind;

pub static BITCRUSHER: DeviceDef = DeviceDef {
    id: "bitcrusher",
    name: "Bit Crusher",
    kind: PluginKind::Effect,
    category: "Distortion",
    purpose: "Lo-fi digital degradation: fewer bits for quantisation grit, sample-and-hold downsampling for aliasing.",
    params: &[
        ParamDef::new("bitdepth", "Bit Depth", 1.0, 32.0, 16.0, LINEAR, "bits", Format::Plain(0)).help("Resolution of each sample: fewer bits, more grit."),
        ParamDef::new("downsample", "Downsample", 1.0, 100.0, 1.0, Taper::Log, "x", Format::Factor).help("Holds each sample this many times: lowers the sample rate and adds aliasing."),
        ParamDef::new("mix", "Mix", 0.0, 100.0, 100.0, LINEAR, "%", Format::Percent0).help("Blend of crushed and dry signal."),
        ParamDef::new("dither", "Dither", 0.0, 100.0, 0.0, LINEAR, "%", Format::Percent0).help("Noise added before quantising: trades stepped distortion for hiss."),
        ParamDef::new("jitter", "Jitter", 0.0, 100.0, 0.0, LINEAR, "%", Format::Percent0).help("Randomises the hold length for an unstable, broken-converter sound."),
    ],
    layout: Layout {
        picker: &[],
        view: Some(View::Custom("crush")),
        sliders: &[],
        clusters: &[&[Control::Knob("bitdepth")], &[Control::Knob("dither"), Control::Knob("downsample"), Control::Knob("jitter")], &[Control::Knob("mix")]],
        faces: &[],
        min_height: 176.0,
        shown_when: &[],
        fixed_width: 0.0,
    },
    factory: &[
        FactoryPreset { name: "Vintage Sampler 12", values: &[("bitdepth", 12.0), ("downsample", 2.5)] },
        FactoryPreset { name: "Toy Keyboard", values: &[("bitdepth", 7.0), ("downsample", 3.5)] },
        FactoryPreset { name: "Pocket Game", values: &[("bitdepth", 4.0), ("downsample", 7.0)] },
        FactoryPreset { name: "Grainy Parallel", values: &[("bitdepth", 6.0), ("downsample", 2.0), ("mix", 45.0)] },
        FactoryPreset { name: "Dithered Dust", values: &[("bitdepth", 9.0), ("downsample", 1.6), ("dither", 70.0), ("mix", 80.0)] },
        FactoryPreset { name: "Shaky Clock", values: &[("bitdepth", 10.0), ("downsample", 6.0), ("jitter", 45.0)] },
        FactoryPreset { name: "Aliased Air", values: &[("bitdepth", 20.0), ("downsample", 18.0), ("mix", 55.0)] },
        FactoryPreset { name: "Two-Bit Rubble", values: &[("bitdepth", 2.0), ("downsample", 12.0), ("jitter", 20.0)] },
    ],
    sidechain: false,
};

/// The crushed picture: one cycle of a 100 Hz sine at 48 kHz (480 samples)
/// through the sample-and-hold and quantiser model, deterministic noise.
/// Returns (dry, crushed) per sample.
pub fn crush_picture(bits: f32, downsample: f32, mix: f32, dither: f32, jitter: f32) -> Vec<(f32, f32)> {
    const N: usize = 480;
    let levels = 2f32.powf(bits.clamp(1.0, 32.0) - 1.0);
    let h = downsample.max(1.0);
    let (mix, dither, jitter) = (mix / 100.0, dither / 100.0, jitter / 100.0);
    let mut seed: u32 = 1;
    let mut rand = move || {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        (seed >> 8) as f32 / 8_388_608.0 - 1.0
    };
    let mut out = Vec::with_capacity(N);
    let mut held = 0.0f32;
    let mut count = f32::INFINITY;
    let mut hold = h;
    for i in 0..N {
        let x = (std::f32::consts::TAU * i as f32 / N as f32).sin();
        count += 1.0;
        if count >= hold {
            count = if count - hold < 1.0 { count - hold } else { 0.0 };
            hold = (h + rand() * jitter * h * 0.5).max(1.0);
            let tpdf = 0.5 * (rand() + rand()) * dither / levels;
            held = ((x + tpdf) * levels).round() / levels;
        }
        out.push((x, x * (1.0 - mix) + held * mix));
    }
    out
}
