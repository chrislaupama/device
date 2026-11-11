//! Per-clip scale view of the piano roll: the root and scale type the editor shows, the
//! footer's Scale highlight and the scale-only rows. It changes no notes.

use crate::model::ProjectKey;
use serde::{Deserialize, Serialize};

/// Root note spellings, in the order the Root menu lists them.
pub const ROOTS: [&str; 17] = ["C", "C#", "Db", "D", "D#", "Eb", "E", "F", "F#", "Gb", "G", "G#", "Ab", "A", "A#", "Bb", "B"];
const ROOT_PCS: [u8; 17] = [0, 1, 1, 2, 3, 3, 4, 5, 6, 6, 7, 8, 8, 9, 10, 10, 11];

/// Scale types, in the order the Scale type menu lists them.
pub const SCALES: [&str; 17] = [
    "Major",
    "Minor",
    "Dorian",
    "Phrygian",
    "Lydian",
    "Mixolydian",
    "Locrian",
    "Harmonic Minor",
    "Melodic Minor",
    "Harmonic Major",
    "Phrygian Dominant",
    "Major Pentatonic",
    "Minor Pentatonic",
    "Blues",
    "Whole Tone",
    "Diminished (Whole–Half)",
    "Diminished (Half–Whole)",
];
/// Semitones above the root of each scale type.
const STEPS: [&[u8]; 17] = [
    &[0, 2, 4, 5, 7, 9, 11],
    &[0, 2, 3, 5, 7, 8, 10],
    &[0, 2, 3, 5, 7, 9, 10],
    &[0, 1, 3, 5, 7, 8, 10],
    &[0, 2, 4, 6, 7, 9, 11],
    &[0, 2, 4, 5, 7, 9, 10],
    &[0, 1, 3, 5, 6, 8, 10],
    &[0, 2, 3, 5, 7, 8, 11],
    &[0, 2, 3, 5, 7, 9, 11],
    &[0, 2, 4, 5, 7, 8, 11],
    &[0, 1, 4, 5, 7, 8, 10],
    &[0, 2, 4, 7, 9],
    &[0, 3, 5, 7, 10],
    &[0, 3, 5, 6, 7, 10],
    &[0, 2, 4, 6, 8, 10],
    &[0, 2, 3, 5, 6, 8, 9, 11],
    &[0, 1, 3, 4, 6, 7, 9, 10],
];

/// A root (index into [ROOTS]) and a scale type (index into [SCALES]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScaleKey {
    pub root: u8,
    pub kind: u8,
}

/// What a MIDI clip keeps of its scale view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ClipScale {
    /// Set once a root or scale type is picked for the clip; until then it follows the project key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<ScaleKey>,
    /// The footer's Scale switch: in-scale rows and keys are highlighted.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub highlight: bool,
    /// The Scale button above the keys: only scale rows (and rows holding notes) show.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub fold: bool,
}

impl Default for ScaleKey {
    fn default() -> Self {
        Self::C_MAJOR
    }
}

impl ScaleKey {
    pub const C_MAJOR: ScaleKey = ScaleKey { root: 0, kind: 0 };

    /// A project key spelled with sharps; a project without a key shows C Major.
    pub fn from_project(key: Option<ProjectKey>) -> Self {
        key.map_or(Self::C_MAJOR, |k| ScaleKey { root: ROOT_PCS.iter().position(|&pc| pc == k.root % 12).unwrap_or(0) as u8, kind: u8::from(k.minor) })
    }

    pub fn validate(&self) -> Result<(), String> {
        if (self.root as usize) < ROOTS.len() && (self.kind as usize) < SCALES.len() { Ok(()) } else { Err(format!("scale key out of range: {self:?}")) }
    }

    /// Pitch class of the root, 0 = C.
    pub fn root_pc(&self) -> u8 {
        ROOT_PCS[self.root as usize % ROOTS.len()]
    }

    pub fn root_name(&self) -> &'static str {
        ROOTS[self.root as usize % ROOTS.len()]
    }

    pub fn kind_name(&self) -> &'static str {
        SCALES[self.kind as usize % SCALES.len()]
    }

    pub fn contains(&self, pitch: i32) -> bool {
        let rel = (pitch - self.root_pc() as i32).rem_euclid(12) as u8;
        STEPS[self.kind as usize % STEPS.len()].contains(&rel)
    }

    pub fn is_root(&self, pitch: i32) -> bool {
        pitch.rem_euclid(12) == self.root_pc() as i32
    }
}

impl ClipScale {
    /// The key the editor shows: the clip's own, else the project key.
    pub fn key_or(scale: Option<&ClipScale>, project: Option<ProjectKey>) -> ScaleKey {
        scale.and_then(|s| s.key).unwrap_or_else(|| ScaleKey::from_project(project))
    }

    pub fn validate(&self) -> Result<(), String> {
        self.key.map_or(Ok(()), |k| k.validate())
    }
}

/// The pitches the scale-only view shows, top row first: every scale pitch, plus the
/// pitches of any notes outside the scale.
pub fn fold_rows(key: ScaleKey, note_pitches: impl IntoIterator<Item = u8>) -> Vec<u8> {
    let mut show = [false; 128];
    for p in 0..128 {
        show[p] = key.contains(p as i32);
    }
    for p in note_pitches {
        show[p as usize & 127] = true;
    }
    (0..128u8).rev().filter(|&p| show[p as usize]).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_key_spells_with_sharps_and_defaults_to_c_major() {
        assert_eq!(ScaleKey::from_project(None), ScaleKey::C_MAJOR);
        let k = ScaleKey::from_project(Some(ProjectKey { root: 3, minor: true }));
        assert_eq!((k.root_name(), k.kind_name(), k.root_pc()), ("D#", "Minor", 3));
        let own = ClipScale { key: Some(ScaleKey { root: 5, kind: 13 }), ..Default::default() };
        assert_eq!(ClipScale::key_or(Some(&own), Some(ProjectKey { root: 0, minor: false })).kind_name(), "Blues");
    }

    #[test]
    fn membership_and_fold_rows() {
        let eb_blues = ScaleKey { root: 5, kind: 13 };
        let pcs: Vec<i32> = (0..12).filter(|&p| eb_blues.contains(p)).collect();
        assert_eq!(pcs, vec![1, 3, 6, 8, 9, 10]);
        assert!(eb_blues.is_root(63) && !eb_blues.is_root(64));
        let rows = fold_rows(ScaleKey::C_MAJOR, [61u8]);
        assert_eq!(rows.len(), 75 + 1, "75 white keys in 0..=127 plus the C#4 note");
        assert!(rows.windows(2).all(|w| w[0] > w[1]) && rows.contains(&61) && !rows.contains(&63));
    }

    #[test]
    fn every_scale_starts_on_the_root_and_climbs() {
        for (name, steps) in SCALES.iter().zip(STEPS) {
            assert_eq!(steps[0], 0, "{name}");
            assert!(steps.windows(2).all(|w| w[0] < w[1]) && *steps.last().unwrap() < 12, "{name}");
        }
        assert!(ScaleKey { root: 17, kind: 0 }.validate().is_err() && ScaleKey { root: 0, kind: 17 }.validate().is_err());
    }
}

