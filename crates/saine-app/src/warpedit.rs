//! Audio clip warping in the clip editor: the Speed / Warp / Mode menus, the
//! Original BPM drag, warp markers (handle band) and transients (lane), and
//! the onset analysis that feeds Drums / Auto slicing.
//!
//! Every edit is one `Command::SetClipWarp` named like the reference app's undo steps;
//! drags commit live and coalesce into one step per gesture kind.

use crate::controller::{Controller, JobResult};
use crate::peaks::{L0_FRAMES, Pyramid};
use crate::pianoroll::{BAND_Y, GRID_Y, PrView};
use saine_core::warp::{ClipWarp, SPEED_PRESETS, Transient, WarpMode};
use saine_core::{AssetId, Clip, ClipId, Command};
use std::collections::HashMap;
use std::time::{Duration, Instant};

const DOUBLE_CLICK: Duration = Duration::from_millis(350);
/// Warp-marker flags sit in the upper half of the handle band.
const MARKER_BAND: (f32, f32) = (BAND_Y, 40.5);
const MARKER_HIT: f32 = 6.0;
const TRANSIENT_HIT: f32 = 4.0;
const TRANSIENT_SNAP: f32 = 8.0;

#[derive(Debug, Clone)]
pub struct AcDrag {
    /// The view when the gesture started (the clip length changes under a marker drag).
    pub view: PrView,
    pub clip: ClipId,
    pub warp: ClipWarp,
    pub kind: AcDragKind,
    pub moved: bool,
}

#[derive(Debug, Clone, Copy)]
pub enum AcDragKind {
    Marker { source: u64 },
    Transient { from: u64, to: u64 },
    Bpm { start: f64 },
    Transpose { start: i32 },
}

/// Semitones per point of Transpose drag (the reference app: 48 st over 200 pt).
const TRANSPOSE_PER_PT: f32 = 48.0 / 200.0;

/// "0 st", "+3 st", "-1.5 st" (whole semitones bare, others to one decimal).
pub fn format_semitones(st: f64) -> String {
    let st = (st * 10.0).round() / 10.0;
    if st == 0.0 {
        return "0 st".into();
    }
    let sign = if st > 0.0 { "+" } else { "" };
    if st.fract() == 0.0 { format!("{sign}{st:.0} st") } else { format!("{sign}{st:.1} st") }
}

/// "2x", "0.75x", "1.33x" (the reference app's speed format).
pub fn format_speed(speed: f64) -> String {
    let s = format!("{:.2}", (speed * 100.0).round() / 100.0);
    format!("{}x", s.trim_end_matches('0').trim_end_matches('.'))
}

/// Conversions between clip-relative beats and source frames of one clip
/// (mirrored when the clip plays reversed).
struct Map<'a> {
    w: &'a ClipWarp,
    rate: f64,
    first: f64,
    /// Reversed clips: their length in beats.
    mirror: Option<f64>,
}

impl<'a> Map<'a> {
    fn new(c: &'a Clip, w: &'a ClipWarp, rate: f64) -> Self {
        let first = w.beat_at(c.source_offset as f64, rate);
        let mirror = w.reverse.then(|| (w.beat_at((c.source_offset + c.source_length) as f64, rate) - first) / w.speed);
        Map { w, rate, first, mirror }
    }
    fn flip(&self, clip_beat: f64) -> f64 {
        self.mirror.map_or(clip_beat, |len| len - clip_beat)
    }
    fn clip_beat(&self, source: f64) -> f64 {
        self.flip((self.w.beat_at(source, self.rate) - self.first) / self.w.speed)
    }
    fn original_beat(&self, clip_beat: f64) -> f64 {
        self.first + self.flip(clip_beat) * self.w.speed
    }
    fn source(&self, clip_beat: f64) -> f64 {
        self.w.source_at(self.original_beat(clip_beat), self.rate)
    }
}

impl Controller {
    fn asset_rate(&self, asset: AssetId) -> f64 {
        self.project().asset(asset).map_or(48_000.0, |a| a.sample_rate)
    }

    /// Make the engine's reversed copy (and its peaks) of every asset a
    /// reversed clip plays. A copy is a few ms per minute of audio.
    pub fn ensure_reversed_assets(&mut self) {
        let wanted: Vec<(AssetId, u64)> = self
            .project()
            .tracks
            .iter()
            .flat_map(|t| &t.clips)
            .filter(|c| c.warp.reverse && !self.reversed_assets.contains_key(&c.asset))
            .filter_map(|c| Some((c.asset, *self.asset_map.get(&c.asset)?)))
            .collect();
        for (asset, ea) in wanted {
            if self.reversed_assets.contains_key(&asset) {
                continue;
            }
            match self.engine.reverse_asset(ea) {
                Ok(info) => {
                    let pairs = (info.frames / L0_FRAMES as u64 + 1) as usize;
                    if let Ok(l0) = self.engine.peaks(info.asset_id, 0, L0_FRAMES, pairs) {
                        self.peaks.insert(info.asset_id, Pyramid::from_level0(l0));
                    }
                    self.reversed_assets.insert(asset, info.asset_id);
                }
                Err(e) => self.set_status(format!("Reverse failed: {e}"), 6),
            }
        }
    }

    /// Reverse toggle in the clip editor toolbar (one undo step).
    pub fn ac_reverse_toggle(&mut self) {
        let Some(c) = self.ac_clip().cloned() else { return };
        self.set_clip_warp(c.id, ClipWarp { reverse: !c.warp.reverse, ..c.warp.clone() }, "Toggle clip reverse");
    }

    /// Detected transients of a model asset (empty until analysed).
    pub fn detected_transients(&self, asset: AssetId) -> &[Transient] {
        self.asset_map.get(&asset).and_then(|ea| self.onsets.get(ea)).map_or(&[], |v| v.as_slice())
    }

    /// Detected transients of every analysed asset, for the snapshot.
    pub fn transients_by_asset(&self) -> HashMap<AssetId, Vec<Transient>> {
        self.asset_map.iter().filter_map(|(a, ea)| self.onsets.get(ea).map(|v| (*a, v.clone()))).collect()
    }

    /// Analyse the assets of warped clips that have no onsets yet (worker thread).
    pub fn request_onsets(&mut self) {
        let wanted: Vec<u64> = self
            .project()
            .tracks
            .iter()
            .flat_map(|t| &t.clips)
            .filter(|c| c.warp.enabled)
            .filter_map(|c| self.asset_map.get(&c.asset).copied())
            .filter(|ea| !self.onsets.contains_key(ea) && !self.onsets_pending.contains(ea))
            .collect();
        for ea in wanted {
            self.onsets_pending.insert(ea);
            self.pending_jobs += 1;
            let (engine, tx) = (self.engine.clone(), self.jobs_tx.clone());
            let _ = std::thread::Builder::new().name("onsets".into()).spawn(move || {
                let _ = tx.send(JobResult::Onsets { engine_asset: ea, result: engine.detect_onsets(ea).map_err(|e| e.to_string()) });
            });
        }
    }

    pub fn finish_onsets(&mut self, engine_asset: u64, result: Result<Vec<saine_engine::Onset>, String>) {
        self.onsets_pending.remove(&engine_asset);
        let Some(asset) = self.asset_map.iter().find(|(_, ea)| **ea == engine_asset).map(|(a, _)| *a) else { return };
        let k = self.asset_rate(asset) / self.rate;
        let list = match result {
            Ok(v) => v.iter().map(|o| Transient { source: (o.frame as f64 * k).round().max(0.0) as u64, strength: o.strength, percussive: o.percussive }).collect(),
            Err(e) => {
                self.set_status(format!("Transient detection failed: {e}"), 4);
                Vec::new()
            }
        };
        self.onsets.insert(engine_asset, list);
        self.publish();
        self.ui_dirty = true;
    }

    fn set_clip_warp(&mut self, clip: ClipId, warp: ClipWarp, label: &str) -> bool {
        if self.project().clip(clip).is_none_or(|c| c.warp == warp) {
            return false;
        }
        self.execute(Command::SetClipWarp { clip, warp, label: label.into() })
    }

    /// The open audio clip's warp, switched on the way the reference app does it: the
    /// first time at the project tempo, so the clip keeps its length.
    fn warp_on(&self, c: &Clip) -> ClipWarp {
        let mut w = c.warp.clone();
        if !w.enabled && w.tempo_is_default() {
            w.original_bpm = self.project().transport.bpm;
        }
        w.enabled = true;
        w
    }

    /// Speed menu: a preset turns warp on.
    pub fn warp_pick_speed(&mut self, index: usize) {
        let (Some(c), Some(&speed)) = (self.ac_clip().cloned(), SPEED_PRESETS.get(index)) else { return };
        let w = ClipWarp { speed, ..self.warp_on(&c) };
        self.set_clip_warp(c.id, w, "Set clip speed");
    }

    /// Warp menu. Turning warp off with a speed other than 1x also resets the speed.
    pub fn warp_pick_enabled(&mut self, on: bool) {
        let Some(c) = self.ac_clip().cloned() else { return };
        if on {
            let w = self.warp_on(&c);
            self.set_clip_warp(c.id, w, "Set clip warp mode");
            return;
        }
        let reset = (c.warp.speed - 1.0).abs() > 0.001;
        let w = ClipWarp { enabled: false, speed: 1.0, ..c.warp.clone() };
        self.set_clip_warp(c.id, w, if reset { "Reset clip speed" } else { "Set clip warp mode" });
    }

    /// Mode menu (only with warp on).
    pub fn warp_pick_mode(&mut self, index: usize) {
        let (Some(c), Some(&mode)) = (self.ac_clip().cloned(), WarpMode::ALL.get(index)) else { return };
        if !c.warp.enabled {
            return;
        }
        self.set_clip_warp(c.id, ClipWarp { mode, ..c.warp.clone() }, "Set clip mode");
    }

    /// Transpose can be edited except in Re-pitch, where the pitch follows the speed.
    pub fn ac_transpose_editable(c: &Clip) -> bool {
        !(c.warp.enabled && c.warp.mode == WarpMode::Repitch)
    }

    /// Transpose readout: the clip's semitones, or in Re-pitch the shift its tempo ratio makes.
    pub fn ac_transpose_text(&self, c: &Clip) -> String {
        if Self::ac_transpose_editable(c) {
            return format_semitones(c.warp.transpose as f64);
        }
        format_semitones(12.0 * (self.project().transport.bpm * c.warp.speed / c.warp.original_bpm).log2())
    }

    /// Transpose drag (phase 0 press, 1 move, 2 release): right or up raises,
    /// 48 st per 200 pt, clamped to -24..=24, one undo step.
    pub fn ac_transpose_drag(&mut self, phase: i32, dx: f32, dy: f32) {
        let Some(c) = self.ac_clip().cloned() else { return };
        self.ui_dirty = true;
        match phase {
            0 if Self::ac_transpose_editable(&c) => {
                let start = c.warp.transpose;
                self.pr.ac_drag = Some(AcDrag { view: self.pr_view(), clip: c.id, warp: c.warp.clone(), kind: AcDragKind::Transpose { start }, moved: false });
            }
            1 => {
                let Some(d) = self.pr.ac_drag.as_mut() else { return };
                let AcDragKind::Transpose { start } = d.kind else { return };
                let st = (start + ((dx - dy) * TRANSPOSE_PER_PT).round() as i32).clamp(-saine_core::warp::MAX_TRANSPOSE, saine_core::warp::MAX_TRANSPOSE);
                if st == d.warp.transpose {
                    return;
                }
                d.moved = true;
                d.warp.transpose = st;
                let (clip, w) = (d.clip, d.warp.clone());
                self.set_clip_warp(clip, w, "Adjust clip transpose");
            }
            _ => {
                if matches!(self.pr.ac_drag, Some(AcDrag { kind: AcDragKind::Transpose { .. }, .. })) {
                    self.pr.ac_drag = None;
                }
            }
        }
    }

    /// Original BPM shown in the toolbar (the project tempo until it is set).
    pub fn ac_original_bpm(&self, c: &Clip) -> f64 {
        if let Some(AcDrag { kind: AcDragKind::Bpm { .. }, warp, .. }) = &self.pr.ac_drag {
            return warp.original_bpm;
        }
        if c.warp.tempo_is_default() { self.project().transport.bpm } else { c.warp.original_bpm }
    }

    /// Original BPM drag (phase 0 press, 1 move, 2 release): one BPM per point,
    /// right or up, clamped to 20..300, one undo step.
    pub fn ac_bpm_drag(&mut self, phase: i32, dx: f32, dy: f32) {
        let Some(c) = self.ac_clip().cloned() else { return };
        self.ui_dirty = true;
        match phase {
            0 => {
                let start = self.ac_original_bpm(&c).round();
                self.pr.ac_drag = Some(AcDrag { view: self.pr_view(), clip: c.id, warp: c.warp.clone(), kind: AcDragKind::Bpm { start }, moved: false });
            }
            1 => {
                let Some(d) = self.pr.ac_drag.as_mut() else { return };
                let AcDragKind::Bpm { start } = d.kind else { return };
                let acc = dx - dy;
                if !d.moved && acc.abs() <= 2.0 {
                    return;
                }
                d.moved = true;
                d.warp.original_bpm = (start + acc.round() as f64).clamp(saine_core::warp::MIN_ORIGINAL_BPM, saine_core::warp::MAX_ORIGINAL_BPM);
                let (clip, w) = (d.clip, d.warp.clone());
                self.set_clip_warp(clip, w, "Set original BPM");
            }
            _ => {
                if matches!(self.pr.ac_drag, Some(AcDrag { kind: AcDragKind::Bpm { .. }, .. })) {
                    self.pr.ac_drag = None;
                }
            }
        }
    }

    /// Warp markers of the open audio clip (clip-relative beats; drag preview included).
    pub fn ac_marker_beats(&self) -> Vec<f64> {
        let Some(c) = self.ac_clip() else { return Vec::new() };
        if !c.warp.enabled {
            return Vec::new();
        }
        let rate = self.asset_rate(c.asset);
        let m = Map::new(c, &c.warp, rate);
        c.warp.markers.iter().map(|k| m.clip_beat(k.source as f64)).collect()
    }

    /// Transients of the open audio clip (clip-relative beats; drag preview included).
    pub fn ac_transient_beats(&self) -> Vec<f64> {
        let Some(c) = self.ac_clip() else { return Vec::new() };
        if !c.warp.enabled {
            return Vec::new();
        }
        let rate = self.asset_rate(c.asset);
        let m = Map::new(c, &c.warp, rate);
        let (from, to) = match self.pr.ac_drag {
            Some(AcDrag { kind: AcDragKind::Transient { from, to }, .. }) => (Some(from), to),
            _ => (None, 0),
        };
        let end = c.source_offset + c.source_length;
        c.warp
            .transients(self.detected_transients(c.asset), rate)
            .iter()
            .map(|t| if Some(t.source) == from { to } else { t.source })
            .filter(|&s| s >= c.source_offset && s < end)
            .map(|s| m.clip_beat(s as f64))
            .collect()
    }

    /// Pointer in the audio clip editor (kind 0 down, 1 move, 2 up).
    /// Handle band (upper half): click adds a warp marker (snapped to a
    /// transient unless Alt), drag moves one, click without dragging removes
    /// it. Lane: drag a transient to move it, double-click to add or remove
    /// one. Ruler: move the playhead. Markers and transients need warp on.
    pub fn ac_pointer(&mut self, kind: i32, x: f32, y: f32, alt: bool) {
        let Some(c) = self.ac_clip().cloned() else { return };
        self.ui_dirty = true;
        let rate = self.asset_rate(c.asset);
        match kind {
            0 => {
                self.menu_close();
                let now = Instant::now();
                let double = self.pr.ac_last_down.is_some_and(|(t, lx, ly)| now - t < DOUBLE_CLICK && (lx - x).abs() < 4.0 && (ly - y).abs() < 4.0);
                self.pr.ac_last_down = Some((now, x, y));
                let view = self.pr_view();
                if y < BAND_Y {
                    self.seek_beats((c.start as f64 / saine_core::PPQ as f64) + view.beat_at(x).max(0.0));
                    return;
                }
                if !c.warp.enabled {
                    return;
                }
                let m = Map::new(&c, &c.warp, rate);
                let detected = self.detected_transients(c.asset).to_vec();
                let transients = c.warp.transients(&detected, rate);
                let near = |source: u64, tol: f32| (view.x_of(m.clip_beat(source as f64)) - x).abs() <= tol;
                if (MARKER_BAND.0..MARKER_BAND.1).contains(&y) {
                    if let Some(k) = c.warp.markers.iter().filter(|k| near(k.source, MARKER_HIT)).min_by(|a, b| {
                        let d = |s: u64| (view.x_of(m.clip_beat(s as f64)) - x).abs();
                        d(a.source).total_cmp(&d(b.source))
                    }) {
                        self.pr.ac_drag = Some(AcDrag { view, clip: c.id, warp: c.warp.clone(), kind: AcDragKind::Marker { source: k.source }, moved: false });
                        return;
                    }
                    let beat = view.beat_at(x);
                    let snapped = if alt { None } else { transients.iter().map(|t| t.source).filter(|&s| near(s, TRANSIENT_SNAP)).min_by_key(|&s| (view.x_of(m.clip_beat(s as f64)) - x).abs() as i64) };
                    let source = snapped.unwrap_or_else(|| m.source(beat).round().max(0.0) as u64);
                    if source <= c.source_offset || source >= c.source_offset + c.source_length {
                        return;
                    }
                    let mut w = c.warp.clone();
                    if w.markers.is_empty() {
                        w.set_marker(c.source_offset, m.original_beat(0.0));
                    }
                    if w.set_marker(source, c.warp.beat_at(source as f64, rate)) {
                        self.set_clip_warp(c.id, w, "Add warp marker");
                    }
                    return;
                }
                if y < GRID_Y {
                    return;
                }
                let hit = transients.iter().map(|t| t.source).filter(|&s| near(s, TRANSIENT_HIT)).min_by_key(|&s| (view.x_of(m.clip_beat(s as f64)) - x).abs() as i64);
                let mut w = c.warp.clone();
                match (double, hit) {
                    (true, Some(s)) => {
                        if w.remove_transient(&detected, s, rate) {
                            self.set_clip_warp(c.id, w, "Remove transient");
                        }
                    }
                    (true, None) => {
                        let s = m.source(view.beat_at(x)).round().max(0.0) as u64;
                        if w.add_transient(&detected, s, rate) {
                            self.set_clip_warp(c.id, w, "Add transient");
                        }
                    }
                    (false, Some(s)) => {
                        self.pr.ac_drag = Some(AcDrag { view, clip: c.id, warp: c.warp.clone(), kind: AcDragKind::Transient { from: s, to: s }, moved: false });
                    }
                    (false, None) => {}
                }
            }
            1 => {
                let Some(d) = self.pr.ac_drag.as_mut() else { return };
                let m = Map::new(&c, &d.warp, rate);
                let beat = d.view.beat_at(x);
                match d.kind {
                    AcDragKind::Marker { source } => {
                        let mut w = d.warp.clone();
                        if !w.set_marker(source, m.original_beat(beat)) {
                            return;
                        }
                        d.moved = true;
                        let clip = d.clip;
                        self.set_clip_warp(clip, w, "Move warp marker");
                    }
                    AcDragKind::Transient { from, .. } => {
                        let to = m.source(beat).round().clamp(c.source_offset as f64, (c.source_offset + c.source_length - 1) as f64) as u64;
                        d.moved = d.moved || to != from;
                        d.kind = AcDragKind::Transient { from, to };
                    }
                    AcDragKind::Bpm { .. } | AcDragKind::Transpose { .. } => {}
                }
            }
            _ => {
                let Some(d) = self.pr.ac_drag.take() else { return };
                match d.kind {
                    AcDragKind::Marker { source } if !d.moved => {
                        let mut w = d.warp.clone();
                        if let Some(i) = w.markers.iter().position(|k| k.source == source)
                            && w.remove_marker(i)
                        {
                            self.set_clip_warp(d.clip, w, "Remove warp marker");
                        }
                    }
                    AcDragKind::Transient { from, to } if d.moved => {
                        let detected = self.detected_transients(c.asset).to_vec();
                        let mut w = d.warp.clone();
                        if w.move_transient(&detected, from, to, rate) {
                            self.set_clip_warp(d.clip, w, "Move transient");
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::format_speed;

    #[test]
    fn speeds_format_like_create() {
        assert_eq!(format_speed(2.0), "2x");
        assert_eq!(format_speed(0.5), "0.5x");
        assert_eq!(format_speed(0.75), "0.75x");
        assert_eq!(format_speed(1.333), "1.33x");
        assert_eq!(format_speed(1.0), "1x");
    }

    #[test]
    fn semitones_format_like_create() {
        use super::format_semitones;
        assert_eq!(format_semitones(0.0), "0 st");
        assert_eq!(format_semitones(-0.04), "0 st");
        assert_eq!(format_semitones(3.0), "+3 st");
        assert_eq!(format_semitones(-12.0), "-12 st");
        assert_eq!(format_semitones(4.98), "+5 st");
        assert_eq!(format_semitones(-1.53), "-1.5 st");
    }
}
