//! Trance Gate (reference: the reference app's built-in "Trance Gate"): a tempo-synced
//! step gate with eight stored patterns of up to 32 steps, shaped by an
//! attack / decay / sustain / release envelope, with swing and dry/wet mix.
//! Parameter ids, ranges, defaults, tapers and the panel arrangement are taken
//! from the reference as facts; names of the factory presets and their
//! values are our own.
//!
//! Pattern storage: pattern n keeps its length in `p{n}_steps` and its step
//! bits in `p{n}_lo` (steps 1-16, bit i = step i + 1) and `p{n}_hi` (steps
//! 17-32). The ring on the panel writes them directly; they are not
//! automatable.

use super::*;
use crate::device::PluginKind;

/// Step divisions, slowest first (plain value = index).
pub const RATES: &[&str] = &["1/4.", "1/4", "1/4T", "1/8.", "1/8", "1/8T", "1/16.", "1/16", "1/16T", "1/32", "1/64"];
/// Quarter notes per step for each entry of [`RATES`].
pub const RATE_QUARTERS: [f64; 11] = [1.5, 1.0, 2.0 / 3.0, 0.75, 0.5, 1.0 / 3.0, 0.375, 0.25, 1.0 / 6.0, 0.125, 0.0625];
const PATTERNS: &[&str] = &["1", "2", "3", "4", "5", "6", "7", "8"];

pub const MAX_STEPS: u32 = 32;
pub const PATTERN_COUNT: usize = 8;

macro_rules! trance_gate_params {
    ($( ($n:literal, $steps:expr, $lo:expr, $hi:expr) ),* $(,)?) => {
        [
            ParamDef::choice("pattern", "Pattern", PATTERNS, 0).help("Which of the eight stored step patterns plays."),
            ParamDef::choice("rate", "Rate", RATES, 7).help("Length of one step as a note value."),
            ParamDef::new("attack", "Attack", 0.5, 250.0, 3.0, Taper::Log, "ms", Format::Time).help("Fade-in time at the start of each step that is on."),
            ParamDef::new("decay", "Decay", 1.0, 1000.0, 160.0, Taper::Log, "ms", Format::Time).help("Time to fall from full level to the sustain level."),
            ParamDef::new("sustain", "Sustain", 0.0, 100.0, 60.0, LINEAR, "%", Format::Percent).help("Level held after the decay while a step stays on."),
            ParamDef::new("release", "Release", 1.0, 500.0, 40.0, Taper::Log, "ms", Format::Time).help("Fade-out time after a step ends."),
            ParamDef::new("swing", "Swing", 0.0, 100.0, 0.0, LINEAR, "%", Format::Percent).help("Delays every second step by up to half a step."),
            ParamDef::new("mix", "Mix", 0.0, 100.0, 100.0, LINEAR, "%", Format::Percent).help("Blend of gated and dry signal."),
            $(
                ParamDef::new(concat!("p", $n, "_steps"), concat!("Pattern ", $n, " Length"), 1.0, 32.0, $steps, LINEAR, "steps", Format::Plain(0))
                    .label("Steps").step(1.0).not_automatable().help("Number of steps before the pattern repeats."),
                ParamDef::new(concat!("p", $n, "_lo"), concat!("Pattern ", $n, " Steps 1-16"), 0.0, 65535.0, $lo, LINEAR, "", Format::Plain(0))
                    .step(1.0).not_automatable().help("On/off bits of steps 1 to 16."),
                ParamDef::new(concat!("p", $n, "_hi"), concat!("Pattern ", $n, " Steps 17-32"), 0.0, 65535.0, $hi, LINEAR, "", Format::Plain(0))
                    .step(1.0).not_automatable().help("On/off bits of steps 17 to 32."),
            )*
        ]
    };
}

static PARAMS: [ParamDef; 32] = trance_gate_params![
    (1, 16.0, 23387.0, 0.0),
    (2, 16.0, 21845.0, 0.0),
    (3, 16.0, 46967.0, 0.0),
    (4, 16.0, 19983.0, 0.0),
    (5, 12.0, 2779.0, 0.0),
    (6, 32.0, 44397.0, 30125.0),
    (7, 16.0, 18761.0, 0.0),
    (8, 16.0, 56669.0, 0.0),
];

/// Ids of pattern n's (0-based) length, low and high bit parameters.
pub fn pattern_ids(n: usize) -> (&'static str, &'static str, &'static str) {
    let i = 8 + 3 * n.min(PATTERN_COUNT - 1);
    (PARAMS[i].id, PARAMS[i + 1].id, PARAMS[i + 2].id)
}

/// The 32-bit step mask of a pattern from its two 16-bit halves.
pub fn mask(lo: f32, hi: f32) -> u32 {
    (lo.clamp(0.0, 65535.0) as u32) | ((hi.clamp(0.0, 65535.0) as u32) << 16)
}

/// Text form of a pattern: one character per step, x = on, . = off.
pub fn steps_text(mask: u32, steps: u32) -> String {
    (0..steps.min(MAX_STEPS)).map(|i| if mask >> i & 1 == 1 { 'x' } else { '.' }).collect()
}

/// Parse a pattern string (x or X on, anything else off): (mask, steps).
pub fn parse_steps(text: &str) -> Option<(u32, u32)> {
    let t: Vec<char> = text.chars().filter(|c| !c.is_whitespace()).collect();
    if t.is_empty() || t.len() > MAX_STEPS as usize {
        return None;
    }
    let mask = t.iter().enumerate().fold(0u32, |m, (i, c)| if matches!(c, 'x' | 'X') { m | 1 << i } else { m });
    Some((mask, t.len() as u32))
}

/// Pattern bits as the (lo, hi) parameter values.
pub const fn halves(mask: u32) -> (f32, f32) {
    ((mask & 0xffff) as f32, (mask >> 16) as f32)
}

/// Bits of a pattern string at compile time (x on).
const fn bits(s: &str) -> u32 {
    let b = s.as_bytes();
    let mut m = 0u32;
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'x' {
            m |= 1 << i;
        }
        i += 1;
    }
    m
}
const fn lo(s: &str) -> f32 {
    halves(bits(s)).0
}
const fn hi(s: &str) -> f32 {
    halves(bits(s)).1
}

pub static TRANCE_GATE: DeviceDef = DeviceDef {
    id: "trance-gate",
    name: "Trance Gate",
    kind: PluginKind::Effect,
    category: "Modulation",
    purpose: "Chops a sound into rhythmic, tempo-synced pulses from a step pattern, with an envelope that shapes each pulse.",
    params: &PARAMS,
    layout: Layout { picker: &[], view: Some(View::Custom("trance-gate")), sliders: &[], clusters: &[], faces: &[], min_height: 220.0, shown_when: &[] },
    factory: &[
        FactoryPreset {
            name: "Pulse Grid",
            values: &[("rate", 7.0), ("attack", 1.5), ("decay", 70.0), ("sustain", 55.0), ("release", 9.0), ("p1_steps", 16.0), ("p1_lo", lo("xxx.xxx.xxx.xx.x")), ("p1_hi", 0.0)],
        },
        FactoryPreset {
            name: "Lifted Sixteenths",
            values: &[("rate", 7.0), ("attack", 2.5), ("decay", 110.0), ("sustain", 72.0), ("release", 14.0), ("p1_steps", 16.0), ("p1_lo", lo("x.xxx.x.xx.xxx.x")), ("p1_hi", 0.0)],
        },
        FactoryPreset {
            name: "Thirty-Two Steps",
            values: &[
                ("rate", 9.0),
                ("attack", 0.8),
                ("decay", 38.0),
                ("sustain", 64.0),
                ("release", 5.0),
                ("p1_steps", 32.0),
                ("p1_lo", lo("xx.xx.x.xx.xx.x.xx.x.xx.xx.x.xxx")),
                ("p1_hi", hi("xx.xx.x.xx.xx.x.xx.x.xx.xx.x.xxx")),
            ],
        },
        FactoryPreset {
            name: "Bass Shuffle",
            values: &[("rate", 7.0), ("attack", 1.0), ("decay", 55.0), ("sustain", 35.0), ("release", 7.0), ("swing", 45.0), ("p1_steps", 16.0), ("p1_lo", lo(".xx.xx.x.xx.xx.x")), ("p1_hi", 0.0)],
        },
        FactoryPreset {
            name: "Pad Breath",
            values: &[("rate", 4.0), ("attack", 45.0), ("decay", 260.0), ("sustain", 88.0), ("release", 180.0), ("mix", 78.0), ("p1_steps", 8.0), ("p1_lo", lo("xx.xx.x.")), ("p1_hi", 0.0)],
        },
        FactoryPreset {
            name: "Quarter Bloom",
            values: &[("rate", 1.0), ("attack", 160.0), ("decay", 140.0), ("sustain", 95.0), ("release", 90.0), ("mix", 85.0), ("p1_steps", 4.0), ("p1_lo", lo("xxxx")), ("p1_hi", 0.0)],
        },
        FactoryPreset {
            name: "Upbeat Chop",
            values: &[("rate", 7.0), ("attack", 3.0), ("decay", 90.0), ("sustain", 50.0), ("release", 18.0), ("p1_steps", 8.0), ("p1_lo", lo(".x.x.x.x")), ("p1_hi", 0.0)],
        },
        FactoryPreset {
            name: "Ducked Pump",
            values: &[("rate", 7.0), ("attack", 85.0), ("decay", 2.0), ("sustain", 100.0), ("release", 6.0), ("p1_steps", 4.0), ("p1_lo", lo("xxxx")), ("p1_hi", 0.0)],
        },
        FactoryPreset {
            name: "Triplet Rain",
            values: &[("rate", 8.0), ("attack", 1.0), ("decay", 45.0), ("sustain", 60.0), ("release", 6.0), ("p1_steps", 12.0), ("p1_lo", lo("xx.xxx.x.xx.")), ("p1_hi", 0.0)],
        },
        FactoryPreset {
            name: "Dotted Steps",
            values: &[("rate", 6.0), ("attack", 2.0), ("decay", 100.0), ("sustain", 70.0), ("release", 12.0), ("p1_steps", 16.0), ("p1_lo", lo("x.xx.xx.x.xx.xx.")), ("p1_hi", 0.0)],
        },
        FactoryPreset {
            name: "Riser Fill",
            values: &[
                ("rate", 7.0),
                ("attack", 1.0),
                ("decay", 80.0),
                ("sustain", 65.0),
                ("release", 10.0),
                ("p1_steps", 32.0),
                ("p1_lo", lo("x...x...x.x.x.x.xxxxxxxxxxxxxxxx")),
                ("p1_hi", hi("x...x...x.x.x.x.xxxxxxxxxxxxxxxx")),
            ],
        },
        FactoryPreset {
            name: "Stutter Burst",
            values: &[
                ("rate", 10.0),
                ("attack", 0.5),
                ("decay", 15.0),
                ("sustain", 75.0),
                ("release", 2.0),
                ("p1_steps", 32.0),
                ("p1_lo", lo("xxxx..xx.x.xxx..x.x.xxxx.xx..x.x")),
                ("p1_hi", hi("xxxx..xx.x.xxx..x.x.xxxx.xx..x.x")),
            ],
        },
        FactoryPreset {
            name: "Telegraph",
            values: &[
                ("rate", 9.0),
                ("attack", 1.0),
                ("decay", 900.0),
                ("sustain", 100.0),
                ("release", 4.0),
                ("p1_steps", 32.0),
                ("p1_lo", lo("xxx.x.xxx...x.x.x...xxx.xxx.x...")),
                ("p1_hi", hi("xxx.x.xxx...x.x.x...xxx.xxx.x...")),
            ],
        },
        FactoryPreset {
            name: "Swing House",
            values: &[("rate", 7.0), ("attack", 1.5), ("decay", 75.0), ("sustain", 60.0), ("release", 9.0), ("swing", 55.0), ("p1_steps", 16.0), ("p1_lo", lo("x.x.x.x.x.x.x.x.")), ("p1_hi", 0.0)],
        },
    ],
    sidechain: false,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_and_ids() {
        let d = &TRANCE_GATE;
        assert_eq!(d.params.len(), 32);
        assert_eq!(pattern_ids(0), ("p1_steps", "p1_lo", "p1_hi"));
        assert_eq!(pattern_ids(7), ("p8_steps", "p8_lo", "p8_hi"));
        assert_eq!(d.index_of("p8_hi"), Some(31));
        assert!(d.params[8..].iter().all(|p| !p.automatable));
        assert_eq!(d.param("rate").unwrap().format(7.0), "1/16");
        assert_eq!(d.param("p5_steps").unwrap().format(12.0), "12 steps");
        assert_eq!(d.param("attack").unwrap().format(3.0), "3 ms");
        assert_eq!(d.param("sustain").unwrap().format(60.0), "60%");
    }

    #[test]
    fn pattern_text_round_trips() {
        let m = mask(23387.0, 0.0);
        assert_eq!(steps_text(m, 16), "xx.xx.x.xx.xx.x.");
        assert_eq!(parse_steps("xx.xx.x.xx.xx.x."), Some((m, 16)));
        assert_eq!(parse_steps(""), None);
        assert_eq!(parse_steps(&"x".repeat(33)), None);
        let (l, h) = halves(0xdead_beef);
        assert_eq!(mask(l, h), 0xdead_beef);
        assert_eq!(bits("x...x"), 0b10001);
    }
}
