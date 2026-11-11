//! Export dialog (mixdown or stems to WAV) and the unsaved-changes prompt shown before
//! File › New Project. DStudio renders with the engine's offline
//! render at the engine sample rate. A stem is one track played alone: DStudio has no
//! returns, groups or master devices, so that is the post-effects track
//! render.
use crate::Studio;
use crate::controller::{Controller, sync_model};
use crate::resample::Resampler;
use crate::wavgen;
use crate::ChoiceItem;
use dstudio_core::{Project, Ticks, TrackId};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// Silence rendered after the last bar so reverb and delay tails ring out.
pub const TAIL_S: f64 = 2.0;
const CHUNK: usize = 4096;
/// Tracks menu rows (window points): item row, separator gap, top/bottom padding.
const MENU_ROW: f32 = 21.0;
const MENU_GAP: f32 = 7.0;
const MENU_PAD: f32 = 3.0;

/// File › Open Recent shows a project by its folder name without the extension.
fn recent_name(path: &str) -> String {
    Path::new(path).file_stem().map_or_else(|| path.to_string(), |s| s.to_string_lossy().into_owned())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Recipe {
    #[default]
    Mixdown,
    Stems,
    /// An Ableton Live set (src/live_set.rs).
    Project,
}

/// Which tracks Stems renders (the Tracks picker).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StemsPick {
    /// One file per heard track that has clips.
    #[default]
    AllTracks,
    /// The selected track (DStudio selects one track at a time).
    Selected,
    Track(TrackId),
}

pub struct ExportUi {
    pub open: bool,
    pub recipe: Recipe,
    pub stems: StemsPick,
    pub tracks_menu: bool,
    pub bits24: bool,
    /// The file's sample rate as picked in the dialog (44.1 or 48 kHz); None until then, which
    /// means the one nearest the engine's.
    pub rate: Option<u32>,
    /// The Name field: "<name>.wav" when one file is written, the stems folder base otherwise.
    pub name: String,
    pub dir: PathBuf,
    /// The name the dialog filled in; while the field still holds it, picking one track
    /// renames it after that track.
    auto_name: String,
    default_name: String,
}

impl Default for ExportUi {
    fn default() -> Self {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
        ExportUi {
            open: false,
            recipe: Recipe::Mixdown,
            stems: StemsPick::AllTracks,
            tracks_menu: false,
            bits24: true,
            rate: None,
            name: "Untitled.wav".into(),
            dir: home.join("Music").join("DStudio Exports"),
            auto_name: "Untitled".into(),
            default_name: "Untitled".into(),
        }
    }
}

impl ExportUi {
    /// Stems written into a folder (all tracks, selected tracks) rather than as one file.
    pub fn writes_folder(&self) -> bool {
        self.recipe == Recipe::Stems && !matches!(self.stems, StemsPick::Track(_))
    }

    /// The Name field without a ".wav" ending, trimmed, with characters a file name can't hold
    /// replaced.
    pub fn base_name(&self) -> String {
        let n = self.name.trim();
        let n = if n.to_lowercase().ends_with(".wav") { &n[..n.len() - 4] } else { n };
        file_safe(n, usize::MAX)
    }

    fn normalize_name(&mut self) {
        let base = self.base_name();
        if !base.is_empty() {
            self.name = if self.writes_folder() || self.recipe == Recipe::Project { base } else { format!("{base}.wav") };
        }
    }
}

/// A file or folder name: path separators, reserved and control characters become "_",
/// cut to "max" characters; empty stays empty.
pub fn file_safe(name: &str, max: usize) -> String {
    let s: String = name.chars().map(|c| if "<>:\"/\\|?*".contains(c) || c.is_control() { '_' } else { c }).take(max).collect();
    s.trim().to_string()
}

/// "NN Name.wav": the 1-based position in the export, two digits, then the track name (60
/// characters at most, "Track" when it has none).
pub fn stem_file_name(index: usize, track_name: &str) -> String {
    let name = file_safe(track_name, 60);
    format!("{:02} {}.wav", index + 1, if name.is_empty() { "Track" } else { &name })
}

/// Bars covered by the export: up to the bar that holds the end of the last clip (at least one).
pub fn export_bars(p: &Project) -> i64 {
    let end: Ticks = p
        .tracks
        .iter()
        .flat_map(|t| t.clips.iter().map(|c| c.start + p.clip_length_ticks(c)).chain(t.midi_clips.iter().map(|c| c.end())))
        .max()
        .unwrap_or(0);
    let tpb = p.ticks_per_bar();
    ((end + tpb - 1) / tpb).max(1)
}

/// Tracks the stems export renders, in track order. All tracks: every track that is heard
/// (not muted, soloed when any track is) and has clips. Selected or one track: that track when
/// it has clips.
pub fn stem_tracks(p: &Project, pick: StemsPick, selected: Option<TrackId>) -> Vec<TrackId> {
    // A soloed return doesn't silence the ordinary tracks' stems (stems are dry).
    let any_solo = p.tracks.iter().any(|t| t.solo && !t.is_return());
    p.tracks
        .iter()
        .filter(|t| !t.is_return() && (!t.clips.is_empty() || !t.midi_clips.is_empty()))
        .filter(|t| match pick {
            StemsPick::AllTracks => !t.mute && (!any_solo || t.solo),
            StemsPick::Selected => selected == Some(t.id),
            StemsPick::Track(id) => t.id == id,
        })
        .map(|t| t.id)
        .collect()
}

/// "3.7 MB • 0:14" for "files" stereo PCM files of "frames" each at "bits".
pub fn estimate_text(frames: u64, rate: f64, bits: u32, files: usize) -> String {
    let mb = files as f64 * frames as f64 * 2.0 * (bits / 8) as f64 / 1e6;
    let secs = (frames as f64 / rate).round() as u64;
    format!("{mb:.1} MB • {}:{:02}", secs / 60, secs % 60)
}

/// Opens "dir/<stem>.wav", or "<stem> 2.wav", "<stem> 3.wav", … when taken; never an existing file.
pub(crate) fn create_free_file(dir: &Path, stem: &str) -> Result<(PathBuf, std::fs::File), String> {
    (1..1000)
        .find_map(|n| {
            let path = dir.join(if n == 1 { format!("{stem}.wav") } else { format!("{stem} {n}.wav") });
            match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(f) => Some(Ok((path, f))),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => None,
                Err(e) => Some(Err(e.to_string())),
            }
        })
        .unwrap_or_else(|| Err("no free file name".into()))
}

/// "dir/<name>", or "<name> 2", "<name> 3", … : the first that doesn't exist or is empty.
fn free_folder(dir: &Path, name: &str) -> Result<PathBuf, String> {
    (1..1000)
        .map(|n| dir.join(if n == 1 { name.to_string() } else { format!("{name} {n}") }))
        .find(|p| std::fs::read_dir(p).map_or(true, |mut d| d.next().is_none()))
        .ok_or_else(|| "no free folder name".into())
}

/// DStudio lets MIDI clips overlap on a track and a Live track can't, so overlapping clips are
/// spread over lanes (one Live track each): in start order, each clip takes the first lane that
/// is free at its start. Disabled clips are left out.
pub fn midi_lanes(clips: &[dstudio_core::MidiClip]) -> Vec<Vec<&dstudio_core::MidiClip>> {
    let mut sorted: Vec<&dstudio_core::MidiClip> = clips.iter().filter(|c| !c.disabled).collect();
    sorted.sort_by_key(|c| c.start);
    let mut lanes: Vec<Vec<&dstudio_core::MidiClip>> = Vec::new();
    for c in sorted {
        match lanes.iter_mut().find(|l| l.last().is_some_and(|last| last.end() <= c.start)) {
            Some(lane) => lane.push(c),
            None => lanes.push(vec![c]),
        }
    }
    lanes
}

/// What one render pass writes.
struct Pass {
    /// None: the mix; Some: that track alone.
    track: Option<TrackId>,
    path: PathBuf,
    file: std::fs::File,
}

impl Controller {
    pub fn export_frames(&self) -> u64 {
        let p = self.project();
        let secs = p.ticks_to_seconds(export_bars(p) * p.ticks_per_bar()) + TAIL_S;
        (secs * self.engine.sample_rate()).round() as u64
    }

    /// The sample rate Mixdown and Stems write: the one picked in the dialog, else 48 kHz when
    /// the engine runs at 47 kHz or more and 44.1 kHz otherwise. Project renders stay at the
    /// engine rate.
    pub fn export_rate(&self) -> u32 {
        self.export.rate.unwrap_or(if self.engine.sample_rate() >= 47_000.0 { 48_000 } else { 44_100 })
    }

    /// Frames in each Mixdown or Stems file, at `export_rate`.
    pub fn export_out_frames(&self) -> u64 {
        let rate = self.engine.sample_rate().round() as u32;
        let frames = self.export_frames() as usize;
        Resampler::new(rate, self.export_rate()).map_or(frames, |r| r.out_frames(frames)) as u64
    }

    pub fn export_set_rate(&mut self, rate48: bool) {
        self.export.rate = Some(if rate48 { 48_000 } else { 44_100 });
        self.ui_dirty = true;
    }

    fn export_stem_tracks(&self) -> Vec<TrackId> {
        stem_tracks(self.project(), self.export.stems, self.selected_track)
    }

    pub fn export_open(&mut self) {
        let stem = self.project_dir.as_ref().and_then(|d| d.file_stem()).map(|s| s.to_string_lossy().into_owned());
        let base = stem.unwrap_or_else(|| "Untitled".into());
        self.export.default_name = base.clone();
        self.export.auto_name = base.clone();
        self.export.name = base;
        self.export.recipe = Recipe::Mixdown;
        self.export.tracks_menu = false;
        if let StemsPick::Track(id) = self.export.stems
            && self.project().track(id).is_none()
        {
            self.export.stems = StemsPick::AllTracks;
        }
        self.export.normalize_name();
        self.export.open = true;
        self.ui_dirty = true;
    }

    /// Close the export dialog, the unsaved-changes prompt and Check for Updates (Close, ×,
    /// Cancel, OK, Escape).
    pub fn dialogs_close(&mut self) {
        self.export.open = false;
        self.export.tracks_menu = false;
        self.unsaved_open = false;
        if let Some(crate::live_import::PendingImport::Loaded(job)) = self.pending_import.take() {
            self.release_live_set(*job);
        }
        self.updates_open = false;
        self.undo_history_open = false;
        self.ui_dirty = true;
    }

    /// Recipe cards: 0 Mixdown, 1 Stems, 2 Project.
    pub fn export_pick_recipe(&mut self, recipe: i32) {
        self.export.recipe = match recipe {
            1 => Recipe::Stems,
            2 => Recipe::Project,
            _ => Recipe::Mixdown,
        };
        self.export.tracks_menu = false;
        self.export_follow_auto_name();
        self.ui_dirty = true;
    }

    pub fn export_toggle_tracks_menu(&mut self) {
        self.export.tracks_menu = !self.export.tracks_menu;
        self.ui_dirty = true;
    }

    /// Tracks menu rows: 0 all individual tracks, 1 selected track, then one per track (the
    /// separator row between them is not clickable).
    pub fn export_pick_tracks(&mut self, row: i32) {
        let p = self.project();
        let pick = match row {
            0 => Some(StemsPick::AllTracks),
            1 => self.selected_track.map(|_| StemsPick::Selected),
            r => p.tracks.iter().filter(|t| !t.is_return()).nth((r - 3).max(0) as usize).filter(|_| r >= 3).map(|t| StemsPick::Track(t.id)),
        };
        if let Some(pick) = pick {
            self.export.stems = pick;
        }
        self.export.tracks_menu = false;
        self.export_follow_auto_name();
        self.ui_dirty = true;
    }

    /// Keep the Name field's form in step with what is written ("name.wav" or a folder base),
    /// and rename it after the picked track while it still holds the dialog's own name.
    fn export_follow_auto_name(&mut self) {
        if self.export.base_name() == self.export.auto_name {
            let track = match (self.export.recipe, self.export.stems) {
                (Recipe::Stems, StemsPick::Track(id)) => self.project().track(id).map(|t| t.name.trim().to_string()),
                _ => None,
            };
            let auto = match track {
                Some(t) if !t.is_empty() => file_safe(&format!("{} {t}", self.export.default_name), usize::MAX),
                _ => self.export.default_name.clone(),
            };
            self.export.auto_name = auto.clone();
            self.export.name = auto;
        }
        self.export.normalize_name();
    }

    pub fn export_set_bits(&mut self, bits24: bool) {
        self.export.bits24 = bits24;
        self.ui_dirty = true;
    }

    pub fn export_set_name(&mut self, name: &str) {
        self.export.name = name.to_string();
        self.ui_dirty = true;
    }

    pub fn export_set_dir(&mut self, dir: PathBuf) {
        self.export.dir = dir;
        self.ui_dirty = true;
    }

    /// Render and write the picked recipe: the mixdown as one file, stems as one file per track
    /// (in a "<name> Stems" folder unless one track is picked), or a Live set.
    pub fn export_run(&mut self) {
        self.ui_dirty = true;
        let base = self.export.base_name();
        let base = if base.is_empty() { "Untitled".to_string() } else { base };
        let result = match self.export.recipe {
            Recipe::Mixdown => self.export_plan(&self.export.dir.clone(), &[(None, base)], self.export_rate()),
            Recipe::Stems => {
                let tracks = self.export_stem_tracks();
                if tracks.is_empty() {
                    self.set_status("Export failed: no track to export has clips", 6);
                    return;
                }
                if self.export.writes_folder() {
                    let p = self.project();
                    let names: Vec<(Option<TrackId>, String)> = tracks
                        .iter()
                        .enumerate()
                        .map(|(i, id)| (Some(*id), stem_file_name(i, p.track(*id).map_or("", |t| t.name.as_str())).trim_end_matches(".wav").to_string()))
                        .collect();
                    free_folder(&self.export.dir, &format!("{base} Stems")).and_then(|dir| self.export_plan(&dir, &names, self.export_rate()))
                } else {
                    self.export_plan(&self.export.dir.clone(), &[(Some(tracks[0]), base)], self.export_rate())
                }
            }
            Recipe::Project => self.export_project(&base),
        };
        match result {
            Ok((written, restart)) => {
                self.export.open = false;
                self.export.tracks_menu = false;
                let what = match written.as_slice() {
                    [one] => one.display().to_string(),
                    many => format!("{} files to {}", many.len(), many[0].parent().map_or(String::new(), |d| d.display().to_string())),
                };
                match restart {
                    Ok(()) => self.set_status(format!("Exported {what}"), 4),
                    Err(e) => self.set_status(format!("Exported {what}, but the audio device did not restart: {e}"), 8),
                }
            }
            Err(e) => self.set_status(format!("Export failed: {e}"), 6),
        }
    }

    /// Export › Project: a "<name> Project" folder holding "<name>.als", every track with clips
    /// rendered after its devices into Samples/Processed/DStudio (Live can't load DStudio's devices,
    /// so the set sounds like the project), and "DStudio Export Notes.txt". Instrument tracks keep
    /// their MIDI clips on a second track next to the audio, turned off. The set is checked
    /// against Live's structure before it is written. Returns the set's path.
    /// A failed export removes its folder (it was new or empty), so no half-written project stays.
    fn export_project(&mut self, name: &str) -> Result<(Vec<PathBuf>, Result<(), String>), String> {
        let folder = free_folder(&self.export.dir, &format!("{name} Project"))?;
        let written = self.write_project(name, &folder);
        if written.is_err() {
            let _ = std::fs::remove_dir_all(&folder);
        }
        written
    }

    fn write_project(&mut self, name: &str, folder: &Path) -> Result<(Vec<PathBuf>, Result<(), String>), String> {
        use crate::live_set::{self, LiveSet, SetAudioClip, SetContent, SetMidiClip, SetNote, SetTrack};
        const RENDERS: &str = "Samples/Processed/DStudio";
        let has_clips = |t: &dstudio_core::Track| !t.clips.is_empty() || !t.midi_clips.is_empty();
        let rendered: Vec<(TrackId, String)> = self
            .project()
            .tracks
            .iter()
            .filter(|t| has_clips(t))
            .enumerate()
            .map(|(i, t)| (t.id, stem_file_name(i, &t.name)))
            .collect();
        let restart = if rendered.is_empty() {
            Ok(())
        } else {
            let files: Vec<(Option<TrackId>, String)> = rendered.iter().map(|(id, f)| (Some(*id), f.trim_end_matches(".wav").to_string())).collect();
            // The set records the renders' length and rate as the engine made them.
            let rate = self.engine.sample_rate().round() as u32;
            self.export_plan(&folder.join(RENDERS), &files, rate)?.1
        };
        let p = self.project();
        let rate = self.engine.sample_rate();
        let frames = self.export_frames();
        let beats = |ticks: i64| ticks as f64 / dstudio_core::PPQ as f64;
        let render_beats = frames as f64 / rate * p.transport.bpm / 60.0;
        let mut tracks = Vec::new();
        let mut notes = String::new();
        let mut off = Vec::new();
        for t in &p.tracks {
            let color = t.color.0 as u32 % 70;
            let render = rendered.iter().find(|(id, _)| *id == t.id).map(|(_, f)| f);
            let content = match (render, t.kind) {
                (Some(file), _) => {
                    let path = folder.join(RENDERS).join(file);
                    SetContent::Audio(vec![SetAudioClip {
                        name: t.name.clone(),
                        start: 0.0,
                        length: render_beats,
                        relative_path: format!("{RENDERS}/{file}"),
                        absolute_path: path.display().to_string(),
                        file_size: std::fs::metadata(&path).map_or(0, |m| m.len()),
                        frames,
                        sample_rate: rate.round() as u32,
                        markers: Vec::new(),
                    }])
                }
                (None, dstudio_core::TrackKind::Audio) => SetContent::Audio(Vec::new()),
                (None, dstudio_core::TrackKind::Instrument) => SetContent::Midi(Vec::new()),
                // Return tracks have no material of their own (track renders are dry).
                (None, dstudio_core::TrackKind::Return) => continue,
            };
            if t.mute {
                off.push(t.name.clone());
            }
            // A rendered track already carries its volume and pan.
            let (volume, pan) = if render.is_some() { (1.0, 0.0) } else { (t.gain, t.pan) };
            tracks.push(SetTrack { name: t.name.clone(), color, on: !t.mute, volume, pan, content });
            if render.is_some() && !t.midi_clips.is_empty() {
                for (k, lane) in midi_lanes(&t.midi_clips).iter().enumerate() {
                let clips = lane
                    .iter()
                    .map(|c| SetMidiClip {
                        name: c.name.clone(),
                        start: beats(c.start),
                        length: beats(c.length),
                        notes: if c.pattern.is_some() { c.notes.clone() } else { c.tiled_notes() }
                            .iter()
                            .filter(|n| n.start >= 0 && n.start < c.length && n.length > 0)
                            .map(|n| SetNote { pitch: n.pitch.min(127), time: beats(n.start), duration: beats(n.length.min(c.length - n.start)), velocity: n.velocity.clamp(1, 127) })
                            .collect(),
                    })
                    .collect();
                let label = if k == 0 { format!("{} (MIDI)", t.name) } else { format!("{} (MIDI {})", t.name, k + 1) };
                tracks.push(SetTrack { name: label, color, on: false, volume: t.gain, pan: t.pan, content: SetContent::Midi(clips) });
                }
            }
        }
        let tr = &p.transport;
        let set = LiveSet {
            bpm: tr.bpm,
            numerator: tr.time_signature.numerator as u32,
            denominator: tr.time_signature.denominator as u32,
            loop_on: tr.loop_enabled,
            loop_start: beats(tr.loop_start),
            loop_length: beats(tr.loop_end - tr.loop_start),
            tracks,
        };
        let xml = live_set::to_xml(&set);
        live_set::validate(&xml, Some(folder)).map_err(|e| format!("the Live set failed its structure check: {e}"))?;
        let als = folder.join(format!("{name}.als"));
        let bytes = live_set::gzip(&xml).map_err(|e| e.to_string())?;
        std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
        std::fs::write(&als, bytes).map_err(|e| e.to_string())?;
        let _ = writeln!(notes, "Exported from Device Studio for Ableton Live 11 or later.\n");
        if rendered.is_empty() {
            let _ = writeln!(notes, "No track has clips, so the set has empty tracks only.");
        } else {
            let _ = writeln!(
                notes,
                "Live can't load DStudio's instruments and effects, so {} rendered as audio after {} devices, into {RENDERS}. The set sounds like the DStudio project.",
                if rendered.len() == 1 { "1 track was".to_string() } else { format!("{} tracks were", rendered.len()) },
                if rendered.len() == 1 { "its" } else { "their" }
            );
            let _ = writeln!(notes, "Instrument tracks keep their MIDI clips on a second track next to the audio, turned off.");
        }
        if !off.is_empty() {
            let _ = writeln!(notes, "\nMuted in DStudio, so turned off in the set: {}.", off.join(", "));
        }
        let _ = writeln!(notes, "\nNot carried over: automation and device settings. Track colours are not matched to Live's palette.");
        std::fs::write(folder.join("DStudio Export Notes.txt"), notes).map_err(|e| e.to_string())?;
        Ok((vec![als], restart))
    }

    /// Opens every destination file first (never an existing one), then renders and writes each
    /// pass at `rate` Hz. Returns the written files and the device restart result, which does not
    /// undo them.
    fn export_plan(&mut self, dir: &Path, files: &[(Option<TrackId>, String)], rate: u32) -> Result<(Vec<PathBuf>, Result<(), String>), String> {
        let frames = self.export_frames() as usize;
        let engine_rate = self.engine.sample_rate().round() as u32;
        let resampler = Resampler::new(engine_rate, rate).ok_or_else(|| format!("can't convert {engine_rate} Hz to {rate} Hz"))?;
        // WAV sizes are 32-bit: refuse before rendering rather than after.
        if resampler.out_frames(frames) as u64 * 2 * if self.export.bits24 { 3 } else { 2 } > (u32::MAX - 36) as u64 {
            return Err("too long for a WAV file (4 GB limit)".into());
        }
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let mut passes = Vec::with_capacity(files.len());
        for (track, stem) in files {
            match create_free_file(dir, stem) {
                Ok((path, file)) => passes.push(Pass { track: *track, path, file }),
                Err(e) => {
                    for p in &passes {
                        let _ = std::fs::remove_file(&p.path);
                    }
                    return Err(e);
                }
            }
        }
        let paths: Vec<PathBuf> = passes.iter().map(|p| p.path.clone()).collect();
        let (written, restarted) = self.render_passes(passes, frames, rate, &resampler);
        match (written, restarted) {
            (Ok(()), restarted) => Ok((paths, restarted)),
            (Err(e), restarted) => {
                for p in &paths {
                    let _ = std::fs::remove_file(p);
                }
                Err(match restarted {
                    Ok(()) => e,
                    Err(r) => format!("{e}; the audio device also did not restart: {r}"),
                })
            }
        }
    }

    /// Plays the arrangement once per pass (no loop, no click) and writes it as stereo PCM at
    /// `out_rate` Hz, converted by `resampler` when the engine runs at another rate. The audio
    /// device is paused for the renders; transport, mutes and the device are restored afterwards.
    fn render_passes(&mut self, passes: Vec<Pass>, frames: usize, out_rate: u32, resampler: &Resampler) -> (Result<(), String>, Result<(), String>) {
        let rate = self.engine.sample_rate();
        let bits = if self.export.bits24 { 24 } else { 16 };
        let restore = self.project().ticks_to_frames(self.playhead_ticks(), rate);
        let solo_passes = passes.iter().any(|p| p.track.is_some());
        let e = self.engine.clone();
        let was_running = e.device_running();
        if was_running
            && let Err(err) = e.stop_device()
        {
            return (Err(err.to_string()), Ok(()));
        }
        let metronome_was = std::mem::replace(&mut self.metronome_on, false);
        self.sync_loop_and_click();
        let mut out = vec![0f32; frames * 2];
        let mix = self.current_snapshot();
        let written = (|| {
            for pass in passes {
                if let Some(id) = pass.track {
                    let mut snap = mix.clone();
                    for t in &mut snap.tracks {
                        t.solo = false;
                        t.mute = t.track_id != id.0;
                    }
                    e.publish(&snap).map_err(|e| e.to_string())?;
                } else if solo_passes {
                    e.publish(&mix).map_err(|e| e.to_string())?;
                }
                e.set_loop(false, 0, 0).map_err(|e| e.to_string())?;
                e.stop().map_err(|e| e.to_string())?;
                e.seek(0).map_err(|e| e.to_string())?;
                e.play().map_err(|e| e.to_string())?;
                for chunk in out.chunks_mut(CHUNK * 2) {
                    e.render_offline(chunk).map_err(|e| e.to_string())?;
                }
                e.stop().map_err(|e| e.to_string())?;
                let converted = resampler.process(&out, 2);
                wavgen::encode_wav_pcm(pass.file, out_rate, 2, bits, &converted).map_err(|e| e.to_string())?;
            }
            Ok::<(), String>(())
        })();
        let _ = e.stop();
        if solo_passes {
            self.publish();
        }
        let _ = e.seek(restore);
        self.metronome_on = metronome_was;
        self.sync_loop_and_click();
        let restarted = if was_running { e.start_device().map_err(|e| e.to_string()) } else { Ok(()) };
        (written, restarted)
    }

    /// Edit › Undo History rows: "Open Project" (the state before any edit), then every edit in
    /// sentence case like the Undo and Redo items, undone ones last. State 0 applied, 1
    /// current, 2 undone.
    fn undo_history_rows(&self) -> Vec<crate::UndoHistoryRow> {
        let depth = self.doc.undo_depth();
        let state = |i: usize| if i < depth { 0 } else if i == depth { 1 } else { 2 };
        let base = crate::UndoHistoryRow { number: 1, label: "Open Project".into(), date: "—".into(), time: "—".into(), state: state(0) };
        let steps = self.doc.history_steps().into_iter().enumerate().map(|(i, (label, made))| {
            let made = chrono::DateTime::<chrono::Local>::from(made);
            crate::UndoHistoryRow {
                number: i as i32 + 2,
                label: dstudio_core::history::sentence_case(label).into(),
                date: made.format("%b %-d, %Y").to_string().into(),
                time: made.format("%-I:%M:%S %p").to_string().into(),
                state: state(i + 1),
            }
        });
        std::iter::once(base).chain(steps).collect()
    }

    /// Undo History: undo or redo until row `row` (0 = Open Project) is the current state.
    pub fn undo_history_travel(&mut self, row: i32) {
        let target = row.max(0) as usize;
        while self.doc.undo_depth() > target && self.doc.can_undo() {
            let depth = self.doc.undo_depth();
            self.undo();
            if self.doc.undo_depth() == depth {
                break;
            }
        }
        while self.doc.undo_depth() < target && self.doc.can_redo() {
            let depth = self.doc.undo_depth();
            self.redo();
            if self.doc.undo_depth() == depth {
                break;
            }
        }
        self.ui_dirty = true;
    }

    /// File › New Project: ask first when the project has unsaved changes.
    pub fn new_project_request(&mut self) {
        if self.doc.is_dirty() {
            self.unsaved_open = true;
            self.ui_dirty = true;
        } else {
            self.new_project();
        }
    }

    /// The unsaved-changes prompt was answered with Save or Don't Save: go on with what asked.
    pub fn unsaved_continue(&mut self) {
        use crate::live_import::PendingImport;
        match self.pending_import.take() {
            Some(PendingImport::File(file)) => {
                self.dialogs_close();
                self.import_live_set(file);
            }
            Some(PendingImport::Loaded(job)) => self.apply_live_set(*job),
            None => self.new_project(),
        }
    }

    pub fn new_project(&mut self) {
        self.dialogs_close();
        self.stop_return();
        for (_, ea) in self.asset_map.drain().chain(self.reversed_assets.drain()) {
            let _ = self.engine.release_asset(ea);
        }
        self.project_dir = None;
        self.view = Default::default();
        self.replace_project(Project::default());
    }

    pub(crate) fn push_dialogs(&self, s: &Studio) {
        s.set_export_open(self.export.open);
        s.set_unsaved_open(self.unsaved_open);
        s.set_updates_open(self.updates_open);
        s.set_app_version(env!("CARGO_PKG_VERSION").into());
        s.set_undo_history_open(self.undo_history_open);
        s.set_undo_history_current(self.doc.undo_depth() as i32);
        if self.undo_history_open {
            sync_model(&self.undo_history_model, self.undo_history_rows());
        }
        use slint::Model as _;
        let recent = &self.settings.prefs.recent_projects;
        let names = s.get_recent_projects();
        let unchanged = names.row_count() == recent.len() && recent.iter().enumerate().all(|(i, p)| names.row_data(i).is_some_and(|n| n == recent_name(p)));
        if !unchanged {
            let rows: Vec<slint::SharedString> = recent.iter().map(|p| recent_name(p).into()).collect();
            s.set_recent_projects(slint::ModelRc::new(slint::VecModel::from(rows)));
        }
        if !self.export.open {
            return;
        }
        let bits = if self.export.bits24 { 24 } else { 16 };
        // Project renders stay at the engine rate; Mixdown and Stems use the picked one.
        let (frames, rate) = if self.export.recipe == Recipe::Project {
            (self.export_frames(), self.engine.sample_rate())
        } else {
            (self.export_out_frames(), self.export_rate() as f64)
        };
        let p = self.project();
        let stems = self.export.recipe == Recipe::Stems;
        let files = if stems { self.export_stem_tracks().len() } else { stem_tracks(p, StemsPick::AllTracks, None).len() };
        s.set_export_recipe(match self.export.recipe {
            Recipe::Mixdown => 0,
            Recipe::Stems => 1,
            Recipe::Project => 2,
        });
        s.set_export_bits24(self.export.bits24);
        s.set_export_rate48(self.export_rate() == 48_000);
        s.set_export_recipe_text(format!("WAV {bits}-bit").into());
        let plural = if files == 1 { "file" } else { "files" };
        s.set_export_stems_text(format!("{files} WAV {plural}").into());
        s.set_export_files_text(format!("{files} {plural}").into());
        s.set_export_bars_text(format!("Bars 1–{}", export_bars(p)).into());
        s.set_export_name(self.export.name.clone().into());
        s.set_export_name_label(if self.export.writes_folder() { "Folder" } else { "Name" }.into());
        s.set_export_dir(self.export.dir.display().to_string().into());
        s.set_export_estimate(estimate_text(frames, rate, bits, if stems { files } else { 1 }).into());
        let tracks_label = match self.export.stems {
            StemsPick::AllTracks => "All individual tracks".to_string(),
            StemsPick::Selected => "Selected tracks only".to_string(),
            StemsPick::Track(id) => p.track(id).map(|t| t.name.trim()).filter(|n| !n.is_empty()).unwrap_or("Track").to_string(),
        };
        s.set_export_tracks_label(tracks_label.into());
        s.set_export_tracks_menu_open(self.export.tracks_menu);
        if self.export.tracks_menu {
            let mut y = MENU_PAD;
            let mut rows = Vec::new();
            let mut row = |label: String, checked: bool, hint: bool, h: f32, rows: &mut Vec<ChoiceItem>| {
                rows.push(ChoiceItem { label: label.into(), checked, hint, y, h });
                y += h;
            };
            row("All individual tracks".into(), self.export.stems == StemsPick::AllTracks, false, MENU_ROW, &mut rows);
            row("Selected tracks only".into(), self.export.stems == StemsPick::Selected, self.selected_track.is_none(), MENU_ROW, &mut rows);
            row(String::new(), false, true, MENU_GAP, &mut rows);
            // Returns hold no clips of their own, so they are never a stem.
            for t in p.tracks.iter().filter(|t| !t.is_return()) {
                let name = t.name.trim();
                row(if name.is_empty() { "Track".into() } else { name.into() }, self.export.stems == StemsPick::Track(t.id), false, MENU_ROW, &mut rows);
            }
            s.set_export_tracks_menu_h(y + MENU_PAD);
            sync_model(&self.export_tracks_model, rows);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimate_matches_the_reference_readout() {
        // 6 bars at 120 BPM + tail, 44.1 kHz, 24-bit → "3.7 MB • 0:14".
        assert_eq!(estimate_text(14 * 44_100, 44_100.0, 24, 1), "3.7 MB • 0:14");
        assert_eq!(estimate_text(14 * 44_100, 44_100.0, 24, 2), "7.4 MB • 0:14");
    }

    #[test]
    fn the_export_rate_follows_the_engine_until_one_is_picked() {
        let engine = std::sync::Arc::new(dstudio_engine::Engine::new(dstudio_engine::Config::default()).unwrap());
        let mut c = Controller::new(engine, Project::default());
        let engine_rate = c.engine.sample_rate();
        assert_eq!(c.export_rate(), if engine_rate >= 47_000.0 { 48_000 } else { 44_100 });
        c.export_set_rate(false);
        assert_eq!(c.export_rate(), 44_100);
        let frames = c.export_frames() as usize;
        let expect = (frames * 44_100).div_ceil(engine_rate.round() as usize);
        assert_eq!(c.export_out_frames() as usize, expect, "{frames} frames at {engine_rate} Hz");
        c.export_set_rate(true);
        assert_eq!(c.export_rate(), 48_000);
    }

    #[test]
    fn overlapping_midi_clips_get_their_own_lanes() {
        use dstudio_core::MidiClip;
        let bar = 4 * dstudio_core::PPQ;
        let clips = [MidiClip::empty(0, 2 * bar), MidiClip::empty(bar, bar), MidiClip::empty(2 * bar, bar), MidiClip::empty(bar, 3 * bar)];
        let lanes = midi_lanes(&clips);
        let starts: Vec<Vec<i64>> = lanes.iter().map(|l| l.iter().map(|c| c.start / bar).collect()).collect();
        assert_eq!(starts, vec![vec![0, 2], vec![1], vec![1]]);
        assert!(lanes.iter().all(|l| l.windows(2).all(|w| w[0].end() <= w[1].start)));
        let off = [MidiClip::empty(0, bar), MidiClip { disabled: true, ..MidiClip::empty(0, bar) }];
        assert_eq!(midi_lanes(&off).iter().map(Vec::len).collect::<Vec<_>>(), vec![1]);
    }

    #[test]
    fn stem_files_are_numbered_and_safe() {
        assert_eq!(stem_file_name(0, "Bass"), "01 Bass.wav");
        assert_eq!(stem_file_name(11, " Vox/Lead: 2 "), "12 Vox_Lead_ 2.wav");
        assert_eq!(stem_file_name(2, ""), "03 Track.wav");
        assert_eq!(stem_file_name(0, &"x".repeat(80)).len(), "01 .wav".len() + 60);
    }

    #[test]
    fn all_tracks_skips_unheard_and_empty_tracks() {
        let mut p = Project::default();
        let (a, b) = (p.tracks[0].id, p.tracks[1].id);
        assert!(stem_tracks(&p, StemsPick::AllTracks, None).is_empty(), "no clips, no stems");
        p.tracks[1].midi_clips.push(dstudio_core::MidiClip::empty(0, 4 * dstudio_core::PPQ));
        assert_eq!(stem_tracks(&p, StemsPick::AllTracks, None), vec![b]);
        assert_eq!(stem_tracks(&p, StemsPick::Selected, Some(a)), vec![]);
        assert_eq!(stem_tracks(&p, StemsPick::Track(b), None), vec![b]);
        p.tracks[1].mute = true;
        assert!(stem_tracks(&p, StemsPick::AllTracks, None).is_empty(), "muted tracks are not heard");
        assert_eq!(stem_tracks(&p, StemsPick::Track(b), None), vec![b], "a picked track renders even when muted");
    }

    #[test]
    fn a_soloed_return_is_not_a_stem_and_does_not_hide_the_tracks() {
        use dstudio_core::{ColorIndex, Track, TrackKind};
        let mut p = Project::default();
        let b = p.tracks[1].id;
        p.tracks[1].midi_clips.push(dstudio_core::MidiClip::empty(0, 4 * dstudio_core::PPQ));
        let r = TrackId(p.alloc_id());
        let mut ret = Track::new(r, TrackKind::Return, "Return A", ColorIndex(9));
        ret.solo = true;
        p.tracks.push(ret);
        assert_eq!(stem_tracks(&p, StemsPick::AllTracks, None), vec![b]);
        assert!(stem_tracks(&p, StemsPick::Track(r), None).is_empty());
    }

    #[test]
    fn pcm24_round_trips() {
        let path = std::env::temp_dir().join(format!("dstudio-pcm24-{}.wav", std::process::id()));
        let s = [0.0f32, 0.5, -0.5, 0.999];
        wavgen::write_wav_pcm(&path, 48_000, 2, 24, &s).unwrap();
        let (rate, ch, back) = wavgen::read_wav(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!((rate, ch, back.len()), (48_000, 2, 4));
        assert!(s.iter().zip(&back).all(|(a, b)| (a - b).abs() < 1e-6));
    }

    #[test]
    fn empty_project_exports_one_bar() {
        assert_eq!(export_bars(&Project::default()), 1);
    }
}
