//! Smart Quantize: snap notes to the grid they already imply (1/8, 1/16, 1/16T or
//! 1/32) without picking a grid first, and drop the small artifacts a transcription
//! leaves (quiet blips and ghost overtones). Behaviour and values follow the
//! reference's piano-roll Smart Quantize (findings in reference/findings/clip-editors.md).

use crate::midi::Note;
use crate::model::{PPQ, Ticks};

/// Candidate grids in beats, tried in this order.
const GRIDS: [(f64, &str); 4] = [
    (0.5, "1/8"),
    (0.25, "1/16"),
    (1.0 / 6.0, "1/16T"),
    (0.125, "1/32"),
];
/// Share of onsets a grid must catch to win outright.
const CAPTURE: f64 = 0.85;
/// Capture window: 30 ms, at most a third of the grid.
const WINDOW_SEC: f64 = 0.03;
/// Fewer onsets than this keep the 1/16 grid.
const MIN_NOTES: usize = 3;
/// A common offset of the notes from the grid is kept when they agree on it this
/// strongly and it is under 90 ms and 0.126 beats.
const PHASE_AGREEMENT: f64 = 0.75;
const PHASE_MAX_SEC: f64 = 0.09;
const PHASE_MAX_BEATS: f64 = 0.126;
/// Blips: quieter than this and shorter than half a 1/16.
const BLIP_VELOCITY: f64 = 0.12;
/// Ghost overtones: an octave or an octave and a fifth above a note, within a 1/32,
/// at under half its velocity.
const OVERTONES: [i32; 2] = [12, 19];
const OVERTONE_WINDOW: f64 = 0.125;
const OVERTONE_RATIO: f64 = 0.5;
/// On the 1/32 grid, when most onsets sit on the even steps, an odd-step onset
/// outside a run of at least three moves back one step.
const EVEN_SHARE: f64 = 0.5;
const RUN: usize = 3;

pub struct SmartQuantized {
    pub notes: Vec<Note>,
    pub grid: &'static str,
}

fn beats(t: Ticks) -> f64 {
    t as f64 / PPQ as f64
}

fn vel(n: &Note) -> f64 {
    n.velocity as f64 / 127.0
}

/// The offset (beats, within half a grid) the onsets share, or 0 when they don't agree.
fn phase(times: &[f64], grid: f64, bpm: f64) -> f64 {
    if times.is_empty() {
        return 0.0;
    }
    let (mut x, mut y) = (0.0, 0.0);
    for t in times {
        let a = t.rem_euclid(grid) / grid * std::f64::consts::TAU;
        x += a.cos();
        y += a.sin();
    }
    if (x * x + y * y).sqrt() / (times.len() as f64) < PHASE_AGREEMENT {
        return 0.0;
    }
    let mut p = y.atan2(x).rem_euclid(std::f64::consts::TAU) / std::f64::consts::TAU * grid;
    if p > grid / 2.0 {
        p -= grid;
    }
    if p.abs() / (bpm / 60.0) > PHASE_MAX_SEC || p.abs() > PHASE_MAX_BEATS {
        0.0
    } else {
        p
    }
}

fn choose(times: &[f64], bpm: f64) -> (f64, &'static str, f64) {
    if times.len() < MIN_NOTES || bpm <= 0.0 {
        return (0.25, "1/16", 0.0);
    }
    let window = bpm / 60.0 * WINDOW_SEC;
    let mut best: Option<(f64, f64, &'static str, f64)> = None;
    for (grid, label) in GRIDS {
        let tol = window.min(grid / 3.0);
        let p = phase(times, grid, bpm);
        let caught = times
            .iter()
            .filter(|&&t| ((t - p) - ((t - p) / grid).round() * grid).abs() <= tol)
            .count();
        let capture = caught as f64 / times.len() as f64;
        if capture >= CAPTURE {
            return (grid, label, p);
        }
        if best.is_none_or(|b| capture >= b.0) {
            best = Some((capture, grid, label, p));
        }
    }
    let (_, grid, label, p) = best.expect("grids");
    (grid, label, p)
}

fn artifacts_removed(notes: &[Note]) -> Vec<Note> {
    let kept: Vec<Note> = notes
        .iter()
        .filter(|n| !(vel(n) < BLIP_VELOCITY && beats(n.length) < 0.125))
        .copied()
        .collect();
    kept.iter()
        .filter(|e| {
            !kept.iter().any(|t| {
                !std::ptr::eq(*e, t)
                    && OVERTONES.contains(&(e.pitch as i32 - t.pitch as i32))
                    && (beats(t.start) - beats(e.start)).abs() <= OVERTONE_WINDOW
                    && vel(e) < vel(t) * OVERTONE_RATIO
            })
        })
        .copied()
        .collect()
}

/// Odd 1/32 steps that stand alone move back onto the even step before them, when
/// most onsets are on even steps.
fn settle_odd_steps(notes: &mut [Note], step: Ticks) {
    let mut onsets: Vec<Ticks> = notes.iter().map(|n| n.start).collect();
    onsets.sort_unstable();
    onsets.dedup();
    if onsets.is_empty()
        || (onsets.iter().filter(|&&t| (t / step) % 2 == 0).count() as f64) / (onsets.len() as f64)
            < EVEN_SHARE
    {
        return;
    }
    let mut run_of = std::collections::HashMap::new();
    let mut from = 0;
    for i in 1..=onsets.len() {
        if i == onsets.len() || onsets[i] - onsets[i - 1] > step {
            for &t in &onsets[from..i] {
                run_of.insert(t, i - from);
            }
            from = i;
        }
    }
    for n in notes {
        if (n.start / step) % 2 != 0 && run_of[&n.start] < RUN {
            n.start = (n.start - step).max(0);
        }
    }
}

/// Smart Quantize the notes at the project tempo. Kept notes keep their ids.
pub fn smart_quantize(notes: &[Note], bpm: f64) -> SmartQuantized {
    if notes.is_empty() {
        return SmartQuantized {
            notes: Vec::new(),
            grid: "1/16",
        };
    }
    let clean = artifacts_removed(notes);
    let src = if clean.is_empty() {
        notes.to_vec()
    } else {
        clean
    };
    let times: Vec<f64> = src.iter().map(|n| beats(n.start)).collect();
    let (grid, label, p) = choose(&times, bpm);
    let step = (grid * PPQ as f64).round() as Ticks;
    let offset = (p * PPQ as f64).round() as Ticks;
    let snap = |t: Ticks| ((t - offset) as f64 / step as f64).round() as Ticks * step;
    let mut snapped: Vec<Note> = src
        .iter()
        .map(|n| Note {
            start: snap(n.start).max(0),
            length: ((n.length as f64 / step as f64).round() as Ticks * step).max(step),
            ..*n
        })
        .collect();
    if grid <= 0.125 {
        settle_odd_steps(&mut snapped, step);
    }
    let mut best: std::collections::BTreeMap<(Ticks, u8), Note> = std::collections::BTreeMap::new();
    for n in snapped {
        let slot = best.entry((n.start, n.pitch)).or_insert(n);
        if n.velocity > slot.velocity || (n.velocity == slot.velocity && n.length > slot.length) {
            *slot = n;
        }
    }
    let mut out: Vec<Note> = best.into_values().collect();
    out.sort_by_key(|n| (n.pitch, n.start));
    for i in 0..out.len().saturating_sub(1) {
        if out[i + 1].pitch == out[i].pitch {
            out[i].length = out[i].length.min(out[i + 1].start - out[i].start);
        }
    }
    out.sort_by_key(|n| (n.start, n.pitch));
    SmartQuantized {
        notes: out,
        grid: label,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(id: u32, pitch: u8, start: Ticks, length: Ticks, velocity: u8) -> Note {
        Note {
            id,
            pitch,
            start,
            length,
            velocity,
        }
    }

    #[test]
    fn loose_sixteenths_snap_to_sixteenths() {
        let jitter = [0, 18, -14, 9, -20, 12, 0, -7];
        let notes: Vec<Note> = jitter
            .iter()
            .enumerate()
            .map(|(i, j)| {
                n(
                    i as u32 + 1,
                    60,
                    (i as Ticks * PPQ / 4 + j).max(0),
                    PPQ / 4 - 10,
                    100,
                )
            })
            .collect();
        let out = smart_quantize(&notes, 120.0);
        assert_eq!(out.grid, "1/16");
        assert_eq!(
            out.notes.iter().map(|n| n.start).collect::<Vec<_>>(),
            (0..8).map(|i| i * PPQ / 4).collect::<Vec<_>>()
        );
        assert!(out.notes.iter().all(|n| n.length == PPQ / 4));
        assert_eq!(
            smart_quantize(&out.notes, 120.0).notes,
            out.notes,
            "already quantized"
        );
    }

    #[test]
    fn triplets_and_eighths_are_found() {
        let trip: Vec<Note> = (0..6)
            .map(|i| n(i + 1, 64, i as Ticks * PPQ / 6 + 7, PPQ / 6, 90))
            .collect();
        assert_eq!(smart_quantize(&trip, 120.0).grid, "1/16T");
        let eighths: Vec<Note> = (0..6)
            .map(|i| {
                n(
                    i + 1,
                    64,
                    i as Ticks * PPQ / 2 - (i as Ticks % 2) * 15,
                    PPQ / 2,
                    90,
                )
            })
            .collect();
        assert_eq!(smart_quantize(&eighths, 120.0).grid, "1/8");
    }

    #[test]
    fn blips_and_ghost_overtones_go() {
        let notes = vec![
            n(1, 48, 0, PPQ, 110),
            n(2, 60, 10, PPQ / 2, 30),
            n(3, 50, PPQ, PPQ, 100),
            n(4, 52, 2 * PPQ, PPQ, 100),
            n(5, 70, 3 * PPQ, 20, 5),
        ];
        let out = smart_quantize(&notes, 120.0);
        assert_eq!(
            out.notes.iter().map(|n| n.id).collect::<Vec<_>>(),
            vec![1, 3, 4],
            "the octave ghost and the blip are removed"
        );
    }

    #[test]
    fn same_pitch_collisions_keep_the_louder_and_lengths_stop_at_the_next() {
        let notes = vec![
            n(1, 60, 0, PPQ, 70),
            n(2, 60, 20, PPQ / 2, 100),
            n(3, 60, PPQ / 2, PPQ, 80),
            n(4, 62, PPQ, PPQ, 80),
        ];
        let out = smart_quantize(&notes, 120.0);
        let c: Vec<&Note> = out.notes.iter().filter(|n| n.pitch == 60).collect();
        assert_eq!(c[0].id, 2);
        assert!(c[0].start + c[0].length <= c[1].start);
    }

    #[test]
    fn few_notes_use_sixteenths() {
        assert_eq!(
            smart_quantize(&[n(1, 60, 130, 200, 90)], 120.0).notes[0].start,
            2 * PPQ / 8
        );
        assert!(smart_quantize(&[], 120.0).notes.is_empty());
    }
}
