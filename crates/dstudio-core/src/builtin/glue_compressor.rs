//! Glue Compressor: parameter ids, ranges, defaults, tapers, the panel
//! arrangement and the factory presets.

use super::*;
use crate::device::PluginKind;

const OFF_ON: &[&str] = &["Off", "On"];
const RATIOS: &[&str] = &["2:1", "4:1", "10:1"];
const ATTACKS: &[&str] = &["0.1 ms", "0.3 ms", "1 ms", "3 ms", "10 ms", "30 ms"];
const RELEASES: &[&str] = &["0.1 s", "0.3 s", "0.6 s", "1.2 s", "Auto"];
const SC_HPF: &[&str] = &["Off", "60", "90", "150", "300"];

pub static GLUE_COMPRESSOR: DeviceDef = DeviceDef {
    id: "glue-compressor",
    name: "Glue Compressor",
    kind: PluginKind::Effect,
    category: "Dynamics",
    purpose: "A bus compressor with stepped controls that pulls a group or mix together and adds weight.",
    params: &[
        ParamDef::new("threshold", "Threshold", -40.0, 0.0, -15.0, LINEAR, "dB", Format::Db)
            .step(0.1)
            .help("Level above which the compressor works."),
        ParamDef::choice("ratio", "Ratio", RATIOS, 1).help("2:1 glues gently, 4:1 is the classic bus setting, 10:1 squashes."),
        ParamDef::choice("attack", "Attack", ATTACKS, 5)
            .help("How fast reduction sets in; slower settings let more of each transient through."),
        ParamDef::choice("release", "Release", RELEASES, 4)
            .help("How fast reduction lets go; Auto recovers fast from short peaks and slowly from sustained ones."),
        ParamDef::new("makeup", "Makeup", 0.0, 24.0, 0.0, LINEAR, "dB", Format::Db).step(0.1).help("Gain after compression."),
        ParamDef::new("mix", "Mix", 0.0, 100.0, 100.0, LINEAR, "%", Format::Percent)
            .step(0.1)
            .help("Blend of compressed and dry signal (parallel compression)."),
        ParamDef::new("color", "Color", 0.0, 100.0, 50.0, LINEAR, "%", Format::Percent)
            .step(0.1)
            .help("Analogue-style saturation and weight from the gain stage."),
        ParamDef::new("range", "Range", 1.0, 24.0, 24.0, LINEAR, "dB", Format::Plain(1))
            .step(0.1)
            .help("Most gain reduction the compressor may apply."),
        ParamDef::choice("sc_hpf", "SC HPF", SC_HPF, 0)
            .label("SC HPF (Hz)")
            .help("Filters low end out of the detector so the kick does not make the bus pump."),
        ParamDef::choice("sidechain", "Sidechain", OFF_ON, 0).not_automatable().help("Set while a sidechain source is picked."),
        ParamDef::new("output_gain", "Output", -12.0, 12.0, 0.0, LINEAR, "dB", Format::Db).step(0.1).help("Output level."),
    ],
    layout: Layout {
        picker: &[Control::Menu("sc_hpf"), Control::Sidechain],
        view: Some(View::Strip(Strip::new("threshold").range(0.0, -48.0))),
        sliders: &[Control::Slider("output_gain")],
        clusters: &[
            &[Control::Knob("threshold"), Control::Knob("ratio")],
            &[Control::Knob("attack"), Control::Knob("release")],
            &[Control::Knob("range"), Control::Knob("makeup")],
            &[Control::Knob("color"), Control::Knob("mix")],
        ],
        faces: &[("ratio", Face::BusVca { threshold: "threshold", ratio: "ratio", range: "range" })],
        fixed_width: 0.0,
        min_height: 217.0,
        shown_when: &[],
        ..Layout::EMPTY
    },
    factory: &[
        FactoryPreset {
            name: "Drum Group Glue",
            values: &[
                ("threshold", -22.0),
                ("ratio", 1.0),
                ("attack", 3.0),
                ("release", 1.0),
                ("makeup", 5.5),
                ("color", 65.0),
                ("sc_hpf", 1.0),
            ],
        },
        FactoryPreset {
            name: "Mix Bus Gentle",
            values: &[
                ("threshold", -10.0),
                ("ratio", 0.0),
                ("attack", 5.0),
                ("release", 4.0),
                ("makeup", 2.5),
                ("color", 40.0),
                ("range", 6.0),
                ("sc_hpf", 2.0),
            ],
        },
        FactoryPreset {
            name: "Vocal Stack",
            values: &[
                ("threshold", -18.0),
                ("ratio", 1.0),
                ("attack", 2.0),
                ("release", 2.0),
                ("makeup", 5.0),
                ("color", 30.0),
                ("sc_hpf", 3.0),
            ],
        },
        FactoryPreset {
            name: "Keys Bed",
            values: &[
                ("threshold", -16.0),
                ("ratio", 0.0),
                ("attack", 4.0),
                ("release", 4.0),
                ("makeup", 4.0),
                ("color", 55.0),
                ("sc_hpf", 1.0),
            ],
        },
        FactoryPreset {
            name: "Room Crush",
            values: &[
                ("threshold", -30.0),
                ("ratio", 2.0),
                ("attack", 0.0),
                ("release", 1.0),
                ("makeup", 12.0),
                ("color", 85.0),
                ("range", 16.0),
                ("sc_hpf", 4.0),
            ],
        },
        FactoryPreset {
            name: "New York Blend",
            values: &[
                ("threshold", -28.0),
                ("ratio", 2.0),
                ("attack", 1.0),
                ("release", 0.0),
                ("makeup", 11.0),
                ("mix", 35.0),
                ("color", 70.0),
                ("output_gain", 1.5),
            ],
        },
        FactoryPreset {
            name: "Pump Feel",
            values: &[
                ("threshold", -26.0),
                ("ratio", 2.0),
                ("attack", 2.0),
                ("release", 1.0),
                ("makeup", 9.0),
                ("color", 50.0),
                ("range", 10.0),
            ],
        },
        FactoryPreset {
            name: "Clean Control",
            values: &[
                ("threshold", -12.0),
                ("ratio", 1.0),
                ("attack", 4.0),
                ("release", 4.0),
                ("makeup", 3.0),
                ("color", 0.0),
                ("range", 8.0),
                ("sc_hpf", 2.0),
            ],
        },
    ],
    sidechain: true,
    samples: 0,
};
