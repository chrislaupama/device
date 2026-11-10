//! Undo/redo history built on invertible [`Command`]s.

use crate::command::{Command, EditError};
use crate::model::{Project, TrackId};
use std::time::{Duration, Instant, SystemTime};

#[derive(Debug, Clone)]
struct Entry {
    label: String,
    /// Command that re-applies the edit.
    redo: Command,
    /// Command that reverts the edit.
    undo: Command,
    coalesce_key: Option<String>,
    at: Instant,
    /// Wall-clock time the edit was made (Undo History's Date and Time).
    made: SystemTime,
}

/// Where an edit came from; used for logging, AI attribution and tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    User,
    Script,
    Assistant,
}

/// Owns the authoritative project and its undo history.
#[derive(Debug)]
pub struct Document {
    project: Project,
    undo: Vec<Entry>,
    redo: Vec<Entry>,
    /// Incremented on every change, including undo/redo. Consumers (engine
    /// snapshot publisher, UI) compare revisions to detect changes.
    revision: u64,
    saved_revision: u64,
    pub coalesce_window: Duration,
    pub max_history: usize,
}

impl Document {
    pub fn new(project: Project) -> Self {
        Document {
            project,
            undo: Vec::new(),
            redo: Vec::new(),
            revision: 0,
            saved_revision: 0,
            coalesce_window: Duration::from_millis(750),
            max_history: 1000,
        }
    }

    pub fn project(&self) -> &Project {
        &self.project
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn is_dirty(&self) -> bool {
        self.revision != self.saved_revision
    }

    pub fn mark_saved(&mut self) {
        self.saved_revision = self.revision;
    }

    /// Validate and execute a command, recording it for undo.
    pub fn execute(&mut self, cmd: Command, _origin: Origin) -> Result<(), EditError> {
        self.execute_at(cmd, Instant::now())
    }

    /// Like [`execute`](Self::execute) with an explicit timestamp (deterministic tests).
    pub fn execute_at(&mut self, cmd: Command, now: Instant) -> Result<(), EditError> {
        let mut cmd = cmd;
        let saved_next_id = self.project.next_id;
        // Resolve "allocate me" ids (0) up front so the stored redo command
        // reproduces exactly the same ids, including inside batches.
        assign_ids(&mut cmd, &mut self.project);
        let label = cmd.label();
        let key = cmd.coalesce_key();
        let redo = cmd.clone();
        let undo = match cmd.apply(&mut self.project) {
            Ok(u) => u,
            Err(e) => {
                self.project.next_id = saved_next_id;
                return Err(e);
            }
        };
        self.revision += 1;
        self.redo.clear();

        if let (Some(k), Some(last)) = (&key, self.undo.last_mut())
            && last.coalesce_key.as_deref() == Some(k.as_str())
            && now.duration_since(last.at) <= self.coalesce_window
        {
            // Keep the original undo, update redo to the newest value.
            last.redo = redo;
            last.at = now;
            return Ok(());
        }
        self.undo.push(Entry { label, redo, undo, coalesce_key: key, at: now, made: SystemTime::now() });
        if self.undo.len() > self.max_history {
            self.undo.remove(0);
        }
        Ok(())
    }

    /// Mark the end of a gesture: the next command never merges into the
    /// current last undo entry, whatever its coalesce key or timing.
    pub fn seal_coalescing(&mut self) {
        if let Some(last) = self.undo.last_mut() {
            last.coalesce_key = None;
        }
    }

    /// Store plugin states pulled from the engine (before saving). Not an
    /// undo step and not a revision change: the engine already has the state.
    pub fn sync_device_states(&mut self, states: &[(crate::device::DeviceId, String)]) {
        for (id, st) in states {
            for t in &mut self.project.tracks {
                for d in t.devices.iter_mut().chain(t.instrument_plugin.iter_mut()) {
                    if d.id == *id {
                        d.state = Some(st.clone());
                    }
                }
            }
        }
    }

    /// Record the modulation bases of plugin devices before saving (no undo
    /// step, like [Self::sync_device_states]); devices not listed get none.
    pub fn sync_mod_bases(&mut self, bases: &[(crate::device::DeviceId, Vec<(u32, f32)>)]) {
        for t in &mut self.project.tracks {
            for d in t.devices.iter_mut().chain(t.instrument_plugin.iter_mut()) {
                d.mod_bases = bases.iter().find(|(id, _)| *id == d.id).map(|(_, b)| b.clone()).unwrap_or_default();
            }
        }
    }

    /// Remember the bottom panel height for a panel kind (splitter drag). Saved with the
    /// project but not an undo step, like the reference's panel layout.
    /// Track row height scale (1.0 = default). Like the reference, a view change: saved with
    /// the project, never an undo step.
    pub fn set_track_height(&mut self, track: TrackId, height: f32) -> Result<(), EditError> {
        let (lo, hi) = crate::Track::HEIGHT_RANGE;
        if !(lo..=hi).contains(&height) {
            return Err(EditError::InvalidValue { field: "height", reason: format!("{height} outside {lo}..{hi}") });
        }
        let t = self.project.track_mut(track).ok_or(EditError::TrackNotFound(track))?;
        if t.height != height {
            t.height = height;
            self.revision += 1;
        }
        Ok(())
    }

    pub fn set_bottom_panel_height(&mut self, kind: &str, h: Option<f32>) {
        let map = &mut self.project.bottom_panel_heights;
        let changed = match h {
            Some(h) => map.insert(kind.to_string(), h) != Some(h),
            None => map.remove(kind).is_some(),
        };
        if changed {
            self.revision += 1;
        }
    }

    pub fn undo_depth(&self) -> usize {
        self.undo.len()
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo_label(&self) -> Option<&str> {
        self.undo.last().map(|e| e.label.as_str())
    }

    pub fn redo_label(&self) -> Option<&str> {
        self.redo.last().map(|e| e.label.as_str())
    }

    pub fn undo(&mut self) -> Result<bool, EditError> {
        let Some(mut e) = self.undo.pop() else { return Ok(false) };
        let redo = e.undo.clone().apply(&mut self.project)?;
        e.redo = redo;
        e.coalesce_key = None; // never coalesce into an entry that was undone
        self.revision += 1;
        self.redo.push(e);
        Ok(true)
    }

    pub fn redo(&mut self) -> Result<bool, EditError> {
        let Some(mut e) = self.redo.pop() else { return Ok(false) };
        let undo = e.redo.clone().apply(&mut self.project)?;
        e.undo = undo;
        self.revision += 1;
        self.undo.push(e);
        Ok(true)
    }

    /// Labels of the undo stack, oldest first (for an "Undo History" view).
    pub fn history_labels(&self) -> Vec<String> {
        self.undo.iter().map(|e| e.label.clone()).collect()
    }

    /// Every step Edit › Undo History lists, oldest first: the undo stack, then the redo stack
    /// with the next redo first. The first `undo_depth()` of them are applied.
    pub fn history_steps(&self) -> Vec<(&str, SystemTime)> {
        self.undo.iter().chain(self.redo.iter().rev()).map(|e| (e.label.as_str(), e.made)).collect()
    }
}

/// Replace zero ids with freshly allocated ones, recursively.
pub fn assign_ids(cmd: &mut Command, p: &mut Project) {
    use crate::model::{AssetId, ClipId, TrackId};
    match cmd {
        Command::AddTrack { track, .. } => {
            if track.id.0 == 0 {
                track.id = TrackId(p.alloc_id());
            }
            for c in &mut track.clips {
                if c.id.0 == 0 {
                    c.id = ClipId(p.alloc_id());
                }
            }
        }
        Command::AddClip { clip, .. } if clip.id.0 == 0 => clip.id = ClipId(p.alloc_id()),
        Command::AddMidiClip { clip, .. } if clip.id.0 == 0 => clip.id = ClipId(p.alloc_id()),
        Command::AddDevice { device, .. } if device.id.0 == 0 => device.id = crate::device::DeviceId(p.alloc_id()),
        Command::SetInstrumentPlugin { device: Some(device), .. } if device.id.0 == 0 => device.id = crate::device::DeviceId(p.alloc_id()),
        Command::AddAsset { asset } if asset.id.0 == 0 => asset.id = AssetId(p.alloc_id()),
        Command::Batch { commands, .. } => commands.iter_mut().for_each(|c| assign_ids(c, p)),
        _ => {}
    }
}
