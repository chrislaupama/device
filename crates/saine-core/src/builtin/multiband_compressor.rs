//! Multiband Compressor (reference: the reference app's built-in "Multiband Compressor").
//! Parameter ids, ranges, defaults, tapers and the panel arrangement are taken
//! from the device spec as facts; the factory preset names and values are our own.
//!
//! Three bands split at two crossovers. Each band compresses downwards above
//! its threshold and upwards below its floor (up threshold); Depth, Upward,
//! Downward and Time act on all bands. The card shows one band at a time (a
//! band selector and the band field, crates/saine-app/src/panel_multiband.rs);
//! the dial row is defined with the Low band's knobs and follows the selection.

use super::*;
use crate::device::PluginKind;

/// Parameter ids per band (Low, Mid, High): threshold, ratio, attack,
/// release, makeup, input, floor (up threshold), lift (up ratio).
pub const MULTIBAND_BANDS: [[&str; 8]; 3] = [
    ["low_threshold", "low_ratio", "low_attack", "low_release", "low_makeup", "low_input", "low_up_threshold", "low_up_ratio"],
    ["mid_threshold", "mid_ratio", "mid_attack", "mid_release", "mid_makeup", "mid_input", "mid_up_threshold", "mid_up_ratio"],
    ["high_threshold", "high_ratio", "high_attack", "high_release", "high_makeup", "high_input", "high_up_threshold", "high_up_ratio"],
];

fn ratio_text(v: f32) -> String {
    if v >= 10.0 { format!("{v:.0}:1") } else { format!("{v:.1}:1") }
}

fn attack_text(v: f32) -> String {
    if v < 1.0 { format!("{v:.2} ms") } else { format!("{v:.1} ms") }
}

fn release_text(v: f32) -> String {
    if v < 100.0 { format!("{v:.1} ms") } else { format!("{v:.0} ms") }
}

fn percent_text(v: f32) -> String {
    format!("{:.0}%", v.round() + 0.0)
}

const THR: Format = Format::Custom(TextFn(db_text));
const RATIO: Format = Format::Custom(TextFn(ratio_text));
const ATTACK: Format = Format::Custom(TextFn(attack_text));
const RELEASE: Format = Format::Custom(TextFn(release_text));
const PCT: Format = Format::Custom(TextFn(percent_text));

macro_rules! band {
    ($b:literal, $name:literal, $thr:literal, $ratio:literal, $att:literal, $rel:literal) => {
        [
            ParamDef::new(concat!($b, "_threshold"), concat!($name, " Threshold"), -60.0, 0.0, $thr, LINEAR, "dB", THR).label("Threshold").help("Level above which this band is compressed downwards."),
            ParamDef::new(concat!($b, "_ratio"), concat!($name, " Ratio"), 1.0, 20.0, $ratio, Taper::Log, ":1", RATIO).label("Ratio").help("Downward ratio of this band."),
            ParamDef::new(concat!($b, "_attack"), concat!($name, " Attack"), 0.1, 200.0, $att, Taper::Log, "ms", ATTACK).label("Attack").help("How fast this band's detector follows rising level."),
            ParamDef::new(concat!($b, "_release"), concat!($name, " Release"), 10.0, 2000.0, $rel, Taper::Log, "ms", RELEASE).label("Release").help("How fast this band's detector follows falling level."),
            ParamDef::new(concat!($b, "_makeup"), concat!($name, " Makeup"), -12.0, 12.0, 0.0, LINEAR, "dB", Format::Db).label("Makeup").help("Gain of this band after compression."),
        ]
    };
}

macro_rules! band_up {
    ($b:literal, $name:literal) => {
        [
            ParamDef::new(concat!($b, "_input"), concat!($name, " Input"), -24.0, 24.0, 0.0, LINEAR, "dB", Format::Db).label("Input").help("Gain into this band before its detector."),
            ParamDef::new(concat!($b, "_up_threshold"), concat!($name, " Up Threshold"), -80.0, 0.0, -60.0, LINEAR, "dB", THR).label("Floor").help("Level below which this band is lifted (upward compression)."),
            ParamDef::new(concat!($b, "_up_ratio"), concat!($name, " Up Ratio"), 1.0, 20.0, 1.0, Taper::Log, ":1", RATIO).label("Lift").help("Upward ratio of this band; 1:1 is off."),
        ]
    };
}

const LOW: [ParamDef; 5] = band!("low", "Low", -18.0, 2.5, 30.0, 200.0);
const MID: [ParamDef; 5] = band!("mid", "Mid", -20.0, 2.0, 15.0, 120.0);
const HIGH: [ParamDef; 5] = band!("high", "High", -22.0, 2.0, 5.0, 80.0);
const LOW_UP: [ParamDef; 3] = band_up!("low", "Low");
const MID_UP: [ParamDef; 3] = band_up!("mid", "Mid");
const HIGH_UP: [ParamDef; 3] = band_up!("high", "High");

pub static MULTIBAND_COMPRESSOR: DeviceDef = DeviceDef {
    id: "multiband-compressor",
    name: "Multiband Compressor",
    kind: PluginKind::Effect,
    category: "Dynamics",
    purpose: "Splits the sound into low, mid and high bands and compresses each down above its threshold and up below its floor, for density and balance.",
    params: &[
        ParamDef::new("low_crossover", "Low Crossover", 20.0, 1000.0, 120.0, Taper::Log, "Hz", Format::Freq).help("Split between the low and mid bands."),
        ParamDef::new("high_crossover", "High Crossover", 1000.0, 20000.0, 2500.0, Taper::Log, "Hz", Format::Freq).help("Split between the mid and high bands."),
        LOW[0], LOW[1], LOW[2], LOW[3], LOW[4],
        MID[0], MID[1], MID[2], MID[3], MID[4],
        HIGH[0], HIGH[1], HIGH[2], HIGH[3], HIGH[4],
        ParamDef::new("mix", "Mix", 0.0, 100.0, 100.0, LINEAR, "%", PCT).help("Blend of dry and processed signal."),
        ParamDef::new("output_gain", "Output", -12.0, 12.0, 0.0, LINEAR, "dB", Format::Db).help("Output level."),
        LOW_UP[0], LOW_UP[1], LOW_UP[2],
        MID_UP[0], MID_UP[1], MID_UP[2],
        HIGH_UP[0], HIGH_UP[1], HIGH_UP[2],
        ParamDef::new("input_gain", "Input", -24.0, 24.0, 0.0, LINEAR, "dB", Format::Db).help("Gain into all bands."),
        ParamDef::new("time", "Time", 10.0, 1000.0, 100.0, Taper::Log, "%", PCT).help("Scales every band's attack and release."),
        ParamDef::new("upward", "Upward", 0.0, 200.0, 100.0, LINEAR, "%", PCT).help("Scales the upward compression of all bands."),
        ParamDef::new("downward", "Downward", 0.0, 200.0, 100.0, LINEAR, "%", PCT).help("Scales the downward compression of all bands."),
        ParamDef::new("depth", "Depth", 0.0, 100.0, 100.0, LINEAR, "%", PCT).help("Overall amount of the gain changes."),
    ],
    layout: Layout {
        picker: &[],
        view: Some(View::Custom("multiband")),
        sliders: &[Control::Slider("output_gain")],
        clusters: &[
            &[Control::Knob("low_threshold"), Control::Knob("low_ratio")],
            &[Control::Knob("low_up_threshold"), Control::Knob("low_up_ratio")],
            &[Control::Knob("depth"), Control::Knob("time")],
            &[Control::Knob("mix")],
        ],
        faces: &[("low_ratio", Face::Transfer)],
        min_height: 213.0,
        shown_when: &[],
        fixed_width: 0.0,
    },
    factory: &[
        FactoryPreset {
            name: "Three-Way Squash",
            values: &[
                ("low_crossover", 95.0), ("high_crossover", 2800.0),
                ("low_threshold", -38.0), ("low_ratio", 16.0), ("low_attack", 5.0), ("low_release", 35.0), ("low_makeup", 10.0), ("low_up_threshold", -44.0), ("low_up_ratio", 3.5),
                ("mid_threshold", -33.0), ("mid_ratio", 16.0), ("mid_attack", 4.0), ("mid_release", 32.0), ("mid_makeup", 6.0), ("mid_up_threshold", -45.0), ("mid_up_ratio", 3.5),
                ("high_threshold", -40.0), ("high_ratio", 16.0), ("high_attack", 6.0), ("high_release", 16.0), ("high_makeup", 10.0), ("high_up_threshold", -45.0), ("high_up_ratio", 3.5),
                ("depth", 80.0), ("output_gain", 1.0),
            ],
        },
        FactoryPreset {
            name: "Half-Strength Squash",
            values: &[
                ("low_crossover", 95.0), ("high_crossover", 2800.0),
                ("low_threshold", -38.0), ("low_ratio", 16.0), ("low_attack", 5.0), ("low_release", 35.0), ("low_makeup", 10.0), ("low_up_threshold", -44.0), ("low_up_ratio", 3.5),
                ("mid_threshold", -33.0), ("mid_ratio", 16.0), ("mid_attack", 4.0), ("mid_release", 32.0), ("mid_makeup", 6.0), ("mid_up_threshold", -45.0), ("mid_up_ratio", 3.5),
                ("high_threshold", -40.0), ("high_ratio", 16.0), ("high_attack", 6.0), ("high_release", 16.0), ("high_makeup", 10.0), ("high_up_threshold", -45.0), ("high_up_ratio", 3.5),
                ("depth", 40.0), ("output_gain", 1.5),
            ],
        },
        FactoryPreset {
            name: "Mastering Polish",
            values: &[("low_threshold", -22.0), ("low_ratio", 1.6), ("low_attack", 35.0), ("low_release", 220.0), ("mid_threshold", -22.0), ("mid_ratio", 1.4), ("mid_attack", 25.0), ("mid_release", 160.0), ("high_threshold", -28.0), ("high_ratio", 1.6), ("high_attack", 12.0), ("high_release", 110.0), ("output_gain", 1.0)],
        },
        FactoryPreset {
            name: "Bus Density",
            values: &[("low_threshold", -26.0), ("low_ratio", 3.5), ("low_attack", 2.0), ("low_release", 110.0), ("low_makeup", 2.5), ("mid_threshold", -24.0), ("mid_ratio", 3.0), ("mid_attack", 2.0), ("mid_release", 85.0), ("mid_makeup", 2.5), ("high_threshold", -30.0), ("high_ratio", 3.0), ("high_attack", 1.5), ("high_release", 55.0), ("high_makeup", 2.0), ("output_gain", 3.5)],
        },
        FactoryPreset {
            name: "Quiet Detail Up",
            values: &[("low_ratio", 1.0), ("mid_ratio", 1.0), ("high_ratio", 1.0), ("low_up_threshold", -26.0), ("low_up_ratio", 3.0), ("mid_up_threshold", -24.0), ("mid_up_ratio", 3.0), ("high_up_threshold", -34.0), ("high_up_ratio", 2.5), ("output_gain", -1.0)],
        },
        FactoryPreset {
            name: "Drum Bus Snap",
            values: &[("low_threshold", -22.0), ("low_ratio", 3.5), ("low_attack", 30.0), ("low_release", 70.0), ("mid_threshold", -24.0), ("mid_ratio", 3.0), ("mid_attack", 22.0), ("mid_release", 55.0), ("high_threshold", -30.0), ("high_ratio", 2.5), ("high_attack", 12.0), ("high_release", 45.0), ("output_gain", 2.5)],
        },
        FactoryPreset {
            name: "Drum Bus Crush",
            values: &[("low_threshold", -30.0), ("low_ratio", 7.0), ("low_attack", 0.4), ("low_release", 55.0), ("low_makeup", 6.0), ("mid_threshold", -30.0), ("mid_ratio", 7.0), ("mid_attack", 0.4), ("mid_release", 45.0), ("mid_makeup", 6.0), ("high_threshold", -34.0), ("high_ratio", 6.0), ("high_attack", 0.2), ("high_release", 35.0), ("high_makeup", 5.0), ("mix", 70.0), ("output_gain", 4.0)],
        },
        FactoryPreset {
            name: "Low End Control",
            values: &[("low_crossover", 110.0), ("low_threshold", -20.0), ("low_ratio", 3.5), ("low_attack", 18.0), ("low_release", 140.0), ("low_makeup", 4.0), ("mid_ratio", 1.0), ("high_ratio", 1.0)],
        },
        FactoryPreset {
            name: "Vocal Even Out",
            values: &[("low_crossover", 220.0), ("high_crossover", 4000.0), ("low_threshold", -32.0), ("low_ratio", 2.5), ("low_attack", 15.0), ("low_release", 130.0), ("mid_threshold", -25.0), ("mid_ratio", 3.0), ("mid_attack", 8.0), ("mid_release", 90.0), ("mid_makeup", 2.0), ("high_threshold", -38.0), ("high_ratio", 2.5), ("high_attack", 1.5), ("high_release", 55.0), ("output_gain", 3.0)],
        },
        FactoryPreset {
            name: "Voice-Over Presence",
            values: &[("low_crossover", 160.0), ("high_crossover", 3500.0), ("low_threshold", -28.0), ("low_ratio", 4.0), ("low_attack", 8.0), ("low_release", 90.0), ("mid_threshold", -24.0), ("mid_ratio", 3.5), ("mid_attack", 4.0), ("mid_release", 70.0), ("mid_makeup", 3.0), ("mid_up_threshold", -48.0), ("mid_up_ratio", 1.8), ("high_threshold", -34.0), ("high_ratio", 2.5), ("high_attack", 1.5), ("high_release", 50.0), ("high_makeup", 3.0), ("output_gain", 3.0)],
        },
        FactoryPreset {
            name: "Acoustic Balance",
            values: &[("low_crossover", 200.0), ("high_crossover", 3200.0), ("low_threshold", -24.0), ("low_ratio", 2.5), ("low_attack", 25.0), ("low_release", 170.0), ("mid_threshold", -21.0), ("mid_ratio", 1.8), ("mid_attack", 18.0), ("mid_release", 130.0), ("high_threshold", -34.0), ("high_ratio", 1.8), ("high_attack", 6.0), ("high_release", 90.0), ("output_gain", 2.0)],
        },
        FactoryPreset {
            name: "Harsh Highs Tamer",
            values: &[("high_crossover", 3200.0), ("low_ratio", 1.0), ("mid_ratio", 1.0), ("high_threshold", -32.0), ("high_ratio", 3.5), ("high_attack", 0.8), ("high_release", 70.0)],
        },
        FactoryPreset {
            name: "Mud Reducer",
            values: &[("low_crossover", 180.0), ("high_crossover", 1200.0), ("low_ratio", 1.0), ("mid_threshold", -24.0), ("mid_ratio", 3.5), ("mid_attack", 12.0), ("mid_release", 110.0), ("high_ratio", 1.0), ("output_gain", 1.5)],
        },
        FactoryPreset {
            name: "Cymbal Softener",
            values: &[("high_crossover", 7000.0), ("low_ratio", 1.0), ("mid_ratio", 1.0), ("high_threshold", -36.0), ("high_ratio", 3.5), ("high_attack", 0.3), ("high_release", 55.0)],
        },
        FactoryPreset {
            name: "Upward Air",
            values: &[("low_ratio", 1.0), ("mid_ratio", 1.0), ("high_ratio", 1.0), ("high_crossover", 5000.0), ("high_up_threshold", -40.0), ("high_up_ratio", 3.0), ("mid_up_threshold", -50.0), ("mid_up_ratio", 1.5), ("downward", 0.0), ("depth", 60.0)],
        },
        FactoryPreset {
            name: "Parallel Band Crush",
            values: &[
                ("low_threshold", -36.0), ("low_ratio", 12.0), ("low_attack", 3.0), ("low_release", 40.0), ("low_makeup", 9.0),
                ("mid_threshold", -34.0), ("mid_ratio", 12.0), ("mid_attack", 3.0), ("mid_release", 35.0), ("mid_makeup", 7.0),
                ("high_threshold", -40.0), ("high_ratio", 12.0), ("high_attack", 2.0), ("high_release", 25.0), ("high_makeup", 9.0),
                ("mix", 45.0), ("time", 200.0), ("output_gain", 3.0),
            ],
        },
    ],
    sidechain: false,
    samples: 0,
};

/// One-sided ramp with a 0.5 dB soft knee: 0 below -0.25 dB, the identity above +0.25 dB.
fn ramp(o: f32) -> f32 {
    const K: f32 = 0.5;
    if o <= -K / 2.0 {
        0.0
    } else if o >= K / 2.0 {
        o
    } else {
        (o + K / 2.0).powi(2) / (2.0 * K)
    }
}

/// Gain change (dB) of one band for a band level `x` (dB): downward above
/// the threshold, upward below the floor, limited to −40..+36 dB and scaled
/// by Depth. `downward`, `upward` and `depth` are in percent. The engine
/// kernel uses the same law.
#[allow(clippy::too_many_arguments)]
pub fn multiband_gain(x: f32, thr: f32, ratio: f32, floor: f32, lift: f32, downward: f32, upward: f32, depth: f32) -> f32 {
    if ratio < 1.0 || lift < 1.0 {
        return 0.0;
    }
    let down = ((1.0 - 1.0 / ratio) * downward / 100.0).min(1.0);
    let up = (1.0 - 1.0 / lift) * upward / 100.0;
    (-down * ramp(x - thr) + up * ramp(floor - x)).clamp(-40.0, 36.0) * depth / 100.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def() -> &'static DeviceDef {
        &MULTIBAND_COMPRESSOR
    }

    #[test]
    fn parameter_order_and_band_map_follow_the_reference() {
        assert_eq!(def().params.len(), 33);
        for (b, ids) in MULTIBAND_BANDS.iter().enumerate() {
            for (o, id) in ids.iter().enumerate() {
                let want = if o < 5 { 2 + 5 * b + o } else { 19 + 3 * b + (o - 5) };
                assert_eq!(def().index_of(id), Some(want), "{id}");
            }
        }
        assert_eq!(def().index_of("mix"), Some(17));
        assert_eq!(def().index_of("input_gain"), Some(28));
        assert_eq!(def().index_of("depth"), Some(32));
        assert_eq!(def().param("mid_up_threshold").unwrap().label, "Floor");
        assert_eq!(def().param("high_up_ratio").unwrap().label, "Lift");
    }

    #[test]
    fn card_width_matches_the_reference() {
        assert_eq!(def().layout.width(), 543.0, "clusters [2,2,2,1] -> 543");
    }

    #[test]
    fn value_text() {
        let p = |id| def().param(id).unwrap();
        assert_eq!(p("low_threshold").format(-18.0), "\u{2212}18.0 dB");
        assert_eq!(p("low_up_threshold").format(-60.0), "\u{2212}60.0 dB");
        assert_eq!(p("low_ratio").format(2.5), "2.5:1");
        assert_eq!(p("low_up_ratio").format(1.0), "1.0:1");
        assert_eq!(p("mid_ratio").format(12.0), "12:1");
        assert_eq!(p("high_attack").format(0.3), "0.30 ms");
        assert_eq!(p("high_attack").format(5.0), "5.0 ms");
        assert_eq!(p("mid_release").format(80.0), "80.0 ms");
        assert_eq!(p("mid_release").format(120.0), "120 ms");
        assert_eq!(p("low_crossover").format(120.0), "120 Hz");
        assert_eq!(p("high_crossover").format(2500.0), "2.5 kHz");
        assert_eq!(p("time").format(100.0), "100%");
        assert_eq!(p("output_gain").format(0.0), "+0.0 dB");
        assert_eq!(p("high_crossover").parse("4k"), Some(4000.0));
        assert_eq!(p("low_attack").parse("2 ms"), Some(2.0));
    }

    #[test]
    fn band_law() {
        // Defaults: no upward (1:1), 2.5:1 downward.
        assert_eq!(multiband_gain(-40.0, -18.0, 2.5, -60.0, 1.0, 100.0, 100.0, 100.0), 0.0);
        assert!((multiband_gain(-8.0, -18.0, 2.5, -60.0, 1.0, 100.0, 100.0, 100.0) + 6.0).abs() < 1e-4);
        // Upward below the floor: 4:1 lift, 10 dB under -> +7.5 dB.
        assert!((multiband_gain(-55.0, -18.0, 2.5, -45.0, 4.0, 100.0, 100.0, 100.0) - 7.5).abs() < 1e-4);
        // Depth, Downward and Upward scale; the result is limited.
        assert!((multiband_gain(-8.0, -18.0, 2.5, -60.0, 1.0, 100.0, 100.0, 50.0) + 3.0).abs() < 1e-4);
        assert!((multiband_gain(-8.0, -18.0, 2.5, -60.0, 1.0, 50.0, 100.0, 100.0) + 3.0).abs() < 1e-4);
        assert_eq!(multiband_gain(-8.0, -18.0, 20.0, -60.0, 1.0, 200.0, 100.0, 100.0), -10.0, "downward slope capped at 1");
        assert_eq!(multiband_gain(-200.0, -18.0, 2.0, -20.0, 20.0, 100.0, 200.0, 100.0), 36.0);
        // Soft 0.5 dB knee: continuous at the threshold.
        assert!(multiband_gain(-18.0, -18.0, 4.0, -60.0, 1.0, 100.0, 100.0, 100.0) < 0.0);
        assert!(multiband_gain(-18.0, -18.0, 4.0, -60.0, 1.0, 100.0, 100.0, 100.0) > -0.1);
    }
}
