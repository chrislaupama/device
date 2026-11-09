//! Spectrum (reference: the reference app's built-in "Spectrum"). Parameter ids,
//! ranges, defaults, units and the panel arrangement are taken from the
//! reference as facts. The reference ships no factory presets for it, and
//! neither do we.

use super::*;
use crate::device::PluginKind;

const FFT_SIZES: &[&str] = &["512", "1024", "2048", "4096", "8192", "16384"];
const WINDOWS: &[&str] = &["Hann", "Hamming", "Blackman", "Blackman-Harris", "Flat-top", "Rect"];
const CHANNELS: &[&str] = &["Stereo", "L", "R", "Mid", "Side", "Mid+Side"];
const PEAK_HOLD: &[&str] = &["Off", "2s", "Infinite"];
const DISPLAY: &[&str] = &["2D", "3D", "Split"];
/// Colour schemes; the panel draws the trace in the matching stroke colour.
const COLOR_SCHEMES: &[&str] = &["Gold", "Fire", "Viridis", "Magma", "Ice", "Mono"];
const HISTORY: &[&str] = &["32", "64", "96", "128", "192", "256"];

pub static SPECTRUM: DeviceDef = DeviceDef {
    id: "spectrum",
    name: "Spectrum",
    kind: PluginKind::Effect,
    category: "Analyzer",
    purpose: "A real-time frequency analyser; audio passes through unchanged.",
    params: &[
        ParamDef::choice("fft_size", "FFT Size", FFT_SIZES, 3).label("FFT").help("Analysis length: larger sizes resolve low notes better and react more slowly."),
        ParamDef::choice("window", "Window", WINDOWS, 0).help("Analysis window: trades frequency resolution against leakage."),
        ParamDef::choice("channel_mode", "Channel Mode", CHANNELS, 0).label("Channel").help("Which signal is analysed: both channels, one side, mid, side, or mid and side together."),
        ParamDef::new("smoothing", "Smoothing", 0.0, 100.0, 30.0, LINEAR, "%", Format::Percent).help("Blurs the curve across neighbouring frequencies."),
        ParamDef::new("slope", "Slope", -3.0, 6.0, 4.5, LINEAR, "dB/oct", Format::Signed { decimals: 1, scale: 1 }).help("Tilts the display around 1 kHz; 4.5 dB/oct shows a balanced mix as roughly flat."),
        ParamDef::new("db_ceiling", "dB Ceiling", -20.0, 20.0, 0.0, LINEAR, "dB", Format::Db).label("Ceiling").help("Level at the top of the display."),
        ParamDef::new("db_range", "dB Range", 30.0, 120.0, 90.0, LINEAR, "dB", Format::Db).label("Range").help("Level span from the top to the bottom of the display."),
        ParamDef::choice("peak_hold", "Peak Hold", PEAK_HOLD, 1).help("Keeps each frequency's highest level for two seconds or until reset."),
        ParamDef::new("averaging", "Averaging", 0.0, 100.0, 30.0, LINEAR, "%", Format::Percent).help("Slows the curve over time: rises and especially falls."),
        ParamDef::new("decay", "Decay", 0.0, 100.0, 50.0, LINEAR, "%", Format::Percent).help("How slowly levels fall back after a peak."),
        ParamDef::choice("display_mode", "Display Mode", DISPLAY, 1).help("Display style setting kept with the device."),
        ParamDef::choice("color_scheme", "Color Scheme", COLOR_SCHEMES, 0).help("Colour of the curve."),
        ParamDef::choice("history_depth", "History Depth", HISTORY, 3).help("History length setting kept with the device (frames)."),
    ],
    layout: Layout {
        picker: &[Control::Dropdown("fft_size"), Control::Dropdown("channel_mode")],
        view: Some(View::Custom("spectrum")),
        sliders: &[],
        clusters: &[&[Control::Knob("slope"), Control::Knob("db_ceiling"), Control::Knob("db_range"), Control::Knob("smoothing"), Control::Knob("averaging")]],
        faces: &[],
        min_height: 202.0,
        shown_when: &[],
        fixed_width: 397.0,
    },
    factory: &[],
    sidechain: false,
    samples: 0,
};
