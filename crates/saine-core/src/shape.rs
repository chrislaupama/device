//! Shape: the clip sidebar's second tab. Its controls reshape what the clip's
//! composer plays (a library pattern, or the clip's kept notes) without losing
//! it: while a Shape is not neutral, the unshaped notes stay in
//! \`Composer::base\` and the clip plays \`apply(base)\`.
//!
//! The controls, their ranges, steps and neutral values follow the reference's
//! Shape rack (reference/findings/patterns.md); the transforms are Saine's own.

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
/// Octave steps either way.
pub const MAX_OCTAVE: i8 = 2;
/// Most voices the Voices control offers (1 = the line alone).
pub const MAX_VOICES: u8 = 3;
/// How far Nudge moves notes at either end, in beats.
const NUDGE_BEATS: f64 = 0.5;
/// Part of each retrigger or arp step that sounds.
const GATE: f64 = 0.86;
/// Shortest Length, as a part of each note.
const MIN_LENGTH: f64 = 0.1;
/// Arp range in octaves (the arp plays up and down through them).
const ARP_OCTAVES: i32 = 2;

/// One source's Shape. Continuous controls are 0..=1 as the rack shows them.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
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
}

/// The velocity value that plays a part's notes as made.
pub fn neutral_velocity(part: Option<PartType>) -> f32 {
    if part == Some(PartType::Bass) { 0.85 } else { 0.8 }
}

impl Shape {
    /// The Shape that plays a source's notes as made.
    pub fn neutral(part: Option<PartType>) -> Shape {
        Shape { octave: 0, voices: 1, density: 1.0, length: 0.5, velocity: neutral_velocity(part), nudge: 0.5, repeat: 0, arp: 0 }
    }

    pub fn validate(&self) -> Result<(), String> {
        let unit = [self.density, self.length, self.velocity, self.nudge];
        if !(-MAX_OCTAVE..=MAX_OCTAVE).contains(&self.octave) || !(1..=MAX_VOICES).contains(&self.voices) {
            return Err("shape octave or voices out of range".into());
        }
        if unit.iter().any(|v| !(0.0..=1.0).contains(v)) || self.repeat as usize >= REPEAT_BEATS.len() || self.arp as usize >= ARP_BEATS.len() {
            return Err("shape value out of range".into());
        }
        Ok(())
    }
}

/// A clip's Shape for one source: a part's patterns (\`part\` Some) or the kept notes (None).
/// Each part keeps its own; switching parts plays the other part's Shape.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ShapeSlot {
    pub part: Option<PartType>,
    pub shape: Shape,
}

/// Whether Voices has anything to add: a single line (no two notes sounding
/// together) that is not a bass line or chords.
pub fn has_voices(notes: &[Note], part: Option<PartType>) -> bool {
    if matches!(part, Some(PartType::Bass | PartType::Chords)) || notes.is_empty() {
        return false;
    }
    let mut v: Vec<&Note> = notes.iter().collect();
    v.sort_by_key(|n| (n.start, n.pitch));
    v.windows(2).all(|w| w[1].start > w[0].start && w[1].start + PPQ / 8 >= w[0].start + w[0].length)
}

/// The notes as the Shape plays them, sorted and numbered from 1. A neutral Shape
/// returns them unchanged.
pub fn apply(notes: &[Note], shape: Shape, part: Option<PartType>, key: Option<ProjectKey>, clip: Ticks) -> Vec<Note> {
    if shape == Shape::neutral(part) {
        return notes.to_vec();
    }
    let mut v = notes.to_vec();
    v.sort_by_key(|n| (n.start, n.pitch));
    if shape.density < 1.0 {
        v = thin(&v, shape.density as f64);
    }
    if shape.octave != 0 {
        let by = 12 * shape.octave as i32;
        for n in &mut v {
            n.pitch = fold(n.pitch as i32 + by);
        }
    }
    let ratio = shape.velocity / neutral_velocity(part);
    if (ratio - 1.0).abs() > 1e-4 {
        for n in &mut v {
            n.velocity = (n.velocity as f32 * ratio).round().clamp(1.0, 127.0) as u8;
        }
    }
    if shape.voices > 1 && has_voices(&v, part) {
        v = add_voices(&v, shape.voices, key);
    }
    let arp = ARP_BEATS[shape.arp as usize];
    if arp > 0.0 {
        v = arpeggiate(&v, ((arp * PPQ as f64).round() as Ticks).max(1));
    }
    if (shape.length - 0.5).abs() > 1e-4 {
        set_lengths(&mut v, shape.length as f64, clip);
    }
    let rate = REPEAT_BEATS[shape.repeat as usize];
    if rate > 0.0 && arp == 0.0 {
        v = repeat(&v, ((rate * PPQ as f64).round() as Ticks).max(1));
    }
    let shift = ((shape.nudge as f64 - 0.5) * 2.0 * NUDGE_BEATS * PPQ as f64).round() as Ticks;
    if shift != 0 {
        for n in &mut v {
            n.start = nudged(n.start, shift, clip);
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

/// Metric weight of a position: the bar line highest, off-sixteenths lowest.
fn weight(start: Ticks) -> u32 {
    let unit = PPQ / 4;
    if start % unit != 0 {
        return 0;
    }
    let s = start / unit;
    if s == 0 { 9 } else { s.trailing_zeros().min(7) + 1 }
}

fn onsets(v: &[Note]) -> BTreeMap<Ticks, Vec<usize>> {
    let mut m: BTreeMap<Ticks, Vec<usize>> = BTreeMap::new();
    for (i, n) in v.iter().enumerate() {
        m.entry(n.start).or_default().push(i);
    }
    m
}

/// Density: chords keep their outer notes first, single notes the strongest beats first.
fn thin(v: &[Note], density: f64) -> Vec<Note> {
    let mut keep = vec![false; v.len()];
    let mut singles = Vec::new();
    for (start, mut idx) in onsets(v) {
        if idx.len() == 1 {
            singles.push((weight(start), start, idx[0]));
            continue;
        }
        idx.sort_by_key(|&i| v[i].pitch);
        let n = ((density * idx.len() as f64).ceil() as usize).max(1);
        let last = idx.len() - 1;
        let order = [idx[0], idx[last]].into_iter().chain(idx[1..last].iter().copied());
        for i in order.take(n) {
            keep[i] = true;
        }
    }
    singles.sort_by_key(|&(w, start, _)| (std::cmp::Reverse(w), start));
    let n = (density * singles.len() as f64).round() as usize;
    for &(_, _, i) in singles.iter().take(n) {
        keep[i] = true;
    }
    if !keep.contains(&true)
        && let Some(&(_, _, i)) = singles.first()
    {
        keep[i] = true;
    }
    v.iter().zip(keep).filter(|(_, k)| *k).map(|(n, _)| *n).collect()
}

/// The scale the notes fit best: the project key, else the major or minor key
/// holding most of them (ties go to the key of the first note).
fn scale_of(v: &[Note], key: Option<ProjectKey>) -> (i32, &'static [i32; 7]) {
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

/// Voices: a third under each note, then a fifth, in the scale.
fn add_voices(v: &[Note], voices: u8, key: Option<ProjectKey>) -> Vec<Note> {
    let (root, steps) = scale_of(v, key);
    let pitch_of = |deg: i32| root + steps[deg.rem_euclid(7) as usize] + 12 * deg.div_euclid(7);
    let mut out = v.to_vec();
    for n in v {
        let rel = n.pitch as i32 - root;
        let in_oct = rel.rem_euclid(12);
        let step = steps.iter().rposition(|&s| s <= in_oct).unwrap_or(0) as i32;
        let deg = 7 * rel.div_euclid(12) + step;
        let off = n.pitch as i32 - pitch_of(deg);
        for k in 1..voices as i32 {
            let p = pitch_of(deg - 2 * k) + off;
            if (0..=127).contains(&p) {
                out.push(Note { pitch: p as u8, velocity: ((n.velocity as u32 * 7) / 8).max(1) as u8, ..*n });
            }
        }
    }
    out
}

/// Arp: each chord (or note) plays its notes up and down across ARP_OCTAVES, one
/// per step, until the next chord.
fn arpeggiate(v: &[Note], step: Ticks) -> Vec<Note> {
    let groups: Vec<(Ticks, Vec<usize>)> = onsets(v).into_iter().collect();
    let mut out = Vec::new();
    for (g, (start, idx)) in groups.iter().enumerate() {
        let mut end = idx.iter().map(|&i| v[i].start + v[i].length).max().unwrap_or(*start);
        if let Some((next, _)) = groups.get(g + 1) {
            end = end.min(*next);
        }
        let mut pool: Vec<(i32, u8)> = idx.iter().map(|&i| (v[i].pitch as i32, v[i].velocity)).collect();
        pool.sort();
        pool.dedup_by_key(|p| p.0);
        let base = pool.clone();
        for o in 1..ARP_OCTAVES {
            pool.extend(base.iter().map(|&(p, vel)| (p + 12 * o, vel)));
        }
        pool.retain(|p| p.0 <= 127);
        let m = pool.len();
        if m == 0 {
            continue;
        }
        let order: Vec<(i32, u8)> = if m < 3 { pool.clone() } else { pool.iter().chain(pool[1..m - 1].iter().rev()).copied().collect() };
        let len = ((step as f64 * GATE).round() as Ticks).max(1);
        let mut t = *start;
        let mut k = 0;
        while t < end {
            let (p, vel) = order[k % order.len()];
            out.push(Note { id: 0, pitch: p as u8, start: t, length: len.min(end - t), velocity: vel });
            t += step;
            k += 1;
        }
    }
    out
}

/// Length under 0.5 shortens each note (to MIN_LENGTH at 0); over 0.5 holds it
/// toward the next note's start (the clip end for the last).
fn set_lengths(v: &mut [Note], length: f64, clip: Ticks) {
    let starts: Vec<Ticks> = onsets(v).into_keys().collect();
    for n in v.iter_mut() {
        let l = n.length as f64;
        let new = if length < 0.5 {
            l * (MIN_LENGTH + (1.0 - MIN_LENGTH) * length / 0.5)
        } else {
            let next = starts.iter().copied().find(|&s| s > n.start).unwrap_or(clip.max(n.start + n.length));
            let target = ((next - n.start) as f64).max(l);
            l + (target - l) * (length - 0.5) / 0.5
        };
        n.length = (new.round() as Ticks).max(1);
    }
}

/// Repeat: each note breaks into retriggers at the rate inside its own length.
fn repeat(v: &[Note], step: Ticks) -> Vec<Note> {
    let len = ((step as f64 * GATE).round() as Ticks).max(1);
    let mut out = Vec::with_capacity(v.len());
    for n in v {
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

    fn chord() -> Vec<Note> {
        let mut v: Vec<Note> = [60, 64, 67, 71].iter().map(|&p| note(p, 0.0, 4.0)).collect();
        v.extend([60, 63, 67].iter().map(|&p| note(p, 4.0, 4.0)));
        for (i, n) in v.iter_mut().enumerate() {
            n.id = i as u32 + 1;
        }
        v
    }

    fn line() -> Vec<Note> {
        (0..8).map(|i| Note { id: i + 1, ..note(72 + (i % 3) as u8 * 2, i as f64 * 0.5, 0.5) }).collect()
    }

    const BAR2: Ticks = 8 * PPQ;

    #[test]
    fn neutral_shape_plays_the_notes_as_made() {
        for part in [None, Some(PartType::Chords), Some(PartType::Bass), Some(PartType::Lead)] {
            assert_eq!(apply(&chord(), Shape::neutral(part), part, None, BAR2), chord());
            assert!(Shape::neutral(part).validate().is_ok());
        }
    }

    #[test]
    fn octave_velocity_and_nudge() {
        let s = Shape { octave: -1, velocity: 0.4, nudge: 0.75, ..Shape::neutral(None) };
        let out = apply(&chord(), s, None, None, BAR2);
        assert_eq!(out.len(), 7);
        assert!(out.iter().all(|n| n.velocity == 40 && n.start % (4 * PPQ) == PPQ / 4));
        assert_eq!(out[0].pitch, 48);
        midi::validate_notes(&out).unwrap();
    }

    #[test]
    fn density_keeps_outer_chord_notes_and_strong_beats() {
        let half = Shape { density: 0.5, ..Shape::neutral(None) };
        let out = apply(&chord(), half, None, None, BAR2);
        let first: Vec<u8> = out.iter().filter(|n| n.start == 0).map(|n| n.pitch).collect();
        assert_eq!(first, vec![60, 71]);
        let out = apply(&line(), half, None, None, BAR2);
        assert_eq!(out.iter().map(|n| n.start / (PPQ / 2)).collect::<Vec<_>>(), vec![0, 2, 4, 6]);
        let none = Shape { density: 0.0, ..Shape::neutral(None) };
        assert_eq!(apply(&line(), none, None, None, BAR2).len(), 1, "one note always stays");
    }

    #[test]
    fn length_shortens_and_holds_to_the_next_note() {
        let short = apply(&line(), Shape { length: 0.0, ..Shape::neutral(None) }, None, None, BAR2);
        assert!(short.iter().all(|n| n.length == (PPQ as f64 * 0.5 * MIN_LENGTH).round() as Ticks));
        let gaps: Vec<Note> = (0..4).map(|i| Note { id: i + 1, ..note(60, i as f64, 0.25) }).collect();
        let held = apply(&gaps, Shape { length: 1.0, ..Shape::neutral(None) }, None, None, 4 * PPQ);
        assert!(held.iter().all(|n| n.length == PPQ));
    }

    #[test]
    fn repeat_retriggers_inside_each_note_and_chords_repeat_as_chords() {
        let out = apply(&chord(), Shape { repeat: 2, ..Shape::neutral(None) }, None, None, BAR2);
        assert_eq!(out.len(), 7 * 8);
        assert_eq!(out.iter().filter(|n| n.start == PPQ / 2).count(), 4);
        assert!(out.iter().all(|n| n.length == (PPQ as f64 * 0.5 * GATE).round() as Ticks));
    }

    #[test]
    fn arp_plays_each_chord_up_and_down_until_the_next() {
        let out = apply(&chord(), Shape { arp: 3, ..Shape::neutral(Some(PartType::Chords)) }, Some(PartType::Chords), None, BAR2);
        assert_eq!(out.len(), 32, "a sixteenth arp over two bars");
        let first: Vec<u8> = out.iter().take(9).map(|n| n.pitch).collect();
        assert_eq!(first, vec![60, 64, 67, 71, 72, 76, 79, 83, 79]);
        assert_eq!(out[16].pitch, 60, "the second chord starts its own arp");
        assert!(out.windows(2).all(|w| w[0].start < w[1].start));
    }

    #[test]
    fn voices_harmonise_a_single_line_in_key() {
        let key = Some(ProjectKey { root: 0, minor: false });
        let lead = Some(PartType::Lead);
        assert!(has_voices(&line(), lead));
        assert!(!has_voices(&chord(), lead) && !has_voices(&line(), Some(PartType::Bass)));
        let out = apply(&line(), Shape { voices: 3, ..Shape::neutral(lead) }, lead, key, BAR2);
        assert_eq!(out.len(), 24);
        let at0: Vec<u8> = out.iter().filter(|n| n.start == 0).map(|n| n.pitch).collect();
        assert_eq!(at0, vec![65, 69, 72], "C5 gets A4 and F4 under it in C major");
        let chords = apply(&chord(), Shape { voices: 3, ..Shape::neutral(None) }, None, key, BAR2);
        assert_eq!(chords.len(), 7, "chords have no extra voices to add");
    }

    #[test]
    fn octave_and_nudge_never_lose_notes() {
        let top = vec![Note { id: 1, ..note(120, 3.75, 0.25) }];
        let out = apply(&top, Shape { octave: 2, nudge: 1.0, ..Shape::neutral(None) }, None, None, 4 * PPQ);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].pitch, 120, "folded back an octave at a time into range");
        assert!(out[0].start < 4 * PPQ);
    }

    #[test]
    fn out_of_range_values_are_refused() {
        let n = Shape::neutral(None);
        for bad in [Shape { octave: 3, ..n }, Shape { voices: 0, ..n }, Shape { density: 1.5, ..n }, Shape { nudge: f32::NAN, ..n }, Shape { repeat: 5, ..n }, Shape { arp: 6, ..n }] {
            assert!(bad.validate().is_err(), "{bad:?}");
        }
    }
}
