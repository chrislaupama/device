//! Structured edit commands.
//!
//! Every mutation of the project goes through [`Command::apply`], which validates
//! its arguments, mutates the model and returns the exact inverse command.
//! The same commands are used by the UI, keyboard shortcuts, scripting, tests
//! and the AI assistant (they are serde-serialisable, so an LLM tool call can
//! be deserialised straight into a `Command` and validated before execution).

use crate::automation::{AutomationLane, AutomationTarget};
use crate::device::{Device, DeviceId, PluginKind};
use crate::instrument::SynthParams;
use crate::midi::{self, MidiClip, Note};
use crate::model::*;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error, PartialEq)]
pub enum EditError {
    #[error("track {0:?} not found")]
    TrackNotFound(TrackId),
    #[error("clip {0:?} not found")]
    ClipNotFound(ClipId),
    #[error("asset {0:?} not found")]
    AssetNotFound(AssetId),
    #[error("invalid value for {field}: {reason}")]
    InvalidValue { field: &'static str, reason: String },
    #[error("id {0} already in use")]
    DuplicateId(u64),
}

fn invalid(field: &'static str, reason: impl Into<String>) -> EditError {
    EditError::InvalidValue { field, reason: reason.into() }
}

/// Limits enforced by validation.
pub const MAX_GAIN: f32 = 3.981_071_7; // +12 dB
pub const MIN_BPM: f64 = 20.0;
pub const MAX_BPM: f64 = 999.0;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Command {
    /// Insert a track. If `track.id` is 0 a fresh id is allocated.
    AddTrack { index: Option<usize>, track: Track },
    RemoveTrack { track: TrackId },
    RenameTrack { track: TrackId, name: String },
    SetTrackGain { track: TrackId, gain: f32 },
    SetTrackPan { track: TrackId, pan: f32 },
    SetTrackMute { track: TrackId, mute: bool },
    SetTrackSolo { track: TrackId, solo: bool },
    SetMasterGain { gain: f32 },
    /// Register an imported media asset (id 0 → allocate).
    AddAsset { asset: Asset },
    RemoveAsset { asset: AssetId },
    /// Insert a clip on a track (id 0 → allocate).
    AddClip { track: TrackId, clip: Clip },
    RemoveClip { clip: ClipId },
    /// Move a clip in time and/or to another track.
    MoveClip { clip: ClipId, track: TrackId, start: Ticks },
    /// Set the visible region of a clip (left/right trim).
    TrimClip { clip: ClipId, start: Ticks, source_offset: u64, source_length: u64 },
    SetClipGain { clip: ClipId, gain: f32 },
    SetTempo { bpm: f64 },
    SetTimeSignature { numerator: u8, denominator: u8 },
    SetLoop { enabled: bool, start: Ticks, end: Ticks },
    /// Insert a MIDI clip on an instrument track (id 0 → allocate).
    AddMidiClip { track: TrackId, clip: MidiClip },
    RemoveMidiClip { clip: ClipId },
    /// Move a MIDI clip in time and/or to another instrument track.
    MoveMidiClip { clip: ClipId, track: TrackId, start: Ticks },
    /// Change a MIDI clip's start and length (notes keep clip-relative times).
    ResizeMidiClip { clip: ClipId, start: Ticks, length: Ticks },
    /// Replace all notes of a MIDI clip (draw, erase, move, resize, velocity,
    /// quantize, transpose... each editor gesture is one of these).
    SetMidiNotes { clip: ClipId, notes: Vec<Note> },
    /// Replace an instrument track's synth parameters (knob drags coalesce
    /// per track; loading a preset is one step).
    SetSynthParams { track: TrackId, params: SynthParams },
    SetTrackArm { track: TrackId, arm: bool },
    SetTrackMonitor { track: TrackId, monitor: bool },
    SetTrackInput { track: TrackId, input: Option<InputSource> },
    /// Insert an effect plugin into a track's chain (id 0 → allocate).
    AddDevice { track: TrackId, index: Option<usize>, device: Device },
    RemoveDevice { device: DeviceId },
    /// Reorder an insert, possibly onto another track.
    MoveDevice { device: DeviceId, track: TrackId, index: usize },
    /// Bypass toggle.
    SetDeviceEnabled { device: DeviceId, enabled: bool },
    /// Set or clear an instrument track's instrument plugin (None = built-in synth).
    SetInstrumentPlugin { track: TrackId, device: Option<Device> },
    /// Set one parameter of a built-in device (plain value in the
    /// parameter's unit; knob drags coalesce per parameter).
    SetDeviceParam { device: DeviceId, param: String, value: f32 },
    /// Set several parameters of a built-in device at once (preset load, one
    /// undo step). `preset` becomes the name shown in the card header.
    SetDeviceParams { device: DeviceId, params: std::collections::BTreeMap<String, f32>, preset: Option<String> },
    /// Pick (or clear) the sidechain source track of a built-in device.
    SetDeviceSidechain { device: DeviceId, source: Option<TrackId> },
    /// Create, replace (points, enabled) or delete (`lane: None`) the lane for
    /// `target` on a track, or on the master when `track` is None.
    SetAutomationLane { track: Option<TrackId>, target: AutomationTarget, lane: Option<AutomationLane> },
    /// Several commands applied atomically (all or nothing).
    Batch { label: String, commands: Vec<Command> },
}

impl Command {
    /// Human-readable label used for "Undo ⟨label⟩" (reference: Edit menu
    /// titles the action, e.g. "Undo Shortcut Change").
    pub fn label(&self) -> String {
        match self {
            Command::AddTrack { track, .. } => format!("Add {} Track", kind_name(track.kind)),
            Command::RemoveTrack { .. } => "Delete Track".into(),
            Command::RenameTrack { .. } => "Rename Track".into(),
            Command::SetTrackGain { .. } => "Track Volume Change".into(),
            Command::SetTrackPan { .. } => "Track Pan Change".into(),
            Command::SetTrackMute { .. } => "Mute Change".into(),
            Command::SetTrackSolo { .. } => "Solo Change".into(),
            Command::SetMasterGain { .. } => "Master Volume Change".into(),
            Command::AddAsset { .. } => "Import Audio".into(),
            Command::RemoveAsset { .. } => "Remove Audio".into(),
            Command::AddClip { .. } => "Add Clip".into(),
            Command::RemoveClip { .. } => "Delete Clip".into(),
            Command::MoveClip { .. } => "Move Clip".into(),
            Command::TrimClip { .. } => "Trim Clip".into(),
            Command::SetClipGain { .. } => "Clip Gain Change".into(),
            Command::SetTempo { .. } => "Tempo Change".into(),
            Command::SetTimeSignature { .. } => "Time Signature Change".into(),
            Command::SetLoop { .. } => "Loop Change".into(),
            Command::AddMidiClip { .. } => "Insert MIDI Clip".into(),
            Command::RemoveMidiClip { .. } => "Delete Clip".into(),
            Command::MoveMidiClip { .. } => "Move Clip".into(),
            Command::ResizeMidiClip { .. } => "Resize Clip".into(),
            Command::SetMidiNotes { .. } => "Edit Notes".into(),
            Command::SetSynthParams { .. } => "Synth Parameter Change".into(),
            Command::SetTrackArm { arm, .. } => (if *arm { "Arm Track" } else { "Disarm Track" }).into(),
            Command::SetTrackMonitor { .. } => "Input Monitoring Change".into(),
            Command::SetTrackInput { .. } => "Input Change".into(),
            Command::AddDevice { device, .. } => format!("Add {}", device.plugin.name),
            Command::RemoveDevice { .. } => "Remove Device".into(),
            Command::MoveDevice { .. } => "Move Device".into(),
            Command::SetDeviceEnabled { enabled, .. } => (if *enabled { "Enable Device" } else { "Bypass Device" }).into(),
            Command::SetDeviceParam { .. } => "Device Parameter Change".into(),
            Command::SetDeviceParams { preset: Some(name), .. } => format!("Load Preset {name}"),
            Command::SetDeviceParams { preset: None, .. } => "Device Parameters Change".into(),
            Command::SetDeviceSidechain { .. } => "Sidechain Change".into(),
            Command::SetInstrumentPlugin { device, .. } => match device {
                Some(d) => format!("Load {}", d.plugin.name),
                None => "Use Built-in Synth".into(),
            },
            Command::SetAutomationLane { lane, target, .. } => match lane {
                None => format!("Delete {} Automation", target.labels().1),
                Some(_) => format!("Edit {} Automation", target.labels().1),
            },
            Command::Batch { label, .. } => label.clone(),
        }
    }

    /// Key used to coalesce rapid successive edits (e.g. a fader drag) into a
    /// single undo step. `None` means never coalesce.
    pub fn coalesce_key(&self) -> Option<String> {
        match self {
            Command::SetTrackGain { track, .. } => Some(format!("gain:{}", track.0)),
            Command::SetTrackPan { track, .. } => Some(format!("pan:{}", track.0)),
            Command::SetMasterGain { .. } => Some("master_gain".into()),
            Command::SetClipGain { clip, .. } => Some(format!("clipgain:{}", clip.0)),
            Command::SetTempo { .. } => Some("tempo".into()),
            Command::SetSynthParams { track, params } => Some(format!("synth:{}:{}", track.0, params.preset)),
            Command::SetDeviceParam { device, param, .. } => Some(format!("devparam:{}:{param}", device.0)),
            _ => None,
        }
    }

    /// Validate and apply; returns the inverse command. On error the project is unchanged.
    pub fn apply(self, p: &mut Project) -> Result<Command, EditError> {
        match self {
            Command::AddTrack { index, mut track } => {
                if track.id.0 == 0 {
                    track.id = TrackId(p.alloc_id());
                } else if p.track(track.id).is_some() {
                    return Err(EditError::DuplicateId(track.id.0));
                } else {
                    p.next_id = p.next_id.max(track.id.0 + 1);
                }
                validate_gain("gain", track.gain)?;
                validate_pan(track.pan)?;
                for c in &track.clips {
                    validate_clip(p, c)?;
                }
                let idx = index.unwrap_or(p.tracks.len());
                if idx > p.tracks.len() {
                    return Err(invalid("index", format!("{idx} > {}", p.tracks.len())));
                }
                let id = track.id;
                p.tracks.insert(idx, track);
                Ok(Command::RemoveTrack { track: id })
            }
            Command::RemoveTrack { track } => {
                let idx = p.track_index(track).ok_or(EditError::TrackNotFound(track))?;
                let t = p.tracks.remove(idx);
                Ok(Command::AddTrack { index: Some(idx), track: t })
            }
            Command::RenameTrack { track, name } => {
                let name = name.trim().to_string();
                if name.is_empty() || name.chars().count() > 128 {
                    return Err(invalid("name", "must be 1..=128 characters"));
                }
                let t = p.track_mut(track).ok_or(EditError::TrackNotFound(track))?;
                let old = std::mem::replace(&mut t.name, name);
                Ok(Command::RenameTrack { track, name: old })
            }
            Command::SetTrackGain { track, gain } => {
                validate_gain("gain", gain)?;
                let t = p.track_mut(track).ok_or(EditError::TrackNotFound(track))?;
                let old = std::mem::replace(&mut t.gain, gain);
                Ok(Command::SetTrackGain { track, gain: old })
            }
            Command::SetTrackPan { track, pan } => {
                validate_pan(pan)?;
                let t = p.track_mut(track).ok_or(EditError::TrackNotFound(track))?;
                let old = std::mem::replace(&mut t.pan, pan);
                Ok(Command::SetTrackPan { track, pan: old })
            }
            Command::SetTrackMute { track, mute } => {
                let t = p.track_mut(track).ok_or(EditError::TrackNotFound(track))?;
                let old = std::mem::replace(&mut t.mute, mute);
                Ok(Command::SetTrackMute { track, mute: old })
            }
            Command::SetTrackSolo { track, solo } => {
                let t = p.track_mut(track).ok_or(EditError::TrackNotFound(track))?;
                let old = std::mem::replace(&mut t.solo, solo);
                Ok(Command::SetTrackSolo { track, solo: old })
            }
            Command::SetMasterGain { gain } => {
                validate_gain("master_gain", gain)?;
                let old = std::mem::replace(&mut p.master_gain, gain);
                Ok(Command::SetMasterGain { gain: old })
            }
            Command::AddAsset { mut asset } => {
                if asset.id.0 == 0 {
                    asset.id = AssetId(p.alloc_id());
                } else if p.asset(asset.id).is_some() {
                    return Err(EditError::DuplicateId(asset.id.0));
                } else {
                    p.next_id = p.next_id.max(asset.id.0 + 1);
                }
                if !(asset.sample_rate > 0.0) || asset.channels == 0 {
                    return Err(invalid("asset", "sample_rate and channels must be positive"));
                }
                let id = asset.id;
                p.assets.push(asset);
                Ok(Command::RemoveAsset { asset: id })
            }
            Command::RemoveAsset { asset } => {
                if p.tracks.iter().flat_map(|t| &t.clips).any(|c| c.asset == asset) {
                    return Err(invalid("asset", "still referenced by clips"));
                }
                let idx = p.assets.iter().position(|a| a.id == asset).ok_or(EditError::AssetNotFound(asset))?;
                let a = p.assets.remove(idx);
                Ok(Command::AddAsset { asset: a })
            }
            Command::AddClip { track, mut clip } => {
                if p.track(track).is_none() {
                    return Err(EditError::TrackNotFound(track));
                }
                if clip.id.0 == 0 {
                    clip.id = ClipId(p.alloc_id());
                } else if p.clip_id_in_use(clip.id) {
                    return Err(EditError::DuplicateId(clip.id.0));
                } else {
                    p.next_id = p.next_id.max(clip.id.0 + 1);
                }
                validate_clip(p, &clip)?;
                let id = clip.id;
                let t = p.track_mut(track).expect("checked");
                t.clips.push(clip);
                t.clips.sort_by_key(|c| (c.start, c.id));
                Ok(Command::RemoveClip { clip: id })
            }
            Command::RemoveClip { clip } => {
                let (ti, ci) = p.find_clip(clip).ok_or(EditError::ClipNotFound(clip))?;
                let track = p.tracks[ti].id;
                let c = p.tracks[ti].clips.remove(ci);
                Ok(Command::AddClip { track, clip: c })
            }
            Command::MoveClip { clip, track, start } => {
                if start < 0 {
                    return Err(invalid("start", "must be >= 0"));
                }
                let (ti, ci) = p.find_clip(clip).ok_or(EditError::ClipNotFound(clip))?;
                let dest = p.track_index(track).ok_or(EditError::TrackNotFound(track))?;
                if p.tracks[dest].kind != p.tracks[ti].kind {
                    return Err(invalid("track", "clip cannot move between audio and instrument tracks"));
                }
                let old_track = p.tracks[ti].id;
                let mut c = p.tracks[ti].clips.remove(ci);
                let old_start = std::mem::replace(&mut c.start, start);
                p.tracks[dest].clips.push(c);
                p.tracks[dest].clips.sort_by_key(|c| (c.start, c.id));
                Ok(Command::MoveClip { clip, track: old_track, start: old_start })
            }
            Command::TrimClip { clip, start, source_offset, source_length } => {
                let (ti, ci) = p.find_clip(clip).ok_or(EditError::ClipNotFound(clip))?;
                let mut probe = p.tracks[ti].clips[ci].clone();
                probe.start = start;
                probe.source_offset = source_offset;
                probe.source_length = source_length;
                validate_clip(p, &probe)?;
                let c = &mut p.tracks[ti].clips[ci];
                let inv = Command::TrimClip { clip, start: c.start, source_offset: c.source_offset, source_length: c.source_length };
                *c = probe;
                p.tracks[ti].clips.sort_by_key(|c| (c.start, c.id));
                Ok(inv)
            }
            Command::SetClipGain { clip, gain } => {
                validate_gain("clip_gain", gain)?;
                let (ti, ci) = p.find_clip(clip).ok_or(EditError::ClipNotFound(clip))?;
                let old = std::mem::replace(&mut p.tracks[ti].clips[ci].gain, gain);
                Ok(Command::SetClipGain { clip, gain: old })
            }
            Command::SetTempo { bpm } => {
                if !(MIN_BPM..=MAX_BPM).contains(&bpm) {
                    return Err(invalid("bpm", format!("must be within {MIN_BPM}..={MAX_BPM}")));
                }
                let old = std::mem::replace(&mut p.transport.bpm, bpm);
                Ok(Command::SetTempo { bpm: old })
            }
            Command::SetTimeSignature { numerator, denominator } => {
                if numerator == 0 || numerator > 32 || ![1, 2, 4, 8, 16, 32].contains(&denominator) {
                    return Err(invalid("time_signature", "numerator 1..=32, denominator power of two ≤ 32"));
                }
                let ts = &mut p.transport.time_signature;
                let old = Command::SetTimeSignature { numerator: ts.numerator, denominator: ts.denominator };
                *ts = TimeSignature { numerator, denominator };
                Ok(old)
            }
            Command::SetLoop { enabled, start, end } => {
                if start < 0 || end <= start {
                    return Err(invalid("loop", "requires 0 <= start < end"));
                }
                let t = &mut p.transport;
                let old = Command::SetLoop { enabled: t.loop_enabled, start: t.loop_start, end: t.loop_end };
                t.loop_enabled = enabled;
                t.loop_start = start;
                t.loop_end = end;
                Ok(old)
            }
            Command::AddMidiClip { track, mut clip } => {
                let ti = p.track_index(track).ok_or(EditError::TrackNotFound(track))?;
                if p.tracks[ti].kind != TrackKind::Instrument {
                    return Err(invalid("track", "MIDI clips belong on instrument tracks"));
                }
                if clip.id.0 == 0 {
                    clip.id = ClipId(p.alloc_id());
                } else if p.clip_id_in_use(clip.id) {
                    return Err(EditError::DuplicateId(clip.id.0));
                } else {
                    p.next_id = p.next_id.max(clip.id.0 + 1);
                }
                validate_midi_clip(&clip)?;
                midi::sort_notes(&mut clip.notes);
                let id = clip.id;
                let t = &mut p.tracks[ti];
                t.midi_clips.push(clip);
                t.midi_clips.sort_by_key(|c| (c.start, c.id));
                Ok(Command::RemoveMidiClip { clip: id })
            }
            Command::RemoveMidiClip { clip } => {
                let (ti, ci) = p.find_midi_clip(clip).ok_or(EditError::ClipNotFound(clip))?;
                let track = p.tracks[ti].id;
                let c = p.tracks[ti].midi_clips.remove(ci);
                Ok(Command::AddMidiClip { track, clip: c })
            }
            Command::MoveMidiClip { clip, track, start } => {
                if start < 0 {
                    return Err(invalid("start", "must be >= 0"));
                }
                let (ti, ci) = p.find_midi_clip(clip).ok_or(EditError::ClipNotFound(clip))?;
                let dest = p.track_index(track).ok_or(EditError::TrackNotFound(track))?;
                if p.tracks[dest].kind != TrackKind::Instrument {
                    return Err(invalid("track", "MIDI clips belong on instrument tracks"));
                }
                let old_track = p.tracks[ti].id;
                let mut c = p.tracks[ti].midi_clips.remove(ci);
                let old_start = std::mem::replace(&mut c.start, start);
                p.tracks[dest].midi_clips.push(c);
                p.tracks[dest].midi_clips.sort_by_key(|c| (c.start, c.id));
                Ok(Command::MoveMidiClip { clip, track: old_track, start: old_start })
            }
            Command::ResizeMidiClip { clip, start, length } => {
                if start < 0 || length <= 0 {
                    return Err(invalid("clip", "requires start >= 0 and length > 0"));
                }
                let (ti, ci) = p.find_midi_clip(clip).ok_or(EditError::ClipNotFound(clip))?;
                let c = &mut p.tracks[ti].midi_clips[ci];
                let inv = Command::ResizeMidiClip { clip, start: c.start, length: c.length };
                c.start = start;
                c.length = length;
                p.tracks[ti].midi_clips.sort_by_key(|c| (c.start, c.id));
                Ok(inv)
            }
            Command::SetMidiNotes { clip, mut notes } => {
                midi::validate_notes(&notes).map_err(|r| invalid("notes", r))?;
                let (ti, ci) = p.find_midi_clip(clip).ok_or(EditError::ClipNotFound(clip))?;
                midi::sort_notes(&mut notes);
                let old = std::mem::replace(&mut p.tracks[ti].midi_clips[ci].notes, notes);
                Ok(Command::SetMidiNotes { clip, notes: old })
            }
            Command::SetSynthParams { track, params } => {
                params.validate().map_err(|r| invalid("synth", r))?;
                let t = p.track_mut(track).ok_or(EditError::TrackNotFound(track))?;
                if t.kind != TrackKind::Instrument {
                    return Err(invalid("track", "only instrument tracks have a synth"));
                }
                let old = std::mem::replace(&mut t.synth, params);
                Ok(Command::SetSynthParams { track, params: old })
            }
            Command::SetTrackArm { track, arm } => {
                let t = p.track_mut(track).ok_or(EditError::TrackNotFound(track))?;
                if arm && t.kind != TrackKind::Audio {
                    return Err(invalid("track", "only audio tracks record audio (MIDI recording is not implemented)"));
                }
                let old = std::mem::replace(&mut t.arm, arm);
                Ok(Command::SetTrackArm { track, arm: old })
            }
            Command::SetTrackMonitor { track, monitor } => {
                let t = p.track_mut(track).ok_or(EditError::TrackNotFound(track))?;
                if monitor && t.kind != TrackKind::Audio {
                    return Err(invalid("track", "input monitoring is for audio tracks"));
                }
                let old = std::mem::replace(&mut t.monitor, monitor);
                Ok(Command::SetTrackMonitor { track, monitor: old })
            }
            Command::SetTrackInput { track, input } => {
                if let Some(i) = input
                    && i.stereo
                    && i.channel == u16::MAX
                {
                    return Err(invalid("input", "stereo pair exceeds channel range"));
                }
                let t = p.track_mut(track).ok_or(EditError::TrackNotFound(track))?;
                if input.is_some() && t.kind != TrackKind::Audio {
                    return Err(invalid("track", "only audio tracks take an audio input"));
                }
                let old = std::mem::replace(&mut t.input, input);
                Ok(Command::SetTrackInput { track, input: old })
            }
            Command::AddDevice { track, index, mut device } => {
                let ti = p.track_index(track).ok_or(EditError::TrackNotFound(track))?;
                if device.plugin.kind != PluginKind::Effect {
                    return Err(invalid("device", "only effect plugins go in the insert chain"));
                }
                sanitize_builtin(&mut device)?;
                if device.id.0 == 0 {
                    device.id = DeviceId(p.alloc_id());
                } else if p.find_device(device.id).is_some() {
                    return Err(EditError::DuplicateId(device.id.0));
                } else {
                    p.next_id = p.next_id.max(device.id.0 + 1);
                }
                let n = p.tracks[ti].devices.len();
                let idx = index.unwrap_or(n);
                if idx > n {
                    return Err(invalid("index", format!("{idx} > {n}")));
                }
                let id = device.id;
                p.tracks[ti].devices.insert(idx, device);
                Ok(Command::RemoveDevice { device: id })
            }
            Command::RemoveDevice { device } => {
                let (ti, slot) = p.find_device(device).ok_or_else(|| invalid("device", format!("{} not found", device.0)))?;
                let track = p.tracks[ti].id;
                match slot {
                    DeviceSlot::Insert(i) => {
                        let d = p.tracks[ti].devices.remove(i);
                        Ok(Command::AddDevice { track, index: Some(i), device: d })
                    }
                    DeviceSlot::Instrument => {
                        let d = p.tracks[ti].instrument_plugin.take();
                        Ok(Command::SetInstrumentPlugin { track, device: d })
                    }
                }
            }
            Command::MoveDevice { device, track, index } => {
                let (ti, slot) = p.find_device(device).ok_or_else(|| invalid("device", format!("{} not found", device.0)))?;
                let DeviceSlot::Insert(from) = slot else {
                    return Err(invalid("device", "instrument plugins can't be moved into a chain"));
                };
                let dest = p.track_index(track).ok_or(EditError::TrackNotFound(track))?;
                let old_track = p.tracks[ti].id;
                let d = p.tracks[ti].devices.remove(from);
                if index > p.tracks[dest].devices.len() {
                    p.tracks[ti].devices.insert(from, d);
                    return Err(invalid("index", "out of range"));
                }
                p.tracks[dest].devices.insert(index, d);
                Ok(Command::MoveDevice { device, track: old_track, index: from })
            }
            Command::SetDeviceEnabled { device, enabled } => {
                let (ti, slot) = p.find_device(device).ok_or_else(|| invalid("device", format!("{} not found", device.0)))?;
                let d = match slot {
                    DeviceSlot::Insert(i) => &mut p.tracks[ti].devices[i],
                    DeviceSlot::Instrument => p.tracks[ti].instrument_plugin.as_mut().expect("found"),
                };
                let old = std::mem::replace(&mut d.enabled, enabled);
                Ok(Command::SetDeviceEnabled { device, enabled: old })
            }
            Command::SetInstrumentPlugin { track, mut device } => {
                let ti = p.track_index(track).ok_or(EditError::TrackNotFound(track))?;
                if p.tracks[ti].kind != TrackKind::Instrument {
                    return Err(invalid("track", "instrument plugins belong on instrument tracks"));
                }
                if let Some(d) = device.as_mut() {
                    if d.plugin.kind != PluginKind::Instrument {
                        return Err(invalid("device", "not an instrument plugin"));
                    }
                    if d.id.0 == 0 {
                        d.id = DeviceId(p.alloc_id());
                    } else if p.find_device(d.id).is_some_and(|(t, s)| !(t == ti && s == DeviceSlot::Instrument)) {
                        return Err(EditError::DuplicateId(d.id.0));
                    } else {
                        p.next_id = p.next_id.max(d.id.0 + 1);
                    }
                }
                let old = std::mem::replace(&mut p.tracks[ti].instrument_plugin, device);
                Ok(Command::SetInstrumentPlugin { track, device: old })
            }
            Command::SetDeviceParam { device, param, value } => {
                let (d, def) = builtin_mut(p, device)?;
                let v = crate::builtin::validate_value(def, &param, value).map_err(|r| invalid("value", r))?;
                let old = d.params.insert(param.clone(), v).unwrap_or_else(|| def.param(&param).map(|q| q.default).unwrap_or(v));
                Ok(Command::SetDeviceParam { device, param, value: old })
            }
            Command::SetDeviceParams { device, params, preset } => {
                let (d, def) = builtin_mut(p, device)?;
                let mut checked = std::collections::BTreeMap::new();
                for (k, v) in &params {
                    checked.insert(k.clone(), crate::builtin::validate_value(def, k, *v).map_err(|r| invalid("value", r))?);
                }
                let mut old = std::collections::BTreeMap::new();
                for (k, v) in checked {
                    let prev = d.params.insert(k.clone(), v).unwrap_or_else(|| def.param(&k).map(|q| q.default).unwrap_or(v));
                    old.insert(k, prev);
                }
                let old_preset = std::mem::replace(&mut d.preset, preset);
                Ok(Command::SetDeviceParams { device, params: old, preset: old_preset })
            }
            Command::SetDeviceSidechain { device, source } => {
                let (ti, _) = p.find_device(device).ok_or_else(|| invalid("device", format!("{} not found", device.0)))?;
                if let Some(s) = source {
                    let si = p.track_index(s).ok_or(EditError::TrackNotFound(s))?;
                    if si == ti {
                        return Err(invalid("source", "a device can't sidechain from its own track"));
                    }
                }
                let (d, def) = builtin_mut(p, device)?;
                if !def.sidechain {
                    return Err(invalid("device", format!("{} has no sidechain input", def.name)));
                }
                let old = std::mem::replace(&mut d.sidechain, source);
                Ok(Command::SetDeviceSidechain { device, source: old })
            }
            Command::SetAutomationLane { track, target, lane } => {
                if let Some(l) = &lane {
                    if l.target != target {
                        return Err(invalid("lane", "lane.target must equal target"));
                    }
                    l.validate().map_err(|r| invalid("automation", r))?;
                }
                let lanes: &mut Vec<AutomationLane> = match track {
                    None => {
                        if target != AutomationTarget::Volume {
                            return Err(invalid("target", "the master only has volume automation"));
                        }
                        &mut p.master_automation
                    }
                    Some(id) => {
                        let ti = p.track_index(id).ok_or(EditError::TrackNotFound(id))?;
                        match &target {
                            AutomationTarget::Synth { .. } if p.tracks[ti].kind != TrackKind::Instrument => {
                                return Err(invalid("target", "synth automation needs an instrument track"));
                            }
                            AutomationTarget::Plugin { device, .. }
                                if !p.tracks[ti].devices.iter().chain(p.tracks[ti].instrument_plugin.iter()).any(|d| d.id == *device) =>
                            {
                                return Err(invalid("target", "plugin automation must target a device on the same track"));
                            }
                            AutomationTarget::Builtin { device, device_type, param } => {
                                let d = p.tracks[ti].devices.iter().find(|d| d.id == *device).ok_or_else(|| invalid("target", "device automation must target a device on the same track"))?;
                                let def = crate::builtin::def_of(d).filter(|def| def.id == device_type).ok_or_else(|| invalid("target", "device kind mismatch"))?;
                                if !def.param(param).is_some_and(|q| q.automatable) {
                                    return Err(invalid("target", format!("{} has no automatable parameter {param:?}", def.name)));
                                }
                            }
                            _ => {}
                        }
                        &mut p.tracks[ti].automation
                    }
                };
                let pos = lanes.iter().position(|l| l.target == target);
                let old = match (pos, lane) {
                    (Some(i), Some(l)) => Some(std::mem::replace(&mut lanes[i], l)),
                    (Some(i), None) => Some(lanes.remove(i)),
                    (None, Some(l)) => {
                        lanes.push(l);
                        None
                    }
                    (None, None) => None,
                };
                Ok(Command::SetAutomationLane { track, target, lane: old })
            }
            Command::Batch { label, commands } => {
                let mut inverses = Vec::with_capacity(commands.len());
                for cmd in commands {
                    match cmd.apply(p) {
                        Ok(inv) => inverses.push(inv),
                        Err(e) => {
                            // Roll back what was applied so the batch is atomic.
                            for inv in inverses.into_iter().rev() {
                                inv.apply(p).expect("inverse of a successful command must apply");
                            }
                            return Err(e);
                        }
                    }
                }
                inverses.reverse();
                Ok(Command::Batch { label, commands: inverses })
            }
        }
    }
}

/// The built-in device `id` and its definition.
fn builtin_mut(p: &mut Project, id: DeviceId) -> Result<(&mut Device, &'static crate::builtin::DeviceDef), EditError> {
    let (ti, slot) = p.find_device(id).ok_or_else(|| invalid("device", format!("{} not found", id.0)))?;
    let d = match slot {
        DeviceSlot::Insert(i) => &mut p.tracks[ti].devices[i],
        DeviceSlot::Instrument => p.tracks[ti].instrument_plugin.as_mut().expect("found"),
    };
    let def = crate::builtin::def_of(d).ok_or_else(|| invalid("device", "not a built-in device"))?;
    Ok((d, def))
}

/// Built-in devices entering the model: the definition must exist; stored
/// values are snapped into range, unknown ids dropped and missing ids filled
/// with defaults (so the saved state is always complete).
fn sanitize_builtin(d: &mut Device) -> Result<(), EditError> {
    if d.plugin.format != crate::device::PluginFormat::Builtin {
        return Ok(());
    }
    let def = crate::builtin::find(&d.plugin.id).ok_or_else(|| invalid("device", format!("unknown built-in device {:?}", d.plugin.id)))?;
    let mut values = def.defaults();
    for (k, v) in std::mem::take(&mut d.params) {
        if let Some(q) = def.param(&k) {
            values.insert(k, q.snap(v));
        }
    }
    d.params = values;
    if !def.sidechain {
        d.sidechain = None;
    }
    Ok(())
}

fn kind_name(k: TrackKind) -> &'static str {
    match k {
        TrackKind::Audio => "Audio",
        TrackKind::Instrument => "Instrument",
    }
}

fn validate_gain(field: &'static str, g: f32) -> Result<(), EditError> {
    if !g.is_finite() || !(0.0..=MAX_GAIN).contains(&g) {
        return Err(invalid(field, format!("{g} outside 0..={MAX_GAIN} (linear)")));
    }
    Ok(())
}

fn validate_pan(pan: f32) -> Result<(), EditError> {
    if !pan.is_finite() || !(-1.0..=1.0).contains(&pan) {
        return Err(invalid("pan", format!("{pan} outside -1..=1")));
    }
    Ok(())
}

fn validate_midi_clip(c: &MidiClip) -> Result<(), EditError> {
    if c.start < 0 || c.length <= 0 {
        return Err(invalid("clip", "requires start >= 0 and length > 0"));
    }
    midi::validate_notes(&c.notes).map_err(|r| invalid("notes", r))
}

fn validate_clip(p: &Project, c: &Clip) -> Result<(), EditError> {
    let a = p.asset(c.asset).ok_or(EditError::AssetNotFound(c.asset))?;
    if c.start < 0 {
        return Err(invalid("start", "must be >= 0"));
    }
    if c.source_length == 0 {
        return Err(invalid("source_length", "must be > 0"));
    }
    if c.source_offset.checked_add(c.source_length).is_none_or(|end| end > a.frames) {
        return Err(invalid("source_region", format!("offset {} + length {} exceeds asset length {}", c.source_offset, c.source_length, a.frames)));
    }
    validate_gain("clip_gain", c.gain)
}
