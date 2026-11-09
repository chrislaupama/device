//! Automation lanes (ADR 0005, reference/ui-inventory.md §12).
//!
//! UI state only lives here: whether lanes are shown, Draw mode, the Automation Arm, which
//! parameter each track shows (NOT saved: `AutoUi::targets`), the expanded tracks, the
//! parameter picker, the point selection and in-progress gestures. Every edit is ONE
//! `Command::SetAutomationLane` committed on release (write passes: ONE `Batch` "Write
//! Automation"); drags preview through `AutoUi::pending`, which both the renderer and the
//! engine snapshot (`Controller::current_snapshot`) use instead of the model lane.
//!
//! Layout (our design where the reference shows nothing): the shown lane of a track is drawn
//! OVER the track lane (as in 60-automation-lanes.png). The "Show N automated lanes"
//! disclosure expands the track's OTHER lanes that have points as extra rows of
//! `EXTRA_H` below it (pill with power toggle + parameter; clicking the name makes it the
//! shown lane). The master row grows to `MASTER_H_AUTO` and has a volume lane.
//!
//! Vertical mapping (ours, fitted to the reference: 0 dB at 0.557 of the lane height from the
//! top in both the 74 pt track rows and the 63.5 pt master row): `y = top + h·(PAD_T + (1 −
//! norm)·(1 − PAD_T − PAD_B))`, where volume `norm` is linear amplitude over 0..+6 dB, pan
//! is linear, synth parameters linear (geometric for cutoff / LFO rate) and plugin
//! parameters their normalized value.

use crate::controller::{Controller, fmt_db, fmt_pan, slider_from_gain, sync_model};
use crate::timeline::{self, TRACK_H, TRACK_PITCH};
use crate::{AutoExtra, AutoLabel, MenuItem, Studio, TrackRow};
use saine_core::automation::{AutoPoint, AutomationLane, AutomationTarget, SynthParam, VOLUME_MAX_DB, VOLUME_MIN_DB};
use saine_core::{Command, PPQ, Ticks, TrackId, TrackKind, db_to_gain, gain_to_db};
use slint::Model;
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

/// `None` = the master bus.
pub type LaneKey = Option<TrackId>;

/// Extra (expanded) automation rows below a track.
pub const EXTRA_H: f32 = 40.0;
pub const EXTRA_PITCH: f32 = 40.5;
/// Fractions of the row height above the top / below the bottom of the value range.
const PAD_T: f32 = 0.12;
const PAD_B: f32 = 0.006;
/// Hit radius (points) for handles and the line.
const HIT: f32 = 5.0;
const DOUBLE_CLICK: Duration = Duration::from_millis(400);
/// Freehand drawing and write passes drop points within this fraction of the value range of the
/// line through their kept neighbours.
pub const THIN_TOLERANCE: f32 = 0.004;
/// Plugin parameter changes without gesture boundaries end a write gesture after this idle time.
const PLUGIN_IDLE: Duration = Duration::from_millis(400);

/// Dimming over track lanes while automation is shown (fitted per channel to 60-*.png: lane
/// #16171a → #171819, MIDI clip body #8092eb → #3a405d).
pub const OVERLAY: [f32; 4] = [23.5 / 255.0, 24.4 / 255.0, 24.5 / 255.0, 0.672];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineStyle {
    /// Enabled lane with points: the parameter's bright hue.
    Active,
    /// No lane / no points yet: the parameter's dim hue.
    Inactive,
    /// Lane switched off with the power toggle (ORIGINAL grey).
    Disabled,
}

fn hex(h: u32, a: f32) -> [f32; 4] {
    let c = timeline::rgb(h);
    [c[0], c[1], c[2], a]
}

/// Lane colour of an automated parameter (reference/ui-inventory.md §12, research-findings.md).
/// The reference keys every parameter as "<device>/<parameter>" (track parameters use the device
/// "Channel"): a few track keys have fixed colours (Volume gold), everything else takes
/// `PALETTE[fnv1a32(key) % 8]`. Each hue has a bright form (lane enabled) and a dim form (no
/// points / power off), both measured from 2x captures of the reference's power-on and
/// power-off lanes (the reference darkens its base palette on the timeline).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hue {
    Gold,
    Blue,
    Magenta,
    Green,
    Purple,
    Orange,
    Coral,
    Teal,
    Indigo,
}

impl Hue {
    /// The hashed palette, in the reference's order (Gold is reserved for Volume).
    pub const PALETTE: [Hue; 8] = [Hue::Blue, Hue::Magenta, Hue::Green, Hue::Purple, Hue::Orange, Hue::Coral, Hue::Teal, Hue::Indigo];

    /// (bright, dim) as 0xRRGGBB.
    pub fn hex(self) -> (u32, u32) {
        match self {
            Hue::Gold => (0xebc542, 0x61551f),
            Hue::Blue => (0x65bce3, 0x30525f),
            Hue::Magenta => (0xcb62b3, 0x55314e),
            Hue::Green => (0x7aca8a, 0x395641),
            Hue::Purple => (0x8f78e4, 0x41395f),
            Hue::Orange => (0xdd9336, 0x5c431d),
            Hue::Indigo => (0x6285ec, 0x313f62),
            Hue::Teal => (0x65c9b8, 0x2f5651),
            Hue::Coral => (0xd56e5f, 0x593531),
        }
    }

    /// Hue of a parameter. `device` is the display name of the plugin that owns a `Plugin`
    /// target (ignored otherwise); the same device + parameter name is the same colour on every
    /// track.
    pub fn of(target: &AutomationTarget, device: &str) -> Hue {
        match target {
            AutomationTarget::Volume => Hue::key("Channel", "Volume"),
            AutomationTarget::Pan => Hue::key("Channel", "Pan"),
            AutomationTarget::Synth { param } => Hue::key("Synth", param.name()),
            AutomationTarget::Plugin { name, .. } => Hue::key(device, name),
            AutomationTarget::Builtin { .. } => {
                let (device, param) = target.labels();
                Hue::key(&device, &param)
            }
        }
    }

    /// Colour of the key "<device>/<parameter>".
    pub fn key(device: &str, param: &str) -> Hue {
        if device == "Channel" {
            // Fixed track-parameter colours (Polarity: coral, we have no polarity parameter).
            match param {
                "Volume" => return Hue::Gold,
                "Pan" => return Hue::Blue,
                "Mute" => return Hue::Magenta,
                "Trim" => return Hue::Green,
                "Width" => return Hue::Purple,
                "Mono" => return Hue::Orange,
                "Polarity" => return Hue::Coral,
                _ => {}
            }
        }
        // 32-bit FNV-1a over the UTF-16 code units (the reference hashes JavaScript strings).
        let key = format!("{device}/{param}");
        let h = key.encode_utf16().fold(0x811c_9dc5u32, |h, u| (h ^ u as u32).wrapping_mul(0x0100_0193));
        Hue::PALETTE[(h % Hue::PALETTE.len() as u32) as usize]
    }

    /// Bright form (enabled lane with points) as linear RGBA for the renderer.
    pub fn bright(self) -> [f32; 4] {
        hex(self.hex().0, 1.0)
    }

    /// Dim form (no points yet) as linear RGBA for the renderer.
    pub fn dim(self) -> [f32; 4] {
        hex(self.hex().1, 1.0)
    }

    /// Bright form for Slint (labels, header pill, power icon).
    pub fn slint_bright(self) -> slint::Color {
        let h = self.hex().0;
        slint::Color::from_rgb_u8((h >> 16) as u8, (h >> 8) as u8, h as u8)
    }

    /// Dim form for Slint (value labels of lanes without points).
    pub fn slint_dim(self) -> slint::Color {
        let h = self.hex().1;
        slint::Color::from_rgb_u8((h >> 16) as u8, (h >> 8) as u8, h as u8)
    }
}

impl LineStyle {
    /// Line colour of a lane in this state with the parameter's hue (grey while disabled).
    pub fn line(self, hue: Hue) -> [f32; 4] {
        match self {
            LineStyle::Active => hue.bright(),
            LineStyle::Inactive => hue.dim(),
            LineStyle::Disabled => hex(0x55565a, 1.0),
        }
    }
    /// State code for Slint (`TrackRow.auto-state`: 0 inactive, 1 active, 2 disabled).
    pub fn code(self) -> i32 {
        match self {
            LineStyle::Inactive => 0,
            LineStyle::Active => 1,
            LineStyle::Disabled => 2,
        }
    }
}

/// One automation row in lane-local coordinates (see timeline.rs).
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub key: LaneKey,
    pub target: AutomationTarget,
    pub y0: f32,
    pub h: f32,
    pub master: bool,
    pub extra: bool,
}

impl Row {
    pub fn y_of(&self, norm: f32) -> f32 {
        self.y0 + self.h * (PAD_T + (1.0 - norm.clamp(0.0, 1.0)) * (1.0 - PAD_T - PAD_B))
    }
    pub fn norm_at(&self, y: f32) -> f32 {
        (1.0 - ((y - self.y0) / self.h - PAD_T) / (1.0 - PAD_T - PAD_B)).clamp(0.0, 1.0)
    }
    fn contains(&self, y: f32) -> bool {
        y >= self.y0 && y < self.y0 + self.h
    }
}

/// Per-row draw data for `timeline::build` (logical lane coordinates).
#[derive(Debug, Clone)]
pub struct RowDraw {
    pub y0: f32,
    pub h: f32,
    pub master: bool,
    pub overlay: bool,
    pub extra: bool,
    pub style: LineStyle,
    pub hue: Hue,
    pub poly: Vec<(f32, f32)>,
    pub handles: Vec<(f32, f32, bool)>,
    pub start_dot: Option<(f32, f32)>,
}

#[derive(Debug, Clone, Default)]
pub struct AutoScene {
    pub rows: Vec<RowDraw>,
    pub marquee: Option<[f32; 4]>,
}

// ------------------------------------------------------------------ value mapping

/// Lane value → 0 (bottom) .. 1 (top).
pub fn norm_of(t: &AutomationTarget, v: f32) -> f32 {
    let n = match t {
        AutomationTarget::Volume => {
            if v <= VOLUME_MIN_DB { 0.0 } else { db_to_gain(v) / db_to_gain(VOLUME_MAX_DB) }
        }
        AutomationTarget::Pan => (v + 1.0) / 2.0,
        AutomationTarget::Synth { param } => {
            let (lo, hi) = param.range();
            if param.logarithmic() { (v.max(lo) / lo).ln() / (hi / lo).ln() } else { (v - lo) / (hi - lo) }
        }
        AutomationTarget::Plugin { .. } => v,
        AutomationTarget::Builtin { .. } => t.builtin_param().map_or(v, |p| p.to_normalized(v)),
    };
    n.clamp(0.0, 1.0)
}

/// 0..1 → lane value (inside the target's range).
pub fn value_of(t: &AutomationTarget, n: f32) -> f32 {
    let n = n.clamp(0.0, 1.0);
    let v = match t {
        AutomationTarget::Volume => {
            let a = n * db_to_gain(VOLUME_MAX_DB);
            if a <= db_to_gain(VOLUME_MIN_DB) { VOLUME_MIN_DB } else { (gain_to_db(a) * 100.0).round() / 100.0 }
        }
        AutomationTarget::Pan => ((n * 2.0 - 1.0) * 1000.0).round() / 1000.0,
        AutomationTarget::Synth { param } => {
            let (lo, hi) = param.range();
            if param.logarithmic() { lo * (hi / lo).powf(n) } else { lo + (hi - lo) * n }
        }
        AutomationTarget::Plugin { .. } => n,
        AutomationTarget::Builtin { .. } => t.builtin_param().map_or(n, |p| p.snap(p.from_normalized(n))),
    };
    let (lo, hi) = t.range();
    v.clamp(lo, hi)
}

/// Lane value label ("+0.0" dB, "C"/"50L", synth units, plugin text).
pub fn volume_label(db: f32) -> String {
    if db <= VOLUME_MIN_DB + 1e-3 {
        return "−∞".into();
    }
    let r = (db * 10.0).round() / 10.0;
    if r >= 0.0 { format!("+{:.1}", r.abs()) } else { format!("−{:.1}", -r) }
}

/// Keep points within `tol` (fraction of 0..1) of the line through their kept neighbours out.
/// Input is (time, norm) sorted by time; first and last are always kept.
pub fn thin(points: &[(Ticks, f32)], tol: f32) -> Vec<(Ticks, f32)> {
    if points.len() <= 2 {
        return points.to_vec();
    }
    let mut out = vec![points[0]];
    for i in 1..points.len() - 1 {
        let a = *out.last().unwrap();
        let (b, c) = (points[i], points[i + 1]);
        let line = if c.0 == a.0 { a.1 } else { a.1 + (c.1 - a.1) * (b.0 - a.0) as f32 / (c.0 - a.0) as f32 };
        if (b.1 - line).abs() > tol || b.0 == a.0 {
            out.push(b);
        }
    }
    out.push(points[points.len() - 1]);
    // Same-time duplicates keep only the last value (vertical steps stay as two points).
    out.dedup_by(|b, a| b.0 == a.0 && (b.1 - a.1).abs() < 1e-6);
    out
}

/// "Touch" merge: replace `base` points in `start..=end` by `new` (plain values), keeping the
/// original curve outside the range (its values at `start` / `end` are pinned so playback
/// returns to it). With an empty `base` the new points are the lane.
pub fn touch_merge(base: &AutomationLane, start: Ticks, end: Ticks, new: &[(Ticks, f32)]) -> AutomationLane {
    let mut lane = base.clone();
    if new.is_empty() {
        return lane;
    }
    let had = !base.points.is_empty();
    let before = base.value_at(start);
    let after = base.value_at(end);
    let mut pts: Vec<AutoPoint> = base.points.iter().filter(|p| p.time < start).copied().collect();
    if had && let Some(v) = before {
        pts.push(AutoPoint { time: start, value: v, curve: 0.0 });
    }
    pts.extend(new.iter().map(|&(time, value)| AutoPoint { time, value, curve: 0.0 }));
    if had && let Some(v) = after {
        pts.push(AutoPoint { time: end, value: v, curve: 0.0 });
    }
    pts.extend(base.points.iter().filter(|p| p.time > end).copied());
    pts.sort_by_key(|p| p.time);
    lane.points = pts;
    lane
}

// ------------------------------------------------------------------ state

#[derive(Debug, Clone)]
pub struct Picker {
    pub key: LaneKey,
    /// Window position of the popup's top-left corner.
    pub pos: (f32, f32),
    pub query: String,
    /// Open category submenu.
    pub sub: Option<usize>,
    /// Targets of the rows currently listed (search results or the open submenu).
    rows: Vec<Option<AutomationTarget>>,
    sub_rows: Vec<Option<AutomationTarget>>,
}

#[derive(Debug, Clone)]
enum Gesture {
    Points { row: Row, base: AutomationLane, idx: Vec<usize>, grab: (f32, f32), grab_time: Ticks, created: bool, alt: bool },
    Curve { row: Row, base: AutomationLane, seg: usize, y0: f32, c0: f32, dir: f32 },
    Marquee { row: Row, x0: f32, y0: f32, x1: f32, y1: f32, add: Vec<usize> },
    Freehand { row: Row, base: AutomationLane, samples: Vec<(Ticks, f32)>, alt: bool },
}

#[derive(Debug, Clone)]
pub struct WritePass {
    pub key: LaneKey,
    pub target: AutomationTarget,
    pub start: Ticks,
    pub samples: Vec<(Ticks, f32)>,
    base: AutomationLane,
    last: Instant,
}

#[derive(Debug, Default)]
pub struct AutoUi {
    /// Timeline header "Show/Hide automation lanes".
    pub shown: bool,
    /// Timeline header "Draw mode (B)".
    pub draw_mode: bool,
    /// Transport "Automation Arm".
    pub armed: bool,
    /// Parameter shown per track (UI state, not saved). Default: Track › Volume.
    pub targets: HashMap<LaneKey, AutomationTarget>,
    pub expanded: HashSet<TrackId>,
    pub picker: Option<Picker>,
    /// Selected points: lane and point indices.
    pub sel: Option<(LaneKey, AutomationTarget, Vec<usize>)>,
    gesture: Option<Gesture>,
    /// Lane preview during a drag or write pass (renderer + engine use it instead of the model).
    pub pending: Option<(LaneKey, AutomationLane)>,
    pub writes: Vec<WritePass>,
    last_down: Option<(Instant, LaneKey, AutomationTarget, usize)>,
    /// Write passes committed (counter for tests / diagnostics).
    pub writes_committed: usize,
    /// Display names of every plugin device the project has had (`remember_device_names`, after
    /// each change), so a lane whose plugin was removed (its lanes stay in the project) keeps the
    /// colour it had. Device ids are not reused within a project; cleared with the project.
    device_names: std::cell::RefCell<HashMap<saine_core::DeviceId, String>>,
}

impl AutoUi {
    /// A different project: per-track UI state no longer applies.
    pub fn reset_project_state(&mut self) {
        self.targets.clear();
        self.device_names.borrow_mut().clear();
        self.expanded.clear();
        self.picker = None;
        self.sel = None;
        self.gesture = None;
        self.pending = None;
        self.writes.clear();
    }
}

#[derive(Debug, Clone)]
struct PickEntry {
    label: String,
    target: Option<AutomationTarget>,
}

// ------------------------------------------------------------------ controller glue

impl Controller {
    pub fn auto_target(&self, key: LaneKey) -> AutomationTarget {
        self.auto.targets.get(&key).cloned().unwrap_or(AutomationTarget::Volume)
    }

    /// Model lane for a target (ignores the preview).
    pub fn model_lane(&self, key: LaneKey, target: &AutomationTarget) -> Option<&AutomationLane> {
        let p = self.project();
        let lanes = match key {
            None => &p.master_automation,
            Some(t) => &p.track(t)?.automation,
        };
        lanes.iter().find(|l| l.target == *target)
    }

    /// Lane as displayed / played: the preview while a gesture runs, else the model.
    pub fn lane(&self, key: LaneKey, target: &AutomationTarget) -> Option<AutomationLane> {
        if let Some((k, l)) = &self.auto.pending
            && *k == key
            && l.target == *target
        {
            return Some(l.clone());
        }
        self.model_lane(key, target).cloned()
    }

    /// The parameter's static value (what plays without automation).
    pub fn static_value(&self, key: LaneKey, target: &AutomationTarget) -> f32 {
        let p = self.project();
        let clamp_db = |g: f32| gain_to_db(g).clamp(VOLUME_MIN_DB, VOLUME_MAX_DB);
        let Some(t) = key.and_then(|k| p.track(k)) else {
            return clamp_db(p.master_gain);
        };
        match target {
            AutomationTarget::Volume => clamp_db(t.gain),
            AutomationTarget::Pan => t.pan,
            AutomationTarget::Synth { param } => param.get(&t.synth),
            AutomationTarget::Plugin { device, param, .. } => self
                .engine
                .plugin_param_value(*device, *param)
                .or_else(|| self.engine.plugin_params(*device).iter().find(|i| i.id == *param).map(|i| i.default_normalized))
                .unwrap_or(0.0)
                .clamp(0.0, 1.0),
            AutomationTarget::Builtin { device, param, .. } => t
                .devices
                .iter()
                .find(|d| d.id == *device)
                .and_then(|d| saine_core::builtin::def_of(d).and_then(|def| def.param(param).map(|q| def.value(d, q))))
                .or_else(|| target.builtin_param().map(|q| q.default))
                .unwrap_or(0.0),
        }
    }

    /// Record the display name of every plugin device in the project. Called after every change
    /// (`Controller::after_change`, which also runs on load), so a device is always recorded
    /// before any later edit can remove it.
    pub fn remember_device_names(&self) {
        let mut names = self.auto.device_names.borrow_mut();
        for d in self.project().tracks.iter().flat_map(|t| t.instrument_plugin.iter().chain(t.devices.iter())) {
            names.entry(d.id).or_insert_with(|| d.plugin.name.clone());
        }
    }

    /// Lane colour of a target (`Hue::of` with the owning plugin's display name, which outlives
    /// the plugin's removal, see `remember_device_names`).
    pub fn lane_hue(&self, target: &AutomationTarget) -> Hue {
        let AutomationTarget::Plugin { device: id, .. } = target else {
            return Hue::of(target, "");
        };
        let names = self.auto.device_names.borrow();
        Hue::of(target, names.get(id).map_or("", String::as_str))
    }

    /// Drawing state of a lane: inactive without points, disabled when switched off, else active.
    pub fn auto_style(&self, key: LaneKey, target: &AutomationTarget) -> LineStyle {
        match self.lane(key, target) {
            Some(l) if l.points.is_empty() => LineStyle::Inactive,
            Some(l) if !l.enabled => LineStyle::Disabled,
            Some(_) => LineStyle::Active,
            None => LineStyle::Inactive,
        }
    }

    /// Automated value at the audible position (enabled lane with points), else None.
    pub fn auto_value_now(&self, key: LaneKey, target: &AutomationTarget) -> Option<f32> {
        let l = self.lane(key, target)?;
        if !l.enabled {
            return None;
        }
        l.value_at(self.playhead_ticks())
    }

    pub fn auto_label(&self, key: LaneKey, target: &AutomationTarget, v: f32) -> String {
        match target {
            AutomationTarget::Volume => volume_label(v),
            AutomationTarget::Pan => fmt_pan(v),
            AutomationTarget::Synth { .. } => format!("{v:.2}"),
            AutomationTarget::Plugin { device, param, .. } => {
                let _ = key;
                self.engine.plugin_param_text(*device, *param, v).unwrap_or_else(|| format!("{:.0}%", v * 100.0))
            }
            AutomationTarget::Builtin { .. } => target.builtin_param().map_or_else(|| format!("{v:.2}"), |p| p.format(v)),
        }
    }

    /// Other lanes of a track with points (the expandable "automated lanes").
    pub fn extra_targets(&self, track: TrackId) -> Vec<AutomationTarget> {
        let shown = self.auto_target(Some(track));
        self.project()
            .track(track)
            .map(|t| t.automation.iter().filter(|l| !l.points.is_empty() && l.target != shown).map(|l| l.target.clone()).collect())
            .unwrap_or_default()
    }

    /// Mirror the row layout into the view (expanded rows, master height).
    pub fn auto_sync_view(&mut self) {
        let extra: Vec<f32> = if self.auto.shown {
            let ids: Vec<TrackId> = self.project().tracks.iter().map(|t| t.id).collect();
            ids.iter().map(|id| if self.auto.expanded.contains(id) { self.extra_targets(*id).len() as f32 * EXTRA_PITCH } else { 0.0 }).collect()
        } else {
            Vec::new()
        };
        self.view.extra = extra;
        self.view.master_h = if self.auto.shown { timeline::MASTER_H_AUTO } else { timeline::MASTER_H };
    }

    /// All automation rows (lane-local), track rows first, master last.
    pub fn auto_rows(&self) -> Vec<Row> {
        if !self.auto.shown {
            return Vec::new();
        }
        let v = &self.view;
        let mut rows = Vec::new();
        for (i, t) in self.project().tracks.iter().enumerate() {
            let key = Some(t.id);
            let top = v.track_top(i);
            rows.push(Row { key, target: self.auto_target(key), y0: top, h: TRACK_H, master: false, extra: false });
            if self.auto.expanded.contains(&t.id) {
                for (k, target) in self.extra_targets(t.id).into_iter().enumerate() {
                    rows.push(Row { key, target, y0: top + TRACK_PITCH + k as f32 * EXTRA_PITCH, h: EXTRA_H, master: false, extra: true });
                }
            }
        }
        let h = self.lanes_size.1;
        rows.push(Row { key: None, target: AutomationTarget::Volume, y0: h - v.master_h + 1.0, h: v.master_h - 1.0, master: true, extra: false });
        rows
    }

    fn sel_indices(&self, row: &Row) -> Vec<usize> {
        match &self.auto.sel {
            Some((k, t, idx)) if *k == row.key && *t == row.target => idx.clone(),
            _ => Vec::new(),
        }
    }

    /// Draw data for the renderer (None while lanes are hidden).
    pub fn auto_scene(&self, scale: f32) -> Option<AutoScene> {
        if !self.auto.shown {
            return None;
        }
        let v = &self.view;
        let w = self.lanes_size.0;
        let step = 1.0 / scale.max(1.0);
        let mut out = AutoScene::default();
        for row in self.auto_rows() {
            let lane = self.lane(row.key, &row.target);
            let stat = self.static_value(row.key, &row.target);
            let style = self.auto_style(row.key, &row.target);
            let value_at = |t: Ticks| lane.as_ref().and_then(|l| l.value_at(t)).unwrap_or(stat);
            let mut poly = Vec::with_capacity((w / step) as usize + 2);
            let mut x = 0.0f32;
            while x <= w + step {
                let t = (v.beat_at(x) * PPQ as f64).round() as Ticks;
                poly.push((x, row.y_of(norm_of(&row.target, value_at(t)))));
                x += step;
            }
            let sel = self.sel_indices(&row);
            let handles = lane
                .as_ref()
                .map(|l| {
                    l.points
                        .iter()
                        .enumerate()
                        .filter_map(|(i, p)| {
                            let x = v.x_of(p.time as f64 / PPQ as f64);
                            (x > -4.0 && x < w + 4.0).then(|| (x, row.y_of(norm_of(&row.target, p.value)), sel.contains(&i)))
                        })
                        .collect()
                })
                .unwrap_or_default();
            let empty = lane.as_ref().is_none_or(|l| l.points.is_empty());
            let x0 = v.x_of(0.0);
            let start_dot = (empty && x0 > -3.0 && x0 < w).then(|| (x0, row.y_of(norm_of(&row.target, stat))));
            out.rows.push(RowDraw { y0: row.y0, h: row.h, master: row.master, overlay: !row.master, extra: row.extra, style, hue: self.lane_hue(&row.target), poly, handles, start_dot });
        }
        if let Some(Gesture::Marquee { x0, y0, x1, y1, .. }) = &self.auto.gesture {
            out.marquee = Some([x0.min(*x1), y0.min(*y1), (x1 - x0).abs(), (y1 - y0).abs()]);
        }
        Some(out)
    }

    /// Value labels at the left of every visible row (Slint text over the texture).
    pub fn auto_labels(&self) -> Vec<AutoLabel> {
        let v = &self.view;
        let bottom = self.lanes_size.1 - v.master_h;
        let t0 = (v.beat_at(0.0) * PPQ as f64).round() as Ticks;
        self.auto_rows()
            .into_iter()
            .filter(|r| r.master || (r.y0 >= timeline::RULER_H - 1.0 && r.y0 + 12.0 <= bottom))
            .map(|r| {
                let val = self.lane(r.key, &r.target).and_then(|l| l.value_at(t0)).unwrap_or_else(|| self.static_value(r.key, &r.target));
                let ly = r.y_of(norm_of(&r.target, val));
                let y = (ly - 9.5 - 6.5).clamp(r.y0 + 1.0, r.y0 + r.h - 14.0);
                {
                    let hue = self.lane_hue(&r.target);
                    AutoLabel {
                        x: 1.5,
                        y,
                        text: self.auto_label(r.key, &r.target, val).into(),
                        state: self.auto_style(r.key, &r.target).code(),
                        accent: hue.slint_bright(),
                        dim: hue.slint_dim(),
                    }
                }
            })
            .collect()
    }

    // ---------------------------------------------------------------- header toggles

    pub fn auto_toggle_shown(&mut self) {
        self.auto.shown = !self.auto.shown;
        self.auto.sel = None;
        self.auto.picker = None;
        self.auto_sync_view();
        self.ui_dirty = true;
    }

    pub fn auto_toggle_draw(&mut self) {
        self.auto.draw_mode = !self.auto.draw_mode;
        self.ui_dirty = true;
    }

    pub fn auto_toggle_arm(&mut self) {
        self.auto.armed = !self.auto.armed;
        if !self.auto.armed {
            self.auto_write_end(|_| true);
        }
        self.set_status(if self.auto.armed { "Automation armed: control moves write automation while playing" } else { "Automation arm off" }, 3);
        self.ui_dirty = true;
    }

    pub fn auto_toggle_expanded(&mut self, track: TrackId) {
        if !self.auto.expanded.remove(&track) {
            self.auto.expanded.insert(track);
        }
        self.auto_sync_view();
        self.ui_dirty = true;
    }

    /// Power toggle: flips the lane's `enabled` (one step). Without a lane there is nothing to switch.
    pub fn auto_power(&mut self, key: LaneKey, extra: Option<usize>) {
        let target = match (key, extra) {
            (Some(t), Some(i)) => match self.extra_targets(t).get(i) {
                Some(x) => x.clone(),
                None => return,
            },
            _ => self.auto_target(key),
        };
        match self.model_lane(key, &target).cloned() {
            Some(mut l) => {
                l.enabled = !l.enabled;
                self.execute(Command::SetAutomationLane { track: key, target, lane: Some(l) });
            }
            None => self.set_status(format!("No {} automation yet: click the line to add a point", target.labels().1), 3),
        }
    }

    /// Clicking an expanded row's name shows that lane in the track row.
    pub fn auto_show_extra(&mut self, track: TrackId, index: usize) {
        if let Some(t) = self.extra_targets(track).get(index).cloned() {
            self.auto.targets.insert(Some(track), t);
            self.auto.sel = None;
            self.auto_sync_view();
            self.ui_dirty = true;
        }
    }

    // ---------------------------------------------------------------- picker

    fn pick_categories(&self, key: LaneKey) -> Vec<(String, Vec<PickEntry>)> {
        let e = |label: &str, target: AutomationTarget| PickEntry { label: label.into(), target: Some(target) };
        let mut cats = vec![("Track".to_string(), vec![e("Volume", AutomationTarget::Volume)])];
        let Some(t) = key.and_then(|k| self.project().track(k)) else { return cats };
        cats[0].1.push(e("Pan", AutomationTarget::Pan));
        if t.kind == TrackKind::Instrument && t.instrument_plugin.is_none() {
            cats.push(("Synth".into(), SynthParam::ALL.iter().map(|p| e(p.name(), AutomationTarget::Synth { param: *p })).collect()));
        }
        for d in t.instrument_plugin.iter().chain(t.devices.iter()) {
            if let Some(def) = saine_core::builtin::def_of(d) {
                let items = def
                    .params
                    .iter()
                    .filter(|p| p.automatable)
                    .map(|p| e(p.name, AutomationTarget::Builtin { device: d.id, device_type: def.id.into(), param: p.id.into() }))
                    .collect();
                cats.push((def.name.into(), items));
                continue;
            }
            let mut params: Vec<_> = self.engine.plugin_params(d.id).into_iter().filter(|p| p.automatable).collect();
            params.sort_by(|a, b| a.group.cmp(&b.group));
            let mut items = Vec::new();
            let mut group = String::new();
            for p in params {
                if !p.group.is_empty() && p.group != group {
                    group = p.group.clone();
                    items.push(PickEntry { label: group.clone(), target: None });
                }
                items.push(e(&p.name, AutomationTarget::Plugin { device: d.id, param: p.id, name: p.name.clone() }));
            }
            if items.is_empty() {
                items.push(PickEntry { label: "No automatable parameters".into(), target: None });
            }
            cats.push((d.plugin.name.clone(), items));
        }
        cats
    }

    /// Open the "Automation parameter" popup below a pill (window coordinates of its bottom-left).
    pub fn auto_open_picker(&mut self, key: LaneKey, x: f32, y: f32) {
        self.auto.picker = Some(Picker { key, pos: (x, y + 1.5), query: String::new(), sub: None, rows: Vec::new(), sub_rows: Vec::new() });
        self.ui_dirty = true;
    }

    pub fn auto_picker_close(&mut self) {
        if self.auto.picker.take().is_some() {
            self.ui_dirty = true;
        }
    }

    pub fn auto_picker_query(&mut self, q: &str) {
        if let Some(p) = &mut self.auto.picker {
            p.query = q.to_string();
            p.sub = None;
        }
        self.ui_dirty = true;
    }

    fn auto_choose(&mut self, key: LaneKey, target: AutomationTarget) {
        self.auto.targets.insert(key, target);
        self.auto.picker = None;
        self.auto.sel = None;
        self.auto_sync_view();
        self.ui_dirty = true;
    }

    /// Row of the main popup: opens a category submenu, or picks a search result.
    pub fn auto_picker_pick(&mut self, i: usize) {
        let Some(p) = &mut self.auto.picker else { return };
        if p.query.is_empty() {
            p.sub = if p.sub == Some(i) { None } else { Some(i) };
            self.ui_dirty = true;
            return;
        }
        if let Some(Some(t)) = p.rows.get(i).cloned() {
            let key = p.key;
            self.auto_choose(key, t);
        }
    }

    pub fn auto_picker_sub_pick(&mut self, i: usize) {
        let Some(p) = &self.auto.picker else { return };
        if let Some(Some(t)) = p.sub_rows.get(i).cloned() {
            let key = p.key;
            self.auto_choose(key, t);
        }
    }

    // ---------------------------------------------------------------- lane gestures

    fn row_at(&self, y: f32) -> Option<Row> {
        if y < timeline::RULER_H {
            return None;
        }
        let rows = self.auto_rows();
        let master = rows.iter().find(|r| r.master && r.contains(y)).cloned();
        master.or_else(|| rows.into_iter().find(|r| !r.master && r.contains(y) && y < self.lanes_size.1 - self.view.master_h))
    }

    fn ticks_at(&self, x: f32, snap: bool) -> Ticks {
        let b = self.view.beat_at(x).max(0.0);
        if snap { (b.round() * PPQ as f64) as Ticks } else { (b * PPQ as f64).round() as Ticks }
    }

    fn x_of_ticks(&self, t: Ticks) -> f32 {
        self.view.x_of(t as f64 / PPQ as f64)
    }

    /// Lane to edit: the model lane, or a new enabled lane (created on the first edit).
    fn edit_base(&self, row: &Row) -> AutomationLane {
        self.model_lane(row.key, &row.target).cloned().unwrap_or_else(|| AutomationLane::new(row.target.clone()))
    }

    fn point_hit(&self, row: &Row, lane: &AutomationLane, x: f32, y: f32) -> Option<usize> {
        let mut best = None;
        let mut bd = HIT;
        for (i, p) in lane.points.iter().enumerate() {
            let d = (self.x_of_ticks(p.time) - x).hypot(row.y_of(norm_of(&row.target, p.value)) - y);
            if d <= bd {
                bd = d;
                best = Some(i);
            }
        }
        best
    }

    fn line_y(&self, row: &Row, lane: &AutomationLane, x: f32) -> f32 {
        let t = self.ticks_at(x, false);
        let v = lane.value_at(t).unwrap_or_else(|| self.static_value(row.key, &row.target));
        row.y_of(norm_of(&row.target, v))
    }

    fn select_row_track(&mut self, row: &Row) {
        if let Some(t) = row.key {
            self.select_track(t);
        }
        self.select_clip(None);
        self.pr.focused = false;
    }

    /// Lane pointer while automation is shown. Returns true when the automation took the event.
    pub fn auto_pointer(&mut self, kind: i32, x: f32, y: f32, shift: bool, alt: bool) -> bool {
        match kind {
            0 => {
                let Some(row) = self.row_at(y) else { return false };
                self.auto_down(row, x, y, shift, alt);
                self.ui_dirty = true;
                true
            }
            1 => {
                if self.auto.gesture.is_none() {
                    return false;
                }
                self.auto_move(x, y);
                self.ui_dirty = true;
                true
            }
            _ => {
                if self.auto.gesture.is_none() {
                    return false;
                }
                self.auto_up();
                self.ui_dirty = true;
                true
            }
        }
    }

    fn auto_down(&mut self, row: Row, x: f32, y: f32, shift: bool, alt: bool) {
        self.select_row_track(&row);
        self.auto.picker = None;
        let base = self.edit_base(&row);
        let now = Instant::now();
        if self.auto.draw_mode {
            let t = self.ticks_at(x, !alt);
            let v = value_of(&row.target, row.norm_at(y));
            self.auto.sel = None;
            self.auto.gesture = Some(Gesture::Freehand { row, base, samples: vec![(t, v)], alt });
            return;
        }
        if let Some(i) = self.point_hit(&row, &base, x, y) {
            // Double-click on a point deletes it (one step).
            if let Some((t0, k, tg, j)) = &self.auto.last_down
                && now.duration_since(*t0) < DOUBLE_CLICK
                && *k == row.key
                && *tg == row.target
                && *j == i
            {
                self.auto.last_down = None;
                let mut l = base;
                l.points.remove(i);
                self.auto.sel = None;
                self.auto_commit(row.key, l);
                return;
            }
            self.auto.last_down = Some((now, row.key, row.target.clone(), i));
            let mut idx = self.sel_indices(&row);
            if shift {
                if let Some(pos) = idx.iter().position(|j| *j == i) {
                    idx.remove(pos);
                } else {
                    idx.push(i);
                }
            } else if !idx.contains(&i) {
                idx = vec![i];
            }
            idx.sort_unstable();
            self.auto.sel = Some((row.key, row.target.clone(), idx.clone()));
            if !idx.contains(&i) {
                return;
            }
            let grab_time = base.points[i].time;
            self.auto.gesture = Some(Gesture::Points { row, base, idx, grab: (x, y), grab_time, created: false, alt });
            return;
        }
        self.auto.last_down = None;
        let on_line = (self.line_y(&row, &base, x) - y).abs() <= HIT;
        if on_line && shift && base.points.len() >= 2 {
            let t = self.ticks_at(x, false);
            let seg = base.points.partition_point(|p| p.time <= t);
            if seg >= 1 && seg < base.points.len() {
                let (a, b) = (base.points[seg - 1], base.points[seg]);
                let dir = if norm_of(&row.target, b.value) >= norm_of(&row.target, a.value) { 1.0 } else { -1.0 };
                self.auto.sel = None;
                self.auto.gesture = Some(Gesture::Curve { row, c0: a.curve, base, seg: seg - 1, y0: y, dir });
                return;
            }
        }
        if on_line && !shift {
            // Click on the line: a new point (time snapped to the grid unless Alt), value on the line.
            let t = self.ticks_at(x, !alt);
            let v = base.value_at(t).unwrap_or_else(|| self.static_value(row.key, &row.target));
            let mut lane = base.clone();
            let i = lane.points.partition_point(|p| p.time <= t);
            lane.points.insert(i, AutoPoint { time: t, value: v, curve: 0.0 });
            self.auto.sel = Some((row.key, row.target.clone(), vec![i]));
            self.auto.pending = Some((row.key, lane.clone()));
            self.auto.gesture = Some(Gesture::Points { row, base: lane, idx: vec![i], grab: (x, y), grab_time: t, created: true, alt });
            return;
        }
        // Empty area: marquee (Shift adds to the selection).
        let add = if shift { self.sel_indices(&row) } else { Vec::new() };
        if !shift {
            self.auto.sel = None;
        }
        self.auto.gesture = Some(Gesture::Marquee { row, x0: x, y0: y, x1: x, y1: y, add });
    }

    fn auto_move(&mut self, x: f32, y: f32) {
        let Some(g) = self.auto.gesture.clone() else { return };
        match g {
            Gesture::Points { row, base, idx, grab, grab_time, alt, .. } => {
                let raw = self.ticks_at(x, false) - self.ticks_at(grab.0, false);
                let mut delta = if alt { grab_time + raw } else { ((grab_time + raw) as f64 / PPQ as f64).round() as Ticks * PPQ } - grab_time;
                // Keep the moved points between their unmoved neighbours (and at t >= 0).
                let pts = &base.points;
                for &i in &idx {
                    let p = pts[i];
                    let prev = (0..i).rev().find(|j| !idx.contains(j)).map(|j| pts[j].time).unwrap_or(0);
                    let next = (i + 1..pts.len()).find(|j| !idx.contains(j)).map(|j| pts[j].time);
                    delta = delta.max(prev - p.time);
                    if let Some(n) = next {
                        delta = delta.min(n - p.time);
                    }
                }
                let dn = row.norm_at(y) - row.norm_at(grab.1);
                let mut lane = base.clone();
                for &i in &idx {
                    let p = &mut lane.points[i];
                    p.time = (p.time + delta).max(0);
                    p.value = value_of(&row.target, norm_of(&row.target, base.points[i].value) + dn);
                }
                lane.points.sort_by_key(|p| p.time);
                self.auto.pending = Some((row.key, lane));
            }
            Gesture::Curve { row, base, seg, y0, c0, dir } => {
                let mut lane = base.clone();
                let up = (y0 - y) / 40.0;
                lane.points[seg].curve = ((c0 - dir * up) * 100.0).round().clamp(-100.0, 100.0) / 100.0;
                self.auto.pending = Some((row.key, lane));
            }
            Gesture::Marquee { row, x0, y0, add, .. } => {
                let (lx, hx, ly, hy) = (x0.min(x), x0.max(x), y0.min(y), y0.max(y));
                let base = self.lane(row.key, &row.target);
                let mut idx = add.clone();
                if let Some(l) = base {
                    for (i, p) in l.points.iter().enumerate() {
                        let (px, py) = (self.x_of_ticks(p.time), row.y_of(norm_of(&row.target, p.value)));
                        if px >= lx && px <= hx && py >= ly && py <= hy && !idx.contains(&i) {
                            idx.push(i);
                        }
                    }
                }
                idx.sort_unstable();
                self.auto.sel = (!idx.is_empty()).then(|| (row.key, row.target.clone(), idx));
                self.auto.gesture = Some(Gesture::Marquee { row, x0, y0, x1: x, y1: y, add });
            }
            Gesture::Freehand { row, base, mut samples, alt } => {
                let t = self.ticks_at(x, false);
                let v = value_of(&row.target, row.norm_at(y));
                match samples.last_mut() {
                    Some(last) if last.0 == t => last.1 = v,
                    _ => samples.push((t, v)),
                }
                let lane = freehand_lane(&base, &row.target, &samples);
                self.auto.pending = Some((row.key, lane));
                self.auto.gesture = Some(Gesture::Freehand { row, base, samples, alt });
            }
        }
        self.publish_preview();
    }

    fn auto_up(&mut self) {
        let Some(g) = self.auto.gesture.take() else { return };
        let pending = self.auto.pending.take();
        match g {
            Gesture::Points { row, idx, created, .. } => {
                // Moved points stay between their unmoved neighbours, so `idx` is still valid.
                if let Some((_, lane)) = pending {
                    let changed = created || self.model_lane(row.key, &row.target) != Some(&lane);
                    if changed {
                        self.auto_commit(row.key, lane);
                    }
                    self.auto.sel = Some((row.key, row.target.clone(), idx));
                }
            }
            Gesture::Curve { row, .. } => {
                if let Some((_, lane)) = pending {
                    self.auto_commit(row.key, lane);
                }
            }
            Gesture::Marquee { .. } => {}
            Gesture::Freehand { row, base, samples, .. } => {
                let lane = if samples.len() == 1 {
                    let (t, v) = samples[0];
                    let mut l = base;
                    let i = l.points.partition_point(|p| p.time <= t);
                    l.points.insert(i, AutoPoint { time: t, value: v, curve: 0.0 });
                    l
                } else {
                    freehand_lane(&base, &row.target, &samples)
                };
                self.auto_commit(row.key, lane);
            }
        }
        self.publish();
    }

    /// Commit one lane (one undo step). Returns false when nothing changed.
    pub fn auto_commit(&mut self, key: LaneKey, lane: AutomationLane) -> bool {
        if self.model_lane(key, &lane.target) == Some(&lane) {
            return false;
        }
        let target = lane.target.clone();
        self.execute(Command::SetAutomationLane { track: key, target, lane: Some(lane) })
    }

    /// Delete / Backspace: remove the selected points (one step). False if nothing is selected.
    pub fn auto_delete_selected(&mut self) -> bool {
        let Some((key, target, idx)) = self.auto.sel.take() else { return false };
        let Some(mut lane) = self.model_lane(key, &target).cloned() else { return false };
        let before = lane.points.len();
        let mut i = 0;
        lane.points.retain(|_| {
            i += 1;
            !idx.contains(&(i - 1))
        });
        if lane.points.len() == before {
            return false;
        }
        self.auto_commit(key, lane);
        true
    }

    /// Engine preview of the pending lane (heard while dragging / writing).
    fn publish_preview(&mut self) {
        let snap = self.current_snapshot();
        let _ = self.engine.publish(&snap);
    }

    // ---------------------------------------------------------------- write (automation arm)

    /// A control moved (track volume / pan, master volume, synth knob). Returns true when
    /// automation took the move: recorded (armed + playing), or blocked because the lane is
    /// active (the control follows the automation; a status hint explains why).
    pub fn auto_intercept(&mut self, key: LaneKey, target: AutomationTarget, value: f32) -> bool {
        let (lo, hi) = target.range();
        let value = value.clamp(lo, hi);
        if self.auto.armed && self.engine.transport().playing {
            self.auto_record(key, target, value, None);
            return true;
        }
        if self.auto_value_now(key, &target).is_some() {
            let what = match key {
                None => "Master volume".to_string(),
                Some(_) => target.labels().1,
            };
            let msg = format!("{what} is automated: arm automation to write, or disable the lane");
            if self.status_text() != Some(msg.as_str()) {
                self.set_status(msg, 4);
            }
            return true;
        }
        false
    }

    /// Record one sample of a write pass at the audible position (or `at` ticks).
    pub fn auto_record(&mut self, key: LaneKey, target: AutomationTarget, value: f32, at: Option<Ticks>) {
        let now = at.unwrap_or_else(|| self.playhead_ticks());
        let i = match self.auto.writes.iter().position(|w| w.key == key && w.target == target) {
            Some(i) => i,
            None => {
                let base = self.model_lane(key, &target).cloned().unwrap_or_else(|| AutomationLane::new(target.clone()));
                self.auto.writes.push(WritePass { key, target: target.clone(), start: now, samples: Vec::new(), base, last: Instant::now() });
                self.auto.writes.len() - 1
            }
        };
        let w = &mut self.auto.writes[i];
        w.last = Instant::now();
        match w.samples.last_mut() {
            Some(last) if last.0 >= now => last.1 = value,
            _ => w.samples.push((now, value)),
        }
        // Preview: recorded so far plus a short hold at the current value.
        let mut s = w.samples.clone();
        let hold = now + PPQ / 2;
        s.push((hold, value));
        let lane = touch_merge(&w.base, w.start, hold, &s);
        self.auto.pending = Some((key, lane));
        self.publish_preview();
        self.ui_dirty = true;
    }

    /// End write passes matching `f`: thin, merge ("touch") and commit them as ONE `Batch`.
    pub fn auto_write_end(&mut self, f: impl Fn(&WritePass) -> bool) -> bool {
        let (done, keep): (Vec<WritePass>, Vec<WritePass>) = std::mem::take(&mut self.auto.writes).into_iter().partition(|w| f(w));
        self.auto.writes = keep;
        if done.is_empty() {
            return false;
        }
        if self.auto.pending.as_ref().is_some_and(|(k, l)| done.iter().any(|w| w.key == *k && w.target == l.target)) {
            self.auto.pending = None;
        }
        let mut cmds = Vec::new();
        for w in done {
            let Some(&(end, _)) = w.samples.last() else { continue };
            let norm: Vec<(Ticks, f32)> = w.samples.iter().map(|&(t, v)| (t, norm_of(&w.target, v))).collect();
            let thinned: Vec<(Ticks, f32)> = thin(&norm, THIN_TOLERANCE).into_iter().map(|(t, n)| (t, w.samples.iter().find(|s| s.0 == t).map(|s| s.1).unwrap_or_else(|| value_of(&w.target, n)))).collect();
            let mut lane = touch_merge(&w.base, w.start, end, &thinned);
            lane.enabled = true;
            cmds.push(Command::SetAutomationLane { track: w.key, target: w.target.clone(), lane: Some(lane) });
        }
        if cmds.is_empty() {
            self.publish();
            return false;
        }
        self.doc.seal_coalescing();
        let ok = self.execute(Command::Batch { label: "Write Automation".into(), commands: cmds });
        if ok {
            self.auto.writes_committed += 1;
        }
        self.publish();
        ok
    }

    /// UI timer: plugin editor changes (`take_plugin_param_changes`) become write passes while
    /// armed and playing; passes without a gesture end finish after `PLUGIN_IDLE`.
    pub fn auto_poll(&mut self) {
        let changes = self.engine.take_plugin_param_changes();
        let recording = self.auto.armed && self.engine.transport().playing;
        for ch in changes {
            if !recording {
                continue;
            }
            let Some((ti, _)) = self.project().find_device(ch.device) else { continue };
            let track = self.project().tracks[ti].id;
            let name = self.engine.plugin_params(ch.device).into_iter().find(|p| p.id == ch.param).map(|p| p.name).unwrap_or_else(|| format!("Parameter {}", ch.param));
            let target = AutomationTarget::Plugin { device: ch.device, param: ch.param, name };
            let at = self.project().frames_to_ticks(ch.frame, self.rate);
            self.auto_record(Some(track), target.clone(), ch.normalized.clamp(0.0, 1.0), Some(at));
            if ch.gesture_end {
                self.auto_write_end(|w| w.key == Some(track) && w.target == target);
            }
        }
        let idle: Vec<(LaneKey, AutomationTarget)> = self
            .auto
            .writes
            .iter()
            .filter(|w| matches!(w.target, AutomationTarget::Plugin { .. }) && w.last.elapsed() > PLUGIN_IDLE)
            .map(|w| (w.key, w.target.clone()))
            .collect();
        for (k, t) in idle {
            self.auto_write_end(|w| w.key == k && w.target == t);
        }
        if !self.auto.armed || !self.engine.transport().playing {
            // Transport stopped mid-gesture: the pass ends with what was recorded.
            if !self.auto.writes.is_empty() && !self.engine.transport().playing {
                self.auto_write_end(|_| true);
            }
        }
    }

    // ---------------------------------------------------------------- view models

    /// Effective (automated or static) values of the track controls.
    fn track_feedback(&self, id: TrackId) -> (f32, bool, f32, bool) {
        let p = self.project();
        let Some(t) = p.track(id) else { return (0.0, false, 0.0, false) };
        let vol = self.auto_value_now(Some(id), &AutomationTarget::Volume);
        let pan = self.auto_value_now(Some(id), &AutomationTarget::Pan);
        let gain = vol.map(|db| if db <= VOLUME_MIN_DB { 0.0 } else { db_to_gain(db) }).unwrap_or(t.gain);
        (gain, vol.is_some(), pan.unwrap_or(t.pan), pan.is_some())
    }

    /// Any enabled lane with points (controls then follow the playhead every frame).
    pub fn auto_any_active(&self) -> bool {
        let p = self.project();
        p.master_automation.iter().chain(p.tracks.iter().flat_map(|t| t.automation.iter())).any(|l| l.enabled && !l.points.is_empty())
            || self.auto.pending.is_some()
    }

    /// Controls that follow automation (display only) + their "automated" style.
    pub fn push_auto_feedback(&mut self, s: &Studio) {
        let p = self.project();
        let ids: Vec<TrackId> = p.tracks.iter().map(|t| t.id).collect();
        for (i, id) in ids.iter().enumerate() {
            let (gain, va, pan, pa) = self.track_feedback(*id);
            if let Some(r) = self.tracks_model.row_data(i) {
                let new = TrackRow { volume_pos: slider_from_gain(gain), pan, vol_auto: va, pan_auto: pa, ..r.clone() };
                if new != r {
                    self.tracks_model.set_row_data(i, new);
                }
            }
        }
        if let Some(id) = self.selected_track {
            let (gain, va, pan, pa) = self.track_feedback(id);
            let db = gain_to_db(gain);
            let f = crate::controller::fader_from_db(db);
            if (s.get_sel_fader() - f).abs() > 1e-5 {
                s.set_sel_fader(f);
            }
            let t = fmt_db(db);
            if s.get_sel_db_text().as_str() != t {
                s.set_sel_db_text(t.into());
            }
            if (s.get_sel_pan() - pan).abs() > 1e-6 {
                s.set_sel_pan(pan);
                s.set_sel_pan_text(fmt_pan(pan).into());
            }
            s.set_sel_vol_auto(va);
            s.set_sel_pan_auto(pa);
        }
        let mv = self.auto_value_now(None, &AutomationTarget::Volume);
        let mg = mv.map(|db| if db <= VOLUME_MIN_DB { 0.0 } else { db_to_gain(db) }).unwrap_or(self.project().master_gain);
        let mp = slider_from_gain(mg);
        if (s.get_master_pos() - mp).abs() > 1e-5 {
            s.set_master_pos(mp);
        }
        s.set_master_vol_auto(mv.is_some());
        // Synth knobs (built-in Synth of the selected instrument track) follow their lanes.
        if let (Some(t), Some(device)) = (self.synth_track(), self.synth_device()) {
            let def = &saine_core::builtin::synth_v2::SYNTH_V2;
            let targets: Vec<AutomationTarget> = self.project().track(t).map_or(Vec::new(), |tr| tr.automation.iter().filter(|l| matches!(&l.target, AutomationTarget::Builtin { device: d, .. } if *d == device)).map(|l| l.target.clone()).collect());
            for target in targets {
                let AutomationTarget::Builtin { param, .. } = &target else { continue };
                let (Some(id), Some(v)) = (def.index_of(param), self.auto_value_now(Some(t), &target)) else { continue };
                if let Some(mut row) = self.synth_params_model.row_data(id) {
                    let p = &def.params[id];
                    let (n, text) = (p.to_normalized(v), p.format(v));
                    if !row.auto || (row.norm - n).abs() > 1e-5 || row.text.as_str() != text {
                        row.auto = true;
                        row.norm = n;
                        row.text = text.into();
                        self.synth_params_model.set_row_data(id, row);
                    }
                }
            }
        }
    }

    /// Track header rows (controls, automation pill, row geometry).
    pub fn track_rows(&self) -> Vec<TrackRow> {
        let p = self.project();
        p.tracks
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let target = self.auto_target(Some(t.id));
                let (cat, param) = target.labels();
                let (gain, va, pan, pa) = self.track_feedback(t.id);
                TrackRow {
                    id: t.id.0 as i32,
                    number: i as i32 + 1,
                    name: t.name.clone().into(),
                    instrument: t.kind == TrackKind::Instrument,
                    color: crate::controller::slint_color(crate::controller::track_color(t.color)),
                    selected: Some(t.id) == self.selected_track,
                    mute: t.mute,
                    solo: t.solo,
                    arm: t.arm,
                    monitor: t.monitor,
                    volume_pos: slider_from_gain(gain),
                    pan,
                    y: self.view.track_offset(i),
                    h: self.view.track_span(i),
                    auto_cat: cat.into(),
                    auto_param: param.into(),
                    auto_state: self.auto_style(Some(t.id), &target).code(),
                    auto_accent: self.lane_hue(&target).slint_bright(),
                    auto_count: self.extra_targets(t.id).len() as i32 + 1,
                    auto_expanded: self.auto.expanded.contains(&t.id),
                    vol_auto: va,
                    pan_auto: pa,
                }
            })
            .collect()
    }

    /// Header extras, labels and the picker → Slint.
    pub fn push_automation(&mut self, s: &Studio) {
        self.auto_sync_view();
        s.set_auto_shown(self.auto.shown);
        s.set_auto_draw(self.auto.draw_mode);
        s.set_auto_armed(self.auto.armed);
        // Slint's master row excludes the 0.5 pt separator above it.
        s.set_master_h(self.view.master_h - 0.5);
        s.set_tracks_height(self.view.track_offset(self.project().tracks.len()));
        let n = self.project().tracks.len();
        let mut extras = Vec::new();
        if self.auto.shown {
            for i in 0..n {
                let id = self.project().tracks[i].id;
                if !self.auto.expanded.contains(&id) {
                    continue;
                }
                for (k, t) in self.extra_targets(id).into_iter().enumerate() {
                    let (cat, param) = t.labels();
                    extras.push(AutoExtra {
                        track_id: id.0 as i32,
                        index: k as i32,
                        y: self.view.track_offset(i) + TRACK_PITCH + k as f32 * EXTRA_PITCH,
                        cat: cat.into(),
                        param: param.into(),
                        state: self.auto_style(Some(id), &t).code(),
                        accent: self.lane_hue(&t).slint_bright(),
                    });
                }
            }
        }
        sync_model(&self.auto_extras_model, extras);
        let ms = self.auto_style(None, &AutomationTarget::Volume).code();
        s.set_master_auto_state(ms);
        self.push_picker(s);
        self.push_auto_feedback(s);
    }

    /// Value labels (every frame: they move with scrolling and the playhead does not matter).
    pub fn update_auto_labels(&mut self) {
        let labels = if self.auto.shown { self.auto_labels() } else { Vec::new() };
        sync_model(&self.auto_labels_model, labels);
    }

    fn push_picker(&mut self, s: &Studio) {
        let Some(mut pk) = self.auto.picker.clone() else {
            s.set_picker_open(false);
            // The search field is destroyed with the popup without a focus-change callback.
            s.set_picker_focused(false);
            return;
        };
        const ROW: f32 = 21.0;
        const TOP: f32 = 30.5;
        let cats = self.pick_categories(pk.key);
        let current = self.auto_target(pk.key);
        let item = |label: String, submenu: bool, enabled: bool, checked: bool, i: usize, top: f32| MenuItem {
            label: label.into(),
            shortcut: if checked { "✓".into() } else { "".into() },
            enabled,
            separator: false,
            submenu,
            y: top + i as f32 * ROW,
        };
        let q = pk.query.trim().to_lowercase();
        let rows: Vec<MenuItem>;
        if q.is_empty() {
            rows = cats.iter().enumerate().map(|(i, (name, _))| item(name.clone(), true, true, false, i, TOP)).collect();
            pk.rows = vec![None; cats.len()];
        } else {
            let mut found = Vec::new();
            for (cname, items) in &cats {
                for e in items {
                    if let Some(t) = &e.target
                        && (e.label.to_lowercase().contains(&q) || cname.to_lowercase().contains(&q))
                    {
                        found.push((format!("{cname} › {}", e.label), t.clone()));
                    }
                }
            }
            found.truncate(14);
            if found.is_empty() {
                rows = vec![item("No matching parameters".into(), false, false, false, 0, TOP)];
                pk.rows = vec![None];
            } else {
                rows = found.iter().enumerate().map(|(i, (l, t))| item(l.clone(), false, true, *t == current, i, TOP)).collect();
                pk.rows = found.into_iter().map(|(_, t)| Some(t)).collect();
            }
        }
        let h = TOP + rows.len() as f32 * ROW + 4.0;
        s.set_picker_open(true);
        s.set_picker_x(pk.pos.0);
        s.set_picker_y(pk.pos.1);
        s.set_picker_h(h);
        if s.get_picker_query().as_str() != pk.query {
            s.set_picker_query(pk.query.clone().into());
        }
        sync_model(&self.picker_model, rows);
        let sub = pk.sub.filter(|_| q.is_empty()).and_then(|i| cats.get(i).map(|c| (i, c)));
        match sub {
            Some((i, (_, items))) => {
                let srows: Vec<MenuItem> = items
                    .iter()
                    .enumerate()
                    .map(|(k, e)| item(e.label.clone(), false, e.target.is_some(), e.target.as_ref() == Some(&current), k, 4.0))
                    .collect();
                pk.sub_rows = items.iter().map(|e| e.target.clone()).collect();
                s.set_picker_sub_open(true);
                s.set_picker_sub_y(TOP + i as f32 * ROW - 4.0);
                s.set_picker_sub_h(8.0 + srows.len() as f32 * ROW);
                sync_model(&self.picker_sub_model, srows);
            }
            None => {
                pk.sub_rows.clear();
                s.set_picker_sub_open(false);
            }
        }
        self.auto.picker = Some(pk);
    }
}

/// Freehand stroke → lane: the stroke's samples (thinned) replace the points in its time range.
fn freehand_lane(base: &AutomationLane, target: &AutomationTarget, samples: &[(Ticks, f32)]) -> AutomationLane {
    let mut s = samples.to_vec();
    s.sort_by_key(|p| p.0);
    let (lo, hi) = (s[0].0, s[s.len() - 1].0);
    let norm: Vec<(Ticks, f32)> = s.iter().map(|&(t, v)| (t, norm_of(target, v))).collect();
    let kept = thin(&norm, THIN_TOLERANCE);
    let mut lane = base.clone();
    lane.points.retain(|p| p.time < lo || p.time > hi);
    for (t, n) in kept {
        let v = s.iter().find(|p| p.0 == t).map(|p| p.1).unwrap_or_else(|| value_of(target, n));
        let i = lane.points.partition_point(|p| p.time <= t);
        lane.points.insert(i, AutoPoint { time: t, value: v, curve: 0.0 });
    }
    lane
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mapping_round_trips_and_zero_db_position() {
        let v = AutomationTarget::Volume;
        assert!((norm_of(&v, 0.0) - 0.501).abs() < 0.002);
        for db in [-48.0, -12.0, -6.0, 0.0, 3.0, 6.0] {
            assert!((value_of(&v, norm_of(&v, db)) - db).abs() < 0.02, "{db}");
        }
        assert_eq!(value_of(&v, 0.0), VOLUME_MIN_DB);
        let c = AutomationTarget::Synth { param: SynthParam::FilterCutoff };
        assert!((value_of(&c, 0.5) - 632.46).abs() < 1.0);
        let row = Row { key: None, target: v.clone(), y0: 0.0, h: 74.0, master: false, extra: false };
        // 0 dB at 0.557 of the lane height (60-automation-lanes.png).
        assert!((row.y_of(norm_of(&v, 0.0)) / 74.0 - 0.557).abs() < 0.003);
        assert!((row.norm_at(row.y_of(0.3)) - 0.3).abs() < 1e-5);
    }

    #[test]
    fn labels() {
        assert_eq!(volume_label(0.0), "+0.0");
        assert_eq!(volume_label(-0.04), "+0.0");
        assert_eq!(volume_label(-6.0), "−6.0");
        assert_eq!(volume_label(-60.0), "−∞");
    }

    #[test]
    fn thinning_drops_collinear_points() {
        let pts: Vec<(Ticks, f32)> = (0..=10).map(|i| (i * 100, i as f32 * 0.05)).collect();
        assert_eq!(thin(&pts, 0.004), vec![(0, 0.0), (1000, 0.5)]);
        let bent = vec![(0, 0.0), (100, 0.5), (200, 0.0)];
        assert_eq!(thin(&bent, 0.004).len(), 3);
    }

    #[test]
    fn touch_merge_replaces_range_and_returns_to_curve() {
        let base = AutomationLane { target: AutomationTarget::Volume, enabled: true, points: vec![AutoPoint { time: 0, value: -12.0, curve: 0.0 }, AutoPoint { time: 1000, value: -12.0, curve: 0.0 }, AutoPoint { time: 2000, value: 0.0, curve: 0.0 }] };
        let m = touch_merge(&base, 500, 1500, &[(500, -30.0), (1500, -20.0)]);
        m.validate().unwrap();
        assert_eq!(m.value_at(0), Some(-12.0));
        assert_eq!(m.value_at(2000), Some(0.0));
        assert!(m.points.iter().all(|p| p.time != 1000), "point inside the range replaced");
        let empty = AutomationLane::new(AutomationTarget::Volume);
        let m = touch_merge(&empty, 500, 1500, &[(500, -30.0), (1500, -20.0)]);
        assert_eq!(m.points.len(), 2);
    }

    #[test]
    fn lane_hue_follows_the_parameter() {
        let none = "";
        assert_eq!(Hue::of(&AutomationTarget::Volume, none), Hue::Gold);
        assert_eq!(Hue::of(&AutomationTarget::Pan, none), Hue::Blue);
        assert_eq!(LineStyle::Active.line(Hue::Gold), hex(0xebc542, 1.0));
        assert_eq!(LineStyle::Inactive.line(Hue::Blue), hex(0x30525f, 1.0));
        // The hashed rule reproduces colours sampled from the reference (ui-inventory.md §12).
        for (dev, param, hue) in [
            ("Synth", "Cutoff", Hue::Indigo),
            ("Synth", "Osc 1 Level", Hue::Orange),
            ("Synth", "Osc 2 Octave", Hue::Coral),
            ("Synth", "Resonance", Hue::Teal),
            ("Synth", "LFO 1 Rate", Hue::Blue),
            ("Synth", "Unison Detune", Hue::Magenta),
            ("Synth", "Noise Level", Hue::Green),
            ("Synth", "Glide", Hue::Purple),
            ("Character EQ", "B1 Freq", Hue::Blue),
        ] {
            assert_eq!(Hue::key(dev, param), hue, "{dev}/{param}");
        }
        assert_eq!(Hue::of(&AutomationTarget::Synth { param: SynthParam::FilterCutoff }, none), Hue::Indigo);
        // Plugin parameters: same device + parameter name → same hue on every track.
        let p = |device: u64, param: u32| AutomationTarget::Plugin { device: saine_core::DeviceId(device), param, name: "B1 Freq".into() };
        assert_eq!(Hue::of(&p(1, 3), "Character EQ"), Hue::of(&p(7, 9), "Character EQ"));
    }
}
