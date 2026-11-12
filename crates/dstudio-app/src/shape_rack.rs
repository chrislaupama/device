//! Clip sidebar, Shape tab: the rack's rows (shown by \`ClipSidebar\`), the curves
//! it draws over the piano roll, and drawing them with the Pencil, Steps and Ramp
//! brushes. The transforms are \`dstudio_core::shape\`; every edit is one SetComposer
//! command ("Shape Change"), a drag or a stroke one undo step.

use crate::controller::Controller;
use crate::pianoroll::{GRID_X, GRID_Y, PrView};
use dstudio_core::command::ComposerEdit;
use dstudio_core::library::PartType;
use dstudio_core::midi::{MidiClip, Note};
use dstudio_core::shape::{self, Shape, ShapeKey};
use dstudio_core::{Command, PPQ};

/// How a rack row's control works.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShapeKind {
    Slider,
    Stepper,
    Segments,
}

/// One row of the Shape rack.
#[derive(Debug, Clone, PartialEq)]
pub struct ShapeRowView {
    pub key: ShapeKey,
    pub value: String,
    pub kind: ShapeKind,
    /// Base value 0..1 (slider position).
    pub norm: f32,
    /// The slider fills from its middle (Nudge).
    pub bipolar: bool,
    /// Segment count and the lit one.
    pub segs: i32,
    pub seg: i32,
    pub enabled: bool,
    pub tip: &'static str,
    /// The row whose curve the piano roll edits.
    pub selected: bool,
    /// A curve is drawn on it.
    pub curved: bool,
}

/// Shape callback ops: slider press / drag / release, stepper step, segment pick,
/// reset to neutral, select the row's curve.
pub mod shape_op {
    pub const PRESS: i32 = 0;
    pub const DRAG: i32 = 1;
    pub const RELEASE: i32 = 2;
    pub const STEP: i32 = 3;
    pub const PICK: i32 = 4;
    pub const RESET: i32 = 5;
    pub const SELECT: i32 = 6;
}

/// Curve brushes, in the toolbar's order.
pub const BRUSHES: [&str; 3] = ["Pencil", "Steps", "Ramp"];

const NO_VOICES: &str = "This pattern has no extra voices to add";
const NOT_AVAILABLE: &str = "Not available for this pattern";

/// Piano roll side of the Shape tab: the curve being edited, draw mode and brush.
#[derive(Debug, Clone)]
pub struct ShapeUi {
    pub key: ShapeKey,
    pub drawing: bool,
    pub brush: usize,
    stroke: Option<Stroke>,
}

impl Default for ShapeUi {
    fn default() -> Self {
        ShapeUi { key: ShapeKey::Density, drawing: false, brush: 0, stroke: None }
    }
}

#[derive(Debug, Clone)]
struct Stroke {
    key: ShapeKey,
    /// The curve when the stroke began (Ramp redraws from it).
    orig: Vec<f32>,
    start: (f64, f32),
    last: (f64, f32),
}

/// One curve as the piano roll draws it, in grid points.
#[derive(Debug, Clone, PartialEq)]
pub struct CurveDraw {
    pub color: [f32; 3],
    pub active: bool,
    pub line: Vec<(f32, f32)>,
    /// Beat points of the active curve.
    pub dots: Vec<(f32, f32)>,
    /// Neutral line of a two-sided control.
    pub guide: Option<f32>,
}

fn kind(k: ShapeKey) -> ShapeKind {
    match k {
        ShapeKey::Voices => ShapeKind::Segments,
        ShapeKey::Octave | ShapeKey::Repeat | ShapeKey::Inversion | ShapeKey::Arp | ShapeKey::ArpPattern | ShapeKey::ArpOctaves => {
            ShapeKind::Stepper
        }
        _ => ShapeKind::Slider,
    }
}

/// The rack's rows: Pattern and Octaves join while Arp plays.
pub fn visible_keys(s: &Shape) -> Vec<ShapeKey> {
    let arp = s.active(ShapeKey::Arp, &Shape::neutral(None));
    ShapeKey::ALL.into_iter().filter(|k| arp || !matches!(k, ShapeKey::ArpPattern | ShapeKey::ArpOctaves)).collect()
}

fn enabled(k: ShapeKey, base: &[Note], source: Option<PartType>) -> bool {
    match k {
        ShapeKey::Voices => shape::has_voices(base, source),
        k if k.is_harmony() => shape::has_harmony(base, source),
        _ => true,
    }
}

/// Curve area of the piano roll grid: value 1 at the top, 0 at the bottom.
fn value_y(v: &PrView, value: f32) -> f32 {
    let (top, bottom) = (GRID_Y + 8.0, v.h - 8.0);
    bottom - value.clamp(0.0, 1.0) * (bottom - top)
}

fn y_value(v: &PrView, y: f32) -> f32 {
    let (top, bottom) = (GRID_Y + 8.0, v.h - 8.0);
    ((bottom - y) / (bottom - top).max(1.0)).clamp(0.0, 1.0)
}

fn rgb(c: u32) -> [f32; 3] {
    [((c >> 16) & 255) as f32 / 255.0, ((c >> 8) & 255) as f32 / 255.0, (c & 255) as f32 / 255.0]
}

/// A curve fitted to \`len\` points (the clip's length may have changed since it was drawn).
fn fitted(points: Option<&[f32]>, len: usize) -> Vec<f32> {
    let mut p = points.map(<[f32]>::to_vec).unwrap_or_default();
    let last = p.last().copied().unwrap_or(0.0);
    p.resize(len, last);
    p
}

impl Controller {
    /// Rows of the rack for what plays.
    pub(crate) fn shape_rows(&self, base: &[Note], source: Option<PartType>, s: &Shape) -> Vec<ShapeRowView> {
        visible_keys(s)
            .into_iter()
            .map(|k| {
                let kind = kind(k);
                let v = s.norm(k);
                let on = enabled(k, base, source);
                let value = match kind {
                    ShapeKind::Slider if k == ShapeKey::Nudge => format!("{:+}%", ((v - 0.5) * 200.0).round() as i32),
                    ShapeKind::Slider => format!("{}%", (v * 100.0).round() as i32),
                    _ => k.options()[k.index_of(v)].clone(),
                };
                ShapeRowView {
                    key: k,
                    value,
                    kind,
                    norm: v,
                    bipolar: k == ShapeKey::Nudge,
                    segs: if kind == ShapeKind::Segments { k.options().len() as i32 } else { 0 },
                    seg: k.index_of(v) as i32,
                    enabled: on,
                    tip: if on {
                        ""
                    } else if k == ShapeKey::Voices {
                        NO_VOICES
                    } else {
                        NOT_AVAILABLE
                    },
                    selected: self.shape_ui.key == k,
                    curved: s.curve(k).is_some(),
                }
            })
            .collect()
    }

    /// The Shape tab is showing (a MIDI clip with notes, Shape picked or opening first).
    pub(crate) fn shape_tab_shown(&self) -> bool {
        let Some(c) = self.pr_clip() else { return false };
        c.pattern.is_none()
            && !c.notes.is_empty()
            && self.composer_shape_tab.unwrap_or(c.composer.as_ref().is_none_or(|k| k.playing.is_none()))
    }

    /// Drawing curves in the piano roll (the toolbar's Shape button, on the Shape tab).
    pub(crate) fn shape_drawing(&self) -> bool {
        self.shape_ui.drawing && self.shape_tab_shown()
    }

    /// Patterns / Shape tabs: the pick holds on every clip.
    pub fn composer_tab(&mut self, shape: bool) {
        self.composer_shape_tab = Some(shape);
        self.ui_dirty = true;
    }

    fn shape_target(&self) -> Option<(MidiClip, Vec<Note>, Option<PartType>, Shape)> {
        let c = self.composer_clip()?;
        if c.pattern.is_some() || c.notes.is_empty() {
            return None;
        }
        let (base, source, s) = self.shape_state(&c);
        Some((c, base, source, s))
    }

    fn commit_shape(&mut self, c: &MidiClip, base: Vec<Note>, source: Option<PartType>, s: Shape, old: &Shape) {
        if s == *old {
            return;
        }
        let notes = self.shaped(&base, &s, source, c);
        let mut comp = c.composer.clone().unwrap_or_default();
        comp.base = if s == Shape::neutral(source) { Vec::new() } else { base };
        comp.set_shape(source, s);
        self.execute(Command::SetComposer {
            clip: c.id,
            edit: ComposerEdit::Shape,
            composer: Some(comp),
            notes,
            pattern: c.pattern.clone(),
        });
    }

    /// A rack row (\`shape_op\`): sliders take a 0..1 value and coalesce into one undo
    /// step until released; steppers step by \`value\`'s sign; segments pick \`value\`
    /// (0-based); reset puts the base back to neutral; select (or any edit) makes the
    /// row's curve the one the piano roll edits.
    pub fn composer_shape(&mut self, row: i32, op: i32, value: f32) {
        use shape_op::*;
        self.ui_dirty = true;
        if op == RELEASE {
            self.doc.seal_coalescing();
            return;
        }
        let Some((c, base, source, old)) = self.shape_target() else { return };
        let Some(&k) = visible_keys(&old).get(row.max(0) as usize) else { return };
        self.shape_ui.key = k;
        if op == SELECT || !enabled(k, &base, source) {
            return;
        }
        let mut s = old.clone();
        let n = k.options().len();
        match (kind(k), op) {
            (ShapeKind::Slider, PRESS | DRAG) => s.set_norm(k, value),
            (ShapeKind::Stepper, STEP) => {
                let i = (k.index_of(s.norm(k)) as i32 + value.signum() as i32).clamp(0, n as i32 - 1);
                s.set_norm(k, k.value_of(i as usize));
            }
            (ShapeKind::Segments, PICK) => s.set_norm(k, k.value_of(value.round().max(0.0) as usize)),
            (_, RESET) => s.set_norm(k, Shape::neutral(source).norm(k)),
            _ => return,
        }
        self.commit_shape(&c, base, source, s, &old);
        if op != PRESS && op != DRAG {
            self.doc.seal_coalescing();
        }
    }

    /// Toolbar: Shape (draw curves in the piano roll) on or off.
    pub fn shape_draw_toggle(&mut self) {
        self.shape_ui.drawing = !self.shape_ui.drawing;
        self.ui_dirty = true;
    }

    pub fn shape_brush(&mut self, brush: i32) {
        self.shape_ui.brush = brush.clamp(0, BRUSHES.len() as i32 - 1) as usize;
        self.ui_dirty = true;
    }

    /// Toolbar Reset: every curve of what plays goes; the base values stay.
    pub fn shape_reset(&mut self) {
        let Some((c, base, source, old)) = self.shape_target() else { return };
        let mut s = old.clone();
        s.curves.clear();
        self.commit_shape(&c, base, source, s, &old);
        self.doc.seal_coalescing();
        self.ui_dirty = true;
    }

    /// Any curve drawn on what plays (the toolbar's Reset is live).
    pub fn shape_has_curves(&self) -> bool {
        self.shape_tab_shown() && self.shape_target().is_some_and(|(.., s)| !s.curves.is_empty())
    }

    /// A pointer in the piano roll grid while drawing (kind 0 down, 1 move, 2 up).
    /// Pencil sets the nearest beat's point, Steps the beat under the pointer, both
    /// filling the beats crossed since the last move; Ramp draws a straight line from
    /// the press. One stroke is one undo step.
    pub fn shape_draw(&mut self, kind: i32, x: f32, y: f32) {
        self.ui_dirty = true;
        if kind == 2 {
            if self.shape_ui.stroke.take().is_some() {
                self.doc.seal_coalescing();
            }
            return;
        }
        let v = self.pr_view();
        let (beat, value) = (v.beat_at(x.max(GRID_X)).max(0.0), y_value(&v, y));
        let Some((c, base, source, old)) = self.shape_target() else { return };
        let k = self.shape_ui.key;
        if !enabled(k, &base, source) {
            return;
        }
        let len = shape::curve_len(c.length);
        if kind == 0 {
            self.shape_ui.stroke = Some(Stroke { key: k, orig: fitted(old.curve(k), len), start: (beat, value), last: (beat, value) });
        }
        let brush = self.shape_ui.brush;
        let Some(st) = self.shape_ui.stroke.as_mut().filter(|s| s.key == k) else { return };
        let b0 = old.norm(k);
        let index = |b: f64| -> usize { (if brush == 1 { b.floor() } else { b.round() }).clamp(0.0, (len - 1) as f64) as usize };
        let (from, to) = if brush == 2 { (st.start, (beat, value)) } else { (st.last, (beat, value)) };
        let mut points = if brush == 2 { st.orig.clone() } else { fitted(old.curve(k), len) };
        let (i0, i1) = (index(from.0), index(to.0));
        let (lo, hi) = (i0.min(i1), i0.max(i1));
        for (i, p) in points.iter_mut().enumerate().take(hi + 1).skip(lo) {
            let t = if i0 == i1 { 1.0 } else { (i as f32 - i0 as f32) / (i1 as f32 - i0 as f32) };
            *p = (from.1 + (to.1 - from.1) * t - b0).clamp(-1.0, 1.0);
        }
        st.last = (beat, value);
        let mut s = old.clone();
        s.set_curve(k, points);
        self.commit_shape(&c, base, source, s, &old);
    }

    /// Curves for the piano roll while the Shape tab shows: the edited one bright
    /// (flat at its base when nothing is drawn), the other drawn ones dim.
    pub(crate) fn shape_scene_curves(&self, v: &PrView) -> Vec<CurveDraw> {
        if !self.shape_tab_shown() {
            return Vec::new();
        }
        let Some((c, base, source, s)) = self.shape_target() else { return Vec::new() };
        let sel = self.shape_ui.key;
        let clip_beats = c.length as f64 / PPQ as f64;
        let (b0, b1) = (v.beat_at(GRID_X).max(0.0), v.beat_at(v.w).min(clip_beats));
        if b1 <= b0 {
            return Vec::new();
        }
        let step = (2.0 / v.ppb.max(1e-3)).max(1.0 / 64.0);
        let samples = ((b1 - b0) / step).ceil() as usize;
        let alpha = if self.shape_ui.drawing { 1.0 } else { 0.55 };
        visible_keys(&s)
            .into_iter()
            .filter(|&k| k != sel && s.curve(k).is_some())
            .chain(std::iter::once(sel).filter(|&k| enabled(k, &base, source)))
            .map(|k| {
                let active = k == sel;
                let line = (0..=samples)
                    .map(|i| b0 + (i as f64 * step).min(b1 - b0))
                    .map(|b| (v.x_of(b), value_y(v, s.at(k, b.min(clip_beats - 1e-3)))))
                    .collect();
                let dots = match s.curve(k) {
                    Some(p) if active => (0..p.len())
                        .filter(|&i| i as f64 >= b0 && i as f64 <= b1)
                        .map(|i| (v.x_of(i as f64), value_y(v, s.at(k, i as f64))))
                        .collect(),
                    _ => Vec::new(),
                };
                let color = rgb(k.color()).map(|ch| ch * if active { alpha } else { 0.6 });
                CurveDraw { color, active, line, dots, guide: (active && k.bipolar()).then(|| value_y(v, 0.5)) }
            })
            .collect()
    }

    /// Draw a stroke through (beat, value) points with a brush, as the pointer would
    /// (review states and the self-test).
    pub(crate) fn shape_stroke(&mut self, key: ShapeKey, brush: usize, path: &[(f64, f32)]) {
        self.shape_ui.key = key;
        self.shape_ui.brush = brush;
        let v = self.pr_view();
        for (i, &(b, val)) in path.iter().enumerate() {
            self.shape_draw(if i == 0 { 0 } else { 1 }, v.x_of(b), value_y(&v, val));
        }
        self.shape_draw(2, 0.0, 0.0);
    }
}
