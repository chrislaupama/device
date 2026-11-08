//! Built-in "Synth" device panel (bottom panel, Devices tab, instrument tracks).
//!
//! Every control maps to a real `saine_core::SynthParams` field (contract in
//! `saine_core::instrument`). Continuous controls (knobs, level faders) preview
//! while dragging: the edited parameters are held in `SynthUi::pending` and
//! published to the engine with the snapshot override in `Controller::publish`,
//! but no command is executed. The gesture end (`synth_param_end`) commits
//! exactly ONE `SetSynthParams`. Dropdowns, preset loads and double-click
//! resets commit one `SetSynthParams` each.
//!
//! The core coalesces `SetSynthParams` per track+preset within 750 ms. That
//! would merge two separate gestures made in quick succession (or a preset load
//! followed by a quick tweak) into one undo step, so gesture commits are
//! executed with the document's coalesce window set to zero (`commit_synth`).
//!
//! The displays (oscillator shapes, filter response, envelopes, LFO) are SVG
//! path strings computed here and drawn with Slint `Path` elements. The
//! oscillator shapes follow the field docs in `instrument.rs`; the exact DSP is
//! the engine's (see `osc_sample`).

use crate::controller::{BottomTab, Controller, sync_model};
use crate::{MenuItem, Studio, SynthDisplay, SynthParam};
use saine_core::instrument::{Adsr, FilterMode, Waveform, factory_presets};
use saine_core::{Command, SynthParams, TrackId, TrackKind};
use std::f32::consts::PI;

// ------------------------------------------------------------------ parameter table

/// Continuous parameters, indexed like `Studio.synth-params` (mirrored by the
/// `SP` global in `ui/synth.slint`).
pub mod p {
    pub const OSC1_SHAPE: usize = 0;
    pub const OSC1_FINE: usize = 1;
    pub const OSC1_LEVEL: usize = 2;
    pub const CROSS_FM: usize = 3;
    pub const OSC2_SHAPE: usize = 4;
    pub const OSC2_FINE: usize = 5;
    pub const OSC2_LEVEL: usize = 6;
    pub const SUB_LEVEL: usize = 7;
    pub const NOISE_LEVEL: usize = 8;
    pub const CUTOFF: usize = 9;
    pub const RESONANCE: usize = 10;
    pub const KEY_TRACK: usize = 11;
    pub const ENV_AMOUNT: usize = 12;
    pub const AMP_A: usize = 13;
    pub const AMP_D: usize = 14;
    pub const AMP_S: usize = 15;
    pub const AMP_R: usize = 16;
    pub const FLT_A: usize = 17;
    pub const FLT_D: usize = 18;
    pub const FLT_S: usize = 19;
    pub const FLT_R: usize = 20;
    pub const LFO_RATE: usize = 21;
    pub const LFO_PITCH: usize = 22;
    pub const LFO_CUTOFF: usize = 23;
    pub const LFO_AMP: usize = 24;
    pub const GLIDE: usize = 25;
    pub const UNISON_DETUNE: usize = 26;
    pub const OUTPUT: usize = 27;
    pub const POLYPHONY: usize = 28;
    pub const UNISON: usize = 29;
    pub const COUNT: usize = 30;
}

/// Dropdown (discrete) parameters, indexed like `Studio.synth-choices`.
pub mod ch {
    pub const OSC1_WAVE: usize = 0;
    pub const OSC1_OCT: usize = 1;
    pub const OSC1_SEMI: usize = 2;
    pub const OSC2_WAVE: usize = 3;
    pub const OSC2_OCT: usize = 4;
    pub const OSC2_SEMI: usize = 5;
    pub const FILTER_MODE: usize = 6;
    pub const LFO_WAVE: usize = 7;
    pub const COUNT: usize = 8;
}

/// Display slots, indexed like `Studio.synth-displays`.
pub mod disp {
    pub const OSC1: usize = 0;
    #[allow(dead_code)]
    pub const OSC2: usize = 1;
    pub const SUB: usize = 2;
    pub const NOISE: usize = 3;
    pub const FILTER: usize = 4;
    pub const AMP_ENV: usize = 5;
    pub const FLT_ENV: usize = 6;
    pub const LFO: usize = 7;
    pub const COUNT: usize = 8;
}

/// Display sizes in points (the path viewbox equals the element size).
pub const DISP_W: f32 = 245.0;
pub const DISP_H: f32 = 128.0;
pub const FILTER_W: f32 = 610.0;

/// Category list (left column), in reference order. Modulation, Macros and Effects are
/// PLACEHOLDER pages ("not available yet").
pub const PAGES: [&str; 9] = ["Oscillators", "Sub & Noise", "Filter", "Envelopes", "LFOs", "Modulation", "Macros", "Voice", "Effects"];

#[derive(Clone, Copy)]
enum Taper {
    Lin,
    /// Logarithmic between positive bounds.
    Log,
    /// v = lo + (hi - lo) * n^2 (finer control near the bottom).
    Sq,
}

#[derive(Clone, Copy)]
enum Fmt {
    Pct,
    SignedPct,
    Cents,
    PlainCents,
    Hz,
    Time,
    Oct,
    Db,
    Voices,
    Unison,
    Glide,
}

struct Def {
    lo: f32,
    hi: f32,
    taper: Taper,
    fmt: Fmt,
    int: bool,
}

const fn d(lo: f32, hi: f32, taper: Taper, fmt: Fmt) -> Def {
    Def { lo, hi, taper, fmt, int: false }
}

fn def(id: usize) -> Def {
    use Fmt::*;
    use Taper::*;
    match id {
        p::OSC1_SHAPE | p::OSC2_SHAPE | p::OSC1_LEVEL | p::OSC2_LEVEL | p::CROSS_FM | p::SUB_LEVEL | p::NOISE_LEVEL => d(0.0, 1.0, Lin, Pct),
        p::OSC1_FINE | p::OSC2_FINE => d(-100.0, 100.0, Lin, Cents),
        p::CUTOFF => d(20.0, 20_000.0, Log, Hz),
        p::RESONANCE | p::KEY_TRACK | p::AMP_S | p::FLT_S | p::LFO_AMP => d(0.0, 1.0, Lin, Pct),
        p::ENV_AMOUNT => d(-1.0, 1.0, Lin, SignedPct),
        p::AMP_A | p::AMP_D | p::FLT_A | p::FLT_D => d(0.001, 10.0, Log, Time),
        p::AMP_R | p::FLT_R => d(0.001, 20.0, Log, Time),
        p::LFO_RATE => d(0.01, 20.0, Log, Hz),
        p::LFO_PITCH => d(0.0, 1200.0, Sq, PlainCents),
        p::LFO_CUTOFF => d(0.0, 6.0, Lin, Oct),
        p::GLIDE => d(0.0, 2.0, Sq, Glide),
        p::UNISON_DETUNE => d(0.0, 50.0, Lin, PlainCents),
        p::OUTPUT => d(0.0, 2.0, Lin, Db),
        p::POLYPHONY => Def { lo: 1.0, hi: 16.0, taper: Lin, fmt: Voices, int: true },
        p::UNISON => Def { lo: 1.0, hi: 4.0, taper: Lin, fmt: Unison, int: true },
        _ => d(0.0, 1.0, Lin, Pct),
    }
}

/// Bipolar controls draw their value arc from the top of the knob.
pub fn bipolar(id: usize) -> bool {
    matches!(id, p::OSC1_FINE | p::OSC2_FINE | p::ENV_AMOUNT)
}

fn env(sp: &SynthParams, id: usize) -> (&Adsr, usize) {
    if (p::AMP_A..=p::AMP_R).contains(&id) { (&sp.amp_env, id - p::AMP_A) } else { (&sp.filter_env, id - p::FLT_A) }
}

pub fn get(sp: &SynthParams, id: usize) -> f32 {
    match id {
        p::OSC1_SHAPE => sp.osc[0].shape,
        p::OSC1_FINE => sp.osc[0].fine_cents,
        p::OSC1_LEVEL => sp.osc[0].level,
        p::CROSS_FM => sp.cross_fm,
        p::OSC2_SHAPE => sp.osc[1].shape,
        p::OSC2_FINE => sp.osc[1].fine_cents,
        p::OSC2_LEVEL => sp.osc[1].level,
        p::SUB_LEVEL => sp.sub_level,
        p::NOISE_LEVEL => sp.noise_level,
        p::CUTOFF => sp.filter.cutoff_hz,
        p::RESONANCE => sp.filter.resonance,
        p::KEY_TRACK => sp.filter.key_track,
        p::ENV_AMOUNT => sp.filter.env_amount,
        p::AMP_A..=p::FLT_R => {
            let (e, k) = env(sp, id);
            [e.attack, e.decay, e.sustain, e.release][k]
        }
        p::LFO_RATE => sp.lfo.rate_hz,
        p::LFO_PITCH => sp.lfo.to_pitch_cents,
        p::LFO_CUTOFF => sp.lfo.to_cutoff_oct,
        p::LFO_AMP => sp.lfo.to_amp,
        p::GLIDE => sp.voice.glide,
        p::UNISON_DETUNE => sp.voice.unison_detune_cents,
        p::OUTPUT => sp.output,
        p::POLYPHONY => sp.voice.polyphony as f32,
        p::UNISON => sp.voice.unison as f32,
        _ => 0.0,
    }
}

/// Set a parameter (clamped to its documented range, lightly rounded).
pub fn set(sp: &mut SynthParams, id: usize, v: f32) {
    let df = def(id);
    let mut v = if v.is_finite() { v.clamp(df.lo, df.hi) } else { get(sp, id) };
    v = match df.fmt {
        _ if df.int => v.round(),
        Fmt::Hz => (v * 100.0).round() / 100.0,
        Fmt::Cents | Fmt::PlainCents => (v * 10.0).round() / 10.0,
        Fmt::Time | Fmt::Glide => (v * 10_000.0).round() / 10_000.0,
        _ => (v * 1000.0).round() / 1000.0,
    }
    .clamp(df.lo, df.hi);
    match id {
        p::OSC1_SHAPE => sp.osc[0].shape = v,
        p::OSC1_FINE => sp.osc[0].fine_cents = v,
        p::OSC1_LEVEL => sp.osc[0].level = v,
        p::CROSS_FM => sp.cross_fm = v,
        p::OSC2_SHAPE => sp.osc[1].shape = v,
        p::OSC2_FINE => sp.osc[1].fine_cents = v,
        p::OSC2_LEVEL => sp.osc[1].level = v,
        p::SUB_LEVEL => sp.sub_level = v,
        p::NOISE_LEVEL => sp.noise_level = v,
        p::CUTOFF => sp.filter.cutoff_hz = v,
        p::RESONANCE => sp.filter.resonance = v,
        p::KEY_TRACK => sp.filter.key_track = v,
        p::ENV_AMOUNT => sp.filter.env_amount = v,
        p::AMP_A..=p::FLT_R => {
            let e = if id <= p::AMP_R { &mut sp.amp_env } else { &mut sp.filter_env };
            match (id - p::AMP_A) % 4 {
                0 => e.attack = v,
                1 => e.decay = v,
                2 => e.sustain = v,
                _ => e.release = v,
            }
        }
        p::LFO_RATE => sp.lfo.rate_hz = v,
        p::LFO_PITCH => sp.lfo.to_pitch_cents = v,
        p::LFO_CUTOFF => sp.lfo.to_cutoff_oct = v,
        p::LFO_AMP => sp.lfo.to_amp = v,
        p::GLIDE => sp.voice.glide = v,
        p::UNISON_DETUNE => sp.voice.unison_detune_cents = v,
        p::OUTPUT => sp.output = v,
        p::POLYPHONY => sp.voice.polyphony = v as u8,
        p::UNISON => sp.voice.unison = v as u8,
        _ => {}
    }
}

/// Control position 0..1 for a value.
pub fn to_norm(id: usize, v: f32) -> f32 {
    let df = def(id);
    let n = match df.taper {
        Taper::Lin => (v - df.lo) / (df.hi - df.lo),
        Taper::Log => (v / df.lo).ln() / (df.hi / df.lo).ln(),
        Taper::Sq => ((v - df.lo) / (df.hi - df.lo)).max(0.0).sqrt(),
    };
    n.clamp(0.0, 1.0)
}

/// Value for a control position 0..1.
pub fn from_norm(id: usize, n: f32) -> f32 {
    let df = def(id);
    let n = n.clamp(0.0, 1.0);
    match df.taper {
        Taper::Lin => df.lo + (df.hi - df.lo) * n,
        Taper::Log => df.lo * (df.hi / df.lo).powf(n),
        Taper::Sq => df.lo + (df.hi - df.lo) * n * n,
    }
}

fn signed(v: f32, decimals: usize, unit: &str) -> String {
    let s = format!("{:.*}", decimals, v.abs());
    let zero = s.chars().all(|c| c == '0' || c == '.');
    if zero {
        format!("{s}{unit}")
    } else if v < 0.0 {
        format!("−{s}{unit}")
    } else {
        format!("+{s}{unit}")
    }
}

fn fmt_time(s: f32) -> String {
    if s < 0.01 {
        format!("{:.1} ms", s * 1000.0)
    } else if s < 1.0 {
        format!("{:.0} ms", s * 1000.0)
    } else {
        format!("{s:.2} s")
    }
}

/// Readout text with units.
pub fn fmt(id: usize, v: f32) -> String {
    match def(id).fmt {
        Fmt::Pct => format!("{:.0}%", v * 100.0),
        Fmt::SignedPct => signed(v * 100.0, 0, "%"),
        Fmt::Cents => signed(v, 0, " ct"),
        Fmt::PlainCents => format!("{v:.0} ct"),
        Fmt::Hz if v >= 10_000.0 => format!("{:.1} kHz", v / 1000.0),
        Fmt::Hz if v >= 1000.0 => format!("{:.2} kHz", v / 1000.0),
        Fmt::Hz if v >= 100.0 => format!("{v:.0} Hz"),
        Fmt::Hz if v >= 1.0 => format!("{v:.1} Hz"),
        Fmt::Hz => format!("{v:.2} Hz"),
        Fmt::Time => fmt_time(v),
        Fmt::Glide if v <= 0.0 => "Off".into(),
        Fmt::Glide => fmt_time(v),
        Fmt::Oct => format!("{v:.2} oct"),
        Fmt::Db if v <= 1e-4 => "−∞ dB".into(),
        Fmt::Db => signed(20.0 * v.log10(), 1, " dB"),
        Fmt::Voices if v as u8 == 1 => "Mono".into(),
        Fmt::Voices => format!("{} voices", v as u8),
        Fmt::Unison => format!("{}×", v as u8),
    }
}

// ------------------------------------------------------------------ dropdowns

const WAVES: [Waveform; 4] = [Waveform::Saw, Waveform::Square, Waveform::Triangle, Waveform::Sine];
const MODES: [FilterMode; 3] = [FilterMode::LowPass, FilterMode::HighPass, FilterMode::BandPass];

pub fn wave_name(w: Waveform) -> &'static str {
    match w {
        Waveform::Saw => "Saw",
        Waveform::Square => "Square",
        Waveform::Triangle => "Triangle",
        Waveform::Sine => "Sine",
    }
}

fn mode_name(m: FilterMode) -> &'static str {
    match m {
        FilterMode::LowPass => "Low-pass",
        FilterMode::HighPass => "High-pass",
        FilterMode::BandPass => "Band-pass",
    }
}

fn oct_text(o: i32) -> String {
    if o == 0 { "0".into() } else { signed(o as f32, 0, "") }
}

fn semi_text(s: i32) -> String {
    signed(s as f32, 0, " st")
}

/// Options of a dropdown and the selected index.
pub fn choice_options(sp: &SynthParams, id: usize) -> (Vec<String>, usize) {
    let waves = || WAVES.iter().map(|w| wave_name(*w).to_string()).collect::<Vec<_>>();
    let pos = |w: Waveform| WAVES.iter().position(|x| *x == w).unwrap_or(0);
    match id {
        ch::OSC1_WAVE | ch::OSC2_WAVE => (waves(), pos(sp.osc[(id == ch::OSC2_WAVE) as usize].wave)),
        ch::OSC1_OCT | ch::OSC2_OCT => ((-3..=3).map(oct_text).collect(), (sp.osc[(id == ch::OSC2_OCT) as usize].octave + 3) as usize),
        ch::OSC1_SEMI | ch::OSC2_SEMI => ((-12..=12).map(semi_text).collect(), (sp.osc[(id == ch::OSC2_SEMI) as usize].semitones + 12) as usize),
        ch::FILTER_MODE => (MODES.iter().map(|m| mode_name(*m).to_string()).collect(), MODES.iter().position(|m| *m == sp.filter.mode).unwrap_or(0)),
        ch::LFO_WAVE => (waves(), pos(sp.lfo.wave)),
        _ => (Vec::new(), 0),
    }
}

pub fn set_choice(sp: &mut SynthParams, id: usize, index: usize) {
    let k = matches!(id, ch::OSC2_WAVE | ch::OSC2_OCT | ch::OSC2_SEMI) as usize;
    match id {
        ch::OSC1_WAVE | ch::OSC2_WAVE => sp.osc[k].wave = WAVES[index.min(3)],
        ch::OSC1_OCT | ch::OSC2_OCT => sp.osc[k].octave = (index.min(6) as i8) - 3,
        ch::OSC1_SEMI | ch::OSC2_SEMI => sp.osc[k].semitones = (index.min(24) as i8) - 12,
        ch::FILTER_MODE => sp.filter.mode = MODES[index.min(2)],
        ch::LFO_WAVE => sp.lfo.wave = WAVES[index.min(3)],
        _ => {}
    }
}

// ------------------------------------------------------------------ displays

fn saw(t: f32) -> f32 {
    2.0 * t - 1.0
}

/// Triangle aligned with the sine (peak at 0.25, trough at 0.75).
fn tri(t: f32) -> f32 {
    if t < 0.25 {
        4.0 * t
    } else if t < 0.75 {
        2.0 - 4.0 * t
    } else {
        4.0 * t - 4.0
    }
}

fn sin_turns(a: f32) -> f32 {
    (2.0 * PI * a).sin()
}

/// One oscillator sample for phase `ph` in [0, 1), mirroring the naive
/// (non-band-limited) shapes of the engine's synth (`engine/src/synth.cpp`,
/// `osc_sample` / `shape`): Saw and Triangle crossfade toward each other by
/// `shape`; Square is a pulse of width 50% → 5% (drawn without the engine's DC
/// correction so it stays inside the display); Sine wavefolds with a fold
/// gain of 5 × shape.
pub fn osc_sample(w: Waveform, shape: f32, ph: f32) -> f32 {
    let t = ph.rem_euclid(1.0);
    let sh = shape.clamp(0.0, 1.0);
    match w {
        Waveform::Saw => saw(t) + sh * (tri(t) - saw(t)),
        Waveform::Triangle => tri(t) + sh * (saw(t) - tri(t)),
        Waveform::Square => {
            if t < 0.5 - 0.45 * sh { 1.0 } else { -1.0 }
        }
        Waveform::Sine => {
            let fold = 5.0 * sh;
            let y = sin_turns(t);
            if fold <= 1e-4 {
                y
            } else {
                let norm = if fold < 1.0 { 1.0 / sin_turns(0.25 * fold) } else { 1.0 };
                sin_turns(0.25 * fold * y) * norm
            }
        }
    }
}

/// LFO shapes as in the engine (`lfo_value`): Saw rises through 0 at phase 0.
pub fn lfo_sample(w: Waveform, ph: f32) -> f32 {
    let p = ph.rem_euclid(1.0);
    match w {
        Waveform::Square => {
            if p < 0.5 { 1.0 } else { -1.0 }
        }
        Waveform::Triangle => tri(p),
        Waveform::Saw => {
            if p < 0.5 { 2.0 * p } else { 2.0 * p - 2.0 }
        }
        Waveform::Sine => sin_turns(p),
    }
}

fn path(points: &[(f32, f32)]) -> String {
    let mut s = String::with_capacity(points.len() * 13);
    for (i, (x, y)) in points.iter().enumerate() {
        s.push_str(if i == 0 { "M" } else { " L" });
        s.push_str(&format!("{x:.2} {y:.2}"));
    }
    if s.is_empty() { "M0 0".into() } else { s }
}

/// Line plus a fill closed to the horizontal line `base_y`.
fn line_and_fill(points: Vec<(f32, f32)>, base_y: f32) -> SynthDisplay {
    let line = path(&points);
    let mut fill = points.clone();
    if let (Some(first), Some(last)) = (points.first(), points.last()) {
        fill.push((last.0, base_y));
        fill.push((first.0, base_y));
    }
    let mut f = path(&fill);
    f.push_str(" Z");
    SynthDisplay { line: line.into(), fill: f.into() }
}

/// `cycles` periods of `f(phase)` (−1..1) at `amp` (fraction of the half height).
fn wave_display(cycles: f32, amp: f32, f: impl Fn(f32) -> f32) -> SynthDisplay {
    let (w, h) = (DISP_W, DISP_H);
    let mid = h / 2.0;
    let n = (w * 2.0) as usize;
    let pts = (0..=n)
        .map(|i| {
            let x = i as f32 / n as f32 * w;
            (x, mid - f(i as f32 / n as f32 * cycles) * amp * mid * 0.8)
        })
        .collect();
    line_and_fill(pts, mid)
}

/// Analog-prototype magnitude (dB) of the engine's 2-pole state-variable filter
/// at `f` Hz (Q = 0.707 · (25 / 0.707)^resonance as in `engine/src/synth.cpp`;
/// band-pass normalised to 0 dB at the cutoff). Key tracking, envelope and LFO
/// are not drawn (the curve is at C4 with everything at rest).
pub fn svf_db(mode: FilterMode, cutoff: f32, resonance: f32, f: f32) -> f32 {
    const BUTTERWORTH_Q: f32 = std::f32::consts::FRAC_1_SQRT_2;
    let q = BUTTERWORTH_Q * (25.0 / BUTTERWORTH_Q).powf(resonance.clamp(0.0, 1.0));
    let w = f / cutoff;
    let den = ((1.0 - w * w).powi(2) + (w / q).powi(2)).sqrt().max(1e-9);
    let num = match mode {
        FilterMode::LowPass => 1.0,
        FilterMode::HighPass => w * w,
        FilterMode::BandPass => w / q,
    };
    20.0 * (num / den).max(1e-9).log10()
}

/// Filter display dB range (top, bottom); x is log frequency 20 Hz .. 20 kHz.
pub const FILTER_DB: (f32, f32) = (-42.0, 24.0);

fn filter_display(sp: &SynthParams) -> SynthDisplay {
    let n = (FILTER_W * 1.5) as usize;
    let (lo, hi) = FILTER_DB;
    let pts = (0..=n)
        .map(|i| {
            let x = i as f32 / n as f32 * FILTER_W;
            let f = 20.0 * 1000.0f32.powf(x / FILTER_W);
            let db = svf_db(sp.filter.mode, sp.filter.cutoff_hz, sp.filter.resonance, f).clamp(lo - 2.0, hi);
            (x, (hi - db) / (hi - lo) * DISP_H)
        })
        .collect();
    line_and_fill(pts, DISP_H + 2.0)
}

/// Envelope display: segment widths grow logarithmically with time, the
/// sustain plateau has a fixed width.
fn env_display(e: &Adsr) -> SynthDisplay {
    let seg = |t: f32| DISP_W * 0.25 * (1.0 + t / 0.001).log10() / (20_001.0f32).log10();
    let (top, bot) = (DISP_H * 0.1, DISP_H * 0.94);
    let y = |v: f32| bot - v * (bot - top);
    let (wa, wd, wr) = (seg(e.attack), seg(e.decay), seg(e.release));
    let hold = DISP_W * 0.2;
    let mut pts = vec![(0.0, y(0.0))];
    let steps = 40;
    for i in 1..=steps {
        let u = i as f32 / steps as f32;
        pts.push((wa * u, y(u)));
    }
    let x0 = wa;
    for i in 1..=steps {
        let u = i as f32 / steps as f32;
        let v = e.sustain + (1.0 - e.sustain) * ((-5.0 * u).exp() - (-5.0f32).exp() * u);
        pts.push((x0 + wd * u, y(v)));
    }
    let x1 = x0 + wd + hold;
    pts.push((x1, y(e.sustain)));
    for i in 1..=steps {
        let u = i as f32 / steps as f32;
        let v = e.sustain * ((-5.0 * u).exp() - (-5.0f32).exp() * u);
        pts.push((x1 + wr * u, y(v)));
    }
    pts.push((DISP_W, y(0.0)));
    line_and_fill(pts, bot)
}

/// Deterministic white-noise picture (same every frame), scaled by the level.
fn noise_display(level: f32) -> SynthDisplay {
    let mut s: u32 = 0x9e37_79b9;
    let mid = DISP_H / 2.0;
    let n = (DISP_W / 1.5) as usize;
    let pts = (0..=n)
        .map(|i| {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            let r = (s as f32 / u32::MAX as f32) * 2.0 - 1.0;
            (i as f32 * 1.5, mid - r * level * mid * 0.8)
        })
        .collect();
    line_and_fill(pts, mid)
}

/// All display paths for a parameter set.
pub fn displays(sp: &SynthParams) -> Vec<SynthDisplay> {
    let mut v = vec![SynthDisplay::default(); disp::COUNT];
    for k in 0..2 {
        let o = sp.osc[k];
        // Each picture starts at a natural point of the cycle: the triangle at its trough,
        // the others at phase 0 (saw/pulse edge, sine zero crossing).
        let start = if o.wave == Waveform::Triangle { 0.75 } else { 0.0 };
        v[disp::OSC1 + k] = wave_display(2.0, o.level, |ph| osc_sample(o.wave, o.shape, ph + start));
    }
    v[disp::SUB] = wave_display(1.0, sp.sub_level, |ph| (2.0 * PI * ph).sin());
    v[disp::NOISE] = noise_display(sp.noise_level);
    v[disp::FILTER] = filter_display(sp);
    v[disp::AMP_ENV] = env_display(&sp.amp_env);
    v[disp::FLT_ENV] = env_display(&sp.filter_env);
    // LFO: one second of modulation.
    let l = sp.lfo;
    v[disp::LFO] = wave_display(l.rate_hz, 1.0, |ph| lfo_sample(l.wave, ph));
    v
}

// ------------------------------------------------------------------ controller glue

/// What a synth popup list is for.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MenuTarget {
    Presets,
    Choice(usize),
}

#[derive(Debug, Clone, Copy)]
pub struct SynthMenu {
    pub target: MenuTarget,
    /// Window position of the list's top-left corner and its width.
    pub pos: (f32, f32),
    pub w: f32,
}

#[derive(Debug, Default)]
pub struct SynthUi {
    pub page: usize,
    /// Parameters being dragged (previewed in the engine, not yet a command).
    pub pending: Option<(TrackId, SynthParams)>,
    pub menu: Option<SynthMenu>,
    /// Last values pushed to Slint (avoid rebuilding the paths every frame).
    pushed: Option<SynthParams>,
}

pub const MENU_ROW: f32 = 20.5;
pub const MENU_PAD: f32 = 4.0;

impl Controller {
    /// The instrument track whose synth the Devices tab shows.
    pub fn synth_track(&self) -> Option<TrackId> {
        let t = self.selected_track.and_then(|t| self.project().track(t))?;
        (t.kind == TrackKind::Instrument && self.bottom_tab == BottomTab::Devices).then_some(t.id)
    }

    /// Current parameters (the pending drag preview, else the model).
    pub fn synth_params(&self) -> Option<SynthParams> {
        let t = self.synth_track()?;
        if let Some((pt, sp)) = &self.synth.pending
            && *pt == t
        {
            return Some(sp.clone());
        }
        self.project().track(t).map(|t| t.synth.clone())
    }

    /// Execute one `SetSynthParams` as its own undo step (never coalesced
    /// with the previous entry, see the module docs).
    fn commit_synth(&mut self, track: TrackId, params: SynthParams) -> bool {
        if self.project().track(track).is_some_and(|t| t.synth == params) {
            return false;
        }
        self.doc.seal_coalescing();
        self.execute(Command::SetSynthParams { track, params })
    }

    pub fn synth_page(&mut self, page: usize) {
        self.synth.page = page.min(PAGES.len() - 1);
        self.synth.menu = None;
        self.ui_dirty = true;
    }

    /// Knob / fader drag (control position 0..1): preview only.
    pub fn synth_param(&mut self, id: usize, norm: f32) {
        let (Some(t), Some(mut sp)) = (self.synth_track(), self.synth_params()) else { return };
        if id >= p::COUNT {
            return;
        }
        set(&mut sp, id, from_norm(id, norm));
        if self.synth.pending.as_ref().is_some_and(|(_, cur)| *cur == sp) {
            return;
        }
        self.synth.pending = Some((t, sp));
        self.publish();
        self.ui_dirty = true;
    }

    /// Gesture end: the drag becomes ONE command.
    pub fn synth_param_end(&mut self) {
        if let Some((t, sp)) = self.synth.pending.take() {
            if !self.commit_synth(t, sp) {
                self.publish(); // drop the preview override
            }
            self.ui_dirty = true;
        }
    }

    /// Double-click: back to the `SynthParams::default()` value (one step).
    pub fn synth_param_reset(&mut self, id: usize) {
        self.synth.pending = None;
        let (Some(t), Some(mut sp)) = (self.synth_track(), self.synth_params()) else { return };
        set(&mut sp, id, get(&SynthParams::default(), id));
        self.commit_synth(t, sp);
    }

    pub fn synth_choice(&mut self, id: usize, index: usize) {
        let (Some(t), Some(mut sp)) = (self.synth_track(), self.synth_params()) else { return };
        set_choice(&mut sp, id, index);
        self.commit_synth(t, sp);
    }

    /// Index of the current preset in `factory_presets()` (by name).
    pub fn synth_preset_index(&self) -> Option<usize> {
        let name = self.synth_params()?.preset;
        factory_presets().iter().position(|p| p.preset == name)
    }

    /// Load a factory preset: one `SetSynthParams` with the preset's params.
    pub fn synth_load_preset(&mut self, index: usize) {
        let Some(t) = self.synth_track() else { return };
        let presets = factory_presets();
        let Some(p) = presets.get(index).cloned() else { return };
        self.synth.pending = None;
        self.synth.menu = None;
        self.commit_synth(t, p);
    }

    /// ‹ / › in the preset browser (wraps; unknown presets start at the first).
    pub fn synth_preset_step(&mut self, dir: i32) {
        let n = factory_presets().len() as i32;
        let next = match self.synth_preset_index() {
            Some(i) => (i as i32 + dir).rem_euclid(n),
            None => {
                if dir > 0 { 0 } else { n - 1 }
            }
        };
        self.synth_load_preset(next as usize);
    }

    pub fn synth_open_menu(&mut self, target: MenuTarget, x: f32, y: f32, w: f32) {
        self.synth.menu = Some(SynthMenu { target, pos: (x, y), w });
        self.ui_dirty = true;
    }

    pub fn synth_menu_pick(&mut self, index: usize) {
        let Some(m) = self.synth.menu.take() else { return };
        match m.target {
            MenuTarget::Presets => self.synth_load_preset(index),
            MenuTarget::Choice(id) => self.synth_choice(id, index),
        }
        self.ui_dirty = true;
    }

    pub fn synth_menu_dismiss(&mut self) {
        self.synth.menu = None;
        self.ui_dirty = true;
    }

    /// Popup items (labels) and the selected row for the open menu.
    pub fn synth_menu_items(&self) -> (Vec<String>, usize) {
        let (Some(m), Some(sp)) = (self.synth.menu, self.synth_params()) else { return (Vec::new(), 0) };
        match m.target {
            MenuTarget::Presets => {
                let ps = factory_presets();
                let sel = ps.iter().position(|p| p.preset == sp.preset).unwrap_or(usize::MAX);
                (ps.into_iter().map(|p| p.preset).collect(), sel)
            }
            MenuTarget::Choice(id) => choice_options(&sp, id),
        }
    }

    /// Synth device view models → Slint.
    pub fn push_synth(&mut self, s: &Studio) {
        let sp = self.synth_params();
        s.set_synth_visible(sp.is_some());
        s.set_synth_page(self.synth.page as i32);
        let Some(sp) = sp else {
            s.set_synth_menu_open(false);
            return;
        };
        s.set_synth_preset(sp.preset.clone().into());
        let params: Vec<SynthParam> = (0..p::COUNT)
            .map(|id| {
                let v = get(&sp, id);
                SynthParam { norm: to_norm(id, v), text: fmt(id, v).into(), bipolar: bipolar(id) }
            })
            .collect();
        sync_model(&self.synth_params_model, params);
        let choices: Vec<slint::SharedString> = (0..ch::COUNT)
            .map(|id| {
                let (opts, i) = choice_options(&sp, id);
                opts.get(i).cloned().unwrap_or_default().into()
            })
            .collect();
        sync_model(&self.synth_choices_model, choices);
        if self.synth.pushed.as_ref() != Some(&sp) {
            sync_model(&self.synth_displays_model, displays(&sp));
            self.synth.pushed = Some(sp);
        }
        // Popup list.
        let (items, sel) = self.synth_menu_items();
        s.set_synth_menu_open(self.synth.menu.is_some() && !items.is_empty());
        if let Some(m) = self.synth.menu {
            let rows: Vec<MenuItem> = items
                .iter()
                .enumerate()
                .map(|(i, l)| MenuItem {
                    label: l.clone().into(),
                    shortcut: if i == sel { "✓".into() } else { "".into() },
                    enabled: true,
                    separator: false,
                    submenu: false,
                    y: MENU_PAD + i as f32 * MENU_ROW,
                })
                .collect();
            s.set_synth_menu_h(MENU_PAD * 2.0 + rows.len() as f32 * MENU_ROW);
            sync_model(&self.synth_menu_model, rows);
            s.set_synth_menu_x(m.pos.0);
            s.set_synth_menu_y(m.pos.1);
            s.set_synth_menu_w(m.w.max(110.0));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn norm_round_trip_and_ranges() {
        let sp = SynthParams::default();
        for id in 0..p::COUNT {
            let v = get(&sp, id);
            let back = from_norm(id, to_norm(id, v));
            assert!((back - v).abs() <= v.abs() * 1e-3 + 1e-3, "{id}: {v} -> {back}");
            let mut q = sp.clone();
            set(&mut q, id, from_norm(id, 1.0));
            set(&mut q, id, from_norm(id, 0.0));
            q.validate().unwrap();
        }
        assert!((from_norm(p::CUTOFF, 0.5) - 632.5).abs() < 1.0, "log taper: geometric mean of 20 Hz..20 kHz");
    }

    #[test]
    fn readouts_have_units() {
        assert_eq!(fmt(p::OSC1_FINE, 7.0), "+7 ct");
        assert_eq!(fmt(p::OSC1_FINE, 0.0), "0 ct");
        assert_eq!(fmt(p::OSC1_FINE, -5.0), "−5 ct");
        assert_eq!(fmt(p::CUTOFF, 2093.0), "2.09 kHz");
        assert_eq!(fmt(p::CUTOFF, 600.0), "600 Hz");
        assert_eq!(fmt(p::AMP_A, 0.005), "5.0 ms");
        assert_eq!(fmt(p::AMP_D, 0.2), "200 ms");
        assert_eq!(fmt(p::AMP_R, 2.5), "2.50 s");
        assert_eq!(fmt(p::OSC1_LEVEL, 0.8), "80%");
        assert_eq!(fmt(p::OUTPUT, 1.0), "0.0 dB");
        assert_eq!(fmt(p::ENV_AMOUNT, -0.3), "−30%");
    }

    #[test]
    fn choices_round_trip() {
        let mut sp = SynthParams::default();
        for id in 0..ch::COUNT {
            let (opts, _) = choice_options(&sp, id);
            for i in 0..opts.len() {
                set_choice(&mut sp, id, i);
                assert_eq!(choice_options(&sp, id).1, i, "choice {id} index {i}");
                sp.validate().unwrap();
            }
        }
        set_choice(&mut sp, ch::OSC2_OCT, 4);
        assert_eq!(sp.osc[1].octave, 1);
        assert_eq!(choice_options(&sp, ch::OSC2_OCT).0[4], "+1");
    }

    #[test]
    fn shapes_differ_and_stay_in_range() {
        for w in WAVES {
            for s in [0.0, 0.5, 1.0] {
                for i in 0..100 {
                    let v = osc_sample(w, s, i as f32 / 100.0);
                    assert!((-1.0001..=1.0001).contains(&v), "{w:?} {s} {v}");
                }
            }
        }
        let mut sp = SynthParams::default();
        let saw = displays(&sp)[disp::OSC1].line.clone();
        sp.osc[0].wave = Waveform::Square;
        assert_ne!(displays(&sp)[disp::OSC1].line, saw);
        // Pulse width shrinks with shape.
        assert_eq!(osc_sample(Waveform::Square, 1.0, 0.1), -1.0);
        assert_eq!(osc_sample(Waveform::Square, 0.0, 0.1), 1.0);
    }

    #[test]
    fn filter_response_shape() {
        let lp = |f| svf_db(FilterMode::LowPass, 1000.0, 0.0, f);
        assert!(lp(50.0).abs() < 0.5 && lp(10_000.0) < -35.0);
        let hp = |f| svf_db(FilterMode::HighPass, 1000.0, 0.0, f);
        assert!(hp(50.0) < -45.0 && hp(10_000.0).abs() < 0.5);
        assert!(svf_db(FilterMode::LowPass, 1000.0, 1.0, 1000.0) > 18.0, "resonance peak");
    }
}
