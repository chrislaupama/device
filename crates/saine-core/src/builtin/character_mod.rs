//! Character Mod (reference: the reference app's built-in "Character Mod"): one card,
//! eleven modulation circuits (BBD choruses, ensemble, tape, photocell,
//! flanger, phaser, tremolo, rotary speaker, panner), each with its own modes
//! and artifacts. Parameter ids, ranges, defaults, tapers, the per-circuit
//! defaults and the panel arrangement are taken from the reference as facts;
//! the factory presets (names and values) and the circuits' sound are our own.

use super::*;

/// Circuits in the order of the model parameter.
pub const CIRCUITS: &[&str] = &["Single BBD", "Crossed", "Twin BBD", "Ensemble", "Tape", "Photocell", "Flanger", "Phaser", "Tremolo", "Rotary", "Panner"];
pub const SINGLE: u32 = 0;
pub const CROSSED: u32 = 1;
pub const TWIN: u32 = 2;
pub const ENSEMBLE: u32 = 3;
pub const TAPE: u32 = 4;
pub const PHOTOCELL: u32 = 5;
pub const FLANGER: u32 = 6;
pub const PHASER: u32 = 7;
pub const TREMOLO: u32 = 8;
pub const ROTARY: u32 = 9;
pub const PANNER: u32 = 10;

/// The Mode parameter of each circuit (Ensemble has none).
pub const MODE_PARAMS: [Option<&str>; 11] = [
    Some("mode_single"),
    Some("mode_crossed"),
    Some("mode_twin"),
    None,
    Some("mode_tape"),
    Some("mode_photocell"),
    Some("mode_flanger"),
    Some("mode_phaser"),
    Some("mode_trem"),
    Some("mode_rotary"),
    Some("mode_panner"),
];

/// Artifact parameters (Hiss, Drive, Darken, Wow).
pub const ARTIFACTS: [&str; 4] = ["art_hiss", "art_drive", "art_darken", "art_wow"];

/// Default artifact amounts (%) of a circuit (Hiss, Drive, Darken, Wow);
/// every circuit has a default for each, used or not.
pub fn artifact_defaults(circuit: u32) -> [f32; 4] {
    let mut d = [35.0f32; 4];
    match circuit {
        PHOTOCELL => d[2] = 0.0,
        PHASER | ROTARY => d[0] = 0.0,
        TREMOLO => {
            d[0] = 0.0;
            d[2] = 0.0;
        }
        PANNER => {
            d[0] = 0.0;
            d[1] = 0.0;
            d[2] = 0.0;
        }
        _ => {}
    }
    d
}


/// The Rate a circuit (in a mode) starts from, when it has its own.
pub fn rate_default(circuit: u32, mode: u32) -> Option<f32> {
    match circuit {
        CROSSED => [0.25, 0.25, 0.5, 0.5].get(mode as usize).copied(),
        TWIN => [0.513, 0.863, 9.75].get(mode as usize).copied(),
        ENSEMBLE => Some(0.6),
        _ => None,
    }
}

const OFF_ON: &[&str] = &["Off", "On"];

pub static CHARACTER_MOD: DeviceDef = DeviceDef {
    id: "character-mod",
    name: "Character Mod",
    kind: PluginKind::Effect,
    category: "Modulation",
    purpose: "Classic modulation hardware in one box: bucket-brigade choruses, ensemble, tape, photocell vibe, flanger, phaser, tremolo, rotary speaker and panner, with their noise and colour.",
    params: &[
        ParamDef::choice("model", "Character", CIRCUITS, TWIN).not_automatable().help("The circuit: each has its own modes, sweep and artifacts."),
        ParamDef::choice("mode_single", "Mode (Single BBD)", &["Chorus", "Vibrato"], 0).label("Mode").help("Chorus blends a moving copy; Vibrato bends the pitch."),
        ParamDef::choice("mode_crossed", "Mode (Crossed)", &["1", "2", "3", "4"], 0).label("Mode").help("Four cross-wired stereo chorus settings, from subtle to glassy."),
        ParamDef::choice("mode_twin", "Mode (Twin BBD)", &["I", "II", "I+II"], 0).label("Mode").help("Slow, faster, or the fast shallow combination of the twin bucket-brigade chorus."),
        ParamDef::choice("mode_tape", "Mode (Tape)", &["ADT", "TZF Flange", "TZF Phase"], 0).label("Mode").help("Double tracking, or two tape machines flanging through zero."),
        ParamDef::choice("mode_photocell", "Mode (Photocell)", &["Chorus", "Vibrato"], 0).label("Mode").help("Lamp-driven phase shifter blended (Chorus) or alone (Vibrato)."),
        ParamDef::choice("mode_flanger", "Mode (Flanger)", &["Soft", "Jet", "Dark"], 0).label("Mode").help("Gentle, resonant, or a filtered bucket-brigade flanger."),
        ParamDef::choice("mode_phaser", "Mode (Phaser)", &["Four Plain", "Four Peak", "OTA Dark", "Dual Six", "Barber"], 0)
            .label("Mode")
            .help("Four-stage plain or resonant, a dark transconductance phaser, two six-stage phasers, or an endless barber-pole rise."),
        ParamDef::choice("mode_trem", "Mode (Tremolo)", &["Optical", "Bias", "Harmonic"], 0).label("Mode").help("Smooth lamp tremolo, a tube bias pulse, or a two-band harmonic tremolo."),
        ParamDef::choice("mode_rotary", "Mode (Rotary)", &["Slow", "Fast", "Stop"], 0).label("Mode").help("Rotor speed; the rotors spin up and slow down with their own inertia."),
        ParamDef::choice("mode_panner", "Mode (Panner)", &["Pan", "Rotary-Pan"], 0).label("Mode").help("Side-to-side panning, or a source circling the listener."),
        ParamDef::choice("input", "Input", &["Off", "Lo", "Hi"], 0).help("Input level into the circuit: Lo and Hi drive it harder."),
        ParamDef::new("rate", "Rate", 0.02, 20.0, 0.513, Taper::Linear { skew: 0.35 }, "Hz", Format::Freq).help("Modulation speed while the timebase is Free."),
        ParamDef::choice("sync", "Sync", OFF_ON, 0).label("Timebase").help("Off runs at Rate in Hz; On follows the song tempo at Note."),
        ParamDef::choice("note", "Note", MOD_NOTES, 10).help("Modulation cycle as a note value while synced."),
        ParamDef::new("depth", "Depth", 0.0, 100.0, 50.0, LINEAR, "%", Format::Percent).step(0.1).help("Modulation amount: 50% is the circuit's own depth, 100% doubles it."),
        ParamDef::new("mix", "Mix", 0.0, 100.0, 50.0, LINEAR, "%", Format::Percent).step(0.1).help("Balance of the dry and the processed signal."),
        ParamDef::new("width", "Width", 0.0, 100.0, 100.0, LINEAR, "%", Format::Percent).step(0.1).help("Stereo spread of the effect; 0% is mono."),
        ParamDef::new("depth_m", "Depth-M", 0.0, 100.0, 100.0, LINEAR, "%", Format::Percent).step(0.1).help("Level of the effect's centre (mid) part."),
        ParamDef::new("depth_s", "Depth-S", 0.0, 100.0, 100.0, LINEAR, "%", Format::Percent).step(0.1).help("Level of the effect's stereo (side) part."),
        ParamDef::new("drift", "Drift", 0.0, 100.0, 35.0, LINEAR, "%", Format::Percent).step(0.1).help("Slow random wander of the modulation, like ageing parts."),
        ParamDef::new("voice", "Voice", -100.0, 100.0, 0.0, LINEAR, "%", Format::Percent).step(0.1).help("Negative: a brighter, quicker sheen. Positive: a slower, deeper wobble."),
        ParamDef::new("art_hiss", "Hiss", 0.0, 100.0, 35.0, LINEAR, "%", Format::Percent).step(0.1).help("Noise floor of the circuit."),
        ParamDef::new("art_drive", "Drive", 0.0, 100.0, 35.0, LINEAR, "%", Format::Percent).step(0.1).help("Saturation of the circuit."),
        ParamDef::new("art_darken", "Darken", 0.0, 100.0, 35.0, LINEAR, "%", Format::Percent).step(0.1).help("High-frequency loss of the circuit."),
        ParamDef::new("art_wow", "Wow", 0.0, 100.0, 35.0, LINEAR, "%", Format::Percent).step(0.1).help("Slow pitch wander of the tape transport."),
        ParamDef::new("trim", "Output", -12.0, 12.0, 0.0, LINEAR, "dB", Format::Db).step(0.1).help("Output level."),
        ParamDef::new("seed", "Seed", 1.0, 9999.0, 1.0, LINEAR, "", Format::Plain(0)).step(1.0).not_automatable().preset_locked().help("Starting point of the random wander and noise."),
        ParamDef::choice("mono_audition", "Mono Audition", OFF_ON, 0).not_automatable().reset_on_load().help("Listen to the output summed to mono."),
        ParamDef::new("patch", "Patch", 0.0, 1.0, 0.0, LINEAR, "", Format::Plain(2)).not_automatable().help("Internal patch slot."),
    ],
    layout: Layout {
        picker: &[
            Control::Dropdown("model"),
            Control::Dropdown("mode_single"),
            Control::Dropdown("mode_crossed"),
            Control::Dropdown("mode_twin"),
            Control::Dropdown("mode_tape"),
            Control::Dropdown("mode_photocell"),
            Control::Dropdown("mode_flanger"),
            Control::Dropdown("mode_phaser"),
            Control::Dropdown("mode_trem"),
            Control::Dropdown("mode_rotary"),
            Control::Dropdown("mode_panner"),
            Control::Gap,
            Control::Segments("sync", FREE_SYNC),
        ],
        view: Some(View::Field(Field::new("character-mod", "Sweep", &["rate", "note"], &["depth"], 354.0))),
        sliders: &[Control::Slider("trim")],
        clusters: &[&[Control::Swap(&["rate", "note"]), Control::Knob("depth")], &[Control::Knob("width"), Control::Knob("voice")], &[Control::Knob("mix")]],
        faces: &[],
        min_height: 202.0,
        shown_when: &[
            ("mode_single", "model", &[SINGLE]),
            ("mode_crossed", "model", &[CROSSED]),
            ("mode_twin", "model", &[TWIN]),
            ("mode_tape", "model", &[TAPE]),
            ("mode_photocell", "model", &[PHOTOCELL]),
            ("mode_flanger", "model", &[FLANGER]),
            ("mode_phaser", "model", &[PHASER]),
            ("mode_trem", "model", &[TREMOLO]),
            ("mode_rotary", "model", &[ROTARY]),
            ("mode_panner", "model", &[PANNER]),
            // Artifacts a circuit lacks are hidden (no card controls; automation lists).
            ("art_hiss", "model", &[SINGLE, CROSSED, TWIN, ENSEMBLE, TAPE, PHOTOCELL, FLANGER]),
            ("art_drive", "model", &[SINGLE, CROSSED, TWIN, ENSEMBLE, TAPE, PHOTOCELL, FLANGER, PHASER, TREMOLO, ROTARY]),
            ("art_darken", "model", &[SINGLE, CROSSED, TWIN, ENSEMBLE, TAPE, FLANGER, PHASER, ROTARY]),
            ("art_wow", "model", &[TAPE]),
            ("rate", "sync", &[0]),
            ("note", "sync", &[1]),
        ],
    },
    factory: &[
        FactoryPreset { name: "Juno Wash", values: &[("mix", 45.0)] },
        FactoryPreset { name: "Fast Twin Sparkle", values: &[("mode_twin", 2.0), ("rate", 9.75), ("mix", 30.0), ("voice", -50.0)] },
        FactoryPreset { name: "Mono BBD Thicken", values: &[("model", 0.0), ("rate", 0.45), ("depth", 40.0), ("mix", 30.0), ("width", 0.0)] },
        FactoryPreset { name: "Warbly Vibrato", values: &[("model", 0.0), ("mode_single", 1.0), ("rate", 5.0), ("depth", 45.0), ("mix", 100.0), ("art_hiss", 20.0)] },
        FactoryPreset { name: "Cross Glass", values: &[("model", 1.0), ("mode_crossed", 3.0), ("rate", 0.5), ("mix", 35.0), ("trim", -2.0), ("voice", -60.0)] },
        FactoryPreset { name: "Cross Wide Double", values: &[("model", 1.0), ("mode_crossed", 1.0), ("rate", 0.25), ("depth", 55.0), ("mix", 40.0)] },
        FactoryPreset { name: "String Machine", values: &[("model", 3.0), ("rate", 0.6), ("depth", 60.0), ("mix", 55.0), ("drift", 45.0)] },
        FactoryPreset { name: "Tape Double", values: &[("model", 4.0), ("rate", 0.3), ("mix", 30.0), ("drift", 50.0), ("art_wow", 40.0)] },
        FactoryPreset { name: "Zero-Point Flange", values: &[("model", 4.0), ("mode_tape", 1.0), ("rate", 0.09), ("depth", 85.0), ("mix", 50.0)] },
        FactoryPreset { name: "Tape Phase Drift", values: &[("model", 4.0), ("mode_tape", 2.0), ("rate", 0.12), ("depth", 90.0), ("mix", 50.0), ("drift", 60.0)] },
        FactoryPreset { name: "Lamp Throb", values: &[("model", 5.0), ("mode_photocell", 1.0), ("rate", 5.5), ("depth", 55.0), ("mix", 100.0), ("art_darken", 0.0)] },
        FactoryPreset { name: "Vibe Chorus", values: &[("model", 5.0), ("rate", 3.5), ("mix", 50.0), ("art_darken", 0.0), ("trim", 2.0)] },
        FactoryPreset { name: "Soft BBD Flange", values: &[("model", 6.0), ("rate", 0.18), ("depth", 60.0), ("mix", 50.0)] },
        FactoryPreset { name: "Jet Sweep", values: &[("model", 6.0), ("mode_flanger", 1.0), ("rate", 0.22), ("depth", 70.0), ("mix", 50.0)] },
        FactoryPreset { name: "Plain Four Phase", values: &[("model", 7.0), ("rate", 0.6), ("mix", 50.0), ("art_hiss", 0.0)] },
        FactoryPreset { name: "Peaky Phase", values: &[("model", 7.0), ("mode_phaser", 1.0), ("rate", 0.5), ("depth", 60.0), ("art_hiss", 0.0)] },
        FactoryPreset { name: "Endless Rise", values: &[("model", 7.0), ("mode_phaser", 4.0), ("rate", 0.2), ("mix", 55.0), ("width", 70.0), ("art_hiss", 0.0)] },
        FactoryPreset { name: "Lamp Tremolo", values: &[("model", 8.0), ("rate", 4.8), ("depth", 60.0), ("mix", 100.0), ("art_hiss", 0.0), ("art_darken", 0.0), ("trim", -2.5)] },
        FactoryPreset { name: "Two-Band Pulse", values: &[("model", 8.0), ("mode_trem", 2.0), ("rate", 3.8), ("depth", 55.0), ("mix", 100.0), ("art_hiss", 0.0), ("art_darken", 0.0)] },
        FactoryPreset { name: "Eighth Chop", values: &[("model", 8.0), ("mode_trem", 1.0), ("sync", 1.0), ("note", 13.0), ("depth", 90.0), ("mix", 100.0), ("voice", 60.0), ("art_hiss", 0.0), ("art_darken", 0.0)] },
        FactoryPreset { name: "Organ Speaker Slow", values: &[("model", 9.0), ("rate", 6.5), ("mix", 100.0), ("art_hiss", 0.0), ("trim", 3.5)] },
        FactoryPreset { name: "Organ Speaker Fast", values: &[("model", 9.0), ("mode_rotary", 1.0), ("rate", 6.5), ("mix", 100.0), ("art_hiss", 0.0), ("trim", 3.5)] },
        FactoryPreset { name: "Side to Side", values: &[("model", 10.0), ("rate", 1.5), ("depth", 75.0), ("mix", 100.0), ("art_hiss", 0.0), ("art_drive", 0.0), ("art_darken", 0.0)] },
        FactoryPreset { name: "Circling Source", values: &[("model", 10.0), ("mode_panner", 1.0), ("rate", 0.35), ("depth", 90.0), ("mix", 100.0), ("art_hiss", 0.0), ("art_drive", 0.0), ("art_darken", 0.0)] },
    ],
    sidechain: false,
};

/// Circuit switch writes (reference behaviour): changing the circuit or its
/// mode moves every artifact that still sits within 0.01 of the old
/// circuit's default to the new circuit's default, and Rate when it sits
/// within 0.001 of the old circuit's own rate. Edited values stay.
pub fn switch_writes(def: &DeviceDef, device: &Device, param: &str, value: f32) -> (Vec<(&'static str, f32)>, Option<String>) {
    let get = |id: &str| def.param(id).map(|p| def.value(device, p)).unwrap_or(0.0);
    let circuit = get("model").round() as u32;
    let mode_of = |c: u32| MODE_PARAMS.get(c as usize).copied().flatten().map(|m| get(m).round() as u32).unwrap_or(0);
    let (old, new, change) = if param == "model" {
        let c = value.round() as u32;
        ((circuit, mode_of(circuit)), (c, mode_of(c)), "Character Change")
    } else if MODE_PARAMS.get(circuit as usize).copied().flatten() == Some(param) {
        ((circuit, mode_of(circuit)), (circuit, value.round() as u32), "Mode Change")
    } else {
        return (Vec::new(), None);
    };
    if old == new {
        return (Vec::new(), None);
    }
    let mut writes = Vec::new();
    let (from, to) = (artifact_defaults(old.0), artifact_defaults(new.0));
    for (i, id) in ARTIFACTS.iter().enumerate() {
        let (a, b) = (from[i], to[i]);
        if (get(id) - a).abs() <= 0.01 && a != b {
            writes.push((*id, b));
        }
    }
    if let (Some(a), Some(b)) = (rate_default(old.0, old.1), rate_default(new.0, new.1))
        && (get("rate") - a).abs() <= 0.001
        && a != b
    {
        writes.push(("rate", b));
    }
    if writes.is_empty() {
        return (writes, None);
    }
    let names: Vec<&str> = writes.iter().filter_map(|(id, _)| def.param(id).map(|p| p.name)).collect();
    (writes, Some(format!("{change}, with {} moved to its own defaults", names.join(", "))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switching_circuits_moves_untouched_defaults_only() {
        let def = &CHARACTER_MOD;
        let mut d = def.instantiate();
        // Twin BBD -> Phaser: Hiss 35 -> 0 (untouched), Rate has no Phaser default.
        let (w, label) = switch_writes(def, &d, "model", PHASER as f32);
        assert_eq!(w, vec![("art_hiss", 0.0)]);
        assert_eq!(label.as_deref(), Some("Character Change, with Hiss moved to its own defaults"));
        // Twin I -> Twin II moves Rate 0.513 -> 0.863.
        assert_eq!(switch_writes(def, &d, "mode_twin", 1.0).0, vec![("rate", 0.863)]);
        // An edited Rate stays.
        d.params.insert("rate".into(), 2.0);
        assert!(switch_writes(def, &d, "mode_twin", 1.0).0.is_empty());
        // Twin -> Panner: Hiss and Darken move to 0; the edited Drive stays.
        d.params.insert("art_drive".into(), 60.0);
        let ids: Vec<&str> = switch_writes(def, &d, "model", PANNER as f32).0.iter().map(|w| w.0).collect();
        assert_eq!(ids, ["art_hiss", "art_darken"]);
        // A mode of another circuit is no switch.
        assert!(switch_writes(def, &d, "mode_tape", 1.0).0.is_empty());
    }

    #[test]
    fn rate_skew_matches_the_reference() {
        let rate = CHARACTER_MOD.param("rate").unwrap();
        assert!((rate.to_normalized(0.513) - 0.2737).abs() < 1e-3);
        assert!((rate.from_normalized(0.2737) - 0.513).abs() < 2e-3);
    }
}
