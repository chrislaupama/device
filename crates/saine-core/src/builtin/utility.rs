//! Utility (reference: the reference app's built-in "Utility"). Parameter ids, ranges,
//! defaults, tapers and the panel arrangement are taken from the reference as
//! facts; the factory presets are our own.
//!
//! The card is a two-column grid (Input | Output) drawn by the "utility"
//! custom view; Bass Mono, its frequency and its audition switch have no
//! control on the card (as in the reference) and are reached through presets,
//! automation and the parameter list.

use super::*;
use crate::device::PluginKind;

const CHANNELS: &[&str] = &["Stereo", "Left", "Right", "Swap"];
const OFF_ON: &[&str] = &["Off", "On"];

pub static UTILITY: DeviceDef = DeviceDef {
    id: "utility",
    name: "Utility",
    kind: PluginKind::Effect,
    category: "Utility",
    purpose: "Fixes a channel: level, balance, stereo width, polarity, channel routing, mono and bass mono, mute and DC removal.",
    params: &[
        ParamDef::new("gain", "Gain", -70.0, 35.0, 0.0, LINEAR, "dB", Format::Db).help("Output level."),
        ParamDef::new("balance", "Balance", -50.0, 50.0, 0.0, LINEAR, "", Format::Balance).help("Left / right balance."),
        ParamDef::new("width", "Width", 0.0, 400.0, 100.0, LINEAR, "%", Format::Percent0).help("Stereo width: 0% mono, 100% unchanged, up to 400% wider."),
        ParamDef::choice("channel_mode", "Channel Mode", CHANNELS, 0).help("Stereo passes both channels, Left / Right put one channel on both sides, Swap exchanges them."),
        ParamDef::choice("phase_l", "Phase L", OFF_ON, 0).label("\u{d8}L").help("Inverts the polarity of the left channel."),
        ParamDef::choice("phase_r", "Phase R", OFF_ON, 0).label("\u{d8}R").help("Inverts the polarity of the right channel."),
        ParamDef::choice("mono", "Mono", OFF_ON, 0).help("Sums both channels to mono."),
        ParamDef::choice("bass_mono", "Bass Mono", OFF_ON, 0).help("Makes everything below the Bass Mono frequency mono."),
        ParamDef::new("bass_mono_freq", "Bass Mono Freq", 50.0, 500.0, 120.0, Taper::Log, "Hz", Format::Freq).help("Crossover below which Bass Mono sums to mono."),
        ParamDef::choice("bass_mono_audition", "Bass Mono Audition", OFF_ON, 0).reset_on_load().help("Plays only the part below the Bass Mono frequency."),
        ParamDef::choice("mute", "Mute", OFF_ON, 0).help("Silences the output."),
        ParamDef::choice("dc_filter", "DC Filter", OFF_ON, 0).label("DC").help("Removes DC offset (a very low high-pass)."),
    ],
    layout: Layout {
        picker: &[],
        view: Some(View::Custom("utility")),
        sliders: &[],
        clusters: &[],
        faces: &[],
        min_height: 176.0,
        shown_when: &[],
        fixed_width: 221.5,
        ..Layout::EMPTY
    },
    factory: &[
        FactoryPreset { name: "Headroom -12 dB", values: &[("gain", -12.0)] },
        FactoryPreset { name: "Sub Focus 90 Hz", values: &[("bass_mono", 1.0), ("bass_mono_freq", 90.0)] },
        FactoryPreset { name: "Half Width", values: &[("width", 50.0)] },
        FactoryPreset { name: "Wider 150%", values: &[("width", 150.0), ("bass_mono", 1.0), ("bass_mono_freq", 150.0)] },
        FactoryPreset { name: "Clean Up DC", values: &[("dc_filter", 1.0)] },
        FactoryPreset { name: "Polarity Flip Both", values: &[("phase_l", 1.0), ("phase_r", 1.0)] },
        FactoryPreset { name: "Left Mic Centred", values: &[("channel_mode", 1.0), ("gain", -3.0)] },
    ],
    sidechain: false,
    samples: 0,
};
