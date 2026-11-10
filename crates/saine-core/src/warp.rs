//! Audio clip warping: how a clip follows the project tempo.
//!
//! A warped clip is placed in beats. Its source has an original tempo
//! (`original_bpm`); without markers, source second `s` sits at original beat
//! `s * original_bpm / 60`. Warp markers pin source frames to original beats,
//! the map is linear between them and extends the first / last segment beyond
//! them (one marker: the default slope through it). `speed` divides every
//! original beat, so 2x plays the source twice as fast (half the footprint).
//! The engine gets the map as (timeline frame, source frame) points.
//!
//! Transients are detected by the engine (`se_asset_detect_onsets`) and are
//! not stored; the user's additions and removals are, in source frames.

use crate::model::{Asset, Clip, PPQ, Project};
use serde::{Deserialize, Serialize};

/// The Mode menu (the reference app's names): Auto decides per passage, Drums keeps hits
/// sharp, Melody keeps tones even, Vocal keeps a voice natural when
/// transposed, Re-pitch lets pitch move with speed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WarpMode {
    #[default]
    Auto,
    Drums,
    Melody,
    Vocal,
    Repitch,
}

impl WarpMode {
    pub const ALL: [WarpMode; 5] = [WarpMode::Auto, WarpMode::Drums, WarpMode::Melody, WarpMode::Vocal, WarpMode::Repitch];

    pub fn label(self) -> &'static str {
        match self {
            WarpMode::Auto => "Auto",
            WarpMode::Drums => "Drums",
            WarpMode::Melody => "Melody",
            WarpMode::Vocal => "Vocal",
            WarpMode::Repitch => "Re-pitch",
        }
    }
}

/// Speed menu presets.
pub const SPEED_PRESETS: [f64; 6] = [0.5, 0.75, 1.0, 1.25, 1.5, 2.0];
/// Original BPM range (drag and inline edit).
pub const MIN_ORIGINAL_BPM: f64 = 20.0;
pub const MAX_ORIGINAL_BPM: f64 = 300.0;
pub const MIN_SPEED: f64 = 0.125;
pub const MAX_SPEED: f64 = 8.0;
/// Transients closer than this are the same transient.
pub const TRANSIENT_TOLERANCE_S: f64 = 0.001;

/// A source frame pinned to an original beat.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WarpMarker {
    /// Asset frame (native rate).
    pub source: u64,
    /// Original beats from the asset start (before `speed`).
    pub beat: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ClipWarp {
    /// Warp on: the clip follows the project tempo. Off: it plays 1:1.
    pub enabled: bool,
    pub mode: WarpMode,
    /// Tempo the source was recorded at (20..=300).
    pub original_bpm: f64,
    pub speed: f64,
    /// Sorted, strictly increasing in source and beat.
    pub markers: Vec<WarpMarker>,
    /// User edits on top of the detected transients (asset frames, sorted).
    pub transients_added: Vec<u64>,
    pub transients_removed: Vec<u64>,
}

impl Default for ClipWarp {
    fn default() -> Self {
        ClipWarp { enabled: false, mode: WarpMode::Auto, original_bpm: 120.0, speed: 1.0, markers: Vec::new(), transients_added: Vec::new(), transients_removed: Vec::new() }
    }
}

/// A detected transient (asset frames).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transient {
    pub source: u64,
    pub strength: f32,
    pub percussive: bool,
}

impl ClipWarp {
    pub fn is_default(&self) -> bool {
        *self == ClipWarp::default()
    }

    /// Map points as (source frame, original beat), at least two.
    fn points(&self, rate: f64) -> Vec<(f64, f64)> {
        let slope = self.original_bpm / 60.0 / rate;
        match self.markers.as_slice() {
            [] => vec![(0.0, 0.0), (rate, rate * slope)],
            [m] => vec![(m.source as f64, m.beat), (m.source as f64 + rate, m.beat + rate * slope)],
            ms => ms.iter().map(|m| (m.source as f64, m.beat)).collect(),
        }
    }

    /// Original beat of a source frame (asset rate).
    pub fn beat_at(&self, source: f64, rate: f64) -> f64 {
        interp(&self.points(rate), source, false)
    }

    /// Source frame (asset rate) at an original beat.
    pub fn source_at(&self, beat: f64, rate: f64) -> f64 {
        interp(&self.points(rate), beat, true)
    }

    /// Speed shown in the editor: 1 when warp is off.
    pub fn effective_speed(&self) -> f64 {
        if self.enabled { self.speed } else { 1.0 }
    }

    pub fn validate(&self, asset: &Asset) -> Result<(), String> {
        if !self.original_bpm.is_finite() || !(MIN_ORIGINAL_BPM..=MAX_ORIGINAL_BPM).contains(&self.original_bpm) {
            return Err(format!("original BPM {} outside {MIN_ORIGINAL_BPM}..={MAX_ORIGINAL_BPM}", self.original_bpm));
        }
        if !self.speed.is_finite() || !(MIN_SPEED..=MAX_SPEED).contains(&self.speed) {
            return Err(format!("speed {} outside {MIN_SPEED}..={MAX_SPEED}", self.speed));
        }
        if self.markers.iter().any(|m| !m.beat.is_finite() || m.source > asset.frames) {
            return Err("warp marker outside the source".into());
        }
        if self.markers.windows(2).any(|w| w[1].source <= w[0].source || w[1].beat <= w[0].beat) {
            return Err("warp markers must increase in source and beat".into());
        }
        for list in [&self.transients_added, &self.transients_removed] {
            if list.windows(2).any(|w| w[1] <= w[0]) || list.last().is_some_and(|&t| t >= asset.frames) {
                return Err("transient edits must be sorted and inside the source".into());
            }
        }
        Ok(())
    }

    /// Detected transients with the user's additions and removals applied.
    /// An added transient takes the percussive flag of the passage it splits.
    pub fn transients(&self, detected: &[Transient], rate: f64) -> Vec<Transient> {
        let tol = (TRANSIENT_TOLERANCE_S * rate).round() as u64;
        let near = |a: u64, b: u64| a.abs_diff(b) <= tol;
        let mut out: Vec<Transient> = detected.iter().filter(|t| !self.transients_removed.iter().any(|&r| near(r, t.source))).copied().collect();
        for &a in &self.transients_added {
            if out.iter().any(|t| near(t.source, a)) {
                continue;
            }
            let percussive = detected.iter().rev().find(|t| t.source <= a).is_none_or(|t| t.percussive);
            out.push(Transient { source: a, strength: 1.0, percussive });
        }
        out.sort_by_key(|t| t.source);
        out
    }

    /// Add a transient at `source` (no-op returns false when one is there).
    pub fn add_transient(&mut self, detected: &[Transient], source: u64, rate: f64) -> bool {
        let tol = (TRANSIENT_TOLERANCE_S * rate).round() as u64;
        if self.transients(detected, rate).iter().any(|t| t.source.abs_diff(source) <= tol) {
            return false;
        }
        if let Some(i) = self.transients_removed.iter().position(|&r| r.abs_diff(source) <= tol) {
            self.transients_removed.remove(i);
        } else {
            insert_sorted(&mut self.transients_added, source);
        }
        true
    }

    /// Remove the transient at `source` (within the tolerance).
    pub fn remove_transient(&mut self, detected: &[Transient], source: u64, rate: f64) -> bool {
        let tol = (TRANSIENT_TOLERANCE_S * rate).round() as u64;
        let Some(t) = self.transients(detected, rate).into_iter().find(|t| t.source.abs_diff(source) <= tol) else { return false };
        if let Some(i) = self.transients_added.iter().position(|&a| a.abs_diff(t.source) <= tol) {
            self.transients_added.remove(i);
        } else {
            insert_sorted(&mut self.transients_removed, t.source);
        }
        true
    }

    pub fn move_transient(&mut self, detected: &[Transient], from: u64, to: u64, rate: f64) -> bool {
        let saved = self.clone();
        if self.remove_transient(detected, from, rate) && self.add_transient(detected, to, rate) {
            return true;
        }
        *self = saved;
        false
    }

    /// Pin `source` at original beat `beat` (replacing a marker at that
    /// source). Refused when it would cross a neighbour.
    pub fn set_marker(&mut self, source: u64, beat: f64) -> bool {
        let mut ms: Vec<WarpMarker> = self.markers.iter().copied().filter(|m| m.source != source).collect();
        let i = ms.partition_point(|m| m.source < source);
        if (i > 0 && ms[i - 1].beat >= beat) || (i < ms.len() && ms[i].beat <= beat) || !beat.is_finite() {
            return false;
        }
        ms.insert(i, WarpMarker { source, beat });
        self.markers = ms;
        true
    }

    pub fn remove_marker(&mut self, index: usize) -> bool {
        if index >= self.markers.len() {
            return false;
        }
        self.markers.remove(index);
        true
    }
}

fn insert_sorted(v: &mut Vec<u64>, x: u64) {
    let i = v.partition_point(|&y| y < x);
    v.insert(i, x);
}

/// Piecewise-linear lookup on (x, y) points, extending the end segments;
/// `inverse` looks up x from y.
fn interp(points: &[(f64, f64)], at: f64, inverse: bool) -> f64 {
    let get = |p: (f64, f64)| if inverse { (p.1, p.0) } else { p };
    let n = points.len();
    let i = points.partition_point(|&p| get(p).0 <= at).clamp(1, n - 1);
    let (x0, y0) = get(points[i - 1]);
    let (x1, y1) = get(points[i]);
    y0 + (at - x0) * (y1 - y0) / (x1 - x0)
}

/// Engine mode for a model mode (`SE_WARP_*`).
pub fn engine_mode(mode: WarpMode) -> u8 {
    match mode {
        WarpMode::Repitch => 0,
        WarpMode::Drums => 1,
        WarpMode::Melody => 2,
        WarpMode::Vocal => 3,
        WarpMode::Auto => 4,
    }
}

impl Project {
    /// Timeline beats a warped clip covers.
    pub fn warped_clip_beats(&self, clip: &Clip) -> f64 {
        let rate = self.asset(clip.asset).map_or(48_000.0, |a| a.sample_rate);
        let w = &clip.warp;
        let a = w.beat_at(clip.source_offset as f64, rate);
        let b = w.beat_at((clip.source_offset + clip.source_length) as f64, rate);
        (b - a) / w.speed
    }

    /// The warp map of a warped clip as (timeline frame, source frame) at
    /// `engine_rate`: the clip's ends plus every marker inside it.
    pub fn warp_map(&self, clip: &Clip, engine_rate: f64) -> Vec<(i64, f64)> {
        let rate = self.asset(clip.asset).map_or(48_000.0, |a| a.sample_rate);
        let w = &clip.warp;
        let ratio = engine_rate / rate;
        let first = w.beat_at(clip.source_offset as f64, rate);
        let start = self.ticks_to_frames(clip.start, engine_rate);
        let frame_of = |beat: f64| start + ((beat - first) / w.speed * 60.0 / self.transport.bpm * engine_rate).round() as i64;
        let end_source = (clip.source_offset + clip.source_length) as f64;
        let mut sources = vec![clip.source_offset as f64];
        sources.extend(w.markers.iter().map(|m| m.source as f64).filter(|&s| s > clip.source_offset as f64 && s < end_source));
        sources.push(end_source);
        let mut out: Vec<(i64, f64)> = Vec::with_capacity(sources.len());
        for s in sources {
            let p = (frame_of(w.beat_at(s, rate)), s * ratio);
            if out.last().is_none_or(|q| p.0 > q.0 && p.1 > q.1) {
                out.push(p);
            }
        }
        if out.len() < 2 {
            let (f, s) = out[0];
            out.push((f + 1, s + 1.0));
        }
        out
    }
}

/// Ticks for a beat count.
pub fn beats_to_ticks(beats: f64) -> i64 {
    (beats * PPQ as f64).round() as i64
}
