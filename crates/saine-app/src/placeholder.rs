//! PLACEHOLDER content for parts of the shell that have no backing feature yet.
//!
//! The browser's Devices tree reproduces the *layout* of the reference (row
//! heights, indentation, counts column) with our own names and made-up
//! counts. Nothing here is loaded from a catalogue; clicking rows does
//! nothing. Listed in crates/saine-app/FIDELITY.md.

use crate::TreeRow;

/// (depth, label, count, icon: 0 folder / 1 keys, expander: 0 none / 1 closed / 2 open)
const ROWS: &[(i32, &str, &str, i32, i32)] = &[
    (0, "Saine", "5", 0, 2),
    (1, "Synth", "100", 1, 2),
    (2, "Bass", "17", 0, 1),
    (2, "Keys", "12", 0, 1),
    (2, "Pluck", "12", 0, 1),
    (2, "Lead", "12", 0, 1),
    (2, "Pad", "13", 0, 1),
    (2, "Brass", "7", 0, 1),
    (2, "Motion", "8", 0, 1),
    (2, "Texture", "7", 0, 1),
    (2, "FX", "6", 0, 1),
    (2, "Drums", "6", 0, 1),
    (1, "Wavetable", "229", 1, 1),
    (1, "Drum Rack", "", 1, 0),
    (1, "Sampler", "", 1, 0),
];

pub fn device_tree() -> Vec<TreeRow> {
    ROWS.iter()
        .map(|&(depth, label, count, icon, expander)| TreeRow { depth, label: label.into(), count: count.into(), icon, expander })
        .collect()
}
