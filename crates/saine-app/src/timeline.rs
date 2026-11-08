//! Timeline geometry: view transform, hit-testing and the per-frame draw list.
//!
//! Coordinates here are *lane-local logical points*: origin at the top-left of
//! the lanes surface (window x 453, y 105.5 at the reference size), i.e. the
//! ruler is at the top, track 0 starts at `RULER_H`. Measured from
//! reference/screenshots/01-main-default.png (see FIDELITY.md).

use crate::gpu::{DrawList, RectInst, WaveInst};
use crate::peaks::Slot;

pub const RULER_H: f32 = 39.0;
pub const TRACK_PITCH: f32 = 74.5;
pub const TRACK_H: f32 = 74.0;
pub const MASTER_H: f32 = 30.5;
/// Master row height while automation lanes are shown (60-automation-lanes.png: separator at
/// window y 439.5, lanes bottom 504).
pub const MASTER_H_AUTO: f32 = 64.5;
pub const CLIP_HEADER: f32 = 17.0;
pub const EDGE_GRAB: f32 = 5.0;
/// Default zoom: 51.03 pt per 4/4 bar (measured: bar 20 line at x 1422.5, bar 1 at 453).
pub const DEFAULT_PX_PER_BEAT: f64 = 969.5 / 76.0;

#[derive(Debug, Clone)]
pub struct View {
    pub start_beats: f64,
    pub px_per_beat: f64,
    pub scroll_y: f64,
    /// Extra height below each track (expanded automation lanes, src/automation.rs), by track index.
    pub extra: Vec<f32>,
    /// Height of the master row pinned to the bottom (taller while automation is shown).
    pub master_h: f32,
}

impl Default for View {
    fn default() -> Self {
        View { start_beats: 0.0, px_per_beat: DEFAULT_PX_PER_BEAT, scroll_y: 0.0, extra: Vec::new(), master_h: MASTER_H }
    }
}

impl View {
    pub fn x_of(&self, beat: f64) -> f32 {
        ((beat - self.start_beats) * self.px_per_beat) as f32
    }
    pub fn beat_at(&self, x: f32) -> f64 {
        self.start_beats + x as f64 / self.px_per_beat
    }
    /// Content offset of track `i` (sum of the rows above it, without the ruler or scroll).
    pub fn track_offset(&self, i: usize) -> f32 {
        i as f32 * TRACK_PITCH + self.extra.iter().take(i).sum::<f32>()
    }
    /// Total height of track `i` including its expanded automation rows.
    pub fn track_span(&self, i: usize) -> f32 {
        TRACK_PITCH + self.extra.get(i).copied().unwrap_or(0.0)
    }
    pub fn track_top(&self, i: usize) -> f32 {
        RULER_H + self.track_offset(i) - self.scroll_y as f32
    }
    /// Track whose row (or expanded automation rows) contains `y`.
    pub fn track_at(&self, y: f32) -> Option<usize> {
        let mut top = RULER_H - self.scroll_y as f32;
        if y < top {
            return None;
        }
        for (i, e) in self.extra.iter().enumerate() {
            let h = TRACK_PITCH + e;
            if y < top + h {
                return Some(i);
            }
            top += h;
        }
        Some(self.extra.len() + ((y - top) / TRACK_PITCH).floor() as usize)
    }
    pub fn zoom_at(&mut self, factor: f64, x: f32) {
        let anchor = self.beat_at(x);
        self.px_per_beat = (self.px_per_beat * factor).clamp(0.5, 2000.0);
        self.start_beats = anchor - x as f64 / self.px_per_beat;
        self.clamp();
    }
    pub fn clamp(&mut self) {
        self.start_beats = self.start_beats.max(0.0);
    }
    pub fn clamp_scroll(&mut self, tracks: usize, lanes_h: f32) {
        let content = self.track_offset(tracks) + 40.0;
        let visible = lanes_h - RULER_H - self.master_h;
        self.scroll_y = self.scroll_y.clamp(0.0, (content - visible).max(0.0) as f64);
    }
}

#[derive(Debug, Clone, Copy)]
pub struct TrackVis {
    pub selected: bool,
}

#[derive(Debug, Clone)]
pub struct ClipVis {
    pub track_index: usize,
    pub start_beats: f64,
    pub len_beats: f64,
    pub color: [f32; 3],
    pub selected: bool,
    pub name: String,
    /// Waveform source: pyramid slot, first frame (engine rate), frame count.
    pub wave: Option<(Slot, f64, f64)>,
    /// MIDI clip: mini note view as (relative start 0..1, relative length, relative
    /// pitch 0 = top .. 1 = bottom). `None` for audio clips.
    pub midi: Option<Vec<[f32; 3]>>,
    /// Take being recorded: live min/max peaks from `Engine::recording_peaks`
    /// (one pair every `.1` logical points from the clip start), drawn with the
    /// recording tint.
    pub live: Option<(Vec<[f32; 2]>, f32)>,
}

impl ClipVis {
    pub fn rect(&self, v: &View) -> (f32, f32, f32, f32) {
        let x = v.x_of(self.start_beats);
        let w = (self.len_beats * v.px_per_beat) as f32;
        (x, v.track_top(self.track_index) + 0.5, w.max(1.0), TRACK_H - 1.0)
    }
}

pub struct Scene<'a> {
    pub view: View,
    pub size: (f32, f32), // logical
    pub scale: f32,
    pub tracks: &'a [TrackVis],
    pub clips: &'a [ClipVis],
    /// Growing clips of takes in progress (not hit-testable).
    pub live: &'a [ClipVis],
    pub playhead_beats: f64,
    pub loop_beats: (f64, f64),
    pub beats_per_bar: f64,
    pub frames_per_beat: f64,
    /// Automation overlay (src/automation.rs) while lanes are shown.
    pub auto: Option<&'a crate::automation::AutoScene>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Zone {
    Body,
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Hit {
    Ruler,
    Master,
    Clip { index: usize, zone: Zone },
    Track(usize),
    Empty,
}

pub fn hit_test(v: &View, size: (f32, f32), tracks: usize, clips: &[ClipVis], x: f32, y: f32) -> Hit {
    if y < RULER_H {
        return Hit::Ruler;
    }
    if y >= size.1 - v.master_h {
        return Hit::Master;
    }
    for (i, c) in clips.iter().enumerate().rev() {
        let (cx, cy, cw, ch) = c.rect(v);
        if x >= cx && x < cx + cw && y >= cy && y < cy + ch {
            let edge = EDGE_GRAB.min(cw / 3.0);
            let zone = if x < cx + edge {
                Zone::Left
            } else if x >= cx + cw - edge {
                Zone::Right
            } else {
                Zone::Body
            };
            return Hit::Clip { index: i, zone };
        }
    }
    match v.track_at(y) {
        Some(t) if t < tracks && y < v.track_top(t) + TRACK_H => Hit::Track(t),
        _ => Hit::Empty,
    }
}

pub fn rgb(hex: u32) -> [f32; 3] {
    [((hex >> 16) & 0xff) as f32 / 255.0, ((hex >> 8) & 0xff) as f32 / 255.0, (hex & 0xff) as f32 / 255.0]
}

fn rgba(c: [f32; 3], a: f32) -> [f32; 4] {
    [c[0], c[1], c[2], a]
}

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

const LANE: [u32; 2] = [0x131417, 0x16171a];
const LANE_SEL: [u32; 2] = [0x1a1b1e, 0x1d1e21];
const WHITE: [f32; 3] = [1.0, 1.0, 1.0];
const BLACK: [f32; 3] = [0.0, 0.0, 0.0];

pub struct Px {
    pub s: f32,
}
impl Px {
    /// Logical rect -> physical rect, snapped to whole pixels.
    pub fn r(&self, x: f32, y: f32, w: f32, h: f32) -> [f32; 4] {
        let x0 = (x * self.s).round();
        let y0 = (y * self.s).round();
        let x1 = ((x + w) * self.s).round();
        let y1 = ((y + h) * self.s).round();
        [x0, y0, (x1 - x0).max(1.0), (y1 - y0).max(1.0)]
    }
    pub fn rect(&self, x: f32, y: f32, w: f32, h: f32, color: [f32; 4]) -> RectInst {
        RectInst { rect: self.r(x, y, w, h), color, params: [0.0; 4] }
    }
    pub fn rounded(&self, x: f32, y: f32, w: f32, h: f32, radius: f32, color: [f32; 4]) -> RectInst {
        RectInst { rect: [x * self.s, y * self.s, w * self.s, h * self.s], color, params: [radius * self.s, 0.0, 0.0, 0.0] }
    }
}

/// Lane background (alternating 4-bar bands) and grid for the band y0..y1.
fn bands_and_grid(out: &mut Vec<RectInst>, sc: &Scene, px: &Px, y0: f32, y1: f32, colors: [u32; 2]) {
    let v = &sc.view;
    let w = sc.size.0;
    let band = sc.beats_per_bar * 4.0;
    let first = (v.start_beats / band).floor() as i64;
    let mut k = first;
    loop {
        let x0 = v.x_of(k as f64 * band);
        if x0 > w {
            break;
        }
        let x1 = v.x_of((k + 1) as f64 * band).min(w);
        out.push(px.rect(x0.max(0.0), y0, x1 - x0.max(0.0), y1 - y0, rgba(rgb(colors[(k.rem_euclid(2)) as usize]), 1.0)));
        k += 1;
    }
    // Grid: half-bar lines at the default zoom (reference), coarser when zoomed out.
    let mut step = sc.beats_per_bar / 2.0;
    while step * v.px_per_beat < 10.0 {
        step *= 2.0;
    }
    while step * v.px_per_beat > 80.0 && step > 0.25 {
        step /= 2.0;
    }
    let first = (v.start_beats / step).floor() as i64;
    let lw = 1.0 / px.s; // one physical pixel (0.5 pt on Retina, as measured)
    let mut i = first;
    loop {
        let beat = i as f64 * step;
        let x = v.x_of(beat);
        if x > w {
            break;
        }
        if x >= -1.0 {
            let on_bar = (beat / sc.beats_per_bar).fract().abs() < 1e-9;
            let a = if on_bar { 0.058 } else { 0.021 };
            out.push(px.rect(x, y0, lw, y1 - y0, rgba(WHITE, a)));
        }
        i += 1;
    }
}

pub fn build(dl: &mut DrawList, sc: &Scene) {
    dl.clear_all();
    let px = Px { s: sc.scale };
    let (w, h) = sc.size;
    let v = &sc.view;
    let lanes_bottom = h - v.master_h;
    dl.clear = [0.0745, 0.0784, 0.0902];
    dl.lanes_clip = ((RULER_H * sc.scale).round() as u32, (lanes_bottom * sc.scale).round() as u32);

    // ---- lanes (scrolling) ----
    bands_and_grid(&mut dl.base, sc, &px, RULER_H, lanes_bottom, LANE);
    for (i, t) in sc.tracks.iter().enumerate() {
        let top = v.track_top(i);
        if top > lanes_bottom || top + TRACK_PITCH < RULER_H {
            continue;
        }
        if t.selected {
            bands_and_grid(&mut dl.base, sc, &px, top, top + TRACK_H, LANE_SEL);
        }
        let sep = if t.selected { 0x2c2d30 } else { 0x26272a };
        dl.base.push(px.rect(0.0, top + TRACK_H - 0.5, w, 1.0, rgba(rgb(sep), 1.0)));
    }

    // ---- clips ----
    for c in sc.clips {
        let (x, y, cw, ch) = c.rect(v);
        if x > w || x + cw < 0.0 || y > lanes_bottom || y + ch < RULER_H {
            continue;
        }
        let r = 3.0;
        if let Some(notes) = &c.midi {
            midi_clip(&mut dl.base, sc, &px, c, notes, (x, y, cw, ch));
            continue;
        }
        let (body, wave) = if c.selected {
            (mix(c.color, WHITE, 0.72), rgba(mix(c.color, BLACK, 0.55), 1.0))
        } else {
            (mix(rgb(0x131417), c.color, 0.30), rgba(mix(c.color, WHITE, 0.15), 1.0))
        };
        // Clip outline = rounded rect in the header colour; body drawn over it.
        dl.base.push(px.rounded(x, y, cw, ch, r, rgba(c.color, 1.0)));
        dl.base.push(px.rect(x, y + CLIP_HEADER, cw, ch - CLIP_HEADER - r, rgba(body, 1.0)));
        dl.base.push(px.rounded(x, y + ch - 2.0 * r, cw, 2.0 * r, r, rgba(body, 1.0)));
        if let Some((slot, first_frame, frames)) = c.wave {
            let vx0 = x.max(0.0);
            let vx1 = (x + cw).min(w);
            if vx1 > vx0 {
                let fpp = sc.frames_per_beat / (v.px_per_beat * sc.scale as f64);
                let p0 = (vx0 * sc.scale).floor();
                let frame_at = first_frame + (p0 as f64 - (x * sc.scale) as f64) * fpp;
                let wy = y + CLIP_HEADER + 2.0;
                let wh = ch - CLIP_HEADER - 4.0;
                dl.waves.push(WaveInst {
                    rect: [p0, (wy * sc.scale).round(), ((vx1 * sc.scale).ceil() - p0).max(1.0), (wh * sc.scale).round()],
                    color: wave,
                    params: [frame_at as f32, fpp as f32, (first_frame + frames) as f32, 0.0],
                    src: [slot.base, slot.len0, slot.levels, 0],
                });
            }
        }
    }

    // ---- takes being recorded: red-tinted growing clips with a live waveform ----
    for c in sc.live {
        live_clip(&mut dl.base, sc, &px, c);
    }

    // ---- ruler (over) ----
    let o = &mut dl.over;
    o.push(px.rect(0.0, 0.0, w, RULER_H, rgba(rgb(0x1a1b1e), 1.0)));
    let lw = 1.0 / sc.scale;
    let bar_px = sc.beats_per_bar * v.px_per_beat;
    let mut label_step = 1i64;
    while (label_step as f64) * bar_px < 24.0 {
        label_step *= 2;
    }
    let first_bar = (v.start_beats / sc.beats_per_bar).floor() as i64;
    let mut b = first_bar;
    loop {
        let x = v.x_of(b as f64 * sc.beats_per_bar);
        if x > w {
            break;
        }
        if b % label_step == 0 {
            o.push(px.rect(x, 0.0, lw, 16.5, rgba(rgb(0x4c4d4f), 1.0)));
        }
        if bar_px >= 24.0 {
            for q in 1..(sc.beats_per_bar.round() as i64) {
                let xq = v.x_of(b as f64 * sc.beats_per_bar + q as f64);
                let tall = if (q as f64) * 2.0 == sc.beats_per_bar { 4.0 } else { 2.5 };
                o.push(px.rect(xq, 16.5 - tall, lw, tall, rgba(rgb(0x3a3b3e), 1.0)));
            }
        }
        b += 1;
    }
    for y in [16.5, 25.0, 38.0] {
        o.push(px.rect(0.0, y, w, 1.0, rgba(rgb(0x2c2d30), 1.0)));
    }
    // Loop region bar (model transport.loop_start..loop_end).
    let lx0 = v.x_of(sc.loop_beats.0);
    let lx1 = v.x_of(sc.loop_beats.1);
    if lx1 > 0.0 && lx0 < w {
        o.push(px.rect(lx0.max(0.0), 18.0, lx1.min(w) - lx0.max(0.0), 6.0, rgba(rgb(0x4d4d4f), 1.0)));
    }

    // ---- master lane (over, pinned to the bottom) ----
    o.push(px.rect(0.0, h - v.master_h, w, 1.0, rgba(rgb(0x26272a), 1.0)));
    let mut tmp = Vec::new();
    bands_and_grid(&mut tmp, sc, &px, h - v.master_h + 1.0, h, LANE);
    o.extend(tmp);

    // ---- automation: track rows over the clips (scissored), master row over its lane ----
    if let Some(a) = sc.auto {
        for row in &a.rows {
            let out = if row.master { &mut dl.over } else { &mut dl.top };
            automation_row(out, &px, w, row);
        }
        if let Some([x, y, mw, mh]) = a.marquee {
            dl.top.push(px.rect(x, y, mw, mh, [0.79, 0.66, 0.24, 0.12]));
            for (rx, ry, rw, rh) in [(x, y, mw, 0.5), (x, y + mh - 0.5, mw, 0.5), (x, y, 0.5, mh), (x + mw - 0.5, y, 0.5, mh)] {
                dl.top.push(px.rect(rx, ry, rw, rh, [0.79, 0.66, 0.24, 0.8]));
            }
        }
    }

    // ---- playhead ----
    let phx = v.x_of(sc.playhead_beats);
    if phx >= -1.0 && phx <= w {
        dl.over.push(px.rect(phx, 0.0, lw, h, rgba(rgb(0x949495), 1.0)));
    }
}

/// One automation row: the dimming overlay (track lanes: clips and grid are dimmed under the
/// line, 60-automation-lanes.png), a separator under expanded rows, the curve as merged
/// per-column rects (1 pt line) and the point handles.
fn automation_row(out: &mut Vec<RectInst>, px: &Px, w: f32, row: &crate::automation::RowDraw) {
    use crate::automation::OVERLAY;
    if row.overlay {
        out.push(px.rect(0.0, row.y0, w, row.h, OVERLAY));
    }
    if row.extra {
        out.push(px.rect(0.0, row.y0 + row.h - 0.5, w, 0.5, rgba(rgb(0x26272a), 1.0)));
    }
    let color = row.style.line();
    // Curve: consecutive samples become one rect per run of equal y (flat lines are one rect).
    let half = 0.5;
    let mut i = 0;
    let p = &row.poly;
    while i + 1 < p.len() {
        let (x0, y0) = p[i];
        let mut j = i + 1;
        if (p[j].1 - y0).abs() < 1e-3 {
            while j + 1 < p.len() && (p[j + 1].1 - y0).abs() < 1e-3 {
                j += 1;
            }
            out.push(px.rect(x0, y0 - half, p[j].0 - x0, 2.0 * half, color));
        } else {
            let (lo, hi) = (y0.min(p[j].1), y0.max(p[j].1));
            out.push(px.rect(x0, lo - half, (p[j].0 - x0).max(1.0 / px.s), hi - lo + 2.0 * half, color));
        }
        i = j;
    }
    let r = 2.5;
    if let Some((x, y)) = row.start_dot {
        out.push(px.rounded(x - r, y - r, 2.0 * r, 2.0 * r, r, color));
    }
    for &(x, y, selected) in &row.handles {
        if selected {
            out.push(px.rounded(x - r - 1.0, y - r - 1.0, 2.0 * r + 2.0, 2.0 * r + 2.0, r + 1.0, color));
            out.push(px.rounded(x - r + 0.5, y - r + 0.5, 2.0 * r - 1.0, 2.0 * r - 1.0, r - 0.5, [1.0, 1.0, 1.0, 1.0]));
        } else {
            out.push(px.rounded(x - r, y - r, 2.0 * r, 2.0 * r, r, color));
        }
    }
}

/// MIDI clip: track-colour title strip, light lavender body (the reference draws it
/// the same selected or not), grid lines inside the body and a mini note view.
fn midi_clip(out: &mut Vec<RectInst>, sc: &Scene, px: &Px, c: &ClipVis, notes: &[[f32; 3]], (x, y, cw, ch): (f32, f32, f32, f32)) {
    let r = 3.0;
    // On a track that isn't selected the reference draws the body in the plain track colour
    // with a darker line under the title and no grid (50-input-selector-open.png); on the
    // selected track it is the light lavender body with grid lines (33/34-*.png).
    let track_selected = sc.tracks.get(c.track_index).is_some_and(|t| t.selected);
    let body = if track_selected { mix(c.color, WHITE, 0.67) } else { c.color };
    // 0.5 pt outline (greyish in the reference), then title strip and body inset by it.
    out.push(px.rounded(x, y, cw, ch, r, rgba(mix(body, rgb(0x4a4c58), 0.45), 1.0)));
    out.push(px.rounded(x + 0.5, y + 0.5, cw - 1.0, ch - 1.0, r - 0.5, rgba(c.color, 1.0)));
    let by = y + CLIP_HEADER + 0.5;
    out.push(px.rect(x + 0.5, by, cw - 1.0, ch - CLIP_HEADER - 0.5 - r, rgba(body, 1.0)));
    out.push(px.rounded(x + 0.5, y + ch - 2.0 * r, cw - 1.0, 2.0 * r - 0.5, r - 0.5, rgba(body, 1.0)));
    let v = &sc.view;
    if !track_selected {
        out.push(px.rect(x + 0.5, by - 0.5, cw - 1.0, 0.5, rgba(mix(c.color, BLACK, 0.4), 1.0)));
    }
    let mut step = sc.beats_per_bar / 2.0;
    while step * v.px_per_beat < 10.0 {
        step *= 2.0;
    }
    while step * v.px_per_beat > 80.0 && step > 0.25 {
        step /= 2.0;
    }
    let mut b = (c.start_beats / step).floor() * step + step;
    while track_selected && b < c.start_beats + c.len_beats - 1e-6 {
        let lx = v.x_of(b);
        if lx > x + 0.5 {
            out.push(px.rect(lx, by, 1.0 / px.s, ch - CLIP_HEADER - 1.0, rgba(mix(body, BLACK, 0.075), 1.0)));
        }
        b += step;
    }
    let nh = ch - CLIP_HEADER - 6.0;
    let bar_h = (nh / 24.0).clamp(1.5, 3.0);
    let ink = rgba(mix(c.color, BLACK, 0.45), 1.0);
    for n in notes {
        let nx = x + n[0] * cw;
        let nw = (n[1] * cw).max(1.0).min(x + cw - nx);
        if nw <= 0.0 {
            continue;
        }
        out.push(px.rect(nx, by + 2.5 + n[2] * (nh - bar_h), nw, bar_h, ink));
    }
}

/// A take in progress: recording-red header strip, dark red body, live peaks.
fn live_clip(out: &mut Vec<RectInst>, sc: &Scene, px: &Px, c: &ClipVis) {
    let v = &sc.view;
    let w = sc.size.0;
    let lanes_bottom = sc.size.1 - sc.view.master_h;
    let (x, y, cw, ch) = c.rect(v);
    if x > w || x + cw < 0.0 || y > lanes_bottom || y + ch < RULER_H {
        return;
    }
    let r = 3.0;
    let body = mix(rgb(0x131417), c.color, 0.28);
    out.push(px.rounded(x, y, cw, ch, r, rgba(c.color, 1.0)));
    out.push(px.rect(x, y + CLIP_HEADER, cw, ch - CLIP_HEADER - r, rgba(body, 1.0)));
    out.push(px.rounded(x, y + ch - 2.0 * r, cw, 2.0 * r, r, rgba(body, 1.0)));
    let Some((peaks, step)) = &c.live else { return };
    let wy = y + CLIP_HEADER + 2.0;
    let wh = ch - CLIP_HEADER - 4.0;
    let mid = wy + wh / 2.0;
    let ink = rgba(mix(c.color, WHITE, 0.25), 1.0);
    let lw = (*step).max(1.0 / px.s);
    for (i, p) in peaks.iter().enumerate() {
        let px_x = x + i as f32 * step;
        if px_x > x + cw || px_x > w {
            break;
        }
        if px_x + lw < 0.0 {
            continue;
        }
        let (lo, hi) = (p[0].clamp(-1.0, 1.0), p[1].clamp(-1.0, 1.0));
        let top = mid - hi * wh / 2.0;
        let h = ((hi - lo) * wh / 2.0).max(1.0 / px.s);
        out.push(px.rect(px_x, top, lw, h, ink));
    }
}

/// Bar-number labels for the ruler (Slint text over the texture).
pub fn ruler_labels(v: &View, w: f32, beats_per_bar: f64) -> Vec<(f32, String)> {
    let bar_px = beats_per_bar * v.px_per_beat;
    let mut step = 1i64;
    while (step as f64) * bar_px < 24.0 {
        step *= 2;
    }
    let mut out = Vec::new();
    let mut b = ((v.start_beats / beats_per_bar).floor() as i64).div_euclid(step) * step;
    loop {
        let x = v.x_of(b as f64 * beats_per_bar);
        if x > w {
            break;
        }
        if x > -30.0 && b >= 0 {
            out.push((x, format!("{}", b + 1)));
        }
        b += step;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn view_roundtrip_and_hits() {
        let v = View::default();
        assert!((v.x_of(4.0) - 51.03).abs() < 0.01);
        assert!((v.beat_at(v.x_of(4.0)) - 4.0).abs() < 1e-4);
        assert_eq!(v.track_at(RULER_H + 1.0), Some(0));
        assert_eq!(v.track_at(RULER_H + TRACK_PITCH + 1.0), Some(1));
        let clips = vec![ClipVis { track_index: 0, start_beats: 4.0, len_beats: 4.0, color: [1.0; 3], selected: false, name: String::new(), wave: None, midi: None, live: None }];
        let size = (900.0, 400.0);
        assert_eq!(hit_test(&v, size, 2, &clips, 10.0, 5.0), Hit::Ruler);
        assert_eq!(hit_test(&v, size, 2, &clips, 70.0, 60.0), Hit::Clip { index: 0, zone: Zone::Body });
        assert_eq!(hit_test(&v, size, 2, &clips, 52.0, 60.0), Hit::Clip { index: 0, zone: Zone::Left });
        assert_eq!(hit_test(&v, size, 2, &clips, 101.0, 60.0), Hit::Clip { index: 0, zone: Zone::Right });
        assert_eq!(hit_test(&v, size, 2, &clips, 10.0, 60.0), Hit::Track(0));
        assert_eq!(hit_test(&v, size, 2, &clips, 10.0, RULER_H + 2.0 * TRACK_PITCH + 5.0), Hit::Empty);
        assert_eq!(hit_test(&v, size, 2, &clips, 10.0, 395.0), Hit::Master);
    }
}
