//! Saine's pattern library: the parts the clip sidebar's **Create** button makes.
//!
//! The reference app asks an online composer and catalogue services for
//! these. Saine calls no service: every progression, rhythm and groove below was
//! written for Saine, and the sets are generated from them deterministically
//! (the same part, genre, key and set number always give the same six patterns).
//!
//! A MIDI part (Chords, Bass, Lead, Arp) is a chord progression in the project
//! key (C minor or C major when the project has none) played in one of the
//! part's styles. A drum part is one of the genre's grooves on the Drum Rack
//! pads (Pad 1 kick, 2 snare, 3 closed hat, 4 open hat, 5 clap, 6 percussion),
//! with a variation per pattern.

use crate::midi::{self, Note};
use crate::model::ProjectKey;
use crate::shape::{Shape, ShapeSlot};
use crate::steps::{StepLane, StepPattern, StepRate, StepRows};
use crate::{PPQ, Ticks};
use serde::{Deserialize, Serialize};

/// What the Type field picks (reference order).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum PartType {
    #[default]
    Chords,
    Bass,
    Lead,
    Arp,
}

impl PartType {
    pub const ALL: [PartType; 4] = [PartType::Chords, PartType::Bass, PartType::Lead, PartType::Arp];

    pub fn label(self) -> &'static str {
        match self {
            PartType::Chords => "Chords",
            PartType::Bass => "Bass",
            PartType::Lead => "Lead",
            PartType::Arp => "Arp",
        }
    }

    /// What the Create pill says it makes ("Create basslines", then "Create new basslines").
    pub fn plural(self) -> &'static str {
        match self {
            PartType::Chords => "chords",
            PartType::Bass => "basslines",
            PartType::Lead => "leads",
            PartType::Arp => "arps",
        }
    }
}

/// Genre field entries; 0 is Any (the library's neutral feel), then the reference's
/// 21 genres in its (alphabetical) order. Saved clips store the index, so new
/// entries only ever go at the end.
pub const GENRES: [&str; 22] = [
    "Any",
    "Afrobeats",
    "Ambient",
    "Drill",
    "Drum & bass",
    "EDM",
    "Electronic",
    "Experimental",
    "Folk",
    "Future bass",
    "Hip-hop",
    "House",
    "Indie",
    "Lo-fi",
    "Neo-soul",
    "Pop",
    "R&B",
    "Reggaeton",
    "Singer-songwriter",
    "Techno",
    "Trap",
    "UK garage",
];

/// Patterns one Create makes.
pub const SET_SIZE: usize = 6;

/// One pattern of a set: its name and its content (notes, plus the step grid for drums).
#[derive(Debug, Clone, PartialEq)]
pub struct LibPattern {
    pub name: String,
    pub notes: Vec<Note>,
    pub pattern: Option<StepPattern>,
}

/// How a genre plays: scale when the project has no key, note grid, push of
/// the off-beats, how busy the parts are (0 sparse .. 2 busy), the octave
/// the parts sit in, and which progressions it uses.
struct Feel {
    minor: bool,
    grid: Ticks,
    swing: f64,
    density: u8,
    octave: i32,
    progressions: &'static [usize],
}

const S16: Ticks = PPQ / 4;
const S8: Ticks = PPQ / 2;

fn feel(genre: usize) -> Feel {
    let f = |minor, grid, swing, density, octave, progressions| Feel { minor, grid, swing, density, octave, progressions };
    match GENRES.get(genre).copied().unwrap_or("Any") {
        "Afrobeats" => f(false, S16, 0.12, 2, 0, &[1, 3, 6]),
        "Ambient" => f(false, S8, 0.0, 0, 0, &[4, 5, 7]),
        "Drill" => f(true, S16, 0.0, 1, -1, &[0, 2, 7]),
        "Drum & bass" => f(true, S16, 0.0, 2, 0, &[0, 2, 5]),
        "EDM" => f(true, S16, 0.0, 2, 0, &[0, 1, 3]),
        "Electronic" => f(true, S16, 0.0, 1, 0, &[0, 5, 7]),
        "Experimental" => f(true, S16, 0.0, 1, 0, &[2, 5, 7]),
        "Folk" => f(false, S8, 0.0, 1, 0, &[1, 4, 6]),
        "Future bass" => f(false, S16, 0.0, 2, 0, &[1, 3, 4]),
        "Hip-hop" => f(true, S16, 0.18, 1, -1, &[0, 2, 6]),
        "House" => f(true, S16, 0.0, 2, 0, &[1, 3, 4]),
        "Indie" => f(false, S8, 0.0, 1, 0, &[1, 3, 7]),
        "Lo-fi" => f(false, S16, 0.2, 0, -1, &[5, 4, 7]),
        "Neo-soul" => f(false, S16, 0.15, 1, 0, &[5, 7, 3]),
        "Pop" => f(false, S8, 0.0, 1, 0, &[1, 3, 4]),
        "R&B" => f(true, S16, 0.1, 1, 0, &[5, 2, 6]),
        "Reggaeton" => f(true, S16, 0.0, 2, 0, &[0, 6, 2]),
        "Singer-songwriter" => f(false, S8, 0.0, 0, 0, &[1, 4, 3]),
        "Techno" => f(true, S16, 0.0, 2, -1, &[0, 2, 7]),
        "Trap" => f(true, S16, 0.0, 1, -1, &[0, 2, 6]),
        "UK garage" => f(true, S16, 0.25, 2, 0, &[5, 0, 3]),
        _ => f(true, S16, 0.0, 1, 0, &[0, 1, 2, 3, 4, 5, 6, 7]),
    }
}

/// Four-chord progressions as 0-based scale degrees (0 = the tonic chord).
/// Minor feels read them in natural minor, major feels in major.
const PROGRESSIONS: [[u8; 4]; 8] = [
    [0, 5, 2, 6], // i VI III VII
    [0, 4, 5, 3], // I V vi IV
    [0, 3, 4, 0], // i iv v i
    [5, 3, 0, 4], // vi IV I V
    [0, 3, 0, 4], // I IV I V
    [1, 4, 0, 5], // ii V I vi
    [0, 6, 5, 6], // i VII VI VII
    [3, 4, 2, 5], // IV V iii vi
];

const MAJOR: [i32; 7] = [0, 2, 4, 5, 7, 9, 11];
const MINOR: [i32; 7] = [0, 2, 3, 5, 7, 8, 10];

/// Small deterministic generator (xorshift), so a set never changes once made.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

/// Pitches of the scale around a key, as a function of a scale step (0 = tonic in octave 4).
struct Scale {
    tonic: i32,
    steps: [i32; 7],
}

impl Scale {
    fn new(key: Option<ProjectKey>, minor: bool) -> Self {
        let (root, minor) = key.map_or((0, minor), |k| (k.root as i32 % 12, k.minor));
        Scale { tonic: 60 + root - if root > 5 { 12 } else { 0 }, steps: if minor { MINOR } else { MAJOR } }
    }
    fn pitch(&self, step: i32) -> i32 {
        self.tonic + self.steps[step.rem_euclid(7) as usize] + 12 * step.div_euclid(7)
    }
}

struct Ctx {
    scale: Scale,
    feel: Feel,
    /// One entry per chord: (scale degree, start, length).
    chords: Vec<(i32, Ticks, Ticks)>,
    rng: Rng,
    out: Vec<Note>,
}

impl Ctx {
    fn note(&mut self, pitch: i32, start: Ticks, length: Ticks, velocity: i32) {
        let swing = if start % (2 * self.feel.grid) == self.feel.grid { (self.feel.swing * self.feel.grid as f64) as Ticks } else { 0 };
        let pitch = (pitch + 12 * self.feel.octave).clamp(0, 127) as u8;
        let length = (length - swing).max(PPQ / 16);
        self.out.push(Note { id: 0, pitch, start: start + swing, length, velocity: velocity.clamp(1, 127) as u8 });
    }

    /// Chord tones of a degree (root, third, fifth, then the seventh), in scale steps.
    fn tones(degree: i32) -> [i32; 4] {
        [degree, degree + 2, degree + 4, degree + 6]
    }

    /// Voice a chord close to middle C: each tone moved by octaves into 55..=72.
    fn voiced(&self, degree: i32, seventh: bool) -> Vec<i32> {
        let n = if seventh { 4 } else { 3 };
        let mut v: Vec<i32> = Self::tones(degree)[..n].iter().map(|&s| {
            let mut p = self.scale.pitch(s);
            while p < 55 {
                p += 12;
            }
            while p > 72 {
                p -= 12;
            }
            p
        }).collect();
        v.sort_unstable();
        v
    }

    fn chord_at(&self, t: Ticks) -> (i32, Ticks, Ticks) {
        *self.chords.iter().rev().find(|c| c.1 <= t).unwrap_or(&self.chords[0])
    }
}

/// Styles of each MIDI part: (name, player). Players fill `Ctx::out` over the chord spans.
type Style = (&'static str, fn(&mut Ctx));

fn held(c: &mut Ctx) {
    for (d, s, l) in c.chords.clone() {
        for p in c.voiced(d, false) {
            c.note(p, s, l - PPQ / 16, 92);
        }
    }
}

fn pulsing(c: &mut Ctx) {
    let step = if c.feel.density == 0 { PPQ } else { S8 };
    for (d, s, l) in c.chords.clone() {
        let mut t = s;
        while t < s + l {
            let accent = if (t - s) % PPQ == 0 { 100 } else { 78 };
            for p in c.voiced(d, true) {
                c.note(p, t, step * 3 / 4, accent);
            }
            t += step;
        }
    }
}

fn stabs(c: &mut Ctx) {
    for (d, s, l) in c.chords.clone() {
        let mut t = s + S8;
        while t < s + l {
            for p in c.voiced(d, false) {
                c.note(p + 12, t, S16, 104);
            }
            t += PPQ;
        }
    }
}

/// Dotted-quarter pushes (3 + 3 + 2 eighths), the last one anticipating the next chord.
fn tresillo_chords(c: &mut Ctx) {
    for (d, s, l) in c.chords.clone() {
        let mut bar = s;
        while bar < s + l {
            for (off, len) in [(0, 3 * S8), (3 * S8, 3 * S8), (6 * S8, 2 * S8)] {
                if bar + off < s + l {
                    for p in c.voiced(d, true) {
                        c.note(p, bar + off, len.min(s + l - bar - off) - S16 / 2, if off == 0 { 100 } else { 86 });
                    }
                }
            }
            bar += 4 * PPQ;
        }
    }
}

fn root_notes(c: &mut Ctx) {
    for (d, s, l) in c.chords.clone() {
        let p = c.scale.pitch(d) - 24;
        c.note(p, s, l - S16, 100);
    }
}

fn octave_bounce(c: &mut Ctx) {
    for (d, s, l) in c.chords.clone() {
        let root = c.scale.pitch(d) - 24;
        let mut t = s;
        let mut up = false;
        while t < s + l {
            c.note(if up { root + 12 } else { root }, t, S8 - S16 / 2, if up { 84 } else { 104 });
            up = !up;
            t += S8;
        }
    }
}

/// Quarter notes through the chord tones, approaching the next root by a step.
fn walking(c: &mut Ctx) {
    let chords = c.chords.clone();
    for (i, &(d, s, l)) in chords.iter().enumerate() {
        let next = chords.get(i + 1).map_or(chords[0].0, |n| n.0);
        let beats = (l / PPQ).max(1);
        for b in 0..beats {
            let step = match (b, beats - b) {
                (0, _) => d,
                (_, 1) => next + if c.rng.below(2) == 0 { 1 } else { -1 },
                _ => Ctx::tones(d)[(b as usize) % 3],
            };
            c.note(c.scale.pitch(step) - 24, s + b * PPQ, PPQ - S16, if b == 0 { 104 } else { 90 });
        }
    }
}

/// Sixteenth syncopation: hits on 1, the "a" of 1, the "and" of 2 and 4.
fn syncopated_bass(c: &mut Ctx) {
    for (d, s, l) in c.chords.clone() {
        let root = c.scale.pitch(d) - 24;
        let mut bar = s;
        while bar < s + l {
            for (off, len, up) in [(0, 3 * S16, 0), (3 * S16, 2 * S16, 0), (6 * S16, S16, 12), (10 * S16, 2 * S16, 0), (14 * S16, S16, 7)] {
                if bar + off < s + l {
                    c.note(root + up, bar + off, len, if off == 0 { 106 } else { 92 });
                }
            }
            bar += 4 * PPQ;
        }
    }
}

/// A random walk over the scale that lands on a chord tone at each chord change.
fn stepwise(c: &mut Ctx) {
    let end = c.chords.last().map_or(0, |x| x.1 + x.2);
    let step = if c.feel.density >= 2 { S16 * 2 } else { S8 * 2 };
    let mut pos = Ctx::tones(c.chords[0].0)[2] + 7;
    let mut t = 0;
    while t < end {
        let (d, s, _) = c.chord_at(t);
        if t == s {
            pos = Ctx::tones(d)[c.rng.below(3) as usize] + 7;
        } else {
            pos += [-2, -1, -1, 1, 1, 2][c.rng.below(6) as usize];
        }
        let rest = c.rng.below(5) == 0 && t != s;
        if !rest {
            let long = if c.rng.below(3) == 0 { 2 } else { 1 };
            let v = 96 - c.rng.below(16) as i32;
            c.note(c.scale.pitch(pos), t, step * long - S16 / 2, v);
            t += step * long;
        } else {
            t += step;
        }
    }
}

/// A two-beat motif repeated a step higher each time, falling back at the chord change.
fn rising_hook(c: &mut Ctx) {
    let shape: [(i32, Ticks, Ticks); 3] = [(0, 0, S8), (2, S8, S8), (4, 2 * S8, 2 * S8)];
    for (d, s, l) in c.chords.clone() {
        let mut t = s;
        let mut lift = 0;
        while t < s + l {
            for (iv, off, len) in shape {
                if t + off < s + l {
                    c.note(c.scale.pitch(d + 7 + iv + lift), t + off, len - S16 / 2, 98);
                }
            }
            lift += 1;
            t += 2 * PPQ;
        }
    }
}

/// A phrase on the chord's tones, then the same rhythm answered lower, a bar each.
fn call_response(c: &mut Ctx) {
    let rhythm: Vec<(Ticks, Ticks)> = (0..4).map(|i| (i * PPQ + if c.rng.below(2) == 0 { 0 } else { S8 }, S8 + S16)).collect();
    let end = c.chords.last().map_or(0, |x| x.1 + x.2);
    let mut bar = 0;
    let mut answer = false;
    while bar < end {
        for (i, &(off, len)) in rhythm.iter().enumerate() {
            let t = bar + off;
            if t >= end {
                break;
            }
            let (d, _, _) = c.chord_at(t);
            let tone = Ctx::tones(d)[i % 3] + if answer { 4 } else { 7 };
            c.note(c.scale.pitch(tone), t, len, if answer { 84 } else { 100 });
        }
        answer = !answer;
        bar += 4 * PPQ;
    }
}

/// Few notes, long ones: a chord tone on beat 1, an upper neighbour late in the bar.
fn sparse_hook(c: &mut Ctx) {
    for (d, s, l) in c.chords.clone() {
        let top = Ctx::tones(d)[2] + 7;
        c.note(c.scale.pitch(top), s, (l / 2).max(S8), 96);
        if l >= 2 * PPQ {
            c.note(c.scale.pitch(top + 1), s + l - PPQ + S8, S8, 82);
        }
    }
}

fn arp(c: &mut Ctx, order: &[usize]) {
    let step = if c.feel.density == 0 { S8 } else { S16 };
    for (d, s, l) in c.chords.clone() {
        let tones = c.voiced(d, true);
        let mut t = s;
        let mut i = 0;
        while t < s + l {
            let p = tones[order[i % order.len()] % tones.len()] + if order[i % order.len()] >= tones.len() { 12 } else { 0 };
            c.note(p, t, step - S16 / 4, if (t - s) % PPQ == 0 { 100 } else { 82 });
            t += step;
            i += 1;
        }
    }
}

fn arp_up(c: &mut Ctx) {
    arp(c, &[0, 1, 2, 3]);
}
fn arp_down(c: &mut Ctx) {
    arp(c, &[4, 3, 2, 1]);
}
fn arp_up_down(c: &mut Ctx) {
    arp(c, &[0, 1, 2, 3, 4, 3, 2, 1]);
}
fn broken_thirds(c: &mut Ctx) {
    arp(c, &[0, 2, 1, 3, 2, 4]);
}

fn styles(part: PartType) -> &'static [Style] {
    match part {
        PartType::Chords => &[("Held chords", held), ("Pulsing chords", pulsing), ("Off-beat stabs", stabs), ("Pushed chords", tresillo_chords)],
        PartType::Bass => &[("Root bass", root_notes), ("Octave bass", octave_bounce), ("Walking bass", walking), ("Syncopated bass", syncopated_bass)],
        PartType::Lead => &[("Stepwise lead", stepwise), ("Rising hook", rising_hook), ("Call and answer", call_response), ("Sparse hook", sparse_hook)],
        PartType::Arp => &[("Up arp", arp_up), ("Down arp", arp_down), ("Up-down arp", arp_up_down), ("Broken thirds", broken_thirds)],
    }
}

/// Chord spans for four chords over a clip: one chord per bar from four bars
/// up (the progression repeats), faster on shorter clips.
fn spans(prog: [u8; 4], clip: Ticks, ticks_per_bar: Ticks) -> Vec<(i32, Ticks, Ticks)> {
    let bars = (clip / ticks_per_bar.max(1)).max(1);
    let each = if bars >= 4 { ticks_per_bar } else { (bars * ticks_per_bar / 4).max(PPQ) };
    let mut out = Vec::new();
    let mut t = 0;
    let mut i = 0;
    while t < clip.max(each) {
        out.push((prog[i % 4] as i32, t, each.min(clip.max(each) - t)));
        t += each;
        i += 1;
    }
    out
}

/// The six patterns of a MIDI part's Create: set number `set` (0 = the first
/// Create) for this genre and key, written to fill `clip` ticks.
pub fn create_set(part: PartType, genre: usize, key: Option<ProjectKey>, set: u32, clip: Ticks, ticks_per_bar: Ticks) -> Vec<LibPattern> {
    let styles = styles(part);
    let f = feel(genre);
    let mut taken: Vec<&str> = Vec::new();
    (0..SET_SIZE)
        .map(|i| {
            let seed = (part as u64) << 40 ^ (genre as u64) << 32 ^ (set as u64) << 8 ^ i as u64;
            let mut rng = Rng::new(seed + 1);
            let (name, play) = styles[(i + set as usize) % styles.len()];
            let prog = PROGRESSIONS[f.progressions[(rng.below(64) as usize + i) % f.progressions.len()]];
            let fe = feel(genre);
            let mut c = Ctx { scale: Scale::new(key, fe.minor), feel: fe, chords: spans(prog, clip, ticks_per_bar), rng, out: Vec::new() };
            play(&mut c);
            let end = clip.max(PPQ);
            let mut notes: Vec<Note> = c.out.into_iter().filter(|n| n.start < end).map(|n| Note { length: n.length.min(end - n.start), ..n }).collect();
            midi::sort_notes(&mut notes);
            for (id, n) in notes.iter_mut().enumerate() {
                n.id = id as u32 + 1;
            }
            let count = taken.iter().filter(|&&t| t == name).count();
            taken.push(name);
            let name = if count == 0 { name.to_string() } else { format!("{name} {}", count + 1) };
            LibPattern { name, notes, pattern: None }
        })
        .collect()
}

/// Drum grooves: (name, rows for pads 1..6 as 16 sixteenths; 'x' hit, 'o' soft hit, '-' rest).
type Groove = (&'static str, [&'static str; 6]);

const GROOVES: [Groove; 10] = [
    ("Four on the floor", ["x---x---x---x---", "----x-------x---", "--x---x---x---x-", "------------x---", "----------------", "-------o-------o"]),
    ("Boom bap", ["x------x-x------", "----x-------x---", "x-x-x-x-x-x-x-x-", "----------------", "----------------", "---o------o-----"]),
    ("Trap rolls", ["x-------x-x-----", "--------x-------", "x-x-xxx-x-x-x-xx", "----------------", "--------x-------", "------o-------o-"]),
    ("Two-step break", ["x---------x-----", "----x-------x---", "x-x-x-x-x-x-x-x-", "-------x--------", "----------------", "--o-------o--o--"]),
    ("Afro swing", ["x-----x---x-----", "------x-------x-", "x-xxx-xxx-xxx-xx", "----------------", "----x-------x---", "--o--o----o--o--"]),
    ("Drill slide", ["x------x--x-----", "------x-------x-", "x-x-x-x-xxx-x-x-", "----------------", "------x-------x-", "---o-------o----"]),
    ("Half-time float", ["x-------------x-", "--------x-------", "x---x---x---x---", "------x---------", "----------------", "------o-----o---"]),
    ("Folk shuffle", ["x-------x-------", "----x-------x---", "x--x--x-x--x--x-", "----------------", "----x-------x---", "--o-------o-----"]),
    ("Dembow", ["x---x---x---x---", "---x--x----x--x-", "x-x-x-x-x-x-x-x-", "----------------", "----------------", "-o-----o-o-----o"]),
    ("Warehouse pulse", ["x---x---x---x---", "----o-------o---", "x-x-x-x-x-x-x-x-", "--x---x---x---x-", "----x-------x---", "-o--o--o-o--o--o"]),
];

fn grooves_for(genre: usize) -> &'static [usize] {
    match GENRES.get(genre).copied().unwrap_or("Any") {
        "Afrobeats" => &[4, 0, 3],
        "Ambient" => &[6, 7, 1],
        "Drill" => &[5, 2, 6],
        "Drum & bass" => &[3, 6, 1],
        "EDM" | "Electronic" | "Future bass" => &[0, 3, 2],
        "Experimental" => &[6, 3, 4],
        "Folk" => &[7, 1, 0],
        "Hip-hop" => &[1, 2, 6],
        "House" => &[0, 9, 3],
        "Indie" => &[1, 7, 0],
        "Lo-fi" => &[1, 6, 7],
        "Neo-soul" => &[1, 6, 4],
        "Pop" => &[0, 3, 1],
        "R&B" => &[6, 1, 2],
        "Reggaeton" => &[8, 4, 0],
        "Singer-songwriter" => &[7, 6, 1],
        "Techno" => &[9, 0, 3],
        "Trap" => &[2, 5, 6],
        "UK garage" => &[3, 4, 9],
        // Any keeps the first eight, so sets made before Dembow and Warehouse pulse stay the same.
        _ => &[0, 1, 2, 3, 4, 5, 6, 7],
    }
}

/// Variation names, in the order a set uses them.
const VARIATIONS: [&str; 4] = ["", "with ghost notes", "busier hats", "kick push"];

/// The six patterns of a drum clip's Create (16 steps of 1/16 on the pads).
pub fn create_drum_set(genre: usize, set: u32) -> Vec<LibPattern> {
    let pool = grooves_for(genre);
    (0..SET_SIZE)
        .map(|i| {
            let mut rng = Rng::new(0xD5 << 32 ^ (genre as u64) << 16 ^ (set as u64) << 4 ^ i as u64);
            let (gname, rows) = GROOVES[pool[(i + set as usize * 2) % pool.len()]];
            let var = (2 * (i / pool.len().min(SET_SIZE)) + set as usize + i) % VARIATIONS.len();
            let mut p = StepPattern::new(StepRows::Pads);
            p.rate = StepRate::Sixteenth;
            if feel(genre).swing > 0.0 {
                p.swing = 0.3;
            }
            for (pad, row) in rows.iter().enumerate() {
                for (step, ch) in row.bytes().enumerate() {
                    let v = match ch {
                        b'x' => 104 - (rng.below(12) as i32),
                        b'o' => 62 + (rng.below(10) as i32),
                        _ => 0,
                    };
                    if v > 0 {
                        p = p.toggled(36 + pad as u8, step as u16).with_lane(36 + pad as u8, step as u16, StepLane::Velocity, v);
                    }
                }
            }
            match var {
                1 => {
                    for step in [3u16, 6, 11, 14] {
                        if p.cell(37, step).is_none() {
                            p = p.toggled(37, step).with_lane(37, step, StepLane::Velocity, 40);
                        }
                    }
                }
                2 => {
                    for step in 0..16u16 {
                        if p.cell(38, step).is_none() && rng.below(3) > 0 {
                            p = p.toggled(38, step).with_lane(38, step, StepLane::Velocity, 58);
                        }
                    }
                    if let Some(s) = (8..16u16).find(|&s| p.cell(38, s).is_some()) {
                        p = p.with_lane(38, s, StepLane::Repeat, 2);
                    }
                }
                3 => {
                    if let Some(last) = (0..16u16).rev().find(|&s| p.cell(36, s).is_some() && s > 0) {
                        p = p.toggled(36, last);
                        if p.cell(36, last - 1).is_none() {
                            p = p.toggled(36, last - 1);
                        }
                    }
                    p = p.toggled(36, 15).with_lane(36, 15, StepLane::Velocity, 70);
                }
                _ => {}
            }
            let name = if VARIATIONS[var].is_empty() { gname.to_string() } else { format!("{gname}, {}", VARIATIONS[var]) };
            LibPattern { name, notes: p.bake(), pattern: Some(p) }
        })
        .collect()
}

/// A set the clip's Create made: for a MIDI part (`part` Some) or the drums (None).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MadeSet {
    pub part: Option<PartType>,
    pub genre: u8,
    pub set: u32,
    /// The row last played from this set (Type switches back to it).
    #[serde(default)]
    pub last: u8,
}

/// A clip's pattern list state (the sidebar's Patterns tab), saved with the clip.
/// The patterns are not stored: `MadeSet` regenerates them from the library.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Composer {
    /// The Type field (MIDI clips).
    pub part: PartType,
    /// The Genre field (index into GENRES) the next Create builds on.
    pub genre: u8,
    /// The last set made per part (at most one entry per part, drums included).
    pub made: Vec<MadeSet>,
    /// The row playing in the clip (of the shown part's set), None while the kept notes play.
    pub playing: Option<u8>,
    /// The clip's own notes (and step grid), kept while a pattern plays in its place.
    pub kept: Vec<Note>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kept_pattern: Option<StepPattern>,
    /// The Shape tab's values per source (only sources that are not neutral).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shapes: Vec<ShapeSlot>,
    /// The unshaped notes of what plays, while its Shape is not neutral.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub base: Vec<Note>,
}

impl Composer {
    /// The set shown for a part (None: the drums).
    pub fn set_for(&self, part: Option<PartType>) -> Option<MadeSet> {
        self.made.iter().copied().find(|m| m.part == part)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.genre as usize >= GENRES.len() || self.made.iter().any(|m| m.genre as usize >= GENRES.len()) {
            return Err("genre out of range".into());
        }
        if self.playing.is_some_and(|i| i as usize >= SET_SIZE) || self.made.iter().any(|m| m.last as usize >= SET_SIZE) {
            return Err("playing row out of range".into());
        }
        if let Some(p) = &self.kept_pattern {
            p.validate()?;
        }
        for s in &self.shapes {
            s.shape.validate()?;
        }
        midi::validate_notes(&self.base)?;
        midi::validate_notes(&self.kept)
    }

    /// What the Shape tab shapes: the shown part's patterns while one plays, else the kept notes.
    pub fn shape_source(&self) -> Option<PartType> {
        self.playing.map(|_| self.part)
    }

    pub fn shape_of(&self, source: Option<PartType>) -> Shape {
        self.shapes.iter().find(|s| s.part == source).map_or(Shape::neutral(source), |s| s.shape)
    }

    /// Store a source's Shape (a neutral one is dropped).
    pub fn set_shape(&mut self, source: Option<PartType>, shape: Shape) {
        self.shapes.retain(|s| s.part != source);
        if shape != Shape::neutral(source) {
            self.shapes.push(ShapeSlot { part: source, shape });
        }
    }
}

/// The patterns of a made set, for a clip of `clip` ticks.
pub fn patterns_of(made: MadeSet, key: Option<ProjectKey>, clip: Ticks, ticks_per_bar: Ticks) -> Vec<LibPattern> {
    match made.part {
        Some(part) => create_set(part, made.genre as usize, key, made.set, clip, ticks_per_bar),
        None => create_drum_set(made.genre as usize, made.set),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sets_are_six_valid_deterministic_patterns_in_key() {
        let key = Some(ProjectKey { root: 2, minor: true });
        for part in PartType::ALL {
            for genre in 0..GENRES.len() {
                for clip in [4 * PPQ, 16 * PPQ] {
                    let a = create_set(part, genre, key, 3, clip, 4 * PPQ);
                    assert_eq!(a, create_set(part, genre, key, 3, clip, 4 * PPQ));
                    assert_eq!(a.len(), SET_SIZE);
                    for p in &a {
                        assert!(!p.notes.is_empty(), "{part:?} {genre} {}", p.name);
                        midi::validate_notes(&p.notes).unwrap();
                        assert!(p.notes.iter().all(|n| n.start < clip));
                        let d_minor = [2, 4, 5, 7, 9, 10, 0];
                        assert!(p.notes.iter().all(|n| d_minor.contains(&(n.pitch as i32 % 12)) || part == PartType::Bass), "{part:?} {} out of key", p.name);
                    }
                }
            }
        }
    }

    #[test]
    fn a_new_set_differs_and_names_are_unique() {
        let a = create_set(PartType::Bass, 0, None, 0, 8 * PPQ, 4 * PPQ);
        let b = create_set(PartType::Bass, 0, None, 1, 8 * PPQ, 4 * PPQ);
        assert_ne!(a, b);
        let mut names: Vec<&str> = a.iter().map(|p| p.name.as_str()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), SET_SIZE);
    }

    #[test]
    fn saved_genre_indices_keep_their_genre() {
        let before = ["Any", "Afrobeats", "Ambient", "Drill", "Drum & bass", "EDM", "Electronic", "Experimental", "Folk", "Future bass", "Hip-hop"];
        assert_eq!(GENRES[..before.len()], before);
        assert_eq!(grooves_for(0), &[0, 1, 2, 3, 4, 5, 6, 7], "Any's drum sets are unchanged");
    }

    #[test]
    fn drum_sets_are_valid_pad_patterns() {
        for genre in 0..GENRES.len() {
            for set in 0..3 {
                let s = create_drum_set(genre, set);
                assert_eq!(s.len(), SET_SIZE);
                let mut names: Vec<&str> = s.iter().map(|p| p.name.as_str()).collect();
                names.sort_unstable();
                names.dedup();
                assert_eq!(names.len(), SET_SIZE, "genre {genre} set {set}");
                for p in &s {
                    let pat = p.pattern.as_ref().unwrap();
                    pat.validate().unwrap();
                    assert!(pat.cells.iter().any(|c| c.key == 36) && pat.cells.iter().any(|c| c.key == 37), "{}", p.name);
                    assert_eq!(p.notes, pat.bake());
                }
            }
        }
    }
}
