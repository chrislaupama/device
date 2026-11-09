//! LFO: a per-track modulation source (handoffs/device-specs/lfo.md).
//!
//! It passes audio through and moves the parameters in its `Device::links`
//! (crate::modulation). Parameters, ranges, defaults and the tempo-division
//! list follow the spec; the panel is the custom "lfo" view (wave picture,
//! Depth fader, Shape menu, Free/Sync latch, Rate and Phase dials, targets).

use super::{DeviceDef, Format, Layout, ParamDef, Taper, View, DIVISIONS, DIVISION_QUARTER, LINEAR};
use crate::device::PluginKind;

/// Wave shapes, in the reference order.
pub const SHAPES: &[&str] = &["Sine", "Triangle", "Saw Up", "Saw Down", "Square", "Random", "Smooth Random"];

const PARAMS: &[ParamDef] = &[
    ParamDef::choice("shape", "Shape", SHAPES, 0).help("Wave shape of the modulation."),
    ParamDef::choice("sync", "Sync", &["Free", "Sync"], 1).help("Free runs at Rate in hertz; Sync follows the project tempo."),
    ParamDef::new("rate", "Rate", 0.01, 40.0, 1.0, Taper::Log, "Hz", Format::Plain(2)).help("Speed in hertz while Sync is off."),
    ParamDef::choice("rateSync", "Rate Sync", DIVISIONS, DIVISION_QUARTER).label("Rate").help("Cycle length as a note division while Sync is on."),
    ParamDef::new("depth", "Depth", 0.0, 100.0, 100.0, LINEAR, "%", Format::Percent).help("Scales how far every target moves."),
    ParamDef::new("phase", "Phase", 0.0, 100.0, 0.0, LINEAR, "%", Format::Percent).help("Start position inside the cycle."),
];

pub static LFO: DeviceDef = DeviceDef {
    id: "lfo",
    name: "LFO",
    kind: PluginKind::Effect,
    category: "Modulation",
    purpose: "Moves other parameters on the track up and down in a repeating wave.",
    params: PARAMS,
    layout: Layout { picker: &[], view: Some(View::Custom("lfo")), sliders: &[], clusters: &[], faces: &[], min_height: 0.0, shown_when: &[], fixed_width: 0.0, ..Layout::EMPTY },
    factory: &[],
    sidechain: false,
    samples: 0,
};
