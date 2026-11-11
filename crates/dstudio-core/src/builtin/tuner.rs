//! Tuner: parameter ids, choices, defaults and the panel arrangement. It
//! ships no factory presets.

use super::*;
use crate::device::PluginKind;

const REFERENCE: &[&str] = &["440 Hz", "432 Hz"];
const TUNINGS: &[&str] = &["Chromatic", "Standard", "Drop D", "DADGAD", "Sweetened"];
const OFF_ON: &[&str] = &["Off", "On"];

/// Open strings (MIDI, low to high) of each tuning; Chromatic has none.
pub const TUNING_STRINGS: [&[u8]; 5] = [&[], &[40, 45, 50, 55, 59, 64], &[38, 45, 50, 55, 59, 64], &[38, 45, 50, 55, 57, 62], &[40, 45, 50, 55, 59, 64]];

pub static TUNER: DeviceDef = DeviceDef {
    id: "tuner",
    name: "Tuner",
    kind: PluginKind::Effect,
    category: "Analyzer",
    purpose: "Shows which note is played and how many cents it is off; audio passes through unless Mute is on.",
    params: &[
        ParamDef::choice("reference", "Reference", REFERENCE, 0).label("A4").help("Pitch of A4 the notes are measured against."),
        ParamDef::choice("tuning", "Tuning", TUNINGS, 0).help("Chromatic names the nearest note; a guitar tuning names the nearest string."),
        ParamDef::choice("mute_output", "Mute Output", OFF_ON, 0).label("Mute").help("Silences the output while tuning; the reading keeps working."),
    ],
    layout: Layout {
        picker: &[Control::Dropdown("tuning"), Control::Menu("reference"), Control::Gap, Control::Latch("mute_output")],
        view: Some(View::Custom("tuner")),
        sliders: &[],
        clusters: &[],
        faces: &[],
        min_height: 202.0,
        shown_when: &[],
        fixed_width: 335.0,
        inert_when: &[],
        views: &[],
        spread_dials: false,
    },
    factory: &[],
    sidechain: false,
    samples: 0,
};
