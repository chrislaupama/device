//! Delay (reference: the reference app's built-in "Delay"). Parameter ids, ranges,
//! defaults, tapers, display formats and the panel arrangement are taken from
//! the reference as facts; factory preset names and values are our own.

use super::*;
use crate::device::PluginKind;

/// Note values of the Note dials, longest first.
pub const DIVISIONS: &[&str] = &["2/1", "1/1", "Dot 1/2", "1/2", "Trip 1/2", "Dot 1/4", "1/4", "Trip 1/4", "Dot 1/8", "1/8", "Trip 1/8", "Dot 1/16", "1/16", "Trip 1/16", "1/32"];
/// Length of each division in quarter notes.
pub const DIVISION_BEATS: [f32; 15] = [8.0, 4.0, 3.0, 2.0, 4.0 / 3.0, 1.5, 1.0, 2.0 / 3.0, 0.75, 0.5, 1.0 / 3.0, 0.375, 0.25, 1.0 / 6.0, 0.125];
pub const MODES: &[&str] = &["Tape", "Digital", "BBD"];
const OFF_ON: &[&str] = &["Off", "On"];
const FREE_SYNC: &[&str] = &["Free", "Sync"];
/// EQ bands acting on the repeats: name, default freq, Q, enabled.
pub const BANDS: [(&str, f32, f32, bool); 6] = [
    ("High Pass", 20.0, 0.71, false),
    ("Low Shelf", 150.0, 1.0, true),
    ("Low Mid", 500.0, 1.0, true),
    ("High Mid", 3000.0, 1.0, true),
    ("High Shelf", 8000.0, 1.0, true),
    ("Low Pass", 20000.0, 0.71, false),
];

/// "500 ms", "4.5 ms", "1.25 s".
pub fn ms_text(v: f32) -> String {
    if v >= 1000.0 {
        format!("{:.2} s", v / 1000.0)
    } else if v < 10.0 {
        format!("{v:.1} ms")
    } else {
        format!("{v:.0} ms")
    }
}

fn read_ms(t: &str) -> Option<f32> {
    let t = t.trim();
    if let Some(ms) = t.strip_suffix("ms").or_else(|| t.strip_suffix("msec")) {
        return ms.trim().parse().ok();
    }
    if let Some(s) = t.strip_suffix("seconds").or_else(|| t.strip_suffix("sec")).or_else(|| t.strip_suffix('s')) {
        return s.trim().parse::<f32>().ok().map(|v| v * 1000.0);
    }
    None
}

/// "0.07 Hz", "2.5 Hz".
pub fn rate_text(v: f32) -> String {
    if v < 1.0 { format!("{v:.2} Hz") } else { format!("{v:.1} Hz") }
}

/// "Mono", "Stereo", "Ping 40%", "60%".
pub fn width_text(v: f32) -> String {
    let r = v.round();
    if r <= 0.0 {
        "Mono".into()
    } else if r == 100.0 {
        "Stereo".into()
    } else if r > 100.0 {
        format!("Ping {}%", r as i32 - 100)
    } else {
        format!("{}%", r as i32)
    }
}

fn read_width(t: &str) -> Option<f32> {
    let t = t.trim();
    match t {
        "mono" => Some(0.0),
        "stereo" => Some(100.0),
        _ => t.strip_prefix("ping").and_then(|n| n.trim().trim_end_matches('%').trim().parse::<f32>().ok()).map(|n| 100.0 + n),
    }
}

/// "440 Hz", "2.5 kHz".
pub fn freq_text(v: f32) -> String {
    if v >= 1000.0 { format!("{:.1} kHz", v / 1000.0) } else { format!("{v:.0} Hz") }
}

fn read_freq(t: &str) -> Option<f32> {
    let t = t.trim();
    if let Some(k) = t.strip_suffix("khz").or_else(|| t.strip_suffix('k')) {
        return k.trim().parse::<f32>().ok().map(|v| v * 1000.0);
    }
    None
}

/// "0.71", "12.0".
pub fn q_text(v: f32) -> String {
    if v >= 10.0 { format!("{v:.1}") } else { format!("{v:.2}") }
}

const MS: Format = Format::Custom(TextFns { show: ms_text, read: Some(read_ms) });
const FREQ: Format = Format::Custom(TextFns { show: freq_text, read: Some(read_freq) });
const Q: Format = Format::Custom(TextFns { show: q_text, read: None });

const fn band(n: usize, freq: &'static str, gain: &'static str, q: &'static str, on: &'static str, names: [&'static str; 4]) -> [ParamDef; 4] {
    let (_, f, q0, enabled) = BANDS[n];
    [
        ParamDef::new(freq, names[0], 20.0, 20000.0, f, Taper::Log, "Hz", FREQ).help("Band frequency of the repeat EQ."),
        ParamDef::new(gain, names[1], -30.0, 15.0, 0.0, LINEAR, "dB", Format::Db).help("Band gain of the repeat EQ."),
        ParamDef::new(q, names[2], 0.1, 20.0, q0, Taper::Log, "", Q).help("Band width (Q) of the repeat EQ."),
        ParamDef::choice(on, names[3], OFF_ON, enabled as u32).help("Switches the band on."),
    ]
}

const B0: [ParamDef; 4] = band(0, "eq0Freq", "eq0Gain", "eq0Q", "eq0Enabled", ["HP Freq", "HP Gain", "HP Q", "HP On"]);
const B1: [ParamDef; 4] = band(1, "eq1Freq", "eq1Gain", "eq1Q", "eq1Enabled", ["LoS Freq", "LoS Gain", "LoS Q", "LoS On"]);
const B2: [ParamDef; 4] = band(2, "eq2Freq", "eq2Gain", "eq2Q", "eq2Enabled", ["Pk1 Freq", "Pk1 Gain", "Pk1 Q", "Pk1 On"]);
const B3: [ParamDef; 4] = band(3, "eq3Freq", "eq3Gain", "eq3Q", "eq3Enabled", ["Pk2 Freq", "Pk2 Gain", "Pk2 Q", "Pk2 On"]);
const B4: [ParamDef; 4] = band(4, "eq4Freq", "eq4Gain", "eq4Q", "eq4Enabled", ["HiS Freq", "HiS Gain", "HiS Q", "HiS On"]);
const B5: [ParamDef; 4] = band(5, "eq5Freq", "eq5Gain", "eq5Q", "eq5Enabled", ["LP Freq", "LP Gain", "LP Q", "LP On"]);

/// "Time L" / "Note L" while the sides are unlinked.
fn label(def: &DeviceDef, d: &Device, p: &ParamDef) -> Option<&'static str> {
    let unlinked = def.param("link").is_some_and(|l| def.value(d, l) < 0.5);
    match (p.id, unlinked) {
        ("timeMs", true) => Some("Time L"),
        ("timeDivision", true) => Some("Note L"),
        _ => None,
    }
}

pub static DELAY: DeviceDef = DeviceDef {
    id: "delay",
    name: "Delay",
    kind: PluginKind::Effect,
    category: "Delay",
    purpose: "Repeats the sound in time with the song or freely, as clean digital, warm tape or murky bucket-brigade echoes, with an EQ on the repeats.",
    params: &[
        ParamDef::new("time", "BPM", 40.0, 250.0, 120.0, LINEAR, "BPM", Format::Plain(0)).step(1.0).help("Tempo used for synced times when the session reports none."),
        ParamDef::choice("timeDivision", "Time Division", DIVISIONS, 8).label("Note").reversed().help("Synced delay time of the left side (both sides while linked)."),
        ParamDef::new("feedback", "Feedback", 0.0, 100.0, 50.0, LINEAR, "%", Format::Percent).help("How much of each repeat feeds the next."),
        ParamDef::new("grit", "Grit", 0.0, 100.0, 0.0, LINEAR, "%", Format::Percent).help("Saturation and wear of the repeats, in the character of the mode."),
        ParamDef::new("modSpeed", "Mod Speed", 0.01, 10.0, 0.0692, Taper::Log, "Hz", Format::Custom(TextFns { show: rate_text, read: None })).label("Rate").help("Speed of the delay-time wobble."),
        ParamDef::new("modDepth", "Mod Depth", 0.0, 100.0, 0.0, LINEAR, "%", Format::Percent).help("Depth of the delay-time wobble."),
        ParamDef::new("stereoWidth", "Stereo Width", 0.0, 200.0, 100.0, LINEAR, "%", Format::Custom(TextFns { show: width_text, read: Some(read_width) }))
            .label("Width")
            .help("Mono to stereo; above 100% the repeats bounce between the sides (ping-pong)."),
        ParamDef::new("mix", "Mix", 0.0, 100.0, 50.0, LINEAR, "%", Format::Percent).help("Level of the repeats against the dry signal."),
        ParamDef::choice("mode", "Mode", MODES, 0).help("Character of the delay line: tape, clean digital or bucket-brigade."),
        B0[0], B0[1], B0[2], B0[3],
        B1[0], B1[1], B1[2], B1[3],
        B2[0], B2[1], B2[2], B2[3],
        B3[0], B3[1], B3[2], B3[3],
        B4[0], B4[1], B4[2], B4[3],
        B5[0], B5[1], B5[2], B5[3],
        ParamDef::choice("sync", "Sync", FREE_SYNC, 1).latch("Free", "Sync").help("Sync: times follow the session tempo; Free: times in milliseconds."),
        ParamDef::choice("dry", "Dry", OFF_ON, 1).latch("Dry Off", "Dry On").help("Pass the dry signal; turn off when the delay sits on a send."),
        ParamDef::new("timeMs", "Time", 1.0, 12000.0, 500.0, Taper::Log, "ms", MS).help("Free delay time of the left side (both sides while linked)."),
        ParamDef::choice("link", "Link", OFF_ON, 1).latch("Link Off", "Link On").help("Both sides use the left time; off gives each side its own time."),
        ParamDef::choice("timeDivisionR", "Time Division R", DIVISIONS, 8).label("Note R").reversed().help("Synced delay time of the right side while unlinked."),
        ParamDef::new("timeMsR", "Time R", 1.0, 12000.0, 500.0, Taper::Log, "ms", MS).help("Free delay time of the right side while unlinked."),
    ],
    layout: Layout {
        picker: &[Control::Toggle("link"), Control::Toggle("sync"), Control::Menu("mode"), Control::Gap, Control::Views, Control::Toggle("dry")],
        view: Some(View::Custom("delay")),
        clusters: &[
            &[Control::Knob("timeMs"), Control::Knob("timeDivision"), Control::Knob("timeMsR"), Control::Knob("timeDivisionR")],
            &[Control::Knob("feedback"), Control::Knob("grit")],
            &[Control::Knob("modSpeed"), Control::Knob("modDepth"), Control::Knob("stereoWidth")],
            &[Control::Knob("mix")],
        ],
        min_height: 199.0,
        shown_when: &[("timeMs", "sync", &[0]), ("timeMsR", "sync", &[0]), ("timeDivision", "sync", &[1]), ("timeDivisionR", "sync", &[1])],
        inert_when: &[("timeMsR", "link", &[1]), ("timeDivisionR", "link", &[1])],
        // Clusters [2,2,3,1]: 8 dials * 62 + 4 * 10 + 3 * 21 + 16.
        fixed_width: Some(615.0),
        views: &["Echoes", "EQ"],
        spread_dials: true,
        ..Layout::EMPTY
    },
    factory: &[
        FactoryPreset { name: "Clean Quarter Repeat", values: &[("timeDivision", 6.0), ("feedback", 32.0), ("mix", 28.0), ("mode", 1.0), ("eq0Freq", 170.0), ("eq0Enabled", 1.0)] },
        FactoryPreset { name: "Dotted Drive", values: &[("timeDivision", 8.0), ("feedback", 42.0), ("mix", 27.0), ("mode", 1.0), ("eq0Freq", 210.0), ("eq0Enabled", 1.0), ("eq5Freq", 9500.0), ("eq5Enabled", 1.0)] },
        FactoryPreset { name: "Eighth Pulse", values: &[("timeDivision", 9.0), ("feedback", 28.0), ("mix", 30.0), ("mode", 1.0), ("eq0Freq", 190.0), ("eq0Enabled", 1.0)] },
        FactoryPreset { name: "Triplet Skip", values: &[("timeDivision", 10.0), ("feedback", 33.0), ("mix", 32.0), ("mode", 1.0), ("eq0Freq", 240.0), ("eq0Enabled", 1.0)] },
        FactoryPreset { name: "Long Half Tail", values: &[("timeDivision", 3.0), ("feedback", 22.0), ("mix", 33.0), ("mode", 1.0), ("eq0Freq", 260.0), ("eq0Enabled", 1.0), ("eq5Freq", 7500.0), ("eq5Enabled", 1.0)] },
        FactoryPreset { name: "Rockabilly Slap", values: &[("feedback", 6.0), ("mix", 34.0), ("mode", 0.0), ("eq0Freq", 140.0), ("eq0Enabled", 1.0), ("sync", 0.0), ("timeMs", 95.0)] },
        FactoryPreset { name: "Short Clean Slap", values: &[("feedback", 4.0), ("mix", 26.0), ("mode", 1.0), ("eq0Freq", 230.0), ("eq0Enabled", 1.0), ("eq5Freq", 7500.0), ("eq5Enabled", 1.0), ("sync", 0.0), ("timeMs", 72.0)] },
        FactoryPreset { name: "Wide Double", values: &[("feedback", 0.0), ("modSpeed", 0.45), ("modDepth", 25.0), ("mix", 38.0), ("mode", 1.0), ("eq0Freq", 160.0), ("eq0Enabled", 1.0), ("sync", 0.0), ("timeMs", 21.0), ("link", 0.0), ("timeMsR", 33.0)] },
        FactoryPreset { name: "Precedence Spread", values: &[("feedback", 0.0), ("mix", 100.0), ("mode", 1.0), ("sync", 0.0), ("dry", 0.0), ("timeMs", 2.0), ("link", 0.0), ("timeMsR", 17.0)] },
        FactoryPreset { name: "Bounce Eighths", values: &[("timeDivision", 9.0), ("feedback", 48.0), ("stereoWidth", 200.0), ("mix", 40.0), ("mode", 1.0), ("eq0Freq", 220.0), ("eq0Enabled", 1.0)] },
        FactoryPreset { name: "Bounce Dotted", values: &[("timeDivision", 8.0), ("feedback", 44.0), ("stereoWidth", 185.0), ("mix", 38.0), ("mode", 1.0), ("eq0Freq", 230.0), ("eq0Enabled", 1.0), ("eq5Freq", 9000.0), ("eq5Enabled", 1.0)] },
        FactoryPreset { name: "Bounce Quarters", values: &[("timeDivision", 6.0), ("feedback", 38.0), ("stereoWidth", 170.0), ("mix", 36.0), ("mode", 1.0), ("eq0Freq", 190.0), ("eq0Enabled", 1.0)] },
        FactoryPreset { name: "Offset Quarter / Triplet", values: &[("timeDivision", 6.0), ("feedback", 30.0), ("mix", 30.0), ("mode", 1.0), ("eq0Freq", 210.0), ("eq0Enabled", 1.0), ("link", 0.0), ("timeDivisionR", 7.0)] },
        FactoryPreset { name: "Offset Eighth / Dotted", values: &[("timeDivision", 9.0), ("feedback", 36.0), ("mix", 34.0), ("mode", 1.0), ("eq0Freq", 230.0), ("eq0Enabled", 1.0), ("link", 0.0), ("timeDivisionR", 8.0)] },
        FactoryPreset { name: "Warm Tape Repeats", values: &[("timeDivision", 6.0), ("feedback", 46.0), ("grit", 25.0), ("modSpeed", 0.7), ("modDepth", 15.0), ("mix", 33.0), ("mode", 0.0), ("eq0Freq", 160.0), ("eq0Enabled", 1.0)] },
        FactoryPreset { name: "Stretched Tape", values: &[("timeDivision", 8.0), ("feedback", 52.0), ("grit", 50.0), ("modSpeed", 1.1), ("modDepth", 55.0), ("mix", 40.0), ("mode", 0.0), ("eq0Freq", 240.0), ("eq0Enabled", 1.0), ("eq5Freq", 3800.0), ("eq5Enabled", 1.0)] },
        FactoryPreset { name: "Analog Chip Echo", values: &[("timeDivision", 9.0), ("feedback", 42.0), ("grit", 28.0), ("modSpeed", 0.45), ("modDepth", 22.0), ("mix", 35.0), ("mode", 2.0), ("eq0Freq", 180.0), ("eq0Enabled", 1.0), ("eq5Freq", 4200.0), ("eq5Enabled", 1.0)] },
        FactoryPreset { name: "Broken Radio", values: &[("timeDivision", 10.0), ("feedback", 48.0), ("grit", 85.0), ("modSpeed", 2.2), ("modDepth", 40.0), ("mix", 48.0), ("mode", 2.0), ("eq0Freq", 500.0), ("eq0Enabled", 1.0), ("eq5Freq", 2300.0), ("eq5Enabled", 1.0)] },
        FactoryPreset { name: "Dub Space", values: &[("timeDivision", 8.0), ("feedback", 70.0), ("modDepth", 12.0), ("stereoWidth", 150.0), ("mix", 50.0), ("mode", 0.0), ("eq0Freq", 320.0), ("eq0Enabled", 1.0), ("eq5Freq", 2200.0), ("eq5Enabled", 1.0)] },
        FactoryPreset { name: "Runaway Dub", values: &[("timeDivision", 6.0), ("feedback", 88.0), ("grit", 18.0), ("mix", 30.0), ("mode", 0.0), ("eq0Freq", 420.0), ("eq0Enabled", 1.0), ("eq5Freq", 1600.0), ("eq5Enabled", 1.0)] },
        FactoryPreset { name: "Muffled Repeats", values: &[("timeDivision", 6.0), ("feedback", 58.0), ("mix", 24.0), ("mode", 1.0), ("eq0Freq", 220.0), ("eq0Enabled", 1.0), ("eq4Freq", 2800.0), ("eq4Gain", -10.0), ("eq5Freq", 3600.0), ("eq5Enabled", 1.0)] },
        FactoryPreset { name: "Send: Quarter", values: &[("timeDivision", 6.0), ("feedback", 38.0), ("mix", 100.0), ("mode", 1.0), ("eq0Freq", 210.0), ("eq0Enabled", 1.0), ("dry", 0.0)] },
        FactoryPreset { name: "Send: Dotted", values: &[("timeDivision", 8.0), ("feedback", 42.0), ("mix", 100.0), ("mode", 1.0), ("eq0Freq", 210.0), ("eq0Enabled", 1.0), ("eq5Freq", 9500.0), ("eq5Enabled", 1.0), ("dry", 0.0)] },
        FactoryPreset { name: "Send: Bounce", values: &[("timeDivision", 9.0), ("feedback", 44.0), ("stereoWidth", 190.0), ("mix", 100.0), ("mode", 1.0), ("eq0Freq", 210.0), ("eq0Enabled", 1.0), ("dry", 0.0)] },
    ],
    sidechain: false,
    hooks: Hooks { label: Some(label), text: None },
};
