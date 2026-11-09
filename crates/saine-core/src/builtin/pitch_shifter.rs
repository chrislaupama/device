//! Pitch Shifter (reference: the reference app's built-in "Pitch Shifter"). Parameter
//! ids, ranges, defaults and the panel arrangement are taken from the
//! reference as facts; factory preset names and values are our own.

use super::*;
use crate::device::PluginKind;

const OFF_ON: &[&str] = &["Off", "On"];

/// "+7 st", "−12 st", "+0 st".
pub fn semitones_text(v: f32) -> String {
    signed_int(v, "st")
}

/// "+20 ct", "−7 ct".
pub fn cents_text(v: f32) -> String {
    signed_int(v, "ct")
}

pub(crate) fn signed_int(v: f32, unit: &str) -> String {
    let r = v.round();
    if r < 0.0 { format!("\u{2212}{} {unit}", -r as i32) } else { format!("+{} {unit}", r as i32) }
}

/// Interval caption of a shift: "Unison", "Fifth up", "Octave down",
/// "Fifth up +12 ct", "+35 ct".
pub fn interval_text(semitones: f32, cents: f32) -> String {
    const NAMES: [&str; 12] = ["Unison", "Minor 2nd", "Major 2nd", "Minor 3rd", "Major 3rd", "Fourth", "Tritone", "Fifth", "Minor 6th", "Major 6th", "Minor 7th", "Major 7th"];
    let st = semitones.round() as i32;
    let ct = cents.round() as i32;
    let a = st.unsigned_abs() as usize;
    let name = match (a % 12, a / 12) {
        (0, 0) => String::new(),
        (0, 1) => "Octave".into(),
        (0, n) => format!("{n} octaves"),
        (i, 0) => NAMES[i].into(),
        (i, 1) => format!("Octave + {}", NAMES[i].to_lowercase()),
        (i, n) => format!("{n} oct + {}", NAMES[i].to_lowercase()),
    };
    let dir = if st > 0 { " up" } else { " down" };
    match (name.is_empty(), ct) {
        (true, 0) => "Unison".into(),
        (true, c) => signed_int(c as f32, "ct"),
        (false, 0) => format!("{name}{dir}"),
        (false, c) => format!("{name}{dir} {}", signed_int(c as f32, "ct")),
    }
}

pub static PITCH_SHIFTER: DeviceDef = DeviceDef {
    id: "pitch-shifter",
    name: "Pitch Shifter",
    kind: PluginKind::Effect,
    category: "Pitch",
    purpose: "Moves the pitch up or down by semitones and cents, optionally keeping a voice's formants in place, with a dry/wet blend.",
    params: &[
        ParamDef::new("semitones", "Semitones", -24.0, 24.0, 0.0, LINEAR, "st", Format::Custom(TextFns { show: semitones_text, read: None }))
            .step(1.0)
            .help("Shift in semitones."),
        ParamDef::new("cents", "Cents", -100.0, 100.0, 0.0, LINEAR, "ct", Format::Custom(TextFns { show: cents_text, read: None })).help("Fine shift in cents."),
        ParamDef::choice("formant_preserve", "Formant", OFF_ON, 0).help("Keeps the spectral envelope (a voice's character) in place while the pitch moves."),
        ParamDef::new("mix", "Mix", 0.0, 100.0, 100.0, LINEAR, "%", Format::Percent).help("Blend of shifted and dry signal."),
    ],
    layout: Layout {
        view: Some(View::Custom("pitch-shifter")),
        clusters: &[&[Control::Knob("semitones"), Control::Knob("cents")], &[Control::Toggle("formant_preserve")], &[Control::Knob("mix")]],
        min_height: 220.0,
        fixed_width: 332.0,
        spread_dials: true,
        ..Layout::EMPTY
    },
    factory: &[
        FactoryPreset { name: "Up an Octave, Natural", values: &[("semitones", 12.0), ("formant_preserve", 1.0)] },
        FactoryPreset { name: "Down an Octave", values: &[("semitones", -12.0)] },
        FactoryPreset { name: "Major Third Above", values: &[("semitones", 4.0), ("formant_preserve", 1.0), ("mix", 45.0)] },
        FactoryPreset { name: "Fifth Harmony", values: &[("semitones", 7.0), ("formant_preserve", 1.0), ("mix", 50.0)] },
        FactoryPreset { name: "Low Fifth Bed", values: &[("semitones", -7.0), ("mix", 35.0)] },
        FactoryPreset { name: "Sub Weight", values: &[("semitones", -12.0), ("mix", 30.0)] },
        FactoryPreset { name: "Wide Detune", values: &[("cents", 11.0), ("mix", 40.0)] },
        FactoryPreset { name: "Slight Flat Double", values: &[("cents", -9.0), ("mix", 45.0)] },
        FactoryPreset { name: "Helium", values: &[("semitones", 9.0)] },
        FactoryPreset { name: "Giant", values: &[("semitones", -10.0)] },
        FactoryPreset { name: "Lower Register Voice", values: &[("semitones", -4.0), ("formant_preserve", 1.0)] },
        FactoryPreset { name: "Two Octaves Up", values: &[("semitones", 24.0), ("mix", 25.0)] },
    ],
    sidechain: false,
    hooks: Hooks::NONE,
    samples: 0,
};
