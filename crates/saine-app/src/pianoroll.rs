//! Piano roll (bottom panel, Clip tab): geometry, the wgpu draw list for the
//! note grid + keyboard (a second offscreen texture, same renderer as the
//! timeline), and the note-editing gestures.
//!
//! Every gesture previews locally (no undo step) and commits exactly ONE
//! command on release: `SetMidiNotes` for note edits, `ResizeMidiClip` (or a
//! `Batch`) for clip range changes. Toolbar transforms use the pure functions
//! in `saine_core::midi` on the selection (or all notes when nothing is
//! selected) with the footer's Grid / Amount values.
//!
//! Coordinates are texture-local logical points. The texture sits at panel
//! (255, 54.5) of the bottom panel, i.e. window (260, 563.5) in the 820 pt window
//! with the default 306 pt panel (layout from `34-midi-clip-selected-pianoroll.png`; the
//! panel height from the z100 clip editor captures, where the Clip tab row sits at 454 pt
//! like the Devices tab's).

use crate::controller::{BottomTab, Controller};
use crate::gpu::{DrawList, RectInst};
use crate::timeline::{Px, REF, RulerSteps, rgb};
use saine_core::midi::{self, ClipLoop, MidiClip, Note};
use saine_core::scale::{self, ClipScale, ScaleKey};
use saine_core::{Clip, ClipId, Command, PPQ, Ticks};
use std::collections::BTreeSet;
use std::time::{Duration, Instant};

/// Keyboard column width (white keys end at 50, 1 pt separator, grid from 51).
pub const GRID_X: f32 = 51.0;
/// Bar-number ruler (0..13.5), clip range bar (14.5..26.5), handle band (27..54).
pub const BAR_Y: (f32, f32) = (14.5, 26.5);
pub const BAND_Y: f32 = 27.0;
pub const GRID_Y: f32 = 54.5;
/// One semitone row (12 rows = 176.5 pt in the reference).
pub const ROW_H: f32 = 176.5 / 12.0;
/// Space left and right of the clip when it is fitted to the view, as a fraction of
/// the clip length (the reference app shows 1.06 clip lengths, measured in both clip editors).
pub const FIT_PAD: f64 = 0.03;
/// Texture origin inside the bottom panel.
pub const PANEL_ORIGIN: (f32, f32) = (255.0, 54.5);
/// The Clip tab's default panel height; the app sizes the editors from the live height.
#[cfg(test)]
pub const CLIP_PANEL_H: f32 = crate::highlight::DEFAULT_BOTTOM_H;
pub const FOOTER_H: f32 = 54.5;
/// C4 row top at y 106.5 (window 670 with the default panel) in the reference.
pub const DEFAULT_SCROLL: f32 = GRID_Y + 67.0 * ROW_H - 106.5;
const DOUBLE_CLICK: Duration = Duration::from_millis(350);

#[derive(Debug, Clone, Copy)]
pub struct PrView {
    /// Left edge of the grid: `GRID_X` with the keyboard, 0 in the audio clip editor.
    pub gx: f32,
    pub w: f32,
    pub h: f32,
    /// Points per beat.
    pub ppb: f64,
    /// Clip-relative beat at x = GRID_X.
    pub start: f64,
    pub scroll: f32,
    pub rows: Rows,
    /// Row height: \`ROW_H\` until the vertical zoom slider moves.
    pub row_h: f32,
}

/// The pitch rows the piano roll shows, top first: all 128, or (scale view) the scale
/// pitches plus those holding notes. A hidden pitch sits on the row of the nearest shown
/// pitch above it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rows {
    row: [u8; 128],
    pitch: [u8; 128],
    n: u16,
}

impl Rows {
    pub const ALL: Rows = {
        let mut r = Rows { row: [0; 128], pitch: [0; 128], n: 128 };
        let mut i = 0;
        while i < 128 {
            r.row[i] = (127 - i) as u8;
            r.pitch[i] = (127 - i) as u8;
            i += 1;
        }
        r
    };

    /// Rows for these pitches, given top first.
    pub fn only(pitches: &[u8]) -> Rows {
        if pitches.is_empty() {
            return Rows::ALL;
        }
        let mut r = Rows { row: [0; 128], pitch: [0; 128], n: pitches.len().min(128) as u16 };
        for (i, &p) in pitches.iter().take(128).enumerate() {
            r.pitch[i] = p & 127;
        }
        let (mut at, mut k) = (0usize, 0usize);
        for p in (0..128u8).rev() {
            while k < r.n as usize && r.pitch[k] >= p {
                at = k;
                k += 1;
            }
            r.row[p as usize] = at as u8;
        }
        r
    }

    pub fn len(&self) -> usize {
        self.n as usize
    }

    pub fn folded(&self) -> bool {
        self.n < 128
    }
}

impl PrView {
    /// Horizontal zoom fits the clip with `FIT_PAD` on both sides (reference behaviour).
    pub fn fit(size: (f32, f32), clip_beats: f64, scroll: f32) -> Self {
        Self::fit_from(GRID_X, size, clip_beats, scroll)
    }
    pub fn fit_from(gx: f32, size: (f32, f32), clip_beats: f64, scroll: f32) -> Self {
        let beats = clip_beats.max(0.25);
        let ppb = (size.0 - gx).max(40.0) as f64 / (beats * (1.0 + 2.0 * FIT_PAD));
        PrView { gx, w: size.0, h: size.1, ppb, start: -FIT_PAD * beats, scroll, rows: Rows::ALL, row_h: ROW_H }
    }
    pub fn x_of(&self, beat: f64) -> f32 {
        self.gx + ((beat - self.start) * self.ppb) as f32
    }
    pub fn beat_at(&self, x: f32) -> f64 {
        self.start + (x - self.gx) as f64 / self.ppb
    }
    pub fn y_of(&self, pitch: i32) -> f32 {
        let row = if (0..128).contains(&pitch) { self.rows.row[pitch as usize] as i32 } else { 127 - pitch };
        GRID_Y + row as f32 * self.row_h - self.scroll
    }
    pub fn pitch_at(&self, y: f32) -> i32 {
        let r = ((y - GRID_Y + self.scroll) / self.row_h).floor() as i32;
        let n = self.rows.len() as i32;
        if r < 0 {
            self.rows.pitch[0] as i32 - r
        } else if r >= n {
            self.rows.pitch[n as usize - 1] as i32 - (r - n + 1)
        } else {
            self.rows.pitch[r as usize] as i32
        }
    }
    pub fn max_scroll(&self) -> f32 {
        (self.rows.len() as f32 * self.row_h - (self.h - GRID_Y)).max(0.0)
    }
    /// Pitches of the rows on screen, top first.
    pub fn visible(&self) -> impl Iterator<Item = i32> + '_ {
        let first = ((self.scroll / self.row_h).floor() as i32).max(0);
        let last = (((self.h - GRID_Y + self.scroll) / self.row_h).ceil() as i32).min(self.rows.len() as i32 - 1);
        (first..=last).map(|r| self.rows.pitch[r as usize] as i32)
    }
    pub fn note_rect(&self, n: &Note) -> (f32, f32, f32, f32) {
        let x0 = self.x_of(n.start as f64 / PPQ as f64);
        let x1 = self.x_of((n.start + n.length) as f64 / PPQ as f64);
        (x0, self.y_of(n.pitch as i32), (x1 - x0).max(2.0), self.row_h)
    }
}

/// Points per beat at horizontal slider position \`z\`: from the fitted zoom (0) to one reference
/// px per tick (1), on a log scale.
pub fn zoom_ppb(fit: f64, z: f32) -> f64 {
    let max = PPQ as f64 * REF;
    if fit >= max { fit } else { fit * (max / fit).powf(z.clamp(0.0, 1.0) as f64) }
}

/// Row height range of the vertical slider: at least 3 reference px (16 in the scale view) and
/// enough to fill the view with the rows shown, at most 32 reference px.
pub fn row_h_range(v: &PrView) -> (f32, f32) {
    let r = REF as f32;
    let fill = (v.h - GRID_Y).max(0.0) / v.rows.len().max(1) as f32;
    let lo = (if v.rows.folded() { 16.0 } else { 3.0 } * r).max(fill);
    (lo, (32.0 * r).max(lo))
}

pub fn is_black(pitch: i32) -> bool {
    matches!(pitch.rem_euclid(12), 1 | 3 | 6 | 8 | 10)
}

/// "bars.beats.sixteenths" position (1-based bars/beats), as in the reference footer ("1.1.0").
pub fn fmt_pos(t: Ticks, tpb: Ticks) -> String {
    let t = t.max(0);
    format!("{}.{}.{}", t / tpb + 1, (t % tpb) / PPQ + 1, (t % PPQ) / (PPQ / 4))
}

/// Duration in bars.beats.sixteenths (0-based), e.g. one bar = "1.0.0".
pub fn fmt_len(t: Ticks, tpb: Ticks) -> String {
    let t = t.max(0);
    format!("{}.{}.{}", t / tpb, (t % tpb) / PPQ, (t % PPQ) / (PPQ / 4))
}

#[derive(Debug, Clone)]
pub enum PrDrag {
    /// Move (or resize the right edge of) the notes in `ids`; `dup` = Alt-drag duplicate.
    Notes { resize: bool, dup: bool, ids: Vec<u32>, anchor_beat: f64, anchor_pitch: i32, anchor: Note, preview: Vec<Note> },
    /// Pen: a new note whose length follows the pointer until release.
    Draw { id: u32, preview: Vec<Note> },
    Marquee { x0: f32, y0: f32, x1: f32, y1: f32, base: BTreeSet<u32> },
    /// Clip range handles (clip-relative ticks; the view stays fixed during the drag).
    ClipStart { start: Ticks },
    ClipEnd { end: Ticks },
    /// Loop bar: move it (edge None) or drag its start (Some(false)) or end (Some(true)).
    Loop { edge: Option<bool>, orig: ClipLoop, anchor: Ticks, cur: ClipLoop },
}

/// Footer value fields (drag vertically; Grid cycles on click).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Field {
    Grid = 0,
    QuantizeAmount = 1,
    HumanizeAmount = 2,
    Velocity = 3,
    Length = 4,
    LoopPosition = 5,
}

impl Field {
    fn from_i32(i: i32) -> Option<Field> {
        Some(match i {
            0 => Field::Grid,
            1 => Field::QuantizeAmount,
            2 => Field::HumanizeAmount,
            3 => Field::Velocity,
            4 => Field::Length,
            5 => Field::LoopPosition,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone)]
pub struct ValueDrag {
    orig: f64,
    /// Velocity edits of the selected notes (committed on release).
    preview: Option<Vec<Note>>,
    /// Length edit (ticks), committed on release: the loop's length while the clip loops,
    /// else the clip's.
    length: Option<Ticks>,
    /// Loop Position edit (ticks), committed on release.
    position: Option<Ticks>,
}

impl ValueDrag {
    pub fn length_preview(&self) -> Option<Ticks> {
        self.length
    }
}

/// View state of the piano roll (not part of the document).
#[derive(Debug)]
pub struct PrState {
    pub clip: Option<ClipId>,
    pub selected: BTreeSet<u32>,
    pub pen: bool,
    pub focused: bool,
    /// Grid denominator: 4, 8, 16, 32, or 6, 12, 24, 48 for the triplet grids.
    pub grid: u32,
    /// Open clip editor dropdown (`clipedit::CLIP_MENUS` index), -1 when none.
    pub menu: i32,
    pub quant_amount: f64,
    pub humanize_amount: f64,
    pub velocity: u8,
    pub scroll: f32,
    pub drag: Option<PrDrag>,
    pub value_drag: Option<ValueDrag>,
    last_down: Option<(Instant, f32, f32)>,
    pub size: (f32, f32),
    humanize_count: u64,
    /// Step sequencer view of a pattern clip (src/stepseq.rs).
    pub seq: crate::stepseq::SeqState,
    /// Audio clip editor gesture (warp marker / transient), see `warpedit`.
    pub ac_drag: Option<crate::warpedit::AcDrag>,
    /// Audio clip editor value being typed in (double-click), and its clip.
    pub ac_edit: Option<(crate::warpedit::AcEdit, ClipId)>,
    pub ac_last_down: Option<(Instant, f32, f32)>,
    /// The rows the view last showed: when they change (Scale rows on or off, a new scale,
    /// undo or redo) the pitch in the middle of the view stays in place.
    pub rows: Rows,
    /// Toolbar zoom sliders: horizontal 0 (fit the clip) .. 1 (one reference px per tick), with
    /// the beat at the grid's left edge, and the row height the vertical slider set.
    pub hzoom: f32,
    pub hstart: f64,
    pub vzoom: Option<f32>,
}

impl Default for PrState {
    fn default() -> Self {
        PrState {
            clip: None,
            selected: BTreeSet::new(),
            pen: false,
            focused: false,
            grid: 16,
            menu: -1,
            quant_amount: 1.0,
            humanize_amount: 0.5,
            velocity: 100,
            scroll: DEFAULT_SCROLL,
            drag: None,
            value_drag: None,
            last_down: None,
            size: (1174.5, 281.0),
            humanize_count: 0,
            seq: Default::default(),
            ac_drag: None,
            ac_edit: None,
            ac_last_down: None,
            rows: Rows::ALL,
            hzoom: 0.0,
            hstart: 0.0,
            vzoom: None,
        }
    }
}

pub struct PrScene<'a> {
    pub view: PrView,
    pub scale: f32,
    /// Clip range in clip-relative beats (preview during a handle drag).
    pub range: (f64, f64),
    pub notes: &'a [Note],
    pub selected: &'a BTreeSet<u32>,
    pub color: [f32; 3],
    pub playhead: f64,
    pub beats_per_bar: f64,
    /// Length of one beat of the time signature, in quarter notes (4 / denominator).
    pub beat_len: f64,
    /// The editor's grid value in beats (the piano roll's sub-beat lines).
    pub grid_beats: f64,
    /// Ruler and grid steps for the editor's zoom.
    pub steps: RulerSteps,
    pub marquee: Option<(f32, f32, f32, f32)>,
    /// Audio clip editor: no keyboard or pitch rows, gain dots on the clip handles.
    pub audio: bool,
    /// Warped audio clips: warp markers and transients (clip-relative beats).
    pub warp_markers: Vec<f64>,
    pub transients: Vec<f64>,
    /// Audio clip editor: the clip's waveform, one lane per channel (`Controller::clip_lanes`).
    pub wave: Option<(Vec<crate::peaks::Slot>, Vec<crate::peaks::WaveSeg>)>,
    /// The clip's scale while the footer's Scale switch is on: in-scale rows and keys are tinted.
    pub key: Option<ScaleKey>,
    /// Shape tab curves over the notes (src/shape_rack.rs).
    pub curves: Vec<crate::shape_rack::CurveDraw>,
    /// MIDI clip loop bar: range in clip-relative beats and whether looping is on.
    pub clip_loop: Option<(f64, f64, bool)>,
}

fn rgba(c: [f32; 3], a: f32) -> [f32; 4] {
    [c[0], c[1], c[2], a]
}
fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}
const WHITE: [f32; 3] = [1.0, 1.0, 1.0];
const BLACK: [f32; 3] = [0.0, 0.0, 0.0];

/// The piano roll's finest grid step in beats (S78): the grid value, doubled (or tripled when the
/// beat is not a multiple of twice the step) until its lines are 16 reference px apart. `None`
/// when that reaches a beat, then only beat and bar lines remain.
pub fn sub_step(grid_beats: f64, beat_len: f64, ppb_ref: f64) -> Option<f64> {
    if !(grid_beats > 0.0) {
        return None;
    }
    let mut r = grid_beats;
    while r < beat_len - 1e-9 && r * ppb_ref < 16.0 {
        r = if ((beat_len / (r * 2.0)).round() * (r * 2.0) - beat_len).abs() < 1e-9 { r * 2.0 } else { r * 3.0 };
    }
    (r < beat_len - 1e-9).then_some(r)
}

/// Vertical grid lines of the editors in lane band y0..y1, from x_from on. The piano roll draws
/// sub-beat lines at its grid value, beat lines once a beat is 8 reference px wide, and bar lines;
/// the audio clip editor draws the timeline's step with bar, beat and sub weights (S78).
fn vlines(out: &mut Vec<RectInst>, sc: &PrScene, px: &Px, y0: f32, y1: f32, x_from: f32) {
    let v = &sc.view;
    let ppb_ref = v.ppb / REF;
    let (bar, beat_len) = (sc.beats_per_bar, sc.beat_len);
    let (first, last) = (v.beat_at(x_from.max(v.gx)), v.beat_at(v.w));
    // Lines at every multiple of `step`; `class` gives (alpha, width) or None to skip a spot.
    let mut each = |step: f64, class: &dyn Fn(f64) -> Option<(f32, f32)>| {
        let mut i = (first / step).floor() as i64;
        loop {
            let beat = i as f64 * step;
            if beat > last {
                break;
            }
            i += 1;
            let x = v.x_of(beat);
            if x < x_from {
                continue;
            }
            if let Some((alpha, width)) = class(beat) {
                out.push(px.rect(x - width / 2.0, y0, width, y1 - y0, rgba(WHITE, alpha)));
            }
        }
    };
    if sc.audio {
        each(sc.steps.tick_beats, &|b| {
            Some(if RulerSteps::on(b, bar) {
                (0.06, 1.0)
            } else if RulerSteps::on(b, beat_len) {
                (0.038, 0.5)
            } else {
                (0.024, 0.5)
            })
        });
        return;
    }
    if let Some(step) = sub_step(sc.grid_beats, beat_len, ppb_ref) {
        each(step, &|b| (!RulerSteps::on(b, beat_len)).then_some((0.024, 1.0)));
    }
    if beat_len * ppb_ref >= 8.0 {
        each(beat_len, &|b| (!RulerSteps::on(b, bar)).then_some((0.038, 1.0)));
    }
    each(bar, &|_| Some((0.06, 1.0)));
}

/// Grid rows, beat lines, notes and keyboard (`base`, scissored to the note area),
/// then ruler, clip range bar, handles, playhead and marquee (`over`).
pub fn build(dl: &mut DrawList, sc: &PrScene) {
    dl.clear_all();
    let px = Px { s: sc.scale };
    let v = &sc.view;
    let (w, h, gx) = (v.w, v.h, v.gx);
    dl.clear = [0.102, 0.106, 0.118]; // #1a1b1e
    dl.lanes_clip = ((GRID_Y * sc.scale).round() as u32, (h * sc.scale).round() as u32);
    let b = &mut dl.base;

    // ---- rows (the audio clip editor has one plain lane) ----
    if sc.audio {
        b.push(px.rect(gx, GRID_Y, w - gx, h - GRID_Y, rgba(rgb(0x141518), 1.0)));
    } else {
        for p in v.visible() {
            let y = v.y_of(p);
            let c = if is_black(p) { 0x151619 } else { 0x191a1d };
            b.push(px.rect(gx, y, w - gx, v.row_h, rgba(rgb(c), 1.0)));
            if let Some(k) = sc.key.filter(|k| k.contains(p)) {
                b.push(px.rect(gx, y, w - gx, v.row_h, rgba(sc.color, if k.is_root(p) { 0.22 } else { 0.08 })));
            }
            if p.rem_euclid(12) == 0 {
                b.push(px.rect(gx, y + v.row_h - 1.0, w - gx, 1.0, rgba(rgb(0x252629), 1.0)));
            }
        }
    }
    vlines(b, sc, &px, GRID_Y, h, gx);
    // Outside the clip range: darker.
    let (cx0, cx1) = (v.x_of(sc.range.0), v.x_of(sc.range.1));
    if cx0 > gx {
        b.push(px.rect(gx, GRID_Y, cx0.min(w) - gx, h - GRID_Y, rgba(BLACK, 0.2)));
    }
    if cx1 < w {
        let x = cx1.max(gx);
        b.push(px.rect(x, GRID_Y, w - x, h - GRID_Y, rgba(BLACK, 0.2)));
    }

    // ---- transients (warped audio clips): faint guides through the lane ----
    for &t in &sc.transients {
        let x = v.x_of(t);
        if x >= gx && x <= w {
            b.push(px.rect(x - 0.25, GRID_Y, 0.5, h - GRID_Y, rgba(WHITE, 0.2)));
        }
    }

    // ---- waveform (audio clip editor): track colour halfway to white, 84 % of each
    // channel's lane. Stereo splits the lane in two with a gap of 4 % (at least
    // 2 px), a faint centre line in each and a rule between them, as the reference app does.
    if let Some((slots, segs)) = &sc.wave {
        let stereo = slots.len() == 2;
        let all = h - GRID_Y;
        let gap = if stereo { (all * 0.04).max(2.0) } else { 0.0 };
        let lane = (all - gap) / slots.len().max(1) as f32;
        let wh = lane * 0.84 / 0.92;
        let color = rgba(mix(sc.color, WHITE, 0.5), 0.85);
        let (x0, x1) = (cx0.max(gx), cx1.min(w));
        for (i, slot) in slots.iter().enumerate() {
            let top = GRID_Y + i as f32 * (lane + gap);
            if x1 > x0 {
                b.push(px.rect(x0, top + lane / 2.0 - 0.5, x1 - x0, 1.0, rgba(WHITE, 0.03)));
            }
            crate::peaks::push_wave(&mut dl.waves, *slot, segs, |b| v.x_of(b), (x0, x1), (top + (lane - wh) / 2.0, wh), sc.scale, color);
        }
        if stereo {
            b.push(px.rect(gx, GRID_Y + lane + gap / 2.0 - 0.5, w - gx, 1.0, rgba(WHITE, 0.06)));
        }
    }

    // ---- notes ----
    for n in sc.notes {
        let (x, y, nw, nh) = v.note_rect(n);
        if x > w || x + nw < gx || y > h || y + nh < GRID_Y {
            continue;
        }
        let beat = n.start as f64 / PPQ as f64;
        let inside = beat < sc.range.1 && beat >= sc.range.0;
        let a = if inside { 1.0 } else { 0.4 };
        let vel = 0.55 + 0.45 * n.velocity as f32 / 127.0;
        if sc.selected.contains(&n.id) {
            b.push(px.rounded(x, y + 0.5, nw, nh - 1.0, 2.0, rgba(WHITE, a)));
            b.push(px.rounded(x + 1.0, y + 1.5, nw - 2.0, nh - 3.0, 1.5, rgba(mix(sc.color, WHITE, 0.45), a)));
        } else {
            b.push(px.rounded(x, y + 0.5, nw, nh - 1.0, 2.0, rgba(mix(sc.color, BLACK, 0.45), a)));
            b.push(px.rounded(x + 0.5, y + 1.0, nw - 1.0, nh - 2.0, 1.5, rgba(mix(mix(sc.color, BLACK, 0.35), sc.color, vel), a)));
        }
    }

    // ---- Shape curves (over the notes) ----
    for c in &sc.curves {
        let (th, a) = if c.active { (2.0, 1.0) } else { (1.25, 0.7) };
        if let Some(gy) = c.guide {
            let mut x = gx;
            while x < w {
                b.push(px.rect(x, gy - 0.5, 4.0, 1.0, rgba(WHITE, 0.18)));
                x += 8.0;
            }
        }
        for s in c.line.windows(2) {
            let ((x0, y0), (x1, y1)) = (s[0], s[1]);
            b.push(px.rect(x0, y0.min(y1) - th / 2.0, (x1 - x0).max(1.0), (y0 - y1).abs() + th, rgba(c.color, a)));
        }
        for &(x, y) in &c.dots {
            b.push(px.rounded(x - 3.0, y - 3.0, 6.0, 6.0, 3.0, rgba(c.color, 1.0)));
            b.push(px.rounded(x - 1.5, y - 1.5, 3.0, 3.0, 1.5, rgba(rgb(0x141518), 1.0)));
        }
    }

    // ---- keyboard (covers notes scrolled left of the grid) ----
    if !sc.audio {
        b.push(px.rect(0.0, GRID_Y, gx, h - GRID_Y, rgba(rgb(0x1a1b1e), 1.0)));
        b.push(px.rect(0.5, GRID_Y, 49.5, h - GRID_Y, rgba(WHITE, 1.0)));
        for p in v.visible() {
            let y = v.y_of(p);
            let line = rgba(rgb(0x555555), 1.0);
            if is_black(p) {
                b.push(px.rect(30.5, y + v.row_h / 2.0 - 1.0, 19.5, 1.0, line));
                b.push(px.rect(0.5, y, 30.0, v.row_h, rgba(rgb(0x1a1a1a), 1.0)));
                b.push(px.rect(0.5, y, 30.0, 1.0, rgba(rgb(0x3c3c3c), 1.0)));
                b.push(px.rect(0.5, y + v.row_h - 1.0, 30.0, 1.0, rgba(rgb(0x0d0d0d), 1.0)));
            } else if matches!(p.rem_euclid(12), 0 | 5) || v.rows.folded() {
                b.push(px.rect(0.5, y + v.row_h - 1.0, 49.5, 1.0, line));
            }
            if let Some(k) = sc.key.filter(|k| k.contains(p)) {
                let sw = if k.is_root(p) { 6.0 } else { 3.0 };
                b.push(px.rect(50.0 - sw, y, sw, v.row_h, rgba(sc.color, 1.0)));
            }
        }
        b.push(px.rect(50.0, GRID_Y, 1.0, h - GRID_Y, rgba(rgb(0x333333), 1.0)));
        // Soft fades at the top and bottom of the keyboard.
        for i in 0..8 {
            let t = i as f32 / 8.0;
            b.push(px.rect(0.5, GRID_Y + i as f32 * 2.0, 49.5, 2.0, rgba(BLACK, 0.45 * (1.0 - t))));
            b.push(px.rect(0.5, h - 10.0 + i as f32 * 1.25, 49.5, 1.25, rgba(BLACK, 0.28 * t)));
        }
    }

    // ---- header: ruler, clip range bar, handle band (over) ----
    let o = &mut dl.over;
    o.push(px.rect(gx, 14.0, w - gx, GRID_Y - 14.0, rgba(rgb(0x1e2024), 1.0)));
    let rx = cx0.max(gx);
    if rx < w {
        o.push(px.rect(rx, 0.0, w - rx, 13.5, rgba(rgb(0x2c2d30), 1.0)));
        o.push(px.rect(rx, 13.5, w - rx, 0.5, rgba(rgb(0x3d3e41), 1.0)));
        // Ruler ticks (S77): a full-height tick every `tick_bars` bars, and, while a beat is 16
        // reference px wide, a weak tick on every other beat of every bar, 2 px in from top and bottom.
        let st = &sc.steps;
        let bar = sc.beats_per_bar;
        let beat_ticks = sc.beat_len * v.ppb / REF >= 16.0;
        let mut i = (sc.range.0.max(v.beat_at(gx)) / sc.beat_len).floor() as i64;
        loop {
            let beat = i as f64 * sc.beat_len;
            let x = v.x_of(beat);
            if x > w {
                break;
            }
            i += 1;
            if beat < sc.range.0 - 1e-9 || x < rx {
                continue;
            }
            if RulerSteps::on(beat, bar) {
                if ((beat / bar).round() as i64).rem_euclid(st.tick_bars) == 0 {
                    o.push(px.rect(x, 0.0, 0.5, 13.5, rgba(rgb(0x63656d), 1.0)));
                }
            } else if beat_ticks {
                o.push(px.rect(x, 1.7, 0.5, 13.5 - 3.4, rgba(rgb(0x303236), 1.0)));
            }
        }
    }
    let mut tmp = Vec::new();
    vlines(&mut tmp, sc, &px, BAND_Y, GRID_Y, gx);
    o.extend(tmp);
    for (y, hh) in [(27.0, 0.5), (40.5, 1.0), (54.0, 0.5)] {
        o.push(px.rect(gx, y, w - gx, hh, rgba(rgb(0x292b2e), 1.0)));
    }
    let bar = mix(sc.color, rgb(0x86a6ff), 0.5);
    let (bar, (bx0, bx1)) = match sc.clip_loop {
        Some((s, e, on)) => (if on { bar } else { rgb(0x4a4b50) }, (v.x_of(s), v.x_of(e))),
        None => (bar, (cx0, cx1)),
    };
    if bx1 > gx && bx0 < w {
        let x0 = bx0.max(gx);
        o.push(px.rect(x0, BAR_Y.0, bx1.min(w) - x0, BAR_Y.1 - BAR_Y.0, rgba(bar, 1.0)));
    }
    for (x, left) in [(cx0, true), (cx1, false)] {
        if x < gx - 1.0 || x > w + 1.0 {
            continue;
        }
        o.push(px.rect(x - 0.5, BAND_Y, 1.0, h - BAND_Y, rgba(WHITE, 1.0)));
        for r in 0..16 {
            let ww = 5.0 * (1.0 - r as f32 / 16.0);
            let hx = if left { x - 0.5 } else { x + 0.5 - ww };
            o.push(px.rect(hx, 29.5 + r as f32 * 0.5, ww, 0.5, rgba(WHITE, 1.0)));
        }
        // Audio clips: a gain pin under each handle (rounded top, pointed bottom).
        if sc.audio {
            let pin = rgba(mix(sc.color, rgb(0x383c42), 0.5), 1.0);
            o.push(px.rounded(x - 3.8, 42.8, 7.6, 7.0, 2.5, pin));
            for r in 0..8 {
                let ww = 7.6 * (1.0 - r as f32 / 8.5);
                o.push(px.rect(x - ww / 2.0, 48.0 + r as f32 * 0.75, ww, 0.75, pin));
            }
        }
    }
    // ---- warp markers: a flag in the upper handle band and a line through the lane ----
    let marker = mix(sc.color, WHITE, 0.55);
    for &m in &sc.warp_markers {
        let x = v.x_of(m);
        if x < gx - 4.0 || x > w + 4.0 {
            continue;
        }
        o.push(px.rect(x - 0.5, BAND_Y + 1.0, 1.0, h - BAND_Y - 1.0, rgba(marker, 0.7)));
        for r in 0..10 {
            let ww = 8.0 * (1.0 - r as f32 / 10.0);
            o.push(px.rect(x - ww / 2.0, BAND_Y + 1.5 + r as f32, ww, 1.0, rgba(marker, 1.0)));
        }
    }
    // ---- playhead ----
    let phx = v.x_of(sc.playhead);
    if phx >= gx && phx <= w {
        o.push(px.rect(phx - 0.5, 14.0, 1.0, h - 14.0, rgba(rgb(0x8a8a8b), 1.0)));
    }
    // ---- marquee ----
    if let Some((x0, y0, x1, y1)) = sc.marquee {
        let (l, t, r, bt) = (x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1));
        o.push(px.rect(l, t, r - l, bt - t, rgba(WHITE, 0.07)));
        let c = rgba(WHITE, 0.6);
        o.push(px.rect(l, t, r - l, 0.5, c));
        o.push(px.rect(l, bt - 0.5, r - l, 0.5, c));
        o.push(px.rect(l, t, 0.5, bt - t, c));
        o.push(px.rect(r - 0.5, t, 0.5, bt - t, c));
    }
}

// ------------------------------------------------------------------ controller side

/// Renumber `extra` notes so their ids don't collide with `existing`.
fn renumber(existing: &[Note], extra: &mut [Note]) {
    let mut next = existing.iter().map(|n| n.id + 1).max().unwrap_or(1);
    for n in extra {
        n.id = next;
        next += 1;
    }
}

impl Controller {
    /// The MIDI clip shown in the piano roll (selected clip, Clip tab).
    pub fn pr_clip(&self) -> Option<&MidiClip> {
        if self.bottom_tab != BottomTab::Clip || self.mixer_open {
            return None;
        }
        self.project().midi_clip(self.selected_clip?)
    }

    pub fn pr_visible(&self) -> bool {
        self.pr_clip().is_some()
    }

    /// The audio clip shown in the audio clip editor (selected audio clip, Clip tab).
    pub fn ac_clip(&self) -> Option<&Clip> {
        if self.bottom_tab != BottomTab::Clip || self.mixer_open {
            return None;
        }
        self.project().clip(self.selected_clip?)
    }

    /// One of the two clip editors (piano roll or audio clip editor) is open.
    pub fn clip_editor_visible(&self) -> bool {
        self.pr_visible() || self.ac_clip().is_some()
    }

    /// Reset per-clip view state when another clip is opened.
    pub fn pr_sync_clip(&mut self) {
        let cur = self.pr_clip().map(|c| c.id);
        if cur != self.pr.clip {
            self.pr.clip = cur;
            self.pr.selected.clear();
            self.pr.drag = None;
            self.pr.value_drag = None;
            self.pr.scroll = DEFAULT_SCROLL;
            (self.pr.hzoom, self.pr.vzoom) = (0.0, None);
            self.pr.seq = crate::stepseq::SeqState::opened(self.pr_clip().and_then(|c| c.pattern.as_ref()));
            // A clip with notes opens centred on its pitch range (empty clips: C4 as in the reference).
            if let Some(c) = self.pr_clip()
                && !c.notes.is_empty()
            {
                let lo = c.notes.iter().map(|n| n.pitch).min().unwrap_or(60) as i32;
                let hi = c.notes.iter().map(|n| n.pitch).max().unwrap_or(60) as i32;
                self.pr.scroll = 0.0;
                let v = self.pr_view();
                self.pr.scroll = (v.y_of(lo) + v.y_of(hi)) / 2.0 - GRID_Y + v.row_h / 2.0 - (self.pr.size.1 - GRID_Y) / 2.0;
            }
            self.pr.rows = self.pr_view().rows;
        } else {
            let v = self.pr_view();
            if v.rows != self.pr.rows {
                let mid = GRID_Y + (v.h - GRID_Y) / 2.0;
                let mut old = PrView { rows: self.pr.rows, scroll: self.pr.scroll, ..v };
                old.scroll = old.scroll.clamp(0.0, old.max_scroll());
                let centre = old.pitch_at(mid);
                let top = PrView { scroll: 0.0, ..v };
                self.pr.scroll = (top.y_of(centre) + v.row_h / 2.0 - mid).clamp(0.0, v.max_scroll());
                self.pr.rows = v.rows;
            }
        }
        let pattern = self.pr_clip().and_then(|c| c.pattern.as_ref());
        if pattern.is_some() != self.pr.seq.pattern {
            self.pr.seq = crate::stepseq::SeqState::opened(pattern);
        }
        if let Some(c) = self.pr_clip() {
            let ids: BTreeSet<u32> = c.notes.iter().map(|n| n.id).collect();
            self.pr.selected.retain(|i| ids.contains(i));
        }
    }

    fn grid_ticks(&self) -> Ticks {
        midi::grid_ticks(self.pr.grid)
    }

    /// The step note edits snap to: the toolbar's snap size (shared with the timeline, S76) at
    /// the editor's zoom. Also the length of a drawn note and the shortest edit while snap is off.
    fn pr_snap_ticks(&self, v: &PrView) -> Ticks {
        let bar = self.project().ticks_per_bar() as f64 / PPQ as f64;
        let beats = crate::timeline::note_snap_beats(v.ppb / REF, bar, self.snap.grid);
        ((beats * PPQ as f64).round() as Ticks).max(1)
    }

    pub fn pr_view(&self) -> PrView {
        // Gestures keep the view, except Transpose: an unwarped clip's length follows it.
        if let Some(view) = self.pr.ac_drag.as_ref().filter(|d| !matches!(d.kind, crate::warpedit::AcDragKind::Transpose { .. })).map(|d| d.view) {
            return view;
        }
        if let Some(c) = self.ac_clip() {
            let len = self.project().clip_length_ticks(c);
            return self.pr_zoomed(PrView::fit_from(0.0, self.pr.size, len as f64 / PPQ as f64, 0.0));
        }
        let len = self.pr_clip().map(|c| c.length).unwrap_or(self.project().ticks_per_bar());
        let mut v = PrView::fit(self.pr.size, len as f64 / PPQ as f64, self.pr.scroll);
        if let Some((_, key)) = self.pr_scale().filter(|(s, _)| s.highlight && s.fold) {
            let notes = self.pr_clip().map(|c| c.notes.iter().map(|n| n.pitch).collect::<Vec<_>>()).unwrap_or_default();
            v.rows = Rows::only(&scale::fold_rows(key, notes));
        }
        let mut v = self.pr_zoomed(v);
        if let Some(rh) = self.pr.vzoom {
            let (lo, hi) = row_h_range(&v);
            v.row_h = rh.clamp(lo, hi);
        }
        v.scroll = v.scroll.clamp(0.0, v.max_scroll());
        v
    }

    /// The fitted view at the horizontal slider's zoom, panned within the fitted range.
    fn pr_zoomed(&self, mut v: PrView) -> PrView {
        if self.pr.hzoom <= 0.0 {
            return v;
        }
        let (fit_start, fit_end) = (v.start, v.beat_at(v.w));
        v.ppb = zoom_ppb(v.ppb, self.pr.hzoom);
        let shown = (v.w - v.gx) as f64 / v.ppb;
        v.start = self.pr.hstart.clamp(fit_start, (fit_end - shown).max(fit_start));
        v
    }

    /// Slider positions 0..1 for the toolbar: (vertical, horizontal).
    pub fn pr_zoom_values(&self) -> (f32, f32) {
        let v = self.pr_view();
        let (lo, hi) = row_h_range(&v);
        let vz = if hi > lo { ((v.row_h - lo) / (hi - lo)).clamp(0.0, 1.0) } else { 0.0 };
        (vz, self.pr.hzoom)
    }

    /// A zoom slider moved to \`z\` (0..1). Vertical (\`axis\` 0) keeps the row in the middle of the
    /// view in place; horizontal keeps the grid's left edge, as in the reference.
    pub fn pr_zoom(&mut self, axis: i32, z: f32) {
        let z = z.clamp(0.0, 1.0);
        let v = self.pr_view();
        if axis == 0 {
            let (lo, hi) = row_h_range(&v);
            let rh = lo + z * (hi - lo);
            let mid = (v.h - GRID_Y) / 2.0;
            self.pr.vzoom = Some(rh);
            self.pr.scroll = ((v.scroll + mid) / v.row_h * rh - mid).max(0.0);
        } else {
            self.pr.hstart = v.start;
            self.pr.hzoom = z;
        }
        self.ui_dirty = true;
    }

    /// Notes as displayed (gesture preview while dragging).
    pub fn pr_notes(&self) -> Vec<Note> {
        match (&self.pr.drag, &self.pr.value_drag) {
            (Some(PrDrag::Notes { preview, .. } | PrDrag::Draw { preview, .. }), _) => preview.clone(),
            (_, Some(ValueDrag { preview: Some(p), .. })) => p.clone(),
            _ => self.pr_clip().map(|c| c.notes.clone()).unwrap_or_default(),
        }
    }

    /// Clip range (clip-relative beats) and length including drag previews.
    fn pr_range(&self) -> (f64, f64) {
        let len = self.pr_clip().map(|c| c.length).unwrap_or(0);
        let len = self.pr.value_drag.as_ref().and_then(|d| d.length).filter(|_| !self.pr_loop().is_some_and(|l| l.on)).unwrap_or(len);
        let b = |t: Ticks| t as f64 / PPQ as f64;
        match self.pr.drag {
            Some(PrDrag::ClipStart { start }) => (b(start), b(len)),
            Some(PrDrag::ClipEnd { end }) => (0.0, b(end)),
            _ => (0.0, b(len)),
        }
    }

    fn pr_beat_len(&self) -> f64 {
        4.0 / self.project().transport.time_signature.denominator as f64
    }

    /// Ruler and grid steps for the editor's zoom (the reference app's rules, as the arrangement's).
    pub(crate) fn pr_steps(&self) -> RulerSteps {
        let p = self.project();
        RulerSteps::new(self.pr_view().ppb, p.ticks_per_bar() as f64 / PPQ as f64, self.pr_beat_len(), None)
    }

    pub fn pr_scene<'a>(&'a self, scale: f32, notes: &'a [Note]) -> Option<PrScene<'a>> {
        let p = self.project();
        if let Some(c) = self.ac_clip() {
            let color = p.find_clip(c.id).map(|(t, _)| crate::controller::track_color(p.tracks[t].color)).unwrap_or(WHITE);
            return Some(PrScene {
                view: self.pr_view(),
                scale,
                range: (0.0, p.clip_length_ticks(c) as f64 / PPQ as f64),
                notes,
                selected: &self.pr.selected,
                color,
                playhead: (self.playhead_ticks() - c.start) as f64 / PPQ as f64,
                beats_per_bar: p.ticks_per_bar() as f64 / PPQ as f64,
                beat_len: self.pr_beat_len(),
                grid_beats: self.grid_ticks() as f64 / PPQ as f64,
                steps: self.pr_steps(),
                marquee: None,
                audio: true,
                warp_markers: self.ac_marker_beats(),
                transients: self.ac_transient_beats(),
                wave: self.clip_lanes(c),
                key: None,
                curves: Vec::new(),
                clip_loop: self.pr_loop_preview().map(|l| (l.start as f64 / PPQ as f64, l.end as f64 / PPQ as f64, l.on)),
            });
        }
        let clip = self.pr_clip()?;
        let color = p.find_midi_clip(clip.id).map(|(t, _)| crate::controller::track_color(p.tracks[t].color)).unwrap_or(WHITE);
        let marquee = match self.pr.drag {
            Some(PrDrag::Marquee { x0, y0, x1, y1, .. }) => Some((x0, y0, x1, y1)),
            _ => None,
        };
        let view = self.pr_view();
        let curves = self.shape_scene_curves(&view);
        Some(PrScene {
            view,
            scale,
            range: self.pr_range(),
            notes,
            selected: &self.pr.selected,
            color,
            playhead: (self.playhead_ticks() - clip.start) as f64 / PPQ as f64,
            beats_per_bar: p.ticks_per_bar() as f64 / PPQ as f64,
                beat_len: self.pr_beat_len(),
                grid_beats: self.grid_ticks() as f64 / PPQ as f64,
                steps: self.pr_steps(),
            marquee,
            audio: false,
            warp_markers: Vec::new(),
            transients: Vec::new(),
            wave: None,
            key: self.pr_scale().filter(|(s, _)| s.highlight).map(|(_, k)| k),
            curves,
            clip_loop: self.pr_loop_preview().map(|l| (l.start as f64 / PPQ as f64, l.end as f64 / PPQ as f64, l.on)),
        })
    }

    /// The open MIDI clip's loop (pattern clips repeat their pattern instead).
    pub(crate) fn pr_loop(&self) -> Option<ClipLoop> {
        if let Some(c) = self.ac_clip() {
            let l = c.clip_loop.unwrap_or(saine_core::model::AudioLoop { on: false, start: 0, end: self.project().content_length_ticks(c), length: 0 });
            return Some(ClipLoop { on: l.on, start: l.start, end: l.end, span: 0 });
        }
        self.pr_clip().filter(|c| c.pattern.is_none()).and_then(|c| c.clip_loop)
    }

    /// The loop as shown: with a loop bar drag or a Loop Position / Length drag applied.
    pub(crate) fn pr_loop_preview(&self) -> Option<ClipLoop> {
        if let Some(PrDrag::Loop { cur, .. }) = &self.pr.drag {
            return Some(*cur);
        }
        let mut l = self.pr_loop()?;
        if let Some(d) = self.pr.value_drag.as_ref().filter(|_| l.on) {
            if let Some(p) = d.position {
                l = ClipLoop { start: p, end: p + l.len(), ..l };
            }
            if let Some(len) = d.length {
                l.end = l.start + len;
            }
        }
        Some(l)
    }

    /// The footer Loop switch. On: the kept loop range (else the whole clip) and the length
    /// the clip had while it last looped. Off: the clip plays once, to the loop end.
    pub fn pr_loop_toggle(&mut self) {
        if let Some(c) = self.ac_clip().cloned() {
            // Audio: off plays the source window once; on brings the kept range and looped
            // length back (else the whole content loops once over).
            let content = self.project().content_length_ticks(&c);
            let l = match c.clip_loop {
                Some(l) if l.on => saine_core::model::AudioLoop { on: false, ..l },
                kept => {
                    let whole = saine_core::model::AudioLoop { on: true, start: 0, end: content, length: content };
                    let l = kept.filter(|l| l.validate().is_ok() && l.start < content && l.end.min(content) - l.start > PPQ / 4).unwrap_or(whole);
                    saine_core::model::AudioLoop { on: true, end: l.end.min(content), ..l }
                }
            };
            self.execute(Command::SetClipLoop { clip: c.id, clip_loop: Some(l) });
            self.ui_dirty = true;
            return;
        }
        let Some(clip) = self.pr_clip().filter(|c| c.pattern.is_none()).cloned() else { return };
        let (clip_loop, length) = match clip.clip_loop {
            Some(l) if l.on => (ClipLoop { on: false, span: if clip.length > l.end { clip.length } else { 0 }, ..l }, l.end),
            kept => {
                let l = kept.filter(|l| l.validate().is_ok() && l.len() > PPQ / 4).unwrap_or(ClipLoop::whole(clip.length));
                (ClipLoop { on: true, span: 0, ..l }, if l.span > clip.length { l.span } else { clip.length })
            }
        };
        self.execute(Command::SetMidiLoop { clip: clip.id, clip_loop: Some(clip_loop), length });
        self.ui_dirty = true;
    }

    /// The open MIDI clip's scale view and the key it shows (its own, else the project key).
    pub(crate) fn pr_scale(&self) -> Option<(ClipScale, ScaleKey)> {
        let c = self.pr_clip()?;
        Some((c.scale.unwrap_or_default(), ClipScale::key_or(c.scale.as_ref(), self.project().transport.key)))
    }

    /// Change the open clip's scale view (one undo step).
    fn pr_set_scale(&mut self, f: impl FnOnce(&mut ClipScale, ScaleKey)) {
        let Some(id) = self.pr_clip().map(|c| c.id) else { return };
        let Some((old, key)) = self.pr_scale() else { return };
        let mut s = old;
        f(&mut s, key);
        if s == old {
            return;
        }
        self.execute(Command::SetClipScale { clip: id, scale: (s != ClipScale::default()).then_some(s) });
        self.pr_sync_clip();
        self.ui_dirty = true;
    }

    pub fn pr_scale_root(&mut self, root: usize) {
        self.pr_set_scale(|s, key| s.key = Some(ScaleKey { root: root.min(scale::ROOTS.len() - 1) as u8, kind: key.kind }));
    }

    /// Root note (menu 8) or Scale type (menu 9) from the keyboard: `step` entries on, clamped
    /// to the list (Home and End step past either end).
    pub fn pr_scale_step(&mut self, menu: i32, step: i32) {
        let Some((_, key)) = self.pr_scale() else { return };
        let (cur, n) = if menu == 8 { (key.root as i64, scale::ROOTS.len() as i64) } else { (key.kind as i64, scale::SCALES.len() as i64) };
        let i = (cur + step as i64).clamp(0, n - 1) as usize;
        if menu == 8 { self.pr_scale_root(i) } else { self.pr_scale_kind(i) }
    }

    pub fn pr_scale_kind(&mut self, kind: usize) {
        self.pr_set_scale(|s, key| s.key = Some(ScaleKey { root: key.root, kind: kind.min(scale::SCALES.len() - 1) as u8 }));
    }

    fn commit_notes(&mut self, clip: ClipId, mut notes: Vec<Note>, label: Option<&str>) -> bool {
        let Some(cur) = self.project().midi_clip(clip).map(|c| c.notes.clone()) else { return false };
        midi::sort_notes(&mut notes);
        if notes == cur {
            return false;
        }
        let cmd = Command::SetMidiNotes { clip, notes };
        let cmd = match label {
            Some(l) => Command::Batch { label: l.into(), commands: vec![cmd] },
            None => cmd,
        };
        self.execute(cmd)
    }

    fn hit_note(&self, notes: &[Note], v: &PrView, x: f32, y: f32) -> Option<(Note, bool)> {
        notes.iter().rev().find_map(|n| {
            let (nx, ny, nw, nh) = v.note_rect(n);
            (x >= nx && x < nx + nw && y >= ny && y < ny + nh).then(|| (*n, x >= nx + nw - 4.0f32.min(nw / 3.0)))
        })
    }

    /// Pointer in the piano roll texture (kind 0 down, 1 move, 2 up).
    pub fn pr_pointer(&mut self, kind: i32, x: f32, y: f32, shift: bool, alt: bool, command: bool) {
        if self.ac_clip().is_some() {
            self.ac_pointer(kind, x, y, shift, alt, command);
            return;
        }
        if self.shape_drawing() && self.pr.drag.is_none() && (kind != 0 || (x >= GRID_X && y >= GRID_Y)) {
            self.shape_draw(kind, x, y);
            return;
        }
        let Some(clip) = self.pr_clip().cloned() else { return };
        let v = self.pr_view();
        let g = self.pr_snap_ticks(&v);
        let sg = if self.snap.on { g } else { 1 };
        let beat = v.beat_at(x);
        let raw = (beat * PPQ as f64).round() as Ticks;
        let snap = |t: Ticks| ((t as f64 / sg as f64).round() as Ticks) * sg;
        let snap_floor = |t: Ticks| t.div_euclid(sg) * sg;
        self.ui_dirty = true;
        match kind {
            0 => {
                self.pr.focused = true;
                self.menu_close();
                let now = Instant::now();
                let double = self.pr.last_down.is_some_and(|(t, lx, ly)| now - t < DOUBLE_CLICK && (lx - x).abs() < 4.0 && (ly - y).abs() < 4.0);
                self.pr.last_down = Some((now, x, y));
                if y < GRID_Y {
                    if x < GRID_X {
                        return;
                    }
                    let (x0, x1) = (v.x_of(0.0), v.x_of(clip.length as f64 / PPQ as f64));
                    if self.pr_loop_press(&v, x, y, raw) {
                        return;
                    }
                    if y >= BAND_Y && (x - x1).abs() <= 6.0 {
                        self.pr.drag = Some(PrDrag::ClipEnd { end: clip.length });
                    } else if y >= BAND_Y && (x - x0).abs() <= 6.0 {
                        self.pr.drag = Some(PrDrag::ClipStart { start: 0 });
                    } else {
                        // Ruler / band click: move the playhead (timeline position).
                        self.seek_beats((clip.start + raw.max(0)) as f64 / PPQ as f64);
                    }
                    return;
                }
                if x < GRID_X {
                    // PLACEHOLDER: note preview on key click needs an engine audition API.
                    return;
                }
                let notes = clip.notes.clone();
                if let Some((n, edge)) = self.hit_note(&notes, &v, x, y) {
                    if shift {
                        if !self.pr.selected.remove(&n.id) {
                            self.pr.selected.insert(n.id);
                        } else {
                            return;
                        }
                    } else if !self.pr.selected.contains(&n.id) {
                        self.pr.selected = [n.id].into();
                    }
                    let ids = self.pr.selected.iter().copied().collect();
                    self.pr.drag = Some(PrDrag::Notes { resize: edge, dup: alt, ids, anchor_beat: beat, anchor_pitch: v.pitch_at(y), anchor: n, preview: notes });
                } else if self.pr.pen || double {
                    let pitch = v.pitch_at(y).clamp(0, 127) as u8;
                    let start = snap_floor(raw).max(0);
                    let id = clip.next_note_id();
                    let mut preview = notes;
                    preview.push(Note { id, pitch, start, length: g, velocity: self.pr.velocity });
                    self.pr.selected = [id].into();
                    self.pr.drag = Some(PrDrag::Draw { id, preview });
                } else {
                    let base = if shift { self.pr.selected.clone() } else { BTreeSet::new() };
                    self.pr.selected = base.clone();
                    self.pr.drag = Some(PrDrag::Marquee { x0: x, y0: y, x1: x, y1: y, base });
                }
            }
            1 => {
                let Some(drag) = self.pr.drag.clone() else { return };
                match drag {
                    PrDrag::Notes { resize, dup, ids, anchor_beat, anchor_pitch, anchor, .. } => {
                        let dt_raw = ((beat - anchor_beat) * PPQ as f64).round() as Ticks;
                        let orig = clip.notes.clone();
                        let moving: Vec<Note> = orig.iter().filter(|n| ids.contains(&n.id)).copied().collect();
                        let mut moved: Vec<Note> = if resize {
                            let end = anchor.start + anchor.length;
                            let dl = snap(end + dt_raw) - end;
                            moving.iter().map(|n| Note { length: (n.length + dl).max(g.min(n.length)), ..*n }).collect()
                        } else {
                            let mut dt = snap(anchor.start + dt_raw) - anchor.start;
                            let min_start = moving.iter().map(|n| n.start).min().unwrap_or(0);
                            dt = dt.max(-min_start);
                            let mut dp = v.pitch_at(y) - anchor_pitch;
                            let (lo, hi) = moving.iter().fold((127, 0), |(lo, hi), n| (lo.min(n.pitch as i32), hi.max(n.pitch as i32)));
                            dp = dp.clamp(-lo, 127 - hi);
                            moving.iter().map(|n| Note { start: n.start + dt, pitch: (n.pitch as i32 + dp) as u8, ..*n }).collect()
                        };
                        let changed = moved != moving;
                        let preview = if dup {
                            if changed {
                                renumber(&orig, &mut moved);
                                self.pr.selected = moved.iter().map(|n| n.id).collect();
                            } else {
                                self.pr.selected = ids.iter().copied().collect();
                                moved.clear();
                            }
                            orig.iter().copied().chain(moved).collect()
                        } else {
                            orig.iter().map(|n| moved.iter().find(|m| m.id == n.id).copied().unwrap_or(*n)).collect()
                        };
                        self.pr.drag = Some(PrDrag::Notes { resize, dup, ids, anchor_beat, anchor_pitch, anchor, preview });
                    }
                    PrDrag::Draw { id, mut preview } => {
                        if let Some(n) = preview.iter_mut().find(|n| n.id == id) {
                            let end = ((raw as f64 / sg as f64).ceil() as Ticks) * sg;
                            n.length = (end - n.start).max(g);
                        }
                        self.pr.drag = Some(PrDrag::Draw { id, preview });
                    }
                    PrDrag::Marquee { x0, y0, base, .. } => {
                        let (l, t, r, b) = (x0.min(x), y0.min(y), x0.max(x), y0.max(y));
                        let mut sel = base.clone();
                        for n in &clip.notes {
                            let (nx, ny, nw, nh) = v.note_rect(n);
                            if nx < r && nx + nw > l && ny < b && ny + nh > t {
                                sel.insert(n.id);
                            }
                        }
                        self.pr.selected = sel;
                        self.pr.drag = Some(PrDrag::Marquee { x0, y0, x1: x, y1: y, base });
                    }
                    PrDrag::ClipEnd { .. } => {
                        let end = snap(raw).max(g);
                        self.pr.drag = Some(PrDrag::ClipEnd { end });
                    }
                    PrDrag::ClipStart { .. } => {
                        let start = snap(raw).clamp(-clip.start, (clip.length - g).max(0));
                        self.pr.drag = Some(PrDrag::ClipStart { start });
                    }
                    PrDrag::Loop { .. } => self.pr_loop_move(raw),
                }
            }
            _ => match self.pr.drag.take() {
                Some(PrDrag::Notes { preview, .. } | PrDrag::Draw { preview, .. }) => {
                    self.commit_notes(clip.id, preview, None);
                }
                Some(PrDrag::ClipEnd { end }) if end != clip.length => {
                    self.execute(Command::ResizeMidiClip { clip: clip.id, start: clip.start, length: end });
                }
                Some(PrDrag::ClipStart { start }) if start != 0 => {
                    if let Some(cmd) = midi_left_resize(&clip, clip.start + start) {
                        self.execute(cmd);
                    }
                }
                Some(PrDrag::Loop { orig, cur, .. }) if orig != cur => {
                    self.execute(Command::SetMidiLoop { clip: clip.id, clip_loop: Some(cur), length: clip.length });
                }
                _ => {}
            },
        }
    }

    pub fn pr_scroll(&mut self, dx: f32, dy: f32) {
        let v = self.pr_view();
        self.pr.scroll = (v.scroll - dy).clamp(0.0, v.max_scroll());
        if self.pr.hzoom > 0.0 {
            self.pr.hstart = v.start - dx as f64 / v.ppb;
        }
        self.ui_dirty = true;
    }

    fn pr_target(&self) -> (MidiClip, Vec<Note>) {
        let clip = self.pr_clip().cloned().expect("piano roll clip");
        let sel: Vec<Note> = clip.notes.iter().filter(|n| self.pr.selected.contains(&n.id)).copied().collect();
        let target = if sel.is_empty() { clip.notes.clone() } else { sel };
        (clip, target)
    }

    fn replace_notes(clip: &MidiClip, changed: &[Note]) -> Vec<Note> {
        clip.notes.iter().map(|n| changed.iter().find(|c| c.id == n.id).copied().unwrap_or(*n)).collect()
    }

    /// Toolbar transforms (selection, or all notes when nothing is selected).
    pub fn pr_action(&mut self, action: &str) {
        self.ui_dirty = true;
        if action == "pen" {
            self.pr.pen = !self.pr.pen;
            return;
        }
        if action == "loop" {
            return self.pr_loop_toggle();
        }
        if action == "scale_highlight" {
            return self.pr_set_scale(|s, _| {
                s.highlight = !s.highlight;
                s.fold &= s.highlight;
            });
        }
        if action == "scale_fold" {
            return self.pr_set_scale(|s, _| s.fold = s.highlight && !s.fold);
        }
        if action == "velocity_lane" {
            self.set_status("Velocity lane: not implemented yet", 3);
            return;
        }
        if self.pr_clip().is_none() {
            return;
        }
        let (clip, target) = self.pr_target();
        if target.is_empty() {
            self.set_status("No notes to edit", 2);
            return;
        }
        let g = self.grid_ticks();
        let (label, notes) = match action {
            "quantize" => ("Quantize Notes", Self::replace_notes(&clip, &midi::quantize(&target, g, self.pr.quant_amount))),
            "humanize" => {
                self.pr.humanize_count += 1;
                let seed = clip.id.0.wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ self.pr.humanize_count;
                ("Humanize Notes", Self::replace_notes(&clip, &midi::humanize(&target, self.pr.humanize_amount, seed)))
            }
            "reverse" => ("Reverse Notes", Self::replace_notes(&clip, &midi::reverse(&target, clip.length))),
            "legato" => ("Legato", Self::replace_notes(&clip, &midi::legato(&target, clip.length))),
            "smart_quantize" => {
                let done = saine_core::smart_quantize::smart_quantize(&target, self.project().transport.bpm);
                let mut notes: Vec<Note> = clip.notes.iter().filter(|n| !target.iter().any(|t| t.id == n.id)).copied().collect();
                notes.extend(done.notes);
                if !self.commit_notes(clip.id, notes, Some("Smart Quantize")) {
                    self.set_status("Already smart quantized, nothing changed", 3);
                    return;
                }
                let kept: BTreeSet<u32> = self.project().midi_clip(clip.id).map(|c| c.notes.iter().map(|n| n.id).collect()).unwrap_or_default();
                self.pr.selected.retain(|id| kept.contains(id));
                self.set_status(format!("Smart Quantize: {} grid", done.grid), 2);
                return;
            }
            "double" => {
                let (d, len) = midi::double(&target, clip.length);
                let mut extra = d[target.len()..].to_vec();
                renumber(&clip.notes, &mut extra);
                let mut notes = clip.notes.clone();
                notes.extend(extra);
                midi::sort_notes(&mut notes);
                let mut cmds = vec![Command::SetMidiNotes { clip: clip.id, notes }, Command::ResizeMidiClip { clip: clip.id, start: clip.start, length: len }];
                if let Some(l) = clip.clip_loop.filter(|l| l.start == 0 && l.end == clip.length) {
                    cmds.push(Command::SetMidiLoop { clip: clip.id, clip_loop: Some(ClipLoop { end: len, ..l }), length: len });
                }
                self.execute(Command::Batch { label: "Double".into(), commands: cmds });
                return;
            }
            _ => return,
        };
        if !self.commit_notes(clip.id, notes, Some(label)) {
            self.set_status(format!("{label}: nothing to change"), 2);
        }
    }

    /// ↑/↓ (±1) and ⇧↑/⇧↓ (±12) on the selected notes.
    pub fn pr_transpose(&mut self, semis: i32) {
        if self.pr_clip().is_none() || self.pr.selected.is_empty() {
            return;
        }
        let (clip, target) = self.pr_target();
        match midi::transpose(&target, semis) {
            Some(t) => {
                self.commit_notes(clip.id, Self::replace_notes(&clip, &t), Some("Transpose Notes"));
            }
            None => self.set_status("Transpose: notes would leave the MIDI range", 3),
        }
    }

    /// Delete the selected notes. Returns false when there was nothing to delete.
    pub fn pr_delete_notes(&mut self) -> bool {
        let Some(clip) = self.pr_clip().cloned() else { return false };
        if self.pr.selected.is_empty() {
            return false;
        }
        let notes: Vec<Note> = clip.notes.iter().filter(|n| !self.pr.selected.contains(&n.id)).copied().collect();
        self.pr.selected.clear();
        self.commit_notes(clip.id, notes, Some("Delete Notes"));
        true
    }

    pub fn pr_select_all(&mut self) {
        if let Some(c) = self.pr_clip() {
            self.pr.selected = c.notes.iter().map(|n| n.id).collect();
            self.ui_dirty = true;
        }
    }

    /// Loop bar press at (x, y) in either clip editor: drag the whole range (inside it) or an
    /// edge (within 6 pt). \`raw\` is the clip tick under the pointer. False when it misses.
    pub(crate) fn pr_loop_press(&mut self, v: &PrView, x: f32, y: f32, raw: Ticks) -> bool {
        let Some(l) = self.pr_loop().filter(|l| l.on && y >= BAR_Y.0 && y < BAR_Y.1) else { return false };
        let (lx0, lx1) = (v.x_of(l.start as f64 / PPQ as f64), v.x_of(l.end as f64 / PPQ as f64));
        let edge = if (x - lx1).abs() <= 6.0 {
            Some(Some(true))
        } else if (x - lx0).abs() <= 6.0 {
            Some(Some(false))
        } else {
            (x > lx0 && x < lx1).then_some(None)
        };
        let Some(edge) = edge else { return false };
        self.pr.drag = Some(PrDrag::Loop { edge, orig: l, anchor: raw, cur: l });
        true
    }

    /// Loop bar drag to clip tick \`raw\` on the grid. An audio loop stays inside the clip's
    /// content (the source window), where its end may land off the grid.
    pub(crate) fn pr_loop_move(&mut self, raw: Ticks) {
        let Some(PrDrag::Loop { edge, orig, anchor, .. }) = self.pr.drag.clone() else { return };
        let g = self.grid_ticks();
        let snap = |t: Ticks| ((t as f64 / g as f64).round() as Ticks) * g;
        let max_end = self.ac_clip().map_or(Ticks::MAX, |c| self.project().content_length_ticks(c).max(orig.end));
        let cur = match edge {
            None => {
                let d = snap(raw - anchor).clamp(-orig.start, max_end - orig.end);
                ClipLoop { start: orig.start + d, end: orig.end + d, ..orig }
            }
            Some(false) => ClipLoop { start: snap(raw).clamp(0, (orig.end - g).max(0)), ..orig },
            Some(true) => ClipLoop { end: snap(raw).max(orig.start + g).min(max_end), ..orig },
        };
        self.pr.drag = Some(PrDrag::Loop { edge, orig, anchor, cur });
    }

    /// Footer value controls: phase 0 press, 1 drag (dy = points moved since press, down positive), 2 release.
    pub fn pr_value(&mut self, field: i32, phase: i32, dy: f32) {
        let Some(field) = Field::from_i32(field) else { return };
        self.ui_dirty = true;
        if field == Field::Grid {
            if phase == 2 {
                self.clip_menu_open(3);
            }
            return;
        }
        if let Some(c) = self.ac_clip().cloned() {
            // Audio clip editor: the loop's Position, a beat per 8 pt, kept inside the content.
            let Some(l) = c.clip_loop.filter(|l| l.on && field == Field::LoopPosition) else { return };
            let last = (self.project().content_length_ticks(&c) - (l.end - l.start)).max(0);
            match phase {
                0 => self.pr.value_drag = Some(ValueDrag { orig: l.start as f64, preview: None, length: None, position: None }),
                1 => {
                    if let Some(d) = self.pr.value_drag.as_mut() {
                        let beats = (d.orig / PPQ as f64 - dy as f64 / 8.0).round().max(0.0);
                        d.position = Some(((beats * PPQ as f64) as Ticks).min(last));
                    }
                }
                _ => {
                    if let Some(p) = self.pr.value_drag.take().and_then(|d| d.position).filter(|&p| p != l.start) {
                        self.execute(Command::SetClipLoop { clip: c.id, clip_loop: Some(saine_core::model::AudioLoop { start: p, end: p + l.end - l.start, ..l }) });
                    }
                }
            }
            return;
        }
        let Some(clip) = self.pr_clip().cloned() else { return };
        match phase {
            0 => {
                let orig = match field {
                    Field::QuantizeAmount => self.pr.quant_amount,
                    Field::HumanizeAmount => self.pr.humanize_amount,
                    Field::Velocity => self.pr.velocity as f64,
                    Field::Length => self.pr_loop().filter(|l| l.on).map_or(clip.length, |l| l.len()) as f64,
                    Field::LoopPosition => match self.pr_loop().filter(|l| l.on) {
                        Some(l) => l.start as f64,
                        None => return,
                    },
                    Field::Grid => 0.0,
                };
                self.pr.value_drag = Some(ValueDrag { orig, preview: None, length: None, position: None });
            }
            1 => {
                let Some(mut d) = self.pr.value_drag.clone() else { return };
                match field {
                    Field::QuantizeAmount => self.pr.quant_amount = ((d.orig * 100.0 - dy as f64 / 2.0) / 5.0).round().clamp(0.0, 20.0) * 0.05,
                    Field::HumanizeAmount => self.pr.humanize_amount = ((d.orig * 100.0 - dy as f64 / 2.0) / 5.0).round().clamp(0.0, 20.0) * 0.05,
                    Field::Velocity => {
                        let vel = (d.orig - dy as f64 / 2.0).round().clamp(1.0, 127.0) as u8;
                        self.pr.velocity = vel;
                        if !self.pr.selected.is_empty() {
                            d.preview = Some(clip.notes.iter().map(|n| if self.pr.selected.contains(&n.id) { Note { velocity: vel, ..*n } } else { *n }).collect());
                        }
                    }
                    Field::Length => {
                        let beats = (d.orig / PPQ as f64 - dy as f64 / 8.0).round().max(1.0);
                        d.length = Some((beats * PPQ as f64) as Ticks);
                    }
                    Field::LoopPosition => {
                        let beats = (d.orig / PPQ as f64 - dy as f64 / 8.0).round().max(0.0);
                        d.position = Some((beats * PPQ as f64) as Ticks);
                    }
                    Field::Grid => {}
                }
                self.pr.value_drag = Some(d);
            }
            _ => {
                if let Some(d) = self.pr.value_drag.take() {
                    if let Some(p) = d.preview {
                        self.commit_notes(clip.id, p, Some("Note Velocity"));
                    }
                    match self.pr_loop().filter(|l| l.on) {
                        Some(l) => {
                            let start = d.position.unwrap_or(l.start);
                            let moved = ClipLoop { start, end: start + d.length.unwrap_or(l.len()), ..l };
                            if moved != l {
                                self.execute(Command::SetMidiLoop { clip: clip.id, clip_loop: Some(moved), length: clip.length });
                            }
                        }
                        None => {
                            if let Some(len) = d.length
                                && len != clip.length
                            {
                                self.execute(Command::ResizeMidiClip { clip: clip.id, start: clip.start, length: len });
                            }
                        }
                    }
                }
            }
        }
    }

    pub fn pr_animating(&self) -> bool {
        self.pr.drag.is_some() || self.pr.value_drag.is_some()
    }
}

/// Left-edge resize of a MIDI clip to `new_start` (timeline ticks): the clip end
/// stays put and notes keep their timeline positions, so they shift by the delta
/// (notes that would start before the new start are clipped to it). One `Batch`.
pub fn midi_left_resize(clip: &MidiClip, new_start: Ticks) -> Option<Command> {
    let new_start = new_start.max(0);
    let end = clip.end();
    if new_start >= end || new_start == clip.start {
        return None;
    }
    let delta = new_start - clip.start;
    let notes: Vec<Note> = clip
        .notes
        .iter()
        .filter_map(|n| {
            let s = n.start - delta;
            let e = s + n.length;
            (e > 0).then(|| Note { start: s.max(0), length: e - s.max(0), ..*n })
        })
        .collect();
    let mut commands = vec![Command::ResizeMidiClip { clip: clip.id, start: new_start, length: end - new_start }, Command::SetMidiNotes { clip: clip.id, notes }];
    if let Some(l) = clip.clip_loop {
        let start = (l.start - delta).max(0);
        let moved = ClipLoop { start, end: (l.end - delta).max(start + 1), span: 0, ..l };
        // A trim past (or nearly past) the loop end loops the whole clip, as the Loop switch does.
        let moved = if moved.len() <= PPQ / 4 { ClipLoop { on: l.on, ..ClipLoop::whole(end - new_start) } } else { moved };
        commands.push(Command::SetMidiLoop { clip: clip.id, clip_loop: Some(moved), length: end - new_start });
    }
    Some(Command::Batch { label: "Resize Clip".into(), commands })
}

#[cfg(test)]
mod tests {
    #[test]
    fn zoom_slider_ranges() {
        assert_eq!(zoom_ppb(100.0, 0.0), 100.0);
        assert!((zoom_ppb(100.0, 1.0) - PPQ as f64 * REF).abs() < 1e-9);
        assert!((zoom_ppb(100.0, 0.5) - (100.0 * PPQ as f64 * REF).sqrt()).abs() < 1e-9);
        assert_eq!(zoom_ppb(2000.0, 0.7), 2000.0);
        let v = PrView::fit((1174.5, 281.0), 8.0, 0.0);
        let (lo, hi) = row_h_range(&v);
        assert!((lo - 3.0 * 0.85).abs() < 1e-4 && (hi - 32.0 * 0.85).abs() < 1e-4);
        let folded = PrView { rows: Rows::only(&[64, 62, 60]), ..v };
        let (lo, hi) = row_h_range(&folded);
        assert!((lo - (281.0 - GRID_Y) / 3.0).abs() < 1e-3 && hi == lo);
    }

    use super::*;

    #[test]
    fn sub_lines_double_until_16_reference_px_apart_and_stop_at_the_beat() {
        assert_eq!(sub_step(0.25, 1.0, 100.0), Some(0.25));
        assert_eq!(sub_step(0.25, 1.0, 40.0), Some(0.5));
        assert_eq!(sub_step(0.25, 1.0, 20.0), None, "doubling reaches the beat: beat and bar lines only");
        assert_eq!(sub_step(0.125, 0.5, 100.0), Some(0.25), "6/8: an eighth-note beat");
        assert_eq!(sub_step(1.0 / 3.0, 1.0, 30.0), None, "a third triples to the beat, as a beat is not a multiple of 2/3");
        assert_eq!(sub_step(1.0 / 3.0, 1.0, 100.0), Some(1.0 / 3.0));
        assert_eq!(sub_step(0.0, 1.0, 100.0), None);
    }

    #[test]
    fn view_fits_clip_and_maps_pitches() {
        let v = PrView::fit((1174.5, 281.0), 4.0, DEFAULT_SCROLL);
        let pad = (1174.5 - GRID_X) * (FIT_PAD / (1.0 + 2.0 * FIT_PAD)) as f32;
        assert!((v.x_of(0.0) - (GRID_X + pad)).abs() < 1e-3);
        assert!((v.x_of(4.0) - (1174.5 - pad)).abs() < 1e-3);
        assert!((v.y_of(60) - 106.5).abs() < 1e-3);
        assert_eq!(v.pitch_at(106.6), 60);
        assert_eq!(v.pitch_at(106.4), 61);
    }

    #[test]
    fn positions_format_like_the_reference() {
        assert_eq!(fmt_pos(0, 3840), "1.1.0");
        assert_eq!(fmt_pos(3840, 3840), "2.1.0");
        assert_eq!(fmt_len(3840, 3840), "1.0.0");
        assert_eq!(fmt_pos(960 + 240, 3840), "1.2.1");
    }

    #[test]
    fn left_resize_keeps_note_timeline_positions() {
        let clip = MidiClip { id: ClipId(5), name: "MIDI".into(), start: 3840, length: 3840, notes: vec![Note { id: 1, pitch: 60, start: 960, length: 480, velocity: 100 }], pattern: None, disabled: false, composer: None, scale: None, clip_loop: None };
        let Some(Command::Batch { commands, .. }) = midi_left_resize(&clip, 3840 + 480) else { panic!() };
        assert_eq!(commands[0], Command::ResizeMidiClip { clip: ClipId(5), start: 4320, length: 3360 });
        assert_eq!(commands[1], Command::SetMidiNotes { clip: ClipId(5), notes: vec![Note { id: 1, pitch: 60, start: 480, length: 480, velocity: 100 }] });
    }

    #[test]
    fn left_resize_moves_the_loop_and_falls_back_to_the_whole_clip() {
        let looped = |start, end| MidiClip { clip_loop: Some(ClipLoop { on: true, start, end, span: 0 }), ..MidiClip::empty(3840, 3840) };
        let loop_of = |cmd: Option<Command>| match cmd {
            Some(Command::Batch { commands, .. }) => commands.into_iter().find_map(|c| match c {
                Command::SetMidiLoop { clip_loop, length, .. } => Some((clip_loop, length)),
                _ => None,
            }),
            _ => None,
        };
        assert_eq!(loop_of(midi_left_resize(&looped(960, 2880), 3840 + 480)), Some((Some(ClipLoop { on: true, start: 480, end: 2400, span: 0 }), 3360)));
        assert_eq!(loop_of(midi_left_resize(&looped(0, 960), 3840 + 2000)), Some((Some(ClipLoop::whole(1840)), 1840)));
    }
}
