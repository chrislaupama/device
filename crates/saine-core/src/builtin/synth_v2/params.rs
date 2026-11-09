// Parameter table of the synth-v2 instrument: ids, names, ranges, defaults,
// units and choice lists in engine (index) order. The values are facts from
// the device spec (handoffs/device-specs/synth-v2.md). Never reorder or
// remove a parameter: the engine and saved projects address them by this
// order / id. The engine mirrors the ids in engine/src/devices/synth_v2_params.hpp.

use super::super::{Format, ParamDef, Taper, LINEAR};
use super::PRESETS;

pub const WAVES: &[&str] = &["Sine", "Triangle", "Saw", "Square", "Sync", "FM"];
pub const OCTAVES: &[&str] = &["−2", "−1", "0", "+1", "+2"];
pub const SEMITONES: &[&str] = &["−12 st", "−11 st", "−10 st", "−9 st", "−8 st", "−7 st", "−6 st", "−5 st", "−4 st", "−3 st", "−2 st", "−1 st", "0 st", "+1 st", "+2 st", "+3 st", "+4 st", "+5 st", "+6 st", "+7 st", "+8 st", "+9 st", "+10 st", "+11 st", "+12 st"];
pub const SUB_WAVES_LEGACY: &[&str] = &["Sine", "Triangle", "Square"];
pub const UNISON: &[&str] = &["1", "2", "3", "4", "5", "6", "7", "8"];
pub const FILTER_TYPES: &[&str] = &["Low-pass 24", "Low-pass 12", "Band-pass", "High-pass", "Notch"];
pub const LFO_SHAPES: &[&str] = &["Sine", "Triangle", "Saw Up", "Saw Down", "Square", "Random", "Smooth Random"];
pub const OFF_ON: &[&str] = &["Off", "On"];
pub const LFO_NOTES: &[&str] = &["8 Bars", "4 Bars", "2 Bars", "1 Bar", "1/2.", "1/2", "1/2T", "1/4.", "1/4", "1/4T", "1/8.", "1/8", "1/8T", "1/16.", "1/16", "1/16T", "1/32"];
pub const MOD_SOURCES: &[&str] = &["None", "Env 2", "Amp Env", "LFO 1", "LFO 2", "Velocity", "Key", "Mod Wheel", "Aftertouch", "Random"];
pub const MOD_TARGETS: &[&str] = &["None", "Pitch", "Osc 1 Pitch", "Osc 2 Pitch", "Osc 1 Shape", "Osc 2 Shape", "Osc 1 Level", "Osc 2 Level", "Cross FM", "Sub Level", "Noise Level", "Cutoff", "Resonance", "Filter Drive", "High-pass", "Amp", "Pan", "Unison Detune", "LFO 1 Rate", "LFO 2 Rate", "LFO 1 Depth", "LFO 2 Depth", "FX 1 Mix", "FX 2 Mix", "FX 3 Mix", "FX 4 Mix", "FX 1 Amount", "FX 2 Amount", "FX 3 Amount", "FX 4 Amount"];
pub const VOICE_MODES: &[&str] = &["Poly", "Mono", "Legato"];
pub const BEND_RANGES: &[&str] = &["0 st", "1 st", "2 st", "3 st", "4 st", "5 st", "6 st", "7 st", "8 st", "9 st", "10 st", "11 st", "12 st", "13 st", "14 st", "15 st", "16 st", "17 st", "18 st", "19 st", "20 st", "21 st", "22 st", "23 st", "24 st"];
pub const FX_TYPES: &[&str] = &["Tape", "Distortion", "Bitcrusher", "Chorus", "Flanger", "Phaser", "Filter", "Delay", "Reverb", "Compressor", "Widener"];
pub const TAPE_MODES: &[&str] = &["Reel-to-Reel", "Cassette", "VHS"];
pub const DISTORTION_MODES: &[&str] = &["Overdrive", "Fuzz", "Tube", "Hard Clip"];
pub const CHORUS_VOICES: &[&str] = &["1", "2", "3", "4"];
pub const FLANGER_MODES: &[&str] = &["Classic", "Jet", "Thru-Zero"];
pub const PHASER_STAGES: &[&str] = &["2", "4", "6", "12"];
pub const FX_FILTER_TYPES: &[&str] = &["Low-pass 12", "Low-pass 24", "High-pass 12", "High-pass 24", "Band-pass 12", "Band-pass 24", "Notch 12", "Notch 24"];
pub const FX_FILTER_LFO_SHAPES: &[&str] = &["Sine", "Triangle", "Saw", "Square", "S&H", "Random"];
pub const DELAY_NOTES: &[&str] = &["1/32", "1/16T", "1/16", "1/16.", "1/8T", "1/8", "1/8.", "1/4T", "1/4", "1/4.", "1/2", "1/2.", "1/1"];
pub const DELAY_MODES: &[&str] = &["Tape", "Digital", "BBD"];
pub const REVERB_MODES: &[&str] = &["Room", "Hall", "Plate", "Spring"];
pub const COMPRESSOR_STYLES: &[&str] = &["Clean", "Opto", "Punch", "Smooth"];
pub const MACRO1_NAMES: &[&str] = &["Macro 1", "Brightness", "Bite", "Body", "Air", "Width", "Space", "Movement", "Wobble", "Grit", "Drive", "Warmth", "Tone", "Sweep", "Swell", "Punch", "Attack", "Release", "Decay", "Detune", "Shimmer", "Depth", "Speed", "Wet", "Glide", "Pitch", "Vibrato", "Squelch", "Growl", "Fizz", "Bloom", "Tension", "Character", "Texture", "Color", "Thickness", "Motion", "Spread", "Edge", "Dirt", "Focus"];
pub const MACRO2_NAMES: &[&str] = &["Macro 2", "Brightness", "Bite", "Body", "Air", "Width", "Space", "Movement", "Wobble", "Grit", "Drive", "Warmth", "Tone", "Sweep", "Swell", "Punch", "Attack", "Release", "Decay", "Detune", "Shimmer", "Depth", "Speed", "Wet", "Glide", "Pitch", "Vibrato", "Squelch", "Growl", "Fizz", "Bloom", "Tension", "Character", "Texture", "Color", "Thickness", "Motion", "Spread", "Edge", "Dirt", "Focus"];
pub const MACRO3_NAMES: &[&str] = &["Macro 3", "Brightness", "Bite", "Body", "Air", "Width", "Space", "Movement", "Wobble", "Grit", "Drive", "Warmth", "Tone", "Sweep", "Swell", "Punch", "Attack", "Release", "Decay", "Detune", "Shimmer", "Depth", "Speed", "Wet", "Glide", "Pitch", "Vibrato", "Squelch", "Growl", "Fizz", "Bloom", "Tension", "Character", "Texture", "Color", "Thickness", "Motion", "Spread", "Edge", "Dirt", "Focus"];
pub const MACRO4_NAMES: &[&str] = &["Macro 4", "Brightness", "Bite", "Body", "Air", "Width", "Space", "Movement", "Wobble", "Grit", "Drive", "Warmth", "Tone", "Sweep", "Swell", "Punch", "Attack", "Release", "Decay", "Detune", "Shimmer", "Depth", "Speed", "Wet", "Glide", "Pitch", "Vibrato", "Squelch", "Growl", "Fizz", "Bloom", "Tension", "Character", "Texture", "Color", "Thickness", "Motion", "Spread", "Edge", "Dirt", "Focus"];
pub const MACRO5_NAMES: &[&str] = &["Macro 5", "Brightness", "Bite", "Body", "Air", "Width", "Space", "Movement", "Wobble", "Grit", "Drive", "Warmth", "Tone", "Sweep", "Swell", "Punch", "Attack", "Release", "Decay", "Detune", "Shimmer", "Depth", "Speed", "Wet", "Glide", "Pitch", "Vibrato", "Squelch", "Growl", "Fizz", "Bloom", "Tension", "Character", "Texture", "Color", "Thickness", "Motion", "Spread", "Edge", "Dirt", "Focus"];
pub const MACRO6_NAMES: &[&str] = &["Macro 6", "Brightness", "Bite", "Body", "Air", "Width", "Space", "Movement", "Wobble", "Grit", "Drive", "Warmth", "Tone", "Sweep", "Swell", "Punch", "Attack", "Release", "Decay", "Detune", "Shimmer", "Depth", "Speed", "Wet", "Glide", "Pitch", "Vibrato", "Squelch", "Growl", "Fizz", "Bloom", "Tension", "Character", "Texture", "Color", "Thickness", "Motion", "Spread", "Edge", "Dirt", "Focus"];
pub const MACRO7_NAMES: &[&str] = &["Macro 7", "Brightness", "Bite", "Body", "Air", "Width", "Space", "Movement", "Wobble", "Grit", "Drive", "Warmth", "Tone", "Sweep", "Swell", "Punch", "Attack", "Release", "Decay", "Detune", "Shimmer", "Depth", "Speed", "Wet", "Glide", "Pitch", "Vibrato", "Squelch", "Growl", "Fizz", "Bloom", "Tension", "Character", "Texture", "Color", "Thickness", "Motion", "Spread", "Edge", "Dirt", "Focus"];
pub const MACRO8_NAMES: &[&str] = &["Macro 8", "Brightness", "Bite", "Body", "Air", "Width", "Space", "Movement", "Wobble", "Grit", "Drive", "Warmth", "Tone", "Sweep", "Swell", "Punch", "Attack", "Release", "Decay", "Detune", "Shimmer", "Depth", "Speed", "Wet", "Glide", "Pitch", "Vibrato", "Squelch", "Growl", "Fizz", "Bloom", "Tension", "Character", "Texture", "Color", "Thickness", "Motion", "Spread", "Edge", "Dirt", "Focus"];
pub const MAP_MACROS: &[&str] = &["None", "Macro 1", "Macro 2", "Macro 3", "Macro 4", "Macro 5", "Macro 6", "Macro 7", "Macro 8"];
pub const MAP_PARAMS: &[&str] = &["None", "Osc 1 Wave", "Osc 1 Shape", "Osc 1 Octave", "Osc 1 Semitones", "Osc 1 Fine", "Osc 1 Level", "Osc 2 Wave", "Osc 2 Shape", "Osc 2 Octave", "Osc 2 Semitones", "Osc 2 Fine", "Osc 2 Level", "Cross FM", "Sub Level", "Sub Wave", "Noise Level", "Noise Color", "Unison", "Unison Detune", "Unison Spread", "Filter Type", "Cutoff", "Resonance", "Filter Drive", "Key Track", "Filter Env", "Filter Velocity", "High-pass", "Attack", "Decay", "Sustain", "Release", "Env 2 Attack", "Env 2 Decay", "Env 2 Sustain", "Env 2 Release", "Velocity", "LFO 1 Shape", "LFO 1 Rate", "LFO 1 Sync", "LFO 1 Note", "LFO 1 Retrigger", "LFO 1 Fade In", "LFO 1 Depth", "LFO 2 Shape", "LFO 2 Rate", "LFO 2 Sync", "LFO 2 Note", "LFO 2 Retrigger", "LFO 2 Fade In", "LFO 2 Depth", "Mod 1 Source", "Mod 1 Target", "Mod 1 Amount", "Mod 1 On", "Mod 2 Source", "Mod 2 Target", "Mod 2 Amount", "Mod 2 On", "Mod 3 Source", "Mod 3 Target", "Mod 3 Amount", "Mod 3 On", "Mod 4 Source", "Mod 4 Target", "Mod 4 Amount", "Mod 4 On", "Mod 5 Source", "Mod 5 Target", "Mod 5 Amount", "Mod 5 On", "Mod 6 Source", "Mod 6 Target", "Mod 6 Amount", "Mod 6 On", "Mod 7 Source", "Mod 7 Target", "Mod 7 Amount", "Mod 7 On", "Mod 8 Source", "Mod 8 Target", "Mod 8 Amount", "Mod 8 On", "Mod 9 Source", "Mod 9 Target", "Mod 9 Amount", "Mod 9 On", "Mod 10 Source", "Mod 10 Target", "Mod 10 Amount", "Mod 10 On", "Mod 11 Source", "Mod 11 Target", "Mod 11 Amount", "Mod 11 On", "Mod 12 Source", "Mod 12 Target", "Mod 12 Amount", "Mod 12 On", "Mod 13 Source", "Mod 13 Target", "Mod 13 Amount", "Mod 13 On", "Mod 14 Source", "Mod 14 Target", "Mod 14 Amount", "Mod 14 On", "Mod 15 Source", "Mod 15 Target", "Mod 15 Amount", "Mod 15 On", "Mod 16 Source", "Mod 16 Target", "Mod 16 Amount", "Mod 16 On", "Mod 17 Source", "Mod 17 Target", "Mod 17 Amount", "Mod 17 On", "Mod 18 Source", "Mod 18 Target", "Mod 18 Amount", "Mod 18 On", "Mod 19 Source", "Mod 19 Target", "Mod 19 Amount", "Mod 19 On", "Mod 20 Source", "Mod 20 Target", "Mod 20 Amount", "Mod 20 On", "Mod 21 Source", "Mod 21 Target", "Mod 21 Amount", "Mod 21 On", "Mod 22 Source", "Mod 22 Target", "Mod 22 Amount", "Mod 22 On", "Mod 23 Source", "Mod 23 Target", "Mod 23 Amount", "Mod 23 On", "Mod 24 Source", "Mod 24 Target", "Mod 24 Amount", "Mod 24 On", "Mod 25 Source", "Mod 25 Target", "Mod 25 Amount", "Mod 25 On", "Mod 26 Source", "Mod 26 Target", "Mod 26 Amount", "Mod 26 On", "Mod 27 Source", "Mod 27 Target", "Mod 27 Amount", "Mod 27 On", "Mod 28 Source", "Mod 28 Target", "Mod 28 Amount", "Mod 28 On", "Mod 29 Source", "Mod 29 Target", "Mod 29 Amount", "Mod 29 On", "Mod 30 Source", "Mod 30 Target", "Mod 30 Amount", "Mod 30 On", "Mod 31 Source", "Mod 31 Target", "Mod 31 Amount", "Mod 31 On", "Mod 32 Source", "Mod 32 Target", "Mod 32 Amount", "Mod 32 On", "Voice Mode", "Glide", "Bend Range", "Drift", "FX 1 Type", "FX 1 On", "FX 1 Mix", "FX 1 Tape Drive", "FX 1 Tape Warmth", "FX 1 Tape Compression", "FX 1 Tape Noise", "FX 1 Tape Mode", "FX 1 Tape Output", "FX 1 Distortion Drive", "FX 1 Distortion Tone", "FX 1 Distortion Bias", "FX 1 Distortion Level", "FX 1 Distortion Mode", "FX 1 Bitcrusher Bits", "FX 1 Bitcrusher Downsample", "FX 1 Bitcrusher Dither", "FX 1 Bitcrusher Jitter", "FX 1 Chorus Rate", "FX 1 Chorus Depth", "FX 1 Chorus Voices", "FX 1 Chorus Width", "FX 1 Chorus Feedback", "FX 1 Chorus Sync", "FX 1 Chorus Note", "FX 1 Flanger Rate", "FX 1 Flanger Depth", "FX 1 Flanger Feedback", "FX 1 Flanger Width", "FX 1 Flanger Mode", "FX 1 Flanger Sync", "FX 1 Flanger Note", "FX 1 Phaser Rate", "FX 1 Phaser Depth", "FX 1 Phaser Feedback", "FX 1 Phaser Stages", "FX 1 Phaser Width", "FX 1 Phaser Sync", "FX 1 Phaser Note", "FX 1 Filter Type", "FX 1 Filter Cutoff", "FX 1 Filter Resonance", "FX 1 Filter Drive", "FX 1 Filter Env Amount", "FX 1 Filter LFO Amount", "FX 1 Filter LFO Rate", "FX 1 Filter LFO Shape", "FX 1 Filter LFO Sync", "FX 1 Filter LFO Note", "FX 1 Delay Sync", "FX 1 Delay Link", "FX 1 Delay Note L", "FX 1 Delay Note R", "FX 1 Delay Time L", "FX 1 Delay Time R", "FX 1 Delay Feedback", "FX 1 Delay Mode", "FX 1 Delay Grit", "FX 1 Delay Mod Depth", "FX 1 Delay Width", "FX 1 Reverb Mode", "FX 1 Reverb Size", "FX 1 Reverb Decay", "FX 1 Reverb Pre-Delay", "FX 1 Reverb Damping", "FX 1 Reverb Width", "FX 1 Reverb Low Cut", "FX 1 Reverb High Cut", "FX 1 Compressor Threshold", "FX 1 Compressor Ratio", "FX 1 Compressor Attack", "FX 1 Compressor Release", "FX 1 Compressor Knee", "FX 1 Compressor Makeup", "FX 1 Compressor Style", "FX 1 Widener Width", "FX 1 Widener Mid", "FX 1 Widener Side", "FX 1 Widener Low Cut", "FX 2 Type", "FX 2 On", "FX 2 Mix", "FX 2 Tape Drive", "FX 2 Tape Warmth", "FX 2 Tape Compression", "FX 2 Tape Noise", "FX 2 Tape Mode", "FX 2 Tape Output", "FX 2 Distortion Drive", "FX 2 Distortion Tone", "FX 2 Distortion Bias", "FX 2 Distortion Level", "FX 2 Distortion Mode", "FX 2 Bitcrusher Bits", "FX 2 Bitcrusher Downsample", "FX 2 Bitcrusher Dither", "FX 2 Bitcrusher Jitter", "FX 2 Chorus Rate", "FX 2 Chorus Depth", "FX 2 Chorus Voices", "FX 2 Chorus Width", "FX 2 Chorus Feedback", "FX 2 Chorus Sync", "FX 2 Chorus Note", "FX 2 Flanger Rate", "FX 2 Flanger Depth", "FX 2 Flanger Feedback", "FX 2 Flanger Width", "FX 2 Flanger Mode", "FX 2 Flanger Sync", "FX 2 Flanger Note", "FX 2 Phaser Rate", "FX 2 Phaser Depth", "FX 2 Phaser Feedback", "FX 2 Phaser Stages", "FX 2 Phaser Width", "FX 2 Phaser Sync", "FX 2 Phaser Note", "FX 2 Filter Type", "FX 2 Filter Cutoff", "FX 2 Filter Resonance", "FX 2 Filter Drive", "FX 2 Filter Env Amount", "FX 2 Filter LFO Amount", "FX 2 Filter LFO Rate", "FX 2 Filter LFO Shape", "FX 2 Filter LFO Sync", "FX 2 Filter LFO Note", "FX 2 Delay Sync", "FX 2 Delay Link", "FX 2 Delay Note L", "FX 2 Delay Note R", "FX 2 Delay Time L", "FX 2 Delay Time R", "FX 2 Delay Feedback", "FX 2 Delay Mode", "FX 2 Delay Grit", "FX 2 Delay Mod Depth", "FX 2 Delay Width", "FX 2 Reverb Mode", "FX 2 Reverb Size", "FX 2 Reverb Decay", "FX 2 Reverb Pre-Delay", "FX 2 Reverb Damping", "FX 2 Reverb Width", "FX 2 Reverb Low Cut", "FX 2 Reverb High Cut", "FX 2 Compressor Threshold", "FX 2 Compressor Ratio", "FX 2 Compressor Attack", "FX 2 Compressor Release", "FX 2 Compressor Knee", "FX 2 Compressor Makeup", "FX 2 Compressor Style", "FX 2 Widener Width", "FX 2 Widener Mid", "FX 2 Widener Side", "FX 2 Widener Low Cut", "FX 3 Type", "FX 3 On", "FX 3 Mix", "FX 3 Tape Drive", "FX 3 Tape Warmth", "FX 3 Tape Compression", "FX 3 Tape Noise", "FX 3 Tape Mode", "FX 3 Tape Output", "FX 3 Distortion Drive", "FX 3 Distortion Tone", "FX 3 Distortion Bias", "FX 3 Distortion Level", "FX 3 Distortion Mode", "FX 3 Bitcrusher Bits", "FX 3 Bitcrusher Downsample", "FX 3 Bitcrusher Dither", "FX 3 Bitcrusher Jitter", "FX 3 Chorus Rate", "FX 3 Chorus Depth", "FX 3 Chorus Voices", "FX 3 Chorus Width", "FX 3 Chorus Feedback", "FX 3 Chorus Sync", "FX 3 Chorus Note", "FX 3 Flanger Rate", "FX 3 Flanger Depth", "FX 3 Flanger Feedback", "FX 3 Flanger Width", "FX 3 Flanger Mode", "FX 3 Flanger Sync", "FX 3 Flanger Note", "FX 3 Phaser Rate", "FX 3 Phaser Depth", "FX 3 Phaser Feedback", "FX 3 Phaser Stages", "FX 3 Phaser Width", "FX 3 Phaser Sync", "FX 3 Phaser Note", "FX 3 Filter Type", "FX 3 Filter Cutoff", "FX 3 Filter Resonance", "FX 3 Filter Drive", "FX 3 Filter Env Amount", "FX 3 Filter LFO Amount", "FX 3 Filter LFO Rate", "FX 3 Filter LFO Shape", "FX 3 Filter LFO Sync", "FX 3 Filter LFO Note", "FX 3 Delay Sync", "FX 3 Delay Link", "FX 3 Delay Note L", "FX 3 Delay Note R", "FX 3 Delay Time L", "FX 3 Delay Time R", "FX 3 Delay Feedback", "FX 3 Delay Mode", "FX 3 Delay Grit", "FX 3 Delay Mod Depth", "FX 3 Delay Width", "FX 3 Reverb Mode", "FX 3 Reverb Size", "FX 3 Reverb Decay", "FX 3 Reverb Pre-Delay", "FX 3 Reverb Damping", "FX 3 Reverb Width", "FX 3 Reverb Low Cut", "FX 3 Reverb High Cut", "FX 3 Compressor Threshold", "FX 3 Compressor Ratio", "FX 3 Compressor Attack", "FX 3 Compressor Release", "FX 3 Compressor Knee", "FX 3 Compressor Makeup", "FX 3 Compressor Style", "FX 3 Widener Width", "FX 3 Widener Mid", "FX 3 Widener Side", "FX 3 Widener Low Cut", "FX 4 Type", "FX 4 On", "FX 4 Mix", "FX 4 Tape Drive", "FX 4 Tape Warmth", "FX 4 Tape Compression", "FX 4 Tape Noise", "FX 4 Tape Mode", "FX 4 Tape Output", "FX 4 Distortion Drive", "FX 4 Distortion Tone", "FX 4 Distortion Bias", "FX 4 Distortion Level", "FX 4 Distortion Mode", "FX 4 Bitcrusher Bits", "FX 4 Bitcrusher Downsample", "FX 4 Bitcrusher Dither", "FX 4 Bitcrusher Jitter", "FX 4 Chorus Rate", "FX 4 Chorus Depth", "FX 4 Chorus Voices", "FX 4 Chorus Width", "FX 4 Chorus Feedback", "FX 4 Chorus Sync", "FX 4 Chorus Note", "FX 4 Flanger Rate", "FX 4 Flanger Depth", "FX 4 Flanger Feedback", "FX 4 Flanger Width", "FX 4 Flanger Mode", "FX 4 Flanger Sync", "FX 4 Flanger Note", "FX 4 Phaser Rate", "FX 4 Phaser Depth", "FX 4 Phaser Feedback", "FX 4 Phaser Stages", "FX 4 Phaser Width", "FX 4 Phaser Sync", "FX 4 Phaser Note", "FX 4 Filter Type", "FX 4 Filter Cutoff", "FX 4 Filter Resonance", "FX 4 Filter Drive", "FX 4 Filter Env Amount", "FX 4 Filter LFO Amount", "FX 4 Filter LFO Rate", "FX 4 Filter LFO Shape", "FX 4 Filter LFO Sync", "FX 4 Filter LFO Note", "FX 4 Delay Sync", "FX 4 Delay Link", "FX 4 Delay Note L", "FX 4 Delay Note R", "FX 4 Delay Time L", "FX 4 Delay Time R", "FX 4 Delay Feedback", "FX 4 Delay Mode", "FX 4 Delay Grit", "FX 4 Delay Mod Depth", "FX 4 Delay Width", "FX 4 Reverb Mode", "FX 4 Reverb Size", "FX 4 Reverb Decay", "FX 4 Reverb Pre-Delay", "FX 4 Reverb Damping", "FX 4 Reverb Width", "FX 4 Reverb Low Cut", "FX 4 Reverb High Cut", "FX 4 Compressor Threshold", "FX 4 Compressor Ratio", "FX 4 Compressor Attack", "FX 4 Compressor Release", "FX 4 Compressor Knee", "FX 4 Compressor Makeup", "FX 4 Compressor Style", "FX 4 Widener Width", "FX 4 Widener Mid", "FX 4 Widener Side", "FX 4 Widener Low Cut", "Volume", "Mod Wheel"];
pub const NOISE_TYPES: &[&str] = &["White", "Pink", "Brown", "Breath", "Metallic", "Digital", "Crackle"];
pub const SUB_OCTAVES: &[&str] = &["−1", "−2"];
pub const LATE_PARAMS: &[&str] = &["None", "Noise Type", "Sub Octave", "Sub Shape", "Noise Width", "Map 1 Late Parameter", "Map 2 Late Parameter", "Map 3 Late Parameter", "Map 4 Late Parameter", "Map 5 Late Parameter", "Map 6 Late Parameter", "Map 7 Late Parameter", "Map 8 Late Parameter", "Map 9 Late Parameter", "Map 10 Late Parameter", "Map 11 Late Parameter", "Map 12 Late Parameter", "Map 13 Late Parameter", "Map 14 Late Parameter", "Map 15 Late Parameter", "Map 16 Late Parameter", "Sub Tune", "Note Priority", "Sub Waveform", "Row 605", "Row 606", "Row 607", "Row 608", "Row 609", "Row 610", "Row 611", "Row 612", "Row 613", "Row 614", "Row 615", "Row 616", "Row 617", "Row 618", "Row 619", "Row 620", "Row 621", "Row 622", "Row 623", "Row 624", "Row 625", "Row 626", "Row 627", "Row 628", "Row 629", "Row 630", "Row 631", "Row 632", "Row 633", "Row 634", "Row 635", "Row 636", "Row 637", "Row 638", "Row 639", "Row 640", "Row 641", "Row 642", "Row 643", "Row 644", "Row 645", "Row 646", "Row 647", "Row 648", "Row 649", "Row 650", "Row 651", "Row 652", "Row 653", "Row 654", "Row 655", "Row 656", "Row 657", "Row 658", "Row 659", "Row 660", "Row 661", "Row 662", "Row 663", "Row 664", "Row 665", "Row 666", "Row 667", "Row 668", "Row 669", "Row 670", "Row 671", "Row 672", "Row 673", "Row 674", "Row 675", "Row 676", "Row 677", "Row 678", "Row 679", "Row 680", "Row 681", "Row 682", "Row 683", "Row 684", "Row 685", "Row 686", "Row 687", "Row 688", "Row 689", "Row 690", "Row 691", "Row 692", "Row 693", "Row 694", "Row 695", "Row 696", "Row 697", "Row 698", "Row 699", "Row 700", "Row 701", "Row 702", "Row 703", "Row 704", "Row 705", "Row 706", "Row 707", "Row 708"];
pub const SUB_TUNE: &[&str] = &["−24 st", "−23 st", "−22 st", "−21 st", "−20 st", "−19 st", "−18 st", "−17 st", "−16 st", "−15 st", "−14 st", "−13 st", "−12 st", "−11 st", "−10 st", "−9 st", "−8 st", "−7 st", "−6 st", "−5 st", "−4 st", "−3 st", "−2 st", "−1 st", "0 st", "+1 st", "+2 st", "+3 st", "+4 st", "+5 st", "+6 st", "+7 st", "+8 st", "+9 st", "+10 st", "+11 st", "+12 st", "+13 st", "+14 st", "+15 st", "+16 st", "+17 st", "+18 st", "+19 st", "+20 st", "+21 st", "+22 st", "+23 st", "+24 st"];
pub const NOTE_PRIORITIES: &[&str] = &["Last", "Low", "High"];
pub const SUB_WAVEFORMS: &[&str] = &["Sine", "Triangle", "Saw", "Square"];

const fn f(id: &'static str, name: &'static str, min: f32, max: f32, default: f32, unit: &'static str, format: Format) -> ParamDef {
    ParamDef::new(id, name, min, max, default, LINEAR, unit, format)
}
#[allow(clippy::too_many_arguments)]
const fn sk(id: &'static str, name: &'static str, min: f32, max: f32, default: f32, skew: f32, unit: &'static str, format: Format) -> ParamDef {
    ParamDef::new(id, name, min, max, default, Taper::Linear { skew }, unit, format)
}
const fn lg(id: &'static str, name: &'static str, min: f32, max: f32, default: f32, unit: &'static str, format: Format) -> ParamDef {
    ParamDef::new(id, name, min, max, default, Taper::Log, unit, format)
}
const fn c(id: &'static str, name: &'static str, choices: &'static [&'static str], default: u32) -> ParamDef {
    ParamDef::choice(id, name, choices, default)
}

pub const PARAM_COUNT: usize = 605;

pub static PARAMS: [ParamDef; PARAM_COUNT] = [
    c("osc1_wave", "Osc 1 Wave", WAVES, 2), // 0
    f("osc1_shape", "Osc 1 Shape", 0.0, 1.0, 0.3, "", Format::Fraction), // 1
    c("osc1_octave", "Osc 1 Octave", OCTAVES, 2), // 2
    c("osc1_semi", "Osc 1 Semitones", SEMITONES, 12), // 3
    f("osc1_fine", "Osc 1 Fine", -50.0, 50.0, 0.0, "ct", Format::Cents), // 4
    f("osc1_level", "Osc 1 Level", 0.0, 1.0, 0.8, "", Format::Fraction), // 5
    c("osc2_wave", "Osc 2 Wave", WAVES, 2), // 6
    f("osc2_shape", "Osc 2 Shape", 0.0, 1.0, 0.0, "", Format::Fraction), // 7
    c("osc2_octave", "Osc 2 Octave", OCTAVES, 2), // 8
    c("osc2_semi", "Osc 2 Semitones", SEMITONES, 12), // 9
    f("osc2_fine", "Osc 2 Fine", -50.0, 50.0, 7.0, "ct", Format::Cents), // 10
    f("osc2_level", "Osc 2 Level", 0.0, 1.0, 0.55, "", Format::Fraction), // 11
    f("cross_fm", "Cross FM", 0.0, 1.0, 0.0, "", Format::Fraction), // 12
    f("sub_level", "Sub Level", 0.0, 1.0, 0.2, "", Format::Fraction), // 13
    c("sub_wave", "Sub Wave", SUB_WAVES_LEGACY, 0), // 14
    f("noise_level", "Noise Level", 0.0, 1.0, 0.0, "", Format::Fraction), // 15
    f("noise_color", "Noise Color", 0.0, 1.0, 0.5, "", Format::Fraction), // 16
    c("unison_voices", "Unison", UNISON, 1), // 17
    f("unison_detune", "Unison Detune", 0.0, 1.0, 0.2, "", Format::Fraction), // 18
    f("unison_spread", "Unison Spread", 0.0, 1.0, 0.7, "", Format::Fraction), // 19
    c("filter_type", "Filter Type", FILTER_TYPES, 0), // 20
    lg("cutoff", "Cutoff", 20.0, 20000.0, 1800.0, "Hz", Format::Hz3), // 21
    f("resonance", "Resonance", 0.0, 1.0, 0.22, "", Format::Fraction), // 22
    f("filter_drive", "Filter Drive", 0.0, 1.0, 0.2, "", Format::Fraction), // 23
    f("key_track", "Key Track", 0.0, 1.0, 0.5, "", Format::Fraction), // 24
    f("filter_env", "Filter Env", -1.0, 1.0, 0.3, "", Format::SignedFraction), // 25
    f("filter_velocity", "Filter Velocity", 0.0, 1.0, 0.3, "", Format::Fraction), // 26
    lg("high_pass", "High-pass", 10.0, 2000.0, 10.0, "Hz", Format::Hz3), // 27
    lg("amp_attack", "Attack", 0.0005, 10.0, 0.003, "s", Format::Seconds), // 28
    lg("amp_decay", "Decay", 0.005, 15.0, 0.9, "s", Format::Seconds), // 29
    f("amp_sustain", "Sustain", 0.0, 1.0, 0.7, "", Format::Fraction), // 30
    lg("amp_release", "Release", 0.005, 15.0, 0.35, "s", Format::Seconds), // 31
    lg("env2_attack", "Env 2 Attack", 0.0005, 10.0, 0.002, "s", Format::Seconds), // 32
    lg("env2_decay", "Env 2 Decay", 0.005, 15.0, 0.55, "s", Format::Seconds), // 33
    f("env2_sustain", "Env 2 Sustain", 0.0, 1.0, 0.25, "", Format::Fraction), // 34
    lg("env2_release", "Env 2 Release", 0.005, 15.0, 0.4, "s", Format::Seconds), // 35
    f("amp_velocity", "Velocity", 0.0, 1.0, 0.5, "", Format::Fraction), // 36
    c("lfo1_shape", "LFO 1 Shape", LFO_SHAPES, 0), // 37
    lg("lfo1_rate", "LFO 1 Rate", 0.02, 40.0, 0.6, "Hz", Format::Hz3), // 38
    c("lfo1_sync", "LFO 1 Sync", OFF_ON, 0), // 39
    c("lfo1_note", "LFO 1 Note", LFO_NOTES, 8), // 40
    c("lfo1_retrigger", "LFO 1 Retrigger", OFF_ON, 0), // 41
    sk("lfo1_fade", "LFO 1 Fade In", 0.0, 5.0, 0.0, 0.5, "s", Format::Seconds), // 42
    f("lfo1_depth", "LFO 1 Depth", 0.0, 1.0, 1.0, "", Format::Fraction), // 43
    c("lfo2_shape", "LFO 2 Shape", LFO_SHAPES, 1), // 44
    lg("lfo2_rate", "LFO 2 Rate", 0.02, 40.0, 5.0, "Hz", Format::Hz3), // 45
    c("lfo2_sync", "LFO 2 Sync", OFF_ON, 0), // 46
    c("lfo2_note", "LFO 2 Note", LFO_NOTES, 11), // 47
    c("lfo2_retrigger", "LFO 2 Retrigger", OFF_ON, 1), // 48
    sk("lfo2_fade", "LFO 2 Fade In", 0.0, 5.0, 0.0, 0.5, "s", Format::Seconds), // 49
    f("lfo2_depth", "LFO 2 Depth", 0.0, 1.0, 0.0, "", Format::Fraction), // 50
    c("mod1_source", "Mod 1 Source", MOD_SOURCES, 3), // 51
    c("mod1_target", "Mod 1 Target", MOD_TARGETS, 4), // 52
    f("mod1_amount", "Mod 1 Amount", -1.0, 1.0, 0.15, "", Format::SignedFraction), // 53
    c("mod1_on", "Mod 1 On", OFF_ON, 1), // 54
    c("mod2_source", "Mod 2 Source", MOD_SOURCES, 4), // 55
    c("mod2_target", "Mod 2 Target", MOD_TARGETS, 1), // 56
    f("mod2_amount", "Mod 2 Amount", -1.0, 1.0, 0.0125, "", Format::SignedFraction), // 57
    c("mod2_on", "Mod 2 On", OFF_ON, 1), // 58
    c("mod3_source", "Mod 3 Source", MOD_SOURCES, 7), // 59
    c("mod3_target", "Mod 3 Target", MOD_TARGETS, 21), // 60
    f("mod3_amount", "Mod 3 Amount", -1.0, 1.0, 1.0, "", Format::SignedFraction), // 61
    c("mod3_on", "Mod 3 On", OFF_ON, 1), // 62
    c("mod4_source", "Mod 4 Source", MOD_SOURCES, 0), // 63
    c("mod4_target", "Mod 4 Target", MOD_TARGETS, 0), // 64
    f("mod4_amount", "Mod 4 Amount", -1.0, 1.0, 0.0, "", Format::SignedFraction), // 65
    c("mod4_on", "Mod 4 On", OFF_ON, 1), // 66
    c("mod5_source", "Mod 5 Source", MOD_SOURCES, 0), // 67
    c("mod5_target", "Mod 5 Target", MOD_TARGETS, 0), // 68
    f("mod5_amount", "Mod 5 Amount", -1.0, 1.0, 0.0, "", Format::SignedFraction), // 69
    c("mod5_on", "Mod 5 On", OFF_ON, 1), // 70
    c("mod6_source", "Mod 6 Source", MOD_SOURCES, 0), // 71
    c("mod6_target", "Mod 6 Target", MOD_TARGETS, 0), // 72
    f("mod6_amount", "Mod 6 Amount", -1.0, 1.0, 0.0, "", Format::SignedFraction), // 73
    c("mod6_on", "Mod 6 On", OFF_ON, 1), // 74
    c("mod7_source", "Mod 7 Source", MOD_SOURCES, 0), // 75
    c("mod7_target", "Mod 7 Target", MOD_TARGETS, 0), // 76
    f("mod7_amount", "Mod 7 Amount", -1.0, 1.0, 0.0, "", Format::SignedFraction), // 77
    c("mod7_on", "Mod 7 On", OFF_ON, 1), // 78
    c("mod8_source", "Mod 8 Source", MOD_SOURCES, 0), // 79
    c("mod8_target", "Mod 8 Target", MOD_TARGETS, 0), // 80
    f("mod8_amount", "Mod 8 Amount", -1.0, 1.0, 0.0, "", Format::SignedFraction), // 81
    c("mod8_on", "Mod 8 On", OFF_ON, 1), // 82
    c("mod9_source", "Mod 9 Source", MOD_SOURCES, 0), // 83
    c("mod9_target", "Mod 9 Target", MOD_TARGETS, 0), // 84
    f("mod9_amount", "Mod 9 Amount", -1.0, 1.0, 0.0, "", Format::SignedFraction), // 85
    c("mod9_on", "Mod 9 On", OFF_ON, 1), // 86
    c("mod10_source", "Mod 10 Source", MOD_SOURCES, 0), // 87
    c("mod10_target", "Mod 10 Target", MOD_TARGETS, 0), // 88
    f("mod10_amount", "Mod 10 Amount", -1.0, 1.0, 0.0, "", Format::SignedFraction), // 89
    c("mod10_on", "Mod 10 On", OFF_ON, 1), // 90
    c("mod11_source", "Mod 11 Source", MOD_SOURCES, 0), // 91
    c("mod11_target", "Mod 11 Target", MOD_TARGETS, 0), // 92
    f("mod11_amount", "Mod 11 Amount", -1.0, 1.0, 0.0, "", Format::SignedFraction), // 93
    c("mod11_on", "Mod 11 On", OFF_ON, 1), // 94
    c("mod12_source", "Mod 12 Source", MOD_SOURCES, 0), // 95
    c("mod12_target", "Mod 12 Target", MOD_TARGETS, 0), // 96
    f("mod12_amount", "Mod 12 Amount", -1.0, 1.0, 0.0, "", Format::SignedFraction), // 97
    c("mod12_on", "Mod 12 On", OFF_ON, 1), // 98
    c("mod13_source", "Mod 13 Source", MOD_SOURCES, 0), // 99
    c("mod13_target", "Mod 13 Target", MOD_TARGETS, 0), // 100
    f("mod13_amount", "Mod 13 Amount", -1.0, 1.0, 0.0, "", Format::SignedFraction), // 101
    c("mod13_on", "Mod 13 On", OFF_ON, 1), // 102
    c("mod14_source", "Mod 14 Source", MOD_SOURCES, 0), // 103
    c("mod14_target", "Mod 14 Target", MOD_TARGETS, 0), // 104
    f("mod14_amount", "Mod 14 Amount", -1.0, 1.0, 0.0, "", Format::SignedFraction), // 105
    c("mod14_on", "Mod 14 On", OFF_ON, 1), // 106
    c("mod15_source", "Mod 15 Source", MOD_SOURCES, 0), // 107
    c("mod15_target", "Mod 15 Target", MOD_TARGETS, 0), // 108
    f("mod15_amount", "Mod 15 Amount", -1.0, 1.0, 0.0, "", Format::SignedFraction), // 109
    c("mod15_on", "Mod 15 On", OFF_ON, 1), // 110
    c("mod16_source", "Mod 16 Source", MOD_SOURCES, 0), // 111
    c("mod16_target", "Mod 16 Target", MOD_TARGETS, 0), // 112
    f("mod16_amount", "Mod 16 Amount", -1.0, 1.0, 0.0, "", Format::SignedFraction), // 113
    c("mod16_on", "Mod 16 On", OFF_ON, 1), // 114
    c("mod17_source", "Mod 17 Source", MOD_SOURCES, 0), // 115
    c("mod17_target", "Mod 17 Target", MOD_TARGETS, 0), // 116
    f("mod17_amount", "Mod 17 Amount", -1.0, 1.0, 0.0, "", Format::SignedFraction), // 117
    c("mod17_on", "Mod 17 On", OFF_ON, 1), // 118
    c("mod18_source", "Mod 18 Source", MOD_SOURCES, 0), // 119
    c("mod18_target", "Mod 18 Target", MOD_TARGETS, 0), // 120
    f("mod18_amount", "Mod 18 Amount", -1.0, 1.0, 0.0, "", Format::SignedFraction), // 121
    c("mod18_on", "Mod 18 On", OFF_ON, 1), // 122
    c("mod19_source", "Mod 19 Source", MOD_SOURCES, 0), // 123
    c("mod19_target", "Mod 19 Target", MOD_TARGETS, 0), // 124
    f("mod19_amount", "Mod 19 Amount", -1.0, 1.0, 0.0, "", Format::SignedFraction), // 125
    c("mod19_on", "Mod 19 On", OFF_ON, 1), // 126
    c("mod20_source", "Mod 20 Source", MOD_SOURCES, 0), // 127
    c("mod20_target", "Mod 20 Target", MOD_TARGETS, 0), // 128
    f("mod20_amount", "Mod 20 Amount", -1.0, 1.0, 0.0, "", Format::SignedFraction), // 129
    c("mod20_on", "Mod 20 On", OFF_ON, 1), // 130
    c("mod21_source", "Mod 21 Source", MOD_SOURCES, 0), // 131
    c("mod21_target", "Mod 21 Target", MOD_TARGETS, 0), // 132
    f("mod21_amount", "Mod 21 Amount", -1.0, 1.0, 0.0, "", Format::SignedFraction), // 133
    c("mod21_on", "Mod 21 On", OFF_ON, 1), // 134
    c("mod22_source", "Mod 22 Source", MOD_SOURCES, 0), // 135
    c("mod22_target", "Mod 22 Target", MOD_TARGETS, 0), // 136
    f("mod22_amount", "Mod 22 Amount", -1.0, 1.0, 0.0, "", Format::SignedFraction), // 137
    c("mod22_on", "Mod 22 On", OFF_ON, 1), // 138
    c("mod23_source", "Mod 23 Source", MOD_SOURCES, 0), // 139
    c("mod23_target", "Mod 23 Target", MOD_TARGETS, 0), // 140
    f("mod23_amount", "Mod 23 Amount", -1.0, 1.0, 0.0, "", Format::SignedFraction), // 141
    c("mod23_on", "Mod 23 On", OFF_ON, 1), // 142
    c("mod24_source", "Mod 24 Source", MOD_SOURCES, 0), // 143
    c("mod24_target", "Mod 24 Target", MOD_TARGETS, 0), // 144
    f("mod24_amount", "Mod 24 Amount", -1.0, 1.0, 0.0, "", Format::SignedFraction), // 145
    c("mod24_on", "Mod 24 On", OFF_ON, 1), // 146
    c("mod25_source", "Mod 25 Source", MOD_SOURCES, 0), // 147
    c("mod25_target", "Mod 25 Target", MOD_TARGETS, 0), // 148
    f("mod25_amount", "Mod 25 Amount", -1.0, 1.0, 0.0, "", Format::SignedFraction), // 149
    c("mod25_on", "Mod 25 On", OFF_ON, 1), // 150
    c("mod26_source", "Mod 26 Source", MOD_SOURCES, 0), // 151
    c("mod26_target", "Mod 26 Target", MOD_TARGETS, 0), // 152
    f("mod26_amount", "Mod 26 Amount", -1.0, 1.0, 0.0, "", Format::SignedFraction), // 153
    c("mod26_on", "Mod 26 On", OFF_ON, 1), // 154
    c("mod27_source", "Mod 27 Source", MOD_SOURCES, 0), // 155
    c("mod27_target", "Mod 27 Target", MOD_TARGETS, 0), // 156
    f("mod27_amount", "Mod 27 Amount", -1.0, 1.0, 0.0, "", Format::SignedFraction), // 157
    c("mod27_on", "Mod 27 On", OFF_ON, 1), // 158
    c("mod28_source", "Mod 28 Source", MOD_SOURCES, 0), // 159
    c("mod28_target", "Mod 28 Target", MOD_TARGETS, 0), // 160
    f("mod28_amount", "Mod 28 Amount", -1.0, 1.0, 0.0, "", Format::SignedFraction), // 161
    c("mod28_on", "Mod 28 On", OFF_ON, 1), // 162
    c("mod29_source", "Mod 29 Source", MOD_SOURCES, 0), // 163
    c("mod29_target", "Mod 29 Target", MOD_TARGETS, 0), // 164
    f("mod29_amount", "Mod 29 Amount", -1.0, 1.0, 0.0, "", Format::SignedFraction), // 165
    c("mod29_on", "Mod 29 On", OFF_ON, 1), // 166
    c("mod30_source", "Mod 30 Source", MOD_SOURCES, 0), // 167
    c("mod30_target", "Mod 30 Target", MOD_TARGETS, 0), // 168
    f("mod30_amount", "Mod 30 Amount", -1.0, 1.0, 0.0, "", Format::SignedFraction), // 169
    c("mod30_on", "Mod 30 On", OFF_ON, 1), // 170
    c("mod31_source", "Mod 31 Source", MOD_SOURCES, 0), // 171
    c("mod31_target", "Mod 31 Target", MOD_TARGETS, 0), // 172
    f("mod31_amount", "Mod 31 Amount", -1.0, 1.0, 0.0, "", Format::SignedFraction), // 173
    c("mod31_on", "Mod 31 On", OFF_ON, 1), // 174
    c("mod32_source", "Mod 32 Source", MOD_SOURCES, 0), // 175
    c("mod32_target", "Mod 32 Target", MOD_TARGETS, 0), // 176
    f("mod32_amount", "Mod 32 Amount", -1.0, 1.0, 0.0, "", Format::SignedFraction), // 177
    c("mod32_on", "Mod 32 On", OFF_ON, 1), // 178
    c("voice_mode", "Voice Mode", VOICE_MODES, 0), // 179
    sk("glide", "Glide", 0.0, 2.0, 0.0, 0.4, "s", Format::Seconds), // 180
    c("bend_range", "Bend Range", BEND_RANGES, 2), // 181
    f("drift", "Drift", 0.0, 1.0, 0.35, "", Format::Fraction), // 182
    c("fx1_type", "FX 1 Type", FX_TYPES, 0), // 183
    c("fx1_on", "FX 1 On", OFF_ON, 0), // 184
    f("fx1_mix", "FX 1 Mix", 0.0, 1.0, 1.0, "", Format::Fraction), // 185
    f("fx1_tape_drive", "FX 1 Tape Drive", 0.0, 1.0, 0.3, "", Format::Fraction), // 186
    f("fx1_tape_warmth", "FX 1 Tape Warmth", 0.0, 1.0, 0.4, "", Format::Fraction), // 187
    f("fx1_tape_compression", "FX 1 Tape Compression", 0.0, 1.0, 0.3, "", Format::Fraction), // 188
    f("fx1_tape_noise", "FX 1 Tape Noise", 0.0, 1.0, 0.0, "", Format::Fraction), // 189
    c("fx1_tape_mode", "FX 1 Tape Mode", TAPE_MODES, 0), // 190
    f("fx1_tape_output", "FX 1 Tape Output", -12.0, 12.0, 0.0, "dB", Format::DbNoPlus), // 191
    f("fx1_distortion_drive", "FX 1 Distortion Drive", 0.0, 1.0, 0.3, "", Format::Fraction), // 192
    lg("fx1_distortion_tone", "FX 1 Distortion Tone", 200.0, 20000.0, 4000.0, "Hz", Format::Hz3), // 193
    f("fx1_distortion_bias", "FX 1 Distortion Bias", 0.0, 1.0, 0.0, "", Format::Fraction), // 194
    f("fx1_distortion_level", "FX 1 Distortion Level", -24.0, 6.0, 0.0, "dB", Format::DbNoPlus), // 195
    c("fx1_distortion_mode", "FX 1 Distortion Mode", DISTORTION_MODES, 0), // 196
    f("fx1_bitcrusher_bits", "FX 1 Bitcrusher Bits", 1.0, 16.0, 8.0, "bits", Format::Plain(1)), // 197
    lg("fx1_bitcrusher_downsample", "FX 1 Bitcrusher Downsample", 1.0, 100.0, 2.0, "x", Format::Factor), // 198
    f("fx1_bitcrusher_dither", "FX 1 Bitcrusher Dither", 0.0, 1.0, 0.0, "", Format::Fraction), // 199
    f("fx1_bitcrusher_jitter", "FX 1 Bitcrusher Jitter", 0.0, 1.0, 0.0, "", Format::Fraction), // 200
    lg("fx1_chorus_rate", "FX 1 Chorus Rate", 0.1, 10.0, 0.5, "Hz", Format::Hz3), // 201
    f("fx1_chorus_depth", "FX 1 Chorus Depth", 0.0, 0.008, 0.004, "s", Format::Seconds), // 202
    c("fx1_chorus_voices", "FX 1 Chorus Voices", CHORUS_VOICES, 0), // 203
    f("fx1_chorus_width", "FX 1 Chorus Width", 0.0, 1.0, 1.0, "", Format::Fraction), // 204
    f("fx1_chorus_feedback", "FX 1 Chorus Feedback", 0.0, 0.9, 0.0, "", Format::Fraction), // 205
    c("fx1_chorus_sync", "FX 1 Chorus Sync", OFF_ON, 0), // 206
    c("fx1_chorus_note", "FX 1 Chorus Note", LFO_NOTES, 8), // 207
    lg("fx1_flanger_rate", "FX 1 Flanger Rate", 0.01, 10.0, 0.3, "Hz", Format::Hz3), // 208
    f("fx1_flanger_depth", "FX 1 Flanger Depth", 0.0, 0.01, 0.003, "s", Format::Seconds), // 209
    f("fx1_flanger_feedback", "FX 1 Flanger Feedback", -0.9, 0.9, 0.6, "", Format::SignedFraction), // 210
    f("fx1_flanger_width", "FX 1 Flanger Width", 0.0, 1.0, 0.5, "", Format::Fraction), // 211
    c("fx1_flanger_mode", "FX 1 Flanger Mode", FLANGER_MODES, 0), // 212
    c("fx1_flanger_sync", "FX 1 Flanger Sync", OFF_ON, 0), // 213
    c("fx1_flanger_note", "FX 1 Flanger Note", LFO_NOTES, 8), // 214
    lg("fx1_phaser_rate", "FX 1 Phaser Rate", 0.01, 10.0, 0.5, "Hz", Format::Hz3), // 215
    f("fx1_phaser_depth", "FX 1 Phaser Depth", 0.0, 1.0, 0.7, "", Format::Fraction), // 216
    f("fx1_phaser_feedback", "FX 1 Phaser Feedback", -0.95, 0.95, 0.4, "", Format::SignedFraction), // 217
    c("fx1_phaser_stages", "FX 1 Phaser Stages", PHASER_STAGES, 1), // 218
    f("fx1_phaser_width", "FX 1 Phaser Width", 0.0, 1.0, 0.5, "", Format::Fraction), // 219
    c("fx1_phaser_sync", "FX 1 Phaser Sync", OFF_ON, 0), // 220
    c("fx1_phaser_note", "FX 1 Phaser Note", LFO_NOTES, 8), // 221
    c("fx1_filter_type", "FX 1 Filter Type", FX_FILTER_TYPES, 0), // 222
    lg("fx1_filter_cutoff", "FX 1 Filter Cutoff", 20.0, 20000.0, 1200.0, "Hz", Format::Hz3), // 223
    f("fx1_filter_resonance", "FX 1 Filter Resonance", 0.0, 1.0, 0.25, "", Format::Fraction), // 224
    f("fx1_filter_drive", "FX 1 Filter Drive", 0.0, 24.0, 0.0, "dB", Format::DbNoPlus), // 225
    f("fx1_filter_env", "FX 1 Filter Env Amount", -72.0, 72.0, 0.0, "st", Format::Semitones), // 226
    f("fx1_filter_lfo_amount", "FX 1 Filter LFO Amount", 0.0, 72.0, 0.0, "st", Format::Semitones), // 227
    lg("fx1_filter_lfo_rate", "FX 1 Filter LFO Rate", 0.01, 30.0, 1.0, "Hz", Format::Hz3), // 228
    c("fx1_filter_lfo_shape", "FX 1 Filter LFO Shape", FX_FILTER_LFO_SHAPES, 0), // 229
    c("fx1_filter_lfo_sync", "FX 1 Filter LFO Sync", OFF_ON, 0), // 230
    c("fx1_filter_lfo_note", "FX 1 Filter LFO Note", LFO_NOTES, 8), // 231
    c("fx1_delay_sync", "FX 1 Delay Sync", OFF_ON, 1), // 232
    c("fx1_delay_link", "FX 1 Delay Link", OFF_ON, 1), // 233
    c("fx1_delay_note_l", "FX 1 Delay Note L", DELAY_NOTES, 6), // 234
    c("fx1_delay_note_r", "FX 1 Delay Note R", DELAY_NOTES, 6), // 235
    lg("fx1_delay_time_l", "FX 1 Delay Time L", 0.001, 4.0, 0.5, "s", Format::Seconds), // 236
    lg("fx1_delay_time_r", "FX 1 Delay Time R", 0.001, 4.0, 0.5, "s", Format::Seconds), // 237
    f("fx1_delay_feedback", "FX 1 Delay Feedback", 0.0, 0.95, 0.35, "", Format::Fraction), // 238
    c("fx1_delay_mode", "FX 1 Delay Mode", DELAY_MODES, 0), // 239
    f("fx1_delay_grit", "FX 1 Delay Grit", 0.0, 1.0, 0.0, "", Format::Fraction), // 240
    f("fx1_delay_mod_depth", "FX 1 Delay Mod Depth", 0.0, 1.0, 0.0, "", Format::Fraction), // 241
    f("fx1_delay_width", "FX 1 Delay Width", 0.0, 2.0, 2.0, "", Format::Fraction), // 242
    c("fx1_reverb_mode", "FX 1 Reverb Mode", REVERB_MODES, 1), // 243
    f("fx1_reverb_size", "FX 1 Reverb Size", 0.0, 1.0, 0.5, "", Format::Fraction), // 244
    lg("fx1_reverb_decay", "FX 1 Reverb Decay", 0.1, 10.0, 2.0, "s", Format::Seconds), // 245
    f("fx1_reverb_pre_delay", "FX 1 Reverb Pre-Delay", 0.0, 0.1, 0.01, "s", Format::Seconds), // 246
    f("fx1_reverb_damping", "FX 1 Reverb Damping", 0.0, 1.0, 0.5, "", Format::Fraction), // 247
    f("fx1_reverb_width", "FX 1 Reverb Width", 0.0, 1.0, 1.0, "", Format::Fraction), // 248
    lg("fx1_reverb_low_cut", "FX 1 Reverb Low Cut", 20.0, 2000.0, 20.0, "Hz", Format::Hz3), // 249
    lg("fx1_reverb_high_cut", "FX 1 Reverb High Cut", 1000.0, 20000.0, 12000.0, "Hz", Format::Hz3), // 250
    f("fx1_compressor_threshold", "FX 1 Compressor Threshold", -60.0, 0.0, -18.0, "dB", Format::DbNoPlus), // 251
    lg("fx1_compressor_ratio", "FX 1 Compressor Ratio", 1.0, 20.0, 4.0, "", Format::Ratio), // 252
    lg("fx1_compressor_attack", "FX 1 Compressor Attack", 0.001, 1.0, 0.01, "s", Format::Seconds), // 253
    lg("fx1_compressor_release", "FX 1 Compressor Release", 0.01, 3.0, 0.1, "s", Format::Seconds), // 254
    f("fx1_compressor_knee", "FX 1 Compressor Knee", 0.0, 40.0, 6.0, "dB", Format::DbNoPlus), // 255
    f("fx1_compressor_makeup", "FX 1 Compressor Makeup", -12.0, 24.0, 0.0, "dB", Format::DbNoPlus), // 256
    c("fx1_compressor_style", "FX 1 Compressor Style", COMPRESSOR_STYLES, 0), // 257
    f("fx1_widener_width", "FX 1 Widener Width", 0.0, 2.0, 1.5, "", Format::Fraction), // 258
    f("fx1_widener_mid", "FX 1 Widener Mid", -12.0, 12.0, 0.0, "dB", Format::DbNoPlus), // 259
    f("fx1_widener_side", "FX 1 Widener Side", -12.0, 12.0, 0.0, "dB", Format::DbNoPlus), // 260
    lg("fx1_widener_low_cut", "FX 1 Widener Low Cut", 20.0, 500.0, 120.0, "Hz", Format::Hz3), // 261
    c("fx2_type", "FX 2 Type", FX_TYPES, 3), // 262
    c("fx2_on", "FX 2 On", OFF_ON, 1), // 263
    f("fx2_mix", "FX 2 Mix", 0.0, 1.0, 0.3333333, "", Format::Fraction), // 264
    f("fx2_tape_drive", "FX 2 Tape Drive", 0.0, 1.0, 0.3, "", Format::Fraction), // 265
    f("fx2_tape_warmth", "FX 2 Tape Warmth", 0.0, 1.0, 0.4, "", Format::Fraction), // 266
    f("fx2_tape_compression", "FX 2 Tape Compression", 0.0, 1.0, 0.3, "", Format::Fraction), // 267
    f("fx2_tape_noise", "FX 2 Tape Noise", 0.0, 1.0, 0.0, "", Format::Fraction), // 268
    c("fx2_tape_mode", "FX 2 Tape Mode", TAPE_MODES, 0), // 269
    f("fx2_tape_output", "FX 2 Tape Output", -12.0, 12.0, 0.0, "dB", Format::DbNoPlus), // 270
    f("fx2_distortion_drive", "FX 2 Distortion Drive", 0.0, 1.0, 0.3, "", Format::Fraction), // 271
    lg("fx2_distortion_tone", "FX 2 Distortion Tone", 200.0, 20000.0, 4000.0, "Hz", Format::Hz3), // 272
    f("fx2_distortion_bias", "FX 2 Distortion Bias", 0.0, 1.0, 0.0, "", Format::Fraction), // 273
    f("fx2_distortion_level", "FX 2 Distortion Level", -24.0, 6.0, 0.0, "dB", Format::DbNoPlus), // 274
    c("fx2_distortion_mode", "FX 2 Distortion Mode", DISTORTION_MODES, 0), // 275
    f("fx2_bitcrusher_bits", "FX 2 Bitcrusher Bits", 1.0, 16.0, 8.0, "bits", Format::Plain(1)), // 276
    lg("fx2_bitcrusher_downsample", "FX 2 Bitcrusher Downsample", 1.0, 100.0, 2.0, "x", Format::Factor), // 277
    f("fx2_bitcrusher_dither", "FX 2 Bitcrusher Dither", 0.0, 1.0, 0.0, "", Format::Fraction), // 278
    f("fx2_bitcrusher_jitter", "FX 2 Bitcrusher Jitter", 0.0, 1.0, 0.0, "", Format::Fraction), // 279
    lg("fx2_chorus_rate", "FX 2 Chorus Rate", 0.1, 10.0, 0.5, "Hz", Format::Hz3), // 280
    f("fx2_chorus_depth", "FX 2 Chorus Depth", 0.0, 0.008, 0.004, "s", Format::Seconds), // 281
    c("fx2_chorus_voices", "FX 2 Chorus Voices", CHORUS_VOICES, 0), // 282
    f("fx2_chorus_width", "FX 2 Chorus Width", 0.0, 1.0, 1.0, "", Format::Fraction), // 283
    f("fx2_chorus_feedback", "FX 2 Chorus Feedback", 0.0, 0.9, 0.0, "", Format::Fraction), // 284
    c("fx2_chorus_sync", "FX 2 Chorus Sync", OFF_ON, 0), // 285
    c("fx2_chorus_note", "FX 2 Chorus Note", LFO_NOTES, 8), // 286
    lg("fx2_flanger_rate", "FX 2 Flanger Rate", 0.01, 10.0, 0.3, "Hz", Format::Hz3), // 287
    f("fx2_flanger_depth", "FX 2 Flanger Depth", 0.0, 0.01, 0.003, "s", Format::Seconds), // 288
    f("fx2_flanger_feedback", "FX 2 Flanger Feedback", -0.9, 0.9, 0.6, "", Format::SignedFraction), // 289
    f("fx2_flanger_width", "FX 2 Flanger Width", 0.0, 1.0, 0.5, "", Format::Fraction), // 290
    c("fx2_flanger_mode", "FX 2 Flanger Mode", FLANGER_MODES, 0), // 291
    c("fx2_flanger_sync", "FX 2 Flanger Sync", OFF_ON, 0), // 292
    c("fx2_flanger_note", "FX 2 Flanger Note", LFO_NOTES, 8), // 293
    lg("fx2_phaser_rate", "FX 2 Phaser Rate", 0.01, 10.0, 0.5, "Hz", Format::Hz3), // 294
    f("fx2_phaser_depth", "FX 2 Phaser Depth", 0.0, 1.0, 0.7, "", Format::Fraction), // 295
    f("fx2_phaser_feedback", "FX 2 Phaser Feedback", -0.95, 0.95, 0.4, "", Format::SignedFraction), // 296
    c("fx2_phaser_stages", "FX 2 Phaser Stages", PHASER_STAGES, 1), // 297
    f("fx2_phaser_width", "FX 2 Phaser Width", 0.0, 1.0, 0.5, "", Format::Fraction), // 298
    c("fx2_phaser_sync", "FX 2 Phaser Sync", OFF_ON, 0), // 299
    c("fx2_phaser_note", "FX 2 Phaser Note", LFO_NOTES, 8), // 300
    c("fx2_filter_type", "FX 2 Filter Type", FX_FILTER_TYPES, 0), // 301
    lg("fx2_filter_cutoff", "FX 2 Filter Cutoff", 20.0, 20000.0, 1200.0, "Hz", Format::Hz3), // 302
    f("fx2_filter_resonance", "FX 2 Filter Resonance", 0.0, 1.0, 0.25, "", Format::Fraction), // 303
    f("fx2_filter_drive", "FX 2 Filter Drive", 0.0, 24.0, 0.0, "dB", Format::DbNoPlus), // 304
    f("fx2_filter_env", "FX 2 Filter Env Amount", -72.0, 72.0, 0.0, "st", Format::Semitones), // 305
    f("fx2_filter_lfo_amount", "FX 2 Filter LFO Amount", 0.0, 72.0, 0.0, "st", Format::Semitones), // 306
    lg("fx2_filter_lfo_rate", "FX 2 Filter LFO Rate", 0.01, 30.0, 1.0, "Hz", Format::Hz3), // 307
    c("fx2_filter_lfo_shape", "FX 2 Filter LFO Shape", FX_FILTER_LFO_SHAPES, 0), // 308
    c("fx2_filter_lfo_sync", "FX 2 Filter LFO Sync", OFF_ON, 0), // 309
    c("fx2_filter_lfo_note", "FX 2 Filter LFO Note", LFO_NOTES, 8), // 310
    c("fx2_delay_sync", "FX 2 Delay Sync", OFF_ON, 1), // 311
    c("fx2_delay_link", "FX 2 Delay Link", OFF_ON, 1), // 312
    c("fx2_delay_note_l", "FX 2 Delay Note L", DELAY_NOTES, 6), // 313
    c("fx2_delay_note_r", "FX 2 Delay Note R", DELAY_NOTES, 6), // 314
    lg("fx2_delay_time_l", "FX 2 Delay Time L", 0.001, 4.0, 0.5, "s", Format::Seconds), // 315
    lg("fx2_delay_time_r", "FX 2 Delay Time R", 0.001, 4.0, 0.5, "s", Format::Seconds), // 316
    f("fx2_delay_feedback", "FX 2 Delay Feedback", 0.0, 0.95, 0.35, "", Format::Fraction), // 317
    c("fx2_delay_mode", "FX 2 Delay Mode", DELAY_MODES, 0), // 318
    f("fx2_delay_grit", "FX 2 Delay Grit", 0.0, 1.0, 0.0, "", Format::Fraction), // 319
    f("fx2_delay_mod_depth", "FX 2 Delay Mod Depth", 0.0, 1.0, 0.0, "", Format::Fraction), // 320
    f("fx2_delay_width", "FX 2 Delay Width", 0.0, 2.0, 2.0, "", Format::Fraction), // 321
    c("fx2_reverb_mode", "FX 2 Reverb Mode", REVERB_MODES, 1), // 322
    f("fx2_reverb_size", "FX 2 Reverb Size", 0.0, 1.0, 0.5, "", Format::Fraction), // 323
    lg("fx2_reverb_decay", "FX 2 Reverb Decay", 0.1, 10.0, 2.0, "s", Format::Seconds), // 324
    f("fx2_reverb_pre_delay", "FX 2 Reverb Pre-Delay", 0.0, 0.1, 0.01, "s", Format::Seconds), // 325
    f("fx2_reverb_damping", "FX 2 Reverb Damping", 0.0, 1.0, 0.5, "", Format::Fraction), // 326
    f("fx2_reverb_width", "FX 2 Reverb Width", 0.0, 1.0, 1.0, "", Format::Fraction), // 327
    lg("fx2_reverb_low_cut", "FX 2 Reverb Low Cut", 20.0, 2000.0, 20.0, "Hz", Format::Hz3), // 328
    lg("fx2_reverb_high_cut", "FX 2 Reverb High Cut", 1000.0, 20000.0, 12000.0, "Hz", Format::Hz3), // 329
    f("fx2_compressor_threshold", "FX 2 Compressor Threshold", -60.0, 0.0, -18.0, "dB", Format::DbNoPlus), // 330
    lg("fx2_compressor_ratio", "FX 2 Compressor Ratio", 1.0, 20.0, 4.0, "", Format::Ratio), // 331
    lg("fx2_compressor_attack", "FX 2 Compressor Attack", 0.001, 1.0, 0.01, "s", Format::Seconds), // 332
    lg("fx2_compressor_release", "FX 2 Compressor Release", 0.01, 3.0, 0.1, "s", Format::Seconds), // 333
    f("fx2_compressor_knee", "FX 2 Compressor Knee", 0.0, 40.0, 6.0, "dB", Format::DbNoPlus), // 334
    f("fx2_compressor_makeup", "FX 2 Compressor Makeup", -12.0, 24.0, 0.0, "dB", Format::DbNoPlus), // 335
    c("fx2_compressor_style", "FX 2 Compressor Style", COMPRESSOR_STYLES, 0), // 336
    f("fx2_widener_width", "FX 2 Widener Width", 0.0, 2.0, 1.5, "", Format::Fraction), // 337
    f("fx2_widener_mid", "FX 2 Widener Mid", -12.0, 12.0, 0.0, "dB", Format::DbNoPlus), // 338
    f("fx2_widener_side", "FX 2 Widener Side", -12.0, 12.0, 0.0, "dB", Format::DbNoPlus), // 339
    lg("fx2_widener_low_cut", "FX 2 Widener Low Cut", 20.0, 500.0, 120.0, "Hz", Format::Hz3), // 340
    c("fx3_type", "FX 3 Type", FX_TYPES, 7), // 341
    c("fx3_on", "FX 3 On", OFF_ON, 0), // 342
    f("fx3_mix", "FX 3 Mix", 0.0, 1.0, 0.2, "", Format::Fraction), // 343
    f("fx3_tape_drive", "FX 3 Tape Drive", 0.0, 1.0, 0.3, "", Format::Fraction), // 344
    f("fx3_tape_warmth", "FX 3 Tape Warmth", 0.0, 1.0, 0.4, "", Format::Fraction), // 345
    f("fx3_tape_compression", "FX 3 Tape Compression", 0.0, 1.0, 0.3, "", Format::Fraction), // 346
    f("fx3_tape_noise", "FX 3 Tape Noise", 0.0, 1.0, 0.0, "", Format::Fraction), // 347
    c("fx3_tape_mode", "FX 3 Tape Mode", TAPE_MODES, 0), // 348
    f("fx3_tape_output", "FX 3 Tape Output", -12.0, 12.0, 0.0, "dB", Format::DbNoPlus), // 349
    f("fx3_distortion_drive", "FX 3 Distortion Drive", 0.0, 1.0, 0.3, "", Format::Fraction), // 350
    lg("fx3_distortion_tone", "FX 3 Distortion Tone", 200.0, 20000.0, 4000.0, "Hz", Format::Hz3), // 351
    f("fx3_distortion_bias", "FX 3 Distortion Bias", 0.0, 1.0, 0.0, "", Format::Fraction), // 352
    f("fx3_distortion_level", "FX 3 Distortion Level", -24.0, 6.0, 0.0, "dB", Format::DbNoPlus), // 353
    c("fx3_distortion_mode", "FX 3 Distortion Mode", DISTORTION_MODES, 0), // 354
    f("fx3_bitcrusher_bits", "FX 3 Bitcrusher Bits", 1.0, 16.0, 8.0, "bits", Format::Plain(1)), // 355
    lg("fx3_bitcrusher_downsample", "FX 3 Bitcrusher Downsample", 1.0, 100.0, 2.0, "x", Format::Factor), // 356
    f("fx3_bitcrusher_dither", "FX 3 Bitcrusher Dither", 0.0, 1.0, 0.0, "", Format::Fraction), // 357
    f("fx3_bitcrusher_jitter", "FX 3 Bitcrusher Jitter", 0.0, 1.0, 0.0, "", Format::Fraction), // 358
    lg("fx3_chorus_rate", "FX 3 Chorus Rate", 0.1, 10.0, 0.5, "Hz", Format::Hz3), // 359
    f("fx3_chorus_depth", "FX 3 Chorus Depth", 0.0, 0.008, 0.004, "s", Format::Seconds), // 360
    c("fx3_chorus_voices", "FX 3 Chorus Voices", CHORUS_VOICES, 0), // 361
    f("fx3_chorus_width", "FX 3 Chorus Width", 0.0, 1.0, 1.0, "", Format::Fraction), // 362
    f("fx3_chorus_feedback", "FX 3 Chorus Feedback", 0.0, 0.9, 0.0, "", Format::Fraction), // 363
    c("fx3_chorus_sync", "FX 3 Chorus Sync", OFF_ON, 0), // 364
    c("fx3_chorus_note", "FX 3 Chorus Note", LFO_NOTES, 8), // 365
    lg("fx3_flanger_rate", "FX 3 Flanger Rate", 0.01, 10.0, 0.3, "Hz", Format::Hz3), // 366
    f("fx3_flanger_depth", "FX 3 Flanger Depth", 0.0, 0.01, 0.003, "s", Format::Seconds), // 367
    f("fx3_flanger_feedback", "FX 3 Flanger Feedback", -0.9, 0.9, 0.6, "", Format::SignedFraction), // 368
    f("fx3_flanger_width", "FX 3 Flanger Width", 0.0, 1.0, 0.5, "", Format::Fraction), // 369
    c("fx3_flanger_mode", "FX 3 Flanger Mode", FLANGER_MODES, 0), // 370
    c("fx3_flanger_sync", "FX 3 Flanger Sync", OFF_ON, 0), // 371
    c("fx3_flanger_note", "FX 3 Flanger Note", LFO_NOTES, 8), // 372
    lg("fx3_phaser_rate", "FX 3 Phaser Rate", 0.01, 10.0, 0.5, "Hz", Format::Hz3), // 373
    f("fx3_phaser_depth", "FX 3 Phaser Depth", 0.0, 1.0, 0.7, "", Format::Fraction), // 374
    f("fx3_phaser_feedback", "FX 3 Phaser Feedback", -0.95, 0.95, 0.4, "", Format::SignedFraction), // 375
    c("fx3_phaser_stages", "FX 3 Phaser Stages", PHASER_STAGES, 1), // 376
    f("fx3_phaser_width", "FX 3 Phaser Width", 0.0, 1.0, 0.5, "", Format::Fraction), // 377
    c("fx3_phaser_sync", "FX 3 Phaser Sync", OFF_ON, 0), // 378
    c("fx3_phaser_note", "FX 3 Phaser Note", LFO_NOTES, 8), // 379
    c("fx3_filter_type", "FX 3 Filter Type", FX_FILTER_TYPES, 0), // 380
    lg("fx3_filter_cutoff", "FX 3 Filter Cutoff", 20.0, 20000.0, 1200.0, "Hz", Format::Hz3), // 381
    f("fx3_filter_resonance", "FX 3 Filter Resonance", 0.0, 1.0, 0.25, "", Format::Fraction), // 382
    f("fx3_filter_drive", "FX 3 Filter Drive", 0.0, 24.0, 0.0, "dB", Format::DbNoPlus), // 383
    f("fx3_filter_env", "FX 3 Filter Env Amount", -72.0, 72.0, 0.0, "st", Format::Semitones), // 384
    f("fx3_filter_lfo_amount", "FX 3 Filter LFO Amount", 0.0, 72.0, 0.0, "st", Format::Semitones), // 385
    lg("fx3_filter_lfo_rate", "FX 3 Filter LFO Rate", 0.01, 30.0, 1.0, "Hz", Format::Hz3), // 386
    c("fx3_filter_lfo_shape", "FX 3 Filter LFO Shape", FX_FILTER_LFO_SHAPES, 0), // 387
    c("fx3_filter_lfo_sync", "FX 3 Filter LFO Sync", OFF_ON, 0), // 388
    c("fx3_filter_lfo_note", "FX 3 Filter LFO Note", LFO_NOTES, 8), // 389
    c("fx3_delay_sync", "FX 3 Delay Sync", OFF_ON, 1), // 390
    c("fx3_delay_link", "FX 3 Delay Link", OFF_ON, 1), // 391
    c("fx3_delay_note_l", "FX 3 Delay Note L", DELAY_NOTES, 6), // 392
    c("fx3_delay_note_r", "FX 3 Delay Note R", DELAY_NOTES, 6), // 393
    lg("fx3_delay_time_l", "FX 3 Delay Time L", 0.001, 4.0, 0.5, "s", Format::Seconds), // 394
    lg("fx3_delay_time_r", "FX 3 Delay Time R", 0.001, 4.0, 0.5, "s", Format::Seconds), // 395
    f("fx3_delay_feedback", "FX 3 Delay Feedback", 0.0, 0.95, 0.35, "", Format::Fraction), // 396
    c("fx3_delay_mode", "FX 3 Delay Mode", DELAY_MODES, 0), // 397
    f("fx3_delay_grit", "FX 3 Delay Grit", 0.0, 1.0, 0.0, "", Format::Fraction), // 398
    f("fx3_delay_mod_depth", "FX 3 Delay Mod Depth", 0.0, 1.0, 0.0, "", Format::Fraction), // 399
    f("fx3_delay_width", "FX 3 Delay Width", 0.0, 2.0, 2.0, "", Format::Fraction), // 400
    c("fx3_reverb_mode", "FX 3 Reverb Mode", REVERB_MODES, 1), // 401
    f("fx3_reverb_size", "FX 3 Reverb Size", 0.0, 1.0, 0.5, "", Format::Fraction), // 402
    lg("fx3_reverb_decay", "FX 3 Reverb Decay", 0.1, 10.0, 2.0, "s", Format::Seconds), // 403
    f("fx3_reverb_pre_delay", "FX 3 Reverb Pre-Delay", 0.0, 0.1, 0.01, "s", Format::Seconds), // 404
    f("fx3_reverb_damping", "FX 3 Reverb Damping", 0.0, 1.0, 0.5, "", Format::Fraction), // 405
    f("fx3_reverb_width", "FX 3 Reverb Width", 0.0, 1.0, 1.0, "", Format::Fraction), // 406
    lg("fx3_reverb_low_cut", "FX 3 Reverb Low Cut", 20.0, 2000.0, 20.0, "Hz", Format::Hz3), // 407
    lg("fx3_reverb_high_cut", "FX 3 Reverb High Cut", 1000.0, 20000.0, 12000.0, "Hz", Format::Hz3), // 408
    f("fx3_compressor_threshold", "FX 3 Compressor Threshold", -60.0, 0.0, -18.0, "dB", Format::DbNoPlus), // 409
    lg("fx3_compressor_ratio", "FX 3 Compressor Ratio", 1.0, 20.0, 4.0, "", Format::Ratio), // 410
    lg("fx3_compressor_attack", "FX 3 Compressor Attack", 0.001, 1.0, 0.01, "s", Format::Seconds), // 411
    lg("fx3_compressor_release", "FX 3 Compressor Release", 0.01, 3.0, 0.1, "s", Format::Seconds), // 412
    f("fx3_compressor_knee", "FX 3 Compressor Knee", 0.0, 40.0, 6.0, "dB", Format::DbNoPlus), // 413
    f("fx3_compressor_makeup", "FX 3 Compressor Makeup", -12.0, 24.0, 0.0, "dB", Format::DbNoPlus), // 414
    c("fx3_compressor_style", "FX 3 Compressor Style", COMPRESSOR_STYLES, 0), // 415
    f("fx3_widener_width", "FX 3 Widener Width", 0.0, 2.0, 1.5, "", Format::Fraction), // 416
    f("fx3_widener_mid", "FX 3 Widener Mid", -12.0, 12.0, 0.0, "dB", Format::DbNoPlus), // 417
    f("fx3_widener_side", "FX 3 Widener Side", -12.0, 12.0, 0.0, "dB", Format::DbNoPlus), // 418
    lg("fx3_widener_low_cut", "FX 3 Widener Low Cut", 20.0, 500.0, 120.0, "Hz", Format::Hz3), // 419
    c("fx4_type", "FX 4 Type", FX_TYPES, 8), // 420
    c("fx4_on", "FX 4 On", OFF_ON, 1), // 421
    f("fx4_mix", "FX 4 Mix", 0.0, 1.0, 0.14, "", Format::Fraction), // 422
    f("fx4_tape_drive", "FX 4 Tape Drive", 0.0, 1.0, 0.3, "", Format::Fraction), // 423
    f("fx4_tape_warmth", "FX 4 Tape Warmth", 0.0, 1.0, 0.4, "", Format::Fraction), // 424
    f("fx4_tape_compression", "FX 4 Tape Compression", 0.0, 1.0, 0.3, "", Format::Fraction), // 425
    f("fx4_tape_noise", "FX 4 Tape Noise", 0.0, 1.0, 0.0, "", Format::Fraction), // 426
    c("fx4_tape_mode", "FX 4 Tape Mode", TAPE_MODES, 0), // 427
    f("fx4_tape_output", "FX 4 Tape Output", -12.0, 12.0, 0.0, "dB", Format::DbNoPlus), // 428
    f("fx4_distortion_drive", "FX 4 Distortion Drive", 0.0, 1.0, 0.3, "", Format::Fraction), // 429
    lg("fx4_distortion_tone", "FX 4 Distortion Tone", 200.0, 20000.0, 4000.0, "Hz", Format::Hz3), // 430
    f("fx4_distortion_bias", "FX 4 Distortion Bias", 0.0, 1.0, 0.0, "", Format::Fraction), // 431
    f("fx4_distortion_level", "FX 4 Distortion Level", -24.0, 6.0, 0.0, "dB", Format::DbNoPlus), // 432
    c("fx4_distortion_mode", "FX 4 Distortion Mode", DISTORTION_MODES, 0), // 433
    f("fx4_bitcrusher_bits", "FX 4 Bitcrusher Bits", 1.0, 16.0, 8.0, "bits", Format::Plain(1)), // 434
    lg("fx4_bitcrusher_downsample", "FX 4 Bitcrusher Downsample", 1.0, 100.0, 2.0, "x", Format::Factor), // 435
    f("fx4_bitcrusher_dither", "FX 4 Bitcrusher Dither", 0.0, 1.0, 0.0, "", Format::Fraction), // 436
    f("fx4_bitcrusher_jitter", "FX 4 Bitcrusher Jitter", 0.0, 1.0, 0.0, "", Format::Fraction), // 437
    lg("fx4_chorus_rate", "FX 4 Chorus Rate", 0.1, 10.0, 0.5, "Hz", Format::Hz3), // 438
    f("fx4_chorus_depth", "FX 4 Chorus Depth", 0.0, 0.008, 0.004, "s", Format::Seconds), // 439
    c("fx4_chorus_voices", "FX 4 Chorus Voices", CHORUS_VOICES, 0), // 440
    f("fx4_chorus_width", "FX 4 Chorus Width", 0.0, 1.0, 1.0, "", Format::Fraction), // 441
    f("fx4_chorus_feedback", "FX 4 Chorus Feedback", 0.0, 0.9, 0.0, "", Format::Fraction), // 442
    c("fx4_chorus_sync", "FX 4 Chorus Sync", OFF_ON, 0), // 443
    c("fx4_chorus_note", "FX 4 Chorus Note", LFO_NOTES, 8), // 444
    lg("fx4_flanger_rate", "FX 4 Flanger Rate", 0.01, 10.0, 0.3, "Hz", Format::Hz3), // 445
    f("fx4_flanger_depth", "FX 4 Flanger Depth", 0.0, 0.01, 0.003, "s", Format::Seconds), // 446
    f("fx4_flanger_feedback", "FX 4 Flanger Feedback", -0.9, 0.9, 0.6, "", Format::SignedFraction), // 447
    f("fx4_flanger_width", "FX 4 Flanger Width", 0.0, 1.0, 0.5, "", Format::Fraction), // 448
    c("fx4_flanger_mode", "FX 4 Flanger Mode", FLANGER_MODES, 0), // 449
    c("fx4_flanger_sync", "FX 4 Flanger Sync", OFF_ON, 0), // 450
    c("fx4_flanger_note", "FX 4 Flanger Note", LFO_NOTES, 8), // 451
    lg("fx4_phaser_rate", "FX 4 Phaser Rate", 0.01, 10.0, 0.5, "Hz", Format::Hz3), // 452
    f("fx4_phaser_depth", "FX 4 Phaser Depth", 0.0, 1.0, 0.7, "", Format::Fraction), // 453
    f("fx4_phaser_feedback", "FX 4 Phaser Feedback", -0.95, 0.95, 0.4, "", Format::SignedFraction), // 454
    c("fx4_phaser_stages", "FX 4 Phaser Stages", PHASER_STAGES, 1), // 455
    f("fx4_phaser_width", "FX 4 Phaser Width", 0.0, 1.0, 0.5, "", Format::Fraction), // 456
    c("fx4_phaser_sync", "FX 4 Phaser Sync", OFF_ON, 0), // 457
    c("fx4_phaser_note", "FX 4 Phaser Note", LFO_NOTES, 8), // 458
    c("fx4_filter_type", "FX 4 Filter Type", FX_FILTER_TYPES, 0), // 459
    lg("fx4_filter_cutoff", "FX 4 Filter Cutoff", 20.0, 20000.0, 1200.0, "Hz", Format::Hz3), // 460
    f("fx4_filter_resonance", "FX 4 Filter Resonance", 0.0, 1.0, 0.25, "", Format::Fraction), // 461
    f("fx4_filter_drive", "FX 4 Filter Drive", 0.0, 24.0, 0.0, "dB", Format::DbNoPlus), // 462
    f("fx4_filter_env", "FX 4 Filter Env Amount", -72.0, 72.0, 0.0, "st", Format::Semitones), // 463
    f("fx4_filter_lfo_amount", "FX 4 Filter LFO Amount", 0.0, 72.0, 0.0, "st", Format::Semitones), // 464
    lg("fx4_filter_lfo_rate", "FX 4 Filter LFO Rate", 0.01, 30.0, 1.0, "Hz", Format::Hz3), // 465
    c("fx4_filter_lfo_shape", "FX 4 Filter LFO Shape", FX_FILTER_LFO_SHAPES, 0), // 466
    c("fx4_filter_lfo_sync", "FX 4 Filter LFO Sync", OFF_ON, 0), // 467
    c("fx4_filter_lfo_note", "FX 4 Filter LFO Note", LFO_NOTES, 8), // 468
    c("fx4_delay_sync", "FX 4 Delay Sync", OFF_ON, 1), // 469
    c("fx4_delay_link", "FX 4 Delay Link", OFF_ON, 1), // 470
    c("fx4_delay_note_l", "FX 4 Delay Note L", DELAY_NOTES, 6), // 471
    c("fx4_delay_note_r", "FX 4 Delay Note R", DELAY_NOTES, 6), // 472
    lg("fx4_delay_time_l", "FX 4 Delay Time L", 0.001, 4.0, 0.5, "s", Format::Seconds), // 473
    lg("fx4_delay_time_r", "FX 4 Delay Time R", 0.001, 4.0, 0.5, "s", Format::Seconds), // 474
    f("fx4_delay_feedback", "FX 4 Delay Feedback", 0.0, 0.95, 0.35, "", Format::Fraction), // 475
    c("fx4_delay_mode", "FX 4 Delay Mode", DELAY_MODES, 0), // 476
    f("fx4_delay_grit", "FX 4 Delay Grit", 0.0, 1.0, 0.0, "", Format::Fraction), // 477
    f("fx4_delay_mod_depth", "FX 4 Delay Mod Depth", 0.0, 1.0, 0.0, "", Format::Fraction), // 478
    f("fx4_delay_width", "FX 4 Delay Width", 0.0, 2.0, 2.0, "", Format::Fraction), // 479
    c("fx4_reverb_mode", "FX 4 Reverb Mode", REVERB_MODES, 1), // 480
    f("fx4_reverb_size", "FX 4 Reverb Size", 0.0, 1.0, 0.5, "", Format::Fraction), // 481
    lg("fx4_reverb_decay", "FX 4 Reverb Decay", 0.1, 10.0, 2.0, "s", Format::Seconds), // 482
    f("fx4_reverb_pre_delay", "FX 4 Reverb Pre-Delay", 0.0, 0.1, 0.01, "s", Format::Seconds), // 483
    f("fx4_reverb_damping", "FX 4 Reverb Damping", 0.0, 1.0, 0.5, "", Format::Fraction), // 484
    f("fx4_reverb_width", "FX 4 Reverb Width", 0.0, 1.0, 1.0, "", Format::Fraction), // 485
    lg("fx4_reverb_low_cut", "FX 4 Reverb Low Cut", 20.0, 2000.0, 20.0, "Hz", Format::Hz3), // 486
    lg("fx4_reverb_high_cut", "FX 4 Reverb High Cut", 1000.0, 20000.0, 12000.0, "Hz", Format::Hz3), // 487
    f("fx4_compressor_threshold", "FX 4 Compressor Threshold", -60.0, 0.0, -18.0, "dB", Format::DbNoPlus), // 488
    lg("fx4_compressor_ratio", "FX 4 Compressor Ratio", 1.0, 20.0, 4.0, "", Format::Ratio), // 489
    lg("fx4_compressor_attack", "FX 4 Compressor Attack", 0.001, 1.0, 0.01, "s", Format::Seconds), // 490
    lg("fx4_compressor_release", "FX 4 Compressor Release", 0.01, 3.0, 0.1, "s", Format::Seconds), // 491
    f("fx4_compressor_knee", "FX 4 Compressor Knee", 0.0, 40.0, 6.0, "dB", Format::DbNoPlus), // 492
    f("fx4_compressor_makeup", "FX 4 Compressor Makeup", -12.0, 24.0, 0.0, "dB", Format::DbNoPlus), // 493
    c("fx4_compressor_style", "FX 4 Compressor Style", COMPRESSOR_STYLES, 0), // 494
    f("fx4_widener_width", "FX 4 Widener Width", 0.0, 2.0, 1.5, "", Format::Fraction), // 495
    f("fx4_widener_mid", "FX 4 Widener Mid", -12.0, 12.0, 0.0, "dB", Format::DbNoPlus), // 496
    f("fx4_widener_side", "FX 4 Widener Side", -12.0, 12.0, 0.0, "dB", Format::DbNoPlus), // 497
    lg("fx4_widener_low_cut", "FX 4 Widener Low Cut", 20.0, 500.0, 120.0, "Hz", Format::Hz3), // 498
    f("volume", "Volume", -36.0, 12.0, -1.5, "dB", Format::DbNoPlus), // 499
    f("mod_wheel", "Mod Wheel", 0.0, 1.0, 0.0, "", Format::Fraction), // 500
    f("macro1", "Macro 1", 0.0, 1.0, 0.0, "", Format::Fraction), // 501
    f("macro2", "Macro 2", 0.0, 1.0, 0.0, "", Format::Fraction), // 502
    f("macro3", "Macro 3", 0.0, 1.0, 0.0, "", Format::Fraction), // 503
    f("macro4", "Macro 4", 0.0, 1.0, 0.0, "", Format::Fraction), // 504
    f("macro5", "Macro 5", 0.0, 1.0, 0.0, "", Format::Fraction), // 505
    f("macro6", "Macro 6", 0.0, 1.0, 0.0, "", Format::Fraction), // 506
    f("macro7", "Macro 7", 0.0, 1.0, 0.0, "", Format::Fraction), // 507
    f("macro8", "Macro 8", 0.0, 1.0, 0.0, "", Format::Fraction), // 508
    c("macro1_name", "Macro 1 Name", MACRO1_NAMES, 0).not_automatable(), // 509
    c("macro2_name", "Macro 2 Name", MACRO2_NAMES, 0).not_automatable(), // 510
    c("macro3_name", "Macro 3 Name", MACRO3_NAMES, 0).not_automatable(), // 511
    c("macro4_name", "Macro 4 Name", MACRO4_NAMES, 0).not_automatable(), // 512
    c("macro5_name", "Macro 5 Name", MACRO5_NAMES, 0).not_automatable(), // 513
    c("macro6_name", "Macro 6 Name", MACRO6_NAMES, 0).not_automatable(), // 514
    c("macro7_name", "Macro 7 Name", MACRO7_NAMES, 0).not_automatable(), // 515
    c("macro8_name", "Macro 8 Name", MACRO8_NAMES, 0).not_automatable(), // 516
    c("map1_macro", "Map 1 Macro", MAP_MACROS, 0).not_automatable(), // 517
    c("map1_param", "Map 1 Parameter", MAP_PARAMS, 0).not_automatable(), // 518
    f("map1_from", "Map 1 From", 0.0, 1.0, 0.0, "", Format::Fraction).not_automatable(), // 519
    f("map1_to", "Map 1 To", 0.0, 1.0, 1.0, "", Format::Fraction).not_automatable(), // 520
    c("map2_macro", "Map 2 Macro", MAP_MACROS, 0).not_automatable(), // 521
    c("map2_param", "Map 2 Parameter", MAP_PARAMS, 0).not_automatable(), // 522
    f("map2_from", "Map 2 From", 0.0, 1.0, 0.0, "", Format::Fraction).not_automatable(), // 523
    f("map2_to", "Map 2 To", 0.0, 1.0, 1.0, "", Format::Fraction).not_automatable(), // 524
    c("map3_macro", "Map 3 Macro", MAP_MACROS, 0).not_automatable(), // 525
    c("map3_param", "Map 3 Parameter", MAP_PARAMS, 0).not_automatable(), // 526
    f("map3_from", "Map 3 From", 0.0, 1.0, 0.0, "", Format::Fraction).not_automatable(), // 527
    f("map3_to", "Map 3 To", 0.0, 1.0, 1.0, "", Format::Fraction).not_automatable(), // 528
    c("map4_macro", "Map 4 Macro", MAP_MACROS, 0).not_automatable(), // 529
    c("map4_param", "Map 4 Parameter", MAP_PARAMS, 0).not_automatable(), // 530
    f("map4_from", "Map 4 From", 0.0, 1.0, 0.0, "", Format::Fraction).not_automatable(), // 531
    f("map4_to", "Map 4 To", 0.0, 1.0, 1.0, "", Format::Fraction).not_automatable(), // 532
    c("map5_macro", "Map 5 Macro", MAP_MACROS, 0).not_automatable(), // 533
    c("map5_param", "Map 5 Parameter", MAP_PARAMS, 0).not_automatable(), // 534
    f("map5_from", "Map 5 From", 0.0, 1.0, 0.0, "", Format::Fraction).not_automatable(), // 535
    f("map5_to", "Map 5 To", 0.0, 1.0, 1.0, "", Format::Fraction).not_automatable(), // 536
    c("map6_macro", "Map 6 Macro", MAP_MACROS, 0).not_automatable(), // 537
    c("map6_param", "Map 6 Parameter", MAP_PARAMS, 0).not_automatable(), // 538
    f("map6_from", "Map 6 From", 0.0, 1.0, 0.0, "", Format::Fraction).not_automatable(), // 539
    f("map6_to", "Map 6 To", 0.0, 1.0, 1.0, "", Format::Fraction).not_automatable(), // 540
    c("map7_macro", "Map 7 Macro", MAP_MACROS, 0).not_automatable(), // 541
    c("map7_param", "Map 7 Parameter", MAP_PARAMS, 0).not_automatable(), // 542
    f("map7_from", "Map 7 From", 0.0, 1.0, 0.0, "", Format::Fraction).not_automatable(), // 543
    f("map7_to", "Map 7 To", 0.0, 1.0, 1.0, "", Format::Fraction).not_automatable(), // 544
    c("map8_macro", "Map 8 Macro", MAP_MACROS, 0).not_automatable(), // 545
    c("map8_param", "Map 8 Parameter", MAP_PARAMS, 0).not_automatable(), // 546
    f("map8_from", "Map 8 From", 0.0, 1.0, 0.0, "", Format::Fraction).not_automatable(), // 547
    f("map8_to", "Map 8 To", 0.0, 1.0, 1.0, "", Format::Fraction).not_automatable(), // 548
    c("map9_macro", "Map 9 Macro", MAP_MACROS, 0).not_automatable(), // 549
    c("map9_param", "Map 9 Parameter", MAP_PARAMS, 0).not_automatable(), // 550
    f("map9_from", "Map 9 From", 0.0, 1.0, 0.0, "", Format::Fraction).not_automatable(), // 551
    f("map9_to", "Map 9 To", 0.0, 1.0, 1.0, "", Format::Fraction).not_automatable(), // 552
    c("map10_macro", "Map 10 Macro", MAP_MACROS, 0).not_automatable(), // 553
    c("map10_param", "Map 10 Parameter", MAP_PARAMS, 0).not_automatable(), // 554
    f("map10_from", "Map 10 From", 0.0, 1.0, 0.0, "", Format::Fraction).not_automatable(), // 555
    f("map10_to", "Map 10 To", 0.0, 1.0, 1.0, "", Format::Fraction).not_automatable(), // 556
    c("map11_macro", "Map 11 Macro", MAP_MACROS, 0).not_automatable(), // 557
    c("map11_param", "Map 11 Parameter", MAP_PARAMS, 0).not_automatable(), // 558
    f("map11_from", "Map 11 From", 0.0, 1.0, 0.0, "", Format::Fraction).not_automatable(), // 559
    f("map11_to", "Map 11 To", 0.0, 1.0, 1.0, "", Format::Fraction).not_automatable(), // 560
    c("map12_macro", "Map 12 Macro", MAP_MACROS, 0).not_automatable(), // 561
    c("map12_param", "Map 12 Parameter", MAP_PARAMS, 0).not_automatable(), // 562
    f("map12_from", "Map 12 From", 0.0, 1.0, 0.0, "", Format::Fraction).not_automatable(), // 563
    f("map12_to", "Map 12 To", 0.0, 1.0, 1.0, "", Format::Fraction).not_automatable(), // 564
    c("map13_macro", "Map 13 Macro", MAP_MACROS, 0).not_automatable(), // 565
    c("map13_param", "Map 13 Parameter", MAP_PARAMS, 0).not_automatable(), // 566
    f("map13_from", "Map 13 From", 0.0, 1.0, 0.0, "", Format::Fraction).not_automatable(), // 567
    f("map13_to", "Map 13 To", 0.0, 1.0, 1.0, "", Format::Fraction).not_automatable(), // 568
    c("map14_macro", "Map 14 Macro", MAP_MACROS, 0).not_automatable(), // 569
    c("map14_param", "Map 14 Parameter", MAP_PARAMS, 0).not_automatable(), // 570
    f("map14_from", "Map 14 From", 0.0, 1.0, 0.0, "", Format::Fraction).not_automatable(), // 571
    f("map14_to", "Map 14 To", 0.0, 1.0, 1.0, "", Format::Fraction).not_automatable(), // 572
    c("map15_macro", "Map 15 Macro", MAP_MACROS, 0).not_automatable(), // 573
    c("map15_param", "Map 15 Parameter", MAP_PARAMS, 0).not_automatable(), // 574
    f("map15_from", "Map 15 From", 0.0, 1.0, 0.0, "", Format::Fraction).not_automatable(), // 575
    f("map15_to", "Map 15 To", 0.0, 1.0, 1.0, "", Format::Fraction).not_automatable(), // 576
    c("map16_macro", "Map 16 Macro", MAP_MACROS, 0).not_automatable(), // 577
    c("map16_param", "Map 16 Parameter", MAP_PARAMS, 0).not_automatable(), // 578
    f("map16_from", "Map 16 From", 0.0, 1.0, 0.0, "", Format::Fraction).not_automatable(), // 579
    f("map16_to", "Map 16 To", 0.0, 1.0, 1.0, "", Format::Fraction).not_automatable(), // 580
    c("preset", "Preset", PRESETS, 0).not_automatable(), // 581
    c("noise_type", "Noise Type", NOISE_TYPES, 0), // 582
    c("sub_octave", "Sub Octave", SUB_OCTAVES, 0), // 583
    f("sub_shape", "Sub Shape", 0.0, 1.0, 0.0, "", Format::Fraction), // 584
    f("noise_width", "Noise Width", 0.0, 1.0, 0.0, "", Format::Fraction), // 585
    c("map1_late_param", "Map 1 Late Parameter", LATE_PARAMS, 0).not_automatable(), // 586
    c("map2_late_param", "Map 2 Late Parameter", LATE_PARAMS, 0).not_automatable(), // 587
    c("map3_late_param", "Map 3 Late Parameter", LATE_PARAMS, 0).not_automatable(), // 588
    c("map4_late_param", "Map 4 Late Parameter", LATE_PARAMS, 0).not_automatable(), // 589
    c("map5_late_param", "Map 5 Late Parameter", LATE_PARAMS, 0).not_automatable(), // 590
    c("map6_late_param", "Map 6 Late Parameter", LATE_PARAMS, 0).not_automatable(), // 591
    c("map7_late_param", "Map 7 Late Parameter", LATE_PARAMS, 0).not_automatable(), // 592
    c("map8_late_param", "Map 8 Late Parameter", LATE_PARAMS, 0).not_automatable(), // 593
    c("map9_late_param", "Map 9 Late Parameter", LATE_PARAMS, 0).not_automatable(), // 594
    c("map10_late_param", "Map 10 Late Parameter", LATE_PARAMS, 0).not_automatable(), // 595
    c("map11_late_param", "Map 11 Late Parameter", LATE_PARAMS, 0).not_automatable(), // 596
    c("map12_late_param", "Map 12 Late Parameter", LATE_PARAMS, 0).not_automatable(), // 597
    c("map13_late_param", "Map 13 Late Parameter", LATE_PARAMS, 0).not_automatable(), // 598
    c("map14_late_param", "Map 14 Late Parameter", LATE_PARAMS, 0).not_automatable(), // 599
    c("map15_late_param", "Map 15 Late Parameter", LATE_PARAMS, 0).not_automatable(), // 600
    c("map16_late_param", "Map 16 Late Parameter", LATE_PARAMS, 0).not_automatable(), // 601
    c("sub_tune", "Sub Tune", SUB_TUNE, 12), // 602
    c("note_priority", "Note Priority", NOTE_PRIORITIES, 0), // 603
    c("sub_waveform", "Sub Waveform", SUB_WAVEFORMS, 0), // 604
];
