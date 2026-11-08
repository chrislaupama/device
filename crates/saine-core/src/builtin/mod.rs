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

mod compressor;
mod gate;
mod glue_compressor;
mod limiter;
mod mastering_limiter;
mod transient_shaper;

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
    /// Hertz below 1 kHz ("440 Hz"), kilohertz above ("2.50 kHz").
    Hz,
    /// Compression ratio: one decimal when the range spans less than 20 ("4.0:1"), else whole numbers ("10:1").
    Ratio,
    /// Milliseconds with a fixed number of decimals ("0.50 ms", "100 ms"); typed seconds are converted.
    Ms(u8),
    /// Whole percent with an explicit sign: "+35%", "−20%", "0%".
    SignedPercent,
    /// Percent, at most one decimal: "100%", "12.5%".
    Percent,
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
    /// One-sentence description (tooltips, the assistant).
    pub help: &'static str,
    /// Text shown instead of the number at the top of the range ("Full"); "" = none.
    pub max_text: &'static str,
    /// Discrete parameters: values written together with each choice, by
    /// choice index (one undo step), e.g. a style that also sets its times.
    pub overlays: &'static [&'static [(&'static str, f32)]],
}

impl ParamDef {
    /// A continuous parameter with defaults for the optional fields.
    #[allow(clippy::too_many_arguments)]
    pub const fn new(id: &'static str, name: &'static str, min: f32, max: f32, default: f32, taper: Taper, unit: &'static str, format: Format) -> ParamDef {
        ParamDef { id, name, label: name, min, max, default, taper, step: 0.0, unit, format, choices: &[], automatable: true, reset_on_load: false, help: "", max_text: "", overlays: &[] }
    }

    /// A discrete parameter: plain value = index into `choices`.
    pub const fn choice(id: &'static str, name: &'static str, choices: &'static [&'static str], default: u32) -> ParamDef {
        ParamDef { id, name, label: name, min: 0.0, max: (choices.len() - 1) as f32, default: default as f32, taper: LINEAR, step: 1.0, unit: "", format: Format::Choice, choices, automatable: true, reset_on_load: false, help: "", max_text: "", overlays: &[] }
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
    pub const fn max_text(mut self, text: &'static str) -> ParamDef {
        self.max_text = text;
        self
    }
    pub const fn overlays(mut self, overlays: &'static [&'static [(&'static str, f32)]]) -> ParamDef {
        self.overlays = overlays;
        self
    }

    /// The number a value stands for: the plain value, or for a discrete
    /// parameter the leading number of its choice name ("4:1" -> 4, "0.3 ms"
    /// -> 0.3); NaN for a choice without one ("Auto").
    pub fn number(&self, v: f32) -> f32 {
        if !self.is_discrete() {
            return v;
        }
        let name = self.choices.get(v.round().max(0.0) as usize).copied().unwrap_or("");
        let end = name.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(name.len());
        name[..end].parse().unwrap_or(f32::NAN)
    }

    fn at_max_text(&self, v: f32) -> bool {
        !self.max_text.is_empty() && v >= self.max - 5e-4 * (self.max - self.min)
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
        if self.step <= 0.0 || self.taper == Taper::Log {
            return v;
        }
        // In f64 on the decimal step (0.1, not 0.100000001), so 120 steps of 0.1 from -12 land on 0.
        let dec = |x: f32| (x as f64 * 1e6).round() / 1e6;
        let k = ((v - self.min) / self.step + 0.501).floor() as f64;
        ((((dec(self.min) + k * dec(self.step)) * 1e6).round() / 1e6) as f32).clamp(self.min, self.max)
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
        if self.at_max_text(v) {
            return self.max_text.to_string();
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
        if !self.max_text.is_empty() && t.eq_ignore_ascii_case(self.max_text) {
            return Some(self.max);
        }
        if self.is_discrete() {
            let lower = t.to_lowercase();
            if let Some(i) = self.choices.iter().position(|c| c.to_lowercase() == lower) {
                return Some(i as f32);
            }
        }
        let t = t.replace('\u{2212}', "-").replace(',', ".");
        let split = t.find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-' || c == '+')).unwrap_or(t.len());
        let (num, unit) = t.split_at(split);
        let mut v: f32 = num.parse().ok()?;
        let unit = unit.trim().to_lowercase();
        let unit = unit.trim_end_matches(":1").trim();
        match (self.format, unit) {
            (_, "") => {}
            (Format::Time | Format::Ms(_), "s" | "sec" | "secs" | "second" | "seconds") => v *= 1000.0,
            (Format::Time | Format::Ms(_), "ms" | "msec" | "msecs") => {}
            (Format::Hz, "k" | "khz") => v *= 1000.0,
            (Format::Hz, "hz") => {}
            (Format::Db, "db") | (Format::Percent | Format::SignedPercent, "%") => {}
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
        Format::Ratio => {
            let d = if range < 20.0 { 1 } else { 0 };
            format!("{v:.d$}:1")
        }
        Format::Ms(d) => format!("{v:.*} ms", d as usize),
        Format::SignedPercent => {
            let r = v.round();
            if r == 0.0 { "0%".into() } else if r > 0.0 { format!("+{r:.0}%") } else { format!("{MINUS}{:.0}%", -r) }
        }
        Format::Percent => {
            let r = (v * 10.0).round() / 10.0;
            if r.fract() == 0.0 { format!("{r:.0}%") } else { format!("{r:.1}%") }
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
    /// Two-state latch button (On/Off parameters). In the dial row it takes a
    /// knob cell (latch + caption); in the picker row it is a right-aligned
    /// pill reading "<label> <choice>" ("Auto Release On").
    Toggle(&'static str),
    /// Sidechain source picker (a track list, not a parameter).
    Sidechain,
}

impl Control {
    pub fn param(&self) -> Option<&'static str> {
        match *self {
            Control::Knob(p) | Control::Slider(p) | Control::Menu(p) | Control::Pill(p) | Control::Toggle(p) => Some(p),
            Control::Sidechain => None,
        }
    }
}

/// A visualiser or custom view drawn by the panel code.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum View {
    /// Scrolling level / gain-reduction strip with a draggable threshold
    /// line bound to `threshold` (reference dynamics strip), plot 0..-36 dB.
    /// Shorthand for `View::Strip(Strip::new(threshold))`.
    DynamicsStrip { threshold: &'static str },
    /// Dynamics strip with its own plot range and an optional second line.
    Strip(Strip),
    /// A device-specific view implemented in the panel code under this name
    /// (escape hatch for EQ curves, shaper, trance gate grid...).
    Custom(&'static str),
}

impl View {
    /// The dynamics strip of this view, if it is one.
    pub fn strip(&self) -> Option<Strip> {
        match *self {
            View::DynamicsStrip { threshold } => Some(Strip::new(threshold)),
            View::Strip(s) => Some(s),
            View::Custom(_) => None,
        }
    }
}

/// Dynamics strip settings: draggable lines (dB parameters, capped at 0 dB)
/// and the plot range of the well.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Strip {
    /// First line (dashed): the threshold or ceiling the strip edits.
    pub line: &'static str,
    /// Optional second line (solid); Alt+drag moves both by the same dB.
    pub second: Option<&'static str>,
    /// Plot top and floor (dB). Lines below the floor are drawn dotted at it.
    pub top_db: f32,
    pub floor_db: f32,
}

impl Strip {
    pub const fn new(line: &'static str) -> Strip {
        Strip { line, second: None, top_db: 0.0, floor_db: -36.0 }
    }
    pub const fn range(mut self, top_db: f32, floor_db: f32) -> Strip {
        self.top_db = top_db;
        self.floor_db = floor_db;
        self
    }
    pub const fn second(mut self, line: &'static str) -> Strip {
        self.second = Some(line);
        self
    }
}

/// Drawing inside a knob instead of the plain body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Face {
    /// Static transfer curve of a soft-knee compressor.
    SoftKneeCurve { threshold: &'static str, ratio: &'static str, knee: &'static str },
    /// Brick-wall limiter: output = min(input, ceiling).
    BrickWall { ceiling: &'static str },
    /// Downward expander / gate below the threshold.
    Expander { threshold: &'static str, ratio: &'static str },
    /// Bus VCA compressor: fixed 4 dB soft knee, ratio-dependent threshold
    /// offset, reduction limited by `range` (discrete ratios read as numbers).
    BusVca { threshold: &'static str, ratio: &'static str, range: &'static str },
    /// Mastering limiter "push": gain of ceiling - threshold below the
    /// threshold, pinned at the ceiling above it.
    PushWall { threshold: &'static str, ceiling: &'static str },
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
        let cells: usize = self.clusters.iter().map(|c| c.len()).sum();
        let inner: usize = self.clusters.iter().map(|c| c.len().saturating_sub(1)).sum();
        let gaps = self.clusters.len().saturating_sub(1);
        cells as f32 * KNOB_CELL + inner as f32 * KNOB_GAP + gaps as f32 * CLUSTER_GAP + CARD_PAD
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

/// Ids of the hidden switch that follows a device's sidechain picker.
pub const SIDECHAIN_ENABLE_IDS: [&str; 2] = ["sidechainEnable", "sidechain"];

/// The vendor shown for built-in devices.
pub const VENDOR: &str = "Saine Studio";

/// Every built-in device, in browser order.
pub fn all() -> &'static [&'static DeviceDef] {
    static ALL: [&DeviceDef; 6] = [
        &compressor::COMPRESSOR,
        &glue_compressor::GLUE_COMPRESSOR,
        &limiter::LIMITER,
        &mastering_limiter::MASTERING_LIMITER,
        &gate::GATE,
        &transient_shaper::TRANSIENT_SHAPER,
    ];
    &ALL
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

    /// Values a choice writes: the parameter itself plus its overlay for that
    /// choice (`ParamDef::overlays`), all snapped. Empty if `param` is unknown.
    pub fn choice_values(&self, param: &str, choice: f32) -> BTreeMap<String, f32> {
        let mut out = BTreeMap::new();
        let Some(p) = self.param(param) else { return out };
        let choice = p.snap(choice);
        out.insert(p.id.to_string(), choice);
        for (id, v) in p.overlays.get(choice.round() as usize).copied().unwrap_or(&[]) {
            if let Some(q) = self.param(id) {
                out.insert((*id).to_string(), q.snap(*v));
            }
        }
        out
    }

    /// The parameter that follows the sidechain picker (on while a source is picked).
    pub fn sidechain_enable(&self) -> Option<&'static ParamDef> {
        self.params.iter().find(|p| SIDECHAIN_ENABLE_IDS.contains(&p.id))
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

    /// Values to apply when loading: `reset_on_load` parameters go back to their default.
    pub fn load_values(&self, def: &DeviceDef) -> BTreeMap<String, f32> {
        let mut v = self.values.clone();
        for p in def.params.iter().filter(|p| p.reset_on_load) {
            v.insert(p.id.into(), p.default);
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
                if let Some(id) = c.param() {
                    assert!(def.param(id).is_some(), "{}: layout names unknown param {id}", def.id);
                }
            }
            for f in def.factory {
                for (id, v) in f.values {
                    let p = def.param(id).unwrap_or_else(|| panic!("{} preset {}: unknown {id}", def.id, f.name));
                    assert!(*v >= p.min && *v <= p.max, "{} preset {}: {id} out of range", def.id, f.name);
                }
            }
            for p in def.params {
                assert!(p.overlays.is_empty() || p.overlays.len() == p.choices.len(), "{}.{}: one overlay per choice", def.id, p.id);
                for (id, v) in p.overlays.iter().flat_map(|o| o.iter()) {
                    let q = def.param(id).unwrap_or_else(|| panic!("{}.{} overlay names unknown {id}", def.id, p.id));
                    assert_eq!(q.snap(*v), *v, "{}.{} overlay {id}={v} must be a valid value", def.id, p.id);
                }
            }
            assert!(def.layout.width() > 0.0);
            if let Some(s) = def.layout.view.and_then(|v| v.strip()) {
                for line in std::iter::once(s.line).chain(s.second) {
                    let p = def.param(line).unwrap_or_else(|| panic!("{}: strip line {line} unknown", def.id));
                    assert!(p.max <= 0.0, "{}: strip lines are capped at 0 dB", def.id);
                }
                assert!(s.top_db > s.floor_db);
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
    fn width_follows_clusters() {
        assert_eq!(comp().layout.width(), 521.0, "reference: [2,5] clusters -> 505 + 16");
    }

    #[test]
    fn dynamics_formats() {
        let gate = find("gate").unwrap();
        assert_eq!(gate.param("ratio").unwrap().format(10.0), "10:1", "ranges of 20 or more print whole ratios");
        assert_eq!(gate.param("attack").unwrap().format(0.5), "0.50 ms");
        assert_eq!(gate.param("release").unwrap().format(100.0), "100.0 ms");
        assert_eq!(gate.param("hold").unwrap().format(50.0), "50 ms");
        assert_eq!(gate.param("attack").unwrap().parse("0.02 s"), Some(20.0));
        assert_eq!(gate.param("sidechain_hpf").unwrap().format(80.0), "80 Hz");
        let ts = find("transient-shaper").unwrap();
        let attack = ts.param("attack").unwrap();
        assert_eq!(attack.format(35.0), "+35%");
        assert_eq!(attack.format(-20.0), "\u{2212}20%");
        assert_eq!(attack.format(0.2), "0%");
        assert_eq!(attack.parse("-40%"), Some(-40.0));
        let sus = ts.param("sustain_time").unwrap();
        assert_eq!(sus.format(2000.0), "Full");
        assert_eq!(sus.format(sus.from_normalized(0.9999)), "Full");
        assert_eq!(sus.format(150.0), "150 ms");
        assert_eq!(sus.parse("full"), Some(2000.0));
        assert_eq!(ts.param("sensitivity").unwrap().label, "Sense");
        let ml = find("mastering-limiter").unwrap();
        assert_eq!(ml.param("threshold").unwrap().format(0.0), "0.0 dB");
        assert_eq!(ml.param("ceiling").unwrap().format(-1.0), "\u{2212}1.0 dB");
        assert_eq!(ml.param("release").unwrap().format(150.0), "150 ms");
        assert!((ml.param("release").unwrap().to_normalized(150.0) - 0.6223).abs() < 1e-3, "skew 0.25");
        assert!((ml.param("attack").unwrap().to_normalized(10.0) - 0.5012).abs() < 1e-3, "skew 0.3");
        let glue = find("glue-compressor").unwrap();
        assert_eq!(glue.param("release").unwrap().format(4.0), "Auto");
        assert_eq!(glue.param("range").unwrap().format(24.0), "24.0 dB");
        assert_eq!(glue.param("ratio").unwrap().number(2.0), 10.0);
        assert_eq!(glue.param("attack").unwrap().number(1.0), 0.3);
        assert!(glue.param("release").unwrap().number(4.0).is_nan());
        assert_eq!(glue.param("ratio").unwrap().parse("10:1"), Some(2.0));
    }

    #[test]
    fn choice_overlays_write_their_values() {
        let ml = find("mastering-limiter").unwrap();
        let v = ml.choice_values("style", 3.0);
        assert_eq!(v["style"], 3.0);
        assert_eq!(v["lookahead"], 3.0);
        assert_eq!(v["attack"], 30.0);
        assert_eq!(v["auto_release"], 0.0);
        assert_eq!(v.len(), 7);
        let comp = comp();
        assert_eq!(comp.choice_values("style", 2.0).len(), 1, "no overlays: only the choice");
        assert_eq!(find("glue-compressor").unwrap().sidechain_enable().map(|p| p.id), Some("sidechain"));
        assert_eq!(comp.sidechain_enable().map(|p| p.id), Some("sidechainEnable"));
    }

    #[test]
    fn dynamics_card_widths_follow_the_reference() {
        for (id, w) in [("limiter", 471.0), ("gate", 543.0), ("glue-compressor", 615.0), ("transient-shaper", 543.0), ("mastering-limiter", 388.0)] {
            assert_eq!(find(id).unwrap().layout.width(), w, "{id}");
        }
    }

    #[test]
    fn modulation_is_normalized_and_clamped() {
        let p = comp().param("attack").unwrap();
        assert_eq!(modulate(p, 1000.0, 0.5), 1000.0);
        assert!((modulate(p, 1.0, 0.5) - 1000f32.sqrt()).abs() < 1e-2);
    }
}
