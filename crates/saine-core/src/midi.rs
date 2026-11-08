//! MIDI clips, notes and the pure note transforms behind the piano-roll
//! toolbar (reference: Quantize · Humanize · Reverse · Legato · Double, plus
//! transpose shortcuts). Transforms never mutate the project; the editor
//! wraps their result in `Command::SetMidiNotes`, so every edit is one
//! validated, invertible undo step.

use crate::model::{ClipId, Ticks, PPQ};
use serde::{Deserialize, Serialize};

/// A note inside a MIDI clip. `start` is relative to the clip start.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Note {
    /// Stable within its clip (selection survives edits/undo).
    pub id: u32,
    /// MIDI note number 0..=127 (60 = C4, matching the reference's "C4" label).
    pub pitch: u8,
    pub start: Ticks,
    pub length: Ticks,
    /// 1..=127.
    pub velocity: u8,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MidiClip {
    pub id: ClipId,
    pub name: String,
    /// Timeline position (ticks).
    pub start: Ticks,
    /// Clip duration (ticks). Notes beyond it are kept but not played.
    pub length: Ticks,
    pub notes: Vec<Note>,
}

impl MidiClip {
    /// Reference default: "Insert empty MIDI clip" creates a 1-bar clip named "MIDI".
    pub fn empty(start: Ticks, length: Ticks) -> Self {
        MidiClip { id: ClipId(0), name: "MIDI".into(), start, length, notes: Vec::new() }
    }

    pub fn end(&self) -> Ticks {
        self.start + self.length
    }

    pub fn next_note_id(&self) -> u32 {
        self.notes.iter().map(|n| n.id + 1).max().unwrap_or(1)
    }
}

/// Sort notes canonically (start, pitch, id) for deterministic storage.
pub fn sort_notes(notes: &mut [Note]) {
    notes.sort_by_key(|n| (n.start, n.pitch, n.id));
}

pub fn validate_notes(notes: &[Note]) -> Result<(), String> {
    let mut ids = std::collections::HashSet::new();
    for n in notes {
        if n.pitch > 127 {
            return Err(format!("note {}: pitch {} > 127", n.id, n.pitch));
        }
        if n.velocity == 0 || n.velocity > 127 {
            return Err(format!("note {}: velocity {} outside 1..=127", n.id, n.velocity));
        }
        if n.start < 0 || n.length <= 0 {
            return Err(format!("note {}: start must be >= 0 and length > 0", n.id));
        }
        if !ids.insert(n.id) {
            return Err(format!("duplicate note id {}", n.id));
        }
    }
    Ok(())
}

/// Common grid values in ticks.
pub fn grid_ticks(denominator: u32) -> Ticks {
    PPQ * 4 / denominator.max(1) as Ticks
}

/// Move note starts toward the nearest grid line by `amount` (0..=1, the
/// reference's "Amount 100%"). Lengths are kept; start stays >= 0.
pub fn quantize(notes: &[Note], grid: Ticks, amount: f64) -> Vec<Note> {
    let amount = amount.clamp(0.0, 1.0);
    let grid = grid.max(1);
    notes
        .iter()
        .map(|n| {
            let target = ((n.start as f64 / grid as f64).round() as Ticks) * grid;
            let start = n.start + ((target - n.start) as f64 * amount).round() as Ticks;
            Note { start: start.max(0), ..*n }
        })
        .collect()
}

/// Shift pitch by `semitones`. Fails (returns None) if any note would leave 0..=127.
pub fn transpose(notes: &[Note], semitones: i32) -> Option<Vec<Note>> {
    notes
        .iter()
        .map(|n| {
            let p = n.pitch as i32 + semitones;
            (0..=127).contains(&p).then_some(Note { pitch: p as u8, ..*n })
        })
        .collect()
}

/// Deterministic humanize: offsets start by up to ±`timing` ticks and velocity
/// by up to ±`velocity` using a seeded xorshift, so results are reproducible
/// (tests, undo/redo, AI replays). `amount` scales both (reference "Amount 50%").
pub fn humanize(notes: &[Note], amount: f64, seed: u64) -> Vec<Note> {
    let mut s = seed | 1;
    let mut rnd = move || {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        (s >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0 // -1..1
    };
    let a = amount.clamp(0.0, 1.0);
    let timing = (PPQ as f64 / 16.0) * a; // up to a 1/64 note
    let vel = 20.0 * a;
    notes
        .iter()
        .map(|n| {
            let dt = (rnd() * timing).round() as Ticks;
            let dv = (rnd() * vel).round() as i32;
            Note { start: (n.start + dt).max(0), velocity: (n.velocity as i32 + dv).clamp(1, 127) as u8, ..*n }
        })
        .collect()
}

/// Mirror notes in time within `clip_length`.
pub fn reverse(notes: &[Note], clip_length: Ticks) -> Vec<Note> {
    notes.iter().map(|n| Note { start: (clip_length - n.start - n.length).max(0), ..*n }).collect()
}

/// Extend each note to the start of the next later note start (any pitch);
/// the last notes extend to the clip end.
pub fn legato(notes: &[Note], clip_length: Ticks) -> Vec<Note> {
    let mut starts: Vec<Ticks> = notes.iter().map(|n| n.start).collect();
    starts.sort_unstable();
    starts.dedup();
    notes
        .iter()
        .map(|n| {
            let next = starts.iter().copied().find(|&s| s > n.start).unwrap_or(clip_length.max(n.start + 1));
            Note { length: (next - n.start).max(1), ..*n }
        })
        .collect()
}

/// Duplicate the clip's content after itself. Returns (notes, new clip length).
pub fn double(notes: &[Note], clip_length: Ticks) -> (Vec<Note>, Ticks) {
    let mut next_id = notes.iter().map(|n| n.id + 1).max().unwrap_or(1);
    let mut out = notes.to_vec();
    for n in notes.iter().filter(|n| n.start < clip_length) {
        out.push(Note { id: next_id, start: n.start + clip_length, ..*n });
        next_id += 1;
    }
    (out, clip_length * 2)
}

/// MIDI note name with the reference's octave convention (60 = C4).
pub fn note_name(pitch: u8) -> String {
    const NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
    format!("{}{}", NAMES[(pitch % 12) as usize], pitch as i32 / 12 - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(id: u32, pitch: u8, start: Ticks, length: Ticks) -> Note {
        Note { id, pitch, start, length, velocity: 100 }
    }

    #[test]
    fn quantize_full_and_partial() {
        let g = grid_ticks(16); // 240
        let notes = [n(1, 60, 250, 100), n(2, 62, 470, 100)];
        let q = quantize(&notes, g, 1.0);
        assert_eq!((q[0].start, q[1].start), (240, 480));
        let h = quantize(&notes, g, 0.5);
        assert_eq!((h[0].start, h[1].start), (245, 475));
    }

    #[test]
    fn transpose_bounds() {
        assert_eq!(transpose(&[n(1, 60, 0, 10)], 12).unwrap()[0].pitch, 72);
        assert!(transpose(&[n(1, 120, 0, 10)], 12).is_none());
    }

    #[test]
    fn humanize_is_deterministic_and_bounded() {
        let notes: Vec<Note> = (0..32).map(|i| n(i, 60, i as Ticks * 240, 100)).collect();
        let a = humanize(&notes, 0.5, 42);
        assert_eq!(a, humanize(&notes, 0.5, 42));
        assert!(a.iter().zip(&notes).all(|(h, o)| (h.start - o.start).abs() <= 30 && (h.velocity as i32 - 100).abs() <= 10));
        assert_ne!(a, notes);
    }

    #[test]
    fn reverse_legato_double() {
        let notes = [n(1, 60, 0, 240), n(2, 64, 960, 240)];
        let r = reverse(&notes, 3840);
        assert_eq!((r[0].start, r[1].start), (3600, 2640));
        let l = legato(&notes, 3840);
        assert_eq!((l[0].length, l[1].length), (960, 2880));
        let (d, len) = double(&notes, 3840);
        assert_eq!(len, 7680);
        assert_eq!(d.len(), 4);
        assert_eq!(d[2].start, 3840);
        assert!(validate_notes(&d).is_ok(), "doubled notes get fresh ids");
    }

    #[test]
    fn validation_and_names() {
        assert!(validate_notes(&[n(1, 60, 0, 10), n(1, 61, 0, 10)]).is_err());
        assert!(validate_notes(&[Note { velocity: 0, ..n(1, 60, 0, 10) }]).is_err());
        assert_eq!(note_name(60), "C4");
        assert_eq!(note_name(61), "C#4");
        assert_eq!(note_name(21), "A0");
    }
}
