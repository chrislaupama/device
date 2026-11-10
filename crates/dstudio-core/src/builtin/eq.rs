//! Parametric EQ model shared by the EQ devices (Channel EQ, Character EQ):
//! band shapes, the filter sections a band is made of, their response, and
//! the panel description ([`EqView`]) of an EQ card.
//!
//! The engine kernels (engine/src/devices/eq_sections.hpp) build the same
//! sections from the same settings, so the curve the panel draws is the
//! response the engine applies. Every section is a trapezoidal (TPT)
//! state-variable filter: its digital response is the analog prototype
//! evaluated at `s = j * tan(pi f / fs) / g`, with `g = tan(pi fc / fs)`.

use super::DeviceDef;
use std::f64::consts::PI;

/// What a band does to the spectrum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    Bell,
    LowShelf,
    HighShelf,
    Notch,
    HighPass,
    LowPass,
    /// Low-shelf boost and a lower, broader low-shelf cut together (the
    /// classic passive "boost and attenuate" low end).
    DualLow,
}

impl Shape {
    /// The band's Gain changes the response.
    pub fn has_gain(self) -> bool {
        matches!(self, Shape::Bell | Shape::LowShelf | Shape::HighShelf | Shape::DualLow)
    }
    pub fn is_pass(self) -> bool {
        matches!(self, Shape::HighPass | Shape::LowPass)
    }
    pub fn name(self) -> &'static str {
        match self {
            Shape::Bell => "Bell",
            Shape::LowShelf => "Low Shelf",
            Shape::HighShelf => "High Shelf",
            Shape::Notch => "Notch",
            Shape::HighPass => "High Pass",
            Shape::LowPass => "Low Pass",
            Shape::DualLow => "Dual Low",
        }
    }
}

/// One band in plain units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Band {
    pub shape: Shape,
    pub freq: f32,
    pub gain_db: f32,
    pub q: f32,
    /// Pass and shelf slope in dB/oct (6..48; shelves use 6..24).
    pub slope_db: f32,
    /// Bell Q grows with |gain| (proportional Q).
    pub prop_q: bool,
    /// 0..1: shelves dip back past their corner, pass filters bump at cutoff.
    pub overshoot: f32,
    /// Dual Low: depth of the low cut (dB, >= 0).
    pub low_atten_db: f32,
}

impl Band {
    pub const fn new(shape: Shape, freq: f32, gain_db: f32, q: f32) -> Band {
        Band { shape, freq, gain_db, q, slope_db: 12.0, prop_q: false, overshoot: 0.0, low_atten_db: 0.0 }
    }
}

/// A filter section in TPT state-variable form.
///
/// `Two`: `out = m[0] * x + m[1] * bp + m[2] * lp` with `bp = s / D`,
/// `lp = 1 / D`, `D = s^2 + k s + 1`. `One`: `out = m[0] * hp + m[1] * lp`
/// with `hp = s / (s + 1)`, `lp = 1 / (s + 1)`. `g` is the prewarped corner
/// `tan(pi fc / fs)`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Section {
    Two { g: f64, k: f64, m: [f64; 3] },
    One { g: f64, m: [f64; 2] },
}

/// Highest corner frequency a section may use (fraction of the sample rate).
pub const MAX_CORNER: f64 = 0.49;

fn prewarp(freq: f64, fs: f64) -> f64 {
    (PI * (freq / fs).clamp(1e-6, MAX_CORNER)).tan()
}

impl Section {
    /// Complex response (re, im) at `freq`.
    pub fn response(&self, freq: f64, fs: f64) -> (f64, f64) {
        let w = prewarp(freq, fs);
        match *self {
            Section::Two { g, k, m } => {
                let o = w / g;
                // D = 1 - o^2 + j k o; N = m0 D + m1 j o + m2.
                let (dr, di) = (1.0 - o * o, k * o);
                let (nr, ni) = (m[0] * dr + m[2], m[0] * di + m[1] * o);
                cdiv(nr, ni, dr, di)
            }
            Section::One { g, m } => {
                let o = w / g;
                cdiv(m[1], m[0] * o, 1.0, o)
            }
        }
    }
}

fn cdiv(a: f64, b: f64, c: f64, d: f64) -> (f64, f64) {
    let den = c * c + d * d;
    if den <= 1e-300 {
        return (0.0, 0.0);
    }
    ((a * c + b * d) / den, (b * c - a * d) / den)
}

/// Q of each second-order section of a Butterworth filter of `order`
/// (highest first), and whether a first-order section completes it.
pub fn butterworth_qs(order: u32) -> (Vec<f64>, bool) {
    let n = order.max(1);
    // Pole angles from the negative real axis: (2i - 1) pi / 2n (even), i pi / n (odd).
    let angle = |i: u32| if n % 2 == 0 { (2 * i - 1) as f64 * PI / (2 * n) as f64 } else { i as f64 * PI / n as f64 };
    let mut qs: Vec<f64> = (1..=n / 2).map(|i| 1.0 / (2.0 * angle(i).cos())).collect();
    qs.sort_by(|a, b| b.total_cmp(a));
    (qs, n % 2 == 1)
}

fn bell(freq: f64, gain_db: f64, q: f64, fs: f64) -> Section {
    let a = 10f64.powf(gain_db / 40.0);
    let k = 1.0 / (q * a);
    Section::Two { g: prewarp(freq, fs), k, m: [1.0, k * (a * a - 1.0), 0.0] }
}

fn low_shelf2(freq: f64, gain_db: f64, q: f64, fs: f64) -> Section {
    let a = 10f64.powf(gain_db / 40.0);
    let k = 1.0 / q;
    Section::Two { g: prewarp(freq, fs) / a.sqrt(), k, m: [1.0, k * (a - 1.0), a * a - 1.0] }
}

fn high_shelf2(freq: f64, gain_db: f64, q: f64, fs: f64) -> Section {
    let a = 10f64.powf(gain_db / 40.0);
    let k = 1.0 / q;
    Section::Two { g: prewarp(freq, fs) * a.sqrt(), k, m: [a * a, k * (1.0 - a) * a, 1.0 - a * a] }
}

fn low_shelf1(freq: f64, gain_db: f64, fs: f64) -> Section {
    let gg = 10f64.powf(gain_db / 20.0);
    Section::One { g: prewarp(freq, fs) / gg.sqrt(), m: [1.0, gg] }
}

fn high_shelf1(freq: f64, gain_db: f64, fs: f64) -> Section {
    let gg = 10f64.powf(gain_db / 20.0);
    Section::One { g: prewarp(freq, fs) * gg.sqrt(), m: [gg, 1.0] }
}

/// Overshoot bell of a shelf: past the corner, opposite to the shelf gain.
pub const OVERSHOOT_RATIO: f64 = 1.6;
pub const OVERSHOOT_Q: f64 = 1.2;
/// Dual Low: the cut shelf sits this far below the boost, with this Q.
pub const DUAL_LOW_CUT_RATIO: f64 = 0.7;
pub const DUAL_LOW_CUT_Q: f64 = 0.5;

/// Q a bell actually uses (proportional Q doubles it per 12 dB of gain).
pub fn bell_q(b: &Band) -> f64 {
    let q = b.q.max(0.01) as f64;
    if b.prop_q { q * 2f64.powf(b.gain_db.abs() as f64 / 12.0) } else { q }
}

/// The sections of one band at sample rate `fs` (empty: the band is flat).
pub fn sections(b: &Band, fs: f64) -> Vec<Section> {
    let f = b.freq.max(1.0) as f64;
    let g = b.gain_db as f64;
    let q = b.q.max(0.01) as f64;
    let ov = b.overshoot.clamp(0.0, 1.0) as f64;
    let mut out = Vec::new();
    match b.shape {
        Shape::Bell => {
            if g != 0.0 {
                out.push(bell(f, g, bell_q(b), fs));
            }
        }
        Shape::Notch => out.push(Section::Two { g: prewarp(f, fs), k: 1.0 / q, m: [1.0, -1.0 / q, 0.0] }),
        Shape::LowShelf | Shape::HighShelf => {
            if g != 0.0 {
                let low = b.shape == Shape::LowShelf;
                let (n2, n1) = shelf_split(b.slope_db);
                let units = (2 * n2 + n1) as f64;
                for _ in 0..n2 {
                    let gs = g * 2.0 / units;
                    out.push(if low { low_shelf2(f, gs, q, fs) } else { high_shelf2(f, gs, q, fs) });
                }
                for _ in 0..n1 {
                    let gs = g / units;
                    out.push(if low { low_shelf1(f, gs, fs) } else { high_shelf1(f, gs, fs) });
                }
                if ov > 0.0 {
                    let fo = if low { f * OVERSHOOT_RATIO } else { f / OVERSHOOT_RATIO };
                    out.push(bell(fo, -0.5 * ov * g, OVERSHOOT_Q, fs));
                }
            }
        }
        Shape::HighPass | Shape::LowPass => {
            let hp = b.shape == Shape::HighPass;
            let order = (b.slope_db / 6.0).round().clamp(1.0, 8.0) as u32;
            let (qs, first) = butterworth_qs(order);
            let w = prewarp(f, fs);
            for (i, bq) in qs.iter().enumerate() {
                let qq = if i == 0 { bq * (q / std::f64::consts::FRAC_1_SQRT_2) * (1.0 + 2.0 * ov) } else { *bq };
                let k = 1.0 / qq;
                out.push(Section::Two { g: w, k, m: if hp { [1.0, -k, -1.0] } else { [0.0, 0.0, 1.0] } });
            }
            if first {
                out.push(Section::One { g: w, m: if hp { [1.0, 0.0] } else { [0.0, 1.0] } });
            }
        }
        Shape::DualLow => {
            if g != 0.0 {
                out.push(low_shelf2(f, g, q, fs));
            }
            if b.low_atten_db > 0.0 {
                out.push(low_shelf2(f * DUAL_LOW_CUT_RATIO, -(b.low_atten_db as f64), DUAL_LOW_CUT_Q, fs));
            }
        }
    }
    out
}

/// Second- and first-order section counts of a shelf of `slope_db`
/// (6, 12, 18, 24; steeper settings use 24).
pub fn shelf_split(slope_db: f32) -> (usize, usize) {
    match (slope_db / 6.0).round() as i32 {
        i32::MIN..=1 => (0, 1),
        2 => (1, 0),
        3 => (1, 1),
        _ => (2, 0),
    }
}

/// Magnitude (dB) of a band at `freq`.
pub fn band_db(b: &Band, freq: f64, fs: f64) -> f64 {
    sections_db(&sections(b, fs), freq, fs)
}

/// Magnitude (dB) of a list of sections at `freq`.
pub fn sections_db(s: &[Section], freq: f64, fs: f64) -> f64 {
    let mut m2 = 1.0;
    for sec in s {
        let (re, im) = sec.response(freq, fs);
        m2 *= re * re + im * im;
    }
    10.0 * m2.max(1e-24).log10()
}

/// Log-spaced frequencies from 20 Hz to 20 kHz.
pub fn log_freqs(n: usize) -> impl Iterator<Item = f64> {
    let n = n.max(2);
    (0..n).map(move |i| 20.0 * 1000f64.powf(i as f64 / (n - 1) as f64))
}

/// Auto Gain: the level change that cancels the EQ's average boost or cut
/// (mean dB of the curve over 32 log-spaced points, 20 Hz..20 kHz),
/// limited to +/-24 dB.
pub fn auto_gain_db(sections: &[Section], fs: f64) -> f64 {
    let n = 32;
    let mean = log_freqs(n).map(|f| sections_db(sections, f, fs)).sum::<f64>() / n as f64;
    (-mean).clamp(-24.0, 24.0)
}

// ---------------------------------------------------------------- banks

/// Character EQ banks: hardware-inspired voicings. Each bank has its own
/// frequency detents (the band frequency snaps to them in the panel) and a
/// voicing of the band shapes. The lists and voicings are our own.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bank {
    pub name: &'static str,
    /// Frequency detents (Hz), ascending; empty = free.
    pub detents: &'static [f32],
    /// Multiplies every band's Q (below 1 = broader).
    pub q_scale: f32,
    /// Bells always use proportional Q.
    pub prop_q: bool,
    /// Added to shelf and pass overshoot (0..1).
    pub overshoot: f32,
    /// Band drive asymmetry (0 = odd harmonics only, 1 = strongly even).
    pub asymmetry: f32,
}

pub const BANKS: [Bank; 10] = [
    Bank { name: "Free", detents: &[], q_scale: 1.0, prop_q: false, overshoot: 0.0, asymmetry: 0.15 },
    Bank { name: "Passive Valve", detents: &[20.0, 30.0, 60.0, 100.0, 1500.0, 3000.0, 5000.0, 8000.0, 10_000.0, 12_000.0, 16_000.0], q_scale: 0.8, prop_q: false, overshoot: 0.3, asymmetry: 0.6 },
    Bank { name: "Inductor Broad", detents: &[35.0, 60.0, 110.0, 220.0, 360.0, 700.0, 1600.0, 3200.0, 4800.0, 7200.0, 12_000.0], q_scale: 0.75, prop_q: true, overshoot: 0.15, asymmetry: 0.35 },
    Bank { name: "Transistor Desk", detents: &[50.0, 80.0, 120.0, 200.0, 300.0, 500.0, 800.0, 1200.0, 2000.0, 3000.0, 5000.0, 8000.0, 12_000.0], q_scale: 1.0, prop_q: true, overshoot: 0.05, asymmetry: 0.1 },
    Bank { name: "Discrete Op-Amp", detents: &[40.0, 63.0, 100.0, 160.0, 250.0, 400.0, 630.0, 1000.0, 1600.0, 2500.0, 4000.0, 6300.0, 10_000.0, 16_000.0], q_scale: 1.15, prop_q: false, overshoot: 0.0, asymmetry: 0.05 },
    Bank { name: "Class A Bell", detents: &[60.0, 100.0, 200.0, 400.0, 700.0, 1000.0, 1800.0, 2700.0, 4000.0, 6000.0, 9000.0], q_scale: 0.9, prop_q: true, overshoot: 0.0, asymmetry: 0.45 },
    Bank { name: "Class A Shelf", detents: &[30.0, 50.0, 80.0, 120.0, 200.0, 3500.0, 5000.0, 7500.0, 10_000.0, 15_000.0], q_scale: 1.0, prop_q: false, overshoot: 0.2, asymmetry: 0.45 },
    Bank { name: "Inductor Fine", detents: &[45.0, 75.0, 150.0, 300.0, 500.0, 900.0, 1400.0, 2200.0, 3300.0, 5000.0, 7500.0, 11_000.0], q_scale: 1.4, prop_q: true, overshoot: 0.1, asymmetry: 0.3 },
    Bank { name: "Broadcast Fixed", detents: &[80.0, 150.0, 300.0, 1000.0, 2500.0, 5000.0, 10_000.0], q_scale: 0.85, prop_q: false, overshoot: 0.0, asymmetry: 0.2 },
    Bank { name: "Passive Mid", detents: &[200.0, 300.0, 500.0, 700.0, 1000.0, 1500.0, 2000.0, 3000.0, 4000.0, 5000.0, 7000.0], q_scale: 0.7, prop_q: false, overshoot: 0.1, asymmetry: 0.5 },
];

/// The bank of a Bank parameter value (clamped).
pub fn bank(index: f32) -> &'static Bank {
    &BANKS[(index.round().max(0.0) as usize).min(BANKS.len() - 1)]
}

impl Bank {
    /// A band as this bank voices it.
    pub fn voice(&self, mut b: Band) -> Band {
        b.q *= self.q_scale;
        b.prop_q |= self.prop_q && b.shape == Shape::Bell;
        if matches!(b.shape, Shape::LowShelf | Shape::HighShelf | Shape::HighPass | Shape::LowPass) {
            b.overshoot = (b.overshoot + self.overshoot).min(1.0);
        }
        b
    }

    /// Nearest detent to `freq` (log distance), or `freq` for a free bank.
    pub fn snap(&self, freq: f32) -> f32 {
        let f = freq.max(1.0);
        self.detents.iter().copied().min_by(|a, b| (a / f).ln().abs().total_cmp(&(b / f).ln().abs())).unwrap_or(freq)
    }
}

// ---------------------------------------------------------------- panel description

/// Parameter ids of one EQ band.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EqBand {
    pub on: &'static str,
    pub kind: &'static str,
    pub freq: &'static str,
    pub gain: &'static str,
    pub q: &'static str,
    /// Per-band slope (choice names start with their dB/oct number).
    pub slope: Option<&'static str>,
    pub prop_q: Option<&'static str>,
    pub overshoot: Option<&'static str>,
    /// Dual Low cut depth; the third value cell edits it for Dual Low bands.
    pub low_atten: Option<&'static str>,
    /// Dynamic band switch and its range (dB): the panel draws the reach.
    pub dyn_on: Option<&'static str>,
    pub dyn_range: Option<&'static str>,
}

impl EqBand {
    pub const fn new(on: &'static str, kind: &'static str, freq: &'static str, gain: &'static str, q: &'static str) -> EqBand {
        EqBand { on, kind, freq, gain, q, slope: None, prop_q: None, overshoot: None, low_atten: None, dyn_on: None, dyn_range: None }
    }
}

/// What double-clicking a band node does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeDoubleClick {
    /// Switch the band off (it parks at the bottom of the field).
    Park,
    /// Nothing (bands are removed with Delete / Backspace).
    None,
}

/// Panel description of an EQ card (`View::Eq`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EqView {
    pub bands: &'static [EqBand],
    /// Shape of each choice of the band type parameters (by choice index).
    pub shapes: &'static [Shape],
    /// Type picker order (shapes missing from `shapes` are skipped).
    pub picker_order: &'static [Shape],
    /// Shared pass slopes (choice names start with their dB/oct number):
    /// bands whose type is a high pass use `hp_slope`, low pass `lp_slope`.
    pub hp_slope: Option<&'static str>,
    pub lp_slope: Option<&'static str>,
    /// Output slider parameter.
    pub output: &'static str,
    /// Bank picker parameter (Character EQ).
    pub bank: Option<&'static str>,
    /// Auto Gain switch (adds the make-up to the drawn curve).
    pub auto_gain: Option<&'static str>,
    /// Plot range of the field (dB, top / bottom) and rung spacing.
    pub top_db: f32,
    pub bottom_db: f32,
    pub rung_db: f32,
    /// Slider column width (pt) and the widths of the three value cells (pt).
    pub slider_w: f32,
    pub cells: [f32; 3],
    /// Band selected when the card opens.
    pub first_band: usize,
    pub double_click: NodeDoubleClick,
    /// Clicking empty field space enables the first free band there.
    pub create_on_click: bool,
    /// Band nodes grow with how much the band moves the curve.
    pub prominent_nodes: bool,
}

impl EqView {
    /// Every parameter id the view names.
    pub fn param_ids(&self) -> Vec<&'static str> {
        let mut v = vec![self.output];
        v.extend(self.hp_slope);
        v.extend(self.lp_slope);
        v.extend(self.bank);
        v.extend(self.auto_gain);
        for b in self.bands {
            v.extend([b.on, b.kind, b.freq, b.gain, b.q]);
            v.extend([b.slope, b.prop_q, b.overshoot, b.low_atten, b.dyn_on, b.dyn_range].into_iter().flatten());
        }
        v
    }

    /// Shape of a type choice index.
    pub fn shape(&self, kind: f32) -> Shape {
        self.shapes[(kind.round().max(0.0) as usize).min(self.shapes.len() - 1)]
    }

    /// Type choice index of a shape, if the device offers it.
    pub fn kind_of(&self, shape: Shape) -> Option<usize> {
        self.shapes.iter().position(|s| *s == shape)
    }

    /// The slope parameter a band uses in its current shape.
    pub fn slope_param(&self, i: usize, shape: Shape) -> Option<&'static str> {
        match (shape, self.bands[i].slope) {
            (_, Some(s)) => Some(s),
            (Shape::HighPass, None) => self.hp_slope,
            (Shape::LowPass, None) => self.lp_slope,
            _ => None,
        }
    }

    /// Band `i` from parameter values (`value(id)` = plain value), voiced by
    /// the bank when there is one. `with_dyn` adds the dynamic range to the
    /// gain (the reach of a dynamic band).
    pub fn band(&self, def: &DeviceDef, i: usize, value: &dyn Fn(&str) -> f32, with_dyn: bool) -> Band {
        let d = &self.bands[i];
        let shape = self.shape(value(d.kind));
        let slope_db = self
            .slope_param(i, shape)
            .and_then(|s| def.param(s).map(|p| choice_number(p.choices.get(value(s).round().max(0.0) as usize).copied().unwrap_or("12"))))
            .unwrap_or(12.0);
        let mut gain_db = value(d.gain);
        if with_dyn && d.dyn_on.is_some_and(|p| value(p) >= 0.5) {
            gain_db += d.dyn_range.map_or(0.0, |p| value(p));
        }
        let b = Band {
            shape,
            freq: value(d.freq),
            gain_db,
            q: value(d.q),
            slope_db,
            prop_q: d.prop_q.is_some_and(|p| value(p) >= 0.5),
            overshoot: d.overshoot.map_or(0.0, |p| value(p) / 100.0),
            low_atten_db: d.low_atten.map_or(0.0, |p| value(p)),
        };
        match self.bank {
            Some(p) => bank(value(p)).voice(b),
            None => b,
        }
    }

    pub fn band_on(&self, i: usize, value: &dyn Fn(&str) -> f32) -> bool {
        value(self.bands[i].on) >= 0.5
    }

    /// Sections of every enabled band, in band order.
    pub fn all_sections(&self, def: &DeviceDef, value: &dyn Fn(&str) -> f32, fs: f64) -> Vec<Section> {
        (0..self.bands.len()).filter(|i| self.band_on(*i, value)).flat_map(|i| sections(&self.band(def, i, value, false), fs)).collect()
    }
}

/// The number at the start of a choice name ("24 dB/oct" -> 24, "18" -> 18).
pub fn choice_number(name: &str) -> f32 {
    let end = name.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(name.len());
    name[..end].parse().unwrap_or(12.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FS: f64 = 48_000.0;

    #[test]
    fn bell_peaks_at_its_gain() {
        let b = Band::new(Shape::Bell, 1000.0, 6.0, 1.0);
        assert!((band_db(&b, 1000.0, FS) - 6.0).abs() < 1e-6);
        assert!(band_db(&b, 20.0, FS).abs() < 0.05);
        let cut = Band::new(Shape::Bell, 1000.0, -12.0, 2.0);
        assert!((band_db(&cut, 1000.0, FS) + 12.0).abs() < 1e-6);
        let up = Band::new(Shape::Bell, 1000.0, 12.0, 2.0);
        for f in [300.0, 800.0, 1500.0] {
            assert!((band_db(&up, f, FS) + band_db(&cut, f, FS)).abs() < 1e-6, "a cut mirrors a boost in dB");
        }
    }

    #[test]
    fn shelves_reach_their_gain_and_split_by_slope() {
        for slope in [6.0, 12.0, 18.0, 24.0] {
            let mut ls = Band::new(Shape::LowShelf, 200.0, 9.0, 0.7071);
            ls.slope_db = slope;
            assert!((band_db(&ls, 20.0, FS) - 9.0).abs() < 0.6, "low shelf {slope}: {}", band_db(&ls, 20.0, FS));
            assert!(band_db(&ls, 15_000.0, FS).abs() < 0.1);
            let mut hs = Band::new(Shape::HighShelf, 2000.0, -6.0, 0.7071);
            hs.slope_db = slope;
            assert!((band_db(&hs, 20_000.0, FS) + 6.0).abs() < 0.6, "high shelf {slope}: {}", band_db(&hs, 20_000.0, FS));
            assert!(band_db(&hs, 20.0, FS).abs() < 0.05);
            assert!((band_db(&ls, 200.0, FS) - 4.5).abs() < 0.6, "low shelf {slope} corner {}", band_db(&ls, 200.0, FS));
        }
        let mut steep = Band::new(Shape::LowShelf, 200.0, 9.0, 0.7071);
        steep.slope_db = 24.0;
        let gentle = Band::new(Shape::LowShelf, 200.0, 9.0, 0.7071);
        assert!(band_db(&steep, 100.0, FS) > band_db(&gentle, 100.0, FS), "steeper shelves reach full gain sooner");
    }

    #[test]
    fn pass_filters_are_butterworth_at_default_q() {
        for slope in [6.0, 12.0, 18.0, 24.0, 36.0, 48.0] {
            let mut hp = Band::new(Shape::HighPass, 1000.0, 0.0, std::f32::consts::FRAC_1_SQRT_2);
            hp.slope_db = slope;
            assert!((band_db(&hp, 1000.0, FS) + 3.01).abs() < 0.05, "hp {slope}: -3 dB at cutoff ({})", band_db(&hp, 1000.0, FS));
            let below = band_db(&hp, 125.0, FS);
            assert!((below + 3.0 * slope as f64).abs() < 1.5, "hp {slope}: {below} three octaves down");
            let mut lp = hp;
            lp.shape = Shape::LowPass;
            assert!((band_db(&lp, 1000.0, FS) + 3.01).abs() < 0.05);
            assert!(band_db(&lp, 50.0, FS).abs() < 0.02);
        }
        let mut res = Band::new(Shape::HighPass, 1000.0, 0.0, 4.0);
        res.slope_db = 24.0;
        assert!(band_db(&res, 1000.0, FS) > 6.0, "Q raises the corner");
    }

    #[test]
    fn notch_and_dual_low() {
        let n = Band::new(Shape::Notch, 1000.0, 0.0, 2.0);
        assert!(band_db(&n, 1000.0, FS) < -100.0);
        assert!(band_db(&n, 100.0, FS).abs() < 0.1);
        let mut d = Band::new(Shape::DualLow, 60.0, 8.0, 0.7071);
        d.low_atten_db = 6.0;
        let deep = band_db(&d, 20.0, FS);
        assert!(deep < 8.0 && deep > 0.0, "boost minus cut at the bottom ({deep})");
        assert!(band_db(&d, 60.0 * DUAL_LOW_CUT_RATIO, FS) > deep, "the boost peaks above the cut");
    }

    #[test]
    fn proportional_q_and_overshoot() {
        let mut b = Band::new(Shape::Bell, 1000.0, 12.0, 1.0);
        let wide = band_db(&b, 1500.0, FS);
        b.prop_q = true;
        assert!(band_db(&b, 1500.0, FS) < wide, "proportional Q narrows a strong bell");
        let mut s = Band::new(Shape::LowShelf, 100.0, 6.0, 0.7071);
        s.overshoot = 1.0;
        assert!(band_db(&s, 160.0, FS) < band_db(&Band::new(Shape::LowShelf, 100.0, 6.0, 0.7071), 160.0, FS));
        assert_eq!(sections(&s, FS).len(), 2);
    }

    /// Reference points shared with the engine's tests
    /// (engine/tests/devices/test_eq.cpp, eq_design_matches_the_panel_model):
    /// if these change, the panel and the engine drift apart.
    #[test]
    fn engine_contract_points() {
        let mut a = Band::new(Shape::LowShelf, 200.0, 9.0, 0.7071);
        a.slope_db = 18.0;
        a.overshoot = 0.5;
        let mut b = Band::new(Shape::HighPass, 1000.0, 0.0, 2.0);
        b.slope_db = 36.0;
        let mut c = Band::new(Shape::DualLow, 60.0, 8.0, 0.7071);
        c.low_atten_db = 6.0;
        let mut d = Band::new(Shape::Bell, 2500.0, -7.5, 1.3);
        d.prop_q = true;
        let want = [(a, 100.0, 7.843498), (a, 330.0, -0.675584), (b, 900.0, -0.567544), (b, 400.0, -47.592860), (c, 30.0, 3.508999), (c, 120.0, -0.137253), (d, 2500.0, -7.5), (d, 3500.0, -2.570398)];
        for (band, f, db) in want {
            assert!((band_db(&band, f, FS) - db).abs() < 1e-4, "{:?} at {f}", band.shape);
        }
    }

    #[test]
    fn butterworth_tables() {
        let (q, first) = butterworth_qs(4);
        assert!(!first);
        assert!((q[0] - 1.3065629648763766).abs() < 1e-9 && (q[1] - 0.5411961001461969).abs() < 1e-9);
        let (q, first) = butterworth_qs(3);
        assert!(first && (q[0] - 1.0).abs() < 1e-9);
    }

    #[test]
    fn auto_gain_cancels_the_average() {
        let s = sections(&Band::new(Shape::HighShelf, 1000.0, 12.0, 0.7071), FS);
        let g = auto_gain_db(&s, FS);
        assert!(g < -3.0 && g > -12.0, "{g}");
        assert_eq!(auto_gain_db(&[], FS), 0.0);
    }

    #[test]
    fn banks_snap_and_voice() {
        assert_eq!(BANKS[0].snap(1234.0), 1234.0);
        assert_eq!(BANKS[8].snap(1200.0), 1000.0);
        assert_eq!(BANKS[8].snap(20.0), 80.0);
        let v = BANKS[2].voice(Band::new(Shape::Bell, 1000.0, 6.0, 1.0));
        assert!(v.prop_q && (v.q - 0.75).abs() < 1e-6);
        assert_eq!(bank(99.0).name, "Passive Mid");
    }
}
