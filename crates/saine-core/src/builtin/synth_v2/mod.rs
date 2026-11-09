//! The built-in instrument "Synth" (reference id synth-v2): a polyphonic
//! subtractive synthesizer with two oscillators, sub and noise sources, a
//! multimode filter, two envelopes, two LFOs, a 32-slot modulation matrix,
//! 8 macros with 16 mappings, unison and four effect slots.
//!
//! Parameters, ranges, defaults and the panel pages follow the device spec
//! (handoffs/device-specs/synth-v2.md); the DSP is our own design
//! (engine/src/devices/synth_v2.cpp). Factory presets are our own sounds.
//!
//! Modulation (shared by the engine and the panel):
//! * a route is [source, target, amount -1..1, on]; its offset is
//!   amount × source × the target's depth ([TARGET_DEPTH]: semitones for
//!   pitch targets, octaves for cutoff / high-pass / LFO rates, the target's
//!   0..1 control range otherwise);
//! * a macro mapping holds its target parameter while the macro is assigned:
//!   the target's normalized value is from + (to − from) × macro.

mod legacy;
mod params;

pub use legacy::{device_from_legacy, upgrade_project, upgrade_track, values_from_legacy};
pub use params::*;

use super::{DeviceDef, FactoryPreset, Layout, View};
use crate::device::PluginKind;

/// Factory sound names: the preset selector parameter's choices ("None",
/// our sounds, then numbered placeholders up to 128 entries).
pub const PRESETS: &[&str] = &[
    "None", "Porch Light Keys", "Velour Pad", "Basement Sub", "Rubber Acid", "Seven Saws", "Pebble Pluck",
    "Copper Horns", "Static Bloom", "Glide Ribbon", "Tin Chime", "Wobblefoot", "Hollow Choir",
    "Preset 13", "Preset 14", "Preset 15", "Preset 16", "Preset 17", "Preset 18", "Preset 19",
    "Preset 20", "Preset 21", "Preset 22", "Preset 23", "Preset 24", "Preset 25", "Preset 26",
    "Preset 27", "Preset 28", "Preset 29", "Preset 30", "Preset 31", "Preset 32", "Preset 33",
    "Preset 34", "Preset 35", "Preset 36", "Preset 37", "Preset 38", "Preset 39", "Preset 40",
    "Preset 41", "Preset 42", "Preset 43", "Preset 44", "Preset 45", "Preset 46", "Preset 47",
    "Preset 48", "Preset 49", "Preset 50", "Preset 51", "Preset 52", "Preset 53", "Preset 54",
    "Preset 55", "Preset 56", "Preset 57", "Preset 58", "Preset 59", "Preset 60", "Preset 61",
    "Preset 62", "Preset 63", "Preset 64", "Preset 65", "Preset 66", "Preset 67", "Preset 68",
    "Preset 69", "Preset 70", "Preset 71", "Preset 72", "Preset 73", "Preset 74", "Preset 75",
    "Preset 76", "Preset 77", "Preset 78", "Preset 79", "Preset 80", "Preset 81", "Preset 82",
    "Preset 83", "Preset 84", "Preset 85", "Preset 86", "Preset 87", "Preset 88", "Preset 89",
    "Preset 90", "Preset 91", "Preset 92", "Preset 93", "Preset 94", "Preset 95", "Preset 96",
    "Preset 97", "Preset 98", "Preset 99", "Preset 100", "Preset 101", "Preset 102", "Preset 103",
    "Preset 104", "Preset 105", "Preset 106", "Preset 107", "Preset 108", "Preset 109", "Preset 110",
    "Preset 111", "Preset 112", "Preset 113", "Preset 114", "Preset 115", "Preset 116", "Preset 117",
    "Preset 118", "Preset 119", "Preset 120", "Preset 121", "Preset 122", "Preset 123", "Preset 124",
    "Preset 125", "Preset 126", "Preset 127",
];

pub const MOD_SLOTS: usize = 32;
pub const MACROS: usize = 8;
pub const MAP_SLOTS: usize = 16;
/// Most modulation sources (routes + a macro) one control takes.
pub const MAX_SOURCES_PER_TARGET: usize = 3;

/// Modulation sources (index = MOD_SOURCES index).
pub mod source {
    pub const NONE: usize = 0;
    pub const ENV2: usize = 1;
    pub const AMP_ENV: usize = 2;
    pub const LFO1: usize = 3;
    pub const LFO2: usize = 4;
    pub const VELOCITY: usize = 5;
    pub const KEY: usize = 6;
    pub const MOD_WHEEL: usize = 7;
    pub const AFTERTOUCH: usize = 8;
    pub const RANDOM: usize = 9;
}

/// Sources whose output swings around zero (the panel shows ±|amount|);
/// the others are one-sided.
pub fn bipolar_source(s: usize) -> bool {
    matches!(s, source::LFO1 | source::LFO2 | source::KEY | source::RANDOM)
}

/// Depth of each modulation target at amount 1 (index = MOD_TARGETS index).
pub const TARGET_DEPTH: [f32; 30] = [0.0, 24.0, 24.0, 24.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 8.0, 1.0, 1.0, 6.0, 1.0, 1.0, 1.0, 4.0, 4.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0];

/// The parameter a modulation target moves (None for Pitch, Amp and Pan,
/// which have no knob of their own, and for FX targets: see [fx_target_param]).
pub fn target_param(target: usize) -> Option<&'static str> {
    Some(match target {
        2 => "osc1_semi",
        3 => "osc2_semi",
        4 => "osc1_shape",
        5 => "osc2_shape",
        6 => "osc1_level",
        7 => "osc2_level",
        8 => "cross_fm",
        9 => "sub_level",
        10 => "noise_level",
        11 => "cutoff",
        12 => "resonance",
        13 => "filter_drive",
        14 => "high_pass",
        17 => "unison_detune",
        18 => "lfo1_rate",
        19 => "lfo2_rate",
        20 => "lfo1_depth",
        21 => "lfo2_depth",
        _ => return None,
    })
}

/// The "Amount" parameter of an effect type (FX n Amount targets move it).
pub fn fx_amount_param(fx_type: usize) -> &'static str {
    ["tape_drive", "distortion_drive", "bitcrusher_downsample", "chorus_depth", "flanger_depth", "phaser_depth", "filter_cutoff", "delay_feedback", "reverb_size", "compressor_threshold", "widener_width"][fx_type.min(10)]
}

/// Parameter id of an effect slot (1..=4) setting, e.g. ("mix") -> "fx2_mix".
pub fn fx_param(slot: usize, name: &str) -> String {
    format!("fx{slot}_{name}")
}

/// Modulation target index of a parameter, if it has one (the inverse of
/// [target_param] plus the FX mix / amount targets).
pub fn target_of(param: &str, fx_types: [usize; 4]) -> Option<usize> {
    if let Some(t) = (0..30).find(|&t| target_param(t) == Some(param)) {
        return Some(t);
    }
    let rest = param.strip_prefix("fx")?;
    let slot: usize = rest.get(..1)?.parse().ok()?;
    let name = rest.get(2..)?;
    if !(1..=4).contains(&slot) {
        return None;
    }
    if name == "mix" {
        return Some(21 + slot);
    }
    let ty = fx_types[slot - 1];
    (fx_amount_param(ty) == name).then_some(25 + slot)
}

/// Ids of modulation slot n (1..=32): source, target, amount, on.
pub fn route_ids(n: usize) -> [String; 4] {
    ["source", "target", "amount", "on"].map(|k| format!("mod{n}_{k}"))
}

/// Ids of macro mapping n (1..=16): macro, param, from, to, late param.
pub fn map_ids(n: usize) -> [String; 5] {
    ["macro", "param", "from", "to", "late_param"].map(|k| format!("map{n}_{k}"))
}

/// Index of the first parameter a "Map n Parameter" choice can't name (the
/// choice list covers parameters 0..=500; later ones go through the late list).
const MAP_PARAM_SPAN: usize = 501;

/// The parameter index a mapping slot targets, from its "param" and
/// "late_param" choices (late wins when set).
pub fn map_target(param_choice: usize, late_choice: usize) -> Option<usize> {
    if late_choice > 0 {
        let id = LATE_PARAMS.get(late_choice).copied()?;
        return PARAMS.iter().position(|p| p.name == id);
    }
    (param_choice > 0 && param_choice <= MAP_PARAM_SPAN).then(|| param_choice - 1)
}

/// The (param, late) choices that name parameter index i.
pub fn map_choices_for(i: usize) -> Option<(usize, usize)> {
    if i < MAP_PARAM_SPAN {
        return Some((i + 1, 0));
    }
    let name = PARAMS.get(i)?.name;
    LATE_PARAMS.iter().position(|l| *l == name).map(|late| (0, late))
}

/// Parameters a macro can map: continuous ones outside the macro / mapping /
/// preset block.
pub fn macro_mappable(i: usize) -> bool {
    let macro1 = PARAMS.iter().position(|p| p.id == "macro1").unwrap_or(0);
    let preset = PARAMS.iter().position(|p| p.id == "preset").unwrap_or(0);
    PARAMS.get(i).is_some_and(|p| !p.is_discrete() && (i < macro1 || i > preset) && map_choices_for(i).is_some())
}

pub static SYNTH_V2: DeviceDef = DeviceDef {
    id: "synth-v2",
    name: "Synth",
    kind: PluginKind::Instrument,
    category: "Instruments",
    purpose: "Polyphonic subtractive synth with two oscillators, sub and noise, a multimode filter, two envelopes, two LFOs, a modulation matrix, macros, unison and four effect slots.",
    params: &PARAMS,
    layout: Layout { picker: &[], view: Some(View::Custom("synth-v2")), sliders: &[], clusters: &[], faces: &[], min_height: 265.0, shown_when: &[], fixed_width: 0.0, ..Layout::EMPTY },
    factory: &FACTORY,
    sidechain: false,
    hooks: crate::builtin::Hooks::NONE,
    samples: 0,
};

const OFF: f32 = 0.0;
const ON: f32 = 1.0;
/// Factory-preset values that differ from the defaults. FX 2 (chorus) and FX 4
/// (reverb) are on by default; presets switch them off where they don't fit.
static FACTORY: [FactoryPreset; 12] = [
    FactoryPreset {
        name: "Porch Light Keys",
        values: &[("osc1_wave", 1.0), ("osc1_shape", 0.15), ("osc2_wave", 0.0), ("osc2_octave", 3.0), ("osc2_level", 0.3), ("sub_level", 0.0), ("cutoff", 3200.0), ("resonance", 0.1), ("filter_env", 0.25), ("amp_attack", 0.002), ("amp_decay", 1.6), ("amp_sustain", 0.35), ("amp_release", 0.45), ("amp_velocity", 0.7), ("fx1_on", ON), ("fx1_tape_drive", 0.2)],
    },
    FactoryPreset {
        name: "Velour Pad",
        values: &[("osc1_wave", 2.0), ("osc1_shape", 0.6), ("osc2_wave", 3.0), ("osc2_shape", 0.4), ("osc2_fine", -9.0), ("unison_voices", 3.0), ("unison_detune", 0.3), ("cutoff", 1400.0), ("resonance", 0.15), ("filter_env", 0.15), ("env2_attack", 0.8), ("env2_decay", 2.0), ("amp_attack", 0.6), ("amp_sustain", 0.85), ("amp_release", 2.2), ("lfo1_rate", 0.25), ("mod1_target", 11.0), ("mod1_amount", 0.06), ("fx2_mix", 0.45), ("fx4_mix", 0.3), ("fx4_reverb_decay", 4.5)],
    },
    FactoryPreset {
        name: "Basement Sub",
        values: &[("osc1_wave", 0.0), ("osc1_shape", 0.0), ("osc1_octave", 1.0), ("osc2_wave", 1.0), ("osc2_octave", 1.0), ("osc2_level", 0.2), ("sub_level", 0.6), ("cutoff", 420.0), ("resonance", 0.05), ("filter_env", 0.1), ("amp_attack", 0.001), ("amp_sustain", 1.0), ("amp_release", 0.12), ("voice_mode", 2.0), ("glide", 0.04), ("drift", 0.1), ("fx2_on", OFF), ("fx4_on", OFF), ("mod1_source", 0.0), ("mod1_target", 0.0), ("mod1_amount", 0.0)],
    },
    FactoryPreset {
        name: "Rubber Acid",
        values: &[("osc1_wave", 2.0), ("osc1_shape", 0.0), ("osc2_level", 0.0), ("sub_level", 0.0), ("filter_type", 0.0), ("cutoff", 380.0), ("resonance", 0.78), ("filter_drive", 0.55), ("key_track", 0.3), ("filter_env", 0.62), ("filter_velocity", 0.6), ("env2_decay", 0.22), ("env2_sustain", 0.0), ("amp_sustain", 0.9), ("amp_release", 0.08), ("voice_mode", 2.0), ("glide", 0.06), ("fx1_on", ON), ("fx1_type", 1.0), ("fx1_distortion_drive", 0.35), ("fx2_on", OFF), ("fx3_on", ON), ("fx3_mix", 0.18), ("fx4_on", OFF), ("mod1_source", 0.0), ("mod1_target", 0.0), ("mod1_amount", 0.0)],
    },
    FactoryPreset {
        name: "Seven Saws",
        values: &[("osc1_wave", 2.0), ("osc1_shape", 0.8), ("osc2_wave", 2.0), ("osc2_octave", 3.0), ("osc2_level", 0.35), ("unison_voices", 6.0), ("unison_detune", 0.45), ("unison_spread", 0.9), ("cutoff", 6500.0), ("resonance", 0.1), ("filter_env", 0.1), ("amp_attack", 0.004), ("amp_sustain", 0.9), ("amp_release", 0.5), ("fx2_on", OFF), ("fx3_on", ON), ("fx3_mix", 0.22), ("fx4_mix", 0.22)],
    },
    FactoryPreset {
        name: "Pebble Pluck",
        values: &[("osc1_wave", 3.0), ("osc1_shape", 0.35), ("osc2_wave", 1.0), ("osc2_octave", 3.0), ("osc2_level", 0.4), ("sub_level", 0.0), ("cutoff", 900.0), ("resonance", 0.35), ("filter_env", 0.55), ("env2_decay", 0.28), ("env2_sustain", 0.0), ("amp_decay", 0.45), ("amp_sustain", 0.0), ("amp_release", 0.3), ("fx3_on", ON), ("fx3_mix", 0.25), ("fx4_mix", 0.18)],
    },
    FactoryPreset {
        name: "Copper Horns",
        values: &[("osc1_wave", 2.0), ("osc1_shape", 0.2), ("osc2_wave", 2.0), ("osc2_fine", -6.0), ("cutoff", 700.0), ("resonance", 0.18), ("filter_env", 0.5), ("filter_velocity", 0.5), ("env2_attack", 0.06), ("env2_decay", 0.6), ("env2_sustain", 0.45), ("amp_attack", 0.03), ("amp_sustain", 0.85), ("amp_release", 0.25), ("drift", 0.5), ("fx2_mix", 0.25)],
    },
    FactoryPreset {
        name: "Static Bloom",
        values: &[("osc1_level", 0.0), ("osc2_level", 0.0), ("sub_level", 0.0), ("noise_level", 0.9), ("noise_type", 1.0), ("noise_width", 0.8), ("filter_type", 2.0), ("cutoff", 600.0), ("resonance", 0.6), ("filter_env", 0.8), ("env2_attack", 2.5), ("env2_decay", 3.0), ("env2_sustain", 0.6), ("amp_attack", 1.2), ("amp_sustain", 1.0), ("amp_release", 2.5), ("fx4_mix", 0.4), ("fx4_reverb_decay", 6.0)],
    },
    FactoryPreset {
        name: "Glide Ribbon",
        values: &[("osc1_wave", 3.0), ("osc1_shape", 0.5), ("osc2_wave", 2.0), ("osc2_level", 0.5), ("cutoff", 2400.0), ("resonance", 0.3), ("filter_env", 0.2), ("amp_sustain", 1.0), ("voice_mode", 2.0), ("glide", 0.12), ("lfo2_rate", 5.5), ("lfo2_depth", 0.0), ("lfo2_retrigger", ON), ("lfo2_fade", 0.6), ("mod2_amount", 0.008), ("fx3_on", ON), ("fx3_mix", 0.25)],
    },
    FactoryPreset {
        name: "Tin Chime",
        values: &[("osc1_wave", 5.0), ("osc1_shape", 0.55), ("osc2_wave", 0.0), ("osc2_octave", 4.0), ("osc2_semi", 19.0), ("osc2_level", 0.25), ("cross_fm", 0.25), ("sub_level", 0.0), ("cutoff", 9000.0), ("filter_env", 0.0), ("amp_decay", 2.4), ("amp_sustain", 0.0), ("amp_release", 1.6), ("amp_velocity", 0.8), ("fx2_on", OFF), ("fx4_mix", 0.3)],
    },
    FactoryPreset {
        name: "Wobblefoot",
        values: &[("osc1_wave", 2.0), ("osc1_octave", 1.0), ("osc2_wave", 3.0), ("osc2_octave", 1.0), ("osc2_fine", 4.0), ("sub_level", 0.5), ("cutoff", 260.0), ("resonance", 0.5), ("filter_drive", 0.6), ("filter_env", 0.0), ("amp_sustain", 1.0), ("amp_release", 0.1), ("voice_mode", 1.0), ("lfo1_shape", 0.0), ("lfo1_sync", ON), ("lfo1_note", 11.0), ("lfo1_retrigger", ON), ("mod1_target", 11.0), ("mod1_amount", 0.42), ("fx1_on", ON), ("fx1_type", 1.0), ("fx1_distortion_drive", 0.45), ("fx2_on", OFF), ("fx4_on", OFF)],
    },
    FactoryPreset {
        name: "Hollow Choir",
        values: &[("osc1_wave", 1.0), ("osc1_shape", 0.4), ("osc2_wave", 3.0), ("osc2_shape", 0.7), ("osc2_octave", 3.0), ("osc2_level", 0.3), ("noise_level", 0.12), ("noise_type", 3.0), ("filter_type", 2.0), ("cutoff", 1100.0), ("resonance", 0.45), ("filter_env", 0.0), ("amp_attack", 0.35), ("amp_sustain", 0.9), ("amp_release", 1.4), ("unison_voices", 2.0), ("unison_detune", 0.25), ("fx2_mix", 0.5), ("fx4_mix", 0.35)],
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_shape() {
        assert_eq!(PARAMS.len(), 605);
        assert_eq!(PRESETS.len(), 128);
        assert_eq!(PARAMS[581].id, "preset");
        assert_eq!(PARAMS[499].id, "volume");
        assert_eq!(MAP_PARAMS.len(), 502);
        for (i, name) in MAP_PARAMS.iter().enumerate().skip(1) {
            assert_eq!(PARAMS[i - 1].name, *name, "map param choice {i}");
        }
        assert_eq!(LATE_PARAMS[3], "Sub Shape");
        for f in FACTORY.iter() {
            assert!(PRESETS.contains(&f.name), "{} is a preset choice", f.name);
        }
    }

    #[test]
    fn defaults_follow_the_spec() {
        let v = |id: &str| PARAMS.iter().find(|p| p.id == id).unwrap();
        assert_eq!(v("osc1_wave").format(v("osc1_wave").default), "Saw");
        assert_eq!(v("sub_tune").format(v("sub_tune").default), "\u{2212}12 st");
        assert_eq!(v("osc2_fine").format(7.0), "+7 ct");
        assert_eq!(v("cutoff").format(1800.0), "1.80 kHz");
        assert_eq!(v("lfo1_rate").format(0.18), "0.18 Hz");
        assert_eq!(v("high_pass").format(10.0), "10.0 Hz");
        assert_eq!(v("amp_attack").format(0.004), "4.0 ms");
        assert_eq!(v("amp_decay").format(0.9), "900 ms");
        assert_eq!(v("amp_decay").format(1.2), "1.20 s");
        assert_eq!(v("volume").format(-0.7), "\u{2212}0.7 dB");
        assert_eq!(v("volume").format(0.0), "0.0 dB");
        assert_eq!(v("filter_env").format(0.25), "+25%");
        assert_eq!(v("resonance").format(0.1), "10%");
        assert_eq!(v("fx2_type").format(v("fx2_type").default), "Chorus");
        assert!(!v("map1_param").automatable && !v("preset").automatable && v("mod1_amount").automatable);
    }

    #[test]
    fn parsing() {
        let p = |id: &str| PARAMS.iter().find(|p| p.id == id).unwrap();
        assert_eq!(p("resonance").parse("30%"), Some(0.3));
        assert_eq!(p("resonance").parse("30"), Some(0.3));
        assert_eq!(p("amp_attack").parse("250 ms"), Some(0.25));
        assert_eq!(p("cutoff").parse("2.4k"), Some(2400.0));
        assert_eq!(p("sub_tune").parse("-12 st"), Some(12.0));
        assert_eq!(p("osc1_fine").parse("+7 ct"), Some(7.0));
    }

    #[test]
    fn modulation_tables() {
        assert_eq!(MOD_TARGETS.len(), TARGET_DEPTH.len());
        assert_eq!(target_param(11), Some("cutoff"));
        assert_eq!(TARGET_DEPTH[11], 8.0);
        assert_eq!(target_of("cutoff", [0; 4]), Some(11));
        assert_eq!(target_of("fx2_mix", [0; 4]), Some(23));
        assert_eq!(target_of("fx1_tape_drive", [0; 4]), Some(26));
        assert_eq!(target_of("fx1_tape_drive", [1, 0, 0, 0]), None);
        for t in 1..30 {
            if let Some(p) = target_param(t) {
                assert!(PARAMS.iter().any(|q| q.id == p), "{p}");
            }
        }
        for ty in 0..FX_TYPES.len() {
            assert!(PARAMS.iter().any(|q| q.id == fx_param(1, fx_amount_param(ty))));
        }
    }

    #[test]
    fn macro_mapping_targets() {
        let idx = |id: &str| PARAMS.iter().position(|p| p.id == id).unwrap();
        assert_eq!(map_target(idx("cutoff") + 1, 0), Some(idx("cutoff")));
        assert_eq!(map_target(0, 3), Some(idx("sub_shape")));
        assert_eq!(map_choices_for(idx("noise_width")), Some((0, 4)));
        assert!(macro_mappable(idx("cutoff")) && macro_mappable(idx("sub_shape")) && macro_mappable(idx("volume")));
        assert!(!macro_mappable(idx("osc1_wave")) && !macro_mappable(idx("macro1")) && !macro_mappable(idx("map1_from")));
    }
}
