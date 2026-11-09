//! Built-in devices: one declarative definition per device.
//!
//! A [`DeviceDef`] lists a device's parameters (id, label, range, default,
//! taper, unit, display format, text values), its panel layout and its factory
//! presets. Everything else is derived from it:
//!
//! * model state: a built-in device is an ordinary [`Device`] in a track's
//!   chain (`PluginFormat::Builtin`, `plugin.id` = definition id) whose plain
//!   parameter values live in `Device::params`; add / remove / reorder /
//!   bypass are the existing chain commands;
//! * edits: `Command::SetDeviceParam` (validated against the definition,
//!   knob drags coalesce per parameter) and `Command::SetDeviceParams`
//!   (preset loads, one undo step);
//! * persistence: values are saved with the project by id, so adding a
//!   parameter to a definition later keeps old projects loading (missing ids
//!   read as the default);
//! * automation: `AutomationTarget::Builtin` lanes hold plain values in the
//!   parameter's unit; LFO / macro targets come from [`mod_targets`];
//! * engine: the snapshot sends the values in definition order and the engine
//!   runs the processor registered under the same id (engine/src/devices);
//! * UI: the generic device panel (crates/saine-app/src/device_panel.rs).
//!
//! Normalisation follows the reference: log parameters map 0..1 to
//! `min * (max / min)^n`, linear ones to `min + (max - min) * n^(1 / skew)`;
//! stepped linear parameters snap in the plain domain. Discrete parameters
//! store the choice index as their plain value (`min` = 0, `max` = choices - 1).

mod autopanner;
mod bitcrusher;
pub mod character_mod;
mod chorus;
mod compressor;
mod distortion;
mod flanger;
mod phaser;
mod ring_mod;
mod soft_clipper;
mod tape_saturation;
mod tremolo;
mod utility;
mod widener;

pub use bitcrusher::crush_picture;
pub use distortion::distortion_transfer;
pub use soft_clipper::{clipper_open, clipper_transfer};
pub use tape_saturation::tape_transfer;
pub use widener::widener_response;

use crate::device::{Device, DeviceId, PluginFormat, PluginKind, PluginRef};
use crate::model::Track;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// How a parameter's plain range maps to the 0..1 control range.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Taper {
    /// `min + (max - min) * n^(1 / skew)` (skew 1 = straight line).
    Linear { skew: f32 },
    /// `min * (max / min)^n`; needs `min > 0`.
    Log,
}

pub const LINEAR: Taper = Taper::Linear { skew: 1.0 };

/// Display and text-entry format of a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// Signed decibels, one decimal: "−24.0 dB", "+3.0 dB".
    Db,
    /// Milliseconds, whole numbers when the range spans 20 ms or more ("10 ms", "1500 ms"), else one decimal.
    Time,
    /// Hertz: "4.0 kHz" from 999.5 Hz, "440 Hz" from 10 Hz, one decimal from 1 Hz, else two.
    Hz,
    /// The reference's generic Hz text: two decimals below 1 Hz ("0.50 Hz"),
    /// one below 10 Hz ("5.0 Hz"), whole numbers below 999.5 Hz ("440 Hz"),
    /// then kilohertz with one decimal ("2.2 kHz"). LFO rates, carriers.
    Freq,
    /// Compression ratio: "4.0:1".
    Ratio,
    /// Percent, at most one decimal: "100%", "12.5%", "−40%".
    Percent,
    /// Whole percent: "30%".
    Percent0,
    /// Decibels, one decimal, a minus sign below −0.05 and never a plus: "0.0 dB", "−1.5 dB".
    DbNoPlus,
    /// Signed decibels like `Db`, but the minimum of the range reads "−∞ dB" (off).
    DbOrInf,
    /// Multiplier: one decimal below 10 ("2.5x"), whole numbers above ("12x").
    Factor,
    /// Left / right balance: "C" at the centre, else "25L" / "25R".
    Balance,
    /// Percent with an explicit sign, whole numbers: "+35%", "−100%", "0%".
    SignedPercent,
    /// Choice name from `ParamDef::choices`.
    Choice,
    /// Number with this many decimals and the unit appended when not empty.
    Plain(u8),
}

/// One parameter of a built-in device.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParamDef {
    /// Stable id: persistence, automation, presets.
    pub id: &'static str,
    /// Full name (automation picker, menus, accessibility).
    pub name: &'static str,
    /// Short label on the panel control (often the same as `name`).
    pub label: &'static str,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    pub taper: Taper,
    /// Plain-domain step (0 = continuous). Ignored for log parameters.
    pub step: f32,
    pub unit: &'static str,
    pub format: Format,
    /// Text values of a discrete parameter; the plain value is the index.
    pub choices: &'static [&'static str],
    /// Shown in the automation picker and as an LFO target.
    pub automatable: bool,
    /// Reset to the default when a preset loads (audition switches).
    pub reset_on_load: bool,
    /// A preset load leaves the current value alone (seeds, locks).
    pub preset_lock: bool,
    /// One-sentence description (tooltips, the assistant).
    pub help: &'static str,
}

impl ParamDef {
    /// A continuous parameter with defaults for the optional fields.
    #[allow(clippy::too_many_arguments)]
    pub const fn new(id: &'static str, name: &'static str, min: f32, max: f32, default: f32, taper: Taper, unit: &'static str, format: Format) -> ParamDef {
        ParamDef { id, name, label: name, min, max, default, taper, step: 0.0, unit, format, choices: &[], automatable: true, reset_on_load: false, preset_lock: false, help: "" }
    }

    /// A discrete parameter: plain value = index into `choices`.
    pub const fn choice(id: &'static str, name: &'static str, choices: &'static [&'static str], default: u32) -> ParamDef {
        ParamDef { id, name, label: name, min: 0.0, max: (choices.len() - 1) as f32, default: default as f32, taper: LINEAR, step: 1.0, unit: "", format: Format::Choice, choices, automatable: true, reset_on_load: false, preset_lock: false, help: "" }
    }

    pub const fn label(mut self, label: &'static str) -> ParamDef {
        self.label = label;
        self
    }
    pub const fn step(mut self, step: f32) -> ParamDef {
        self.step = step;
        self
    }
    pub const fn help(mut self, help: &'static str) -> ParamDef {
        self.help = help;
        self
    }
    pub const fn not_automatable(mut self) -> ParamDef {
        self.automatable = false;
        self
    }
    pub const fn reset_on_load(mut self) -> ParamDef {
        self.reset_on_load = true;
        self
    }
    pub const fn preset_locked(mut self) -> ParamDef {
        self.preset_lock = true;
        self
    }

    pub fn is_discrete(&self) -> bool {
        !self.choices.is_empty()
    }

    /// Bipolar controls draw their arc from the centre (ranges symmetric around 0).
    pub fn is_bipolar(&self) -> bool {
        self.min < 0.0 && self.max > 0.0 && (self.min + self.max).abs() <= 1e-6 * (self.max - self.min)
    }

    /// Nearest valid plain value: clamped, stepped (round half up, as the
    /// reference), discrete values rounded.
    pub fn snap(&self, v: f32) -> f32 {
        let v = if v.is_finite() { v.clamp(self.min, self.max) } else { self.default };
        if self.step > 0.0 && self.taper != Taper::Log {
            (self.min + ((v - self.min) / self.step + 0.501).floor() * self.step).clamp(self.min, self.max)
        } else {
            v
        }
    }

    /// Plain value -> 0..1.
    pub fn to_normalized(&self, v: f32) -> f32 {
        let v = v.clamp(self.min, self.max);
        let n = match self.taper {
            Taper::Log => (v / self.min).ln() / (self.max / self.min).ln(),
            Taper::Linear { skew } => {
                let n = ((v - self.min) / (self.max - self.min)).clamp(0.0, 1.0);
                if skew != 1.0 && n > 0.0 { n.powf(skew) } else { n }
            }
        };
        n.clamp(0.0, 1.0)
    }

    /// 0..1 -> plain value (snapped).
    pub fn from_normalized(&self, n: f32) -> f32 {
        let n = if n.is_finite() { n.clamp(0.0, 1.0) } else { 0.0 };
        let v = match self.taper {
            Taper::Log => self.min * (self.max / self.min).powf(n),
            Taper::Linear { skew } => {
                let n = if skew != 1.0 && n > 0.0 { n.powf(1.0 / skew) } else { n };
                self.min + (self.max - self.min) * n
            }
        };
        self.snap(v)
    }

    /// Display text of a plain value.
    pub fn format(&self, v: f32) -> String {
        if self.format == Format::DbOrInf && v <= self.min + 1e-4 * (self.max - self.min) {
            return format!("{MINUS}\u{221e} dB");
        }
        format_value(self.format, v, self.unit, self.choices, self.max - self.min)
    }

    /// Parse typed text ("-12", "−12 dB", "250ms", "1.2 s", "2k", "Opto")
    /// into a snapped plain value. None when it can't be read.
    pub fn parse(&self, text: &str) -> Option<f32> {
        let t = text.trim();
        if t.is_empty() {
            return None;
        }
        if self.is_discrete() {
            let lower = t.to_lowercase();
            if let Some(i) = self.choices.iter().position(|c| c.to_lowercase() == lower) {
                return Some(i as f32);
            }
        }
        let t = t.replace('\u{2212}', "-").replace(',', ".");
        let lower = t.to_lowercase();
        if self.format == Format::DbOrInf && ["-inf", "-infinity", "-\u{221e}"].iter().any(|w| lower.trim_end_matches("db").trim() == *w) {
            return Some(self.min);
        }
        if self.format == Format::Balance {
            return parse_balance(&lower).map(|v| self.snap(v));
        }
        let split = t.find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-' || c == '+')).unwrap_or(t.len());
        let (num, unit) = t.split_at(split);
        let mut v: f32 = num.parse().ok()?;
        let unit = unit.trim().to_lowercase();
        let unit = unit.trim_end_matches(":1").trim();
        match (self.format, unit) {
            (_, "") => {}
            (Format::Time, "s" | "sec" | "secs" | "second" | "seconds") => v *= 1000.0,
            (Format::Time, "ms" | "msec" | "msecs") => {}
            (Format::Hz | Format::Freq, "k" | "khz") => v *= 1000.0,
            (Format::Hz | Format::Freq, "hz") => {}
            (Format::Db | Format::DbNoPlus | Format::DbOrInf, "db") | (Format::Percent | Format::Percent0 | Format::SignedPercent, "%") | (Format::Factor, "x") => {}
            (Format::Plain(_), u) if u == self.unit.to_lowercase() => {}
            _ => return None,
        }
        if !v.is_finite() {
            return None;
        }
        Some(self.snap(v))
    }
}

const MINUS: char = '\u{2212}';

/// Display text for a value (shared by the panel, automation labels and presets).
/// `range` (max - min) picks the decimals of `Format::Time` as the reference does.
pub fn format_value(format: Format, v: f32, unit: &str, choices: &[&str], range: f32) -> String {
    match format {
        Format::Db => {
            let a = (v.abs() * 10.0).round() / 10.0;
            let sign = if v <= -0.05 { MINUS } else { '+' };
            format!("{sign}{a:.1} dB")
        }
        Format::Time => {
            let d = if range < 20.0 { 1 } else { 0 };
            format!("{v:.d$} ms")
        }
        Format::Hz => {
            if v >= 10_000.0 {
                format!("{:.1} kHz", v / 1000.0)
            } else if v >= 1000.0 {
                format!("{:.2} kHz", v / 1000.0)
            } else {
                format!("{v:.0} Hz")
            }
        }
        Format::DbNoPlus => {
            let a = (v.abs() * 10.0).round() / 10.0;
            if v <= -0.05 { format!("{MINUS}{a:.1} dB") } else { format!("{a:.1} dB") }
        }
        Format::DbOrInf => format_value(Format::Db, v, unit, choices, range),
        Format::Percent0 => format!("{:.0}%", v.round() + 0.0),
        Format::Factor => {
            if v < 9.95 { format!("{v:.1}x") } else { format!("{v:.0}x") }
        }
        Format::Balance => {
            let r = v.round();
            if r == 0.0 { "C".into() } else if r < 0.0 { format!("{:.0}L", -r) } else { format!("{r:.0}R") }
        }
        Format::Freq => {
            if v >= 999.5 {
                format!("{:.1} kHz", v / 1000.0)
            } else if v >= 9.995 {
                format!("{v:.0} Hz")
            } else if v >= 0.995 {
                format!("{v:.1} Hz")
            } else {
                format!("{v:.2} Hz")
            }
        }
        Format::Ratio => format!("{v:.1}:1"),
        Format::Percent => {
            let r = (v * 10.0).round() / 10.0;
            let s = if r.fract() == 0.0 { format!("{:.0}%", r.abs()) } else { format!("{:.1}%", r.abs()) };
            if r < 0.0 { format!("{MINUS}{s}") } else { s }
        }
        Format::SignedPercent => {
            let r = v.round();
            if r == 0.0 { "0%".into() } else if r < 0.0 { format!("{MINUS}{}%", -r) } else { format!("+{r}%") }
        }
        Format::Choice => {
            let i = v.round().max(0.0) as usize;
            choices.get(i).copied().unwrap_or("").to_string()
        }
        Format::Plain(d) => {
            let s = format!("{:.*}", d as usize, v).replace('-', &MINUS.to_string());
            if unit.is_empty() { s } else { format!("{s} {unit}") }
        }
    }
}

/// "C", "25L", "L25", "25 r", "-25" (left negative) -> balance value.
fn parse_balance(t: &str) -> Option<f32> {
    let t = t.trim();
    if t == "c" || t == "center" || t == "centre" {
        return Some(0.0);
    }
    let (side, num) = if let Some(n) = t.strip_suffix('l').or_else(|| t.strip_prefix('l')) {
        (-1.0, n)
    } else if let Some(n) = t.strip_suffix('r').or_else(|| t.strip_prefix('r')) {
        (1.0, n)
    } else {
        (1.0, t)
    };
    let v: f32 = num.trim().parse().ok()?;
    (v.is_finite() && (side > 0.0 || v >= 0.0)).then_some(side * v)
}

// ---------------------------------------------------------------- layout

/// A control on the generic device panel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Control {
    /// Rotary knob: value above, label below (reference fx dial row).
    Knob(&'static str),
    /// Vertical slider (reference fx slider column, 62 pt wide).
    Slider(&'static str),
    /// Option pill with a menu (reference picker row): label + chosen value + chevron.
    Menu(&'static str),
    /// Value pill dragged like a knob (picker row), e.g. sidechain gain.
    Pill(&'static str),
    /// Two-state latch button (On/Off parameters).
    Toggle(&'static str),
    /// Sidechain source picker (a track list, not a parameter).
    Sidechain,
    /// Option pill that opens a menu of the choices (picker row): label +
    /// chosen value + chevron, for lists too long for a segmented track.
    Dropdown(&'static str),
    /// Segmented picker whose segments show these labels instead of the
    /// parameter's choice names (e.g. a Sync Off/On parameter shown as Free / Sync).
    Segments(&'static str, &'static [&'static str]),
    /// Flexible gap in the picker row: the controls after it align right.
    Gap,
    /// One knob cell showing the first of these parameters that is shown
    /// (`Layout::shown_when`), e.g. Rate in Hz or Rate Sync as a note value.
    Swap(&'static [&'static str]),
}

impl Control {
    pub fn param(&self) -> Option<&'static str> {
        match *self {
            Control::Knob(p) | Control::Slider(p) | Control::Menu(p) | Control::Pill(p) | Control::Toggle(p) | Control::Dropdown(p) | Control::Segments(p, _) => Some(p),
            Control::Swap(ps) => ps.first().copied(),
            Control::Sidechain | Control::Gap => None,
        }
    }

    /// Every parameter the control can show.
    pub fn params(&self) -> Vec<&'static str> {
        match *self {
            Control::Swap(ps) => ps.to_vec(),
            _ => self.param().into_iter().collect(),
        }
    }
}

/// A visualiser or custom view drawn by the panel code.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum View {
    /// Scrolling level / gain-reduction strip with a draggable threshold
    /// line bound to `threshold` (reference dynamics strip).
    DynamicsStrip { threshold: &'static str },
    /// A device-specific view implemented in the panel code under this name
    /// (escape hatch for EQ curves, shaper, trance gate grid...).
    Custom(&'static str),
    /// Plot well with traces computed by the panel code (reference motion,
    /// response and sweep fields) that edits two parameters by dragging.
    Field(Field),
}

/// A plotted field in the view row (reference `flow-fx-motion-field` and
/// response fields): caption strip, tick labels, up to three traces, a fill,
/// an optional live LFO overlay, and an XY drag.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Field {
    /// Curve model in the panel code (crates/saine-app/src/device_fields.rs).
    pub plot: &'static str,
    /// Caption at the top left ("" = no caption strip; the well starts at the top).
    pub caption: &'static str,
    /// Horizontal drag: the first of these parameters that is shown (Rate / Rate Sync).
    pub x: &'static [&'static str],
    /// Vertical drag: the first of these parameters that is shown.
    pub y: &'static [&'static str],
    /// Width of the field in reference pt; the card is at least this plus
    /// the slider column and padding wide.
    pub width: f32,
}

impl Field {
    pub const fn new(plot: &'static str, caption: &'static str, x: &'static [&'static str], y: &'static [&'static str], width: f32) -> Field {
        Field { plot, caption, x, y, width }
    }
}

/// Drawing inside a knob instead of the plain body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Face {
    /// Static transfer curve of a soft-knee compressor.
    SoftKneeCurve { threshold: &'static str, ratio: &'static str, knee: &'static str },
}

/// Panel layout, top to bottom: picker row, view row (view + slider column),
/// dial row (clusters of knobs). This is the reference fx card template;
/// devices with a different body use `View::Custom`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Layout {
    /// Picker row; a `Control::Sidechain` and everything after it align right.
    pub picker: &'static [Control],
    pub view: Option<View>,
    /// Slider column at the right of the view.
    pub sliders: &'static [Control],
    /// Dial row: clusters of knobs.
    pub clusters: &'static [&'static [Control]],
    /// Knob faces by parameter id.
    pub faces: &'static [(&'static str, Face)],
    /// Card body minimum height (pt).
    pub min_height: f32,
    /// (param, discrete param, values): `param` is shown only while the
    /// discrete parameter has one of `values` (mode-dependent layouts).
    pub shown_when: &'static [(&'static str, &'static str, &'static [u32])],
    /// Card width (pt) for bodies that are not sized by the dial row
    /// (`View::Custom` grids); 0 = derived from `clusters`.
    pub fixed_width: f32,
}

/// Reference dial row metrics (pt).
pub const KNOB_CELL: f32 = 62.0;
pub const KNOB_GAP: f32 = 10.0;
pub const CLUSTER_GAP: f32 = 21.0;
pub const CARD_PAD: f32 = 16.0;

impl Layout {
    /// Card width from the dial clusters: cells * 62 + inner gaps * 10 +
    /// cluster gaps * 21, plus 16 of padding (reference card sizing rule).
    pub fn width(&self) -> f32 {
        if self.fixed_width > 0.0 {
            return self.fixed_width;
        }
        let cells: usize = self.clusters.iter().map(|c| c.len()).sum();
        let inner: usize = self.clusters.iter().map(|c| c.len().saturating_sub(1)).sum();
        let gaps = self.clusters.len().saturating_sub(1);
        let dials = cells as f32 * KNOB_CELL + inner as f32 * KNOB_GAP + gaps as f32 * CLUSTER_GAP + CARD_PAD;
        // A field sets its own width; the view row is field + gap + slider column(s).
        let view = match self.view {
            Some(View::Field(f)) => f.width + self.sliders.len() as f32 * (KNOB_GAP + KNOB_CELL) + CARD_PAD,
            _ => 0.0,
        };
        dials.max(view)
    }

    /// Every control, in panel order.
    pub fn controls(&self) -> impl Iterator<Item = &Control> {
        self.picker.iter().chain(self.sliders.iter()).chain(self.clusters.iter().flat_map(|c| c.iter()))
    }
}

// ---------------------------------------------------------------- devices

/// A factory preset: values by parameter id (missing ids keep their default).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FactoryPreset {
    pub name: &'static str,
    pub values: &'static [(&'static str, f32)],
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DeviceDef {
    /// Stable id (`PluginRef::id`, engine registry key, preset folder).
    pub id: &'static str,
    pub name: &'static str,
    pub kind: PluginKind,
    /// Browser category ("Dynamics", "EQ & Filter", ...).
    pub category: &'static str,
    /// One-sentence purpose (browser, assistant).
    pub purpose: &'static str,
    /// Parameters in engine order: the engine receives values in this order
    /// and the processor declares the same ids (checked by a test).
    pub params: &'static [ParamDef],
    pub layout: Layout,
    pub factory: &'static [FactoryPreset],
    /// The device reads a sidechain input (another track) when one is picked.
    pub sidechain: bool,
}

/// The vendor shown for built-in devices.
pub const VENDOR: &str = "Saine Studio";

/// Every built-in device, in browser order.
pub fn all() -> &'static [&'static DeviceDef] {
    static ALL: [&DeviceDef; 14] = [
        &compressor::COMPRESSOR,
        &distortion::DISTORTION,
        &tape_saturation::TAPE_SATURATION,
        &soft_clipper::SOFT_CLIPPER,
        &bitcrusher::BITCRUSHER,
        &widener::WIDENER,
        &utility::UTILITY,
        &chorus::CHORUS,
        &character_mod::CHARACTER_MOD,
        &flanger::FLANGER,
        &phaser::PHASER,
        &tremolo::TREMOLO,
        &autopanner::AUTOPANNER,
        &ring_mod::RING_MOD,
    ];
    &ALL
}

/// Other parameters a panel edit of `param` moves along in the same undo
/// step, and the undo label to use (e.g. Character Mod's circuit switch
/// carries per-circuit defaults). Empty for most devices.
pub fn linked_writes(def: &DeviceDef, device: &Device, param: &str, value: f32) -> (Vec<(&'static str, f32)>, Option<String>) {
    match def.id {
        "character-mod" => character_mod::switch_writes(def, device, param, value),
        _ => (Vec::new(), None),
    }
}

// ---------------------------------------------------------------- tempo sync

/// Note values of a synced LFO (reference value table LFO_DIVISION), slowest
/// first; the plain value of a Rate Sync parameter is the index.
pub const DIVISIONS: &[&str] = &["8/1", "4/1", "2/1", "1/1", "1/2.", "1/2", "1/2T", "1/4.", "1/4", "1/4T", "1/8.", "1/8", "1/8T", "1/16.", "1/16", "1/16T", "1/32", "1/64.", "1/64", "1/64T"];
/// Length of each of [DIVISIONS] in quarter notes.
pub const DIVISION_QUARTERS: &[f32] =
    &[32.0, 16.0, 8.0, 4.0, 3.0, 2.0, 4.0 / 3.0, 1.5, 1.0, 2.0 / 3.0, 0.75, 0.5, 1.0 / 3.0, 0.375, 0.25, 1.0 / 6.0, 0.125, 0.093_75, 0.0625, 1.0 / 24.0];
/// Index of 1/4 in [DIVISIONS].
pub const DIVISION_QUARTER: u32 = 8;
/// Character Mod's note table (reference MOD_NOTE), slowest first.
pub const MOD_NOTES: &[&str] =
    &["2/1.", "2/1", "2/1T", "1/1.", "1/1", "1/1T", "1/2.", "1/2", "1/2T", "1/4.", "1/4", "1/4T", "1/8.", "1/8", "1/8T", "1/16.", "1/16", "1/16T", "1/32.", "1/32", "1/32T"];
/// Length of each of [MOD_NOTES] in quarter notes.
pub const MOD_NOTE_QUARTERS: &[f32] = &[
    12.0, 8.0, 16.0 / 3.0, 6.0, 4.0, 8.0 / 3.0, 3.0, 2.0, 4.0 / 3.0, 1.5, 1.0, 2.0 / 3.0, 0.75, 0.5, 1.0 / 3.0, 0.375, 0.25, 1.0 / 6.0, 0.1875, 0.125, 1.0 / 12.0,
];
/// Free / Sync labels of a timebase picker.
pub const FREE_SYNC: &[&str] = &["Free", "Sync"];

/// LFO rate in Hz of a synced note value of `quarters` quarter notes at `bpm`.
pub fn synced_hz(quarters: f32, bpm: f32) -> f32 {
    let bpm = if bpm.is_finite() && bpm > 0.0 { bpm } else { 120.0 };
    bpm / 60.0 / quarters.max(1e-3)
}

pub fn find(id: &str) -> Option<&'static DeviceDef> {
    all().iter().copied().find(|d| d.id == id)
}

/// Definition of a device in the model, if it is a built-in.
pub fn def_of(d: &Device) -> Option<&'static DeviceDef> {
    if d.plugin.format == PluginFormat::Builtin { find(&d.plugin.id) } else { None }
}

impl DeviceDef {
    pub fn param(&self, id: &str) -> Option<&'static ParamDef> {
        self.params.iter().find(|p| p.id == id)
    }

    pub fn index_of(&self, id: &str) -> Option<usize> {
        self.params.iter().position(|p| p.id == id)
    }

    pub fn plugin_ref(&self) -> PluginRef {
        PluginRef { format: PluginFormat::Builtin, id: self.id.into(), path: None, name: self.name.into(), vendor: VENDOR.into(), version: String::new(), kind: self.kind }
    }

    pub fn defaults(&self) -> BTreeMap<String, f32> {
        self.params.iter().map(|p| (p.id.to_string(), p.default)).collect()
    }

    /// A new instance with default values (id 0: allocated by `AddDevice`).
    pub fn instantiate(&self) -> Device {
        let mut d = Device::new(self.plugin_ref());
        d.params = self.defaults();
        d
    }

    /// Value of `param` on `device` (the default when the device doesn't store it).
    pub fn value(&self, device: &Device, param: &ParamDef) -> f32 {
        device.params.get(param.id).copied().map(|v| param.snap(v)).unwrap_or(param.default)
    }

    /// All values in engine order.
    pub fn values(&self, device: &Device) -> Vec<f32> {
        self.params.iter().map(|p| self.value(device, p)).collect()
    }

    /// Complete values of a factory preset (defaults filled in).
    pub fn factory_values(&self, preset: &FactoryPreset) -> BTreeMap<String, f32> {
        let mut v = self.defaults();
        for (id, x) in preset.values {
            if let Some(p) = self.param(id) {
                v.insert((*id).to_string(), p.snap(*x));
            }
        }
        v
    }

    /// Whether `param` is visible in the device's current state (`Layout::shown_when`).
    pub fn is_shown(&self, device: &Device, param: &str) -> bool {
        self.layout.shown_when.iter().filter(|(p, _, _)| *p == param).all(|(_, by, values)| {
            self.param(by).map(|b| values.contains(&(self.value(device, b).round() as u32))).unwrap_or(true)
        })
    }
}

/// Check one parameter edit against the definition: the parameter exists and
/// the value is finite and inside the range. Returns the snapped value.
pub fn validate_value(def: &DeviceDef, param: &str, v: f32) -> Result<f32, String> {
    let p = def.param(param).ok_or_else(|| format!("{} has no parameter {param:?}", def.name))?;
    let tol = 1e-4 * (p.max - p.min);
    if !v.is_finite() || v < p.min - tol || v > p.max + tol {
        return Err(format!("{} {v} outside {}..={}", p.name, p.min, p.max));
    }
    Ok(p.snap(v))
}

// ---------------------------------------------------------------- presets

/// A saved device preset (user presets are JSON files written by the app).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DevicePreset {
    /// Definition id the preset belongs to.
    pub device: String,
    pub name: String,
    pub values: BTreeMap<String, f32>,
}

impl DevicePreset {
    /// Capture a device's current values.
    pub fn capture(def: &DeviceDef, device: &Device, name: &str) -> DevicePreset {
        DevicePreset { device: def.id.into(), name: name.trim().into(), values: def.params.iter().map(|p| (p.id.to_string(), def.value(device, p))).collect() }
    }

    pub fn factory(def: &DeviceDef, preset: &FactoryPreset) -> DevicePreset {
        DevicePreset { device: def.id.into(), name: preset.name.into(), values: def.factory_values(preset) }
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("preset serialises")
    }

    /// Parse a preset file for `def`: unknown ids are dropped, values are
    /// snapped into range, missing ids take the default.
    pub fn from_json(def: &DeviceDef, text: &str) -> Result<DevicePreset, String> {
        let raw: DevicePreset = serde_json::from_str(text).map_err(|e| e.to_string())?;
        if raw.device != def.id {
            return Err(format!("preset is for {:?}, not {:?}", raw.device, def.id));
        }
        let mut values = def.defaults();
        for (k, v) in raw.values {
            if let Some(p) = def.param(&k) {
                values.insert(k, p.snap(v));
            }
        }
        Ok(DevicePreset { device: raw.device, name: raw.name, values })
    }

    /// Values to apply when loading: `reset_on_load` parameters go back to
    /// their default; `preset_lock` parameters are left out (they keep the
    /// device's current value).
    pub fn load_values(&self, def: &DeviceDef) -> BTreeMap<String, f32> {
        let mut v = self.values.clone();
        for p in def.params {
            if p.reset_on_load {
                v.insert(p.id.into(), p.default);
            } else if p.preset_lock {
                v.remove(p.id);
            }
        }
        v
    }
}

// ---------------------------------------------------------------- modulation

/// A parameter an LFO (or macro) can modulate.
#[derive(Debug, Clone, PartialEq)]
pub struct ModTarget {
    pub device: DeviceId,
    pub device_name: String,
    pub param: &'static str,
    pub param_name: &'static str,
}

/// LFO / macro targets on a track: every automatable parameter of its
/// built-in audio effects (the reference offers audio-effect parameters, not
/// instruments' or the LFO's own).
pub fn mod_targets(track: &Track) -> Vec<ModTarget> {
    track
        .devices
        .iter()
        .filter_map(|d| def_of(d).map(|def| (d, def)))
        .filter(|(_, def)| def.kind == PluginKind::Effect)
        .flat_map(|(d, def)| def.params.iter().filter(|p| p.automatable).map(move |p| ModTarget { device: d.id, device_name: def.name.into(), param: p.id, param_name: p.name }))
        .collect()
}

/// Apply a modulation offset (normalized units, -1..1) to a plain value the
/// way the reference LFO does: in the normalized domain, clamped.
pub fn modulate(p: &ParamDef, plain: f32, offset: f32) -> f32 {
    p.from_normalized((p.to_normalized(plain) + offset).clamp(0.0, 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn comp() -> &'static DeviceDef {
        find("compressor").unwrap()
    }

    #[test]
    fn every_definition_is_consistent() {
        for def in all() {
            let mut ids = std::collections::HashSet::new();
            for p in def.params {
                assert!(ids.insert(p.id), "{}: duplicate param id {}", def.id, p.id);
                assert!(p.min < p.max && p.default >= p.min && p.default <= p.max, "{}.{} range/default", def.id, p.id);
                assert_eq!(p.snap(p.default), p.default, "{}.{} default must be a valid value", def.id, p.id);
                if p.taper == Taper::Log {
                    assert!(p.min > 0.0, "{}.{} log needs min > 0", def.id, p.id);
                }
                if p.is_discrete() {
                    assert_eq!(p.max as usize + 1, p.choices.len());
                }
            }
            for c in def.layout.controls() {
                for id in c.params() {
                    assert!(def.param(id).is_some(), "{}: layout names unknown param {id}", def.id);
                }
            }
            if let Some(View::Field(f)) = def.layout.view {
                for id in f.x.iter().chain(f.y) {
                    assert!(def.param(id).is_some(), "{}: field names unknown param {id}", def.id);
                }
            }
            for (p, by, values) in def.layout.shown_when {
                let b = def.param(by).unwrap_or_else(|| panic!("{}: shown_when names unknown param {by}", def.id));
                assert!(def.param(p).is_some() && values.iter().all(|v| (*v as f32) <= b.max), "{}: shown_when {p}", def.id);
            }
            for f in def.factory {
                for (id, v) in f.values {
                    let p = def.param(id).unwrap_or_else(|| panic!("{} preset {}: unknown {id}", def.id, f.name));
                    assert!(*v >= p.min && *v <= p.max, "{} preset {}: {id} out of range", def.id, f.name);
                }
            }
        }
    }

    #[test]
    fn normalisation_round_trips() {
        let c = comp();
        for p in c.params {
            for i in 0..=20 {
                let n = i as f32 / 20.0;
                let v = p.from_normalized(n);
                assert!(v >= p.min && v <= p.max);
                if !p.is_discrete() && p.step == 0.0 {
                    assert!((p.to_normalized(v) - n).abs() < 1e-4, "{} n={n}", p.id);
                }
            }
        }
        let attack = c.param("attack").unwrap();
        assert!((attack.from_normalized(0.5) - 1000f32.sqrt()).abs() < 1e-3, "log midpoint is the geometric mean");
        let style = c.param("style").unwrap();
        assert_eq!(style.from_normalized(0.34), 1.0);
        assert_eq!(style.from_normalized(0.9), 3.0);
    }

    #[test]
    fn formatting_and_parsing() {
        let c = comp();
        let th = c.param("threshold").unwrap();
        assert_eq!(th.format(-24.0), "\u{2212}24.0 dB");
        assert_eq!(th.format(0.0), "+0.0 dB");
        assert_eq!(th.parse("-12"), Some(-12.0));
        assert_eq!(th.parse("\u{2212}6.5 dB"), Some(-6.5));
        assert_eq!(th.parse("-100"), Some(-60.0), "clamped");
        assert_eq!(th.parse("loud"), None);
        let rel = c.param("release").unwrap();
        assert_eq!(rel.format(100.0), "100 ms");
        assert_eq!(rel.format(1500.0), "1500 ms");
        assert_eq!(c.param("attack").unwrap().format(10.0), "10 ms");
        assert_eq!(rel.parse("1.2 s"), Some(1200.0));
        assert_eq!(rel.parse("250ms"), Some(250.0));
        assert_eq!(c.param("ratio").unwrap().format(4.0), "4.0:1");
        assert_eq!(c.param("ratio").unwrap().parse("8:1"), Some(8.0));
        assert_eq!(c.param("mix").unwrap().format(100.0), "100%");
        let style = c.param("style").unwrap();
        assert_eq!(style.format(1.0), "Opto");
        assert_eq!(style.parse("punch"), Some(2.0));
    }

    #[test]
    fn presets_round_trip_and_sanitise() {
        let c = comp();
        let mut d = c.instantiate();
        d.params.insert("threshold".into(), -30.0);
        let p = DevicePreset::capture(c, &d, "Mine");
        assert_eq!(DevicePreset::from_json(c, &p.to_json()).unwrap(), p);
        let odd = r#"{"device":"compressor","name":"x","values":{"threshold":-500,"bogus":1}}"#;
        let q = DevicePreset::from_json(c, odd).unwrap();
        assert_eq!(q.values["threshold"], -60.0);
        assert!(!q.values.contains_key("bogus"));
        assert_eq!(q.values.len(), c.params.len());
        assert!(DevicePreset::from_json(c, r#"{"device":"gate","name":"x","values":{}}"#).is_err());
    }

    #[test]
    fn modulation_formats_follow_the_reference() {
        let rate = find("tremolo").unwrap().param("rate").unwrap();
        assert_eq!(rate.format(0.5), "0.50 Hz");
        assert_eq!(rate.format(5.0), "5.0 Hz");
        assert_eq!(rate.format(12.4), "12 Hz");
        let f = find("ring-mod").unwrap().param("frequency").unwrap();
        assert_eq!(f.format(440.0), "440 Hz");
        assert_eq!(f.format(2199.99), "2.2 kHz");
        assert_eq!(f.parse("2.5k"), Some(2500.0));
        assert_eq!(f.parse("80 Hz"), Some(80.0));
        let rect = find("ring-mod").unwrap().param("rectify").unwrap();
        assert_eq!(rect.format(0.0), "0%");
        assert_eq!(rect.format(35.0), "+35%");
        assert_eq!(rect.format(-100.0), "−100%");
        assert_eq!(rect.parse("+35%"), Some(35.0));
        assert_eq!(rect.parse("−20"), Some(-20.0));
        let fb = find("flanger").unwrap().param("feedback").unwrap();
        assert_eq!(fb.format(-45.0), "−45%");
        assert_eq!(fb.format(60.0), "60%");
        let st = find("tremolo").unwrap().param("stereo_phase").unwrap();
        assert_eq!(st.format(90.0), "90 deg");
        let sync = find("chorus").unwrap().param("rate_sync").unwrap();
        assert_eq!(sync.format(8.0), "1/4");
        assert_eq!(sync.parse("1/8T"), Some(12.0));
        assert_eq!(find("flanger").unwrap().param("depth").unwrap().format(3.0), "3.0 ms");
    }

    #[test]
    fn tempo_tables_line_up() {
        assert_eq!(DIVISIONS.len(), DIVISION_QUARTERS.len());
        assert_eq!(MOD_NOTES.len(), MOD_NOTE_QUARTERS.len());
        assert_eq!(DIVISIONS[DIVISION_QUARTER as usize], "1/4");
        assert!((synced_hz(DIVISION_QUARTERS[DIVISION_QUARTER as usize], 120.0) - 2.0).abs() < 1e-6);
        assert!((synced_hz(MOD_NOTE_QUARTERS[10], 0.0) - 2.0).abs() < 1e-6, "bpm falls back to 120");
    }

    #[test]
    fn preset_lock_and_reset_on_load() {
        let def = find("character-mod").unwrap();
        let mut d = def.instantiate();
        d.params.insert("seed".into(), 77.0);
        d.params.insert("mono_audition".into(), 1.0);
        let p = DevicePreset::capture(def, &d, "x");
        let load = p.load_values(def);
        assert!(!load.contains_key("seed"), "seed is preset-locked: the device keeps its own");
        assert_eq!(load["mono_audition"], 0.0, "audition switches reset on load");
        assert_eq!(load["model"], 2.0);
    }

    #[test]
    fn field_cards_size_to_the_view_row() {
        let w = |id: &str| find(id).unwrap().layout.width();
        assert_eq!(w("tremolo"), 457.0);
        assert_eq!(w("autopanner"), 417.0);
        assert_eq!(w("chorus"), 377.0);
        assert_eq!(w("flanger"), 388.0);
        assert_eq!(w("phaser"), 377.0);
        assert_eq!(w("ring-mod"), 390.0);
        assert_eq!(w("character-mod"), 442.0);
    }

    #[test]
    fn width_follows_clusters() {
        assert_eq!(comp().layout.width(), 521.0, "reference: [2,5] clusters -> 505 + 16");
    }

    #[test]
    fn modulation_is_normalized_and_clamped() {
        let p = comp().param("attack").unwrap();
        assert_eq!(modulate(p, 1000.0, 0.5), 1000.0);
        assert!((modulate(p, 1.0, 0.5) - 1000f32.sqrt()).abs() < 1e-2);
    }

    #[test]
    fn registry_lists_the_drive_and_imaging_devices() {
        let ids: Vec<_> = all().iter().map(|d| d.id).collect();
        assert_eq!(ids[..7], ["compressor", "distortion", "tapesaturation", "soft-clipper", "bitcrusher", "widener", "utility"]);
        assert_eq!(find("widener").unwrap().layout.width(), 388.0);
        assert_eq!(find("utility").unwrap().layout.width(), 221.5);
        for def in all() {
            assert!(!def.factory.is_empty(), "{} has factory presets", def.id);
        }
    }

    #[test]
    fn new_formats_print_and_parse() {
        let u = find("utility").unwrap();
        let bal = u.param("balance").unwrap();
        assert_eq!(bal.format(0.0), "C");
        assert_eq!(bal.format(-25.0), "25L");
        assert_eq!(bal.format(25.0), "25R");
        assert_eq!(bal.parse("25L"), Some(-25.0));
        assert_eq!(bal.parse("10 r"), Some(10.0));
        assert_eq!(bal.parse("c"), Some(0.0));
        assert_eq!(u.param("width").unwrap().format(100.0), "100%");
        let f = u.param("bass_mono_freq").unwrap();
        assert_eq!(f.format(120.0), "120 Hz");
        assert_eq!(f.parse("200 Hz"), Some(200.0));
        let inf: Vec<_> = all().iter().flat_map(|d| d.params.iter()).filter(|p| p.format == Format::DbOrInf).collect();
        assert!(!inf.is_empty());
        for p in inf {
            assert!(p.format(p.min).contains('\u{221e}'), "{} at min shows -inf", p.id);
            assert_eq!(p.parse("-inf"), Some(p.min));
        }
    }

    #[test]
    fn drawn_models_behave() {
        use super::{distortion::distortion_transfer, soft_clipper::clipper_transfer, widener::widener_response};
        for mode in 0..4 {
            assert!(distortion_transfer(0.0, 50.0, 40.0, 0.0, 100.0, 0.0, mode).abs() < 1e-6, "mode {mode}: silence stays silent (bias has no DC)");
            let (a, b) = (distortion_transfer(0.2, 50.0, 0.0, 0.0, 100.0, 0.0, mode), distortion_transfer(0.4, 50.0, 0.0, 0.0, 100.0, 0.0, mode));
            assert!(b >= a, "mode {mode} is monotonic");
        }
        assert!((distortion_transfer(0.3, 80.0, 0.0, 0.0, 0.0, 0.0, 0) - 0.3).abs() < 1e-6, "mix 0 is dry");
        for shape in 0..3 {
            for s in [0.0, 50.0, 100.0] {
                let y = clipper_transfer(4.0, 0.0, -6.0, s, shape, false, 100.0, 0.0);
                assert!(y <= 10f32.powf(-6.0 / 20.0) + 1e-4, "shape {shape} softness {s}: held under the ceiling ({y})");
            }
            assert!((clipper_transfer(0.01, 0.0, 0.0, 0.0, shape, false, 100.0, 0.0) - 0.01).abs() < 1e-3, "transparent well below the ceiling");
        }
        assert!(distortion_transfer(1.0, 100.0, 100.0, 0.0, 100.0, 0.0, 2).abs() < 2.0, "Tube stays bounded at full drive and bias");
        for shape in 0..3 {
            let y = clipper_transfer(1.0, 24.0, -12.0, 0.0, shape, false, 100.0, 0.0);
            assert!(y > 0.2 && y <= 10f32.powf(-12.0 / 20.0) + 1e-4, "shape {shape}: extreme drive keeps the ceiling ({y})");
        }
        let pic = super::bitcrusher::crush_picture(32.0, 2.5, 100.0, 0.0, 0.0);
        let changes = pic.windows(2).filter(|w| w[0].1 != w[1].1).count();
        assert!((changes as f32 - 480.0 / 2.5).abs() <= 2.0, "fractional downsample keeps its rate ({changes})");
        let sr = 48_000.0;
        let half = widener_response(120.0, 100.0, 0.0, 0.0, 120.0, 50.0, sr);
        assert!(half > 70.0 && half < 82.0, "partial mix sums the paths with their phase ({half})");
        assert!((widener_response(5000.0, 100.0, 0.0, 0.0, 20.0, 100.0, sr) - 100.0).abs() < 1.0, "100% is unchanged above the cut");
        assert!(widener_response(30.0, 200.0, 0.0, 0.0, 300.0, 100.0, sr) < widener_response(5000.0, 200.0, 0.0, 0.0, 300.0, 100.0, sr), "the low cut narrows the lows");
        assert!((widener_response(5000.0, 200.0, 0.0, 0.0, 20.0, 0.0, sr) - 100.0).abs() < 1e-3, "mix 0 is dry");
    }
}
