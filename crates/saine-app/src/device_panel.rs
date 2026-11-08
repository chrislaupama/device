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
//! Views: the dynamics strip (View::Strip / View::DynamicsStrip: level history
//! from the engine's readouts, one or two draggable lines) and the
//! "gain-scope" custom view are drawn here. View::Custom(name) is the escape
//! hatch for device-specific bodies (EQ curve, shaper, trance-gate grid) and
//! gets its own arm in [Controller::builtin_card].

use crate::controller::{slint_color, track_color, Controller};
use crate::devices::{DevMenu, MenuPick};
use crate::{DeviceCard, PanelKnob, PanelPick, PanelSlider, PanelStrip, PanelTick};
use saine_core::builtin::{Control, DeviceDef, Face, ParamDef, Strip, View, CLUSTER_GAP, KNOB_CELL, KNOB_GAP};
use saine_core::{Command, Device, DeviceId, TrackId};
use slint::{ModelRc, SharedString, VecModel};
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;

/// Reference card lengths are at 100% zoom; ours are drawn at 85%.
pub const Z: f32 = 0.85;
/// Card padding, slider column width and the gap between view and sliders.
const PAD: f32 = 8.0;
const SLIDER_W: f32 = 62.0;
const VIEW_GAP: f32 = 10.0;
/// View row top, caption zone above the well, well bottom (card relative).
const VIEW_Y: f32 = 63.0;
const CAPTION_H: f32 = 19.0;
/// Picker row band above the view row (row top 37 to view top).
const PICKER_H: f32 = 26.0;
/// Slider rail top below the view row top.
const SLIDER_TOP: f32 = 23.0;
const VIEW_BOTTOM: f32 = 214.0;
/// Dynamics strip plot range (dB) and history length (slots across the well).
#[cfg(test)]
const PLOT_TOP_DB: f32 = 0.0;
const PLOT_FLOOR_DB: f32 = -36.0;
const HISTORY: usize = 240;
/// Custom view: scrolling output waveform with the applied-gain trace (transient shaper).
const GAIN_SCOPE: &str = "gain-scope";
/// Gain-scope trace range (+-dB) and the level under which it reads as silence (linear, -48 dB).
const SCOPE_DB: f32 = 12.0;
const SCOPE_SILENT: f32 = 0.004;
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
    /// Strip Alt+drag: a second line moved by the same amount.
    linked: Option<Linked>,
}

#[derive(Debug, Clone, Copy)]
struct Linked {
    index: usize,
    start: f32,
    value: f32,
    /// Accumulated move of both lines (plain units, before clamping).
    delta: f32,
}

/// Smoothed level history of a dynamics strip (newest last).
#[derive(Debug, Clone, Default)]
struct MeterHistory {
    /// (input dB, output dB, gain reduction dB <= 0)
    slots: VecDeque<(f32, f32, f32)>,
    last: (f32, f32, f32),
}

#[derive(Debug, Default)]
pub struct PanelState {
    drag: Option<ParamDrag>,
    meters: HashMap<DeviceId, MeterHistory>,
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
    STATIC_MODELS.with(|m| m.borrow_mut().entry(format!("{}/{}", def.id, p.id)).or_insert_with(|| model(p.choices.iter().map(|c| s(*c)).collect())).clone())
}

/// Latch labels in the picker row: each choice prefixed with the parameter label.
fn latch_model(def: &DeviceDef, p: &ParamDef) -> ModelRc<SharedString> {
    STATIC_MODELS.with(|m| m.borrow_mut().entry(format!("{}/{}/latch", def.id, p.id)).or_insert_with(|| model(p.choices.iter().map(|c| s(format!("{} {c}", p.label))).collect())).clone())
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
}

/// x of each dial-row knob cell (card relative, unscaled), cluster by cluster.
fn knob_xs(def: &DeviceDef) -> Vec<(&'static str, f32)> {
    let mut x = PAD;
    let mut out = Vec::new();
    for (ci, cluster) in def.layout.clusters.iter().enumerate() {
        if ci > 0 {
            x += CLUSTER_GAP - KNOB_GAP;
        }
        for c in cluster.iter() {
            if let Some(p) = c.param() {
                out.push((p, x));
            }
            x += KNOB_CELL + KNOB_GAP;
        }
    }
    out
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

/// Static transfer curve (output dB for input dB) of a knob face, the input
/// level the axis is laid out around and the span its knee highlight covers.
fn face_law(face: Face, num: impl Fn(&str) -> f32) -> (Box<dyn Fn(f32) -> f32>, f32, (f32, f32)) {
    match face {
        Face::SoftKneeCurve { threshold, ratio, knee } => {
            let (t, r, k) = (num(threshold), num(ratio), num(knee).max(0.0));
            (Box::new(move |x| x + soft_knee_gr(x, t, r, k)), t, (t - k / 2.0, t + k / 2.0))
        }
        Face::BrickWall { ceiling } => {
            let c = num(ceiling);
            (Box::new(move |x: f32| x.min(c)), c, (c, c))
        }
        Face::Expander { threshold, ratio } => {
            let (t, r) = (num(threshold), num(ratio).max(1.0));
            (Box::new(move |x| if x < t { t + (x - t) * r } else { x }), t, (t, t))
        }
        Face::BusVca { threshold, ratio, range } => {
            let (t, r, depth) = (num(threshold), num(ratio), num(range).max(0.0));
            (Box::new(move |x| x + soft_knee_gr(x, t, r, 4.0).max(-depth)), t, (t - 2.0, t + 2.0))
        }
        Face::PushWall { threshold, ceiling } => {
            let (t, c) = (num(threshold), num(ceiling));
            (Box::new(move |x| (x + c - t).min(c)), t, (t, t))
        }
    }
}

/// Transfer-face paths in a 32 x 32 box: (curve, knee highlight, dashed unity line).
/// The input axis spans 12 dB below `at` to 24 dB above it (at least to 0 dB).
fn transfer_face(law: &dyn Fn(f32) -> f32, at: f32, (knee_lo, knee_hi): (f32, f32)) -> (String, String, String) {
    let lo = at - 12.0;
    let hi = (at + 24.0).max(0.0);
    let (pad, span) = (3.0, 26.0);
    let px = |db: f32| pad + (db - lo) / (hi - lo) * span;
    let py = |db: f32| 32.0 - pad - ((db - lo) / (hi - lo) * span).clamp(-pad, span + pad);
    let point = |x: f32| (px(x), py(law(x)));
    let n = 48;
    let curve: Vec<(f32, f32)> = (0..=n).map(|i| point(lo + (hi - lo) * i as f32 / n as f32)).collect();
    let margin = 3.0 / span * (hi - lo);
    let (b0, b1) = ((knee_lo - margin).max(lo), (knee_hi + margin).min(hi));
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

/// Strip paths for a level history in a w x wh well: [input fill, gain-reduction
/// band (input to input + GR), input line, output line].
fn strip_paths(slots: &VecDeque<(f32, f32, f32)>, w: f32, wh: f32, top_db: f32, floor_db: f32) -> [String; 4] {
    let span = top_db - floor_db;
    let y_of = |db: f32| ((top_db - db) / span).clamp(0.0, 1.0) * wh;
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

/// The command for one parameter edit: a choice with overlays also writes
/// its overlay values (one step; the preset name stays, so it reads dirty).
fn param_command(d: &Device, def: &DeviceDef, p: &ParamDef, v: f32) -> Command {
    if p.overlays.is_empty() {
        return Command::SetDeviceParam { device: d.id, param: p.id.to_string(), value: v };
    }
    Command::SetDeviceParams { device: d.id, params: def.choice_values(p.id, v), preset: d.preset.clone() }
}

impl Controller {
    fn panel_def(&self, id: DeviceId) -> Option<(&Device, &'static DeviceDef)> {
        let d = self.project().device(id)?;
        Some((d, saine_core::builtin::def_of(d)?))
    }

    /// Value shown for a parameter: the live drag value, else the model value.
    fn panel_value(&self, d: &Device, def: &DeviceDef, p: &ParamDef) -> f32 {
        let is = |i: usize| def.params.get(i).is_some_and(|q| q.id == p.id);
        match self.devices.panel.drag {
            Some(g) if g.device == d.id && is(g.index) => g.value,
            Some(ParamDrag { device, linked: Some(l), .. }) if device == d.id && is(l.index) => l.value,
            _ => def.value(d, p),
        }
    }

    /// The card of a built-in device (generic panel).
    pub(crate) fn builtin_card(&self, d: &Device, def: &'static DeviceDef) -> DeviceCard {
        let width = def.layout.width();
        let track = self.project().find_device(d.id).map(|(ti, _)| &self.project().tracks[ti]);
        let accent = track.map_or(slint::Color::from_rgb_u8(0x43, 0x9b, 0x94), |t| slint_color(track_color(t.color)));
        let idx = |id: &str| def.index_of(id).map_or(-1, |i| i as i32);
        let number = |id: &str| def.param(id).map_or(0.0, |p| p.number(self.panel_value(d, def, p)));

        // Picker row; the sidechain picker and everything after it align right.
        let mut picks = Vec::new();
        for c in def.layout.picker {
            let p = c.param().and_then(|id| def.param(id));
            match (c, p) {
                // Latch pill, right-aligned: "<label> <choice>".
                (Control::Toggle(_), Some(p)) if def.is_shown(d, p.id) => {
                    if !picks.iter().any(|q: &PanelPick| q.kind == 4) {
                        picks.push(PanelPick { kind: 4, ..Default::default() });
                    }
                    picks.push(PanelPick {
                        kind: 5,
                        param: idx(p.id),
                        choices: latch_model(def, p),
                        selected: self.panel_value(d, def, p).round() as i32,
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
                    picks.push(PanelPick { kind: 2, param: -1, label: s("Sidechain"), value: s(source), enabled: true, width: 80.0 * Z, ..Default::default() });
                }
                (Control::Pill(_), Some(p)) if def.is_shown(d, p.id) => picks.push(PanelPick {
                    kind: 3,
                    param: idx(p.id),
                    label: s(p.label),
                    value: s(p.format(self.panel_value(d, def, p))),
                    // A pill after the sidechain picker scales the key: inert without a source.
                    enabled: d.sidechain.is_some() || !def.layout.picker.iter().take_while(|q| *q != c).any(|q| *q == Control::Sidechain),
                    width: 60.0 * Z,
                    ..Default::default()
                }),
                _ => {}
            }
        }

        // Without a picker row the view row moves up into its place.
        let view_y = if def.layout.picker.is_empty() { VIEW_Y - PICKER_H } else { VIEW_Y };

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
                    value: s(p.format(v)),
                    norm: p.to_normalized(v),
                    bipolar: p.is_bipolar(),
                }
            })
            .collect();
        let view_right = if sliders.is_empty() { width - PAD } else { slider_x - (sliders.len() - 1) as f32 * (SLIDER_W + VIEW_GAP) - VIEW_GAP };

        // View row.
        let strip = match def.layout.view {
            Some(View::Custom(GAIN_SCOPE)) => self.gain_scope(d, view_right - PAD, view_y),
            Some(view) => view.strip().map_or_else(PanelStrip::default, |st| self.dynamics_strip(d, def, st, view_right - PAD, view_y)),
            None => PanelStrip::default(),
        };

        // Dial row.
        let toggles: Vec<&str> = def.layout.clusters.iter().flat_map(|c| c.iter()).filter_map(|c| match c {
            Control::Toggle(p) => Some(*p),
            _ => None,
        }).collect();
        let knobs = knob_xs(def)
            .into_iter()
            .filter(|(id, _)| def.is_shown(d, id))
            .filter_map(|(id, x)| {
                let p = def.param(id)?;
                let v = self.panel_value(d, def, p);
                let mut k = PanelKnob {
                    param: idx(id),
                    x: x * Z,
                    label: s(p.label),
                    value: s(p.format(v)),
                    norm: p.to_normalized(v),
                    bipolar: p.is_bipolar(),
                    steps: if p.choices.len() > 2 { p.choices.len() as i32 } else { 0 },
                    ..Default::default()
                };
                if toggles.contains(&id) {
                    k.face = 2;
                    k.choices = choices_model(def, p);
                } else if let Some((_, face)) = def.layout.faces.iter().find(|(f, _)| *f == id) {
                    let (law, at, knee) = face_law(*face, number);
                    let (curve, bend, unity) = transfer_face(&law, at, knee);
                    k.face = 1;
                    k.face_curve = s(curve);
                    k.face_bend = s(bend);
                    k.face_unity = s(unity);
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

        DeviceCard {
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
            slider_top: (view_y + SLIDER_TOP) * Z,
            ..Default::default()
        }
    }

    fn dynamics_strip(&self, d: &Device, def: &DeviceDef, st: Strip, w: f32, view_y: f32) -> PanelStrip {
        let (w, h) = (w * Z, (VIEW_BOTTOM - view_y) * Z);
        let well_y = CAPTION_H * Z;
        let wh = h - well_y;
        let (top, floor) = (st.top_db, st.floor_db);
        let threshold = st.line;
        let span = top - floor;
        let y_of = |db: f32| ((top - db) / span).clamp(0.0, 1.0) * wh;
        // A tick every 12 dB from the top, plus the floor when it is off that grid.
        let ticks: Vec<PanelTick> = (0..=(span / 12.0) as i32)
            .map(|i| top - 12.0 * i as f32)
            .chain((span % 12.0 != 0.0).then_some(floor))
            .map(|db| {
                let text = if db == 0.0 { "0".to_string() } else if db > 0.0 { format!("+{}", db as i32) } else { format!("{MINUS}{}", db.abs() as i32) };
                PanelTick { y: y_of(db), text: s(text), rule: db < top && db > floor }
            })
            .collect();

        let empty = MeterHistory::default();
        let hist = self.devices.panel.meters.get(&d.id).unwrap_or(&empty);
        let [in_fill, gr_fill, in_line, out_line] = strip_paths(&hist.slots, w, wh, top, floor);
        let gr_now = hist.slots.iter().rev().take(30).map(|s| s.2).fold(0.0f32, f32::min);

        let p = def.param(threshold);
        let t = p.map(|p| self.panel_value(d, def, p)).unwrap_or(floor);
        let ty = y_of(t).min(wh - 1.0);
        let q = st.second.and_then(|id| def.param(id));
        let second = q.map(|q| self.panel_value(d, def, q)).unwrap_or(floor);
        let sy = y_of(second).min(wh - 1.0);
        PanelStrip {
            visible: true,
            x: PAD * Z,
            y: view_y * Z,
            w,
            h,
            well_y,
            caption: s("Reduction"),
            readout: s(fmt_db(gr_now)),
            ticks: TICK_MODELS.with(|m| m.borrow_mut().entry(format!("{}/{wh}/{top}/{floor}", def.id)).or_insert_with(|| model(ticks)).clone()),
            in_fill: s(in_fill),
            gr_fill: s(gr_fill),
            in_line: s(in_line),
            out_line: s(out_line),
            thresh_param: def.index_of(threshold).map_or(-1, |i| i as i32),
            thresh_y: ty,
            thresh_dash: s(dashes(30.0 * Z, w, ty, 6.0 * Z, 5.0 * Z)),
            thresh_label: s(p.map_or("", |p| p.label)),
            thresh_value: s(p.map(|p| p.format(t)).unwrap_or_default()),
            // The line follows the pointer: (dB per px) / (dB per unit of the 0..1 range).
            thresh_rate: p.map_or(0.0, |p| span / wh / (p.max - p.min)),
            second_param: q.and_then(|q| def.index_of(q.id)).map_or(-1, |i| i as i32),
            second_y: sy,
            second_line: s(if q.is_some() { polyline(&[(30.0 * Z, sy), (w, sy)]) } else { String::new() }),
            second_label: s(q.map_or("", |q| q.label)),
            second_value: s(q.map(|q| q.format(second)).unwrap_or_default()),
            second_rate: q.map_or(0.0, |q| span / wh / (q.max - q.min)),
            ..Default::default()
        }
    }

    /// Gain scope (custom view): output waveform as mirrored bars (square-root
    /// amplitude law), what a clipper removed above them, a dashed 0 dB line and
    /// the applied-gain trace (+-12 dB). Readouts: output peak, pre-clip peak (linear), gain dB.
    fn gain_scope(&self, d: &Device, w: f32, view_y: f32) -> PanelStrip {
        let (w, h) = (w * Z, (VIEW_BOTTOM - view_y) * Z);
        let well_y = CAPTION_H * Z;
        let wh = h - well_y;
        let mid = wh / 2.0;
        let empty = MeterHistory::default();
        let slots = &self.devices.panel.meters.get(&d.id).unwrap_or(&empty).slots;
        let n = slots.len();
        let x_of = |i: usize| w - (n - 1 - i) as f32 * (w - 6.0 * Z) / (HISTORY - 1) as f32;
        let r_of = |peak: f32| (peak.abs().min(1.41) / 1.41).sqrt() * mid;
        let band = |r: &dyn Fn(usize) -> f32, inner: &dyn Fn(usize) -> f32, sign: f32| {
            let mut pts: Vec<(f32, f32)> = (0..n).map(|i| (x_of(i), mid - sign * r(i))).collect();
            pts.extend((0..n).rev().map(|i| (x_of(i), mid - sign * inner(i))));
            format!("{} Z", polyline(&pts))
        };
        let out_r = |i: usize| r_of(slots[i].0);
        let pre_r = |i: usize| r_of(slots[i].1).max(out_r(i));
        let (mut wave, mut clipped) = (String::new(), String::new());
        // At rest the gain trace lies flat at 0 dB across the well.
        let mut gain = polyline(&[(6.0 * Z, mid), (w, mid)]);
        if n > 1 {
            wave = format!("{} {}", band(&out_r, &|_| 0.0, 1.0), band(&out_r, &|_| 0.0, -1.0));
            if slots.iter().any(|s| s.1 > s.0 * 1.005) {
                clipped = format!("{} {}", band(&pre_r, &out_r, 1.0), band(&pre_r, &out_r, -1.0));
            }
            let y_of = |db: f32| 4.0 * Z + (SCOPE_DB - db.clamp(-SCOPE_DB, SCOPE_DB)) / (2.0 * SCOPE_DB) * (wh - 8.0 * Z);
            gain = polyline(&slots.iter().enumerate().map(|(i, s)| (x_of(i), y_of(s.2))).collect::<Vec<_>>());
        }
        PanelStrip {
            visible: true,
            kind: 1,
            x: PAD * Z,
            y: view_y * Z,
            w,
            h,
            well_y,
            caption: s("Output"),
            readout: s("Applied Gain"),
            in_fill: s(wave),
            gr_fill: s(clipped),
            out_line: s(gain),
            thresh_param: -1,
            second_param: -1,
            thresh_dash: s(dashes(6.0 * Z, w, mid, 3.0 * Z, 4.0 * Z)),
            ..Default::default()
        }
    }

    /// Engine readouts -> strip histories of the selected track's built-in
    /// devices with a dynamics strip (every poll tick, ~30 ms).
    pub(crate) fn panel_poll_meters(&mut self) {
        let Some(t) = self.selected_track.and_then(|t| self.project().track(t)) else { return };
        let ids: Vec<DeviceId> = t
            .devices
            .iter()
            .filter(|d| saine_core::builtin::def_of(d).is_some_and(|def| def.layout.view.is_some_and(|v| v.strip().is_some() || v == View::Custom(GAIN_SCOPE))))
            .map(|d| d.id)
            .collect();
        self.devices.panel.meters.retain(|id, _| ids.contains(id));
        let mut out = Vec::new();
        for id in ids {
            let scope = self.panel_def(id).is_some_and(|(_, def)| def.layout.view == Some(View::Custom(GAIN_SCOPE)));
            if scope {
                // Raw frames (output peak, pre-clip peak, gain dB); parked after silence.
                let next = match self.engine.builtin_readouts(id.0, &mut out) {
                    Ok(()) if out.len() >= 3 => (out[0], out[1], out[2]),
                    _ => (0.0, 0.0, 0.0),
                };
                let h = self.devices.panel.meters.entry(id).or_default();
                let silent = |s: (f32, f32, f32)| s.0 < SCOPE_SILENT && s.1 < SCOPE_SILENT && s.2.abs() < 0.05;
                if silent(next) && h.slots.iter().all(|s| silent(*s)) {
                    h.slots.clear();
                    continue;
                }
                if h.slots.len() == HISTORY {
                    h.slots.pop_front();
                }
                h.slots.push_back(next);
                self.ui_dirty = true;
                continue;
            }
            // Readout order of a strip device: input dB, output dB, gain reduction dB.
            let (i, o, g) = match self.engine.builtin_readouts(id.0, &mut out) {
                Ok(()) if out.len() >= 3 => (out[0], out[1], out[2].min(0.0)),
                _ => (PLOT_FLOOR_DB - 1.0, PLOT_FLOOR_DB - 1.0, 0.0),
            };
            let h = self.devices.panel.meters.entry(id).or_default();
            let silent = |s: (f32, f32, f32)| s.0 < PLOT_FLOOR_DB && s.1 < PLOT_FLOOR_DB && s.2 > -0.05;
            let prev = h.slots.back().copied().unwrap_or((PLOT_FLOOR_DB - 1.0, PLOT_FLOOR_DB - 1.0, 0.0));
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
    }

    // ---------------------------------------------------------------- gestures

    pub fn panel_drag(&mut self, device: DeviceId, index: i32, phase: i32, dy: f32, rate: f32, fine: bool) {
        let Some((d, def)) = self.panel_def(device) else { return };
        let Some(p) = usize::try_from(index).ok().and_then(|i| def.params.get(i)) else { return };
        match phase {
            0 => {
                let v = def.value(d, p);
                self.devices.panel.drag = Some(ParamDrag { device, index: index as usize, start: v, norm: p.to_normalized(v), value: v, travel: 0.0, held: 0.0, active: false, linked: None });
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
                if v != g.value {
                    g.value = v;
                    let _ = self.engine.builtin_set_param(device.0, index as u32, v);
                    self.ui_dirty = true;
                }
            }
            2 => {
                let Some(g) = self.devices.panel.drag else { return };
                let cmd = (g.active && g.value != g.start).then(|| param_command(d, def, p, g.value));
                self.devices.panel.drag = None;
                self.ui_dirty = true;
                if let Some(cmd) = cmd {
                    self.panel_commit(cmd);
                }
            }
            _ => {
                self.panel_cancel_drag();
            }
        }
    }

    /// Alt+drag on a two-line strip: both lines move by the same amount, clamped
    /// so neither leaves its range; one undo step. Alt-click without a move resets `index`.
    pub fn panel_drag_pair(&mut self, device: DeviceId, index: i32, other: i32, phase: i32, dy: f32, rate: f32, fine: bool) {
        let Some((d, def)) = self.panel_def(device) else { return };
        let param = |i: i32| usize::try_from(i).ok().and_then(|i| def.params.get(i).map(|p| (i, p)));
        let (Some((ia, pa)), Some((ib, pb))) = (param(index), param(other)) else { return };
        match phase {
            0 => {
                let (va, vb) = (def.value(d, pa), def.value(d, pb));
                let linked = Some(Linked { index: ib, start: vb, value: vb, delta: 0.0 });
                self.devices.panel.drag = Some(ParamDrag { device, index: ia, start: va, norm: 0.0, value: va, travel: 0.0, held: 0.0, active: false, linked });
            }
            1 => {
                let Some(g) = self.devices.panel.drag.as_mut().filter(|g| g.device == device && g.index == ia) else { return };
                let Some(l) = g.linked.as_mut() else { return };
                g.travel += dy.abs();
                l.delta += dy * rate * if fine { 0.2 } else { 1.0 } * (pa.max - pa.min);
                if !g.active && g.travel < DRAG_SLOP {
                    return;
                }
                g.active = true;
                let delta = l.delta.clamp((pa.min - g.start).max(pb.min - l.start), (pa.max - g.start).min(pb.max - l.start));
                let (va, vb) = (pa.snap(g.start + delta), pb.snap(l.start + delta));
                if (va, vb) != (g.value, l.value) {
                    (g.value, l.value) = (va, vb);
                    let _ = self.engine.builtin_set_param(device.0, ia as u32, va);
                    let _ = self.engine.builtin_set_param(device.0, ib as u32, vb);
                    self.ui_dirty = true;
                }
            }
            2 => {
                let Some(g) = self.devices.panel.drag else { return };
                let Some(l) = g.linked else { return };
                let params = [(pa.id.to_string(), g.value), (pb.id.to_string(), l.value)].into_iter().collect();
                let cmd = (g.active && (g.value, l.value) != (g.start, l.start)).then(|| Command::SetDeviceParams { device, params, preset: d.preset.clone() });
                self.devices.panel.drag = None;
                self.ui_dirty = true;
                match cmd {
                    Some(cmd) => self.panel_commit(cmd),
                    None if !g.active => self.panel_reset(device, index),
                    None => {}
                }
            }
            _ => {
                self.panel_cancel_drag();
            }
        }
    }

    /// Escape or a cancelled pointer: restore the value from before the drag, with no undo step.
    pub fn panel_cancel_drag(&mut self) -> bool {
        let Some(g) = self.devices.panel.drag.take() else { return false };
        let _ = self.engine.builtin_set_param(g.device.0, g.index as u32, g.start);
        if let Some(l) = g.linked {
            let _ = self.engine.builtin_set_param(g.device.0, l.index as u32, l.start);
        }
        self.ui_dirty = true;
        true
    }

    fn panel_set(&mut self, device: DeviceId, index: i32, value: impl FnOnce(&ParamDef, f32) -> Option<f32>) {
        let Some((d, def)) = self.panel_def(device) else { return };
        let Some(p) = usize::try_from(index).ok().and_then(|i| def.params.get(i)) else { return };
        let cur = def.value(d, p);
        if let Some(v) = value(p, cur).map(|v| p.snap(v))
            && v != cur
        {
            let cmd = param_command(d, def, p, v);
            self.panel_commit(cmd);
        }
    }

    /// One gesture = one undo step: never merge with the previous edit of the same parameter.
    fn panel_commit(&mut self, cmd: Command) {
        self.doc.seal_coalescing();
        self.execute(cmd);
        self.doc.seal_coalescing();
    }

    pub fn panel_reset(&mut self, device: DeviceId, index: i32) {
        self.panel_set(device, index, |p, _| Some(p.default));
    }

    /// Arrow keys: discrete parameters move one choice, continuous ones 1% of the range (0.2% fine).
    pub fn panel_step(&mut self, device: DeviceId, index: i32, delta: i32, fine: bool) {
        self.panel_set(device, index, |p, v| {
            Some(if p.is_discrete() {
                v + delta as f32
            } else {
                p.from_normalized(p.to_normalized(v) + delta as f32 * if fine { 0.002 } else { 0.01 })
            })
        });
    }

    pub fn panel_choice(&mut self, device: DeviceId, index: i32, choice: i32) {
        self.panel_set(device, index, |_, _| Some(choice as f32));
    }

    pub fn panel_text(&mut self, device: DeviceId, index: i32, text: &str) {
        let mut bad = None;
        self.panel_set(device, index, |p, _| {
            let v = p.parse(text);
            if v.is_none() {
                bad = Some(format!("{}: can't read “{}”", p.name, text.trim()));
            }
            v
        });
        if let Some(msg) = bad {
            self.set_status(msg, 3);
        }
    }

    /// Sidechain picker (-1) or presets (-2) menu at window x, y.
    pub fn panel_menu(&mut self, device: DeviceId, which: i32, x: f32, y: f32) {
        let Some((d, def)) = self.panel_def(device) else { return };
        let check = |on: bool| if on { "✓".to_string() } else { String::new() };
        let rows = match which {
            -1 => {
                let own = self.project().find_device(device).map(|(ti, _)| self.project().tracks[ti].id);
                // A source that already listens to this track (directly or through other sidechains) would form a cycle.
                let feeds_back = |from: TrackId| {
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
                };
                let mut rows = vec![("None".to_string(), check(d.sidechain.is_none()), Some(MenuPick::Sidechain(device, None)), false)];
                for t in self.project().tracks.iter().filter(|t| Some(t.id) != own && (d.sidechain == Some(t.id) || !feeds_back(t.id))) {
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
            _ => return,
        };
        self.devices.menu = Some(DevMenu { pos: (x, y), w: 180.0, rows });
        self.ui_dirty = true;
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
        if let Some(p) = def.param("sidechainEnable") {
            params.insert(p.id.into(), def.value(d, p));
        }
        self.execute(Command::SetDeviceParams { device, params, preset: Some(f.name.to_string()) });
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
        assert_eq!(def.layout.width(), 521.0);
        let xs: Vec<f32> = knob_xs(def).iter().map(|(_, x)| *x).collect();
        assert_eq!(xs, [8.0, 80.0, 163.0, 235.0, 307.0, 379.0, 451.0]);
        // The last knob cell ends at the card padding.
        assert_eq!(xs[6] + KNOB_CELL + PAD, def.layout.width());
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
            std::hint::black_box(strip_paths(std::hint::black_box(&slots), 368.0, 112.2, PLOT_TOP_DB, PLOT_FLOOR_DB));
        }
        let strip = t0.elapsed().as_secs_f64() * 1e6 / n as f64;
        let t0 = std::time::Instant::now();
        for i in 0..n {
            let t = -24.0 + (i % 7) as f32;
            let (law, at, knee) = face_law(Face::SoftKneeCurve { threshold: "t", ratio: "r", knee: "k" }, |id| match id { "t" => t, "r" => 4.0, _ => 6.0 });
            std::hint::black_box(transfer_face(&law, at, knee));
        }
        let face = t0.elapsed().as_secs_f64() * 1e6 / n as f64;
        println!("panel model: strip paths (240-slot history) {strip:.2} us, transfer face {face:.2} us per refresh");
    }

    #[test]
    fn face_laws() {
        let num = |id: &str| match id { "t" => -10.0, "c" => -1.0, "r" => 4.0, "range" => 3.0, _ => 0.0 };
        let law = |f: Face| face_law(f, num).0;
        let wall = law(Face::BrickWall { ceiling: "c" });
        assert_eq!((wall(-20.0), wall(5.0)), (-20.0, -1.0));
        let push = law(Face::PushWall { threshold: "t", ceiling: "c" });
        assert_eq!((push(-30.0), push(0.0)), (-21.0, -1.0));
        let gate = law(Face::Expander { threshold: "t", ratio: "r" });
        assert_eq!((gate(-12.0), gate(-5.0)), (-18.0, -5.0));
        // Bus VCA reduction stops at the range.
        assert_eq!(law(Face::BusVca { threshold: "t", ratio: "r", range: "range" })(10.0), 7.0);
    }

    #[test]
    fn strip_paths_and_dashes() {
        assert_eq!(polyline(&[(0.0, 1.0), (2.0, 3.0)]), "M0.0 1.0 L2.0 3.0");
        assert_eq!(dashes(0.0, 10.0, 5.0, 6.0, 5.0), "M0.00 5.00 L6.00 5.00 ");
        assert_eq!(fmt_db(-0.02), "0.0 dB");
        assert_eq!(fmt_db(-3.26), "\u{2212}3.3 dB");
    }
}
