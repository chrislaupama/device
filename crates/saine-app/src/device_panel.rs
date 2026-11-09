//! Built-in device panels: the generic card generated from a device
//! definition (saine_core::builtin::DeviceDef) and its gestures.
//!
//! Layout follows the reference fx card template (handoffs/device-specs/_system.md
//! section 6): header, picker row, view row (a visualiser plus the slider column)
//! and the dial row of knob clusters. All lengths below are the reference's
//! 100%-zoom measurements; [Z] scales them to the 85% our UI is built at.
//!
//! Gestures: a knob, slider, pill or strip-line drag sends live values to the
//! engine (se_builtin_set_param, no model change) and commits ONE
//! Command::SetDeviceParam on release, so a drag is one undo step and the
//! session is republished once. Clicks, arrow keys, typed values, resets and
//! presets are single commands. Escape / pointer cancel restores the value
//! the drag started from.
//!
//! Custom views: View::DynamicsStrip is drawn here (paths built from the
//! engine's readouts); View::Field plots a curve model from device_fields.rs
//! (traces, live LFO overlay from the engine's phase readout) and edits two
//! parameters by dragging; View::Custom(name) is the escape hatch for device-
//! specific bodies. The restoration devices' graphs (wave, band, comb and
//! gate views with handles, grip pills and a spectrum overlay) live in
//! src/panel_graph.rs.

use crate::controller::{slint_color, track_color, Controller};
use crate::devices::{DevMenu, MenuPick};
use crate::device_fields::{self, Fill, Live, Style};
use crate::{DeviceCard, PanelField, PanelKnob, PanelPick, PanelSlider, PanelStrip, PanelTick};
use saine_core::builtin::{Control, DeviceDef, Face, Field, ParamDef, Strip, Transfer, View, CLUSTER_GAP, KNOB_CELL, KNOB_GAP};
use saine_core::{Command, Device, DeviceId, TrackId};
use std::borrow::Cow;
use slint::{ModelRc, SharedString, VecModel};
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;

/// Reference card lengths are at 100% zoom; ours are drawn at 85%.
pub const Z: f32 = 0.85;
/// Card padding, slider column width and the gap between view and sliders.
pub(crate) const PAD: f32 = 8.0;
const SLIDER_W: f32 = 62.0;
const VIEW_GAP: f32 = 10.0;
/// View row top, caption zone above the well, well bottom (card relative).
/// Cards without a picker row start the view row higher (VIEW_Y_NO_PICKER).
pub(crate) const VIEW_Y: f32 = 63.0;
pub(crate) const VIEW_Y_NO_PICKER: f32 = 37.0;
pub(crate) const CAPTION_H: f32 = 19.0;
pub(crate) const VIEW_BOTTOM: f32 = 214.0;
/// Dial row top (card relative).
pub(crate) const DIAL_Y: f32 = 229.0;
/// Dynamics strip history length (slots across the well).
const HISTORY: usize = 240;
/// Readout level of silence (below every strip floor).
const SILENT_DB: f32 = -120.0;
/// Picker index of a card's view selector (not a parameter, e.g. the
/// multiband band tabs): `panel_choice` with it changes the view state.
pub(crate) const VIEW_SELECT: i32 = -3;
/// Detent snap distance of a drag, in the 0..1 range (reference: 4 / 160).
const DETENT_SNAP: f32 = 0.025;
/// Plot area inside a field's well: tick gutter on the left, top and bottom insets.
const FIELD_GUTTER: f32 = 24.0;
const FIELD_TOP: f32 = 4.0;
const FIELD_BOTTOM: f32 = 6.0;
/// Pointer travel before a press becomes a drag (px).
const DRAG_SLOP: f32 = 3.0;
const MINUS: char = '\u{2212}';

/// Live state of one parameter drag.
#[derive(Debug, Clone, Copy)]
struct ParamDrag {
    device: DeviceId,
    index: usize,
    start: f32,
    norm: f32,
    value: f32,
    travel: f32,
    /// Movement before the start threshold, applied once the drag starts.
    held: f32,
    active: bool,
}

/// Smoothed level history of a dynamics strip (newest last).
#[derive(Debug, Clone, Default)]
struct MeterHistory {
    /// (input dB, output dB, gain reduction dB <= 0)
    slots: VecDeque<(f32, f32, f32)>,
    last: (f32, f32, f32),
}

/// Live state of a field drag: (parameter index, start, normalised, value) per axis.
#[derive(Debug, Clone, Copy)]
struct FieldDrag {
    device: DeviceId,
    axes: [Option<(usize, f32, f32, f32)>; 2],
    travel: f32,
    active: bool,
}

#[derive(Debug, Default)]
pub struct PanelState {
    drag: Option<ParamDrag>,
    field: Option<FieldDrag>,
    meters: HashMap<DeviceId, MeterHistory>,
    /// Card view state of devices with a custom view (selected band, meters).
    pub(crate) views: HashMap<DeviceId, crate::panel_multiband::ViewState>,
    pub(crate) sampler: crate::sampler_panel::SamplerUi,
    /// Field devices while the transport plays: (last LFO phase, elapsed cycles).
    lfo: HashMap<DeviceId, (f32, f64)>,
    pub(crate) editors: crate::device_editors::ViewState,
    /// Field views (src/panel_graph.rs): drag, hover and spectrum state.
    pub(crate) graph: crate::panel_graph::GraphState,
}

fn s(v: impl Into<SharedString>) -> SharedString {
    v.into()
}

fn model<T: Clone + 'static>(v: Vec<T>) -> ModelRc<T> {
    ModelRc::from(Rc::new(VecModel::from(v)))
}

thread_local! {
    /// Models that only depend on the definition (choice names, strip rules), shared by
    /// every card so a refresh never swaps them under a pressed control.
    static STATIC_MODELS: std::cell::RefCell<HashMap<String, ModelRc<SharedString>>> = Default::default();
    static TICK_MODELS: std::cell::RefCell<HashMap<String, ModelRc<PanelTick>>> = Default::default();
}

fn choices_model(def: &DeviceDef, p: &ParamDef) -> ModelRc<SharedString> {
    labels_model(def, p, p.choices)
}

fn labels_model(def: &DeviceDef, p: &ParamDef, labels: &[&str]) -> ModelRc<SharedString> {
    STATIC_MODELS.with(|m| m.borrow_mut().entry(format!("{}/{}/{}", def.id, p.id, labels.join("|"))).or_insert_with(|| model(labels.iter().map(|c| s(*c)).collect())).clone())
}

/// Width of a dropdown pill: its longest choice, the padding and the chevron (pt).
fn dropdown_width(p: &ParamDef) -> f32 {
    let longest = p.choices.iter().map(|c| c.chars().count()).max().unwrap_or(4);
    longest as f32 * 5.4 + 26.0
}

/// Keep a card's row models across refreshes: the Slint repeaters then update
/// their items in place instead of rebuilding them (a rebuilt knob would drop
/// the drag it is in the middle of).
pub(crate) fn reuse_card_models(old: &DeviceCard, new: &mut DeviceCard) {
    fn keep<T: Clone + PartialEq + 'static>(old: &ModelRc<T>, new: &mut ModelRc<T>) {
        let Some(vm) = slint::Model::as_any(old).downcast_ref::<VecModel<T>>() else { return };
        let rows: Vec<T> = (0..slint::Model::row_count(new)).filter_map(|i| slint::Model::row_data(new, i)).collect();
        crate::controller::sync_model(vm, rows);
        *new = old.clone();
    }
    if old.id != new.id || !old.builtin || !new.builtin {
        return;
    }
    keep(&old.knobs, &mut new.knobs);
    keep(&old.picks, &mut new.picks);
    keep(&old.sliders, &mut new.sliders);
    keep(&old.buttons, &mut new.buttons);
    keep(&old.bands.bands, &mut new.bands.bands);
    keep(&old.bands.xovers, &mut new.bands.xovers);
    // Field zones are touch areas: a rebuilt one would drop its drag.
    keep(&old.canvas.zones, &mut new.canvas.zones);
    keep(&old.canvas.paths, &mut new.canvas.paths);
    keep(&old.canvas.texts, &mut new.canvas.texts);
    keep(&old.canvas.grips, &mut new.canvas.grips);
}

/// x of each visible dial-row cell (card relative, unscaled). The reference
/// spaces knobs evenly: the gap follows from the card width and the cell
/// count of the widest state, and the visible cells are centred (so a mode
/// with fewer knobs keeps the same spacing).
fn dial_xs(def: &DeviceDef, visible: &[&'static str]) -> Vec<(&'static str, f32)> {
    let (w, cells) = def.layout.widest_row();
    let gap = if cells > 1 { (w - 2.0 * PAD - cells as f32 * KNOB_CELL) / (cells - 1) as f32 } else { 0.0 };
    let n = visible.len() as f32;
    let x0 = (w - n * KNOB_CELL - (n - 1.0).max(0.0) * gap) / 2.0;
    visible.iter().enumerate().map(|(i, id)| (*id, x0 + i as f32 * (KNOB_CELL + gap))).collect()
}

/// Nearest detent within the snap distance of `norm` (plain value), if any.
fn snap_to_detent(p: &ParamDef, detents: &[f32], norm: f32, fine: bool) -> Option<f32> {
    let reach = if fine { DETENT_SNAP / 5.0 } else { DETENT_SNAP };
    detents.iter().copied().map(|v| (v, (p.to_normalized(v) - norm).abs())).filter(|(_, d)| *d <= reach).min_by(|a, b| a.1.total_cmp(&b.1)).map(|(v, _)| v)
}

/// Detent tick marks across a knob ring (32 x 32 view box).
fn detent_ticks(p: &ParamDef, detents: &[f32]) -> String {
    let mut out = String::new();
    for v in detents {
        let a = (-135.0 + p.to_normalized(*v) * 270.0).to_radians();
        let (s, c) = a.sin_cos();
        out.push_str(&format!("M{:.2} {:.2} L{:.2} {:.2} ", 16.0 + 13.0 * s, 16.0 - 13.0 * c, 16.0 + 16.0 * s, 16.0 - 16.0 * c));
    }
    out
}

/// Right edge of the view row (card relative, unscaled): left of the slider column(s).
fn view_right(def: &DeviceDef) -> f32 {
    let width = def.layout.width();
    let n = def.layout.sliders.iter().filter(|c| c.param().and_then(|id| def.param(id)).is_some()).count();
    if n == 0 { width - PAD } else { width - PAD - SLIDER_W - (n - 1) as f32 * (SLIDER_W + VIEW_GAP) - VIEW_GAP }
}

/// Plot area of a field w pt wide (px at our zoom): width and height.
fn field_plot_size(f: &Field, w: f32) -> (f32, f32) {
    let well_y = if f.caption.is_empty() { 0.0 } else { CAPTION_H };
    ((w - FIELD_GUTTER) * Z, (VIEW_BOTTOM - VIEW_Y - well_y - FIELD_TOP - FIELD_BOTTOM) * Z)
}

/// x of each dial-row knob cell (card relative, unscaled), cluster by cluster.
/// Field cards (the reference motion / response family) centre the dial row
/// and space its knobs evenly across the row's width instead.
fn knob_xs(def: &DeviceDef) -> Vec<(&'static str, f32)> {
    let mut x = PAD;
    let mut cells = Vec::new();
    for (ci, cluster) in def.layout.clusters.iter().enumerate() {
        if ci > 0 {
            x += CLUSTER_GAP - KNOB_GAP;
        }
        for c in cluster.iter() {
            cells.push((c, x));
            x += KNOB_CELL + KNOB_GAP;
        }
    }
    if matches!(def.layout.view, Some(View::Custom(_))) && cells.len() > 1 {
        // Custom bodies spread the cells across the whole card (fixed-width cards too).
        let step = (def.layout.width() - 2.0 * PAD - KNOB_CELL) / (cells.len() - 1) as f32;
        for (k, cell) in cells.iter_mut().enumerate() {
            cell.1 = PAD + k as f32 * step;
        }
    } else if matches!(def.layout.view, Some(View::Field(_))) && cells.len() > 1 {
        let row = x - KNOB_GAP - PAD;
        let (start, step) = ((def.layout.width() - row) / 2.0, (row - KNOB_CELL) / (cells.len() - 1) as f32);
        for (k, cell) in cells.iter_mut().enumerate() {
            cell.1 = start + k as f32 * step;
        }
    }
    // A swap cell holds several parameters; the shown one is drawn.
    cells.into_iter().flat_map(|(c, x)| c.params().into_iter().map(move |p| (p, x))).collect()
}

fn fmt_db(v: f32) -> String {
    let a = (v.abs() * 10.0).round() / 10.0;
    if a < 0.05 { "0.0 dB".into() } else if v < 0.0 { format!("{MINUS}{a:.1} dB") } else { format!("{a:.1} dB") }
}

/// Points as Slint path commands ("M x y L x y ...").
fn polyline(pts: &[(f32, f32)]) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(pts.len() * 16);
    for (i, (x, y)) in pts.iter().enumerate() {
        let _ = write!(out, "{}{x:.1} {y:.1}", if i == 0 { "M" } else { " L" });
    }
    out
}

/// Dashed horizontal line from x0 to x1 at y (dash / gap lengths).
fn dashes(x0: f32, x1: f32, y: f32, dash: f32, gap: f32) -> String {
    let mut out = String::new();
    let mut x = x0;
    while x < x1 {
        out.push_str(&format!("M{x:.2} {y:.2} L{:.2} {y:.2} ", (x + dash).min(x1)));
        x += dash + gap;
    }
    out
}

/// A polyline cut into dashes (on / off lengths measured along the line).
fn dashed(pts: &[(f32, f32)], on: f32, off: f32) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    let (mut drawing, mut left) = (true, on);
    for seg in pts.windows(2) {
        let ((x0, y0), (x1, y1)) = (seg[0], seg[1]);
        let len = (x1 - x0).hypot(y1 - y0);
        let mut t = 0.0;
        while t < len {
            let step = left.min(len - t);
            let (a, b) = (t / len.max(1e-6), (t + step) / len.max(1e-6));
            if drawing {
                let _ = write!(out, "M{:.1} {:.1} L{:.1} {:.1} ", x0 + (x1 - x0) * a, y0 + (y1 - y0) * a, x0 + (x1 - x0) * b, y0 + (y1 - y0) * b);
            }
            t += step;
            left -= step;
            if left <= 1e-6 {
                drawing = !drawing;
                left = if drawing { on } else { off };
            }
        }
    }
    out
}

/// Gain reduction (dB, <= 0) of the quadratic soft-knee gain computer.
pub fn soft_knee_gr(x: f32, threshold: f32, ratio: f32, knee: f32) -> f32 {
    let slope = 1.0 / ratio.max(1.0) - 1.0;
    let over = x - threshold;
    if knee > 0.0 && over.abs() <= knee / 2.0 {
        slope * (over + knee / 2.0).powi(2) / (2.0 * knee)
    } else if over > 0.0 {
        slope * over
    } else {
        0.0
    }
}

/// Transfer-face paths in a 32 x 32 box: (curve, knee highlight, dashed unity line).
fn soft_knee_face(threshold: f32, ratio: f32, knee: f32) -> (String, String, String) {
    let lo = threshold - 12.0;
    let hi = (threshold + 24.0).max(0.0);
    let (pad, span) = (3.0, 26.0);
    let px = |db: f32| pad + (db - lo) / (hi - lo) * span;
    let py = |db: f32| 32.0 - pad - ((db - lo) / (hi - lo) * span).clamp(-pad, span + pad);
    let point = |x: f32| (px(x), py(x + soft_knee_gr(x, threshold, ratio, knee)));
    let n = 48;
    let curve: Vec<(f32, f32)> = (0..=n).map(|i| point(lo + (hi - lo) * i as f32 / n as f32)).collect();
    let margin = 3.0 / span * (hi - lo);
    let (b0, b1) = ((threshold - knee / 2.0 - margin).max(lo), (threshold + knee / 2.0 + margin).min(hi));
    let bend: Vec<(f32, f32)> = (0..=16).map(|i| point(b0 + (b1 - b0) * i as f32 / 16.0)).collect();
    let mut unity = String::new();
    let mut t = 0.0;
    while t < span {
        let t1 = (t + 2.0).min(span);
        unity.push_str(&format!("M{:.2} {:.2} L{:.2} {:.2} ", pad + t, 32.0 - pad - t, pad + t1, 32.0 - pad - t1));
        t += 4.0;
    }
    (polyline(&curve), polyline(&bend), unity)
}

/// Face paths of a device-computed transfer curve in a 32 x 32 box: (curve,
/// bend highlight, dashed unity line), laid out like [soft_knee_face].
pub(crate) fn transfer_face(t: &Transfer) -> (String, String, String) {
    let (lo, hi) = (t.lo, t.hi.max(t.lo + 1.0));
    let (pad, span) = (3.0, 26.0);
    let px = |db: f32| pad + (db - lo) / (hi - lo) * span;
    let py = |db: f32| 32.0 - pad - ((db - lo) / (hi - lo) * span).clamp(-pad, span + pad);
    let curve: Vec<(f32, f32)> = t.points.iter().map(|(x, y)| (px(*x), py(*y))).collect();
    let bend: Vec<(f32, f32)> = t.points.iter().filter(|(x, _)| *x >= t.bend.0 && *x <= t.bend.1).map(|(x, y)| (px(*x), py(*y))).collect();
    let mut unity = String::new();
    let mut s = 0.0;
    while s < span {
        let s1 = (s + 2.0).min(span);
        unity.push_str(&format!("M{:.2} {:.2} L{:.2} {:.2} ", pad + s, 32.0 - pad - s, pad + s1, 32.0 - pad - s1));
        s += 4.0;
    }
    (polyline(&curve), if bend.len() > 1 { polyline(&bend) } else { String::new() }, unity)
}

/// Strip paths for a level history in a w x wh well: [input fill, gain-reduction
/// band (input to input + GR), input line, output line].
fn strip_paths(slots: &VecDeque<(f32, f32, f32)>, strip: Strip, w: f32, wh: f32) -> [String; 4] {
    let span = strip.top_db - strip.floor_db;
    let y_of = |db: f32| ((strip.top_db - db) / span).clamp(0.0, 1.0) * wh;
    let n = slots.len();
    let x_of = |i: usize| w - (n - 1 - i) as f32 * w / (HISTORY - 1) as f32;
    let input: Vec<(f32, f32)> = slots.iter().enumerate().map(|(i, s)| (x_of(i), y_of(s.0))).collect();
    let output: Vec<(f32, f32)> = slots.iter().enumerate().map(|(i, s)| (x_of(i), y_of(s.1))).collect();
    let audible = |v: &[(f32, f32)]| v.iter().any(|p| p.1 < wh - 0.5);
    let (mut in_fill, mut in_line, mut gr_fill, mut out_line) = (String::new(), String::new(), String::new(), String::new());
    if n > 1 && audible(&input) {
        in_line = polyline(&input);
        in_fill = format!("{in_line} L{:.2} {wh:.2} L{:.2} {wh:.2} Z", input[n - 1].0, input[0].0);
    }
    if n > 1 && slots.iter().any(|s| s.2 < -0.1) {
        let mut area = input.clone();
        area.extend(slots.iter().enumerate().rev().map(|(i, s)| (x_of(i), y_of(s.0 + s.2))));
        gr_fill = format!("{} Z", polyline(&area));
    }
    if n > 1 && audible(&output) {
        out_line = polyline(&output);
    } else if in_line.is_empty() {
        // Silence: the level trace rests on the floor.
        in_line = polyline(&[(30.0 * Z, wh - 0.5), (w, wh - 0.5)]);
    }
    [in_fill, gr_fill, in_line, out_line]
}

/// The command for one parameter edit: with the device's linked writes
/// (Character Mod circuit switching) it becomes one labelled batch.
fn param_command(def: &DeviceDef, d: &Device, p: &ParamDef, value: f32) -> Command {
    let set = Command::SetDeviceParam { device: d.id, param: p.id.to_string(), value };
    let (writes, label) = saine_core::builtin::linked_writes(def, d, p.id, value);
    if writes.is_empty() {
        return set;
    }
    let mut commands = vec![set];
    commands.extend(writes.into_iter().map(|(id, v)| Command::SetDeviceParam { device: d.id, param: id.to_string(), value: v }));
    Command::Batch { label: label.unwrap_or_else(|| "Device Parameter Change".into()), commands }
}

impl Controller {
    pub(crate) fn panel_def(&self, id: DeviceId) -> Option<(&Device, &'static DeviceDef)> {
        let d = self.project().device(id)?;
        Some((d, saine_core::builtin::def_of(d)?))
    }

    /// Value shown for a parameter: the live drag value, else the model value.
    pub(crate) fn panel_value(&self, d: &Device, def: &DeviceDef, p: &ParamDef) -> f32 {
        if let Some(v) = def.index_of(p.id).and_then(|i| self.sampler_live(d.id, i).or_else(|| self.graph_drag_value(d.id, i))) {
            return v;
        }
        let is = |i: usize| def.params.get(i).is_some_and(|q| q.id == p.id);
        if let Some(g) = self.devices.panel.drag.filter(|g| g.device == d.id && is(g.index)) {
            return g.value;
        }
        if let Some(f) = self.devices.panel.field.filter(|f| f.device == d.id)
            && let Some((_, _, _, v)) = f.axes.iter().flatten().find(|a| is(a.0))
        {
            return *v;
        }
        def.value(d, p)
    }

    pub(crate) fn panel_value_of(&self, d: &Device, def: &DeviceDef, p: &ParamDef) -> f32 {
        self.panel_value(d, def, p)
    }

    /// The device as the panel shows it: the model values, with a running
    /// drag's value and the values it links applied.
    fn live_device<'a>(&self, d: &'a Device, def: &DeviceDef) -> Cow<'a, Device> {
        match self.devices.panel.drag {
            Some(g) if g.device == d.id && g.active => {
                let Some(p) = def.params.get(g.index) else { return Cow::Borrowed(d) };
                let mut live = d.clone();
                live.params.extend(def.linked_values(d, p.id, g.value));
                Cow::Owned(live)
            }
            _ => Cow::Borrowed(d),
        }
    }

    /// Transfer curve of a `Face::Transfer` knob.
    fn device_transfer(&self, live: &Device, def: &DeviceDef) -> Option<Transfer> {
        saine_core::builtin::behavior(def).and_then(|b| (b.transfer)(def, live)).or_else(|| self.view_transfer(live, def))
    }

    /// The card of a built-in device (generic panel).
    pub(crate) fn builtin_card(&self, d: &Device, def: &'static DeviceDef) -> DeviceCard {
        let width = def.layout.width();
        let track = self.project().find_device(d.id).map(|(ti, _)| &self.project().tracks[ti]);
        let accent = track.map_or(slint::Color::from_rgb_u8(0x43, 0x9b, 0x94), |t| slint_color(track_color(t.color)));
        let idx = |id: &str| def.index_of(id).map_or(-1, |i| i as i32);
        let live = self.live_device(d, def);
        let live = live.as_ref();
        let value = |id: &str| def.param(id).map(|p| self.panel_value(d, def, p));

        // Picker row; the sidechain picker and everything after it align right.
        let mut picks = Vec::new();
        if let Some(pick) = self.view_picker(d, def) {
            picks.push(pick);
        }
        for (ci, c) in def.layout.picker.iter().enumerate() {
            let p = c.param().and_then(|id| def.param(id));
            match (c, p) {
                // Latches align right (reference: margin-left auto); the label carries the state.
                (Control::Toggle(_), Some(p)) if def.is_shown(d, p.id) => {
                    if !picks.iter().any(|q: &PanelPick| q.kind == 4) {
                        picks.push(PanelPick { kind: 4, ..Default::default() });
                    }
                    let on = self.panel_value(d, def, p) >= 0.5;
                    picks.push(PanelPick {
                        kind: 5,
                        param: idx(p.id),
                        label: s(format!("{} {}", p.label, if on { "On" } else { "Off" })),
                        selected: on as i32,
                        enabled: true,
                        ..Default::default()
                    });
                }
                (Control::Menu(_), Some(p)) if def.is_shown(d, p.id) => picks.push(PanelPick {
                    kind: 1,
                    param: idx(p.id),
                    label: s(p.label),
                    choices: choices_model(def, p),
                    selected: self.panel_value(d, def, p).round() as i32,
                    enabled: true,
                    ..Default::default()
                }),
                (Control::Sidechain, _) => {
                    picks.push(PanelPick { kind: 4, ..Default::default() });
                    let source = match d.sidechain {
                        None => "None".to_string(),
                        Some(t) => self.project().track(t).map_or_else(|| "Removed".to_string(), |t| t.name.clone()),
                    };
                    // Without a value box after it the source pill takes the room (reference).
                    let w = if def.layout.picker[ci + 1..].is_empty() { 140.0 } else { 80.0 };
                    picks.push(PanelPick { kind: 2, param: -1, label: s("Sidechain"), value: s(source), enabled: true, width: w * Z, ..Default::default() });
                }
                (Control::Segments(_, labels), Some(p)) if def.is_shown(d, p.id) => picks.push(PanelPick {
                    kind: 1,
                    param: idx(p.id),
                    label: s(p.label),
                    choices: labels_model(def, p, labels),
                    selected: self.panel_value(d, def, p).round() as i32,
                    enabled: true,
                    ..Default::default()
                }),
                (Control::Dropdown(_), Some(p)) if def.is_shown(d, p.id) => picks.push(PanelPick {
                    kind: 2,
                    param: idx(p.id),
                    label: s(p.label),
                    value: s(def.text(live, p, self.panel_value(d, def, p))),
                    enabled: true,
                    width: dropdown_width(p) * Z,
                    ..Default::default()
                }),
                (Control::Gap, _) => picks.push(PanelPick { kind: 4, ..Default::default() }),
                (Control::Pill(_), Some(p)) if def.is_shown(d, p.id) => picks.push(PanelPick {
                    kind: 3,
                    param: idx(p.id),
                    label: s(p.label),
                    value: s(def.text(live, p, self.panel_value(d, def, p))),
                    // A pill after the sidechain picker scales the key: inert without a source.
                    enabled: d.sidechain.is_some() || !def.layout.picker[..ci].contains(&Control::Sidechain),
                    width: 60.0 * Z,
                    ..Default::default()
                }),
                _ => {}
            }
        }
        // Without a gap the row's items stay on the left.
        if !picks.iter().any(|p| p.kind == 4) {
            picks.push(PanelPick { kind: 4, ..Default::default() });
        }

        // Slider column.
        let slider_x = width - PAD - SLIDER_W;
        let sliders: Vec<PanelSlider> = def
            .layout
            .sliders
            .iter()
            .filter_map(|c| c.param().and_then(|id| def.param(id)))
            .enumerate()
            .map(|(i, p)| {
                let v = self.panel_value(d, def, p);
                PanelSlider {
                    param: idx(p.id),
                    x: (slider_x - i as f32 * (SLIDER_W + VIEW_GAP)) * Z,
                    label: s(p.label),
                    value: s(def.text(live, p, v)),
                    norm: p.to_normalized(v),
                    zero: p.origin(),
                }
            })
            .collect();
        let view_right = view_right(def);

        // View row.
        let (mut strip, mut field) = (PanelStrip::default(), PanelField::default());
        match def.layout.view {
            Some(View::Field(f)) => field = self.field_view(d, def, &f, view_right - PAD),
            Some(v) => {
                if let Some(st) = v.strip() {
                    strip = self.dynamics_strip(d, def, st, view_right - PAD);
                }
            }
            None => {}
        }
        let bands = self.view_field(d, def, live);
        let body = match def.layout.view {
            Some(View::Custom(name)) => {
                let view_y = if def.layout.picker.is_empty() { VIEW_Y_NO_PICKER } else { VIEW_Y };
                self.custom_body(d, def, name, (PAD, view_y, view_right - PAD, VIEW_BOTTOM - view_y))
            }
            _ => crate::device_views::CustomBody::default(),
        };

        // Dial row (or the custom body's knob placement), bound to the view's selection.
        let visible: Vec<&'static str> = def.layout.clusters.iter().flat_map(|c| c.iter()).flat_map(|c| c.params()).filter(|id| def.is_shown(d, id)).collect();
        // Field cards centre their own dial row (knob_xs); the rest spread evenly across the card.
        let places: Vec<(&'static str, f32, f32)> = if !body.knobs.is_empty() {
            body.knobs.clone()
        } else if matches!(def.layout.view, Some(View::Field(_))) {
            knob_xs(def).into_iter().map(|(id, x)| (id, x, DIAL_Y)).collect()
        } else {
            dial_xs(def, &visible).into_iter().map(|(id, x)| (id, x, DIAL_Y)).collect()
        };
        let behavior = saine_core::builtin::behavior(def);
        let knobs = places
            .into_iter()
            .filter(|(id, _, _)| def.is_shown(d, id))
            .filter_map(|(slot, x, y)| {
                let id = self.view_bind(d, def, slot);
                let p = def.param(id)?;
                let v = self.panel_value(d, def, p);
                let detents = def.detents(live, p);
                let mut k = PanelKnob {
                    param: idx(id),
                    x: x * Z,
                    y: y * Z,
                    label: s(p.label),
                    value: s(def.text(live, p, v)),
                    norm: p.to_normalized(v),
                    origin: p.origin(),
                    ticks: s(detent_ticks(p, detents)),
                    dim: body.dim.contains(&id),
                    ..Default::default()
                };
                if let Some(l) = behavior.and_then(|b| (b.latch)(def, live, p)) {
                    k.face = 2;
                    k.value = s(l.labels[l.on as usize]);
                    k.norm = if l.on { 1.0 } else { 0.0 };
                    k.ticks = s("");
                    return Some(k);
                }
                if def.layout.clusters.iter().any(|c| c.contains(&Control::Toggle(id))) {
                    k.face = 2; // latch capsule
                }
                match def.layout.faces.iter().find(|(f, _)| *f == slot).map(|(_, f)| *f) {
                    Some(Face::SoftKneeCurve { threshold, ratio, knee }) => {
                        let (curve, bend, unity) = soft_knee_face(value(threshold).unwrap_or(-24.0), value(ratio).unwrap_or(1.0), value(knee).unwrap_or(0.0));
                        k.face = 1;
                        k.face_curve = s(curve);
                        k.face_bend = s(bend);
                        k.face_unity = s(unity);
                    }
                    Some(Face::Transfer) => {
                        if let Some(t) = self.device_transfer(live, def) {
                            let (curve, bend, unity) = transfer_face(&t);
                            k.face = 1;
                            k.face_curve = s(curve);
                            k.face_bend = s(bend);
                            k.face_unity = s(unity);
                        }
                    }
                    None => {}
                }
                Some(k)
            })
            .collect();

        let preset = match &d.preset {
            None => String::new(),
            Some(name) => {
                let dirty = def.factory.iter().find(|f| f.name == name).is_some_and(|f| {
                    let want = def.factory_values(f);
                    // The sidechain switch follows the panel's source pick, not the preset.
                    def.params.iter().filter(|p| p.id != "sidechainEnable").any(|p| (def.value(d, p) - want.get(p.id).copied().unwrap_or(p.default)).abs() > 1e-4 * (p.max - p.min))
                });
                if dirty { format!("{name}*") } else { name.clone() }
            }
        };

        let mut card = DeviceCard {
            id: d.id.0 as i32,
            name: s(def.name),
            vendor: s(saine_core::builtin::VENDOR),
            format: s("Built-in"),
            kind: s("Audio effect"),
            enabled: d.enabled,
            builtin: true,
            width: width * Z,
            accent,
            preset: s(preset),
            knobs: model(knobs),
            picks: model(picks),
            sliders: model(sliders),
            strip,
            field,
            bands,
            canvas: body.canvas,
            buttons: model(body.buttons),
            ..Default::default()
        };
        if let Some(View::Custom(name)) = def.layout.view {
            self.custom_card(name, d, def, &mut card);
        }
        if crate::sampler_panel::is_sampler(def) {
            return self.sampler_card(d, def, card);
        }
        if matches!(def.layout.view, Some(View::Custom(_))) {
            self.mod_body(d, def, &mut card);
        }
        card
    }

    fn dynamics_strip(&self, d: &Device, def: &DeviceDef, st: Strip, w: f32) -> PanelStrip {
        let (w, h) = (w * Z, (VIEW_BOTTOM - VIEW_Y) * Z);
        let well_y = CAPTION_H * Z;
        let wh = h - well_y;
        let span = st.top_db - st.floor_db;
        let y_of = |db: f32| ((st.top_db - db) / span).clamp(0.0, 1.0) * wh;
        let first = (st.top_db / 12.0).floor() * 12.0;
        let ticks: Vec<PanelTick> = (0..=((first - st.floor_db) / 12.0) as i32)
            .map(|i| {
                let db = first - 12.0 * i as f32;
                let text = if db == 0.0 { "0".to_string() } else if db < 0.0 { format!("{MINUS}{}", db.abs() as i32) } else { format!("+{}", db as i32) };
                PanelTick { y: y_of(db), text: s(text), rule: db < st.top_db && db > st.floor_db }
            })
            .collect();

        let empty = MeterHistory::default();
        let hist = self.devices.panel.meters.get(&d.id).unwrap_or(&empty);
        let [in_fill, gr_fill, in_line, out_line] = strip_paths(&hist.slots, st, w, wh);
        let gr_now = hist.slots.iter().rev().take(30).map(|s| s.2).fold(0.0f32, f32::min);

        let p = def.param(st.line);
        let t = p.map(|p| self.panel_value(d, def, p)).unwrap_or(st.floor_db);
        let ty = y_of(t).min(wh - 1.0);
        PanelStrip {
            visible: true,
            x: PAD * Z,
            y: VIEW_Y * Z,
            w,
            h,
            well_y,
            caption: s("Reduction"),
            readout: s(fmt_db(gr_now)),
            ticks: TICK_MODELS.with(|m| m.borrow_mut().entry(format!("{}/{wh}", def.id)).or_insert_with(|| model(ticks)).clone()),
            in_fill: s(in_fill),
            gr_fill: s(gr_fill),
            in_line: s(in_line),
            out_line: s(out_line),
            thresh_param: def.index_of(st.line).map_or(-1, |i| i as i32),
            thresh_y: ty,
            thresh_dash: s(dashes(30.0 * Z, w, ty, 6.0 * Z, 5.0 * Z)),
            thresh_label: s(p.map_or("", |p| p.label)),
            thresh_value: s(p.map(|p| p.format(t)).unwrap_or_default()),
            // The line follows the pointer: (dB per px) / (dB per unit of the 0..1 range).
            thresh_rate: p.map_or(0.0, |p| span / wh / (p.max - p.min)),
        }
    }

    /// The first of a field axis' parameters that is shown (Rate or Rate Sync).
    fn field_axis(d: &Device, def: &DeviceDef, ids: &[&'static str]) -> Option<usize> {
        ids.iter().find(|id| def.is_shown(d, id)).and_then(|id| def.index_of(id))
    }

    /// A plotted field (View::Field) w pt wide: caption, ticks, traces, fill / band, live overlay.
    fn field_view(&self, d: &Device, def: &DeviceDef, f: &Field, w: f32) -> PanelField {
        let (pw, ph) = field_plot_size(f, w);
        let (w, h) = (w * Z, (VIEW_BOTTOM - VIEW_Y) * Z);
        let well_y = if f.caption.is_empty() { 0.0 } else { CAPTION_H * Z };
        let (px, py) = (FIELD_GUTTER * Z, FIELD_TOP * Z);
        let get = |id: &str| def.param(id).map_or(0.0, |p| self.panel_value(d, def, p));
        let input = device_fields::Input {
            get: &get,
            bpm: self.project().transport.bpm as f32,
            n: (pw.round() as usize).max(64),
            cycles: self.devices.panel.lfo.get(&d.id).map(|l| l.1),
        };
        let Some(plot) = device_fields::plot(f.plot, &input) else { return PanelField::default() };
        let at = |x: f32, y: f32| (px + x * pw, py + y * ph);
        let points = |ys: &[f32]| -> Vec<(f32, f32)> { ys.iter().enumerate().map(|(k, y)| at(k as f32 / (ys.len() - 1).max(1) as f32, *y)).collect() };
        let stroke = |t: &device_fields::Trace| match t.style {
            Style::Solid => polyline(&points(&t.ys)),
            Style::Dash { on, off, .. } => dashed(&points(&t.ys), on * Z, off * Z),
        };
        let alpha = |t: Option<&device_fields::Trace>| match t.map(|t| t.style) {
            Some(Style::Dash { alpha, .. }) => alpha,
            _ => 1.0,
        };
        let main = plot.traces.first().map(|t| points(&t.ys)).unwrap_or_default();
        let rest_y = plot.rest.map(|r| at(0.0, r).1);
        let fill = match (plot.fill, rest_y) {
            (Fill::ToRest, Some(ry)) if main.len() > 1 => format!("{} L{:.2} {ry:.2} L{:.2} {ry:.2} Z", polyline(&main), main[main.len() - 1].0, main[0].0),
            (Fill::Band, _) => {
                let (lo, hi) = plot.traces.iter().flat_map(|t| t.ys.iter()).fold((f32::MAX, f32::MIN), |(a, b), y| (a.min(*y), b.max(*y)));
                let (y0, y1) = (at(0.0, lo).1, at(0.0, hi).1);
                format!("M{px:.2} {y0:.2} L{:.2} {y0:.2} L{:.2} {y1:.2} L{px:.2} {y1:.2} Z", px + pw, px + pw)
            }
            _ => String::new(),
        };
        let ticks: Vec<PanelTick> = plot.ticks.iter().map(|(y, text)| PanelTick { y: at(0.0, *y).1, text: s(text.as_str()), rule: true }).collect();
        let (mut live, mut dot, mut dot_x, mut dot_y) = (String::new(), false, 0.0, 0.0);
        match &plot.live {
            Some(Live::Curve(ys)) => live = polyline(&points(ys)),
            Some(Live::Dot { x, y, trail }) => {
                (dot, (dot_x, dot_y)) = (true, at(*x, *y));
                if trail.len() > 1 {
                    live = polyline(&trail.iter().map(|(x, y)| at(*x, *y)).collect::<Vec<_>>());
                }
            }
            None => {}
        }
        let xi = Self::field_axis(d, def, f.x);
        let yi = Self::field_axis(d, def, f.y);
        PanelField {
            visible: true,
            x: PAD * Z,
            y: VIEW_Y * Z,
            w,
            h,
            well_y,
            caption: s(f.caption),
            readout: s(plot.readout),
            ticks: model(ticks),
            rest: s(rest_y.map(|y| format!("M{px:.2} {y:.2} L{:.2} {y:.2}", px + pw)).unwrap_or_default()),
            fill: s(fill),
            band: plot.fill == Fill::Band,
            main: s(plot.traces.first().map(stroke).unwrap_or_default()),
            main_w: plot.widths.0 * Z,
            main_hot_w: plot.widths.1 * Z,
            alt: s(plot.traces.get(1).map(stroke).unwrap_or_default()),
            alt_alpha: alpha(plot.traces.get(1)),
            third: s(plot.traces.get(2).map(stroke).unwrap_or_default()),
            third_alpha: alpha(plot.traces.get(2)),
            live: s(live),
            dot,
            dot_x,
            dot_y,
            x_param: xi.map_or(-1, |i| i as i32),
            y_param: yi.map_or(-1, |i| i as i32),
            plot_x: px,
            plot_y: well_y + py,
            plot_w: pw,
            plot_h: ph,
        }
    }

    /// Engine readouts -> strip histories, view meters and field LFO phases of
    /// the selected track's built-in devices (every poll tick, ~30 ms).
    pub(crate) fn panel_poll_meters(&mut self) {
        self.sampler_poll();
        let Some(t) = self.selected_track.and_then(|t| self.project().track(t)) else { return };
        let strips: Vec<(DeviceId, Strip)> =
            t.devices.iter().filter_map(|d| saine_core::builtin::def_of(d).and_then(|def| def.layout.view).and_then(|v| v.strip()).map(|st| (d.id, st))).collect();
        // Field devices: follow the LFO phase readout while the transport plays.
        let fields: Vec<DeviceId> = if self.engine.transport().playing {
            t.devices.iter().filter(|d| saine_core::builtin::def_of(d).is_some_and(|def| matches!(def.layout.view, Some(View::Field(_))))).map(|d| d.id).collect()
        } else {
            Vec::new()
        };
        let views: Vec<DeviceId> = t
            .devices
            .iter()
            .filter(|d| saine_core::builtin::def_of(d).is_some_and(|def| matches!(def.layout.view, Some(View::Custom(_)))))
            .map(|d| d.id)
            .collect();
        self.devices.panel.meters.retain(|id, _| strips.iter().any(|(s, _)| s == id));
        let mut out = Vec::new();
        for (id, st) in strips {
            // Readout order of a strip device: input dB, output dB, gain reduction dB.
            let (i, o, g) = match self.engine.builtin_readouts(id.0, &mut out) {
                Ok(()) if out.len() >= 3 => (out[0], out[1], out[2].min(0.0)),
                _ => (SILENT_DB, SILENT_DB, 0.0),
            };
            let h = self.devices.panel.meters.entry(id).or_default();
            let silent = |s: (f32, f32, f32)| s.0 < st.floor_db && s.1 < st.floor_db && s.2 > -0.05;
            let prev = h.slots.back().copied().unwrap_or((st.floor_db - 1.0, st.floor_db - 1.0, 0.0));
            let next = (0.3 * prev.0 + 0.7 * i, 0.3 * prev.1 + 0.7 * o, 0.3 * prev.2 + 0.7 * g);
            if silent(next) && h.slots.iter().all(|s| silent(*s)) {
                // Flat at the floor: nothing to scroll.
                h.slots.clear();
                continue;
            }
            h.last = (i, o, g);
            if h.slots.len() == HISTORY {
                h.slots.pop_front();
            }
            h.slots.push_back(next);
            self.ui_dirty = true;
        }
        // View state (the selected band) lives as long as its device; meters only poll here.
        let gone: Vec<DeviceId> = self.devices.panel.views.keys().copied().filter(|id| self.project().device(*id).is_none()).collect();
        self.devices.panel.views.retain(|id, _| !gone.contains(id));
        for id in views {
            let r = self.engine.builtin_readouts(id.0, &mut out).ok().map(|_| out.clone()).unwrap_or_default();
            if self.devices.panel.views.entry(id).or_default().poll(&r) {
                self.ui_dirty = true;
            }
        }
        self.devices.panel.lfo.retain(|id, _| fields.contains(id));
        for id in fields {
            let Ok(()) = self.engine.builtin_readouts(id.0, &mut out) else { continue };
            let Some(phase) = out.first().copied().filter(|p| p.is_finite()) else { continue };
            let l = self.devices.panel.lfo.entry(id).or_insert((phase, phase as f64));
            // Polled every ~30 ms: the forward step since the last poll (under one cycle).
            l.1 += (phase - l.0).rem_euclid(1.0) as f64;
            l.0 = phase;
            self.ui_dirty = true;
        }
    }

    // ---------------------------------------------------------------- gestures

    pub fn panel_drag(&mut self, device: DeviceId, index: i32, phase: i32, dy: f32, rate: f32, fine: bool) {
        if phase == 0 && self.mod_map_click(device, index) {
            return;
        }
        if index <= crate::sampler_panel::SLICE_GAIN {
            return self.slice_drag(device, index, phase, dy, rate, fine);
        }
        if self.project().device(device).is_some_and(|d| saine_core::builtin::def_of(d).is_none()) {
            return self.card_drag(device, index as u32, phase, dy, rate, fine);
        }
        let Some((d, def)) = self.panel_def(device).map(|(d, def)| (d.clone(), def)) else { return };
        let d = &d;
        let Some(p) = usize::try_from(index).ok().and_then(|i| def.params.get(i)) else { return };
        let rack = saine_core::modulation::is_rack(d);
        match phase {
            0 => {
                let v = def.value(d, p);
                self.devices.panel.drag = Some(ParamDrag { device, index: index as usize, start: v, norm: p.to_normalized(v), value: v, travel: 0.0, held: 0.0, active: false });
            }
            1 => {
                let Some(g) = self.devices.panel.drag.as_mut().filter(|g| g.device == device && g.index == index as usize) else { return };
                g.travel += dy.abs();
                g.held += dy;
                if !g.active && g.travel < DRAG_SLOP {
                    return;
                }
                g.active = true;
                g.norm = (g.norm + std::mem::take(&mut g.held) * rate * if fine { 0.2 } else { 1.0 }).clamp(0.0, 1.0);
                let v = if p.is_discrete() { p.from_normalized(g.norm).round() } else { p.from_normalized(g.norm) };
                let v = snap_to_detent(p, def.detents(d, p), g.norm, fine).unwrap_or(v);
                if v != g.value {
                    g.value = v;
                    // The engine gets the model's values with this edit (and what it links) on
                    // top, so links that stop applying mid-drag fall back; the model changes on release.
                    let mut live = def.values(d);
                    for (id, x) in def.linked_values(d, p.id, v) {
                        if let Some(i) = def.index_of(&id) {
                            live[i] = x;
                        }
                    }
                    for (i, x) in live.into_iter().enumerate() {
                        let _ = self.engine.builtin_set_param(device.0, i as u32, x);
                    }
                    self.ui_dirty = true;
                }
            }
            2 => {
                let clicked = self.devices.panel.drag.is_some_and(|g| g.device == device && !g.active);
                let Some(g) = self.devices.panel.drag.take() else { return };
                self.ui_dirty = true;
                // A click on a chain macro opens (or closes) its side panel.
                if rack && clicked {
                    return self.macro_select(device, index);
                }
                if g.active && g.value != g.start {
                    self.panel_commit(param_command(def, d, p, g.value));
                } else if g.active {
                    for (i, x) in def.values(d).into_iter().enumerate() {
                        let _ = self.engine.builtin_set_param(device.0, i as u32, x);
                    }
                }
            }
            _ => {
                self.panel_cancel_drag();
            }
        }
    }

    /// Parameter index of the running drag.
    pub(crate) fn panel_drag_index(&self) -> Option<i32> {
        self.devices.panel.drag.map(|g| g.index as i32)
    }

    /// True once the running drag has passed the start threshold.
    pub(crate) fn panel_drag_active(&self) -> bool {
        self.devices.panel.drag.is_some_and(|g| g.active)
    }

    /// Escape or a cancelled pointer: restore the values from before the drag
    /// (the dragged one and any it linked), with no undo step.
    pub fn panel_cancel_drag(&mut self) -> bool {
        self.sampler_release_all();
        if self.sampler_cancel_drags() || self.panel_cancel_graph_drag() {
            return true;
        }
        let Some(g) = self.devices.panel.drag.take() else { return false };
        if let Some((d, def)) = self.panel_def(g.device) {
            let values = def.values(d);
            for (i, v) in values.into_iter().enumerate() {
                let _ = self.engine.builtin_set_param(g.device.0, i as u32, v);
            }
        }
        self.ui_dirty = true;
        true
    }

    fn panel_set(&mut self, device: DeviceId, index: i32, value: impl FnOnce(&ParamDef, f32, &[f32]) -> Option<f32>) {
        if index <= crate::sampler_panel::SLICE_GAIN {
            return self.slice_set(device, index, |p, v| value(p, v, &[]));
        }
        let Some((d, def)) = self.panel_def(device) else { return };
        let Some(p) = usize::try_from(index).ok().and_then(|i| def.params.get(i)) else { return };
        let cur = def.value(d, p);
        if let Some(v) = value(p, cur, def.detents(d, p)).map(|v| p.snap(v))
            && v != cur
        {
            let cmd = param_command(def, d, p, v);
            self.panel_commit(cmd);
        }
    }

    /// Field drag: x and y move the field's two parameters at once (travel =
    /// plot width / height for the whole range; a stepped parameter walks its
    /// steps). Phases as panel_drag; dx / dy in px since the last event (up +).
    pub fn panel_field_drag(&mut self, device: DeviceId, phase: i32, dx: f32, dy: f32, fine: bool) {
        let Some((d, def)) = self.panel_def(device) else { return };
        let Some(View::Field(f)) = def.layout.view else { return };
        let axes = [Self::field_axis(d, def, f.x), Self::field_axis(d, def, f.y)];
        let (pw, ph) = field_plot_size(&f, view_right(def) - PAD);
        self.axes_drag(device, axes, phase, [dx, dy], [1.0 / pw.max(1.0), 1.0 / ph.max(1.0)], fine);
    }

    /// Drag zone of a custom body (View::Custom): params [horizontal, vertical]
    /// (-1 = unused), deltas since the last event (px, right / up +) and the
    /// norm-per-px rate of each axis.
    pub fn panel_drag_xy(&mut self, device: DeviceId, params: [i32; 2], phase: i32, delta: [f32; 2], rate: [f32; 2], fine: bool) {
        let Some((_, def)) = self.panel_def(device) else { return };
        let axes = params.map(|i| usize::try_from(i).ok().filter(|&i| i < def.params.len()));
        self.axes_drag(device, axes, phase, delta, rate, fine);
    }

    /// Two-axis drag (fields and zones): live values go to the engine; release
    /// commits one command (a Batch when both axes moved): one undo step.
    fn axes_drag(&mut self, device: DeviceId, axes: [Option<usize>; 2], phase: i32, delta: [f32; 2], rate: [f32; 2], fine: bool) {
        let Some((d, def)) = self.panel_def(device) else { return };
        match phase {
            0 => {
                let axis = |i: Option<usize>| {
                    i.map(|i| {
                        let v = def.value(d, &def.params[i]);
                        (i, v, def.params[i].to_normalized(v), v)
                    })
                };
                self.devices.panel.field = Some(FieldDrag { device, axes: [axis(axes[0]), axis(axes[1])], travel: 0.0, active: false });
            }
            1 => {
                let Some(g) = self.devices.panel.field.as_mut().filter(|g| g.device == device) else { return };
                g.travel += delta[0].abs() + delta[1].abs();
                if !g.active && g.travel < DRAG_SLOP {
                    return;
                }
                g.active = true;
                let k = if fine { 0.2 } else { 1.0 };
                let mut sends = Vec::new();
                for (a, (dl, r)) in g.axes.iter_mut().zip(delta.into_iter().zip(rate)) {
                    let Some((i, _, norm, value)) = a else { continue };
                    let p = &def.params[*i];
                    *norm = (*norm + dl * r * k).clamp(0.0, 1.0);
                    let v = if p.is_discrete() { p.from_normalized(*norm).round() } else { p.from_normalized(*norm) };
                    if v != *value {
                        *value = v;
                        sends.push((*i, v));
                    }
                }
                for (i, v) in sends {
                    let _ = self.engine.builtin_set_param(device.0, i as u32, v);
                    self.ui_dirty = true;
                }
            }
            2 => {
                let Some(g) = self.devices.panel.field.take() else { return };
                self.ui_dirty = true;
                let commands: Vec<Command> = g
                    .axes
                    .iter()
                    .flatten()
                    .filter(|a| g.active && a.3 != a.1)
                    .map(|a| Command::SetDeviceParam { device, param: def.params[a.0].id.to_string(), value: a.3 })
                    .collect();
                match commands.len() {
                    0 => {}
                    1 => {
                        self.panel_commit(commands.into_iter().next().unwrap());
                    }
                    _ => {
                        self.panel_commit(Command::Batch { label: "Device Parameter Change".into(), commands });
                    }
                }
            }
            _ => {
                if let Some(g) = self.devices.panel.field.take() {
                    for a in g.axes.iter().flatten() {
                        let _ = self.engine.builtin_set_param(device.0, a.0 as u32, a.1);
                    }
                    self.ui_dirty = true;
                }
            }
        }
    }

    /// One gesture = one undo step: never merge with the previous edit of the same parameter.
    pub(crate) fn panel_commit(&mut self, cmd: Command) -> bool {
        let cmd = self.sampler_followups(cmd);
        self.doc.seal_coalescing();
        let ok = self.execute(cmd);
        self.doc.seal_coalescing();
        ok
    }

    pub fn panel_reset(&mut self, device: DeviceId, index: i32) {
        if self.project().device(device).is_some_and(|d| saine_core::builtin::def_of(d).is_none()) {
            return self.card_reset(device, index as u32);
        }
        self.panel_set(device, index, |p, _, _| Some(p.default));
    }

    /// Reset of a two-parameter field zone: one command (a Batch when both change), one undo step.
    pub fn panel_reset_xy(&mut self, device: DeviceId, params: [i32; 2]) {
        let Some((d, def)) = self.panel_def(device) else { return };
        let mut commands: Vec<Command> = params
            .iter()
            .filter_map(|&i| usize::try_from(i).ok().and_then(|i| def.params.get(i)))
            .filter(|p| def.value(d, p) != p.default)
            .map(|p| Command::SetDeviceParam { device, param: p.id.to_string(), value: p.default })
            .collect();
        match commands.len() {
            0 => {}
            1 => {
                self.panel_commit(commands.remove(0));
            }
            _ => {
                self.panel_commit(Command::Batch { label: "Device Parameter Change".into(), commands });
            }
        }
    }

    /// Arrow keys: discrete parameters move one choice, continuous ones 1% of
    /// the range (0.2% fine), or to the next detent when the control has them.
    pub fn panel_step(&mut self, device: DeviceId, index: i32, delta: i32, fine: bool) {
        if self.project().device(device).is_some_and(|d| saine_core::builtin::def_of(d).is_none()) {
            return self.card_step(device, index as u32, delta, fine);
        }
        self.panel_set(device, index, |p, v, detents| {
            if p.is_discrete() {
                return Some(v + delta as f32);
            }
            let tol = 1e-4 * (p.max - p.min);
            let next = if delta > 0 { detents.iter().copied().filter(|x| *x > v + tol).reduce(f32::min) } else { detents.iter().copied().filter(|x| *x < v - tol).reduce(f32::max) };
            if !detents.is_empty() {
                return next;
            }
            Some(p.from_normalized(p.to_normalized(v) + delta as f32 * if fine { 0.002 } else { 0.01 }))
        });
    }

    /// A choice (segment, menu row or latch state); `VIEW_SELECT` picks the
    /// card's view state instead of a parameter.
    pub fn panel_choice(&mut self, device: DeviceId, index: i32, choice: i32) {
        if self.project().device(device).is_some_and(|d| saine_core::builtin::def_of(d).is_none()) {
            return self.card_click(device, index as u32);
        }
        if index == VIEW_SELECT {
            self.panel_view(device, 1, choice as f32);
            return;
        }
        let latch = self.panel_def(device).and_then(|(d, def)| {
            let p = def.params.get(usize::try_from(index).ok()?)?;
            saine_core::builtin::behavior(def).and_then(|b| (b.latch)(def, d, p))
        });
        self.panel_set(device, index, |_, _, _| Some(latch.map_or(choice as f32, |l| l.values[choice.clamp(0, 1) as usize])));
    }

    pub fn panel_text(&mut self, device: DeviceId, index: i32, text: &str) {
        if self.project().device(device).is_some_and(|d| saine_core::builtin::def_of(d).is_none()) {
            return self.card_text(device, index as u32, text);
        }
        // Text the panel shows (e.g. "AUTO", "DIAL 3") committed unchanged is not an edit.
        let shown = self.panel_def(device).and_then(|(d, def)| {
            let p = def.params.get(usize::try_from(index).ok()?)?;
            Some(def.text(d, p, def.value(d, p)))
        });
        if shown.is_some_and(|s| s.trim() == text.trim()) {
            return;
        }
        let mut bad = None;
        let typed = usize::try_from(index).ok().and_then(|i| self.panel_def(device).and_then(|(_, def)| def.params.get(i))).and_then(|p| self.sampler_parse(device, p, text));
        self.panel_set(device, index, |p, _, _| {
            let v = typed.or_else(|| p.parse(text));
            if v.is_none() {
                bad = Some(format!("{}: can't read “{}”", p.name, text.trim()));
            }
            v
        });
        if let Some(msg) = bad {
            self.set_status(msg, 3);
        }
    }

    /// Sidechain picker (-1), presets (-2) or the choices of parameter `which` (>= 0) at window x, y.
    pub fn panel_menu(&mut self, device: DeviceId, which: i32, x: f32, y: f32) {
        if which >= 0 {
            return self.sampler_choice_menu(device, which as usize, x, y);
        }
        let Some((d, def)) = self.panel_def(device) else { return };
        let check = |on: bool| if on { "✓".to_string() } else { String::new() };
        let rows = match which {
            i if i >= 0 => {
                let Some(p) = def.params.get(i as usize).filter(|p| p.is_discrete()) else { return };
                let cur = self.panel_value(d, def, p).round() as i32;
                p.choices.iter().enumerate().map(|(c, name)| (name.to_string(), check(c as i32 == cur), Some(MenuPick::Choice(device, i, c as i32)), false)).collect()
            }
            -1 => {
                let own = self.project().find_device(device).map(|(ti, _)| self.project().tracks[ti].id);
                let mut rows = vec![("None".to_string(), check(d.sidechain.is_none()), Some(MenuPick::Sidechain(device, None)), false)];
                for t in self.project().tracks.iter().filter(|t| Some(t.id) != own && (d.sidechain == Some(t.id) || !self.sidechain_feeds_back(device, t.id))) {
                    rows.push((t.name.clone(), check(d.sidechain == Some(t.id)), Some(MenuPick::Sidechain(device, Some(t.id))), false));
                }
                rows
            }
            -2 => {
                let mut rows = vec![("Factory presets".to_string(), String::new(), None, false)];
                for (i, f) in def.factory.iter().enumerate() {
                    rows.push((f.name.to_string(), check(d.preset.as_deref() == Some(f.name)), Some(MenuPick::Preset(device, i)), false));
                }
                rows
            }
            // A dropdown pill: the parameter's choices.
            i => {
                let Some(p) = usize::try_from(i).ok().and_then(|i| def.params.get(i)) else { return };
                let cur = def.value(d, p).round() as usize;
                p.choices.iter().enumerate().map(|(k, c)| (c.to_string(), check(k == cur), Some(MenuPick::Choice(device, i, k as i32)), false)).collect()
            }
        };
        self.devices.menu = Some(DevMenu { pos: (x, y), w: 180.0, rows });
        self.ui_dirty = true;
    }

    /// True when `from` already listens to the track of `device` (directly or
    /// through other sidechains): picking it as a source would form a cycle.
    pub(crate) fn sidechain_feeds_back(&self, device: DeviceId, from: TrackId) -> bool {
        let own = self.project().find_device(device).map(|(ti, _)| self.project().tracks[ti].id);
        let mut stack = vec![from];
        let mut seen = std::collections::HashSet::new();
        while let Some(t) = stack.pop() {
            if Some(t) == own {
                return true;
            }
            if seen.insert(t)
                && let Some(tr) = self.project().track(t)
            {
                stack.extend(tr.devices.iter().filter_map(|x| x.sidechain));
            }
        }
        false
    }

    /// Previous / next factory preset (wraps; from no preset, next is the first).
    pub fn panel_preset_step(&mut self, device: DeviceId, dir: i32) {
        let Some((d, def)) = self.panel_def(device) else { return };
        let n = def.factory.len() as i32;
        if n == 0 {
            return;
        }
        let cur = d.preset.as_deref().and_then(|name| def.factory.iter().position(|f| f.name == name)).map(|i| i as i32);
        let next = match cur {
            Some(i) => (i + dir).rem_euclid(n),
            None if dir > 0 => 0,
            None => n - 1,
        };
        self.panel_load_preset(device, next as usize);
    }

    pub(crate) fn panel_load_preset(&mut self, device: DeviceId, n: usize) {
        let Some((d, def)) = self.panel_def(device) else { return };
        let Some(f) = def.factory.get(n) else { return };
        let mut params = saine_core::builtin::DevicePreset::factory(def, f).load_values(def);
        // The sidechain switch follows the source picked in the panel, not the preset.
        if let Some(p) = def.sidechain_enable() {
            params.insert(p.id.into(), def.value(d, p));
        }
        let cmd = self.sampler_followups(Command::SetDeviceParams { device, params, preset: Some(f.name.to_string()) });
        self.execute(cmd);
    }

    /// Sidechain source pick: the source and the device's enable switch in one undo step.
    pub(crate) fn panel_set_sidechain(&mut self, device: DeviceId, source: Option<TrackId>) {
        let Some((d, def)) = self.panel_def(device) else { return };
        if d.sidechain == source {
            return;
        }
        let mut commands = vec![Command::SetDeviceSidechain { device, source }];
        if let Some(p) = def.sidechain_enable() {
            commands.push(Command::SetDeviceParam { device, param: p.id.into(), value: if source.is_some() { 1.0 } else { 0.0 } });
        }
        self.execute(Command::Batch { label: "Sidechain Change".into(), commands });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compressor_card_geometry_matches_the_reference_template() {
        let def = saine_core::builtin::find("compressor").unwrap();
        let ids: Vec<&'static str> = def.layout.clusters.iter().flat_map(|c| c.iter()).filter_map(|c| c.param()).collect();
        let xs: Vec<f32> = dial_xs(def, &ids).iter().map(|(_, x)| *x).collect();
        // Even spacing (reference capture: knob centres 74 pt apart): gap (521 - 16 - 7 x 62) / 6.
        let step = KNOB_CELL + 71.0 / 6.0;
        for (i, x) in xs.iter().enumerate() {
            assert!((x - (PAD + i as f32 * step)).abs() < 1e-3, "knob {i} at {x}");
        }
        // The last knob cell ends at the card padding.
        assert!((xs[6] + KNOB_CELL + PAD - def.layout.width()).abs() < 1e-3);
    }

    #[test]
    fn mode_dependent_dial_rows_keep_their_spacing_and_centre() {
        let def = saine_core::builtin::find("character-compressor").unwrap();
        let six = dial_xs(def, &["threshold", "ratio", "knee", "attack", "release", "mix"]);
        assert!((six[0].1 - PAD).abs() < 1e-3 && (six[5].1 + KNOB_CELL + PAD - 471.0).abs() < 1e-3);
        let four = dial_xs(def, &["threshold", "ratio", "drive", "mix"]);
        let step = six[1].1 - six[0].1;
        assert!((four[1].1 - four[0].1 - step).abs() < 1e-3, "same gap");
        assert!(((four[0].1 + four[3].1 + KNOB_CELL) / 2.0 - 471.0 / 2.0).abs() < 1e-3, "centred");
    }

    #[test]
    fn detents_snap_within_reach() {
        let def = saine_core::builtin::find("character-compressor").unwrap();
        let ratio = def.param("ratio").unwrap();
        let n4 = ratio.to_normalized(4.0);
        assert_eq!(snap_to_detent(ratio, &[2.0, 4.0, 10.0], n4 + 0.02, false), Some(4.0));
        assert_eq!(snap_to_detent(ratio, &[2.0, 4.0, 10.0], n4 + 0.02, true), None, "fine drags snap closer");
        assert_eq!(snap_to_detent(ratio, &[], n4, false), None);
        assert_eq!(detent_ticks(ratio, &[2.0, 4.0]).matches('M').count(), 2);
    }

    #[test]
    fn transfer_face_paths() {
        let t = Transfer { lo: -30.0, hi: 6.0, points: vec![(-30.0, -30.0), (-18.0, -18.0), (6.0, -12.0)], bend: (-20.0, -16.0) };
        let (curve, bend, unity) = transfer_face(&t);
        assert!(curve.starts_with("M3.0 29.0"));
        assert_eq!(bend.matches('L').count(), 0, "a single bend point draws nothing");
        assert!(bend.is_empty());
        assert!(!unity.is_empty());
    }

    #[test]
    fn field_cards_centre_an_even_dial_row() {
        let xs = |id: &str| {
            let mut v: Vec<f32> = knob_xs(saine_core::builtin::find(id).unwrap()).iter().map(|(_, x)| (x * 100.0).round() / 100.0).collect();
            v.dedup();
            v
        };
        // Reference captures: tremolo knob centres 115 .. 342 pt, character mod 66 .. 376 pt.
        assert_eq!(xs("tremolo"), [84.0, 159.67, 235.33, 311.0]);
        assert_eq!(xs("character-mod"), [35.0, 112.5, 190.0, 267.5, 345.0]);
        assert_eq!(xs("ring-mod"), [9.0, 86.5, 164.0, 241.5, 319.0]);
    }

    #[test]
    fn character_switch_is_one_labelled_batch() {
        let def = saine_core::builtin::find("character-mod").unwrap();
        let d = def.instantiate();
        let p = def.param("model").unwrap();
        match param_command(def, &d, p, 7.0) {
            Command::Batch { label, commands } => {
                assert_eq!(label, "Character Change, with Hiss moved to its own defaults");
                assert_eq!(commands.len(), 2);
            }
            c => panic!("{c:?}"),
        }
        assert!(matches!(param_command(def, &d, def.param("depth").unwrap(), 70.0), Command::SetDeviceParam { .. }));
    }

    #[test]
    fn dashed_lines_alternate_along_the_path() {
        assert_eq!(dashed(&[(0.0, 0.0), (10.0, 0.0)], 4.0, 3.0), "M0.0 0.0 L4.0 0.0 M7.0 0.0 L10.0 0.0 ");
    }

    #[test]
    fn soft_knee_law() {
        assert_eq!(soft_knee_gr(-40.0, -24.0, 4.0, 6.0), 0.0);
        assert!((soft_knee_gr(-12.0, -24.0, 4.0, 6.0) - (-9.0)).abs() < 1e-5);
        // Continuous at both knee edges.
        assert!(soft_knee_gr(-27.0, -24.0, 4.0, 6.0).abs() < 1e-6);
        assert!((soft_knee_gr(-21.0, -24.0, 4.0, 6.0) - (-2.25)).abs() < 1e-5);
        assert_eq!(soft_knee_gr(-10.0, -24.0, 1.0, 0.0), 0.0);
    }

    /// Per-refresh model cost of a compressor card with a full meter history
    /// (`cargo test -p saine-app --release bench_panel_model -- --ignored --nocapture`).
    #[test]
    #[ignore]
    fn bench_panel_model() {
        let slots: VecDeque<(f32, f32, f32)> = (0..HISTORY).map(|i| {
            let t = i as f32 / 10.0;
            (-12.0 + 6.0 * t.sin(), -14.0 + 3.0 * t.cos(), (-6.0 + 5.0 * t.sin()).min(0.0))
        }).collect();
        let n = 2000;
        let t0 = std::time::Instant::now();
        for _ in 0..n {
            std::hint::black_box(strip_paths(std::hint::black_box(&slots), Strip::new("threshold"), 368.0, 112.2));
        }
        let strip = t0.elapsed().as_secs_f64() * 1e6 / n as f64;
        let t0 = std::time::Instant::now();
        for i in 0..n {
            std::hint::black_box(soft_knee_face(-24.0 + (i % 7) as f32, 4.0, 6.0));
        }
        let face = t0.elapsed().as_secs_f64() * 1e6 / n as f64;
        let cc = saine_core::builtin::find("character-compressor").unwrap();
        let mut d = cc.instantiate();
        d.params.insert("model".into(), 1.0);
        let b = saine_core::builtin::behavior(cc).unwrap();
        let t0 = std::time::Instant::now();
        for i in 0..n {
            d.params.insert("threshold".into(), -24.0 + (i % 7) as f32);
            std::hint::black_box(transfer_face(&(b.transfer)(cc, &d).unwrap()));
        }
        let vm = t0.elapsed().as_secs_f64() * 1e6 / n as f64;
        println!("panel model: strip paths (240-slot history) {strip:.2} us, transfer face {face:.2} us, Vari-Mu face {vm:.2} us per refresh");
    }

    #[test]
    fn strip_paths_and_dashes() {
        assert_eq!(polyline(&[(0.0, 1.0), (2.0, 3.0)]), "M0.0 1.0 L2.0 3.0");
        assert_eq!(dashes(0.0, 10.0, 5.0, 6.0, 5.0), "M0.00 5.00 L6.00 5.00 ");
        assert_eq!(fmt_db(-0.02), "0.0 dB");
        assert_eq!(fmt_db(-3.26), "\u{2212}3.3 dB");
    }
}

