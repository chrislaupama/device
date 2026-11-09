//! Grains (reference: the reference app's built-in "Grains"): a live granular effect.
//! Parameter ids, ranges, defaults, tapers and the panel arrangement are
//! taken from the reference as facts; factory preset names and values are our own.

use super::*;
use crate::device::PluginKind;

pub const MODES: &[&str] = &["Cloud", "Rhythm"];
pub const POSITION_NOTES: &[&str] = &["Now", "1/32", "1/16", "1/8", "1/8.", "1/4", "1/2", "1/1", "2/1", "4/1"];
/// Position Note lengths in quarter notes.
pub const POSITION_BEATS: [f32; 10] = [0.0, 0.125, 0.25, 0.5, 0.75, 1.0, 2.0, 4.0, 8.0, 16.0];
pub const RATES: &[&str] = &["1/2", "1/4.", "1/4", "1/4T", "1/8.", "1/8", "1/8T", "1/16.", "1/16", "1/16T", "1/32", "1/64"];
/// Rate step lengths in quarter notes.
pub const RATE_BEATS: [f32; 12] = [2.0, 1.5, 1.0, 2.0 / 3.0, 0.75, 0.5, 1.0 / 3.0, 0.375, 0.25, 1.0 / 6.0, 0.125, 0.0625];
pub const SCALES: &[&str] = &["Free", "Chromatic", "Octaves", "Oct + 5th", "Key", "Key Chord", "Pentatonic"];
pub const CHARACTERS: &[&str] = &["Clean", "Tape", "Sampler"];
const OFF_ON: &[&str] = &["Off", "On"];
pub const RHYTHM: u32 = 1;

/// "250 ms", "4.5 ms", "1.20 s".
pub fn ms_text(v: f32) -> String {
    if v >= 1000.0 {
        format!("{:.2} s", v / 1000.0)
    } else if v < 10.0 && v.fract() != 0.0 {
        format!("{v:.1} ms")
    } else {
        format!("{v:.0} ms")
    }
}

fn read_ms(t: &str) -> Option<f32> {
    let t = t.trim();
    if let Some(ms) = t.strip_suffix("ms") {
        return ms.trim().parse().ok();
    }
    t.strip_suffix('s').and_then(|s| s.trim().parse::<f32>().ok()).map(|v| v * 1000.0)
}

/// "24 /s", "0.5 /s".
pub fn density_text(v: f32) -> String {
    if v < 10.0 && (v * 10.0).round() % 10.0 != 0.0 { format!("{v:.1} /s") } else { format!("{v:.0} /s") }
}

/// "+0 st", "−12 st", "+0.3 st".
pub fn pitch_text(v: f32) -> String {
    let r = (v * 10.0).round() / 10.0;
    let sign = if r < 0.0 { "\u{2212}" } else { "+" };
    if r.fract() == 0.0 { format!("{sign}{:.0} st", r.abs()) } else { format!("{sign}{:.1} st", r.abs()) }
}

/// "0 st", "12 st", "0.3 st".
pub fn spread_text(v: f32) -> String {
    let r = (v * 10.0).round() / 10.0;
    if r.fract() == 0.0 { format!("{r:.0} st") } else { format!("{r:.1} st") }
}

const MS: Format = Format::Custom(TextFn(ms_text));

pub(super) static BEHAVIOR: Behavior = Behavior { read: |_, _, p, t| if p.format == MS { read_ms(t) } else { None }, ..Behavior::NONE };
const CLOUD: &[u32] = &[0];

pub static GRAINS: DeviceDef = DeviceDef {
    id: "grains",
    name: "Grains",
    kind: PluginKind::Effect,
    category: "Delay",
    purpose: "Chops the incoming sound into grains and replays them: clouds, frozen pads, shimmer, stutters and reshuffled chops, with random pitch kept in key.",
    params: &[
        ParamDef::choice("mode", "Mode", MODES, 0).help("Cloud scatters grains freely; Rhythm fires them on the beat."),
        ParamDef::choice("freeze", "Freeze", OFF_ON, 0).latch("Freeze", "Frozen").help("Hold what the grains are playing, as a pad. In Rhythm it lands on the next step."),
        ParamDef::new("position", "Position", 0.0, 8000.0, 250.0, Taper::Linear { skew: 0.3 }, "ms", MS).help("How far back in time grains start reading (Cloud)."),
        ParamDef::choice("position_note", "Position Note", POSITION_NOTES, 3).help("How far back grains start reading, in note values (Rhythm)."),
        ParamDef::new("size", "Size", 5.0, 2000.0, 120.0, Taper::Linear { skew: 0.3 }, "ms", MS).help("Grain length (Cloud)."),
        ParamDef::new("gate", "Gate", 5.0, 400.0, 100.0, Taper::Linear { skew: 0.5 }, "%", Format::Percent).help("Grain length as a share of the step (Rhythm)."),
        ParamDef::new("density", "Density", 0.5, 200.0, 24.0, Taper::Linear { skew: 0.3 }, "/s", Format::Custom(TextFn(density_text))).help("Grains per second (Cloud)."),
        ParamDef::choice("rate", "Rate", RATES, 8).help("Step length (Rhythm)."),
        ParamDef::new("chance", "Chance", 0.0, 100.0, 100.0, LINEAR, "%", Format::Percent).help("Probability that a step fires a grain (Rhythm)."),
        ParamDef::new("spray", "Spray", 0.0, 2000.0, 40.0, Taper::Linear { skew: 0.35 }, "ms", MS).help("Random extra reach back in time for each grain."),
        ParamDef::new("pitch", "Pitch", -24.0, 24.0, 0.0, LINEAR, "st", Format::Custom(TextFn(pitch_text))).help("Transposition of every grain."),
        ParamDef::new("spread", "Spread", 0.0, 24.0, 0.0, Taper::Linear { skew: 0.6 }, "st", Format::Custom(TextFn(spread_text))).help("Random pitch range per grain, kept on the Scale."),
        ParamDef::choice("scale", "Scale", SCALES, 4).help("Which pitches Spread lands on."),
        ParamDef::new("reverse", "Reverse", 0.0, 100.0, 0.0, LINEAR, "%", Format::Percent).help("Share of grains played backwards."),
        ParamDef::new("shape", "Shape", 0.0, 100.0, 50.0, LINEAR, "%", Format::Percent).help("Grain envelope: hard and clicky to soft and smooth."),
        ParamDef::new("width", "Width", 0.0, 100.0, 50.0, LINEAR, "%", Format::Percent).help("Random stereo placement of grains."),
        ParamDef::new("drift", "Drift", 0.0, 100.0, 0.0, LINEAR, "%", Format::Percent).help("Slow wandering of position and pitch."),
        ParamDef::new("feedback", "Feedback", 0.0, 95.0, 0.0, LINEAR, "%", Format::Percent).help("Grains fed back into the capture buffer."),
        ParamDef::new("blur", "Blur", 0.0, 100.0, 0.0, LINEAR, "%", Format::Percent).help("Diffuses the grains into a wash."),
        ParamDef::new("tone", "Tone", -100.0, 100.0, 0.0, LINEAR, "%", Format::Percent).help("Darker to brighter tilt of the grains."),
        ParamDef::new("space", "Space", 0.0, 100.0, 0.0, LINEAR, "%", Format::Percent).help("Reverb around the grains."),
        ParamDef::choice("character", "Character", CHARACTERS, 0).help("Clean, worn tape, or a gritty 12-bit sampler."),
        ParamDef::new("mix", "Mix", 0.0, 100.0, 50.0, LINEAR, "%", Format::Percent).help("Level of the grains against the dry signal."),
        ParamDef::choice("mix_lock", "Mix Lock", OFF_ON, 0).locks(&["mix"]).help("Keep Mix when loading presets."),
        ParamDef::new("output", "Output", -24.0, 12.0, 0.0, LINEAR, "dB", Format::Db).help("Output level of the grains."),
    ],
    layout: Layout {
        picker: &[Control::Menu("mode"), Control::Dropdown("scale"), Control::Dropdown("character"), Control::Gap, Control::Toggle("freeze")],
        view: Some(View::Custom("grains")),
        sliders: &[Control::Slider("mix")],
        // Cloud [size, density] [shape, pitch, spread] [feedback, space];
        // Rhythm [rate, gate, chance] [shape, pitch] [feedback, space].
        clusters: &[
            &[Control::Knob("size"), Control::Knob("rate"), Control::Knob("density"), Control::Knob("gate"), Control::Knob("chance")],
            &[Control::Knob("shape"), Control::Knob("pitch"), Control::Knob("spread")],
            &[Control::Knob("feedback"), Control::Knob("space")],
        ],
        min_height: 220.0,
        shown_when: &[
            ("size", "mode", CLOUD),
            ("density", "mode", CLOUD),
            ("spread", "mode", CLOUD),
            ("rate", "mode", &[RHYTHM]),
            ("gate", "mode", &[RHYTHM]),
            ("chance", "mode", &[RHYTHM]),
        ],
        fixed_width: 532.0,
        spread_dials: true,
        ..Layout::EMPTY
    },
    factory: &[
        FactoryPreset { name: "Crystal Mist", values: &[("size", 160.0), ("density", 34.0), ("spray", 140.0), ("spread", 12.0), ("width", 85.0), ("blur", 35.0), ("space", 30.0), ("mix", 55.0)] },
        FactoryPreset { name: "Soft Veil", values: &[("size", 300.0), ("density", 40.0), ("spray", 260.0), ("shape", 75.0), ("width", 90.0), ("drift", 20.0), ("blur", 55.0), ("tone", -20.0), ("space", 35.0), ("mix", 65.0), ("output", 1.0)] },
        FactoryPreset { name: "Hailstones", values: &[("size", 20.0), ("density", 140.0), ("spray", 700.0), ("spread", 12.0), ("scale", 6.0), ("shape", 12.0), ("width", 100.0), ("space", 15.0), ("mix", 50.0), ("output", 2.0)] },
        FactoryPreset { name: "Floating Specks", values: &[("size", 35.0), ("density", 14.0), ("spray", 1000.0), ("spread", 5.0), ("reverse", 25.0), ("shape", 15.0), ("width", 100.0), ("tone", 25.0), ("space", 40.0), ("output", 1.5)] },
        FactoryPreset { name: "Amber Cloud", values: &[("size", 550.0), ("density", 18.0), ("spray", 450.0), ("shape", 92.0), ("drift", 45.0), ("blur", 75.0), ("tone", -50.0), ("space", 45.0), ("mix", 75.0), ("output", 1.5)] },
        FactoryPreset { name: "Octave Sparkle", values: &[("size", 70.0), ("density", 70.0), ("spray", 220.0), ("pitch", 12.0), ("spread", 12.0), ("scale", 2.0), ("shape", 35.0), ("width", 100.0), ("tone", 20.0), ("space", 25.0), ("mix", 40.0)] },
        FactoryPreset { name: "Half-Speed Dream", values: &[("size", 700.0), ("density", 8.0), ("spray", 120.0), ("pitch", -12.0), ("scale", 2.0), ("shape", 85.0), ("blur", 35.0), ("mix", 65.0)] },
        FactoryPreset { name: "Granular Dust", values: &[("size", 10.0), ("density", 170.0), ("spray", 80.0), ("width", 90.0), ("mix", 55.0), ("output", 2.5)] },
        FactoryPreset { name: "Held Pad", values: &[("size", 480.0), ("density", 16.0), ("spray", 380.0), ("shape", 90.0), ("width", 85.0), ("drift", 35.0), ("blur", 55.0), ("space", 50.0), ("mix", 100.0), ("output", 3.5)] },
        FactoryPreset { name: "Chord Cloud", values: &[("size", 320.0), ("density", 28.0), ("spray", 550.0), ("spread", 12.0), ("scale", 5.0), ("shape", 80.0), ("width", 100.0), ("blur", 65.0), ("space", 45.0), ("mix", 100.0), ("output", 2.5)] },
        FactoryPreset { name: "Ice Shelf", values: &[("size", 1000.0), ("density", 9.0), ("spray", 1400.0), ("shape", 96.0), ("drift", 55.0), ("blur", 85.0), ("tone", -30.0), ("space", 65.0), ("mix", 100.0), ("output", 2.0)] },
        FactoryPreset { name: "Harmonic Organ", values: &[("size", 280.0), ("density", 38.0), ("spray", 60.0), ("spread", 12.0), ("scale", 2.0), ("shape", 100.0), ("blur", 35.0), ("mix", 100.0), ("output", 1.5)] },
        FactoryPreset { name: "Fifth Halo", values: &[("size", 400.0), ("density", 22.0), ("spray", 650.0), ("pitch", 12.0), ("spread", 12.0), ("scale", 3.0), ("feedback", 35.0), ("blur", 65.0), ("tone", 10.0), ("space", 50.0), ("mix", 85.0), ("output", 2.0)] },
        FactoryPreset { name: "Worn Tape Pad", values: &[("size", 360.0), ("density", 20.0), ("spray", 350.0), ("shape", 82.0), ("drift", 45.0), ("blur", 35.0), ("tone", -35.0), ("character", 1.0), ("mix", 100.0), ("output", 1.0)] },
        FactoryPreset { name: "Low Bloom", values: &[("size", 650.0), ("density", 11.0), ("spray", 280.0), ("pitch", -12.0), ("scale", 2.0), ("shape", 88.0), ("blur", 45.0), ("tone", -45.0), ("mix", 85.0), ("output", 2.0)] },
        FactoryPreset { name: "Repeat Last Beat", values: &[("mode", 1.0), ("position_note", 5.0), ("rate", 5.0), ("spray", 0.0), ("shape", 75.0), ("mix", 100.0)] },
        FactoryPreset { name: "Machine Gun 32", values: &[("mode", 1.0), ("position_note", 0.0), ("gate", 55.0), ("rate", 10.0), ("spray", 0.0), ("shape", 25.0), ("mix", 100.0), ("output", 3.5)] },
        FactoryPreset { name: "Shuffle Triplets", values: &[("mode", 1.0), ("gate", 75.0), ("rate", 6.0), ("spray", 0.0), ("shape", 55.0), ("width", 55.0), ("mix", 65.0), ("output", -1.5)] },
        FactoryPreset { name: "Low Chop", values: &[("mode", 1.0), ("position_note", 5.0), ("rate", 5.0), ("pitch", -12.0), ("scale", 2.0), ("shape", 80.0), ("mix", 100.0)] },
        FactoryPreset { name: "Phantom Hits", values: &[("mode", 1.0), ("gate", 40.0), ("chance", 50.0), ("spray", 300.0), ("reverse", 25.0), ("shape", 45.0), ("width", 75.0), ("output", -1.0)] },
        FactoryPreset { name: "Dotted Grain Echo", values: &[("mode", 1.0), ("position_note", 4.0), ("rate", 4.0), ("shape", 65.0), ("width", 85.0), ("feedback", 45.0), ("blur", 20.0), ("mix", 40.0)] },
        FactoryPreset { name: "Scatter Grid", values: &[("mode", 1.0), ("gate", 45.0), ("rate", 10.0), ("chance", 70.0), ("spray", 600.0), ("pitch", 12.0), ("scale", 2.0), ("reverse", 35.0), ("shape", 25.0), ("mix", 65.0), ("output", 3.0)] },
        FactoryPreset { name: "Rolling Hats", values: &[("mode", 1.0), ("position_note", 2.0), ("gate", 65.0), ("rate", 10.0), ("chance", 65.0), ("spray", 200.0), ("shape", 18.0), ("mix", 50.0), ("output", -1.5)] },
        FactoryPreset { name: "Beat Reorder", values: &[("mode", 1.0), ("position_note", 5.0), ("gate", 98.0), ("spray", 1700.0), ("shape", 88.0), ("width", 35.0), ("mix", 100.0)] },
        FactoryPreset { name: "Crate Chops", values: &[("mode", 1.0), ("position_note", 5.0), ("rate", 5.0), ("chance", 85.0), ("spray", 450.0), ("pitch", -12.0), ("scale", 2.0), ("shape", 78.0), ("character", 2.0), ("mix", 100.0)] },
        FactoryPreset { name: "Bar Scramble", values: &[("mode", 1.0), ("position_note", 7.0), ("rate", 2.0), ("spray", 1900.0), ("shape", 88.0), ("mix", 100.0), ("output", 2.0)] },
        FactoryPreset { name: "Backwards Steps", values: &[("mode", 1.0), ("position_note", 5.0), ("rate", 5.0), ("spray", 900.0), ("reverse", 60.0), ("shape", 82.0), ("mix", 100.0)] },
        FactoryPreset { name: "Rising Feedback Shine", values: &[("size", 280.0), ("density", 22.0), ("spray", 220.0), ("spread", 12.0), ("scale", 2.0), ("feedback", 60.0), ("blur", 55.0), ("tone", 15.0), ("space", 55.0), ("mix", 50.0)] },
        FactoryPreset { name: "Key Harmonies", values: &[("density", 36.0), ("spread", 7.0), ("scale", 5.0), ("width", 85.0), ("output", -0.5)] },
        FactoryPreset { name: "Pentatonic Rain", values: &[("size", 80.0), ("density", 45.0), ("spray", 320.0), ("spread", 24.0), ("scale", 6.0), ("shape", 32.0), ("width", 100.0), ("space", 28.0), ("mix", 42.0)] },
        FactoryPreset { name: "Detuned Shimmer", values: &[("size", 220.0), ("density", 55.0), ("spray", 25.0), ("spread", 0.4), ("scale", 0.0), ("width", 100.0), ("blur", 25.0), ("mix", 55.0), ("output", 1.0)] },
        FactoryPreset { name: "Ghost Voices", values: &[("size", 200.0), ("density", 18.0), ("spray", 800.0), ("spread", 12.0), ("reverse", 45.0), ("width", 100.0), ("blur", 45.0), ("space", 40.0), ("mix", 50.0)] },
        FactoryPreset { name: "Cassette Haze", values: &[("size", 240.0), ("density", 24.0), ("spray", 220.0), ("drift", 40.0), ("blur", 30.0), ("tone", -45.0), ("space", 25.0), ("character", 1.0), ("mix", 65.0), ("output", 1.0)] },
        FactoryPreset { name: "Twelve-Bit Fog", values: &[("size", 120.0), ("density", 28.0), ("spray", 180.0), ("tone", -25.0), ("character", 2.0), ("mix", 65.0)] },
        FactoryPreset { name: "Reverse Bloom", values: &[("size", 900.0), ("density", 5.0), ("spray", 450.0), ("reverse", 100.0), ("shape", 80.0), ("blur", 40.0), ("space", 50.0), ("mix", 70.0), ("output", 3.0)] },
        FactoryPreset { name: "Swirling Feedback", values: &[("size", 260.0), ("density", 28.0), ("spray", 260.0), ("width", 85.0), ("feedback", 70.0), ("blur", 45.0), ("space", 35.0), ("mix", 50.0), ("output", 2.0)] },
        FactoryPreset { name: "Deep Water", values: &[("size", 340.0), ("density", 26.0), ("spray", 240.0), ("shape", 72.0), ("width", 75.0), ("drift", 65.0), ("blur", 55.0), ("tone", -80.0), ("mix", 85.0), ("output", 3.5)] },
    ],
    sidechain: false,
    samples: 0,
};
