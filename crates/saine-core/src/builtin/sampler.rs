//! Sampler (reference: the reference app's built-in "Sampler", thumbnail sampler-steam).
//! Parameter ids, ranges, defaults and tapers follow the device spec
//! (handoffs/device-specs/sampler-steam.md) as facts; descriptions, the
//! factory presets and their values are our own. The panel is a custom view
//! (crates/saine-app/src/sampler_panel.rs); the engine kernel is
//! engine/src/devices/sampler.cpp on the shared sampler engine.

use super::*;
use crate::device::PluginKind;

pub(super) const OFF_ON: &[&str] = &["Off", "On"];
/// Warp choices shared with the Pad Sampler: re-pitch (a note changes the
/// sound's length) or stretch the sound to a note length at the project tempo.
pub(super) const WARP: &[&str] = &["Re-pitch", "Whole note", "Half note", "Quarter note", "Eighth note"];
const MODES: &[&str] = &["Classic", "One Shot", "Slice"];
const PLAY_MODES: &[&str] = &["Trigger", "Gate", "Toggle"];
const VOICING: &[&str] = &["Mono", "Poly"];
const FILTER_TYPES: &[&str] = &["LP 12", "LP 24", "HP 12", "BP 12"];
const LFO_SHAPES: &[&str] = &["Sine", "Triangle", "Square", "Saw"];
const LFO_TARGETS: &[&str] = &["Pitch", "Cutoff", "Pan", "Volume"];
const CHOP: &[&str] = &["Beat", "Transient"];
pub const DIVISIONS: &[&str] = &["1/1", "1/2", "1/4", "1/8", "1/16", "1/32"];

const fn ms(id: &'static str, name: &'static str, max: f32, default: f32) -> ParamDef {
    ParamDef::new(id, name, 0.0, max, default, Taper::Linear { skew: 0.3 }, "ms", Format::Time)
}
const fn frac(id: &'static str, name: &'static str, default: f32) -> ParamDef {
    ParamDef::new(id, name, 0.0, 1.0, default, LINEAR, "", Format::Fraction)
}

pub static SAMPLER: DeviceDef = DeviceDef {
    id: "sampler-steam",
    name: "Sampler",
    kind: PluginKind::Instrument,
    category: "Instruments",
    purpose: "Plays one sample over the keyboard: whole (One Shot), sustained with a loop (Classic) or chopped into slices, with an amp envelope, a filter with its own envelope, an LFO and glide.",
    params: &[
        ParamDef::choice("mode", "Mode", MODES, 1).not_automatable().help("Classic plays the sample chromatically with a sustain loop, One Shot plays the whole region per note, Slice plays one slice per key."),
        ParamDef::choice("playMode", "Play", PLAY_MODES, 0).not_automatable().help("One Shot and Slice: Trigger plays the whole slice on press, Gate only while held, Toggle starts on one press and stops on the next."),
        ParamDef::choice("voicing", "Voicing", VOICING, 0).not_automatable().help("Mono plays one note at a time (with legato and glide), Poly up to the voice limit."),
        ParamDef::new("voices", "Voices", 1.0, 32.0, 8.0, LINEAR, "", Format::Plain(0)).step(1.0).not_automatable().help("Poly voice limit; the oldest note is cut when a new one needs a voice."),
        ParamDef::choice("warp", "Warp", WARP, 0).not_automatable().help("Whether a note keeps the sound's length: Re-pitch changes length with pitch, a note value stretches the sound to that length at the project tempo."),
        ParamDef::new("root", "Root", 0.0, 127.0, 60.0, LINEAR, "", Format::Note).step(1.0).not_automatable().help("The key that plays the sample at its original pitch."),
        ParamDef::new("volume", "Gain", -60.0, 12.0, 0.0, LINEAR, "dB", Format::Db).step(0.1).help("Output level of the sampler."),
        ParamDef::new("pan", "Pan", -1.0, 1.0, 0.0, LINEAR, "", Format::Pan).help("Stereo position."),
        ParamDef::new("transpose", "Transpose", -24.0, 24.0, 0.0, LINEAR, "st", Format::Signed { decimals: 0, scale: 1 }).step(1.0).help("Pitch offset in semitones."),
        ParamDef::new("fineTune", "Fine", -100.0, 100.0, 0.0, LINEAR, "ct", Format::Signed { decimals: 0, scale: 1 }).step(1.0).help("Pitch offset in cents."),
        ms("glide", "Glide", 2000.0, 0.0).help("Time a new note takes to slide to its pitch from the last one."),
        ParamDef::new("bendRange", "Bend", 0.0, 24.0, 2.0, LINEAR, "st", Format::Plain(0)).step(1.0).help("Pitch-bend range in semitones."),
        ParamDef::new("velocity", "Velocity", 0.0, 1.0, 1.0, LINEAR, "", Format::Fraction).step(0.01).help("How much note velocity changes the level."),
        ParamDef::choice("legato", "Legato", OFF_ON, 0).not_automatable().help("Mono: a note played while another is held changes pitch without restarting the sample."),
        ms("ampAttack", "Attack", 2000.0, 2.0).help("Amp envelope attack."),
        ms("ampDecay", "Decay", 2000.0, 120.0).help("Amp envelope decay to the sustain level."),
        frac("ampSustain", "Sustain", 1.0).help("Amp envelope level while a key is held."),
        ms("ampRelease", "Release", 4000.0, 48.0).help("Amp envelope release after the key is let go."),
        ParamDef::choice("filterOn", "Filter", OFF_ON, 0).not_automatable().help("Turns the filter on."),
        ParamDef::choice("filterType", "Type", FILTER_TYPES, 1).not_automatable().help("Low-pass 12 or 24 dB/oct, high-pass or band-pass."),
        ParamDef::new("cutoff", "Cutoff", 20.0, 20000.0, 20000.0, Taper::Log, "Hz", Format::Hz).help("Filter cutoff frequency."),
        frac("resonance", "Resonance", 0.0).help("Emphasis around the cutoff."),
        frac("drive", "Drive", 0.0).help("Saturation before the filter."),
        frac("keyTrack", "Key Track", 0.0).help("How much the cutoff follows the played key (full = one octave per octave)."),
        ParamDef::new("envDepth", "Env", -1.0, 1.0, 0.0, LINEAR, "st", Format::Signed { decimals: 0, scale: 48 }).help("How far the filter envelope moves the cutoff, up to 48 semitones either way."),
        ms("filterAttack", "Attack", 2000.0, 0.0).label("Attack").help("Filter envelope attack."),
        ms("filterDecay", "Decay", 2000.0, 300.0).help("Filter envelope decay."),
        frac("filterSustain", "Sustain", 0.0).help("Filter envelope sustain level."),
        ms("filterRelease", "Release", 4000.0, 200.0).help("Filter envelope release."),
        ParamDef::choice("lfoShape", "Shape", LFO_SHAPES, 0).not_automatable().help("LFO waveform."),
        ParamDef::new("lfoRate", "Rate", 0.05, 20.0, 2.0, Taper::Log, "Hz", Format::Rate).help("LFO speed."),
        frac("lfoAmount", "Amount", 0.0).help("LFO depth on its target."),
        ms("lfoFadeIn", "Fade In", 4000.0, 0.0).help("Time the LFO takes to reach full depth after a note starts."),
        frac("lfoPhase", "Phase", 0.0).help("Where in its cycle the LFO starts."),
        ParamDef::choice("lfoTarget", "Target", LFO_TARGETS, 0).not_automatable().help("What the LFO moves: pitch, filter cutoff, pan or volume."),
        ParamDef::choice("lfoKeyTrigger", "Key Trigger", OFF_ON, 1).not_automatable().help("On: every note restarts the LFO; off: it runs freely."),
        frac("start", "Start", 0.0).help("Where playback starts in the sample."),
        frac("end", "End", 1.0).help("Where playback ends in the sample."),
        ParamDef::new("fadeIn", "Fade In", 0.0, 100.0, 0.0, LINEAR, "%", Format::Percent).help("Fade at the start of the region, as a share of its length."),
        ParamDef::new("fadeOut", "Fade Out", 0.0, 100.0, 0.0, LINEAR, "%", Format::Percent).help("Fade at the end of the region, as a share of its length."),
        ParamDef::choice("reverse", "Reverse", OFF_ON, 0).help("When on, the sound plays backwards."),
        ParamDef::choice("loop", "Loop", OFF_ON, 0).not_automatable().help("Classic: loop from the loop start to the end while the key is held."),
        frac("loopStart", "Loop Start", 0.0).help("Classic: where the sustain loop starts."),
        ParamDef::new("sliceKey", "Slice Key", 0.0, 127.0, 60.0, LINEAR, "", Format::Note).step(1.0).not_automatable().help("Slice: the key that plays the first slice."),
        ParamDef::choice("chop", "Chop", CHOP, 0).not_automatable().help("Slice: cut at a fixed note division or at detected transients."),
        ParamDef::new("sensitivity", "Sensitivity", 0.0, 100.0, 50.0, LINEAR, "%", Format::Percent).not_automatable().help("Slice by transient: how readily a hit becomes a slice."),
        ParamDef::choice("division", "Division", DIVISIONS, 2).not_automatable().help("Slice by beat: the note length of each slice."),
    ],
    layout: Layout {
        picker: &[],
        view: Some(View::Custom("sampler")),
        sliders: &[],
        clusters: &[],
        faces: &[],
        min_height: 265.0,
        shown_when: &[],
        fixed_width: 798.0,
    },
    factory: &[
        FactoryPreset { name: "Felt Piano Keys", values: &[("mode", 0.0), ("voicing", 1.0), ("ampAttack", 4.0), ("ampDecay", 900.0), ("ampSustain", 0.35), ("ampRelease", 420.0), ("filterOn", 1.0), ("cutoff", 3200.0), ("keyTrack", 0.6), ("envDepth", 0.15), ("filterDecay", 600.0)] },
        FactoryPreset { name: "Glass Pluck", values: &[("mode", 0.0), ("voicing", 1.0), ("ampAttack", 1.0), ("ampDecay", 380.0), ("ampSustain", 0.0), ("ampRelease", 260.0), ("filterOn", 1.0), ("filterType", 0.0), ("cutoff", 900.0), ("resonance", 0.35), ("envDepth", 0.55), ("filterDecay", 220.0)] },
        FactoryPreset { name: "Slow Bloom", values: &[("mode", 0.0), ("voicing", 1.0), ("loop", 1.0), ("loopStart", 0.35), ("ampAttack", 1400.0), ("ampDecay", 1200.0), ("ampSustain", 0.8), ("ampRelease", 2600.0), ("lfoAmount", 0.25), ("lfoTarget", 1.0), ("lfoRate", 0.3), ("filterOn", 1.0), ("cutoff", 2400.0)] },
        FactoryPreset { name: "Gliding Mono Lead", values: &[("mode", 0.0), ("voicing", 0.0), ("legato", 1.0), ("glide", 120.0), ("loop", 1.0), ("loopStart", 0.5), ("lfoAmount", 0.12), ("lfoFadeIn", 600.0), ("lfoRate", 5.5), ("ampRelease", 180.0)] },
        FactoryPreset { name: "Break Chopper", values: &[("mode", 2.0), ("playMode", 0.0), ("voicing", 1.0), ("voices", 16.0), ("fadeOut", 3.0), ("ampRelease", 30.0)] },
        FactoryPreset { name: "Gated Slices", values: &[("mode", 2.0), ("playMode", 1.0), ("voicing", 1.0), ("ampRelease", 15.0), ("fadeIn", 1.0), ("fadeOut", 2.0)] },
        FactoryPreset { name: "Dusty Low-Pass", values: &[("mode", 1.0), ("filterOn", 1.0), ("filterType", 1.0), ("cutoff", 1400.0), ("resonance", 0.2), ("drive", 0.45), ("volume", -2.0)] },
        FactoryPreset { name: "Backwards Swell", values: &[("mode", 1.0), ("reverse", 1.0), ("fadeIn", 30.0), ("ampAttack", 300.0), ("ampRelease", 800.0)] },
    ],
    sidechain: false,
    samples: 1,
};
