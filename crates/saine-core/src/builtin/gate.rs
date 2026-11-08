//! Gate (reference: the reference app's built-in "Gate"). Parameter ids,
//! ranges, defaults, tapers and the panel arrangement are taken from the
//! reference as facts; the factory preset names and values are our own.

use super::*;
use crate::device::PluginKind;

pub static GATE: DeviceDef = DeviceDef {
    id: "gate",
    name: "Gate",
    kind: PluginKind::Effect,
    category: "Dynamics",
    purpose: "Turns a signal down while it is below a threshold, cutting noise, bleed and room tails between hits.",
    params: &[
        ParamDef::new("threshold", "Threshold", -80.0, 0.0, -40.0, LINEAR, "dB", Format::Db).help("Level the signal must exceed to open the gate."),
        ParamDef::new("ratio", "Ratio", 1.0, 100.0, 10.0, Taper::Log, ":1", Format::Ratio).help("How hard level below the threshold is pushed down; high values close the gate fully."),
        ParamDef::new("attack", "Attack", 0.01, 50.0, 0.5, Taper::Log, "ms", Format::Ms(2)).help("How fast the gate opens."),
        ParamDef::new("hold", "Hold", 0.0, 500.0, 50.0, LINEAR, "ms", Format::Ms(0)).help("How long the gate stays open after the signal falls below the threshold."),
        ParamDef::new("release", "Release", 1.0, 2000.0, 100.0, Taper::Log, "ms", Format::Ms(1)).help("How fast the gate closes after the hold."),
        ParamDef::new("lookahead", "Lookahead", 0.0, 20.0, 5.0, LINEAR, "ms", Format::Ms(1)).help("Opens the gate this much before a hit, so its start is not cut."),
        ParamDef::new("sidechain_hpf", "Sidechain HPF", 20.0, 500.0, 80.0, Taper::Log, "Hz", Format::Hz).help("Filters low end out of the detector only, so rumble or bass does not hold the gate open."),
        ParamDef::new("mix", "Mix", 0.0, 100.0, 100.0, LINEAR, "%", Format::Percent).help("Blend of gated and dry signal."),
    ],
    layout: Layout {
        picker: &[],
        view: Some(View::Strip(Strip::new("threshold").range(12.0, -60.0))),
        sliders: &[],
        clusters: &[
            &[Control::Knob("threshold"), Control::Knob("ratio")],
            &[Control::Knob("attack"), Control::Knob("hold"), Control::Knob("release")],
            &[Control::Knob("lookahead")],
            &[Control::Knob("mix")],
        ],
        faces: &[("ratio", Face::Expander { threshold: "threshold", ratio: "ratio" })],
        min_height: 180.0,
        shown_when: &[],
    },
    factory: &[
        FactoryPreset { name: "Kick Tightener", values: &[("threshold", -20.0), ("ratio", 80.0), ("attack", 0.2), ("hold", 12.0), ("release", 40.0), ("lookahead", 3.0), ("sidechain_hpf", 30.0)] },
        FactoryPreset { name: "Snare Isolate", values: &[("threshold", -22.0), ("ratio", 40.0), ("attack", 0.15), ("hold", 30.0), ("release", 80.0), ("lookahead", 3.0), ("sidechain_hpf", 180.0)] },
        FactoryPreset { name: "Tom Ring Control", values: &[("threshold", -26.0), ("ratio", 20.0), ("attack", 0.3), ("hold", 60.0), ("release", 150.0), ("lookahead", 3.0), ("sidechain_hpf", 60.0)] },
        FactoryPreset { name: "Voice Clean", values: &[("threshold", -38.0), ("ratio", 6.0), ("attack", 1.5), ("hold", 70.0), ("release", 220.0), ("lookahead", 4.0), ("sidechain_hpf", 120.0)] },
        FactoryPreset { name: "Amp Hiss Stop", values: &[("threshold", -48.0), ("ratio", 12.0), ("attack", 0.8), ("hold", 40.0), ("release", 180.0)] },
        FactoryPreset { name: "Soft Expander", values: &[("threshold", -32.0), ("ratio", 2.0), ("attack", 3.0), ("hold", 20.0), ("release", 250.0)] },
        FactoryPreset { name: "Stutter Chop", values: &[("threshold", -10.0), ("ratio", 100.0), ("attack", 0.03), ("hold", 0.0), ("release", 8.0), ("lookahead", 1.0)] },
        FactoryPreset { name: "Big Gated Room", values: &[("threshold", -15.0), ("ratio", 100.0), ("attack", 0.08), ("hold", 120.0), ("release", 6.0), ("lookahead", 1.5), ("sidechain_hpf", 250.0)] },
    ],
    sidechain: false,
};
