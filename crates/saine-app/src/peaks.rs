//! Waveform peak pyramids built from real engine data (`Engine::peaks`).
//!
//! Level 0 holds one (min, max) pair per [`L0_FRAMES`] frames at the engine
//! rate; each further level halves the count until one pair is left. All
//! pyramids are packed into one linear array uploaded as one texture
//! (`gpu::Renderer::sync_peaks`); the shader reconstructs level offsets from
//! the level-0 length, so the packing here must match `fs_wave`.

use crate::gpu::WaveInst;
use saine_core::{Clip, PPQ, Project};
use std::collections::HashMap;

pub const L0_FRAMES: u32 = 32;

/// One asset's pyramid (computed on a worker thread).
#[derive(Debug, Clone)]
pub struct Pyramid {
    pub data: Vec<[f32; 2]>,
    pub len0: u32,
    pub levels: u32,
}

impl Pyramid {
    /// Build from level-0 pairs (as returned by `Engine::peaks(id, 0, L0_FRAMES, n)`).
    pub fn from_level0(l0: Vec<[f32; 2]>) -> Self {
        let len0 = l0.len().max(1) as u32;
        let mut data = if l0.is_empty() { vec![[0.0, 0.0]] } else { l0 };
        let mut start = 0usize;
        let mut len = len0 as usize;
        let mut levels = 1;
        while len > 1 {
            let next = len.div_ceil(2);
            for i in 0..next {
                let a = data[start + 2 * i];
                let b = if 2 * i + 1 < len { data[start + 2 * i + 1] } else { a };
                data.push([a[0].min(b[0]), a[1].max(b[1])]);
            }
            start += len;
            len = next;
            levels += 1;
        }
        Pyramid { data, len0, levels }
    }
}

/// Where an asset's pyramid lives in the packed array.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Slot {
    pub base: u32,
    pub len0: u32,
    pub levels: u32,
}

#[derive(Default)]
pub struct PeakStore {
    pub data: Vec<[f32; 2]>,
    slots: HashMap<u64, Slot>,
    /// Left and right pyramids of stereo assets (the clip editor's lanes).
    channels: HashMap<u64, [Slot; 2]>,
    /// Bumped on every change; the renderer re-uploads when it differs.
    pub generation: u64,
}

impl PeakStore {
    pub fn insert(&mut self, engine_asset: u64, p: Pyramid) {
        if self.slots.contains_key(&engine_asset) {
            return;
        }
        let base = self.data.len() as u32;
        self.data.extend_from_slice(&p.data);
        self.slots.insert(engine_asset, Slot { base, len0: p.len0, levels: p.levels });
        self.generation += 1;
    }

    pub fn slot(&self, engine_asset: u64) -> Option<Slot> {
        self.slots.get(&engine_asset).copied()
    }

    pub fn insert_channels(&mut self, engine_asset: u64, lr: [Pyramid; 2]) {
        if self.channels.contains_key(&engine_asset) {
            return;
        }
        let slots = lr.map(|p| {
            let base = self.data.len() as u32;
            self.data.extend_from_slice(&p.data);
            Slot { base, len0: p.len0, levels: p.levels }
        });
        self.channels.insert(engine_asset, slots);
        self.generation += 1;
    }

    pub fn channel_slots(&self, engine_asset: u64) -> Option<[Slot; 2]> {
        self.channels.get(&engine_asset).copied()
    }
}

/// One linear piece of a clip's waveform: clip-relative `beats` show the
/// asset frames `frames` (engine rate).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WaveSeg {
    pub beats: (f64, f64),
    pub frames: (f64, f64),
}

/// A clip's waveform as linear pieces: one for a plain clip, one per stretch
/// between warp markers for a warped clip.
pub fn wave_segments(p: &Project, clip: &Clip, engine_rate: f64) -> Vec<WaveSeg> {
    let ratio = engine_rate / p.asset(clip.asset).map_or(engine_rate, |a| a.sample_rate);
    let (s0, s1) = (clip.source_offset as f64, (clip.source_offset + clip.source_length) as f64);
    let mut sources = vec![s0];
    if clip.warp.enabled {
        sources.extend(clip.warp.markers.iter().map(|m| m.source as f64).filter(|&s| s > s0 && s < s1));
    }
    sources.push(s1);
    let beat = |s: f64| p.clip_ticks_at(clip, s) / PPQ as f64;
    let segs: Vec<WaveSeg> = sources
        .windows(2)
        .map(|w| WaveSeg { beats: (beat(w[0]), beat(w[1])), frames: (w[0] * ratio, w[1] * ratio) })
        .filter(|s| s.beats.1 > s.beats.0)
        .collect();
    if !clip.warp.reverse {
        return segs;
    }
    // Reversed: the engine's back-to-front copy, read along the mirrored pieces.
    let len = segs.last().map_or(0.0, |s| s.beats.1);
    let frames = p.asset(clip.asset).map_or(0.0, |a| a.frames as f64 * ratio);
    segs.iter().rev().map(|s| WaveSeg { beats: (len - s.beats.1, len - s.beats.0), frames: (frames - s.frames.1, frames - s.frames.0) }).collect()
}

/// Wave instances for `segs`: `x_of` maps clip-relative beats to logical x,
/// `visible` is the logical x range to fill and `(y, h)` the logical band.
/// Pieces meet on whole pixels, so none overlaps its neighbour.
#[allow(clippy::too_many_arguments)]
pub fn push_wave(out: &mut Vec<WaveInst>, slot: Slot, segs: &[WaveSeg], x_of: impl Fn(f64) -> f32, visible: (f32, f32), (y, h): (f32, f32), scale: f32, color: [f32; 4]) {
    let (lo, hi) = ((visible.0 * scale) as f64, (visible.1 * scale) as f64);
    for s in segs {
        let (x0, x1) = ((x_of(s.beats.0) * scale) as f64, (x_of(s.beats.1) * scale) as f64);
        let (p0, p1) = (x0.max(lo).round(), x1.min(hi).round());
        if x1 <= x0 || p1 <= p0 {
            continue;
        }
        let fpp = (s.frames.1 - s.frames.0) / (x1 - x0);
        out.push(WaveInst {
            rect: [p0 as f32, (y * scale).round(), (p1 - p0) as f32, (h * scale).round()],
            color,
            params: [(s.frames.0 + (p0 - x0) * fpp) as f32, fpp as f32, s.frames.1 as f32, 0.0],
            src: [slot.base, slot.len0, slot.levels, 0],
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use saine_core::{Asset, AssetId, ClipId, ClipWarp, WarpMode};
    #[test]
    fn pyramid_levels() {
        let p = Pyramid::from_level0(vec![[-0.5, 0.5], [-1.0, 0.2], [-0.1, 0.9]]);
        assert_eq!(p.len0, 3);
        assert_eq!(p.levels, 3); // 3 -> 2 -> 1
        assert_eq!(p.data.len(), 3 + 2 + 1);
        assert_eq!(p.data[3], [-1.0, 0.5]);
        assert_eq!(p.data[5], [-1.0, 0.9]);
    }

    fn project_with_clip(warp: Option<ClipWarp>) -> (Project, Clip) {
        let mut p = Project::default();
        p.transport.bpm = 120.0;
        p.assets.push(Asset { id: AssetId(0), relative_path: None, absolute_path: "/nonexistent/loop.wav".into(), sample_rate: 44_100.0, channels: 2, frames: 8 * 44_100 });
        let clip = Clip { id: ClipId(1), asset: AssetId(0), name: String::new(), start: 0, source_offset: 0, source_length: 8 * 44_100, gain: 1.0, warp: warp.unwrap_or_default() };
        (p, clip)
    }

    #[test]
    fn plain_clip_is_one_piece_at_the_engine_rate() {
        let (p, c) = project_with_clip(None);
        let segs = wave_segments(&p, &c, 48_000.0);
        assert_eq!(segs.len(), 1);
        assert!((segs[0].beats.1 - 16.0).abs() < 1e-6, "{segs:?}");
        assert!(segs[0].frames.0 == 0.0 && (segs[0].frames.1 - 8.0 * 48_000.0).abs() < 1e-6, "{segs:?}");
    }

    #[test]
    fn warped_clip_follows_its_markers() {
        let mut w = ClipWarp { enabled: true, mode: WarpMode::Drums, original_bpm: 120.0, ..Default::default() };
        w.set_marker(0, 0.0);
        w.set_marker(4 * 44_100, 6.0);
        let (p, c) = project_with_clip(Some(w));
        let segs = wave_segments(&p, &c, 44_100.0);
        assert_eq!(segs.len(), 2, "{segs:?}");
        assert!((segs[0].beats.1 - 6.0).abs() < 1e-6 && segs[0].frames.1 == 4.0 * 44_100.0, "{segs:?}");
        assert_eq!(segs[0].beats.1, segs[1].beats.0);
        let total = p.warped_clip_beats(&c);
        assert!((segs[1].beats.1 - total).abs() < 1e-6, "{segs:?} vs {total}");
    }

    #[test]
    fn pieces_tile_the_pixels_and_clip_to_the_view() {
        let slot = Slot { base: 0, len0: 1, levels: 1 };
        let segs = [WaveSeg { beats: (0.0, 1.3), frames: (0.0, 1000.0) }, WaveSeg { beats: (1.3, 4.0), frames: (1000.0, 4000.0) }];
        let mut out = Vec::new();
        push_wave(&mut out, slot, &segs, |b| 10.0 + b as f32 * 33.3, (20.0, 500.0), (0.0, 50.0), 2.0, [1.0; 4]);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].rect[0], 40.0);
        assert_eq!(out[0].rect[0] + out[0].rect[2], out[1].rect[0]);
        // Clipped on the left: the first column starts 10 logical px (20 px) into the piece.
        let fpp = 1000.0 / (1.3 * 33.3 * 2.0);
        assert!((out[0].params[0] as f64 - 20.0 * fpp).abs() < 0.5, "{:?}", out[0].params);
        assert!((out[1].params[1] as f64 - 3000.0 / (2.7 * 33.3 * 2.0)).abs() < 1e-3);
        assert_eq!(out[1].params[2], 4000.0);
    }
    #[test]
    fn reversed_clip_reads_the_mirrored_pieces() {
        let mut w = ClipWarp { enabled: true, mode: WarpMode::Drums, original_bpm: 120.0, reverse: true, ..Default::default() };
        w.set_marker(0, 0.0);
        w.set_marker(4 * 44_100, 6.0);
        let (p, c) = project_with_clip(Some(w));
        let segs = wave_segments(&p, &c, 44_100.0);
        let total = p.warped_clip_beats(&c);
        assert_eq!(segs.len(), 2, "{segs:?}");
        // The last forward piece (beats 6.., frames 4 s .. 8 s) plays first, from the copy's start.
        assert!(segs[0].beats.0.abs() < 1e-9 && (segs[0].beats.1 - (total - 6.0)).abs() < 1e-6, "{segs:?}");
        assert!(segs[0].frames.0.abs() < 1e-6 && (segs[0].frames.1 - 4.0 * 44_100.0).abs() < 1e-6, "{segs:?}");
        assert!((segs[1].beats.1 - total).abs() < 1e-6 && (segs[1].frames.1 - 8.0 * 44_100.0).abs() < 1e-6, "{segs:?}");
    }
}
