//! Authoritative project model. Lives on the application thread only; the
//! audio engine receives derived, immutable snapshots (see `snapshot`).

use crate::automation::AutomationLane;
use crate::device::{Device, DeviceId};
use crate::instrument::SynthParams;
use crate::midi::MidiClip;
use crate::warp::ClipWarp;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SectionId(pub u64);

/// A named, coloured arrangement range. Like the reference's sections, they tile the
/// timeline from bar 1 in list order: a section starts where the previous one ends.
/// Labels only: editing sections never moves clips.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Section {
    pub id: SectionId,
    pub name: String,
    /// 0xRRGGBB (the reference's section palette, before display tinting).
    pub color: u32,
    pub length: Ticks,
}

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

impl ColorIndex {
    /// Colour of a new project's first track: the reference's pink (palette index 4). Later
    /// tracks follow the cycle from there (orange, then teal, ...).
    pub const FIRST: ColorIndex = ColorIndex(4);
}

/// Which device input feeds a track (reference "In" selector: "None",
/// "Input 1 (Mono)", ...). Channels are 0-based device input channels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputSource {
    /// First (left or mono) device input channel, 0-based.
    pub channel: u16,
    /// Use `channel` and `channel + 1` as a stereo pair.
    pub stereo: bool,
}

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
    /// Armed for recording (reference "R").
    #[serde(default)]
    pub arm: bool,
    /// Input monitoring on (reference "I"): the track's input is heard
    /// through its chain while armed or monitoring.
    #[serde(default)]
    pub monitor: bool,
    /// Input routing; None = "None".
    #[serde(default)]
    pub input: Option<InputSource>,
    pub clips: Vec<Clip>,
    /// MIDI clips (instrument tracks only). Defaulted so schema-v1 files
    /// without the field still load.
    #[serde(default)]
    pub midi_clips: Vec<MidiClip>,
    /// Built-in instrument settings (used by instrument tracks; defaulted for
    /// older files and harmlessly present on audio tracks).
    #[serde(default)]
    pub synth: SynthParams,
    /// Insert effects, processed in order after the track's source.
    #[serde(default)]
    pub devices: Vec<Device>,
    /// Instrument plugin replacing the built-in synth (instrument tracks).
    #[serde(default)]
    pub instrument_plugin: Option<Device>,
    /// Automation lanes (at most one per target).
    #[serde(default)]
    pub automation: Vec<AutomationLane>,
    /// Row height as a multiple of the default 88 px row ([Track::HEIGHT_RANGE]).
    #[serde(default = "unit_height", skip_serializing_if = "is_unit_height")]
    pub height: f32,
}

fn unit_height() -> f32 {
    1.0
}

fn is_unit_height(h: &f32) -> bool {
    *h == 1.0
}

/// Where a device sits on its track.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceSlot {
    Insert(usize),
    Instrument,
}

/// An audio clip referencing a media asset.
///
/// Timeline placement is musical (`start`, ticks) so clips follow tempo
/// changes; the played source region is in source frames at the asset's
/// native sample rate. With warp on, the region follows the project tempo
/// (see [`crate::warp`]); with warp off it plays 1:1.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Clip {
    pub id: ClipId,
    pub asset: AssetId,
    pub name: String,
    pub start: Ticks,
    pub source_offset: u64,
    pub source_length: u64,
    pub gain: f32,
    #[serde(default, skip_serializing_if = "ClipWarp::is_default")]
    pub warp: ClipWarp,
    /// Disabled clips stay on the timeline but make no sound (the Disable menu entry).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub disabled: bool,
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
    /// Project key (None = not set, the reference's "—"). Devices whose Key
    /// Follow is on use it (builtin::engine_device).
    #[serde(default)]
    pub key: Option<ProjectKey>,
}

/// A project key: tonic pitch class and mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectKey {
    /// Pitch class of the tonic, 0 = C .. 11 = B.
    pub root: u8,
    pub minor: bool,
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
    /// Master bus automation (volume only).
    #[serde(default)]
    pub master_automation: Vec<AutomationLane>,
    /// Bottom panel height (window points) per panel kind ("devices", "clip", "mixer") after the
    /// user dragged the splitter; absent kinds use the default height. Not an undo step.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub bottom_panel_heights: std::collections::BTreeMap<String, f32>,
    /// Arrangement sections in timeline order (see [Section]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sections: Vec<Section>,
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
                key: None,
            },
            master_gain: 1.0,
            tracks: Vec::new(),
            assets: Vec::new(),
            next_id: 1,
            master_automation: Vec::new(),
            bottom_panel_heights: Default::default(),
            sections: Vec::new(),
        };
        let a = p.alloc_id();
        p.tracks.push(Track::new(TrackId(a), TrackKind::Audio, "Audio", ColorIndex::FIRST));
        let b = p.alloc_id();
        p.tracks.push(Track::new(TrackId(b), TrackKind::Instrument, "Instrument", ColorIndex(ColorIndex::FIRST.0 + 1)));
        crate::builtin::synth_v2::upgrade_project(&mut p);
        p
    }
}

impl Track {
    pub fn new(id: TrackId, kind: TrackKind, name: &str, color: ColorIndex) -> Self {
        Track { id, kind, name: name.into(), color, gain: 1.0, pan: 0.0, mute: false, solo: false, arm: false, monitor: false, input: None, clips: Vec::new(), midi_clips: Vec::new(), synth: SynthParams::default(), devices: Vec::new(), instrument_plugin: None, automation: Vec::new(), height: 1.0 }
    }

    /// Row height scale limits (the reference's per-track height scale).
    pub const HEIGHT_RANGE: (f32, f32) = (0.25, 6.0);
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

    pub fn section_index(&self, id: SectionId) -> Option<usize> {
        self.sections.iter().position(|s| s.id == id)
    }

    /// Start of section `i` (the sum of the lengths before it).
    pub fn section_start(&self, i: usize) -> Ticks {
        self.sections[..i.min(self.sections.len())].iter().map(|s| s.length).sum()
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

    /// Locate a MIDI clip: (track index, clip index).
    pub fn find_midi_clip(&self, id: ClipId) -> Option<(usize, usize)> {
        self.tracks.iter().enumerate().find_map(|(ti, t)| t.midi_clips.iter().position(|c| c.id == id).map(|ci| (ti, ci)))
    }

    pub fn midi_clip(&self, id: ClipId) -> Option<&MidiClip> {
        self.find_midi_clip(id).map(|(t, c)| &self.tracks[t].midi_clips[c])
    }

    /// Locate a device: (track index, slot).
    pub fn find_device(&self, id: DeviceId) -> Option<(usize, DeviceSlot)> {
        self.tracks.iter().enumerate().find_map(|(ti, t)| {
            if t.instrument_plugin.as_ref().is_some_and(|d| d.id == id) {
                return Some((ti, DeviceSlot::Instrument));
            }
            t.devices.iter().position(|d| d.id == id).map(|i| (ti, DeviceSlot::Insert(i)))
        })
    }

    pub fn device(&self, id: DeviceId) -> Option<&Device> {
        self.find_device(id).map(|(ti, slot)| match slot {
            DeviceSlot::Insert(i) => &self.tracks[ti].devices[i],
            DeviceSlot::Instrument => self.tracks[ti].instrument_plugin.as_ref().expect("found"),
        })
    }

    /// True if any clip (audio or MIDI) uses this id.
    pub fn clip_id_in_use(&self, id: ClipId) -> bool {
        self.find_clip(id).is_some() || self.find_midi_clip(id).is_some()
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

    /// Duration of a clip in ticks at the current tempo: warped clips cover
    /// their beats, the others keep their real-time duration.
    pub fn clip_length_ticks(&self, clip: &Clip) -> Ticks {
        if clip.warp.enabled {
            return crate::warp::beats_to_ticks(self.warped_clip_beats(clip));
        }
        let rate = self.asset(clip.asset).map(|a| a.sample_rate).unwrap_or(48_000.0);
        self.seconds_to_ticks(clip.source_length as f64 / rate / clip.warp.varispeed())
    }
}

/// Linear gain ↔ decibels helpers used by UI and AI commands.
pub fn db_to_gain(db: f32) -> f32 {
    if db <= -120.0 { 0.0 } else { 10f32.powf(db / 20.0) }
}

pub fn gain_to_db(g: f32) -> f32 {
    if g <= 1e-6 { f32::NEG_INFINITY } else { 20.0 * g.log10() }
}
