//! Auto Filter (reference: the reference app's built-in "Auto Filter"): a resonant
//! low-pass / high-pass / band-pass / notch filter (12 or 24 dB/oct) with
//! drive, an envelope follower and an LFO that move the cutoff, a dry/wet
//! mix, and a DJ mode where one position control sweeps a low-pass (left)
//! or a high-pass (right). Parameter ids, ranges, defaults, tapers and the
//! panel arrangement are taken from the reference as facts; the factory
//! presets are our own.
//!
//! [`filter_response`] is the response the panel draws; the engine kernel
//! (engine/src/devices/auto_filter.cpp) runs the same stages.

use super::eq::Section;
use super::*;
use crate::device::PluginKind;
use std::f64::consts::PI;

const TYPES: &[&str] = &["LP", "HP", "BP", "Notch"];
const SLOPES: &[&str] = &["12", "24"];
const NOTE_VALUES: &[&str] = &["1/1", "1/2", "1/2.", "1/2T", "1/4", "1/4.", "1/4T", "1/8", "1/8.", "1/8T", "1/16", "1/16.", "1/16T", "1/32"];
const SHAPES: &[&str] = &["Sine", "Triangle", "Saw", "Square", "S&H", "Random"];
const SYNC: &[&str] = &["Free", "Sync"];
const MODES: &[&str] = &["Filter", "DJ"];

pub static AUTO_FILTER: DeviceDef = DeviceDef {
    samples: 0,
    id: "auto-filter",
    name: "Auto Filter",
    kind: PluginKind::Effect,
    category: "Filter",
    purpose: "A resonant filter that moves on its own: sweep it with an LFO, open it with the input's envelope, or ride one DJ-style control from low-pass to high-pass.",
    params: &[
        ParamDef::choice("filter_type", "Filter Type", TYPES, 0).label("Filter").help("Low-pass, high-pass, band-pass or notch."),
        ParamDef::choice("slope", "Slope", SLOPES, 0).help("12 or 24 dB per octave."),
        ParamDef::new("frequency", "Frequency", 20.0, 20_000.0, 20_000.0, Taper::Log, "Hz", Format::Freq).label("Freq").help("Cutoff (or centre) frequency before the envelope and LFO move it."),
        ParamDef::new("resonance", "Resonance", 0.0, 1.25, 0.0, LINEAR, "", Format::Plain(2)).help("Emphasis at the cutoff; high settings ring."),
        ParamDef::new("drive", "Drive", 0.0, 24.0, 0.0, LINEAR, "dB", Format::Plain(1)).help("Saturates the signal on its way into the filter."),
        ParamDef::new("env_amount", "Env Amount", -72.0, 72.0, 0.0, LINEAR, "st", Format::SignedBareZero(0)).label("Env").help("How far the input's envelope moves the cutoff, in semitones (negative closes it)."),
        ParamDef::new("env_attack", "Env Attack", 0.1, 500.0, 1.0, Taper::Log, "ms", Format::MsAuto).label("Attack").help("How fast the envelope follows a rising level."),
        ParamDef::new("env_release", "Env Release", 1.0, 3000.0, 100.0, Taper::Log, "ms", Format::MsAuto).label("Release").help("How fast the envelope falls back."),
        ParamDef::new("lfo_amount", "LFO Amount", 0.0, 72.0, 0.0, LINEAR, "st", Format::Plain(0)).label("LFO").help("How far the LFO swings the cutoff either way, in semitones."),
        ParamDef::new("lfo_rate_free", "LFO Rate", 0.01, 30.0, 1.0, Taper::Log, "Hz", Format::Rate).label("Rate").help("LFO speed in hertz (Timebase Free)."),
        ParamDef::choice("lfo_rate_sync", "LFO Rate Sync", NOTE_VALUES, 4).label("Rate").help("LFO cycle as a note length (Timebase Sync)."),
        ParamDef::choice("lfo_shape", "LFO Shape", SHAPES, 0).label("LFO").help("Waveform of the LFO."),
        ParamDef::choice("lfo_sync", "LFO Sync", SYNC, 0).label("Timebase").help("Free-running rate in hertz, or locked to the song tempo."),
        ParamDef::new("lfo_stereo", "LFO Stereo", 0.0, 180.0, 0.0, LINEAR, "deg", Format::Degrees).label("Stereo").help("Phase offset of the right channel's LFO; widens the sweep."),
        ParamDef::new("mix", "Mix", 0.0, 100.0, 100.0, LINEAR, "%", Format::Percent).help("Blend of filtered and dry signal."),
        ParamDef::choice("mode", "Mode", MODES, 0).help("Filter, or DJ: one control from low-pass (left) through open to high-pass (right)."),
        ParamDef::new("dj_position", "DJ Position", -1.0, 1.0, 0.0, LINEAR, "", Format::Plain(2)).label("Position").help("DJ mode: left of centre low-passes, right high-passes."),
    ],
    layout: Layout {
        picker: &[Control::ModeDropdown { param: "filter_type", mode: "mode" }, Control::Menu("slope"), Control::Dropdown("lfo_shape"), Control::Menu("lfo_sync")],
        view: Some(View::Custom("auto-filter")),
        sliders: &[],
        clusters: &[
            &[Control::Knob("drive")],
            &[Control::Knob("env_amount"), Control::Knob("env_attack"), Control::Knob("env_release")],
            &[Control::Knob("lfo_amount"), Control::Swap(&["lfo_rate_free", "lfo_rate_sync"])],
            &[Control::Knob("mix")],
        ],
        faces: &[],
        min_height: 202.0,
        shown_when: &[("lfo_rate_free", "lfo_sync", &[0]), ("lfo_rate_sync", "lfo_sync", &[1])],
        fixed_width: 0.0,
    },
    factory: &[
        FactoryPreset { name: "Gentle Dusk", values: &[("frequency", 3200.0), ("resonance", 0.12)] },
        FactoryPreset { name: "Thin Air", values: &[("filter_type", 1.0), ("frequency", 700.0), ("resonance", 0.08), ("slope", 1.0)] },
        FactoryPreset { name: "Through the Wall", values: &[("frequency", 300.0), ("resonance", 0.1), ("slope", 1.0)] },
        FactoryPreset { name: "Bar Sweep Down", values: &[("frequency", 1200.0), ("resonance", 0.35), ("slope", 1.0), ("lfo_amount", 40.0), ("lfo_sync", 1.0), ("lfo_rate_sync", 0.0), ("lfo_shape", 1.0)] },
        FactoryPreset { name: "Half-Note Breathe", values: &[("frequency", 900.0), ("resonance", 0.28), ("lfo_amount", 30.0), ("lfo_sync", 1.0), ("lfo_rate_sync", 1.0), ("lfo_shape", 0.0), ("mix", 90.0)] },
        FactoryPreset { name: "Wide Drift", values: &[("frequency", 1800.0), ("resonance", 0.2), ("lfo_amount", 24.0), ("lfo_rate_free", 0.15), ("lfo_shape", 0.0), ("lfo_stereo", 120.0)] },
        FactoryPreset { name: "Dotted Wub", values: &[("frequency", 400.0), ("resonance", 0.42), ("slope", 1.0), ("drive", 6.0), ("lfo_amount", 40.0), ("lfo_sync", 1.0), ("lfo_rate_sync", 8.0), ("lfo_shape", 0.0)] },
        FactoryPreset { name: "Sixteenth Gate", values: &[("frequency", 650.0), ("resonance", 0.3), ("slope", 1.0), ("lfo_amount", 34.0), ("lfo_sync", 1.0), ("lfo_rate_sync", 10.0), ("lfo_shape", 3.0)] },
        FactoryPreset { name: "Stepped Chatter", values: &[("filter_type", 2.0), ("frequency", 1400.0), ("resonance", 0.3), ("lfo_amount", 28.0), ("lfo_sync", 1.0), ("lfo_rate_sync", 7.0), ("lfo_shape", 4.0)] },
        FactoryPreset { name: "Wandering Notch", values: &[("filter_type", 3.0), ("frequency", 1000.0), ("resonance", 0.4), ("lfo_amount", 30.0), ("lfo_rate_free", 0.3), ("lfo_shape", 5.0), ("lfo_stereo", 90.0)] },
        FactoryPreset { name: "Quack", values: &[("filter_type", 2.0), ("frequency", 450.0), ("resonance", 0.45), ("drive", 3.0), ("env_amount", 48.0), ("env_attack", 3.0), ("env_release", 120.0)] },
        FactoryPreset { name: "Pluck Opener", values: &[("frequency", 300.0), ("resonance", 0.25), ("slope", 1.0), ("env_amount", 60.0), ("env_attack", 0.5), ("env_release", 250.0)] },
        FactoryPreset { name: "Duck Under", values: &[("frequency", 8000.0), ("resonance", 0.2), ("slope", 1.0), ("env_amount", -48.0), ("env_attack", 2.0), ("env_release", 400.0)] },
        FactoryPreset { name: "Saw Rise", values: &[("filter_type", 1.0), ("frequency", 200.0), ("resonance", 0.18), ("lfo_amount", 36.0), ("lfo_sync", 1.0), ("lfo_rate_sync", 0.0), ("lfo_shape", 2.0)] },
        FactoryPreset { name: "Crunch Box", values: &[("filter_type", 2.0), ("frequency", 1100.0), ("resonance", 0.15), ("slope", 1.0), ("drive", 12.0)] },
        FactoryPreset { name: "DJ High Sweep", values: &[("mode", 1.0), ("dj_position", 0.35), ("resonance", 0.25)] },
    ],
    sidechain: false,
};

/// One filter setting (plain units) as the panel draws it and the engine runs it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FilterSetting {
    /// 0 LP, 1 HP, 2 BP, 3 Notch (DJ mode picks LP or HP from the position).
    pub kind: u32,
    pub slope24: bool,
    pub cutoff: f32,
    pub resonance: f32,
    /// 0..1.
    pub mix: f32,
    /// DJ mode position (-1..1); None in Filter mode.
    pub dj: Option<f32>,
}

/// Resonance (0..1.25) to the filter's Q: 0.5 at zero, about 40 at the top.
pub fn resonance_q(reso: f32) -> f64 {
    let t = (reso as f64 / 1.25).clamp(0.0, 1.0);
    0.5 * (t * (6.0 - 1.6 * t)).exp()
}

/// Level correction that keeps a resonant filter's peak in check.
pub fn resonance_gain(q: f64) -> f64 {
    ((1.0 + (q / 2.0).powi(2)) / 1.0625).powf(-0.1)
}

/// Q of the second stage of a 24 dB/oct filter.
pub const SECOND_STAGE_Q: f64 = 0.54;

/// DJ position below this (either way) leaves the signal open.
pub const DJ_DEAD_ZONE: f32 = 0.06;

/// DJ mode: (kind 0 LP / 1 HP, cutoff Hz, resonance scale, wet 0..1).
pub fn dj_map(position: f32) -> (u32, f32, f32, f32) {
    let a = position.abs();
    if a < DJ_DEAD_ZONE {
        return (0, 20_000.0, 0.0, 0.0);
    }
    let n = ((a - DJ_DEAD_ZONE) / (1.0 - DJ_DEAD_ZONE)).clamp(0.0, 1.0);
    let smooth = |x: f32| {
        let x = x.clamp(0.0, 1.0);
        x * x * (3.0 - 2.0 * x)
    };
    if position < 0.0 {
        let fc = 20_000.0 * 0.001f32.powf(n);
        (0, fc, n * n, smooth((20_000.0 / fc).log2() * 3.0))
    } else {
        let fc = 20.0 * 1000f32.powf(n);
        (1, fc, n * n, smooth((fc / 20.0).log2() * 3.0))
    }
}

/// The filter stages of a setting: (sections, wet level incl. resonance
/// correction, wet fraction). The output is `dry * (1 - wet) + wet * level * H(x)`.
pub fn filter_stages(s: &FilterSetting, fs: f64) -> (Vec<Section>, f64, f64) {
    let (kind, cutoff, reso, wet) = match s.dj {
        Some(p) => {
            let (k, fc, scale, w) = dj_map(p);
            (k, fc, s.resonance * scale, w as f64 * s.mix as f64)
        }
        None => (s.kind.min(3), s.cutoff, s.resonance, s.mix as f64),
    };
    let q = resonance_q(reso);
    let g = (PI * (cutoff.max(1.0) as f64 / fs).clamp(1e-6, 0.49)).tan();
    let stage = |q: f64| {
        let k = 1.0 / q;
        let m = match kind {
            0 => [0.0, 0.0, 1.0],
            1 => [1.0, -k, -1.0],
            2 => [0.0, k, 0.0],
            _ => [1.0, -k, 0.0],
        };
        Section::Two { g, k, m }
    };
    let mut v = vec![stage(q)];
    if s.slope24 {
        v.push(stage(SECOND_STAGE_Q));
    }
    (v, resonance_gain(q), wet.clamp(0.0, 1.0))
}

/// Magnitude (dB, floor -120) of the filter at `freq`, mix included.
pub fn filter_response(s: &FilterSetting, freq: f64, fs: f64) -> f64 {
    let (st, level, wet) = filter_stages(s, fs);
    let (mut re, mut im) = (1.0, 0.0);
    for sec in &st {
        let (a, b) = sec.response(freq, fs);
        (re, im) = (re * a - im * b, re * b + im * a);
    }
    let (re, im) = (1.0 - wet + wet * level * re, wet * level * im);
    (10.0 * (re * re + im * im).max(1e-12).log10()).max(-120.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FS: f64 = 48_000.0;
    fn lp(cutoff: f32, reso: f32) -> FilterSetting {
        FilterSetting { kind: 0, slope24: false, cutoff, resonance: reso, mix: 1.0, dj: None }
    }

    #[test]
    fn resonance_map_and_correction() {
        assert!((resonance_q(0.0) - 0.5).abs() < 1e-9);
        assert!((resonance_q(1.25) - 0.5 * 4.4f64.exp()).abs() < 1e-6, "about 40 at the top");
        assert!((resonance_gain(0.5) - 1.0).abs() < 1e-9);
        assert!(resonance_gain(40.0) < 0.6);
    }

    #[test]
    fn response_shapes() {
        let open = lp(20_000.0, 0.0);
        assert!(filter_response(&open, 100.0, FS).abs() < 0.01);
        let s = lp(1000.0, 0.0);
        assert!((filter_response(&s, 1000.0, FS) + 6.02).abs() < 0.05, "Q 0.5: -6 dB at the cutoff");
        assert!(filter_response(&s, 8000.0, FS) < -30.0);
        let mut s24 = s;
        s24.slope24 = true;
        assert!(filter_response(&s24, 8000.0, FS) < filter_response(&s, 8000.0, FS) - 20.0);
        let peak = lp(1000.0, 1.0);
        assert!(filter_response(&peak, 1000.0, FS) > 12.0, "resonance peaks at the cutoff");
        let bp = FilterSetting { kind: 2, ..s };
        assert!(filter_response(&bp, 1000.0, FS).abs() < 0.01);
        let notch = FilterSetting { kind: 3, resonance: 0.5, ..s };
        assert!(filter_response(&notch, 1000.0, FS) < -60.0);
        let half = FilterSetting { mix: 0.5, ..lp(200.0, 0.0) };
        assert!((filter_response(&half, 15_000.0, FS) + 6.02).abs() < 0.1, "half mix keeps half the dry");
    }

    #[test]
    fn dj_mode_maps_both_sides() {
        assert_eq!(dj_map(0.0).3, 0.0);
        assert_eq!(dj_map(0.05).3, 0.0);
        let (k, fc, scale, wet) = dj_map(-1.0);
        assert!(k == 0 && (fc - 20.0).abs() < 0.01 && scale == 1.0 && wet == 1.0);
        let (k, fc, _, wet) = dj_map(1.0);
        assert!(k == 1 && (fc - 20_000.0).abs() < 1.0 && wet == 1.0);
        let (_, fc, _, _) = dj_map(-0.53);
        assert!(fc > 600.0 && fc < 700.0, "half way down is the geometric middle ({fc})");
        let s = FilterSetting { kind: 0, slope24: false, cutoff: 20_000.0, resonance: 0.3, mix: 1.0, dj: Some(0.5) };
        assert!(filter_response(&s, 50.0, FS) < -20.0, "right of centre high-passes");
    }
}
