//! Structured edit commands.
//!
//! Every mutation of the project goes through [`Command::apply`], which validates
//! its arguments, mutates the model and returns the exact inverse command.
//! The same commands are used by the UI, keyboard shortcuts, scripting, tests
//! and the AI assistant (they are serde-serialisable, so an LLM tool call can
//! be deserialised straight into a `Command` and validated before execution).

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
