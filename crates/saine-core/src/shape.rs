//! Shape: the clip sidebar's second tab. Its rack reshapes what the clip's
//! composer plays (a library pattern, or the clip's kept notes) without losing
//! it: while a Shape is not neutral, the unshaped notes stay in
//! \`Composer::base\` and the clip plays \`apply(base)\`.
//!
//! Every control has a base value and an optional drawn curve: one offset per
//! beat from the clip start, added to the base. The rack's rows, ranges, neutral
//! values and the transforms' behaviour follow the reference (findings SQ-13..SQ-18);
//! the code is Saine's own.

use crate::library::PartType;
use crate::midi::{self, Note};
use crate::model::ProjectKey;
use crate::{PPQ, Ticks};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Repeat: beats per retrigger (0 = off) and the rack's labels.
pub const REPEAT_BEATS: [f64; 5] = [0.0, 1.0, 0.5, 0.25, 0.125];
pub const REPEAT_LABELS: [&str; 5] = ["Off", "¼", "⅛", "16", "32"];
/// Arp: beats per step (0 = off) and labels.
pub const ARP_BEATS: [f64; 6] = [0.0, 0.5, 1.0 / 3.0, 0.25, 1.0 / 6.0, 0.125];
pub const ARP_LABELS: [&str; 6] = ["Off", "1/8", "1/8T", "1/16", "1/16T", "1/32"];
/// Arp patterns, in the reference's order.
pub const ARP_PATTERNS: [&str; 7] = ["Up", "Down", "Up & down", "Random", "Thumb", "Pinky", "Chord"];
/// Arp ranges in octaves.
pub const ARP_OCTAVES: [u8; 4] = [1, 2, 3, 4];
pub const INVERSION_LABELS: [&str; 4] = ["Root", "1st", "2nd", "3rd"];
/// Octave steps either way.
pub const MAX_OCTAVE: i8 = 2;
/// Most voices the Voices control offers (1 = the line alone).
pub const MAX_VOICES: u8 = 3;
/// Longest curve (one point per beat).
pub const MAX_CURVE_POINTS: usize = 4097;
/// How far Nudge moves notes at either end, in beats.
const NUDGE_BEATS: f64 = 0.5;
/// Part of each retrigger or arp step that sounds at the neutral Length.
const GATE: f64 = 0.86;
/// Shortest retrigger gate (Length 0).
const MIN_GATE: f64 = 0.1;
/// Shortest note Length leaves.
const MIN_TICKS: Ticks = PPQ / 64;
/// Notes of one pitch closer than this join when Length holds them.
const JOIN_TICKS: Ticks = PPQ / 32;
/// Length holds a note at most this far.
const MAX_HOLD: Ticks = 4 * PPQ;
/// Density never drops below this part of single notes.
const MIN_DENSITY: f64 = 0.15;

/// The rack's controls, in its order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ShapeKey {
    Octave,
    Voices,
    Density,
    Length,
    Velocity,
    Vary,
    Nudge,
    Repeat,
    Air,
    Sub,
    Spread,
    Tension,
    Inversion,
    Arp,
    ArpPattern,
    ArpOctaves,
}

impl ShapeKey {
    pub const ALL: [ShapeKey; 16] = [
        ShapeKey::Octave,
        ShapeKey::Voices,
        ShapeKey::Density,
        ShapeKey::Length,
        ShapeKey::Velocity,
        ShapeKey::Vary,
        ShapeKey::Nudge,
        ShapeKey::Repeat,
        ShapeKey::Air,
        ShapeKey::Sub,
        ShapeKey::Spread,
        ShapeKey::Tension,
        ShapeKey::Inversion,
        ShapeKey::Arp,
        ShapeKey::ArpPattern,
        ShapeKey::ArpOctaves,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ShapeKey::Octave => "Octave",
            ShapeKey::Voices => "Voices",
            ShapeKey::Density => "Density",
            ShapeKey::Length => "Length",
            ShapeKey::Velocity => "Velocity",
            ShapeKey::Vary => "Vary",
            ShapeKey::Nudge => "Nudge",
            ShapeKey::Repeat => "Repeat",
            ShapeKey::Air => "Air",
            ShapeKey::Sub => "Sub",
            ShapeKey::Spread => "Spread",
            ShapeKey::Tension => "Tension",
            ShapeKey::Inversion => "Inversion",
            ShapeKey::Arp => "Arp",
            ShapeKey::ArpPattern => "Pattern",
            ShapeKey::ArpOctaves => "Octaves",
        }
    }

    /// Curve colour (0xRRGGBB).
    pub fn color(self) -> u32 {
        match self {
            ShapeKey::Octave => 0x60D890,
            ShapeKey::Voices => 0x30A098,
            ShapeKey::Density => 0x7888C8,
            ShapeKey::Length => 0xD0A050,
            ShapeKey::Velocity => 0xD85088,
            ShapeKey::Vary => 0x9080D0,
            ShapeKey::Nudge => 0x50B8B0,
            ShapeKey::Repeat => 0xA0D050,
            ShapeKey::Air => 0xC07030,
            ShapeKey::Sub => 0xB068D0,
            ShapeKey::Spread => 0x68A8C0,
            ShapeKey::Tension => 0xC86040,
            ShapeKey::Inversion => 0x30A098,
            ShapeKey::Arp => 0xD098D8,
            ShapeKey::ArpPattern => 0xB8A0D8,
            ShapeKey::ArpOctaves => 0xE08880,
        }
    }

    /// The five chord controls: one value per beat, no smoothing between beats.
    pub fn is_harmony(self) -> bool {
        matches!(self, ShapeKey::Air | ShapeKey::Sub | ShapeKey::Spread | ShapeKey::Tension | ShapeKey::Inversion)
    }

    /// Option labels of a stepped control (empty for sliders).
    pub fn options(self) -> Vec<String> {
        match self {
            ShapeKey::Octave => ["−2", "−1", "0", "+1", "+2"].map(String::from).to_vec(),
            ShapeKey::Voices => ["1", "2", "3"].map(String::from).to_vec(),
            ShapeKey::Repeat => REPEAT_LABELS.map(String::from).to_vec(),
            ShapeKey::Inversion => INVERSION_LABELS.map(String::from).to_vec(),
            ShapeKey::Arp => ARP_LABELS.map(String::from).to_vec(),
            ShapeKey::ArpPattern => ARP_PATTERNS.map(String::from).to_vec(),
            ShapeKey::ArpOctaves => ARP_OCTAVES.map(|o| o.to_string()).to_vec(),
            _ => Vec::new(),
        }
    }

    /// Number of options of a stepped control (1 for sliders).
    pub fn option_count(self) -> usize {
        match self {
            ShapeKey::Octave | ShapeKey::Repeat => 5,
            ShapeKey::Voices => 3,
            ShapeKey::Inversion | ShapeKey::ArpOctaves => 4,
            ShapeKey::Arp => 6,
            ShapeKey::ArpPattern => 7,
            _ => 1,
        }
    }

    /// Option index of a 0..1 value (stepped controls).
    pub fn index_of(self, v: f32) -> usize {
        let v = v.clamp(0.0, 1.0);
        let n = self.option_count();
        match self {
            ShapeKey::Octave | ShapeKey::Voices | ShapeKey::Inversion => (v * (n - 1) as f32).round() as usize,
            _ => ((v * n as f32) as usize).min(n - 1),
        }
    }

    /// The 0..1 value that stands for an option.
    pub fn value_of(self, i: usize) -> f32 {
        let n = self.option_count();
        let i = i.min(n - 1) as f32;
        match self {
            ShapeKey::Octave | ShapeKey::Voices | ShapeKey::Inversion => i / (n - 1) as f32,
            _ => (i + 0.5) / n as f32,
        }
    }

    /// The neutral value centres a two-sided control (its curve guide is drawn at 0.5).
    pub fn bipolar(self) -> bool {
        matches!(self, ShapeKey::Octave | ShapeKey::Length | ShapeKey::Nudge)
    }
}

/// A drawn curve: one offset per beat from the clip start, added to the base value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Curve {
    pub key: ShapeKey,
    pub points: Vec<f32>,
}

fn two() -> u8 {
    2
}
fn one() -> u8 {
    1
}

/// One source's Shape: base values (continuous ones 0..=1, stepped ones as option
/// indices) and the drawn curves.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Shape {
    /// Octaves up or down, -2..=2.
    pub octave: i8,
    /// Voices played per note of a single line, 1..=3.
    pub voices: u8,
    /// Part of the notes kept (1 = all).
    pub density: f32,
    /// Note length: 0 short, 0.5 as made, 1 held to the next note.
    pub length: f32,
    /// Note velocity; the neutral value plays the notes as made.
    pub velocity: f32,
    /// Timing: 0.5 on time, 0 / 1 half a beat early / late.
    pub nudge: f32,
    /// Index into REPEAT_BEATS.
    pub repeat: u8,
    /// Index into ARP_BEATS.
    pub arp: u8,
    /// How much Density picks at random instead of keeping an even spread.
    #[serde(default)]
    pub vary: f32,
    /// Chord tones added above the chord (0..3 as 0..1).
    #[serde(default)]
    pub air: f32,
    /// Chord tones added below the chord.
    #[serde(default)]
    pub sub: f32,
    /// How far the chord's outer notes move apart.
    #[serde(default)]
    pub spread: f32,
    /// Thirds stacked on the chord (0..4 as 0..1).
    #[serde(default)]
    pub tension: f32,
    /// Inversion, 0..=3.
    #[serde(default)]
    pub inversion: u8,
    /// Index into ARP_PATTERNS.
    #[serde(default = "two")]
    pub arp_pattern: u8,
    /// Index into ARP_OCTAVES.
    #[serde(default = "one")]
    pub arp_octaves: u8,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub curves: Vec<Curve>,
}

/// The velocity value that plays a part's notes as made.
pub fn neutral_velocity(part: Option<PartType>) -> f32 {
    if part == Some(PartType::Bass) { 0.85 } else { 0.8 }
}

/// Points of a curve over a clip of \`clip\` ticks: one per beat, the end included.
pub fn curve_len(clip: Ticks) -> usize {
    ((clip as f64 / PPQ as f64).round() as usize + 1).clamp(2, MAX_CURVE_POINTS)
}

impl Shape {
    /// The Shape that plays a source's notes as made.
    pub fn neutral(part: Option<PartType>) -> Shape {
        Shape {
            octave: 0,
            voices: 1,
            density: 1.0,
            length: 0.5,
            velocity: neutral_velocity(part),
            nudge: 0.5,
            repeat: 0,
            arp: 0,
            vary: 0.0,
            air: 0.0,
            sub: 0.0,
            spread: 0.0,
            tension: 0.0,
            inversion: 0,
            arp_pattern: 2,
            arp_octaves: 1,
            curves: Vec::new(),
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        let unit = [self.density, self.length, self.velocity, self.nudge, self.vary, self.air, self.sub, self.spread, self.tension];
        if !(-MAX_OCTAVE..=MAX_OCTAVE).contains(&self.octave) || !(1..=MAX_VOICES).contains(&self.voices) {
            return Err("shape octave or voices out of range".into());
        }
        if unit.iter().any(|v| !(0.0..=1.0).contains(v))
            || self.repeat as usize >= REPEAT_BEATS.len()
            || self.arp as usize >= ARP_BEATS.len()
            || self.arp_pattern as usize >= ARP_PATTERNS.len()
            || self.arp_octaves as usize >= ARP_OCTAVES.len()
            || self.inversion as usize >= INVERSION_LABELS.len()
        {
            return Err("shape value out of range".into());
        }
        for (i, c) in self.curves.iter().enumerate() {
            if self.curves[..i].iter().any(|d| d.key == c.key) {
                return Err("shape curve listed twice".into());
            }
            if c.points.len() < 2 || c.points.len() > MAX_CURVE_POINTS || c.points.iter().any(|p| !(-1.0..=1.0).contains(p)) {
                return Err("shape curve out of range".into());
            }
        }
        Ok(())
    }

    /// A control's base value, 0..1.
    pub fn norm(&self, k: ShapeKey) -> f32 {
        match k {
            ShapeKey::Octave => k.value_of((self.octave + MAX_OCTAVE) as usize),
            ShapeKey::Voices => k.value_of(self.voices as usize - 1),
            ShapeKey::Density => self.density,
            ShapeKey::Length => self.length,
            ShapeKey::Velocity => self.velocity,
            ShapeKey::Vary => self.vary,
            ShapeKey::Nudge => self.nudge,
            ShapeKey::Repeat => k.value_of(self.repeat as usize),
            ShapeKey::Air => self.air,
            ShapeKey::Sub => self.sub,
            ShapeKey::Spread => self.spread,
            ShapeKey::Tension => self.tension,
            ShapeKey::Inversion => k.value_of(self.inversion as usize),
            ShapeKey::Arp => k.value_of(self.arp as usize),
            ShapeKey::ArpPattern => k.value_of(self.arp_pattern as usize),
            ShapeKey::ArpOctaves => k.value_of(self.arp_octaves as usize),
        }
    }

    /// Set a control's base value from 0..1 (stepped controls take the nearest option).
    pub fn set_norm(&mut self, k: ShapeKey, v: f32) {
        let v = v.clamp(0.0, 1.0);
        let i = k.index_of(v);
        match k {
            ShapeKey::Octave => self.octave = i as i8 - MAX_OCTAVE,
            ShapeKey::Voices => self.voices = i as u8 + 1,
            ShapeKey::Density => self.density = v,
            ShapeKey::Length => self.length = v,
            ShapeKey::Velocity => self.velocity = v,
            ShapeKey::Vary => self.vary = v,
            ShapeKey::Nudge => self.nudge = v,
            ShapeKey::Repeat => self.repeat = i as u8,
            ShapeKey::Air => self.air = v,
            ShapeKey::Sub => self.sub = v,
            ShapeKey::Spread => self.spread = v,
            ShapeKey::Tension => self.tension = v,
            ShapeKey::Inversion => self.inversion = i as u8,
            ShapeKey::Arp => self.arp = i as u8,
            ShapeKey::ArpPattern => self.arp_pattern = i as u8,
            ShapeKey::ArpOctaves => self.arp_octaves = i as u8,
        }
    }

    pub fn curve(&self, k: ShapeKey) -> Option<&[f32]> {
        self.curves.iter().find(|c| c.key == k).map(|c| c.points.as_slice())
    }

    /// Store a curve; one with no offset is removed.
    pub fn set_curve(&mut self, k: ShapeKey, points: Vec<f32>) {
        self.curves.retain(|c| c.key != k);
        if points.iter().any(|p| p.abs() > 1e-4) {
            self.curves.push(Curve { key: k, points });
            self.curves.sort_by_key(|c| ShapeKey::ALL.iter().position(|&a| a == c.key));
        }
    }

    /// A control's value at a beat from the clip start: base plus curve, 0..1.
    /// Chord controls hold each beat's value; the others pass smoothly (without
    /// overshoot) from beat to beat.
    pub fn at(&self, k: ShapeKey, beat: f64) -> f32 {
        let base = self.norm(k);
        let Some(c) = self.curve(k) else { return base };
        let n = c.len() - 1;
        let x = beat.clamp(0.0, n as f64);
        let y = |i: usize| (base + c[i]).clamp(0.0, 1.0);
        if k.is_harmony() {
            return y((x.floor() as usize).min(n));
        }
        let i = (x.floor() as usize).min(n - 1);
        let t = (x - i as f64) as f32;
        if t == 0.0 {
            return y(i);
        }
        let slope = |a: usize, b: usize| y(b) - y(a);
        let tangent = |j: usize| {
            let l = if j > 0 { slope(j - 1, j) } else { slope(j, j + 1) };
            let r = if j < n { slope(j, j + 1) } else { l };
            if l * r <= 0.0 { 0.0 } else { 2.0 * l * r / (l + r) }
        };
        let (p0, p1, m0, m1) = (y(i), y(i + 1), tangent(i), tangent(i + 1));
        let (t2, t3) = (t * t, t * t * t);
        ((2.0 * t3 - 3.0 * t2 + 1.0) * p0 + (t3 - 2.0 * t2 + t) * m0 + (-2.0 * t3 + 3.0 * t2) * p1 + (t3 - t2) * m1).clamp(0.0, 1.0)
    }

    /// Whether a control does anything anywhere (base off neutral, or a curve).
    pub fn active(&self, k: ShapeKey, neutral: &Shape) -> bool {
        self.curve(k).is_some() || (self.norm(k) - neutral.norm(k)).abs() > 1e-4
    }

    pub fn index_at(&self, k: ShapeKey, beat: f64) -> usize {
        k.index_of(self.at(k, beat))
    }
}

/// A clip's Shape for one source: a part's patterns (\`part\` Some) or the kept notes (None).
/// Each part keeps its own; switching parts plays the other part's Shape.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShapeSlot {
    pub part: Option<PartType>,
    pub shape: Shape,
}

fn beat(t: Ticks) -> f64 {
    t as f64 / PPQ as f64
}

fn polyphonic(notes: &[Note]) -> bool {
    let mut v: Vec<&Note> = notes.iter().collect();
    v.sort_by_key(|n| (n.start, n.pitch));
    v.windows(2).any(|w| w[1].start == w[0].start || w[1].start + PPQ / 8 < w[0].start + w[0].length)
}

/// Whether Voices has anything to add: a single line that is not a bass line or chords.
pub fn has_voices(notes: &[Note], part: Option<PartType>) -> bool {
    !matches!(part, Some(PartType::Bass | PartType::Chords)) && !notes.is_empty() && !polyphonic(notes)
}

/// Whether the chord controls (Air, Sub, Spread, Tension, Inversion) apply: chords,
/// or kept notes that play several at once.
pub fn has_harmony(notes: &[Note], part: Option<PartType>) -> bool {
    match part {
        Some(p) => p == PartType::Chords,
        None => polyphonic(notes),
    }
}

/// Deterministic 0..1 noise for Vary and the random arp.
fn noise(i: u64) -> f64 {
    let mut z = i.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    ((z ^ (z >> 31)) >> 11) as f64 / (1u64 << 53) as f64
}

/// Where an onset falls in the drop order: 0, 1/2, 1/4, 3/4… (bits reversed), so
/// thinning keeps the onsets evenly spread.
fn spread_rank(i: usize) -> f64 {
    let (mut r, mut f, mut i) = (0.0, 0.5, i);
    while i > 0 {
        r += (i & 1) as f64 * f;
        i >>= 1;
        f /= 2.0;
    }
    r
}

/// The notes as the Shape plays them, sorted and numbered from 1. A neutral Shape
/// returns them unchanged.
pub fn apply(notes: &[Note], shape: &Shape, part: Option<PartType>, key: Option<ProjectKey>, clip: Ticks) -> Vec<Note> {
    let neutral = Shape::neutral(part);
    if *shape == neutral {
        return notes.to_vec();
    }
    let on = |k: ShapeKey| shape.active(k, &neutral);
    let mut v = notes.to_vec();
    v.sort_by_key(|n| (n.start, n.pitch));
    let scale = scale_of(&v, key);
    if ShapeKey::ALL.iter().any(|&k| k.is_harmony() && on(k)) && has_harmony(&v, part) {
        v = harmonise(&v, shape, scale);
    }
    if on(ShapeKey::Density) || on(ShapeKey::Vary) {
        v = thin(&v, shape, part == Some(PartType::Chords) || (part.is_none() && polyphonic(&v)));
    }
    let base_vel = neutral_velocity(part);
    for n in &mut v {
        let b = beat(n.start);
        n.pitch = fold(n.pitch as i32 + 12 * (shape.index_at(ShapeKey::Octave, b) as i32 - MAX_OCTAVE as i32));
        let vel = n.velocity as f32 * shape.at(ShapeKey::Velocity, b) / base_vel;
        n.velocity = vel.round().clamp(6.0, 127.0) as u8;
    }
    if on(ShapeKey::Voices) && has_voices(&v, part) {
        v = add_voices(&v, shape, scale);
    }
    let arp = on(ShapeKey::Arp);
    if arp {
        v = arpeggiate(&v, shape);
    }
    if on(ShapeKey::Length) && !arp {
        set_lengths(&mut v, shape, clip);
    }
    if on(ShapeKey::Repeat) && !arp {
        v = repeat(&v, shape);
    }
    if on(ShapeKey::Nudge) {
        for n in &mut v {
            let shift = (shape.at(ShapeKey::Nudge, beat(n.start)) as f64 - 0.5) * 2.0 * NUDGE_BEATS * PPQ as f64;
            n.start = nudged(n.start, shift.round() as Ticks, clip);
        }
    }
    v.retain(|n| n.start < clip.max(1));
    midi::sort_notes(&mut v);
    v.dedup_by(|b, a| a.start == b.start && a.pitch == b.pitch);
    for (i, n) in v.iter_mut().enumerate() {
        n.id = i as u32 + 1;
    }
    v
}

/// A pitch moved back by octaves into the MIDI range, so Octave never loses a note.
fn fold(mut p: i32) -> u8 {
    while p > 127 {
        p -= 12;
    }
    while p < 0 {
        p += 12;
    }
    p as u8
}

/// A start moved by Nudge, kept inside the clip when it was inside.
fn nudged(start: Ticks, shift: Ticks, clip: Ticks) -> Ticks {
    if start < clip { (start + shift).clamp(0, clip - 1) } else { (start + shift).max(0) }
}

fn onsets(v: &[Note]) -> BTreeMap<Ticks, Vec<usize>> {
    let mut m: BTreeMap<Ticks, Vec<usize>> = BTreeMap::new();
    for (i, n) in v.iter().enumerate() {
        m.entry(n.start).or_default().push(i);
    }
    m
}

/// Density and Vary. Single notes (and, outside chords, whole onsets) stay while
/// their place in an even spread is under the density; Vary blends that place with
/// noise. A chord keeps its lowest and highest notes first, then the inner ones from
/// the top. Something always stays.
fn thin(v: &[Note], shape: &Shape, chords: bool) -> Vec<Note> {
    let groups: Vec<(Ticks, Vec<usize>)> = onsets(v).into_iter().collect();
    let mut keep = vec![false; v.len()];
    let mut fallback: Option<(f64, usize)> = None;
    let mut order: Vec<usize> = (0..groups.len()).collect();
    order.sort_by(|&a, &b| spread_rank(a).total_cmp(&spread_rank(b)));
    let mut rank = vec![0.0; groups.len()];
    for (pos, &g) in order.iter().enumerate() {
        rank[g] = if groups.len() > 1 { pos as f64 / groups.len() as f64 } else { 0.0 };
    }
    for (g, (start, idx)) in groups.iter().enumerate() {
        let b = beat(*start);
        let density = shape.at(ShapeKey::Density, b) as f64;
        let vary = shape.at(ShapeKey::Vary, b) as f64;
        if !chords || idx.len() == 1 {
            let threshold = MIN_DENSITY + density * (1.0 - MIN_DENSITY);
            let score = rank[g] * (1.0 - vary) + noise(g as u64 + 1013) * vary;
            if fallback.is_none_or(|(s, _)| score < s) {
                fallback = Some((score, g));
            }
            if density >= 1.0 || score < threshold {
                for &i in idx {
                    keep[i] = true;
                }
            }
            continue;
        }
        let mut sorted = idx.clone();
        sorted.sort_by_key(|&i| v[i].pitch);
        let last = sorted.len() - 1;
        let outer_first: Vec<usize> = [sorted[0], sorted[last]].into_iter().chain(sorted[1..last].iter().rev().copied()).collect();
        let mut scored: Vec<(f64, usize)> = outer_first
            .iter()
            .enumerate()
            .map(|(j, &i)| ((1.0 - vary) * (j as f64 / outer_first.len() as f64) + vary * noise((g * 31 + j) as u64 + 1013), i))
            .collect();
        scored.sort_by(|a, b| a.0.total_cmp(&b.0));
        let n = ((density * sorted.len() as f64).ceil() as usize).max(1);
        for &(_, i) in scored.iter().take(n) {
            keep[i] = true;
        }
    }
    if !keep.contains(&true)
        && let Some((_, g)) = fallback
    {
        for &i in &groups[g].1 {
            keep[i] = true;
        }
    }
    v.iter().zip(keep).filter(|(_, k)| *k).map(|(n, _)| *n).collect()
}

type Scale = (i32, &'static [i32; 7]);

/// The scale the notes fit best: the project key, else the major or minor key
/// holding most of them (ties go to the key of the first note).
fn scale_of(v: &[Note], key: Option<ProjectKey>) -> Scale {
    const MAJOR: [i32; 7] = [0, 2, 4, 5, 7, 9, 11];
    const MINOR: [i32; 7] = [0, 2, 3, 5, 7, 8, 10];
    if let Some(k) = key {
        return (k.root as i32 % 12, if k.minor { &MINOR } else { &MAJOR });
    }
    let first = v.first().map_or(0, |n| n.pitch as i32 % 12);
    let mut best = (0, (0, &MAJOR));
    for root in 0..12 {
        for steps in [&MAJOR, &MINOR] {
            let fit = v.iter().filter(|n| steps.contains(&((n.pitch as i32 - root).rem_euclid(12)))).count() * 2 + usize::from(root == first);
            if fit > best.0 {
                best = (fit, (root, steps));
            }
        }
    }
    best.1
}

/// Scale degree of a pitch (the scale tone at or under it) and how far above that tone it sits.
fn degree(p: i32, scale: Scale) -> (i32, i32) {
    let (root, steps) = scale;
    let rel = p - root;
    let step = steps.iter().rposition(|&s| s <= rel.rem_euclid(12)).unwrap_or(0) as i32;
    let deg = 7 * rel.div_euclid(12) + step;
    (deg, p - pitch_of(deg, scale))
}

fn pitch_of(deg: i32, (root, steps): Scale) -> i32 {
    root + steps[deg.rem_euclid(7) as usize] + 12 * deg.div_euclid(7)
}

/// Voices: a third under each note, then a fifth, in the scale.
fn add_voices(v: &[Note], shape: &Shape, scale: Scale) -> Vec<Note> {
    let mut out = v.to_vec();
    for n in v {
        let voices = shape.index_at(ShapeKey::Voices, beat(n.start)) as i32 + 1;
        let (deg, off) = degree(n.pitch as i32, scale);
        for k in 1..voices {
            let p = pitch_of(deg - 2 * k, scale) + off;
            if (0..=127).contains(&p) {
                out.push(Note { pitch: p as u8, velocity: ((n.velocity as u32 * 7) / 8).max(1) as u8, ..*n });
            }
        }
    }
    out
}

/// Air, Sub, Tension, Inversion and Spread, beat by beat: each beat's notes are one
/// chord. Air and Sub add the chord's own tones above and below it; Tension stacks
/// scale thirds on top; Inversion lifts the lowest notes an octave; Spread moves the
/// outer notes apart by octaves. New notes take the timing of the chord's notes in order.
fn harmonise(v: &[Note], shape: &Shape, scale: Scale) -> Vec<Note> {
    let mut beats: BTreeMap<i64, Vec<Note>> = BTreeMap::new();
    for n in v {
        beats.entry(n.start.div_euclid(PPQ)).or_default().push(*n);
    }
    let mut out = Vec::with_capacity(v.len());
    for (b, mut notes) in beats {
        notes.sort_by_key(|n| n.pitch);
        let at = |k: ShapeKey| shape.at(k, b as f64);
        let mut keys: Vec<i32> = notes.iter().map(|n| n.pitch as i32).collect();
        let pcs: Vec<i32> = keys.iter().map(|k| k.rem_euclid(12)).collect();
        let extend = |keys: &mut Vec<i32>, count: usize, up: bool| {
            let dir = if up { 1 } else { -1 };
            let mut p = if up { *keys.last().unwrap() } else { keys[0] };
            for _ in 0..count {
                p += dir;
                while (0..=127).contains(&p) && !pcs.contains(&p.rem_euclid(12)) {
                    p += dir;
                }
                if !(0..=127).contains(&p) {
                    break;
                }
                keys.push(p);
            }
            keys.sort();
        };
        extend(&mut keys, (at(ShapeKey::Air) * 3.0).round() as usize, true);
        extend(&mut keys, (at(ShapeKey::Sub) * 3.0).round() as usize, false);
        let tension = ((at(ShapeKey::Tension) * 5.0) as usize).min(4);
        let mut top = *keys.last().unwrap();
        for _ in 0..tension {
            top = pitch_of(degree(top, scale).0 + 2, scale);
            keys.push(top);
        }
        let inversion = ShapeKey::Inversion.index_of(at(ShapeKey::Inversion)).min(keys.len().saturating_sub(1));
        for _ in 0..inversion {
            keys.sort();
            keys[0] += 12;
        }
        keys.sort();
        let spread = at(ShapeKey::Spread);
        if keys.len() > 2 && spread > 0.0 {
            let mid = (keys.len() - 1) as f32 / 2.0;
            for (i, k) in keys.iter_mut().enumerate() {
                *k += 12 * (spread * (i as f32 - mid)).round().clamp(-2.0, 2.0) as i32;
            }
        }
        keys.retain(|k| (0..=127).contains(k));
        keys.sort();
        keys.dedup();
        for (i, k) in keys.into_iter().enumerate() {
            out.push(Note { pitch: k as u8, ..*notes.get(i).unwrap_or(&notes[0]) });
        }
    }
    out
}

/// Arp: each chord (or note) plays its notes one per step in the pattern, across the
/// octave range, until the next chord. Steps sit on the clip's step grid, so the
/// pattern keeps its place across chord changes.
fn arpeggiate(v: &[Note], shape: &Shape) -> Vec<Note> {
    let groups: Vec<(Ticks, Vec<usize>)> = onsets(v).into_iter().collect();
    let mut out = Vec::new();
    for (g, (start, idx)) in groups.iter().enumerate() {
        let b = beat(*start);
        let rate = ARP_BEATS[shape.index_at(ShapeKey::Arp, b)];
        let mut end = idx.iter().map(|&i| v[i].start + v[i].length).max().unwrap_or(*start);
        if let Some((next, _)) = groups.get(g + 1) {
            end = end.min(*next);
        }
        if rate == 0.0 {
            out.extend(idx.iter().map(|&i| v[i]));
            continue;
        }
        let step = ((rate * PPQ as f64).round() as Ticks).max(1);
        let mut chord: Vec<(i32, u8)> = idx.iter().map(|&i| (v[i].pitch as i32, v[i].velocity)).collect();
        chord.sort();
        chord.dedup_by_key(|p| p.0);
        let pattern = ARP_PATTERNS[shape.index_at(ShapeKey::ArpPattern, b)];
        let octaves = if pattern == "Chord" { 1 } else { ARP_OCTAVES[shape.index_at(ShapeKey::ArpOctaves, b)] as i32 };
        let pool: Vec<(i32, u8)> = (0..octaves).flat_map(|o| chord.iter().map(move |&(p, vel)| (p + 12 * o, vel))).filter(|p| p.0 <= 127).collect();
        let m = pool.len();
        if m == 0 {
            continue;
        }
        let len = ((step as f64 * GATE).round() as Ticks).max(1);
        let mut t = (start + step - 1).div_euclid(step) * step;
        if t >= end {
            t = *start;
        }
        while t < end {
            let k = (t / step) as usize;
            let picks: Vec<(i32, u8)> = match pattern {
                "Up" => vec![pool[k % m]],
                "Down" => vec![pool[m - 1 - k % m]],
                "Up & down" => {
                    let cycle = if m < 3 { m } else { 2 * m - 2 };
                    let j = k % cycle;
                    vec![pool[if j < m { j } else { cycle - j }]]
                }
                "Random" => {
                    let cycle = k / m;
                    let mut perm: Vec<usize> = (0..m).collect();
                    perm.sort_by(|&a, &b| noise((cycle * 131 + a) as u64).total_cmp(&noise((cycle * 131 + b) as u64)));
                    vec![pool[perm[k % m]]]
                }
                "Thumb" => {
                    let j = k % (2 * (m - 1)).max(1);
                    vec![if m < 2 || j % 2 == 0 { pool[0] } else { pool[1 + j / 2] }]
                }
                "Pinky" => {
                    let j = k % (2 * (m - 1)).max(1);
                    vec![if m < 2 { pool[0] } else if j % 2 == 0 { pool[j / 2] } else { pool[m - 1] }]
                }
                _ => chord.clone(),
            };
            for (p, vel) in picks {
                out.push(Note { id: 0, pitch: p as u8, start: t, length: len.min(end - t), velocity: vel });
            }
            t += step;
        }
    }
    out
}

/// Length under 0.5 shortens each note toward MIN_TICKS; over 0.5 holds it toward
/// the later of the next onset and the next note of its pitch (at most MAX_HOLD and
/// the clip end), and joins notes of one pitch that then touch. Notes that repeat
/// keep their span: Length sets their retriggers' gate instead.
fn set_lengths(v: &mut Vec<Note>, shape: &Shape, clip: Ticks) {
    let starts: Vec<Ticks> = onsets(v).into_keys().collect();
    let snapshot = v.clone();
    for n in v.iter_mut() {
        let b = beat(n.start);
        if shape.index_at(ShapeKey::Repeat, b) > 0 {
            continue;
        }
        let t = shape.at(ShapeKey::Length, b) as f64;
        let l = n.length.max(1) as f64;
        let new = if t < 0.5 {
            (l * t / 0.5).max(MIN_TICKS as f64)
        } else {
            let next_onset = starts.iter().copied().find(|&s| s > n.start).unwrap_or(clip.max(n.start + n.length)) - n.start;
            let next_same = snapshot.iter().filter(|o| o.pitch == n.pitch && o.start > n.start).map(|o| o.start - MIN_TICKS).min().unwrap_or(clip);
            let hold = (next_same.min(clip).min(n.start + MAX_HOLD) - n.start).max(0);
            let target = l.max(next_onset as f64).max(hold as f64);
            l * (target / l).powf((t - 0.5) / 0.5)
        };
        n.length = (new.round() as Ticks).max(1);
    }
    let mut by_pitch: Vec<Note> = std::mem::take(v);
    by_pitch.sort_by_key(|n| (n.pitch, n.start));
    for n in by_pitch {
        if let Some(last) = v.last_mut()
            && last.pitch == n.pitch
            && shape.at(ShapeKey::Length, beat(last.start)) > 0.5
            && last.start + last.length >= n.start - JOIN_TICKS
        {
            last.length = last.length.max(n.start + n.length - last.start);
            continue;
        }
        v.push(n);
    }
}

/// Repeat: each note breaks into retriggers at its rate inside its own length; the
/// gate follows Length.
fn repeat(v: &[Note], shape: &Shape) -> Vec<Note> {
    let mut out = Vec::with_capacity(v.len());
    for n in v {
        let b = beat(n.start);
        let rate = REPEAT_BEATS[shape.index_at(ShapeKey::Repeat, b)];
        if rate == 0.0 {
            out.push(*n);
            continue;
        }
        let step = ((rate * PPQ as f64).round() as Ticks).max(1);
        let l = shape.at(ShapeKey::Length, b) as f64;
        let gate = if l <= 0.5 { MIN_GATE + (GATE - MIN_GATE) * l / 0.5 } else { GATE + (1.0 - GATE) * (l - 0.5) / 0.5 };
        let len = ((step as f64 * gate).round() as Ticks).max(1);
        let end = n.start + n.length;
        let mut t = n.start;
        while t < end {
            out.push(Note { start: t, length: len.min(end - t), ..*n });
            t += step;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(pitch: u8, beat: f64, beats: f64) -> Note {
        Note { id: 0, pitch, start: (beat * PPQ as f64) as Ticks, length: (beats * PPQ as f64) as Ticks, velocity: 80 }
    }

    fn numbered(mut v: Vec<Note>) -> Vec<Note> {
        for (i, n) in v.iter_mut().enumerate() {
            n.id = i as u32 + 1;
        }
        v
    }

    fn chord() -> Vec<Note> {
        let mut v: Vec<Note> = [60, 64, 67, 71].iter().map(|&p| note(p, 0.0, 4.0)).collect();
        v.extend([60, 63, 67].iter().map(|&p| note(p, 4.0, 4.0)));
        numbered(v)
    }

    fn line() -> Vec<Note> {
        numbered((0..8).map(|i| note(72 + (i % 3) as u8 * 2, i as f64 * 0.5, 0.5)).collect())
    }

    const BAR2: Ticks = 8 * PPQ;
    const C_MAJOR: Option<ProjectKey> = Some(ProjectKey { root: 0, minor: false });
    const CHORDS: Option<PartType> = Some(PartType::Chords);

    fn with(part: Option<PartType>, f: impl FnOnce(&mut Shape)) -> Shape {
        let mut s = Shape::neutral(part);
        f(&mut s);
        s
    }

    fn pitches_at(v: &[Note], beat: f64) -> Vec<u8> {
        v.iter().filter(|n| n.start == (beat * PPQ as f64) as Ticks).map(|n| n.pitch).collect()
    }

    #[test]
    fn neutral_shape_plays_the_notes_as_made() {
        for part in [None, CHORDS, Some(PartType::Bass), Some(PartType::Lead)] {
            assert_eq!(apply(&chord(), &Shape::neutral(part), part, None, BAR2), chord());
            assert!(Shape::neutral(part).validate().is_ok());
        }
    }

    #[test]
    fn every_control_round_trips_through_its_value() {
        let mut s = Shape::neutral(None);
        for k in ShapeKey::ALL {
            assert_eq!(k.option_count(), k.options().len().max(1), "{k:?}");
            for i in 0..k.options().len() {
                s.set_norm(k, k.value_of(i));
                assert_eq!(s.index_at(k, 0.0), i, "{k:?} option {i}");
            }
            s.set_norm(k, 0.3);
            assert!(s.validate().is_ok(), "{k:?}");
        }
    }

    #[test]
    fn octave_velocity_and_nudge() {
        let s = with(None, |s| (s.octave, s.velocity, s.nudge) = (-1, 0.4, 0.75));
        let out = apply(&chord(), &s, None, None, BAR2);
        assert_eq!(out.len(), 7);
        assert!(out.iter().all(|n| n.velocity == 40 && n.start % (4 * PPQ) == PPQ / 4));
        assert_eq!(out[0].pitch, 48);
        midi::validate_notes(&out).unwrap();
    }

    #[test]
    fn density_keeps_outer_chord_notes_and_an_even_spread_of_onsets() {
        let out = apply(&chord(), &with(CHORDS, |s| s.density = 0.5), CHORDS, None, BAR2);
        assert_eq!(pitches_at(&out, 0.0), vec![60, 71]);
        let half = apply(&line(), &with(None, |s| s.density = 0.5), None, None, BAR2);
        assert_eq!(half.iter().map(|n| n.start / (PPQ / 2)).collect::<Vec<_>>(), vec![0, 1, 2, 4, 6]);
        let none = apply(&line(), &with(None, |s| s.density = 0.0), None, None, BAR2);
        assert_eq!(none.iter().map(|n| n.start / (PPQ / 2)).collect::<Vec<_>>(), vec![0, 4], "the floor keeps 15 %");
    }

    #[test]
    fn vary_changes_which_notes_stay_but_not_how_many_chord_notes() {
        let even = apply(&chord(), &with(CHORDS, |s| s.density = 0.5), CHORDS, None, BAR2);
        let varied = apply(&chord(), &with(CHORDS, |s| (s.density, s.vary) = (0.5, 1.0)), CHORDS, None, BAR2);
        assert_eq!(even.len(), varied.len());
        let a = apply(&line(), &with(None, |s| (s.density, s.vary) = (0.5, 1.0)), None, None, BAR2);
        assert_eq!(a, apply(&line(), &with(None, |s| (s.density, s.vary) = (0.5, 1.0)), None, None, BAR2), "deterministic");
    }

    #[test]
    fn length_shortens_and_holds_to_the_next_note() {
        let short = apply(&line(), &with(None, |s| s.length = 0.0), None, None, BAR2);
        assert!(short.iter().all(|n| n.length == MIN_TICKS));
        let gaps = numbered([60, 62, 64, 65].iter().enumerate().map(|(i, &p)| note(p, i as f64, 0.25)).collect());
        let half = apply(&gaps, &with(None, |s| s.length = 0.75), None, None, 4 * PPQ);
        assert!(half[0].length > PPQ / 4 && half[0].length < 4 * PPQ);
        let held = apply(&gaps, &with(None, |s| s.length = 1.0), None, None, 4 * PPQ);
        assert_eq!(held.iter().map(|n| n.length / PPQ).collect::<Vec<_>>(), vec![4, 3, 2, 1], "held to the clip end, at most 4 beats");
        let same = numbered((0..4).map(|i| note(60, i as f64, 0.25)).collect());
        let joined = apply(&same, &with(None, |s| s.length = 1.0), None, None, 4 * PPQ);
        assert_eq!(joined.len(), 1, "held notes of one pitch join");
        assert_eq!(joined[0].length, 4 * PPQ);
    }

    #[test]
    fn repeat_retriggers_inside_each_note_and_chords_repeat_as_chords() {
        let out = apply(&chord(), &with(None, |s| s.repeat = 2), None, None, BAR2);
        assert_eq!(out.len(), 7 * 8);
        assert_eq!(pitches_at(&out, 0.5).len(), 4);
        assert!(out.iter().all(|n| n.length == (PPQ as f64 * 0.5 * GATE).round() as Ticks));
        let tight = apply(&chord(), &with(None, |s| (s.repeat, s.length) = (2, 0.0)), None, None, BAR2);
        assert!(tight.iter().all(|n| n.length == (PPQ as f64 * 0.5 * MIN_GATE).round() as Ticks), "the gate follows Length");
    }

    #[test]
    fn arp_patterns_keep_their_place_on_the_step_grid() {
        let out = apply(&chord(), &with(CHORDS, |s| s.arp = 3), CHORDS, None, BAR2);
        assert_eq!(out.len(), 32, "a sixteenth arp over two bars");
        let first: Vec<u8> = out.iter().take(9).map(|n| n.pitch).collect();
        assert_eq!(first, vec![60, 64, 67, 71, 72, 76, 79, 83, 79], "up and down over two octaves");
        assert_eq!(out[16].pitch, 75, "step 16 of the second chord's cycle");
        let up = apply(&chord(), &with(CHORDS, |s| (s.arp, s.arp_pattern, s.arp_octaves) = (3, 0, 0)), CHORDS, None, BAR2);
        assert_eq!(up.iter().take(5).map(|n| n.pitch).collect::<Vec<_>>(), vec![60, 64, 67, 71, 60]);
        let thumb = apply(&chord(), &with(CHORDS, |s| (s.arp, s.arp_pattern, s.arp_octaves) = (3, 4, 0)), CHORDS, None, BAR2);
        assert_eq!(thumb.iter().take(6).map(|n| n.pitch).collect::<Vec<_>>(), vec![60, 64, 60, 67, 60, 71]);
        let whole = apply(&chord(), &with(CHORDS, |s| (s.arp, s.arp_pattern) = (3, 6)), CHORDS, None, BAR2);
        assert_eq!(whole.len(), 16 * 4 + 16 * 3, "Chord plays the whole chord on every step");
        let random = apply(&chord(), &with(CHORDS, |s| (s.arp, s.arp_pattern, s.arp_octaves) = (3, 3, 0)), CHORDS, None, BAR2);
        let mut cycle: Vec<u8> = random.iter().take(4).map(|n| n.pitch).collect();
        cycle.sort();
        assert_eq!(cycle, vec![60, 64, 67, 71], "each cycle plays every note once");
    }

    #[test]
    fn voices_harmonise_a_single_line_in_key() {
        let lead = Some(PartType::Lead);
        assert!(has_voices(&line(), lead));
        assert!(!has_voices(&chord(), lead) && !has_voices(&line(), Some(PartType::Bass)));
        let out = apply(&line(), &with(lead, |s| s.voices = 3), lead, C_MAJOR, BAR2);
        assert_eq!(out.len(), 24);
        assert_eq!(pitches_at(&out, 0.0), vec![65, 69, 72], "C5 gets A4 and F4 under it in C major");
        let chords = apply(&chord(), &with(None, |s| s.voices = 3), None, C_MAJOR, BAR2);
        assert_eq!(chords.len(), 7, "chords have no extra voices to add");
    }

    #[test]
    fn chord_controls_add_air_sub_tension_and_reshape_the_voicing() {
        assert!(has_harmony(&chord(), CHORDS) && has_harmony(&chord(), None) && !has_harmony(&line(), None) && !has_harmony(&chord(), Some(PartType::Lead)));
        let air = apply(&chord(), &with(CHORDS, |s| s.air = 1.0 / 3.0), CHORDS, C_MAJOR, BAR2);
        assert_eq!(pitches_at(&air, 0.0), vec![60, 64, 67, 71, 72]);
        let sub = apply(&chord(), &with(CHORDS, |s| s.sub = 2.0 / 3.0), CHORDS, C_MAJOR, BAR2);
        assert_eq!(pitches_at(&sub, 0.0), vec![55, 59, 60, 64, 67, 71]);
        let tension = apply(&chord(), &with(CHORDS, |s| s.tension = 0.4), CHORDS, C_MAJOR, BAR2);
        assert_eq!(pitches_at(&tension, 0.0), vec![60, 64, 67, 71, 74, 77]);
        let inv = apply(&chord(), &with(CHORDS, |s| s.inversion = 1), CHORDS, C_MAJOR, BAR2);
        assert_eq!(pitches_at(&inv, 0.0), vec![64, 67, 71, 72]);
        let spread = apply(&chord(), &with(CHORDS, |s| s.spread = 1.0), CHORDS, C_MAJOR, BAR2);
        assert_eq!(pitches_at(&spread, 0.0), vec![36, 52, 79, 95]);
        let lead = apply(&line(), &with(Some(PartType::Lead), |s| s.air = 1.0), Some(PartType::Lead), C_MAJOR, BAR2);
        assert_eq!(lead.len(), 8, "a single line has no chord to reshape");
        midi::validate_notes(&spread).unwrap();
    }

    #[test]
    fn curves_add_to_the_base_beat_by_beat() {
        let mut s = Shape::neutral(None);
        let mut points = vec![0.0; curve_len(BAR2)];
        points[0] = -0.4;
        s.set_curve(ShapeKey::Velocity, points);
        assert!((s.at(ShapeKey::Velocity, 0.0) - 0.4).abs() < 1e-6 && (s.at(ShapeKey::Velocity, 4.0) - 0.8).abs() < 1e-6);
        let mid = s.at(ShapeKey::Velocity, 0.5);
        assert!(mid > 0.4 && mid < 0.8, "smooth between beats: {mid}");
        let out = apply(&chord(), &s, None, None, BAR2);
        assert!(pitches_at(&out, 0.0).len() == 4 && out.iter().filter(|n| n.start == 0).all(|n| n.velocity == 40));
        assert!(out.iter().filter(|n| n.start == 4 * PPQ).all(|n| n.velocity == 80));
        let mut air = vec![0.0; curve_len(BAR2)];
        air[4] = 1.0 / 3.0;
        s.set_curve(ShapeKey::Air, air);
        assert_eq!(s.at(ShapeKey::Air, 4.9), 1.0 / 3.0, "chord controls hold each beat");
        assert_eq!(s.at(ShapeKey::Air, 5.0), 0.0);
        let out = apply(&chord(), &s, None, C_MAJOR, BAR2);
        assert_eq!(pitches_at(&out, 4.0), vec![60, 63, 67, 72], "Air only on beat 5");
        s.set_curve(ShapeKey::Air, vec![0.0; curve_len(BAR2)]);
        assert!(s.curve(ShapeKey::Air).is_none(), "a flat curve is dropped");
    }

    #[test]
    fn old_shapes_load_with_neutral_new_controls() {
        let json = r#"{"octave":1,"voices":1,"density":1.0,"length":0.5,"velocity":0.8,"nudge":0.5,"repeat":0,"arp":0}"#;
        let s: Shape = serde_json::from_str(json).unwrap();
        assert_eq!(s, with(None, |s| s.octave = 1));
    }

    #[test]
    fn octave_and_nudge_never_lose_notes() {
        let top = numbered(vec![note(120, 3.75, 0.25)]);
        let out = apply(&top, &with(None, |s| (s.octave, s.nudge) = (2, 1.0)), None, None, 4 * PPQ);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].pitch, 120, "folded back an octave at a time into range");
        assert!(out[0].start < 4 * PPQ);
    }

    #[test]
    fn out_of_range_values_are_refused() {
        let n = Shape::neutral(None);
        let bad_curve = |points: Vec<f32>| Shape { curves: vec![Curve { key: ShapeKey::Density, points }], ..n.clone() };
        for bad in [
            Shape { octave: 3, ..n.clone() },
            Shape { voices: 0, ..n.clone() },
            Shape { density: 1.5, ..n.clone() },
            Shape { nudge: f32::NAN, ..n.clone() },
            Shape { repeat: 5, ..n.clone() },
            Shape { arp: 6, ..n.clone() },
            Shape { arp_pattern: 7, ..n.clone() },
            Shape { inversion: 4, ..n.clone() },
            bad_curve(vec![0.0]),
            bad_curve(vec![0.0, 2.0]),
        ] {
            assert!(bad.validate().is_err(), "{bad:?}");
        }
    }
}
