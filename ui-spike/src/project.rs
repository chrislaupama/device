//! Synthetic project data: audio sources, a min/max peak pyramid, and a clip layout.

pub const SAMPLE_RATE: f32 = 48_000.0;
pub const BPM: f32 = 120.0;
pub const SAMPLES_PER_BEAT: f32 = SAMPLE_RATE * 60.0 / BPM; // 24_000
pub const SOURCE_SAMPLES: usize = 2_000_000;
pub const NUM_SOURCES: usize = 8;
/// Samples per peak at pyramid level 0 (log2 = 5).
pub const L0_SHIFT: u32 = 5;
pub const PEAK_TEX_WIDTH: u32 = 4096;

/// One min/max peak pyramid for all sources, packed into a single linear
/// array of (min, max) pairs. Every source has the same level layout, so
/// a texel index is `source_base[src] + level_offset[level] + i`.
pub struct PeakPyramid {
    pub data: Vec<[f32; 2]>,
    pub level_offsets: Vec<u32>,
    pub level_lens: Vec<u32>,
    pub source_stride: u32,
    /// Per-source level-0 peaks (min,max) at 32 spp, CPU-side, for the Path baseline.
    pub l0: Vec<Vec<[f32; 2]>>,
}

fn synth_source(seed: u32, out: &mut Vec<f32>) {
    // Deterministic, cheap, audio-looking: decaying tone bursts + noise, rhythmic.
    out.clear();
    out.reserve(SOURCE_SAMPLES);
    let mut rng = seed.wrapping_mul(747_796_405).wrapping_add(2_891_336_453);
    let f1 = 55.0 * (1.0 + (seed % 5) as f32);
    let f2 = 220.0 + 37.0 * seed as f32;
    let beat = SAMPLES_PER_BEAT as usize;
    let sub = beat / (1 + (seed as usize % 4));
    for n in 0..SOURCE_SAMPLES {
        rng ^= rng << 13;
        rng ^= rng >> 17;
        rng ^= rng << 5;
        let noise = (rng as f32 / u32::MAX as f32) * 2.0 - 1.0;
        let t = n as f32 / SAMPLE_RATE;
        let ph = (n % sub) as f32 / sub as f32;
        let env = (-ph * 6.0).exp();
        let slow = 0.55 + 0.45 * (t * 0.37 + seed as f32).sin();
        let s = env * (0.6 * (std::f32::consts::TAU * f1 * t).sin() + 0.25 * (std::f32::consts::TAU * f2 * t).sin())
            + 0.12 * noise * slow;
        out.push((s * slow).clamp(-1.0, 1.0));
    }
}

impl PeakPyramid {
    pub fn build() -> Self {
        let mut level_lens = Vec::new();
        let mut len = (SOURCE_SAMPLES as u32).div_ceil(1 << L0_SHIFT);
        loop {
            level_lens.push(len);
            if len <= 1 {
                break;
            }
            len = len.div_ceil(2);
        }
        let mut level_offsets = Vec::new();
        let mut acc = 0u32;
        for &l in &level_lens {
            level_offsets.push(acc);
            acc += l;
        }
        let source_stride = acc;
        let mut data = Vec::with_capacity(source_stride as usize * NUM_SOURCES);
        let mut l0_all = Vec::new();
        let mut samples = Vec::new();
        for src in 0..NUM_SOURCES {
            synth_source(src as u32 + 1, &mut samples);
            // level 0
            let mut lvl: Vec<[f32; 2]> = samples
                .chunks(1 << L0_SHIFT)
                .map(|c| {
                    let mut mn = f32::MAX;
                    let mut mx = f32::MIN;
                    for &s in c {
                        mn = mn.min(s);
                        mx = mx.max(s);
                    }
                    [mn, mx]
                })
                .collect();
            l0_all.push(lvl.clone());
            data.extend_from_slice(&lvl);
            for _ in 1..level_lens.len() {
                lvl = lvl
                    .chunks(2)
                    .map(|c| {
                        let mut r = c[0];
                        if let Some(b) = c.get(1) {
                            r[0] = r[0].min(b[0]);
                            r[1] = r[1].max(b[1]);
                        }
                        r
                    })
                    .collect();
                data.extend_from_slice(&lvl);
            }
        }
        PeakPyramid { data, level_offsets, level_lens, source_stride, l0: l0_all }
    }

    /// Envelope path (SVG) with `points` columns over a sample range, in viewbox
    /// x in 0..points, y in -1..1 (y down => negate amplitude).
    pub fn svg_envelope(&self, src: usize, start: usize, len: usize, points: usize) -> String {
        use std::fmt::Write;
        let l0 = &self.l0[src];
        let per = (len / points).max(1);
        let mut top = Vec::with_capacity(points);
        let mut bot = Vec::with_capacity(points);
        for p in 0..points {
            let s0 = (start + p * per) >> L0_SHIFT;
            let s1 = ((start + (p + 1) * per) >> L0_SHIFT).max(s0 + 1);
            let mut mn = 0f32;
            let mut mx = 0f32;
            for i in s0..s1.min(l0.len()) {
                mn = mn.min(l0[i][0]);
                mx = mx.max(l0[i][1]);
            }
            top.push(-mx * 0.9);
            bot.push(-mn * 0.9);
        }
        let mut s = String::with_capacity(points * 24);
        let _ = write!(s, "M 0 {:.3}", top[0]);
        for (i, y) in top.iter().enumerate().skip(1) {
            let _ = write!(s, " L {} {:.3}", i, y);
        }
        for (i, y) in bot.iter().enumerate().rev() {
            let _ = write!(s, " L {} {:.3}", i, y);
        }
        s.push_str(" Z");
        s
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Clip {
    pub track: u32,
    pub start_beat: f32,
    pub len_beats: f32,
    pub source: u32,
    pub source_offset: f32, // samples
    pub color: [f32; 3],
}

pub const TRACK_COLORS: [[f32; 3]; 8] = [
    [0.184, 0.420, 1.000], // accent #2f6bff
    [0.180, 0.749, 0.443],
    [0.890, 0.773, 0.227],
    [0.898, 0.282, 0.302],
    [0.576, 0.443, 0.976],
    [0.200, 0.780, 0.820],
    [0.949, 0.553, 0.247],
    [0.851, 0.376, 0.690],
];

pub fn build_clips(tracks: u32, clips_per_track: u32) -> Vec<Clip> {
    let mut v = Vec::new();
    let mut rng = 0x1234_5678u32;
    let mut next = || {
        rng ^= rng << 13;
        rng ^= rng >> 17;
        rng ^= rng << 5;
        rng
    };
    let src_beats = SOURCE_SAMPLES as f32 / SAMPLES_PER_BEAT;
    for t in 0..tracks {
        let mut beat = (next() % 4) as f32;
        for _ in 0..clips_per_track {
            let len = 8.0 + (next() % 9) as f32; // 8..16 beats
            let max_off = (src_beats - len).max(0.0);
            let off_beats = (next() % (max_off as u32 + 1)) as f32;
            v.push(Clip {
                track: t,
                start_beat: beat,
                len_beats: len,
                source: next() % NUM_SOURCES as u32,
                source_offset: off_beats * SAMPLES_PER_BEAT,
                color: TRACK_COLORS[(t as usize) % TRACK_COLORS.len()],
            });
            beat += len + (next() % 3) as f32;
        }
    }
    v
}
