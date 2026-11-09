//! Reverb (reference: the reference app's built-in "Reverb"). Parameter ids, ranges,
//! defaults, tapers, display formats and the mode-dependent dial row are
//! taken from the reference as facts; factory preset names and values are our own.

use super::*;
use crate::device::PluginKind;

pub const MODES: &[&str] = &["Room", "Hall", "Plate", "Spring", "Shimmer", "Haze"];
pub const SHIMMER: u32 = 4;
pub const HAZE: u32 = 5;
const SHIFT1: &[&str] = &["-Oct", "-5th", "-4th", "-10c", "+10c", "+4th", "+5th", "+Oct", "+Oct+5th", "+2 Oct"];
const SHIFT2: &[&str] = &["Off", "-Oct", "-5th", "-4th", "-10c", "+10c", "+4th", "+5th", "+Oct", "+Oct+5th", "+2 Oct"];
/// Semitone offsets of the Shift 1 choices (Shift 2 = Off + the same list).
pub const SHIFT_SEMITONES: [f32; 10] = [-12.0, -7.0, -5.0, -0.1, 0.1, 5.0, 7.0, 12.0, 19.0, 24.0];
const OFF_ON: &[&str] = &["Off", "On"];

/// "0.45 s", "2.0 s".
pub fn seconds_text(v: f32) -> String {
    if v < 1.0 { format!("{v:.2} s") } else { format!("{v:.1} s") }
}

fn read_seconds(t: &str) -> Option<f32> {
    let t = t.trim();
    if let Some(ms) = t.strip_suffix("ms") {
        return ms.trim().parse::<f32>().ok().map(|v| v / 1000.0);
    }
    t.trim_end_matches("seconds").trim_end_matches("sec").trim_end_matches('s').trim().parse().ok()
}

/// "12.0k", "450 Hz".
pub fn cut_text(v: f32) -> String {
    if v >= 1000.0 { format!("{:.1}k", v / 1000.0) } else { format!("{v:.0} Hz") }
}

fn read_hz(t: &str) -> Option<f32> {
    let t = t.trim();
    if let Some(k) = t.strip_suffix("khz").or_else(|| t.strip_suffix('k')) {
        return k.trim().parse::<f32>().ok().map(|v| v * 1000.0);
    }
    t.trim_end_matches("hz").trim().parse().ok()
}

const SECONDS: Format = Format::Custom(TextFns { show: seconds_text, read: Some(read_seconds) });
const CUT: Format = Format::Custom(TextFns { show: cut_text, read: Some(read_hz) });
const ALL_BUT_SHIMMER: &[u32] = &[0, 1, 2, 3, 5];
const CLASSIC: &[u32] = &[0, 1, 2, 3];

pub static REVERB: DeviceDef = DeviceDef {
    id: "reverb",
    name: "Reverb",
    kind: PluginKind::Effect,
    category: "Reverb",
    purpose: "Places the sound in a space: rooms, halls, plates and springs, plus a pitch-shifted shimmer and a diffuse haze.",
    params: &[
        ParamDef::new("size", "Size", 0.0, 100.0, 50.0, LINEAR, "%", Format::Percent).help("Size of the space: spacing of reflections and density of the tail."),
        ParamDef::new("decay", "Decay", 0.1, 10.0, 2.0, Taper::Log, "s", SECONDS).help("Time for the tail to fall by 60 dB at mid frequencies."),
        ParamDef::new("damping", "Damping", 0.0, 100.0, 50.0, LINEAR, "%", Format::Percent).help("How much faster high frequencies die away than the mids."),
        ParamDef::new("predelay", "Pre-Delay", 0.0, 100.0, 10.0, LINEAR, "ms", Format::Time).help("Gap between the dry sound and the start of the reverb."),
        ParamDef::new("mix", "Mix", 0.0, 100.0, 30.0, LINEAR, "%", Format::Percent).help("Blend of reverb and dry signal."),
        ParamDef::new("width", "Width", 0.0, 100.0, 100.0, LINEAR, "%", Format::Percent).help("Stereo width of the reverb (0% is mono)."),
        ParamDef::choice("mode2", "Mode", MODES, 1).help("Reverb algorithm."),
        ParamDef::new("highcut2", "High Cut", 20.0, 20000.0, 12000.0, Taper::Log, "Hz", CUT).help("Low-pass filter on the reverb."),
        ParamDef::new("lowcut2", "Low Cut", 20.0, 20000.0, 20.0, Taper::Log, "Hz", CUT).help("High-pass filter on the reverb; at 20 Hz it is off."),
        ParamDef::choice("dry", "Dry", OFF_ON, 1).latch("Dry Off", "Dry On").help("Pass the dry signal; turn off when the reverb sits on a send."),
        ParamDef::new("moddepth", "Mod Depth", 0.0, 200.0, 20.0, LINEAR, "%", Format::Percent).help("Depth of the slow movement inside the tail (chorusing)."),
        ParamDef::new("modrate", "Mod Rate", 0.25, 4.0, 1.0, Taper::Log, "x", Format::Plain(1)).help("Speed of the tail movement, as a multiple of the mode's own rate."),
        ParamDef::choice("shift1", "Shift 1", SHIFT1, 7).help("First pitch interval fed back into the shimmer tail."),
        ParamDef::choice("shift2", "Shift 2", SHIFT2, 10).help("Second pitch interval of the shimmer tail (or Off)."),
        ParamDef::new("shimmer", "Shimmer", 0.0, 100.0, 60.0, LINEAR, "%", Format::Percent).help("How much pitch-shifted sound feeds the tail."),
        ParamDef::new("diffusion", "Diffusion", 0.0, 100.0, 50.0, LINEAR, "%", Format::Percent).help("How smeared and cloud-like the haze builds up."),
    ],
    layout: Layout {
        picker: &[Control::Dropdown("mode2"), Control::Gap, Control::Views, Control::Toggle("dry")],
        view: Some(View::Custom("reverb")),
        // Hidden dials drop out and empty clusters disappear: [3,3,1] (Room..Spring),
        // [1,2,3,1] (Shimmer), [3,2,1,1] (Haze).
        clusters: &[
            &[Control::Knob("decay"), Control::Knob("damping"), Control::Knob("size")],
            &[Control::Knob("predelay"), Control::Knob("width"), Control::Knob("moddepth")],
            &[Control::Knob("shift1"), Control::Knob("shift2"), Control::Knob("shimmer")],
            &[Control::Knob("diffusion")],
            &[Control::Knob("mix")],
        ],
        min_height: 196.0,
        shown_when: &[
            ("damping", "mode2", ALL_BUT_SHIMMER),
            ("size", "mode2", ALL_BUT_SHIMMER),
            ("width", "mode2", CLASSIC),
            ("shift1", "mode2", &[SHIMMER]),
            ("shift2", "mode2", &[SHIMMER]),
            ("shimmer", "mode2", &[SHIMMER]),
            ("diffusion", "mode2", &[HAZE]),
        ],
        // Widest of the mode layouts: [1,2,3,1] / [3,2,1,1] = 543 (the reference rule).
        fixed_width: 543.0,
        views: &["Tail", "EQ"],
        spread_dials: true,
        ..Layout::EMPTY
    },
    factory: &[
        FactoryPreset { name: "Booth Close", values: &[("size", 10.0), ("decay", 0.25), ("damping", 60.0), ("predelay", 1.0), ("mix", 20.0), ("width", 55.0), ("mode2", 0.0), ("highcut2", 9000.0), ("lowcut2", 180.0)] },
        FactoryPreset { name: "Wooden Studio", values: &[("size", 38.0), ("decay", 0.7), ("damping", 70.0), ("predelay", 3.0), ("mix", 24.0), ("mode2", 0.0), ("highcut2", 7500.0), ("lowcut2", 90.0)] },
        FactoryPreset { name: "Kit in the Garage", values: &[("size", 55.0), ("decay", 1.0), ("damping", 35.0), ("predelay", 6.0), ("mix", 40.0), ("mode2", 0.0), ("highcut2", 11000.0), ("lowcut2", 160.0)] },
        FactoryPreset { name: "Gated Snare Room", values: &[("size", 62.0), ("decay", 0.9), ("damping", 25.0), ("predelay", 0.0), ("mix", 80.0), ("mode2", 0.0), ("highcut2", 14000.0), ("lowcut2", 260.0)] },
        FactoryPreset { name: "Recital Hall", values: &[("size", 70.0), ("decay", 2.4), ("damping", 48.0), ("predelay", 22.0), ("mix", 22.0), ("mode2", 1.0), ("highcut2", 9500.0), ("lowcut2", 70.0)] },
        FactoryPreset { name: "Hall for Vocals", values: &[("size", 58.0), ("decay", 1.8), ("damping", 58.0), ("predelay", 45.0), ("mix", 20.0), ("mode2", 1.0), ("highcut2", 8500.0), ("lowcut2", 200.0)] },
        FactoryPreset { name: "Velvet Hall", values: &[("size", 78.0), ("decay", 3.0), ("damping", 75.0), ("predelay", 18.0), ("mix", 25.0), ("mode2", 1.0), ("highcut2", 5000.0), ("lowcut2", 70.0)] },
        FactoryPreset { name: "Stone Nave", values: &[("size", 100.0), ("decay", 6.5), ("damping", 60.0), ("predelay", 60.0), ("mix", 30.0), ("mode2", 1.0), ("highcut2", 6500.0), ("lowcut2", 110.0), ("moddepth", 35.0)] },
        FactoryPreset { name: "Orchestral Depth", values: &[("size", 88.0), ("decay", 3.2), ("damping", 42.0), ("predelay", 28.0), ("mix", 33.0), ("mode2", 1.0), ("highcut2", 10500.0), ("lowcut2", 85.0)] },
        FactoryPreset { name: "Silver Plate", values: &[("size", 52.0), ("decay", 1.7), ("damping", 25.0), ("predelay", 25.0), ("mix", 16.0), ("mode2", 2.0), ("highcut2", 13000.0), ("lowcut2", 220.0)] },
        FactoryPreset { name: "Snare Sheet", values: &[("size", 42.0), ("decay", 1.1), ("damping", 32.0), ("predelay", 10.0), ("mix", 45.0), ("mode2", 2.0), ("highcut2", 11500.0), ("lowcut2", 240.0)] },
        FactoryPreset { name: "Dark Steel Plate", values: &[("size", 55.0), ("decay", 2.2), ("damping", 65.0), ("predelay", 14.0), ("mix", 18.0), ("mode2", 2.0), ("highcut2", 6500.0), ("lowcut2", 260.0)] },
        FactoryPreset { name: "Long Shine Plate", values: &[("size", 80.0), ("decay", 4.5), ("damping", 30.0), ("predelay", 24.0), ("mix", 13.0), ("mode2", 2.0), ("highcut2", 12500.0), ("lowcut2", 180.0)] },
        FactoryPreset { name: "Amp Tank", values: &[("size", 38.0), ("decay", 1.4), ("damping", 48.0), ("predelay", 0.0), ("mix", 30.0), ("mode2", 3.0), ("highcut2", 5800.0), ("lowcut2", 140.0)] },
        FactoryPreset { name: "Reggae Tank", values: &[("size", 62.0), ("decay", 2.8), ("damping", 38.0), ("predelay", 0.0), ("mix", 40.0), ("mode2", 3.0), ("highcut2", 4600.0), ("lowcut2", 220.0), ("moddepth", 45.0)] },
        FactoryPreset { name: "Twangy Drip", values: &[("size", 48.0), ("decay", 2.0), ("damping", 22.0), ("predelay", 0.0), ("mix", 45.0), ("mode2", 3.0), ("highcut2", 8200.0), ("lowcut2", 320.0)] },
        FactoryPreset { name: "Rising Octaves", values: &[("size", 82.0), ("decay", 6.5), ("damping", 42.0), ("predelay", 18.0), ("mix", 38.0), ("mode2", 4.0), ("highcut2", 12000.0), ("lowcut2", 160.0), ("shift1", 7.0), ("shift2", 10.0), ("shimmer", 55.0)] },
        FactoryPreset { name: "Fifths Above", values: &[("size", 72.0), ("decay", 4.6), ("damping", 50.0), ("predelay", 15.0), ("mix", 30.0), ("mode2", 4.0), ("highcut2", 9800.0), ("lowcut2", 150.0), ("shift1", 6.0), ("shift2", 8.0), ("shimmer", 45.0)] },
        FactoryPreset { name: "Low Shimmer Floor", values: &[("size", 88.0), ("decay", 6.0), ("damping", 62.0), ("predelay", 12.0), ("mix", 32.0), ("mode2", 4.0), ("highcut2", 6200.0), ("lowcut2", 90.0), ("shift1", 0.0), ("shift2", 0.0), ("shimmer", 40.0)] },
        FactoryPreset { name: "Chorused Halo", values: &[("size", 68.0), ("decay", 3.8), ("damping", 48.0), ("predelay", 22.0), ("mix", 30.0), ("mode2", 4.0), ("highcut2", 10000.0), ("lowcut2", 130.0), ("moddepth", 35.0), ("shift1", 3.0), ("shift2", 5.0), ("shimmer", 65.0)] },
        FactoryPreset { name: "Mist", values: &[("size", 62.0), ("decay", 5.0), ("damping", 48.0), ("predelay", 28.0), ("mix", 45.0), ("mode2", 5.0), ("highcut2", 11000.0), ("lowcut2", 130.0), ("diffusion", 55.0)] },
        FactoryPreset { name: "Overcast", values: &[("size", 75.0), ("decay", 7.5), ("damping", 68.0), ("predelay", 35.0), ("mix", 36.0), ("mode2", 5.0), ("highcut2", 6000.0), ("lowcut2", 180.0), ("diffusion", 70.0)] },
        FactoryPreset { name: "Infinite Breath", values: &[("size", 100.0), ("decay", 10.0), ("damping", 45.0), ("predelay", 45.0), ("mix", 48.0), ("mode2", 5.0), ("highcut2", 9500.0), ("lowcut2", 140.0), ("moddepth", 45.0), ("diffusion", 95.0)] },
        FactoryPreset { name: "Tape Fog", values: &[("size", 58.0), ("decay", 3.2), ("damping", 72.0), ("predelay", 8.0), ("mix", 55.0), ("mode2", 5.0), ("highcut2", 2800.0), ("lowcut2", 380.0), ("moddepth", 110.0), ("modrate", 1.4), ("diffusion", 60.0)] },
        FactoryPreset { name: "Drift Hall", values: &[("size", 92.0), ("decay", 5.5), ("damping", 58.0), ("predelay", 55.0), ("mix", 58.0), ("mode2", 1.0), ("highcut2", 6500.0), ("lowcut2", 190.0), ("moddepth", 65.0), ("modrate", 0.7)] },
        FactoryPreset { name: "Send: Room", values: &[("size", 42.0), ("decay", 0.75), ("damping", 50.0), ("predelay", 5.0), ("mix", 100.0), ("mode2", 0.0), ("highcut2", 9500.0), ("lowcut2", 160.0), ("dry", 0.0)] },
        FactoryPreset { name: "Send: Plate", values: &[("size", 50.0), ("decay", 1.5), ("damping", 38.0), ("predelay", 28.0), ("mix", 100.0), ("mode2", 2.0), ("highcut2", 11000.0), ("lowcut2", 210.0), ("dry", 0.0)] },
        FactoryPreset { name: "Send: Big Hall", values: &[("size", 85.0), ("decay", 3.2), ("damping", 52.0), ("predelay", 30.0), ("mix", 100.0), ("mode2", 1.0), ("highcut2", 8500.0), ("lowcut2", 130.0), ("dry", 0.0)] },
    ],
    sidechain: false,
    hooks: Hooks::NONE,
};
