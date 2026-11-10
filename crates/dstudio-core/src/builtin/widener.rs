//! Stereo Widener: parameter ids, ranges, defaults, tapers, the panel
//! arrangement and the factory presets.

use super::*;
use crate::device::PluginKind;

pub static WIDENER: DeviceDef = DeviceDef {
    id: "widener",
    name: "Stereo Widener",
    kind: PluginKind::Effect,
    category: "Imaging",
    purpose: "Mid/side widening: scales the side signal for a wider or narrower image while keeping the low end centred.",
    params: &[
        ParamDef::new("width2", "Width", 0.0, 400.0, 200.0, LINEAR, "%", Format::Percent0).help("Side level relative to the original: 0% is mono, 100% unchanged, 400% very wide."),
        ParamDef::new("midgain", "Mid Gain", -12.0, 12.0, 0.0, LINEAR, "dB", Format::Db).help("Level of the centre (mid) signal."),
        ParamDef::new("sidegain", "Side Gain", -12.0, 12.0, 0.0, LINEAR, "dB", Format::Db).help("Extra level of the side signal."),
        ParamDef::new("lowcut", "Low Cut", 20.0, 500.0, 120.0, Taper::Log, "Hz", Format::Freq).help("Below this frequency the widening fades out so the bass stays centred."),
        ParamDef::new("mix", "Mix", 0.0, 100.0, 100.0, LINEAR, "%", Format::Percent0).help("Blend of widened and original signal."),
    ],
    layout: Layout {
        picker: &[],
        view: Some(View::Custom("stereo-field")),
        sliders: &[],
        clusters: &[&[Control::Knob("width2"), Control::Knob("lowcut"), Control::Knob("midgain"), Control::Knob("sidegain"), Control::Knob("mix")]],
        faces: &[],
        min_height: 176.0,
        shown_when: &[],
        fixed_width: 388.0,
        ..Layout::EMPTY
    },
    factory: &[
        FactoryPreset { name: "Gentle Spread", values: &[("width2", 130.0)] },
        FactoryPreset { name: "Open Chorus Bed", values: &[("width2", 240.0), ("lowcut", 200.0), ("midgain", -1.0)] },
        FactoryPreset { name: "Super Side", values: &[("width2", 340.0), ("lowcut", 250.0), ("sidegain", 2.0), ("midgain", -1.5)] },
        FactoryPreset { name: "Air Only", values: &[("width2", 260.0), ("lowcut", 480.0)] },
        FactoryPreset { name: "Centre Focus", values: &[("width2", 60.0), ("midgain", 1.5)] },
        FactoryPreset { name: "Fold to Centre", values: &[("width2", 0.0), ("lowcut", 20.0)] },
    ],
    sidechain: false,
    samples: 0,
};

/// Effective width (percent) at frequency f: the side path is scaled by
/// width x side gain and high-passed (one pole) at the low cut, the mid by
/// mid gain, each blended with the dry signal by mix.
pub fn widener_response(f: f32, width: f32, mid_db: f32, side_db: f32, lowcut: f32, mix: f32, sample_rate: f32) -> f32 {
    let (w, x) = (width / 100.0, mix / 100.0);
    let m = 10f32.powf(mid_db / 20.0);
    let s = 10f32.powf(side_db / 20.0);
    let pole = (-std::f32::consts::TAU * lowcut / sample_rate).exp();
    let wf = std::f32::consts::TAU * f / sample_rate;
    // High-pass pole (1 - z^-1) / (1 - pole z^-1) at e^{jw}, added to the dry side as complex values.
    let (nr, ni) = (pole * (1.0 - wf.cos()), pole * wf.sin());
    let (dr, di) = (1.0 - pole * wf.cos(), pole * wf.sin());
    let dd = dr * dr + di * di;
    let (hr, hi) = ((nr * dr + ni * di) / dd, (ni * dr - nr * di) / dd);
    let k = x * w * s;
    100.0 * ((1.0 - x + k * hr).powi(2) + (k * hi).powi(2)).sqrt() / (1.0 - x + x * m).max(1e-6)
}
