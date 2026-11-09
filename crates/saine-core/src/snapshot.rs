//! Derivation of the immutable session description sent to the audio engine.
//!
//! Pure data (no FFI) so it is testable without the engine. The engine crate
//! converts it into the C ABI structs (`se_session_desc`). All positions are
//! converted from musical time to frames at the engine sample rate here, so
//! the real-time thread never deals with tempo math.

use crate::device::Device;
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

/// A note as absolute engine frames: [start, end) with the clip boundary
/// already applied (notes crossing the clip end are cut; notes starting at
/// or after it are dropped).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NoteSnapshot {
    pub start: i64,
    pub end: i64,
    pub pitch: u8,
    pub velocity: u8,
}

/// An enabled automation lane with points converted to engine frames.
#[derive(Debug, Clone, PartialEq)]
pub struct LaneSnapshot {
    pub target: crate::automation::AutomationTarget,
    /// (frame, plain value, curve), sorted by frame.
    pub points: Vec<(i64, f32, f32)>,
}

fn lanes(project: &Project, lanes: &[crate::automation::AutomationLane], rate: f64) -> Vec<LaneSnapshot> {
    lanes
        .iter()
        .filter(|l| l.enabled && !l.points.is_empty())
        .map(|l| LaneSnapshot { target: l.target.clone(), points: l.points.iter().map(|q| (project.ticks_to_frames(q.time, rate), q.value, q.curve)).collect() })
        .collect()
}

#[derive(Debug, Clone, PartialEq)]
pub struct TrackSnapshot {
    pub track_id: u64,
    pub gain: f32,
    pub pan: f32,
    pub mute: bool,
    pub solo: bool,
    pub clips: Vec<ClipSnapshot>,
    /// True for instrument tracks (render `notes` through the built-in instrument).
    pub instrument: bool,
    /// All playable notes of the track's MIDI clips, sorted by start frame.
    pub notes: Vec<NoteSnapshot>,
    /// Enabled, non-empty automation lanes.
    pub automation: Vec<LaneSnapshot>,
    /// Recording/monitoring state (audio tracks).
    pub arm: bool,
    pub monitor: bool,
    pub input: Option<crate::model::InputSource>,
    /// Insert effects in processing order.
    pub devices: Vec<crate::device::Device>,
    /// Instrument plugin (replaces the built-in synth when present).
    pub instrument_plugin: Option<crate::device::Device>,
    /// Built-in instrument parameters (instrument tracks only).
    pub synth: Option<crate::instrument::SynthParams>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SessionSnapshot {
    pub tracks: Vec<TrackSnapshot>,
    pub master_gain: f32,
    /// Master volume automation (enabled, non-empty lanes).
    pub master_automation: Vec<LaneSnapshot>,
    /// Engine assets of the audio files built-in devices play
    /// (`Device::samples`); samples whose asset isn't loaded are left out.
    pub device_assets: HashMap<AssetId, u64>,
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
            instrument: t.kind == crate::model::TrackKind::Instrument,
            synth: (t.kind == crate::model::TrackKind::Instrument).then(|| t.synth.clone()),
            // A chain head's switch bypasses its members too.
            devices: t.devices.iter().enumerate().map(|(i, d)| Device { enabled: crate::modulation::effectively_enabled(&t.devices, i), ..d.clone() }).collect(),
            instrument_plugin: if t.kind == crate::model::TrackKind::Instrument { t.instrument_plugin.clone() } else { None },
            automation: lanes(project, &t.automation, engine_rate),
            arm: t.arm,
            monitor: t.monitor,
            input: t.input,
            notes: {
                let mut v: Vec<NoteSnapshot> = t
                    .midi_clips
                    .iter()
                    .flat_map(|c| {
                        c.notes.iter().filter(|n| n.start < c.length).map(move |n| {
                            let s = c.start + n.start;
                            let e = c.start + (n.start + n.length).min(c.length);
                            (s, e, n.pitch, n.velocity)
                        })
                    })
                    .map(|(s, e, pitch, velocity)| NoteSnapshot {
                        start: project.ticks_to_frames(s, engine_rate),
                        end: project.ticks_to_frames(e, engine_rate),
                        pitch,
                        velocity,
                    })
                    .filter(|n| n.end > n.start)
                    .collect();
                v.sort_by_key(|n| (n.start, n.pitch));
                v
            },
        })
        .collect();
    let master_automation = lanes(project, &project.master_automation, engine_rate);
    let device_assets = project
        .tracks
        .iter()
        .flat_map(|t| t.devices.iter().chain(t.instrument_plugin.iter()))
        .flat_map(|d| d.samples.iter())
        .filter_map(|s| Some((s.asset, *engine_assets.get(&s.asset)?)))
        .collect();
    (SessionSnapshot { tracks, master_gain: project.master_gain, master_automation, device_assets }, skipped)
}
