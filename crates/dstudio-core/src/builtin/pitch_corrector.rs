//! Pitch Corrector: parameter ids, ranges, defaults, units, the panel
//! arrangement and the factory presets.

use super::*;
use crate::device::PluginKind;

/// Key names, pitch-class order from C.
const KEYS: &[&str] = &["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
const SCALES: &[&str] = &["Chromatic", "Major", "Minor"];
const FOLLOW: &[&str] = &["Manual", "Project"];

pub static PITCH_CORRECTOR: DeviceDef = DeviceDef {
    id: "pitch-corrector",
    name: "Pitch Corrector",
    kind: PluginKind::Effect,
    category: "Pitch",
    purpose: "Automatic pitch correction: pulls a sung or played line onto the notes of a key and scale, from a gentle nudge to a hard, stepped effect.",
    params: &[
        ParamDef::new("speed", "Speed", 0.0, 100.0, 50.0, LINEAR, "%", Format::Percent)
            .help("How much of the distance to the target note is corrected."),
        ParamDef::choice("key", "Key", KEYS, 0).help("Tonic of the scale the notes are corrected to."),
        ParamDef::choice("scale", "Scale", SCALES, 0)
            .help("Allowed notes: every semitone, or the major or natural minor scale of the key."),
        ParamDef::new("retune_speed_ms", "Retune Speed", 0.0, 400.0, 50.0, LINEAR, "ms", Format::Time)
            .help("Glide time onto the target note; 0 ms jumps at once."),
        ParamDef::new("humanize", "Humanize", 0.0, 100.0, 20.0, LINEAR, "%", Format::Percent)
            .help("Keeps vibrato and natural drift and relaxes the correction on held notes."),
        ParamDef::new("mix", "Mix", 0.0, 100.0, 100.0, LINEAR, "%", Format::Percent).help("Blend of corrected and original signal."),
        ParamDef::choice("key_follow", "Key Follow", FOLLOW, 1)
            .label("Follow")
            .not_automatable()
            .help("Project: use the project's key and scale; Manual: the Key and Scale set here."),
    ],
    layout: Layout {
        picker: &[Control::Menu("key_follow"), Control::Dropdown("key"), Control::Dropdown("scale")],
        view: Some(View::Custom("pitch-trace")),
        sliders: &[],
        clusters: &[&[Control::Knob("speed"), Control::Knob("retune_speed_ms"), Control::Knob("humanize"), Control::Knob("mix")]],
        faces: &[],
        min_height: 202.0,
        shown_when: &[],
        fixed_width: 375.0,
        inert_when: &[],
        views: &[],
        spread_dials: false,
    },
    factory: &[
        FactoryPreset { name: "Robot Snap", values: &[("speed", 100.0), ("retune_speed_ms", 2.0), ("humanize", 0.0)] },
        FactoryPreset { name: "Studio Polish", values: &[("speed", 75.0), ("retune_speed_ms", 70.0), ("humanize", 30.0)] },
        FactoryPreset { name: "Light Touch", values: &[("speed", 35.0), ("retune_speed_ms", 120.0), ("humanize", 40.0)] },
        FactoryPreset { name: "Pop Lead", values: &[("speed", 95.0), ("retune_speed_ms", 25.0), ("humanize", 10.0)] },
        FactoryPreset { name: "Lazy Glide", values: &[("speed", 85.0), ("retune_speed_ms", 220.0), ("humanize", 45.0)] },
        FactoryPreset { name: "Pitched Double", values: &[("speed", 100.0), ("retune_speed_ms", 10.0), ("humanize", 5.0), ("mix", 40.0)] },
    ],
    sidechain: false,
    samples: 0,
};
