//! Waveform peak pyramids built from real engine data (`Engine::peaks`).
//!
//! Level 0 holds one (min, max) pair per [`L0_FRAMES`] frames at the engine
//! rate; each further level halves the count until one pair is left. All
//! pyramids are packed into one linear array uploaded as one texture
//! (`gpu::Renderer::sync_peaks`); the shader reconstructs level offsets from
//! the level-0 length, so the packing here must match `fs_wave`.

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
#[derive(Debug, Clone, Copy)]
pub struct Slot {
    pub base: u32,
    pub len0: u32,
    pub levels: u32,
}

#[derive(Default)]
pub struct PeakStore {
    pub data: Vec<[f32; 2]>,
    slots: HashMap<u64, Slot>,
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
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pyramid_levels() {
        let p = Pyramid::from_level0(vec![[-0.5, 0.5], [-1.0, 0.2], [-0.1, 0.9]]);
        assert_eq!(p.len0, 3);
        assert_eq!(p.levels, 3); // 3 -> 2 -> 1
        assert_eq!(p.data.len(), 3 + 2 + 1);
        assert_eq!(p.data[3], [-1.0, 0.5]);
        assert_eq!(p.data[5], [-1.0, 0.9]);
    }
}
