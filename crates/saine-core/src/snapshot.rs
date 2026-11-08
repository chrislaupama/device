//! Derivation of the immutable session description sent to the audio engine.
//!
//! Pure data (no FFI) so it is testable without the engine. The engine crate
//! converts it into the C ABI structs (`se_session_desc`). All positions are
//! converted from musical time to frames at the engine sample rate here, so
//! the real-time thread never deals with tempo math.

use crate::model::{AssetId, Project};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
pub struct ClipSnapshot {
    pub clip_id: u64,
    pub engine_asset: u64,
    pub timeline_start: i64,
    pub source_offset: i64,
    pub length: i64,
    pub gain: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TrackSnapshot {
    pub track_id: u64,
    pub gain: f32,
    pub pan: f32,
    pub mute: bool,
    pub solo: bool,
    pub clips: Vec<ClipSnapshot>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SessionSnapshot {
    pub tracks: Vec<TrackSnapshot>,
    pub master_gain: f32,
}

/// Build a snapshot. `engine_assets` maps model assets to loaded engine
/// assets; clips whose asset isn't loaded (missing media, still importing)
/// are skipped and reported in the second return value.
pub fn build(project: &Project, engine_rate: f64, engine_assets: &HashMap<AssetId, u64>) -> (SessionSnapshot, Vec<u64>) {
    let mut skipped = Vec::new();
    let tracks = project
        .tracks
        .iter()
        .map(|t| TrackSnapshot {
            track_id: t.id.0,
            gain: t.gain,
            pan: t.pan,
            mute: t.mute,
            solo: t.solo,
            clips: t
                .clips
                .iter()
                .filter_map(|c| {
                    let (Some(&ea), Some(asset)) = (engine_assets.get(&c.asset), project.asset(c.asset)) else {
                        skipped.push(c.id.0);
                        return None;
                    };
                    let ratio = engine_rate / asset.sample_rate;
                    Some(ClipSnapshot {
                        clip_id: c.id.0,
                        engine_asset: ea,
                        timeline_start: project.ticks_to_frames(c.start, engine_rate),
                        source_offset: (c.source_offset as f64 * ratio).round() as i64,
                        length: (c.source_length as f64 * ratio).round() as i64,
                        gain: c.gain,
                    })
                })
                .collect(),
        })
        .collect();
    (SessionSnapshot { tracks, master_gain: project.master_gain }, skipped)
}
