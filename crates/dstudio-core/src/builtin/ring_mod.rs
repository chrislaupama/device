//! Ring Mod (reference: the reference app's built-in "Ring Mod"). Parameter ids,
//! ranges, defaults, tapers and the panel arrangement are taken from the
//! reference as facts; the factory presets (names and values) are our own.

use super::*;

pub static RING_MOD: DeviceDef = DeviceDef {
    id: "ring-mod",
    name: "Ring Mod",
    kind: PluginKind::Effect,
    category: "Modulation",
    purpose: "Multiplies the sound by a carrier wave: metallic, bell-like or robotic tones, and tremolo when biased.",
    params: &[
        ParamDef::new("frequency", "Frequency", 1.0, 20000.0, 440.0, Taper::Log, "Hz", Format::Freq).help("Carrier frequency: low values pulse, audio-rate values add new partials."),
        ParamDef::new("bias", "Bias", 0.0, 100.0, 0.0, LINEAR, "%", Format::Percent).help("Lifts the carrier: 0% is pure ring modulation, 100% keeps the dry note (amplitude modulation)."),
        ParamDef::new("rectify", "Rectify", -100.0, 100.0, 0.0, LINEAR, "%", Format::SignedPercent).help("Folds the carrier's negative half up (+) or its positive half down (−) for a buzzier, octave-up tone."),
        ParamDef::new("spread", "Spread", 0.0, 100.0, 0.0, LINEAR, "%", Format::Percent).help("Detunes the left and right carriers apart, up to a semitone each way."),
        ParamDef::choice("modulator", "Modulator", &["Sine", "Triangle", "Square", "Saw", "Noise", "Random"], 0)
            .help("Carrier wave: Sine is smooth, Triangle brighter, Square and Saw buzzy, Noise grainy, Random a stepped robot."),
        ParamDef::new("mix", "Mix", 0.0, 100.0, 100.0, LINEAR, "%", Format::Percent).help("Balance of the dry and the modulated signal."),
        ParamDef::new("output", "Output", -24.0, 12.0, 0.0, LINEAR, "dB", Format::Db).help("Output level."),
    ],
    layout: Layout {
        picker: &[Control::Dropdown("modulator")],
        view: Some(View::Field(Field::new("ring-mod", "", &["frequency"], &["bias"], 302.0))),
        sliders: &[Control::Slider("output")],
        clusters: &[
            &[Control::Knob("frequency"), Control::Knob("spread")],
            &[Control::Knob("bias"), Control::Knob("rectify")],
            &[Control::Knob("mix")],
        ],
        faces: &[],
        min_height: 204.0,
        fixed_width: 0.0,
        shown_when: &[],
        ..Layout::EMPTY
    },
    factory: &[
        FactoryPreset { name: "Struck Bell", values: &[("frequency", 1100.0), ("mix", 55.0), ("output", 2.5)] },
        FactoryPreset { name: "Machine Talk", values: &[("frequency", 95.0), ("output", 2.0)] },
        FactoryPreset { name: "Clang", values: &[("frequency", 280.0), ("modulator", 2.0), ("mix", 75.0), ("output", 1.0)] },
        FactoryPreset { name: "Sub Growl", values: &[("frequency", 55.0), ("modulator", 3.0), ("mix", 50.0), ("output", 1.5)] },
        FactoryPreset { name: "Half-Wave Buzz", values: &[("frequency", 220.0), ("rectify", 100.0), ("mix", 60.0), ("output", -1.0)] },
        FactoryPreset { name: "Wide Detune Ring", values: &[("frequency", 600.0), ("spread", 100.0), ("modulator", 1.0), ("output", 3.0)] },
        FactoryPreset { name: "Shimmer Air", values: &[("frequency", 3500.0), ("spread", 70.0), ("mix", 25.0), ("output", 0.5)] },
        FactoryPreset { name: "Noise Fizz", values: &[("frequency", 2000.0), ("modulator", 4.0), ("mix", 45.0), ("output", 2.0)] },
        FactoryPreset { name: "Random Bits", values: &[("frequency", 12.0), ("modulator", 5.0), ("bias", 40.0), ("output", 3.0)] },
        FactoryPreset { name: "Soft Pulse", values: &[("frequency", 5.0), ("bias", 100.0), ("mix", 70.0), ("output", 1.5)] },
        FactoryPreset { name: "Square Gate", values: &[("frequency", 6.0), ("bias", 100.0), ("modulator", 2.0), ("output", 2.0)] },
    ],
    sidechain: false,
    samples: 0,
};
