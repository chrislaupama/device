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
//! (255, 54.5) of the 390 pt bottom panel, i.e. window (260, 479.5) at the
//! reference size (measured from `34-midi-clip-selected-pianoroll.png`).

use crate::controller::{BottomTab, Controller};
use crate::gpu::{DrawList, RectInst};
use crate::timeline::{Px, rgb};
use saine_core::midi::{self, MidiClip, Note};
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
/// Texture origin inside the bottom panel, and the panel height while the Clip tab is open.
pub const PANEL_ORIGIN: (f32, f32) = (255.0, 54.5);
pub const CLIP_PANEL_H: f32 = 390.0;
pub const FOOTER_H: f32 = 54.5;
/// C4 row top at y 106.5 (window 586) in the reference.
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
}

impl PrView {
    /// Horizontal zoom fits the clip with `FIT_PAD` on both sides (reference behaviour).
    pub fn fit(size: (f32, f32), clip_beats: f64, scroll: f32) -> Self {
        Self::fit_from(GRID_X, size, clip_beats, scroll)
    }
    pub fn fit_from(gx: f32, size: (f32, f32), clip_beats: f64, scroll: f32) -> Self {
        let beats = clip_beats.max(0.25);
        let ppb = (size.0 - gx).max(40.0) as f64 / (beats * (1.0 + 2.0 * FIT_PAD));
        PrView { gx, w: size.0, h: size.1, ppb, start: -FIT_PAD * beats, scroll }
    }
    pub fn x_of(&self, beat: f64) -> f32 {
        self.gx + ((beat - self.start) * self.ppb) as f32
    }
    pub fn beat_at(&self, x: f32) -> f64 {
        self.start + (x - self.gx) as f64 / self.ppb
    }
    pub fn y_of(&self, pitch: i32) -> f32 {
        GRID_Y + (127 - pitch) as f32 * ROW_H - self.scroll
    }
    pub fn pitch_at(&self, y: f32) -> i32 {
        127 - ((y - GRID_Y + self.scroll) / ROW_H).floor() as i32
    }
    pub fn max_scroll(&self) -> f32 {
        (128.0 * ROW_H - (self.h - GRID_Y)).max(0.0)
    }
    pub fn note_rect(&self, n: &Note) -> (f32, f32, f32, f32) {
        let x0 = self.x_of(n.start as f64 / PPQ as f64);
        let x1 = self.x_of((n.start + n.length) as f64 / PPQ as f64);
        (x0, self.y_of(n.pitch as i32), (x1 - x0).max(2.0), ROW_H)
    }
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
}

/// Footer value fields (drag vertically; Grid cycles on click).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Field {
    Grid = 0,
    QuantizeAmount = 1,
    HumanizeAmount = 2,
    Velocity = 3,
    Length = 4,
}

impl Field {
    fn from_i32(i: i32) -> Option<Field> {
        Some(match i {
            0 => Field::Grid,
            1 => Field::QuantizeAmount,
            2 => Field::HumanizeAmount,
            3 => Field::Velocity,
            4 => Field::Length,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone)]
pub struct ValueDrag {
    orig: f64,
    /// Velocity edits of the selected notes (committed on release).
    preview: Option<Vec<Note>>,
    /// Length edit (ticks), committed on release.
    length: Option<Ticks>,
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
    pub marquee: Option<(f32, f32, f32, f32)>,
    /// Audio clip editor: no keyboard or pitch rows, gain dots on the clip handles.
    pub audio: bool,
    /// Warped audio clips: warp markers and transients (clip-relative beats).
    pub warp_markers: Vec<f64>,
    pub transients: Vec<f64>,
    /// Audio clip editor: the clip's waveform, one lane per channel (`Controller::clip_lanes`).
    pub wave: Option<(Vec<crate::peaks::Slot>, Vec<crate::peaks::WaveSeg>)>,
}

fn rgba(c: [f32; 3], a: f32) -> [f32; 4] {
    [c[0], c[1], c[2], a]
}
fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}
const WHITE: [f32; 3] = [1.0, 1.0, 1.0];
const BLACK: [f32; 3] = [0.0, 0.0, 0.0];

fn vlines(out: &mut Vec<RectInst>, sc: &PrScene, px: &Px, y0: f32, y1: f32, x_from: f32) {
    let v = &sc.view;
    // Finest subdivision: 1/32 notes while they are >= 12 pt apart.
    let mut step = 0.125;
    while step * v.ppb < 12.0 {
        step *= 2.0;
    }
    let first = (v.beat_at(x_from) / step).floor() as i64;
    let mut i = first;
    loop {
        let beat = i as f64 * step;
        let x = v.x_of(beat);
        if x > v.w {
            break;
        }
        if x >= x_from {
            let on_bar = (beat / sc.beats_per_bar).fract().abs() < 1e-9;
            let on_beat = beat.fract().abs() < 1e-9;
            let a = if on_bar { 0.06 } else if on_beat { 0.038 } else { 0.024 };
            out.push(px.rect(x - 0.5, y0, 1.0, y1 - y0, rgba(WHITE, a)));
        }
        i += 1;
    }
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
    let top = v.pitch_at(GRID_Y).min(127);
    let bottom = v.pitch_at(h - 0.01).max(0);
    if sc.audio {
        b.push(px.rect(gx, GRID_Y, w - gx, h - GRID_Y, rgba(rgb(0x141518), 1.0)));
    } else {
        for p in (bottom..=top).rev() {
            let y = v.y_of(p);
            let c = if is_black(p) { 0x151619 } else { 0x191a1d };
            b.push(px.rect(gx, y, w - gx, ROW_H, rgba(rgb(c), 1.0)));
            if p.rem_euclid(12) == 0 {
                b.push(px.rect(gx, y + ROW_H - 1.0, w - gx, 1.0, rgba(rgb(0x252629), 1.0)));
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

    // ---- keyboard (covers notes scrolled left of the grid) ----
    if !sc.audio {
        b.push(px.rect(0.0, GRID_Y, gx, h - GRID_Y, rgba(rgb(0x1a1b1e), 1.0)));
        b.push(px.rect(0.5, GRID_Y, 49.5, h - GRID_Y, rgba(WHITE, 1.0)));
        for p in (bottom..=top).rev() {
            let y = v.y_of(p);
            let line = rgba(rgb(0x555555), 1.0);
            if is_black(p) {
                b.push(px.rect(30.5, y + ROW_H / 2.0 - 1.0, 19.5, 1.0, line));
                b.push(px.rect(0.5, y, 30.0, ROW_H, rgba(rgb(0x1a1a1a), 1.0)));
                b.push(px.rect(0.5, y, 30.0, 1.0, rgba(rgb(0x3c3c3c), 1.0)));
                b.push(px.rect(0.5, y + ROW_H - 1.0, 30.0, 1.0, rgba(rgb(0x0d0d0d), 1.0)));
            } else if matches!(p.rem_euclid(12), 0 | 5) {
                b.push(px.rect(0.5, y + ROW_H - 1.0, 49.5, 1.0, line));
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
        // beat ticks in the ruler
        let mut beat = sc.range.0.max(v.beat_at(gx)).ceil();
        while v.x_of(beat) <= w {
            if beat >= sc.range.0 {
                o.push(px.rect(v.x_of(beat), 0.0, 0.5, 13.5, rgba(rgb(0x303236), 1.0)));
            }
            beat += 1.0;
        }
    }
    let mut tmp = Vec::new();
    vlines(&mut tmp, sc, &px, BAND_Y, GRID_Y, gx);
    o.extend(tmp);
    for (y, hh) in [(27.0, 0.5), (40.5, 1.0), (54.0, 0.5)] {
        o.push(px.rect(gx, y, w - gx, hh, rgba(rgb(0x292b2e), 1.0)));
    }
    let bar = mix(sc.color, rgb(0x86a6ff), 0.5);
    if cx1 > gx && cx0 < w {
        let x0 = cx0.max(gx);
        o.push(px.rect(x0, BAR_Y.0, cx1.min(w) - x0, BAR_Y.1 - BAR_Y.0, rgba(bar, 1.0)));
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
            self.pr.seq = crate::stepseq::SeqState::opened(self.pr_clip().and_then(|c| c.pattern.as_ref()));
            // A clip with notes opens centred on its pitch range (empty clips: C4 as in the reference).
            if let Some(c) = self.pr_clip()
                && !c.notes.is_empty()
            {
                let lo = c.notes.iter().map(|n| n.pitch).min().unwrap_or(60) as f32;
                let hi = c.notes.iter().map(|n| n.pitch).max().unwrap_or(60) as f32;
                let mid = (lo + hi) / 2.0;
                self.pr.scroll = (127.0 - mid) * ROW_H + ROW_H / 2.0 - (self.pr.size.1 - GRID_Y) / 2.0;
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

    pub fn pr_view(&self) -> PrView {
        // Gestures keep the view, except Transpose: an unwarped clip's length follows it.
        if let Some(view) = self.pr.ac_drag.as_ref().filter(|d| !matches!(d.kind, crate::warpedit::AcDragKind::Transpose { .. })).map(|d| d.view) {
            return view;
        }
        if let Some(c) = self.ac_clip() {
            let len = self.project().clip_length_ticks(c);
            return PrView::fit_from(0.0, self.pr.size, len as f64 / PPQ as f64, 0.0);
        }
        let len = self.pr_clip().map(|c| c.length).unwrap_or(self.project().ticks_per_bar());
        let mut v = PrView::fit(self.pr.size, len as f64 / PPQ as f64, self.pr.scroll);
        v.scroll = v.scroll.clamp(0.0, v.max_scroll());
        v
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
        let len = self.pr.value_drag.as_ref().and_then(|d| d.length).unwrap_or(len);
        let b = |t: Ticks| t as f64 / PPQ as f64;
        match self.pr.drag {
            Some(PrDrag::ClipStart { start }) => (b(start), b(len)),
            Some(PrDrag::ClipEnd { end }) => (0.0, b(end)),
            _ => (0.0, b(len)),
        }
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
                marquee: None,
                audio: true,
                warp_markers: self.ac_marker_beats(),
                transients: self.ac_transient_beats(),
                wave: self.clip_lanes(c),
            });
        }
        let clip = self.pr_clip()?;
        let color = p.find_midi_clip(clip.id).map(|(t, _)| crate::controller::track_color(p.tracks[t].color)).unwrap_or(WHITE);
        let marquee = match self.pr.drag {
            Some(PrDrag::Marquee { x0, y0, x1, y1, .. }) => Some((x0, y0, x1, y1)),
            _ => None,
        };
        Some(PrScene {
            view: self.pr_view(),
            scale,
            range: self.pr_range(),
            notes,
            selected: &self.pr.selected,
            color,
            playhead: (self.playhead_ticks() - clip.start) as f64 / PPQ as f64,
            beats_per_bar: p.ticks_per_bar() as f64 / PPQ as f64,
            marquee,
            audio: false,
            warp_markers: Vec::new(),
            transients: Vec::new(),
            wave: None,
        })
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
        let Some(clip) = self.pr_clip().cloned() else { return };
        let v = self.pr_view();
        let g = self.grid_ticks();
        let beat = v.beat_at(x);
        let raw = (beat * PPQ as f64).round() as Ticks;
        let snap = |t: Ticks| ((t as f64 / g as f64).round() as Ticks) * g;
        let snap_floor = |t: Ticks| t.div_euclid(g) * g;
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
                    if y >= BAR_Y.0 && (x - x1).abs() <= 6.0 {
                        self.pr.drag = Some(PrDrag::ClipEnd { end: clip.length });
                    } else if y >= BAR_Y.0 && (x - x0).abs() <= 6.0 {
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
                            let end = ((raw as f64 / g as f64).ceil() as Ticks) * g;
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
                        let start = snap(raw).clamp(-clip.start, clip.length - g);
                        self.pr.drag = Some(PrDrag::ClipStart { start });
                    }
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
                _ => {}
            },
        }
    }

    pub fn pr_scroll(&mut self, dy: f32) {
        let v = self.pr_view();
        self.pr.scroll = (v.scroll - dy).clamp(0.0, v.max_scroll());
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
        if action == "smart_quantize" || action == "velocity_lane" {
            self.set_status(format!("{}: not implemented yet", if action == "smart_quantize" { "Smart Quantize" } else { "Velocity lane" }), 3);
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
            "double" => {
                let (d, len) = midi::double(&target, clip.length);
                let mut extra = d[target.len()..].to_vec();
                renumber(&clip.notes, &mut extra);
                let mut notes = clip.notes.clone();
                notes.extend(extra);
                midi::sort_notes(&mut notes);
                let cmds = vec![Command::SetMidiNotes { clip: clip.id, notes }, Command::ResizeMidiClip { clip: clip.id, start: clip.start, length: len }];
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
        let Some(clip) = self.pr_clip().cloned() else { return };
        match phase {
            0 => {
                let orig = match field {
                    Field::QuantizeAmount => self.pr.quant_amount,
                    Field::HumanizeAmount => self.pr.humanize_amount,
                    Field::Velocity => self.pr.velocity as f64,
                    Field::Length => clip.length as f64,
                    Field::Grid => 0.0,
                };
                self.pr.value_drag = Some(ValueDrag { orig, preview: None, length: None });
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
                    Field::Grid => {}
                }
                self.pr.value_drag = Some(d);
            }
            _ => {
                if let Some(d) = self.pr.value_drag.take() {
                    if let Some(p) = d.preview {
                        self.commit_notes(clip.id, p, Some("Note Velocity"));
                    }
                    if let Some(len) = d.length
                        && len != clip.length
                    {
                        self.execute(Command::ResizeMidiClip { clip: clip.id, start: clip.start, length: len });
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
    Some(Command::Batch {
        label: "Resize Clip".into(),
        commands: vec![Command::ResizeMidiClip { clip: clip.id, start: new_start, length: end - new_start }, Command::SetMidiNotes { clip: clip.id, notes }],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let clip = MidiClip { id: ClipId(5), name: "MIDI".into(), start: 3840, length: 3840, notes: vec![Note { id: 1, pitch: 60, start: 960, length: 480, velocity: 100 }], pattern: None, composer: None };
        let Some(Command::Batch { commands, .. }) = midi_left_resize(&clip, 3840 + 480) else { panic!() };
        assert_eq!(commands[0], Command::ResizeMidiClip { clip: ClipId(5), start: 4320, length: 3360 });
        assert_eq!(commands[1], Command::SetMidiNotes { clip: ClipId(5), notes: vec![Note { id: 1, pitch: 60, start: 480, length: 480, velocity: 100 }] });
    }
}
