//! Timeline header minimap.
//!
//! The strip shows the whole arrangement: one thin bar per clip, in the track's colour, one
//! band per track, plus a window for the part of the song the lanes show and a playhead line.
//! Rust builds the bars as a small model; Slint only places them. Interaction is handled here too:
//! drag the window to scroll, drag it sideways and vertically to scroll and zoom, drag an edge
//! to resize it, press outside it to jump there (and keep dragging), double-click to fit the
//! arrangement, wheel to scroll (zoom with Ctrl / Alt).
//!
//! All geometry below is in 100%-zoom px (CSS); the Controller methods convert the
//! design units (0.85 x 100%-zoom px) that Slint sends.

use crate::controller::{Controller, track_color};
use crate::timeline::{ClipVis, MAX_PX_PER_BEAT, MIN_PX_PER_BEAT};
use crate::{AppWindow, MiniBar, Studio};
use slint::{ComponentHandle, VecModel};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

/// Design units per 100%-zoom px.
pub const CSS: f32 = 0.85;
/// The strip is 20 px high (at 100% zoom).
const HEIGHT: f32 = 20.0;
/// Vertical padding the bands keep from the strip's top and bottom.
const PAD: f32 = 2.0;
/// A band is at most 3 px thick and at most 78 % of its row; at least 1 px.
const BAND_MAX: f32 = 3.0;
const BAND_FILL: f32 = 0.78;
/// Muted and not-soloed tracks are drawn at 30 %, the rest at 95 %.
const ALPHA: f32 = 0.95;
const ALPHA_DIM: f32 = 0.3;
/// The whole-song map is at least this many beats long.
const MIN_MAP_BEATS: f64 = 16.0;
/// Press within 5 px of a window edge grabs the edge.
const EDGE_GRAB: f64 = 5.0;
/// A press must travel 3 px before it counts as a drag.
const DRAG_SLOP: f32 = 3.0;
/// Vertical drag zoom: span x exp(-0.01 x px) (down zooms in).
const ZOOM_PER_PX: f64 = 0.01;
/// Wheel zoom: span x exp(-0.008 x delta).
const WHEEL_ZOOM: f64 = 0.008;
/// Smoothing of the drag velocity that splits a drag between scrolling and zooming.
const VELOCITY_SMOOTH: f32 = 0.3;

/// A bar of the minimap. x and w are fractions of the strip width, y and h design units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bar {
    pub x: f32,
    pub w: f32,
    pub y: f32,
    pub h: f32,
    pub color: [f32; 3],
    pub alpha: f32,
}

/// What the strip needs to know about a track.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrackInfo {
    pub color: [f32; 3],
    /// Muted, or another track is soloed.
    pub dim: bool,
}

/// Visible range in beats.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Range {
    pub start: f64,
    pub end: f64,
}

impl Range {
    fn span(&self) -> f64 {
        self.end - self.start
    }
}

/// Map length and view window.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Metrics {
    pub map_end: f64,
    pub start: f64,
    pub span: f64,
}

/// Limits the lanes impose on a range.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub min_span: f64,
    pub max_span: f64,
    /// Lane width in design units.
    pub lanes_w: f32,
}

impl Limits {
    pub fn new(lanes_w: f32) -> Limits {
        let w = lanes_w.max(1.0) as f64;
        Limits { min_span: w / MAX_PX_PER_BEAT, max_span: w / MIN_PX_PER_BEAT, lanes_w }
    }
}

/// End of the last clip, in beats.
pub fn content_end(clips: &[ClipVis]) -> f64 {
    clips.iter().fold(0.0, |m, c| m.max(c.start_beats + c.len_beats))
}

/// The map covers the whole song, but never less than the window or 16 beats; frozen pins it
/// while a drag is running so the strip does not rescale under the pointer.
pub fn metrics(content_end: f64, start: f64, span: f64, frozen: Option<f64>) -> Metrics {
    Metrics { map_end: frozen.unwrap_or_else(|| content_end.max(start + span).max(MIN_MAP_BEATS)), start, span }
}

/// Bars for every clip: each track gets a band of (20 - 4) / tracks px; the bar is centred in it.
pub fn bars(tracks: &[TrackInfo], clips: &[ClipVis], map_end: f64) -> Vec<Bar> {
    if tracks.is_empty() || map_end <= 0.0 {
        return Vec::new();
    }
    let row = (HEIGHT - 2.0 * PAD) / tracks.len() as f32;
    let h = (row * BAND_FILL).clamp(1.0, BAND_MAX);
    clips
        .iter()
        .filter_map(|c| {
            let t = tracks.get(c.track_index)?;
            let y = PAD + c.track_index as f32 * row + (row - h) / 2.0;
            Some(Bar {
                x: (c.start_beats / map_end) as f32,
                w: (c.len_beats / map_end) as f32,
                y: y * CSS,
                h: h * CSS,
                color: t.color,
                alpha: if t.dim { ALPHA_DIM } else { ALPHA },
            })
        })
        .collect()
}

/// Keep a range inside the song: the start never goes before beat 0 or past the point where the
/// window's end meets the map's end.
fn clamp_range(r: Range, map_end: f64) -> Range {
    let n = r.span();
    let hi = (map_end - n).max(0.0);
    let start = r.start.max(0.0).min(hi);
    Range { start, end: start + n }
}

fn limit_span(span: f64, l: &Limits) -> f64 {
    let lo = l.min_span.max(0.001);
    span.clamp(lo, l.max_span.max(lo))
}

/// Scale the span by factor keeping anchor at the same fraction of the window, then shift by pan.
fn zoom_about(start0: f64, span0: f64, anchor: f64, pan: f64, factor: f64, l: &Limits) -> Range {
    let span = limit_span(span0 * factor, l);
    let frac = if span0 > 0.0 { (anchor - start0) / span0 } else { 0.5 };
    let start = anchor + pan - frac * span;
    Range { start, end: start + span }
}

/// The range that fits the whole arrangement (at least 8 beats, 2 % of slack at the end).
pub fn fit(content_end: f64) -> Range {
    let t = content_end.max(8.0);
    Range { start: 0.0, end: t + (t * 0.02).max(1.0) }
}

/// Wheel over the strip: scroll by the dominant delta, or zoom around the pointer. dx and dy are
/// Slint scroll deltas in design units (positive moves the content right / down).
pub fn wheel(m: &Metrics, x: f32, width: f32, dx: f32, dy: f32, zoom: bool, l: &Limits) -> Range {
    let w = (width / CSS).max(1.0) as f64;
    let anchor = (x / CSS) as f64 / w * m.map_end;
    if zoom {
        let factor = (-(dy as f64) / CSS as f64 * WHEEL_ZOOM).exp();
        return clamp_range(zoom_about(m.start, m.span, anchor, 0.0, factor, l), m.map_end);
    }
    let d = if dx.abs() > dy.abs() { dx } else { dy } / CSS;
    let beats = -(d as f64) * (m.map_end / w);
    clamp_range(Range { start: m.start + beats, end: m.start + beats + m.span }, m.map_end)
}

/// What a press grabbed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Grab {
    EdgeLeft,
    EdgeRight,
    Window,
    /// Pressed outside the window: it jumped there and a drag keeps scrolling / zooming.
    Jump,
}

/// A drag in progress (positions in 100%-zoom px).
#[derive(Debug, Clone)]
pub struct Drag {
    pub grab: Grab,
    /// The map length when the drag began (the strip does not rescale during it).
    pub map_end: f64,
    start0: f64,
    span0: f64,
    width: f32,
    anchor: f64,
    pan_beats: f64,
    zoom_px: f64,
    velocity: (f32, f32),
    pending: (f32, f32),
    travel: (f32, f32),
    press: (f32, f32),
    last: (f32, f32),
    moved: bool,
}

/// Press at (x, y) (design units) in a strip width wide. Returns the drag and, for a press
/// outside the window, the range that centres the window on the pointer.
pub fn press(m: &Metrics, x: f32, y: f32, width: f32) -> Option<(Drag, Option<Range>)> {
    if m.span <= 0.0 || m.map_end <= 0.0 || width <= 0.0 {
        return None;
    }
    let (px, py, w) = (x / CSS, y / CSS, width / CSS);
    let scale = w as f64 / m.map_end;
    let (a, b) = (m.start * scale, (m.start + m.span) * scale);
    let p = px as f64;
    let grab = if (p - a).abs() <= EDGE_GRAB {
        Grab::EdgeLeft
    } else if (p - b).abs() <= EDGE_GRAB {
        Grab::EdgeRight
    } else if p > a && p < b {
        Grab::Window
    } else {
        Grab::Jump
    };
    let anchor = p / scale;
    let mut start0 = m.start;
    let mut jump = None;
    if grab == Grab::Jump {
        let r = clamp_range(Range { start: anchor - m.span / 2.0, end: anchor + m.span / 2.0 }, m.map_end);
        start0 = r.start;
        jump = Some(r);
    }
    let drag = Drag {
        grab,
        map_end: m.map_end,
        start0,
        span0: m.span,
        width: w,
        anchor,
        pan_beats: 0.0,
        zoom_px: 0.0,
        velocity: (0.0, 0.0),
        pending: (0.0, 0.0),
        travel: (0.0, 0.0),
        press: (px, py),
        last: (px, py),
        moved: false,
    };
    Some((drag, jump))
}

impl Drag {
    /// Window and Jump drags hide the pointer once they count as drags; edge drags keep it.
    pub fn hides_pointer(&self) -> bool {
        self.moved && matches!(self.grab, Grab::Window | Grab::Jump)
    }

    /// Where the pointer goes on release, in strip coordinates (design units): the strip position
    /// of the spot the drag grabbed (kept inside the strip), on the row where it was pressed.
    pub fn landing(&self) -> (f32, f32) {
        let x = ((self.anchor + self.pan_beats) / self.map_end * self.width as f64).clamp(0.0, self.width as f64) as f32;
        (x * CSS, self.press.1 * CSS)
    }

    /// Pointer moved to (x, y) (design units); returns the new range once the drag has travelled past the slop.
    pub fn moved(&mut self, x: f32, y: f32, l: &Limits) -> Option<Range> {
        let (px, py) = (x / CSS, y / CSS);
        let (dx, dy) = (px - self.last.0, py - self.last.1);
        self.last = (px, py);
        self.pending.0 += dx;
        self.pending.1 += dy;
        self.velocity.0 += VELOCITY_SMOOTH * (dx.abs() - self.velocity.0);
        self.velocity.1 += VELOCITY_SMOOTH * (dy.abs() - self.velocity.1);
        self.travel.0 += dx;
        self.travel.1 += dy;
        if !self.moved && self.travel.0.abs() + self.travel.1.abs() < DRAG_SLOP {
            return None;
        }
        self.moved = true;
        let r = match self.grab {
            Grab::EdgeLeft | Grab::EdgeRight => {
                let beat = (px as f64 / self.width as f64 * self.map_end).clamp(0.0, self.map_end);
                if self.grab == Grab::EdgeLeft {
                    let fixed = self.start0 + self.span0;
                    let span = limit_span(fixed - beat, l);
                    let start = (fixed - span).max(0.0);
                    Range { start, end: start + span }
                } else {
                    let span = limit_span(beat - self.start0, l);
                    Range { start: self.start0, end: self.start0 + span }
                }
            }
            Grab::Window | Grab::Jump => self.pan_zoom(l),
        };
        self.pending = (0.0, 0.0);
        Some(r)
    }

    /// Splits the pending travel between scrolling and zooming by the smoothed velocity of each axis.
    fn pan_zoom(&mut self, l: &Limits) -> Range {
        let (vx, vy) = (self.velocity.0.powi(4), self.velocity.1.powi(4));
        let (wp, wz) = if vx + vy == 0.0 { (1.0, 1.0) } else { (vx / (vx + vy), vy / (vx + vy)) };
        let zoom_px = self.zoom_px + (self.pending.1 * wz) as f64;
        let factor = (-zoom_px * ZOOM_PER_PX).exp();
        let span = limit_span(self.span0 * factor, l);
        let mut per_px = self.map_end / self.width as f64 * span / self.span0;
        if l.lanes_w > 0.0 {
            per_px = per_px.min(4.0 * span / (l.lanes_w / CSS) as f64);
        }
        let pan = self.pan_beats + (self.pending.0 * wp) as f64 * per_px;
        let r = clamp_range(zoom_about(self.start0, self.span0, self.anchor, pan, factor, l), self.map_end);
        let frac = (self.anchor - self.start0) / self.span0;
        self.zoom_px = -(r.span() / self.span0).ln() / ZOOM_PER_PX;
        self.pan_beats = r.start + frac * r.span() - self.anchor;
        r
    }
}

/// Controller-side state of the strip.
pub struct State {
    pub drag: Option<Drag>,
    pub model: Rc<VecModel<MiniBar>>,
    /// Hash of the clip, track and map-length inputs the bars were built for.
    built: Option<u64>,
    content_end: f64,
    /// Hash of the clip positions content_end was computed from.
    content_key: Option<u64>,
}

impl State {
    pub fn new() -> State {
        State { drag: None, model: Rc::new(VecModel::default()), built: None, content_end: 0.0, content_key: None }
    }
}

impl Controller {
    fn minimap_limits(&self) -> Limits {
        Limits::new(self.lanes_size.0)
    }

    fn minimap_content_end(&mut self) -> f64 {
        let mut h = DefaultHasher::new();
        for c in &self.clip_vis {
            (c.track_index, c.start_beats.to_bits(), c.len_beats.to_bits()).hash(&mut h);
        }
        let key = Some(h.finish());
        if self.mm.content_key != key {
            self.mm.content_key = key;
            self.mm.content_end = content_end(&self.clip_vis);
        }
        self.mm.content_end
    }

    pub fn minimap_content_end_for_test(&self) -> f64 {
        content_end(&self.clip_vis)
    }

    pub fn minimap_metrics(&mut self) -> Metrics {
        let ce = self.minimap_content_end();
        let span = self.lanes_size.0 as f64 / self.view.px_per_beat;
        metrics(ce, self.view.start_beats, span, self.mm.drag.as_ref().map(|d| d.map_end))
    }

    /// Show r in the lanes.
    fn minimap_set_range(&mut self, r: Range) {
        let span = r.span().max(0.001);
        self.view.px_per_beat = (self.lanes_size.0 as f64 / span).clamp(MIN_PX_PER_BEAT, MAX_PX_PER_BEAT);
        self.view.start_beats = r.start;
        self.view.clamp();
    }

    /// Pointer on the strip: kind 0 press, 1 move, 2 release; x and y in strip coordinates, width the strip's width,
    /// (ox, oy) the strip's origin in the window (where the pointer is put back after a hidden drag).
    pub fn minimap_pointer(&mut self, kind: i32, x: f32, y: f32, width: f32, ox: f32, oy: f32) {
        let l = self.minimap_limits();
        match kind {
            0 => {
                let m = self.minimap_metrics();
                if let Some((drag, jump)) = press(&m, x, y, width) {
                    self.mm.drag = Some(drag);
                    if let Some(r) = jump {
                        self.minimap_set_range(r);
                    }
                }
            }
            1 => {
                let Some(mut drag) = self.mm.drag.take() else { return };
                let r = drag.moved(x, y, &l);
                if drag.hides_pointer() {
                    self.ptr.hide();
                }
                self.mm.drag = Some(drag);
                if let Some(r) = r {
                    self.minimap_set_range(r);
                }
            }
            _ => {
                if let Some(d) = self.mm.drag.take() {
                    let (lx, ly) = d.landing();
                    self.ptr.release(Some((ox + lx, oy + ly)));
                }
            }
        }
    }

    /// Double-click: fit the arrangement.
    pub fn minimap_fit(&mut self) {
        let ce = self.minimap_content_end();
        self.minimap_set_range(fit(ce));
    }

    pub fn minimap_scroll(&mut self, dx: f32, dy: f32, x: f32, width: f32, zoom: bool) {
        let m = self.minimap_metrics();
        let l = self.minimap_limits();
        if width > 0.0 && m.span > 0.0 {
            let r = wheel(&m, x, width, dx, dy, zoom, &l);
            self.minimap_set_range(r);
        }
    }

    /// Push the bars, the window and the playhead to Slint (every frame; the bars only when the
    /// clips or the map length changed).
    pub fn update_minimap(&mut self, app: &AppWindow) {
        if self.lanes_size.0 <= 0.0 {
            return;
        }
        let s = app.global::<Studio>();
        let m = self.minimap_metrics();
        let p = self.doc.project();
        let any_solo = p.tracks.iter().any(|t| t.solo);
        let tracks: Vec<TrackInfo> = p
            .tracks
            .iter()
            .take(p.ordinary_track_count())
            .map(|t| TrackInfo { color: track_color(t.color), dim: t.mute || (any_solo && !t.solo) })
            .collect();
        let mut h = DefaultHasher::new();
        (self.mm.content_key, m.map_end.to_bits()).hash(&mut h);
        for t in &tracks {
            (t.color.map(f32::to_bits), t.dim).hash(&mut h);
        }
        let key = Some(h.finish());
        if key != self.mm.built {
            self.mm.built = key;
            let rows: Vec<MiniBar> = bars(&tracks, &self.clip_vis, m.map_end)
                .into_iter()
                .map(|b| MiniBar {
                    x: b.x,
                    w: b.w,
                    y: b.y,
                    h: b.h,
                    color: slint::Color::from_argb_f32(b.alpha, b.color[0], b.color[1], b.color[2]),
                })
                .collect();
            crate::controller::sync_model(&self.mm.model, rows);
        }
        let (wx, ww) = ((m.start / m.map_end) as f32, (m.span / m.map_end) as f32);
        if s.get_minimap_win_x() != wx {
            s.set_minimap_win_x(wx);
        }
        if s.get_minimap_win_w() != ww {
            s.set_minimap_win_w(ww);
        }
        let ph = self.playhead_ticks() as f64 / dstudio_core::PPQ as f64 / m.map_end;
        let ph = if (0.0..=1.0).contains(&ph) { ph as f32 } else { -1.0 };
        if s.get_minimap_playhead() != ph {
            s.set_minimap_playhead(ph);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clip(track: usize, start: f64, len: f64) -> ClipVis {
        ClipVis {
            track_index: track,
            start_beats: start,
            len_beats: len,
            color: [1.0; 3],
            selected: false,
            muted: false,
            name: String::new(),
            wave: None,
            midi: None,
            live: None,
        }
    }

    fn tracks(n: usize) -> Vec<TrackInfo> {
        vec![TrackInfo { color: [0.5; 3], dim: false }; n]
    }

    fn lim() -> Limits {
        Limits::new(30.0)
    }

    #[test]
    fn map_covers_song_window_and_16_beats() {
        assert_eq!(metrics(0.0, 0.0, 4.0, None).map_end, 16.0);
        assert_eq!(metrics(24.0, 0.0, 40.0, None).map_end, 40.0);
        assert_eq!(metrics(100.0, 0.0, 40.0, None).map_end, 100.0);
        assert_eq!(metrics(100.0, 0.0, 40.0, Some(50.0)).map_end, 50.0);
    }

    #[test]
    fn band_geometry_matches_the_reference_rules() {
        let b = bars(&tracks(4), &[clip(0, 8.0, 16.0), clip(3, 0.0, 0.0)], 40.0);
        assert_eq!(b.len(), 2);
        assert!((b[0].x - 0.2).abs() < 1e-6 && (b[0].w - 0.4).abs() < 1e-6);
        assert!((b[0].h - 3.0 * CSS).abs() < 1e-5);
        assert!((b[0].y - 2.5 * CSS).abs() < 1e-5);
        assert!((b[1].y - (2.0 + 3.0 * 4.0 + 0.5) * CSS).abs() < 1e-5);
        let one = bars(&tracks(1), &[clip(0, 0.0, 4.0)], 16.0);
        assert!((one[0].y - 8.5 * CSS).abs() < 1e-5);
        let many = bars(&tracks(40), &[clip(0, 0.0, 4.0)], 16.0);
        assert!((many[0].h - CSS).abs() < 1e-5);
        assert!(bars(&[], &[clip(0, 0.0, 4.0)], 16.0).is_empty());
    }

    #[test]
    fn dim_tracks_use_the_low_alpha() {
        let t = [TrackInfo { color: [0.0; 3], dim: true }];
        assert_eq!(bars(&t, &[clip(0, 0.0, 4.0)], 16.0)[0].alpha, ALPHA_DIM);
        assert_eq!(bars(&tracks(1), &[clip(0, 0.0, 4.0)], 16.0)[0].alpha, ALPHA);
    }

    #[test]
    fn fit_has_slack_and_a_floor() {
        assert_eq!(fit(0.0), Range { start: 0.0, end: 9.0 });
        assert_eq!(fit(100.0), Range { start: 0.0, end: 102.0 });
    }

    #[test]
    fn press_picks_edge_window_or_jump() {
        let m = Metrics { map_end: 100.0, start: 20.0, span: 20.0 };
        let w = 100.0 * CSS;
        let at = |px: f32| press(&m, px * CSS, 5.0, w).unwrap();
        assert_eq!(at(20.0).0.grab, Grab::EdgeLeft);
        assert_eq!(at(24.0).0.grab, Grab::EdgeLeft);
        assert_eq!(at(26.0).0.grab, Grab::Window);
        assert_eq!(at(39.0).0.grab, Grab::EdgeRight);
        let (d, jump) = at(70.0);
        assert_eq!(d.grab, Grab::Jump);
        let j = jump.unwrap();
        assert!((j.start - 60.0).abs() < 1e-9 && (j.end - 80.0).abs() < 1e-9);
        assert!(press(&m, 0.0, 0.0, 0.0).is_none());
    }

    #[test]
    fn jump_stays_inside_the_song() {
        let m = Metrics { map_end: 100.0, start: 0.0, span: 20.0 };
        let (_, j) = press(&m, 99.0 * CSS, 5.0, 100.0 * CSS).unwrap();
        assert_eq!(j.unwrap(), Range { start: 80.0, end: 100.0 });
    }

    #[test]
    fn window_drag_scrolls_one_to_one_when_horizontal() {
        let m = Metrics { map_end: 100.0, start: 20.0, span: 20.0 };
        let (mut d, _) = press(&m, 30.0 * CSS, 5.0 * CSS, 100.0 * CSS).unwrap();
        let mut r = None;
        for i in 1..=10 {
            r = d.moved((30.0 + i as f32) * CSS, 5.0 * CSS, &lim()).or(r);
        }
        let r = r.unwrap();
        assert!((r.span() - 20.0).abs() < 1e-6);
        assert!((r.start - 30.0).abs() < 1e-3, "{r:?}");
    }

    #[test]
    fn dragging_down_zooms_in() {
        let m = Metrics { map_end: 100.0, start: 20.0, span: 20.0 };
        let (mut d, _) = press(&m, 30.0 * CSS, 5.0 * CSS, 100.0 * CSS).unwrap();
        let mut r = None;
        for i in 1..=10 {
            r = d.moved(30.0 * CSS, (5.0 + 5.0 * i as f32) * CSS, &lim()).or(r);
        }
        let r = r.unwrap();
        assert!(r.span() < 20.0 && r.span() > 0.0);
        assert!(r.start > 20.0 && r.end < 40.0, "the window shrinks around the pressed spot: {r:?}");
    }

    #[test]
    fn small_moves_are_ignored() {
        let m = Metrics { map_end: 100.0, start: 20.0, span: 20.0 };
        let (mut d, _) = press(&m, 30.0 * CSS, 5.0 * CSS, 100.0 * CSS).unwrap();
        assert!(d.moved(31.0 * CSS, 5.0 * CSS, &lim()).is_none());
    }

    #[test]
    fn edge_drags_resize_against_the_fixed_edge() {
        let m = Metrics { map_end: 100.0, start: 20.0, span: 20.0 };
        let (mut d, _) = press(&m, 40.0 * CSS, 5.0 * CSS, 100.0 * CSS).unwrap();
        let r = d.moved(60.0 * CSS, 5.0 * CSS, &lim()).unwrap();
        assert!((r.start - 20.0).abs() < 1e-9 && (r.end - 60.0).abs() < 1e-9, "{r:?}");
        let (mut d, _) = press(&m, 20.0 * CSS, 5.0 * CSS, 100.0 * CSS).unwrap();
        let r = d.moved(0.0, 5.0 * CSS, &lim()).unwrap();
        assert!((r.start - 0.0).abs() < 1e-9 && (r.end - 40.0).abs() < 1e-9, "{r:?}");
    }

    #[test]
    fn wheel_scrolls_and_zooms() {
        let m = Metrics { map_end: 100.0, start: 20.0, span: 20.0 };
        let w = 100.0 * CSS;
        let down = wheel(&m, 0.0, w, 0.0, -10.0 * CSS, false, &lim());
        assert!((down.start - 30.0).abs() < 1e-9, "{down:?}");
        let zoom = wheel(&m, 30.0 * CSS, w, 0.0, 50.0 * CSS, true, &lim());
        assert!(zoom.span() < 20.0 && zoom.start >= 20.0 && zoom.end <= 40.0, "{zoom:?}");
        let end = wheel(&m, 0.0, w, 0.0, -1000.0 * CSS, false, &lim());
        assert!((end.end - 100.0).abs() < 1e-9, "{end:?}");
    }
}
