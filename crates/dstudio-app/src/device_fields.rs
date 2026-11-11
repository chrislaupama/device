//! Curve models of the built-in devices' plotted fields (dstudio_core::builtin::View::Field).
//!
//! Each model turns the device's parameter values into what the field draws:
//! tick labels, a rest line, up to three traces, a fill or a min..max band,
//! the caption value and the live LFO overlay. Coordinates are normalised:
//! x runs 0..1 across the plot, y runs 0 (top) to 1 (bottom); the panel code
//! (device_panel.rs) scales them to the well and builds the paths.
//!
//! The tremolo, auto-panner, chorus, flanger, phaser and ring-mod models
//! follow each device's drawing law.
//! Character Mod's model is a picture of what the kernel does
//! (engine/src/devices/character_mod.cpp): the modulated delay, frequency,
//! gain or position over 3.35 LFO cycles (the span at the default 0.51 Hz).

use dstudio_core::builtin::character_mod::{CIRCUITS, CROSSED, ENSEMBLE, FLANGER, MODE_PARAMS, PANNER, PHASER, PHOTOCELL, ROTARY, SINGLE, TAPE, TREMOLO, TWIN};
use dstudio_core::builtin::{synced_hz, DIVISION_QUARTERS};
use std::f32::consts::{PI, TAU};
use std::sync::OnceLock;

const MINUS: char = '\u{2212}';

/// Line style of a trace (dash lengths in pt at 100%).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Style {
    Solid,
    Dash { on: f32, off: f32, alpha: f32 },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Trace {
    /// y per sample, evenly spaced across the plot.
    pub ys: Vec<f32>,
    pub style: Style,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Fill {
    None,
    /// Track colour between the first trace and the rest line.
    ToRest,
    /// Track colour band between the lowest and highest point of all traces.
    Band,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Live {
    /// A moving dot on the first trace, with an optional trailing trace.
    Dot { x: f32, y: f32, trail: Vec<(f32, f32)> },
    /// The response at the LFO's current position.
    Curve(Vec<f32>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Plot {
    pub readout: String,
    /// (y, label); an empty label draws the rule only.
    pub ticks: Vec<(f32, String)>,
    pub rest: Option<f32>,
    pub traces: Vec<Trace>,
    pub fill: Fill,
    /// Width of the first trace at rest and while the field is hot (pt).
    pub widths: (f32, f32),
    pub live: Option<Live>,
}

pub struct Input<'a> {
    /// Plain value of a parameter (0 when the device has no such parameter).
    pub get: &'a dyn Fn(&str) -> f32,
    pub bpm: f32,
    /// Samples across the plot (at least 64).
    pub n: usize,
    /// Elapsed LFO cycles while the transport plays (its fraction is the
    /// engine's LFO phase); None draws no live overlay.
    pub cycles: Option<f64>,
}

impl Input<'_> {
    fn get(&self, id: &str) -> f32 {
        (self.get)(id)
    }
    fn on(&self, id: &str) -> bool {
        self.get(id) >= 0.5
    }
    fn idx(&self, id: &str) -> usize {
        self.get(id).round().max(0.0) as usize
    }
    /// LFO rate in Hz: Rate, or the synced note value at the song tempo.
    fn lfo_hz(&self, rate: &str, note: &str, table: &[f32]) -> f32 {
        if self.on("sync") {
            synced_hz(table[self.idx(note).min(table.len() - 1)], self.bpm)
        } else {
            self.get(rate)
        }
    }
    fn xs(&self) -> impl Iterator<Item = f32> + '_ {
        let n = self.n.max(2);
        (0..n).map(move |k| k as f32 / (n - 1) as f32)
    }
}

/// The plot of a field model by name (Field::plot).
pub fn plot(name: &str, i: &Input) -> Option<Plot> {
    Some(match name {
        "tremolo" => tremolo(i),
        "autopanner" => autopanner(i),
        "chorus" | "flanger" | "phaser" => response(name, i),
        "ring-mod" => ring_mod(i),
        "character-mod" => character_mod(i),
        _ => return None,
    })
}

/// y of v on a linear axis from top to bottom.
fn lin(v: f32, top: f32, bottom: f32) -> f32 {
    ((top - v) / (top - bottom)).clamp(-0.1, 1.1)
}

fn db(g: f32) -> f32 {
    20.0 * g.max(1e-7).log10()
}

/// Value of ys (evenly spaced over 0..1) at x, linearly interpolated.
fn sample(ys: &[f32], x: f32) -> f32 {
    let f = x.clamp(0.0, 1.0) * (ys.len() - 1) as f32;
    let k = (f as usize).min(ys.len() - 2);
    ys[k] + (ys[k + 1] - ys[k]) * (f - k as f32)
}

/// Cycles a motion field shows at an LFO rate (more at faster rates).
fn motion_span(hz: f32) -> f32 {
    3.2239 * hz.max(0.01).powf(0.20735)
}

/// Live dot of a motion field spanning span cycles.
fn motion_dot(i: &Input, span: f32, ys: &[f32]) -> Option<Live> {
    let c = i.cycles?;
    let x = (c.rem_euclid(span as f64) as f32) / span;
    Some(Live::Dot { x, y: sample(ys, x), trail: Vec::new() })
}

/// Small deterministic generator (display only).
struct Lcg(u32);
impl Lcg {
    fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (self.0 >> 8) as f32 / (1u32 << 23) as f32 - 1.0
    }
}

// ---------------------------------------------------------------- tremolo

fn trem_shape(shape: usize, p: f32) -> f32 {
    let f = p.rem_euclid(1.0);
    match shape {
        1 => 1.0 - 2.0 * (f - 0.5).abs(),
        2 if f < 0.48 => 1.0,
        2 if f < 0.52 => 0.5 + 0.5 * (PI * (f - 0.48) / 0.04).cos(),
        2 => 0.0,
        3 => 1.0 - f,
        _ => 0.5 + 0.5 * (TAU * p).sin(),
    }
}

fn tremolo(i: &Input) -> Plot {
    let (depth, mix, shape) = (i.get("depth") / 100.0, i.get("mix") / 100.0, i.idx("shape"));
    let stereo = i.get("stereo_phase") / 180.0;
    let span = motion_span(i.lfo_hz("rate", "rate_sync", DIVISION_QUARTERS));
    let curve = |off: f32| -> Vec<f32> {
        i.xs().map(|x| lin(db(1.0 - mix + mix * (1.0 - depth * (1.0 - trem_shape(shape, x * span + off)))), 0.0, -14.0)).collect()
    };
    let main = curve(0.0);
    let live = motion_dot(i, span, &main);
    let mut traces = vec![Trace { ys: main, style: Style::Solid }];
    if stereo > 0.0005 {
        traces.push(Trace { ys: curve(stereo * 0.5), style: Style::Dash { on: 4.0, off: 3.0, alpha: 0.5 } });
    }
    let att = -db(1.0 - depth * mix);
    Plot {
        readout: if att >= 90.0 { "\u{221e} dB".into() } else { format!("{att:.1} dB") },
        ticks: vec![(0.0, "0".into()), (0.5, format!("{MINUS}7")), (1.0, format!("{MINUS}14"))],
        rest: Some(0.0),
        traces,
        fill: Fill::ToRest,
        widths: (1.6, 2.35),
        live,
    }
}

// ---------------------------------------------------------------- auto-panner

fn autopanner(i: &Input) -> Plot {
    let (depth, mix, shape) = (i.get("depth") / 100.0, i.get("mix") / 100.0, i.idx("shape"));
    let phase = i.get("phase") / 360.0;
    let hz = i.lfo_hz("rate", "rate_sync", DIVISION_QUARTERS).max(0.01);
    let span = motion_span(hz);
    let n = i.n.max(2);
    let dc = span / (n - 1) as f32;
    // 5 ms glide, in plot samples.
    let a = 1.0 - (-(dc / hz) / 0.005).exp();
    let raw = |p: f32| match shape {
        1 => 1.0 - 4.0 * ((p + 0.25).rem_euclid(1.0) - 0.5).abs(),
        2 => {
            if p.rem_euclid(1.0) < 0.5 { -1.0 } else { 1.0 }
        }
        3 => Lcg((p.floor() as i32 as u32).wrapping_mul(2_654_435_761) ^ 0x5eed).next(),
        _ => (TAU * p).sin(),
    };
    // Two lead-in cycles settle the glide before the visible window.
    let lead = (2.0 / dc).ceil() as usize;
    let mut v = raw(phase - lead as f32 * dc) * depth;
    let mut ys = Vec::with_capacity(n);
    for k in 0..lead + n {
        v += a * (raw(phase + (k as f32 - lead as f32) * dc) * depth - v);
        if k >= lead {
            let pos = (0.5 + 0.5 * v) * PI / 2.0;
            let (l, r) = ((1.0 - mix) + pos.cos() * std::f32::consts::SQRT_2 * mix, (1.0 - mix) + pos.sin() * std::f32::consts::SQRT_2 * mix);
            ys.push(lin(r.atan2(l) / (PI / 2.0) * 2.0 - 1.0, -1.0, 1.0));
        }
    }
    let live = motion_dot(i, span, &ys);
    Plot {
        readout: String::new(),
        ticks: vec![(0.0, "L".into()), (0.25, String::new()), (0.5, "C".into()), (0.75, String::new()), (1.0, "R".into())],
        rest: Some(0.5),
        traces: vec![Trace { ys, style: Style::Solid }],
        fill: Fill::ToRest,
        widths: (1.6, 2.35),
        live,
    }
}

// ---------------------------------------------------------------- comb / all-pass responses

#[derive(Debug, Clone, Copy)]
struct C(f32, f32);
impl C {
    fn expj(t: f32) -> C {
        C(t.cos(), t.sin())
    }
    fn add(self, o: C) -> C {
        C(self.0 + o.0, self.1 + o.1)
    }
    fn mul(self, o: C) -> C {
        C(self.0 * o.0 - self.1 * o.1, self.0 * o.1 + self.1 * o.0)
    }
    fn scale(self, k: f32) -> C {
        C(self.0 * k, self.1 * k)
    }
    fn div(self, o: C) -> C {
        let d = (o.0 * o.0 + o.1 * o.1).max(1e-30);
        C((self.0 * o.0 + self.1 * o.1) / d, (self.1 * o.0 - self.0 * o.1) / d)
    }
    fn abs(self) -> f32 {
        self.0.hypot(self.1)
    }
}

/// Equal-power style dry / wet gains with a correlation term.
fn mix_gains(m: f32, corr: f32) -> (f32, f32) {
    let d = 1.0 - m;
    let n = (d * d + m * m + 2.0 * m * d * corr).max(1e-9).sqrt();
    (d / n, m / n)
}

/// "First notch" number: 12k, 1.2k or 850.
fn hz_text(f: f32) -> String {
    if f >= 10_000.0 {
        format!("{}k", (f / 1000.0).round())
    } else if f >= 1000.0 {
        format!("{:.1}k", f / 1000.0)
    } else {
        format!("{}", f.round())
    }
}

fn hz_range(a: Option<f32>, b: Option<f32>) -> String {
    match (a, b) {
        (Some(a), Some(b)) if (a - b).abs() >= 1.0 => format!("{}\u{2013}{} Hz", hz_text(a.min(b)), hz_text(a.max(b))),
        (Some(f), _) | (None, Some(f)) => format!("{} Hz", hz_text(f)),
        (None, None) => "\u{2014}".into(),
    }
}

/// Chorus, flanger and phaser: the response at the two sweep extremes, the
/// right channel and the live LFO position.
fn response(name: &str, i: &Input) -> Plot {
    let (m, width, out_db) = (i.get("mix") / 100.0, i.get("width") / 100.0, i.get("output_gain"));
    let fb_raw = i.get("feedback") / 100.0;
    let n = i.n.max(2);
    // (snapshot phase, sweep phase, right phase, response at (phase, f), x -> f, notch at phase)
    type Resp<'a> = Box<dyn Fn(f32, f32) -> f32 + 'a>;
    let (snap, sweep, right, resp, freq, notch): (f32, f32, f32, Resp, Box<dyn Fn(f32) -> f32>, Box<dyn Fn(f32) -> Option<f32>>) = match name {
        "chorus" => {
            let v = i.idx("voices").min(3) + 1;
            let vf = v as f32;
            let depth = i.get("depth") / 1000.0;
            let shape = |x: f32| ((TAU * x).sin() - 0.063 * (3.0 * TAU * x).sin()) / 1.063;
            let delay = move |p: f32, k: usize| 0.0013 + depth * (shape(p + k as f32 / vf) * 0.5 + 0.5);
            let (dry, wet) = mix_gains(m, 0.0);
            let wet = wet * vf.powf(0.35);
            let span = (8.0 / (0.0013 + depth)).clamp(200.0, 20_000.0);
            let fb = fb_raw.clamp(0.0, 0.9);
            (
                0.25,
                0.25 + 0.5 / vf,
                0.25 + width * 0.5 / vf,
                Box::new(move |p, f| {
                    let mut h = C(0.0, 0.0);
                    for k in 0..v {
                        h = h.add(C::expj(-TAU * f * delay(p, k)));
                    }
                    let hp = C(0.0, f / 35.0).div(C(1.0, f / 35.0));
                    let lp = C(1.0, 0.0).div(C(1.0, f / 4700.0));
                    let hb = h.scale(1.0 / vf).mul(hp).mul(lp);
                    let hc = hb.div(C(1.0, 0.0).add(hb.scale(-fb)));
                    C(dry, 0.0).add(hc.scale(wet)).abs()
                }),
                Box::new(move |x| x * span),
                Box::new(move |p| {
                    let d = if v > 1 { (delay(p, 1) - delay(p, 0)).abs() } else { delay(p, 0) };
                    (d > 1e-6).then(|| 0.5 / d)
                }),
            )
        }
        "flanger" => {
            let mode = i.idx("mode").min(2);
            let depth = i.get("depth") / 1000.0 * if mode == 1 { 0.6 } else { 1.0 };
            let centre = 0.001 + depth;
            let delay = move |p: f32| if mode == 2 { centre + depth * (TAU * p).sin() } else { 0.001 + depth * ((TAU * p).sin() * 0.5 + 0.5) };
            let fb = match mode {
                0 => fb_raw,
                1 => (fb_raw * 1.2).clamp(-0.85, 0.85),
                _ => fb_raw.clamp(-0.85, 0.85),
            };
            let (dry, wet) = mix_gains(m, -0.15);
            let wet = wet * if mode == 2 { 1.318 } else { 1.0 };
            let max = if mode == 2 { centre + depth } else { 0.001 + depth };
            let span = (8.0 / max).clamp(200.0, 20_000.0);
            (
                0.25,
                if mode == 2 { 0.5 } else { 0.75 },
                0.25 + width * 0.5,
                Box::new(move |p, f| {
                    let e = C::expj(-TAU * f * delay(p));
                    let h = e.div(C(1.0, 0.0).add(e.scale(-fb)));
                    let d = if mode == 2 { C::expj(-TAU * f * centre).scale(dry) } else { C(dry, 0.0) };
                    d.add(h.scale(wet)).abs()
                }),
                Box::new(move |x| x * span),
                Box::new(move |p| {
                    let d = if mode == 2 { (delay(p) - centre).abs() } else { delay(p) };
                    (d > 1e-6).then(|| 0.5 / d)
                }),
            )
        }
        _ => {
            let stages = [2, 4, 6, 12][i.idx("stages").min(3)];
            let depth = i.get("depth") / 100.0;
            let fs = 48_000.0f32;
            let (lc, half) = ((200.0f32 * 8000.0).sqrt().ln(), 0.5 * (8000.0f32 / 200.0).ln() * depth);
            let fsweep = move |p: f32| (lc + half * (TAU * p).sin()).exp().min(0.45 * fs);
            let fb = fb_raw.clamp(-0.95, 0.95);
            let (dry, wet) = mix_gains(m, 0.2);
            (
                0.25,
                0.75,
                0.25 + width * 0.5,
                Box::new(move |p, f| {
                    let t = (PI * fsweep(p) / fs).tan();
                    let a = (t - 1.0) / (t + 1.0);
                    let z1 = C::expj(-TAU * f / fs);
                    let ap = C(a, 0.0).add(z1).div(C(1.0, 0.0).add(z1.scale(a)));
                    let mut g = C(1.0, 0.0);
                    for _ in 0..stages {
                        g = g.mul(ap);
                    }
                    let h = g.div(C(1.0, 0.0).add(z1.mul(g).scale(-fb)));
                    C(dry, 0.0).add(h.scale(wet)).abs()
                }),
                Box::new(|x| 20.0 * 1000f32.powf(x)),
                Box::new(move |p| Some(fs / PI * ((PI * fsweep(p) / fs).tan() * (PI / (2.0 * stages as f32)).tan()).atan())),
            )
        }
    };
    let curve = |p: f32| -> Vec<f32> { (0..n).map(|k| lin(db(resp(p, freq(k as f32 / (n - 1) as f32))) + out_db, 12.0, -24.0)).collect() };
    let mut traces = vec![Trace { ys: curve(snap), style: Style::Solid }, Trace { ys: curve(sweep), style: Style::Dash { on: 2.0, off: 3.0, alpha: 0.34 } }];
    if width > 0.0005 {
        traces.push(Trace { ys: curve(right), style: Style::Dash { on: 5.0, off: 4.0, alpha: 0.5 } });
    }
    Plot {
        readout: hz_range(notch(snap), notch(sweep)),
        ticks: vec![(0.0, "+12".into()), (1.0 / 3.0, "0".into()), (2.0 / 3.0, format!("{MINUS}12")), (1.0, format!("{MINUS}24"))],
        rest: Some(1.0 / 3.0),
        traces,
        fill: Fill::None,
        widths: (1.5, 2.25),
        live: i.cycles.map(|c| Live::Curve(curve(c.rem_euclid(1.0) as f32))),
    }
}

// ---------------------------------------------------------------- ring mod

/// Smoothed-noise tables (one per channel): two one-pole low-passes, unit
/// RMS x 1.33, tanh; played at 16 steps per carrier cycle.
fn noise_tables() -> &'static [[f32; 256]; 2] {
    static T: OnceLock<[[f32; 256]; 2]> = OnceLock::new();
    T.get_or_init(|| {
        let k = 1.0 - (-TAU / 16.0).exp();
        let mut out = [[0.0; 256]; 2];
        for (c, t) in out.iter_mut().enumerate() {
            let mut g = Lcg(0x2a17_93c1 ^ (c as u32 * 0x9e37_79b9));
            let (mut a, mut b) = (0.0f32, 0.0f32);
            // Settle the filters, then fill.
            for k2 in 0..256 + 64 {
                a += k * (g.next() - a);
                b += k * (a - b);
                if k2 >= 64 {
                    t[k2 - 64] = b;
                }
            }
            let rms = (t.iter().map(|v| v * v).sum::<f32>() / 256.0).sqrt().max(1e-6);
            for v in t.iter_mut() {
                *v = (*v / rms * 1.33).tanh();
            }
        }
        out
    })
}

fn random_tables() -> &'static [[f32; 64]; 2] {
    static T: OnceLock<[[f32; 64]; 2]> = OnceLock::new();
    T.get_or_init(|| {
        let mut out = [[0.0; 64]; 2];
        for (c, t) in out.iter_mut().enumerate() {
            let mut g = Lcg(0x51a3_0c07 + c as u32 * 7919);
            for v in t.iter_mut() {
                *v = g.next();
            }
        }
        out
    })
}

fn ring_wave(m: usize, p: f32, ch: usize) -> f32 {
    let f = p.rem_euclid(1.0);
    match m {
        1 => 1.0 - 4.0 * ((p + 0.25).rem_euclid(1.0) - 0.5).abs(),
        2 => {
            if f < 0.5 { 1.0 } else { -1.0 }
        }
        3 => 2.0 * f - 1.0,
        4 => {
            let t = &noise_tables()[ch];
            let pos = p.rem_euclid(16.0) * 16.0;
            let k = pos.floor() as isize;
            let u = pos - k as f32;
            let at = |j: isize| t[(k + j).rem_euclid(256) as usize];
            let (p0, p1, p2, p3) = (at(-1), at(0), at(1), at(2));
            0.5 * (2.0 * p1 + (p2 - p0) * u + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * u * u + (3.0 * p1 - p0 - 3.0 * p2 + p3) * u * u * u)
        }
        5 => random_tables()[ch][(p.floor() as i64).rem_euclid(64) as usize],
        _ => (TAU * p).sin(),
    }
}

fn ring_mod(i: &Input) -> Plot {
    let f = i.get("frequency").clamp(1.0, 20_000.0);
    let cycles = 2.0 * f.powf(4f32.ln() / 20_000f32.ln());
    let m = i.idx("modulator").min(5);
    let (r, b, s) = (i.get("rectify") / 100.0, i.get("bias") / 100.0, i.get("spread") / 100.0);
    let n = i.n.max(2) * if m == 0 || m == 4 { 1 } else { 4 };
    let curve = |ch: usize, factor: f32| -> Vec<f32> {
        (0..n)
            .map(|k| {
                let w = ring_wave(m, k as f32 / (n - 1) as f32 * cycles * factor, ch);
                let w = if r >= 0.0 { w + r * (w.abs() - w) } else { w - r * (-w.abs() - w) };
                lin((w + b) / (1.0 + b), 1.0, -1.0)
            })
            .collect()
    };
    let mut traces = vec![Trace { ys: curve(0, 2f32.powf(-s / 12.0)), style: Style::Solid }];
    if m >= 4 || s > 0.0005 {
        traces.push(Trace { ys: curve(1, 2f32.powf(s / 12.0)), style: Style::Dash { on: 4.0, off: 3.0, alpha: 0.5 } });
    }
    Plot {
        readout: String::new(),
        ticks: vec![(0.0, "1".into()), (0.5, "0".into()), (1.0, format!("{MINUS}1"))],
        rest: Some(0.5),
        traces,
        fill: Fill::ToRest,
        widths: (1.6, 2.35),
        live: None,
    }
}

// ---------------------------------------------------------------- character mod

enum Unit {
    Ms,
    Hz,
    Db,
    Pos,
}

/// Crossed modes: centre ms, swing ms, second-LFO ratio (engine kCrossedModes).
const CROSSED_MODES: [(f32, f32, f32); 4] = [(5.0, 1.8, 0.0), (7.5, 2.6, 0.0), (6.0, 2.2, 1.37), (9.0, 2.8, 0.0)];

/// Top of a delay axis: the next whole millisecond (even, so the middle tick is whole too).
fn axis_top(v: f32) -> f32 {
    let t = v.max(1.0).ceil();
    t + t % 2.0
}

fn num(v: f32) -> String {
    if (v - v.round()).abs() < 0.05 { format!("{}", v.round()) } else { format!("{v:.1}") }
}

fn character_mod(i: &Input) -> Plot {
    const SPAN: f32 = 3.35;
    let circuit = i.idx("model").min(CIRCUITS.len() - 1) as u32;
    let mode = MODE_PARAMS[circuit as usize].map_or(0, |p| i.idx(p));
    let amt = i.get("depth") / 50.0;
    let w = i.get("width") / 100.0;
    let triangle = circuit <= TWIN || circuit == FLANGER;
    let u = move |p: f32| if triangle { 1.0 - 4.0 * ((p + 0.25).rem_euclid(1.0) - 0.5).abs() } else { (TAU * p).sin() };
    let dash = Style::Dash { on: 4.0, off: 3.0, alpha: 0.55 };
    let dot = Style::Dash { on: 1.5, off: 2.5, alpha: 0.55 };
    // Series as functions of the LFO position (cycles).
    let mut series: Vec<(Box<dyn Fn(f32) -> f32>, Style)> = Vec::new();
    let unit = match circuit {
        SINGLE | TWIN | FLANGER | CROSSED => {
            let (c, s, lfo2): (f32, f32, f32) = match circuit {
                SINGLE if mode == 1 => (4.0, 2.2, 0.0),
                SINGLE => (6.5, 3.0, 0.0),
                TWIN => (3.55, if mode == 2 { 0.35 } else { 1.85 }, 0.0),
                CROSSED => CROSSED_MODES[(mode as usize).min(3)],
                _ => (0.0, 0.0, 0.0),
            };
            let roff = if circuit == FLANGER { 0.25 * w } else { 0.5 * w };
            for (ch, off) in [(0usize, 0.0f32), (1, roff)] {
                let f: Box<dyn Fn(f32) -> f32> = if circuit == FLANGER {
                    let (base, span) = match mode {
                        1 => (0.35, 3.5),
                        2 => (0.5, 3.0),
                        _ => (0.6, 2.2),
                    };
                    Box::new(move |p| base + span * amt.min(2.0) * (0.5 + 0.5 * u(p + off)))
                } else if lfo2 > 0.0 {
                    let k = if ch == 0 { 0.25 } else { 0.75 };
                    Box::new(move |p| c + s * amt * (0.7 * u(p + off) + 0.3 * (TAU * (p * lfo2 + k)).sin()))
                } else {
                    Box::new(move |p| c + s * amt * u(p + off))
                };
                series.push((f, if ch == 0 { Style::Solid } else { dash }));
            }
            Unit::Ms
        }
        ENSEMBLE => {
            for v in 0..3 {
                let off = v as f32 / 3.0;
                series.push((Box::new(move |p| 6.0 + 1.6 * amt * (0.8 * u(p + off) + 0.2 * (TAU * (p * 10.3 + off)).sin())), [Style::Solid, dash, dot][v]));
            }
            Unit::Ms
        }
        TAPE => {
            if mode == 0 {
                series.push((Box::new(move |p| 18.0 + 1.5 * amt * u(p)), Style::Solid));
                series.push((Box::new(move |p| 18.0 + 2.0 * w + 1.5 * amt * u(p + 0.5 * w)), dash));
            } else {
                let c = 1.0 + 2.0 * amt.min(2.0);
                let s = if mode == 1 { 2.0 } else { 0.25 };
                series.push((Box::new(move |p| (c + s * amt * u(p)).max(0.0)), Style::Solid));
                series.push((Box::new(move |p| (c + s * amt * u(p + 0.5 * w)).max(0.0)), dash));
                // The dry path's fixed delay the copy sweeps through.
                series.push((Box::new(move |_| c), dot));
            }
            Unit::Ms
        }
        PHOTOCELL => {
            for (k, off) in [(0usize, 0.0f32), (1, 0.25 * w)] {
                series.push((
                    Box::new(move |p| 90.0 * 25f32.powf((0.5 + 0.5 * u(p + off) * amt).clamp(0.0, 1.0))),
                    if k == 0 { Style::Solid } else { dash },
                ));
            }
            Unit::Hz
        }
        PHASER if mode == 4 => {
            for k in 0..3 {
                series.push((Box::new(move |p| 80.0 * (6000.0f32 / 80.0).powf((p + k as f32 / 3.0).rem_euclid(1.0))), [Style::Solid, dash, dot][k]));
            }
            Unit::Hz
        }
        PHASER => {
            let (lc, h) = (0.5 * (180.0f32 * 4200.0).ln(), 0.5 * (4200.0f32 / 180.0).ln() * amt.min(2.0));
            let roff = if mode == 3 { 0.25 } else { 0.5 * w };
            series.push((Box::new(move |p| (lc + h * u(p)).exp()), Style::Solid));
            series.push((Box::new(move |p| (lc + h * u(p + roff)).exp()), dash));
            Unit::Hz
        }
        TREMOLO => {
            let d = (0.6 * amt).clamp(0.0, 1.0);
            if mode == 2 {
                series.push((Box::new(move |p| db(1.0 - d * (0.5 - 0.5 * u(p)))), Style::Solid));
                series.push((Box::new(move |p| db(1.0 - d * (0.5 + 0.5 * u(p)))), dash));
            } else {
                let shape = move |x: f32| if mode == 1 { (2.5 * x).tanh() / 2.5f32.tanh() } else { x };
                series.push((Box::new(move |p| db(1.0 - d * (0.5 - 0.5 * shape(u(p))))), Style::Solid));
                series.push((Box::new(move |p| db(1.0 - d * (0.5 - 0.5 * shape(u(p + 0.5 * w))))), dash));
            }
            Unit::Db
        }
        ROTARY => {
            let k = if mode == 2 { 0.0 } else { (0.5 * amt).clamp(0.0, 1.0) };
            series.push((Box::new(move |p| k * (TAU * p).sin()), Style::Solid));
            series.push((Box::new(move |p| k * (TAU * p * 0.85).sin()), dash));
            Unit::Pos
        }
        PANNER => {
            let k = (0.5 * amt).clamp(0.0, 1.0) * w;
            series.push((Box::new(move |p| k * u(p)), Style::Solid));
            Unit::Pos
        }
        _ => Unit::Pos,
    };
    let n = i.n.max(2);
    let values: Vec<Vec<f32>> = series.iter().map(|(f, _)| (0..n).map(|k| f(k as f32 / (n - 1) as f32 * SPAN)).collect()).collect();
    let (lo, hi) = values.iter().flatten().fold((f32::MAX, f32::MIN), |(a, b), v| (a.min(*v), b.max(*v)));
    // Axis, ticks, rest line and caption per unit; y_of maps a value to 0..1.
    let (y_of, ticks, rest, readout, band): (Box<dyn Fn(f32) -> f32>, Vec<(f32, String)>, Option<f32>, String, bool) = match unit {
        Unit::Ms => {
            let top = axis_top(hi);
            (
                Box::new(move |v| lin(v, top, 0.0)),
                vec![(0.0, num(top)), (0.5, num(top / 2.0)), (1.0, "0".into())],
                None,
                if hi - lo < 0.05 { format!("{} ms", num(lo)) } else { format!("{:.1}\u{2009}\u{2013}\u{2009}{:.1} ms", lo, hi) },
                true,
            )
        }
        Unit::Hz => {
            let (a, b) = (30f32.ln(), 20_000f32.ln());
            (
                Box::new(move |v| lin(v.max(1.0).ln(), b, a)),
                [10_000.0, 1000.0, 100.0].iter().map(|f: &f32| ((b - f.ln()) / (b - a), hz_text(*f))).collect(),
                None,
                if hi - lo < 1.0 { format!("{} Hz", hz_text(lo)) } else { format!("{}\u{2009}\u{2013}\u{2009}{} Hz", hz_text(lo), hz_text(hi)) },
                true,
            )
        }
        Unit::Db => (
            Box::new(|v| lin(v, 0.0, -24.0)),
            vec![(0.0, "0".into()), (0.5, format!("{MINUS}12")), (1.0, format!("{MINUS}24"))],
            Some(0.0),
            if lo <= -90.0 { "\u{221e} dB".into() } else { format!("{:.1} dB", -lo) },
            true,
        ),
        Unit::Pos => (
            Box::new(|v| lin(v, -1.0, 1.0)),
            vec![(0.0, "L".into()), (0.5, "C".into()), (1.0, "R".into())],
            Some(0.5),
            format!("\u{00b1}{}%", (lo.abs().max(hi.abs()) * 100.0).round()),
            false,
        ),
    };
    let traces: Vec<Trace> = values.iter().zip(&series).map(|(v, (_, style))| Trace { ys: v.iter().map(|x| y_of(*x)).collect(), style: *style }).collect();
    let live = i.cycles.zip(series.first()).map(|(c, (f, _))| {
        let p = c.rem_euclid(SPAN.floor() as f64) as f32;
        let x = p / SPAN;
        let t0 = (x - 0.12).max(0.0);
        let trail = (0..=24).map(|k| t0 + (x - t0) * k as f32 / 24.0).map(|tx| (tx, y_of(f(tx * SPAN)))).collect();
        Live::Dot { x, y: y_of(f(p)), trail }
    });
    Plot { readout, ticks, rest, traces, fill: if band { Fill::Band } else { Fill::None }, widths: (1.6, 2.35), live }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dstudio_core::builtin;

    fn plot_of(id: &str, set: &[(&str, f32)], cycles: Option<f64>) -> Plot {
        let def = builtin::find(id).unwrap();
        let mut d = def.instantiate();
        for (p, v) in set {
            d.params.insert(p.to_string(), *v);
        }
        let get = |p: &str| def.param(p).map_or(0.0, |q| def.value(&d, q));
        let name = match def.layout.view {
            Some(builtin::View::Field(f)) => f.plot,
            _ => panic!("{id} has no field"),
        };
        plot(name, &Input { get: &get, bpm: 120.0, n: 300, cycles }).unwrap()
    }

    #[test]
    fn every_field_device_has_a_model() {
        for def in builtin::all() {
            if let Some(builtin::View::Field(f)) = def.layout.view {
                let get = |p: &str| def.param(p).map_or(0.0, |q| q.default);
                let p = plot(f.plot, &Input { get: &get, bpm: 120.0, n: 64, cycles: Some(0.3) }).unwrap_or_else(|| panic!("{} has no model", def.id));
                assert!(!p.traces.is_empty() && p.traces.iter().all(|t| t.ys.len() >= 64 && t.ys.iter().all(|y| y.is_finite())), "{}", def.id);
            }
        }
    }

    #[test]
    fn tremolo_caption_and_curve_follow_the_gain_law() {
        let p = plot_of("tremolo", &[], None);
        assert_eq!(p.readout, "10.5 dB");
        let lo = p.traces[0].ys.iter().cloned().fold(0.0, f32::max);
        // Deepest point: 20 log10(0.3) = -10.46 dB on the 0..-14 axis.
        assert!((lo - 10.46 / 14.0).abs() < 0.01, "{lo}");
        assert_eq!(p.traces.len(), 1);
        assert_eq!(plot_of("tremolo", &[("stereo_phase", 90.0)], None).traces.len(), 2);
        assert_eq!(plot_of("tremolo", &[("depth", 100.0)], None).readout, "\u{221e} dB");
        assert!(matches!(plot_of("tremolo", &[], Some(1.25)).live, Some(Live::Dot { .. })));
    }

    #[test]
    fn response_captions_name_the_first_notch() {
        // Phaser default: 4 stages swept around 1265 Hz.
        let p = plot_of("phaser", &[("depth", 0.0)], None);
        let fc = (200.0f32 * 8000.0).sqrt();
        let want = 48_000.0 / PI * ((PI * fc / 48_000.0).tan() * (PI / 8.0).tan()).atan();
        assert_eq!(p.readout, format!("{} Hz", hz_text(want)));
        // Flanger depth 0: one 1 ms tap, the notch at 500 Hz.
        assert_eq!(plot_of("flanger", &[("depth", 0.0)], None).readout, "500 Hz");
        // Chorus, one voice, 4 ms depth: 1.3 ms .. 5.3 ms -> 94 Hz .. 385 Hz.
        assert_eq!(plot_of("chorus", &[], None).readout, "94\u{2013}385 Hz");
        assert_eq!(hz_text(12_345.0), "12k");
        assert_eq!(hz_text(1234.0), "1.2k");
    }

    #[test]
    fn ring_mod_bias_and_rectify_shape_the_carrier() {
        let p = plot_of("ring-mod", &[("bias", 100.0)], None);
        // (w + 1) / 2 stays in 0..1: never below the rest line.
        assert!(p.traces[0].ys.iter().all(|y| *y <= 0.5 + 1e-4));
        let r = plot_of("ring-mod", &[("rectify", -100.0)], None);
        assert!(r.traces[0].ys.iter().all(|y| *y >= 0.5 - 1e-4));
        assert_eq!(plot_of("ring-mod", &[("modulator", 4.0)], None).traces.len(), 2);
    }

    #[test]
    fn character_mod_sweep_matches_the_twin_bbd_range() {
        let p = plot_of("character-mod", &[], Some(0.4));
        assert_eq!(p.readout, "1.7\u{2009}\u{2013}\u{2009}5.4 ms");
        assert_eq!((p.ticks[0].1.as_str(), p.ticks[1].1.as_str()), ("6", "3"));
        assert!(matches!(p.live, Some(Live::Dot { ref trail, .. }) if trail.len() == 25));
        for c in 0..CIRCUITS.len() {
            let p = plot_of("character-mod", &[("model", c as f32)], Some(0.4));
            assert!(p.traces.iter().all(|t| t.ys.iter().all(|y| y.is_finite())), "circuit {c}");
        }
    }
}
