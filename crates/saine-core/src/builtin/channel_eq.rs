//! Channel EQ (reference: the reference app's built-in "Channel EQ"): eight bands
//! (high pass, low shelf, four bells, high shelf, low pass by default; every
//! band can take any type), shared high- and low-pass slopes and an output
//! gain. Parameter ids, ranges, defaults, tapers and the panel arrangement
//! are taken from the reference as facts; factory presets are our own.

use super::eq::{EqBand, EqView, NodeDoubleClick, Shape};
use super::*;
use crate::device::PluginKind;

const OFF_ON: &[&str] = &["Off", "On"];
const TYPES: &[&str] = &["Bell", "Low Shelf", "High Shelf", "Notch", "High Pass", "Low Pass"];
const SLOPES: &[&str] = &["12 dB/oct", "24 dB/oct", "36 dB/oct", "48 dB/oct"];
const LISTEN: &[&str] = &["Off", "Band 1", "Band 2", "Band 3", "Band 4", "Band 5", "Band 6", "Band 7", "Band 8"];

const fn freq(id: &'static str, name: &'static str, default: f32) -> ParamDef {
    ParamDef::new(id, name, 20.0, 20_000.0, default, Taper::Log, "Hz", Format::Freq).label("Freq").help("Centre or corner frequency of the band.")
}
const fn gain(id: &'static str, name: &'static str) -> ParamDef {
    ParamDef::new(id, name, -48.0, 30.0, 0.0, LINEAR, "dB", Format::Db).label("Gain").help("Boost or cut of the band (bells and shelves).")
}
const fn q(id: &'static str, name: &'static str, default: f32) -> ParamDef {
    ParamDef::new(id, name, 0.1, 20.0, default, Taper::Log, "", Format::Plain(2)).label("Q").help("Width of a bell or notch; resonance of a shelf or pass filter.")
}
const fn on(id: &'static str, name: &'static str, default: u32) -> ParamDef {
    ParamDef::choice(id, name, OFF_ON, default).label("On").help("Switches the band in or out.")
}
const fn kind(id: &'static str, name: &'static str, default: u32) -> ParamDef {
    ParamDef::choice(id, name, TYPES, default).label("Type").help("Filter shape of the band.")
}

const fn band(n: &'static [&'static str; 5]) -> EqBand {
    EqBand::new(n[0], n[1], n[2], n[3], n[4])
}

static VIEW: EqView = EqView {
    bands: &[
        band(&["band0_enabled", "band0_type", "band0_freq", "band0_gain2", "band0_q"]),
        band(&["band1_enabled", "band1_type", "band1_freq", "band1_gain2", "band1_q"]),
        band(&["band2_enabled", "band2_type", "band2_freq", "band2_gain2", "band2_q"]),
        band(&["band3_enabled", "band3_type", "band3_freq", "band3_gain2", "band3_q"]),
        band(&["band4_enabled", "band4_type", "band4_freq", "band4_gain2", "band4_q"]),
        band(&["band5_enabled", "band5_type", "band5_freq", "band5_gain2", "band5_q"]),
        band(&["band6_enabled", "band6_type", "band6_freq", "band6_gain2", "band6_q"]),
        band(&["band7_enabled", "band7_type", "band7_freq", "band7_gain2", "band7_q"]),
    ],
    shapes: &[Shape::Bell, Shape::LowShelf, Shape::HighShelf, Shape::Notch, Shape::HighPass, Shape::LowPass],
    picker_order: &[Shape::HighPass, Shape::LowShelf, Shape::DualLow, Shape::Bell, Shape::Notch, Shape::HighShelf, Shape::LowPass],
    hp_slope: Some("hp_slope"),
    lp_slope: Some("lp_slope"),
    output: "output_gain",
    bank: None,
    auto_gain: None,
    top_db: 30.0,
    bottom_db: -48.0,
    rung_db: 12.0,
    slider_w: 58.0,
    cells: [82.0, 84.0, 58.0],
    first_band: 2,
    double_click: NodeDoubleClick::Park,
    create_on_click: false,
    prominent_nodes: false,
};

pub static CHANNEL_EQ: DeviceDef = DeviceDef {
    samples: 0,
    id: "channeleq",
    name: "Channel EQ",
    kind: PluginKind::Effect,
    category: "EQ",
    purpose: "Shapes tone with eight bands: cut rumble and harshness, lift presence or air, and tidy the extremes with high- and low-pass filters.",
    params: &[
        freq("band0_freq", "HP Freq", 20.0),
        gain("band0_gain2", "HP Gain"),
        q("band0_q", "HP Q", 0.71),
        on("band0_enabled", "HP On", 0),
        freq("band1_freq", "Low Shelf Freq", 75.0),
        gain("band1_gain2", "Low Shelf Gain"),
        q("band1_q", "Low Shelf Q", 1.0),
        on("band1_enabled", "Low Shelf On", 1),
        freq("band2_freq", "Peak 2 Freq", 100.0),
        gain("band2_gain2", "Peak 2 Gain"),
        q("band2_q", "Peak 2 Q", 0.6),
        on("band2_enabled", "Peak 2 On", 1),
        freq("band3_freq", "Peak 3 Freq", 250.0),
        gain("band3_gain2", "Peak 3 Gain"),
        q("band3_q", "Peak 3 Q", 0.3),
        on("band3_enabled", "Peak 3 On", 1),
        freq("band4_freq", "Peak 4 Freq", 1040.0),
        gain("band4_gain2", "Peak 4 Gain"),
        q("band4_q", "Peak 4 Q", 0.41),
        on("band4_enabled", "Peak 4 On", 1),
        freq("band5_freq", "Peak 5 Freq", 2500.0),
        gain("band5_gain2", "Peak 5 Gain"),
        q("band5_q", "Peak 5 Q", 0.2),
        on("band5_enabled", "Peak 5 On", 1),
        freq("band6_freq", "High Shelf Freq", 7500.0),
        gain("band6_gain2", "High Shelf Gain"),
        q("band6_q", "High Shelf Q", 1.0),
        on("band6_enabled", "High Shelf On", 1),
        freq("band7_freq", "LP Freq", 20_000.0),
        gain("band7_gain2", "LP Gain"),
        q("band7_q", "LP Q", 0.71),
        on("band7_enabled", "LP On", 0),
        ParamDef::new("output_gain", "Output Gain", -15.0, 15.0, 0.0, LINEAR, "dB", Format::Db).label("Output").help("Level after the EQ."),
        kind("band0_type", "HP Type", 4),
        kind("band1_type", "Low Shelf Type", 1),
        kind("band2_type", "Peak 2 Type", 0),
        kind("band3_type", "Peak 3 Type", 0),
        kind("band4_type", "Peak 4 Type", 0),
        kind("band5_type", "Peak 5 Type", 0),
        kind("band6_type", "High Shelf Type", 2),
        kind("band7_type", "LP Type", 5),
        ParamDef::choice("hp_slope", "HP Slope", SLOPES, 0).label("Slope").help("Steepness of every high-pass band."),
        ParamDef::choice("lp_slope", "LP Slope", SLOPES, 0).label("Slope").help("Steepness of every low-pass band."),
        ParamDef::choice("listen", "Listen", LISTEN, 0).reset_on_load().help("Solos the region one band works on, to find what to cut or boost."),
    ],
    layout: Layout {
        picker: &[],
        view: Some(View::Eq(&VIEW)),
        sliders: &[],
        clusters: &[],
        faces: &[],
        min_height: 155.0,
        shown_when: &[],
        fixed_width: 528.0,
    },
    factory: &[
        FactoryPreset { name: "Rumble Guard", values: &[("band0_enabled", 1.0), ("band0_freq", 60.0), ("hp_slope", 1.0)] },
        FactoryPreset { name: "Mud Scoop", values: &[("band3_freq", 280.0), ("band3_gain2", -3.5), ("band3_q", 1.1), ("output_gain", 1.0)] },
        FactoryPreset { name: "Nasal Dip", values: &[("band4_freq", 900.0), ("band4_gain2", -3.0), ("band4_q", 2.0)] },
        FactoryPreset { name: "Sheen", values: &[("band6_freq", 9000.0), ("band6_gain2", 3.0), ("band6_q", 0.8), ("output_gain", -0.5)] },
        FactoryPreset { name: "Dialog Clean", values: &[("band0_enabled", 1.0), ("band0_freq", 100.0), ("band3_freq", 220.0), ("band3_gain2", -2.0), ("band3_q", 1.4), ("band5_freq", 3200.0), ("band5_gain2", 2.0), ("band5_q", 1.0)] },
        FactoryPreset { name: "Kick Weight", values: &[("band2_freq", 55.0), ("band2_gain2", 3.5), ("band2_q", 1.4), ("band4_freq", 350.0), ("band4_gain2", -3.0), ("band4_q", 1.6), ("band5_freq", 3500.0), ("band5_gain2", 2.0), ("band5_q", 1.2), ("output_gain", -1.0)] },
        FactoryPreset { name: "Snare Body", values: &[("band3_freq", 180.0), ("band3_gain2", 2.5), ("band3_q", 1.2), ("band5_freq", 5000.0), ("band5_gain2", 2.0), ("band5_q", 1.0), ("output_gain", -1.0)] },
        FactoryPreset { name: "Bass Focus", values: &[("band0_enabled", 1.0), ("band0_freq", 30.0), ("hp_slope", 1.0), ("band1_freq", 90.0), ("band1_gain2", 2.0), ("band1_q", 0.8), ("band3_freq", 300.0), ("band3_gain2", -2.5), ("band3_q", 1.2)] },
        FactoryPreset { name: "Lo-Fi Band", values: &[("band0_enabled", 1.0), ("band0_freq", 500.0), ("hp_slope", 1.0), ("band7_enabled", 1.0), ("band7_freq", 2800.0), ("lp_slope", 1.0), ("band4_freq", 1200.0), ("band4_gain2", 3.0), ("band4_q", 0.9), ("output_gain", 2.0)] },
        FactoryPreset { name: "Air Lift", values: &[("band6_freq", 12_000.0), ("band6_gain2", 3.5), ("band6_q", 0.7)] },
        FactoryPreset { name: "Tilt Dark", values: &[("band1_freq", 150.0), ("band1_gain2", 2.0), ("band1_q", 0.7), ("band6_freq", 4000.0), ("band6_gain2", -3.0), ("band6_q", 0.7)] },
        FactoryPreset { name: "Gentle Smile", values: &[("band1_freq", 80.0), ("band1_gain2", 2.5), ("band1_q", 0.7), ("band4_freq", 1000.0), ("band4_gain2", -1.0), ("band4_q", 0.6), ("band6_freq", 10_000.0), ("band6_gain2", 2.5), ("band6_q", 0.7), ("output_gain", -1.5)] },
    ],
    sidechain: false,
};
