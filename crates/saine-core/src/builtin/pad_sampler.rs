//! Pad Sampler (reference: the reference app's built-in pad sampler, the "Sampler" in
//! each Drum Rack pad). One pad: up to 8 sounds that alternate hit by hit,
//! with one set of settings. Ids, ranges and defaults follow the device spec
//! (handoffs/device-specs/pad-sampler.md); presets are our own. The Drum
//! Rack repeats these settings for each of its 16 pads (drum_rack.rs).

use super::sampler::{OFF_ON, WARP};
use super::*;
use crate::device::PluginKind;

/// Sounds (layers) one pad holds.
pub const LAYERS: u32 = 8;

pub(super) const fn gain(id: &'static str, name: &'static str) -> ParamDef {
    ParamDef::new(id, name, -60.0, 12.0, 0.0, LINEAR, "dB", Format::Db).step(0.1).label("Gain").help("Level of the pad.")
}
pub(super) const fn pan(id: &'static str, name: &'static str) -> ParamDef {
    ParamDef::new(id, name, -1.0, 1.0, 0.0, LINEAR, "", Format::Pan).label("Pan").help("Stereo position of the pad.")
}
pub(super) const fn transpose(id: &'static str, name: &'static str) -> ParamDef {
    ParamDef::new(id, name, -24.0, 24.0, 0.0, LINEAR, "st", Format::Signed { decimals: 0, scale: 1 }).step(1.0).label("Transpose").help("Pitch offset in semitones (re-pitch: shorter when higher).")
}
pub(super) const fn fade_in(id: &'static str, name: &'static str) -> ParamDef {
    ParamDef::new(id, name, 0.0, 100.0, 0.0, LINEAR, "%", Format::Percent).label("Fade In").help("Fade at the start of the sound, as a share of its length.")
}
pub(super) const fn fade_out(id: &'static str, name: &'static str) -> ParamDef {
    ParamDef::new(id, name, 0.0, 100.0, 0.0, LINEAR, "%", Format::Percent).label("Fade Out").help("Fade at the end of the sound, as a share of its length.")
}
pub(super) const fn start(id: &'static str, name: &'static str) -> ParamDef {
    ParamDef::new(id, name, 0.0, 1.0, 0.0, LINEAR, "", Format::Fraction).label("Start").help("Where the sound starts.")
}
pub(super) const fn end(id: &'static str, name: &'static str) -> ParamDef {
    ParamDef::new(id, name, 0.0, 1.0, 1.0, LINEAR, "", Format::Fraction).label("End").help("Where the sound ends.")
}
pub(super) const fn reverse(id: &'static str, name: &'static str) -> ParamDef {
    ParamDef::choice(id, name, OFF_ON, 0).label("Reverse").help("When on, the sound plays backwards.")
}
pub(super) const fn gate(id: &'static str, name: &'static str) -> ParamDef {
    ParamDef::choice(id, name, OFF_ON, 0).label("Gate").help("When on, the sound plays only while the note is held. When off, a hit plays the whole sound.")
}
pub(super) const fn warp(id: &'static str, name: &'static str) -> ParamDef {
    ParamDef::choice(id, name, WARP, 0).label("Warp").help("Whether a note keeps the sound's length: Re-pitch, or stretch the sound to a note length at the project tempo.")
}

pub static PAD_SAMPLER: DeviceDef = DeviceDef {
    id: "pad-sampler",
    name: "Pad Sampler",
    kind: PluginKind::Instrument,
    category: "Instruments",
    purpose: "A drum pad: up to eight sounds that alternate hit by hit, with gain, pan, transpose, fades, start and end, reverse, gate and warp.",
    params: &[
        gain("gain", "Gain"),
        pan("pan", "Pan"),
        transpose("transpose", "Transpose"),
        fade_in("fadeIn", "Fade In"),
        fade_out("fadeOut", "Fade Out"),
        start("start", "Start"),
        end("end", "End"),
        reverse("reverse", "Reverse"),
        gate("gate", "Gate"),
        warp("warp", "Warp"),
    ],
    layout: Layout { picker: &[], view: Some(View::Custom("pad-sampler")), sliders: &[], clusters: &[], faces: &[], min_height: 265.0, shown_when: &[], fixed_width: 599.0, ..Layout::EMPTY },
    factory: &[
        FactoryPreset { name: "Short Hit", values: &[("fadeOut", 40.0), ("end", 0.5), ("gate", 0.0)] },
        FactoryPreset { name: "Held Note", values: &[("gate", 1.0), ("fadeOut", 5.0)] },
        FactoryPreset { name: "Low Drop", values: &[("transpose", -7.0), ("gain", 2.0)] },
        FactoryPreset { name: "Reverse Swell", values: &[("reverse", 1.0), ("fadeIn", 40.0)] },
        FactoryPreset { name: "Quarter Stretch", values: &[("warp", 3.0)] },
    ],
    sidechain: false,
    samples: LAYERS,
};
