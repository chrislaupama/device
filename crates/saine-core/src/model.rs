//! Authoritative project model. Lives on the application thread only; the
//! audio engine receives derived, immutable snapshots (see `snapshot`).

use serde::{Deserialize, Serialize};

/// Pulses per quarter note for musical positions.
pub const PPQ: i64 = 960;

/// Musical position/duration in ticks (PPQ = 960).
pub type Ticks = i64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TrackId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ClipId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AssetId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackKind {
    Audio,
    Instrument,
}

/// Track colours observed in the reference (design-system.md). Index into the
/// UI palette; the model stores an index so palettes can be re-themed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ColorIndex(pub u8);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Track {
    pub id: TrackId,
    pub kind: TrackKind,
    pub name: String,
    pub color: ColorIndex,
    /// Linear gain. 1.0 = 0 dB.
    pub gain: f32,
    /// -1.0 (left) .. +1.0 (right).
    pub pan: f32,
    pub mute: bool,
    pub solo: bool,
    pub clips: Vec<Clip>,
}

/// An audio clip referencing a media asset.
///
/// Timeline placement is musical (`start`, ticks) so clips follow tempo
/// changes; the played source region is in source frames at the asset's
/// native sample rate (no time-stretching in milestone 1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Clip {
    pub id: ClipId,
    pub asset: AssetId,
    pub name: String,
    pub start: Ticks,
    pub source_offset: u64,
    pub source_length: u64,
    pub gain: f32,
}

/// External media reference. Media is never copied into the project unless
/// the user chooses "collect"; paths are stored both relative to the project
/// directory (portable) and absolute (fallback).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Asset {
    pub id: AssetId,
    pub relative_path: Option<String>,
    pub absolute_path: String,
    pub sample_rate: f64,
    pub channels: u32,
    pub frames: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TimeSignature {
    pub numerator: u8,
    pub denominator: u8,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Transport {
    pub bpm: f64,
    pub time_signature: TimeSignature,
    pub loop_enabled: bool,
    pub loop_start: Ticks,
    pub loop_end: Ticks,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub name: String,
    pub transport: Transport,
    pub master_gain: f32,
    pub tracks: Vec<Track>,
    pub assets: Vec<Asset>,
    /// Monotonic id allocator shared by tracks, clips and assets.
    pub next_id: u64,
}

impl Default for Project {
    /// Mirrors the reference default project: one Audio and one Instrument
    /// track, 120 BPM, 4/4, loop region bars 1–9 (observed, ui-inventory.md).
    fn default() -> Self {
        let mut p = Project {
            name: "Untitled".into(),
            transport: Transport {
                bpm: 120.0,
                time_signature: TimeSignature { numerator: 4, denominator: 4 },
                loop_enabled: false,
                loop_start: 0,
                loop_end: 8 * 4 * PPQ,
            },
            master_gain: 1.0,
            tracks: Vec::new(),
            assets: Vec::new(),
            next_id: 1,
        };
        let a = p.alloc_id();
        p.tracks.push(Track::new(TrackId(a), TrackKind::Audio, "Audio", ColorIndex(0)));
        let b = p.alloc_id();
        p.tracks.push(Track::new(TrackId(b), TrackKind::Instrument, "Instrument", ColorIndex(1)));
        p
    }
}

impl Track {
    pub fn new(id: TrackId, kind: TrackKind, name: &str, color: ColorIndex) -> Self {
        Track { id, kind, name: name.into(), color, gain: 1.0, pan: 0.0, mute: false, solo: false, clips: Vec::new() }
    }
}

impl Project {
    pub fn empty(name: &str) -> Self {
        let mut p = Project::default();
        p.name = name.into();
        p.tracks.clear();
        p
    }

    pub fn alloc_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    pub fn track(&self, id: TrackId) -> Option<&Track> {
        self.tracks.iter().find(|t| t.id == id)
    }

    pub fn track_mut(&mut self, id: TrackId) -> Option<&mut Track> {
        self.tracks.iter_mut().find(|t| t.id == id)
    }

    pub fn track_index(&self, id: TrackId) -> Option<usize> {
        self.tracks.iter().position(|t| t.id == id)
    }

    pub fn asset(&self, id: AssetId) -> Option<&Asset> {
        self.assets.iter().find(|a| a.id == id)
    }

    /// Locate a clip: (track index, clip index).
    pub fn find_clip(&self, id: ClipId) -> Option<(usize, usize)> {
        self.tracks.iter().enumerate().find_map(|(ti, t)| t.clips.iter().position(|c| c.id == id).map(|ci| (ti, ci)))
    }

    pub fn clip(&self, id: ClipId) -> Option<&Clip> {
        self.find_clip(id).map(|(t, c)| &self.tracks[t].clips[c])
    }

    pub fn ticks_per_bar(&self) -> Ticks {
        let ts = self.transport.time_signature;
        PPQ * 4 * ts.numerator as i64 / ts.denominator as i64
    }

    /// Ticks → seconds at the (currently constant) project tempo.
    pub fn ticks_to_seconds(&self, t: Ticks) -> f64 {
        t as f64 / PPQ as f64 * 60.0 / self.transport.bpm
    }

    pub fn seconds_to_ticks(&self, s: f64) -> Ticks {
        (s * self.transport.bpm / 60.0 * PPQ as f64).round() as Ticks
    }

    /// Ticks → frames at `rate` (rounded to nearest frame).
    pub fn ticks_to_frames(&self, t: Ticks, rate: f64) -> i64 {
        (self.ticks_to_seconds(t) * rate).round() as i64
    }

    pub fn frames_to_ticks(&self, frames: i64, rate: f64) -> Ticks {
        self.seconds_to_ticks(frames as f64 / rate)
    }

    /// Duration of a clip in ticks at the current tempo (audio clips keep
    /// their real-time duration when the tempo changes in milestone 1).
    pub fn clip_length_ticks(&self, clip: &Clip) -> Ticks {
        let rate = self.asset(clip.asset).map(|a| a.sample_rate).unwrap_or(48_000.0);
        self.seconds_to_ticks(clip.source_length as f64 / rate)
    }
}

/// Linear gain ↔ decibels helpers used by UI and AI commands.
pub fn db_to_gain(db: f32) -> f32 {
    if db <= -120.0 { 0.0 } else { 10f32.powf(db / 20.0) }
}

pub fn gain_to_db(g: f32) -> f32 {
    if g <= 1e-6 { f32::NEG_INFINITY } else { 20.0 * g.log10() }
}
