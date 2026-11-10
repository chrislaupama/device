//! Clip context menu renders: Consolidate (⌘J), Bounce › Bounce to new track (⌘B) and Export audio.
//! Values and behaviour: reference/findings/clip-menu.md (S71–S74).
//!
//! All three render one track through the engine's offline render (the same path as the Stems
//! export in src/dialogs.rs) over the range of one clip or one group of clips, with the device
//! paused. The render of a clip's range is short, so it runs on the UI thread in 4096-frame
//! chunks; everything slow (WAV encode, file write, decode, peaks) runs on a worker thread and
//! lands in `poll_jobs` as `JobResult::ClipRendered`. Each edit is one Batch.
//!
//! Files go to `<project>/media/joined` (Consolidate) and `<project>/media/bounce` (Bounce) once
//! the project is saved, else to `<temp>/Saine Studio/Unsaved renders/`; like unsaved
//! recordings they are not moved into the project when it is saved.

use crate::controller::{Controller, JobResult, load_and_analyse};
use crate::dialogs::{CHUNK, TAIL_S, create_free_file, file_safe};
use crate::peaks::Pyramid;
use crate::wavgen;
use saine_core::render_plan::{self, ClipSpan, RenderMode};
use saine_core::{Asset, AssetId, Clip, ClipId, Command, Track, TrackId, TrackKind, snapshot};
use saine_engine::AssetInfo;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Below this a rendered frame counts as silence when the tail after the clip is trimmed (−80 dBFS).
const SILENCE: f32 = 1e-4;
/// How long a render waits for the engine's background warp renders (a warped clip is silent until
/// its render is attached).
const WARP_WAIT: Duration = Duration::from_millis(1500);

/// What a finished render is for.
pub enum ClipOp {
    /// Replace the span's clips by one audio clip.
    Consolidate { span: ClipSpan },
    /// New audio track below the clip's track holding the render; the source clip is disabled.
    Bounce { source: ClipId, span: ClipSpan },
    /// A WAV the user picked; `reveal` shows it in the file manager afterwards.
    Export { reveal: bool },
}

pub struct ClipRendered {
    pub doc_epoch: u64,
    /// Document revision the render was made from; an edit in between cancels the landing.
    pub revision: u64,
    pub op: ClipOp,
    pub path: PathBuf,
    /// The loaded file (Consolidate and Bounce); None for Export.
    pub result: Result<Option<(AssetInfo, Pyramid)>, String>,
}

#[derive(Default)]
pub struct ClipRender {
    /// A render is between its start and its landing: the menu entries wait.
    pub busy: bool,
    /// Overrides the render folder (self-test scratch directory).
    pub dir_override: Option<PathBuf>,
    /// Test hook: where the next Export audio writes, instead of asking with a file dialog.
    pub export_path: Option<PathBuf>,
    /// Export audio waits for the file dialog the main loop runs unborrowed: (clip, suggested name).
    pub dialog_request: Option<(ClipId, String)>,
    /// Milliseconds the last engine render took on the UI thread.
    pub last_ms: f64,
}

/// A finished offline render of a span: interleaved stereo.
struct Rendered {
    samples: Vec<f32>,
    rate: f64,
}

/// Open the platform's file manager on `path`.
pub fn reveal(path: &str) -> Result<(), String> {
    let p = Path::new(path);
    let spawned = if cfg!(target_os = "macos") {
        std::process::Command::new("open").arg("-R").arg(p).spawn()
    } else if cfg!(windows) {
        std::process::Command::new("explorer").arg(format!("/select,{path}")).spawn()
    } else {
        std::process::Command::new("xdg-open").arg(p.parent().map(Path::to_path_buf).unwrap_or_default()).spawn()
    };
    spawned.map(|_| ()).map_err(|e| e.to_string())
}

impl Controller {
    fn render_dir(&self, kind: &str) -> PathBuf {
        if let Some(d) = &self.clip_render.dir_override {
            return d.join(kind);
        }
        match &self.project_dir {
            Some(p) => p.join("media").join(kind),
            None => std::env::temp_dir().join("Saine Studio").join("Unsaved renders").join(kind),
        }
    }

    /// Why clip `id` cannot be rendered, or None. `needs_notes`: an empty MIDI clip has no sound.
    fn render_block(&self, id: ClipId, needs_notes: bool) -> Option<&'static str> {
        let p = self.project();
        if let Some(c) = p.clip(id) {
            return if c.disabled {
                Some("the clip is disabled")
            } else if !self.asset_map.contains_key(&c.asset) {
                Some("the audio file is missing")
            } else {
                None
            };
        }
        match p.midi_clip(id) {
            Some(c) if c.disabled => Some("the clip is disabled"),
            Some(c) if needs_notes && c.notes.is_empty() => Some("the clip has no notes"),
            Some(_) => None,
            None => Some("the clip is gone"),
        }
    }

    /// Whether the menu entry can run on clip `id`.
    pub(crate) fn can_render_clip(&self, id: ClipId, needs_notes: bool) -> bool {
        self.render_block(id, needs_notes).is_none()
    }

    /// Renders `span` alone through the engine's offline render. The device is paused and the
    /// transport, metronome and published session are restored afterwards. The rendered range is
    /// the span plus, when the track's sound rings out, a tail trimmed at silence.
    fn render_span(&mut self, span: &ClipSpan, mode: RenderMode) -> Result<Rendered, String> {
        let p = self.project();
        let rate = self.engine.sample_rate();
        let start = p.ticks_to_frames(span.start, rate);
        let body = (p.ticks_to_frames(span.end, rate) - start).max(1) as usize;
        let tail = if render_plan::rings_out(p, span.track, mode) { (TAIL_S * rate) as usize } else { 0 };
        let projected = render_plan::render_project(p, span, mode);
        let (snap, _) = snapshot::build_full(&projected, self.rate, &self.asset_map, &self.reversed_assets, &self.transients_by_asset());
        let guard = self.offline_begin()?;
        let e = self.engine.clone();
        let t0 = Instant::now();
        let out = (|| {
            e.publish(&snap).map_err(|e| e.to_string())?;
            let waited = Instant::now();
            while e.warp_renders_pending() > 0 {
                if waited.elapsed() > WARP_WAIT {
                    return Err("warped audio is still being prepared, try again in a moment".to_string());
                }
                e.plugin_idle();
                std::thread::sleep(Duration::from_millis(2));
            }
            e.set_loop(false, 0, 0).map_err(|e| e.to_string())?;
            e.stop().map_err(|e| e.to_string())?;
            e.seek(start).map_err(|e| e.to_string())?;
            e.play().map_err(|e| e.to_string())?;
            let mut out = vec![0f32; (body + tail) * 2];
            for chunk in out.chunks_mut(CHUNK * 2) {
                e.render_offline(chunk).map_err(|e| e.to_string())?;
            }
            Ok(out)
        })();
        self.clip_render.last_ms = t0.elapsed().as_secs_f64() * 1e3;
        let restarted = self.offline_end(guard, true);
        let mut samples = out?;
        restarted?;
        let last = samples.chunks_exact(2).rposition(|f| f[0].abs() > SILENCE || f[1].abs() > SILENCE);
        samples.truncate(last.map_or(0, |i| i + 1).clamp(body, body + tail) * 2);
        Ok(Rendered { samples, rate })
    }

    /// Writes the render to `file` and, for Consolidate and Bounce, loads it into the engine, on a
    /// worker thread; the result lands in `poll_jobs`.
    fn spawn_clip_render(&mut self, op: ClipOp, rendered: Rendered, path: PathBuf, file: std::fs::File, bits: u16) {
        let (engine, tx) = (self.engine.clone(), self.jobs_tx.clone());
        let (doc_epoch, revision) = (self.doc_epoch, self.doc.revision());
        let load = !matches!(op, ClipOp::Export { .. });
        self.clip_render.busy = true;
        self.pending_jobs += 1;
        std::thread::Builder::new()
            .name("clip-render".into())
            .spawn(move || {
                let result = wavgen::encode_wav_pcm(file, rendered.rate.round() as u32, 2, bits, &rendered.samples)
                    .map_err(|e| e.to_string())
                    .and_then(|()| if load { load_and_analyse(&engine, &path).map(Some) } else { Ok(None) });
                let _ = tx.send(JobResult::ClipRendered(Box::new(ClipRendered { doc_epoch, revision, op, path, result })));
            })
            .expect("spawn clip render thread");
    }

    /// Consolidate and Bounce: render into a fresh file under `media/<kind>` named after the track.
    fn render_to_file(&mut self, op: ClipOp, span: &ClipSpan, mode: RenderMode, kind: &str, suffix: &str) {
        let track = self.project().track(span.track).map(|t| t.name.clone()).unwrap_or_default();
        let stem = format!("{}-{suffix}", file_safe(&track, 40).replace(' ', "-"));
        let dir = self.render_dir(kind);
        let opened = std::fs::create_dir_all(&dir).map_err(|e| e.to_string()).and_then(|()| create_free_file(&dir, &stem));
        let (path, file) = match opened {
            Ok(v) => v,
            Err(e) => return self.set_status(format!("Render failed: {e}"), 6),
        };
        self.set_status("Rendering…", 30);
        match self.render_span(span, mode) {
            Ok(r) => self.spawn_clip_render(op, r, path, file, 24),
            Err(e) => {
                let _ = std::fs::remove_file(&path);
                self.set_status(format!("Render failed: {e}"), 6);
            }
        }
    }

    // ---------------------------------------------------------------- Consolidate (⌘J)

    /// Joins clip `id` with the clips of its track that overlap it into one clip over their range.
    /// Audio is rendered from the track alone (no devices, fader or pan) and replaces the clips
    /// with one unwarped clip; MIDI clips merge their notes. One undo step.
    pub fn consolidate_clip(&mut self, id: ClipId) {
        if self.clip_render.busy {
            return self.set_status("Still rendering. One moment.", 3);
        }
        let p = self.project();
        let Some(span) = render_plan::consolidate_group(p, id) else { return self.set_status("Consolidate: the clip is disabled or gone", 4) };
        if let Some(why) = span.clips.iter().find_map(|&c| self.render_block(c, false)) {
            return self.set_status(format!("Consolidate: {why}"), 4);
        }
        if span.kind == TrackKind::Instrument {
            let Some(mut merged) = render_plan::merge_midi(p, &span) else { return };
            merged.id = ClipId(p.next_id);
            let new_id = merged.id;
            let mut commands: Vec<Command> = span.clips.iter().map(|&clip| Command::RemoveMidiClip { clip }).collect();
            commands.push(Command::AddMidiClip { track: span.track, clip: merged });
            let n = span.clips.len();
            if self.execute(Command::Batch { label: "Consolidate Clips".into(), commands }) {
                self.select_clip(Some(new_id));
                self.set_status(format!("Consolidated {n} clip{}", if n == 1 { "" } else { "s" }), 3);
            }
            return;
        }
        self.render_to_file(ClipOp::Consolidate { span: span.clone() }, &span, RenderMode::Material, "joined", "merged");
    }

    // ---------------------------------------------------------------- Bounce to new track (⌘B)

    /// Renders clip `id` through its track's devices (neutral fader and pan) to a new audio clip
    /// on a new track below, which takes the track's fader and pan; the source clip is disabled
    /// (the reference app mutes the originals the same way). A MIDI clip becomes audio. One undo step.
    pub fn bounce_clip(&mut self, id: ClipId) {
        if self.clip_render.busy {
            return self.set_status("Still rendering. One moment.", 3);
        }
        if let Some(why) = self.render_block(id, true) {
            return self.set_status(format!("Bounce: {why}"), 4);
        }
        let Some(span) = render_plan::clip_span(self.project(), id) else { return };
        self.render_to_file(ClipOp::Bounce { source: id, span: span.clone() }, &span, RenderMode::Bounce, "bounce", "bounce");
    }

    // ---------------------------------------------------------------- Export audio

    /// The name the save dialog offers: "<track> <clip>.wav".
    pub fn export_clip_name(&self, id: ClipId) -> String {
        let p = self.project();
        let clip = p.clip(id).map(|c| c.name.clone()).or_else(|| p.midi_clip(id).map(|c| c.name.clone())).unwrap_or_default();
        let track = self.clip_track(id).and_then(|t| p.track(t)).map(|t| t.name.clone()).unwrap_or_default();
        let name = file_safe(&format!("{} {}", track.trim(), clip.trim()), 80);
        format!("{}.wav", if name.is_empty() { "Clip" } else { &name })
    }

    /// Export audio to Finder: asks for a file (the main loop runs the dialog with the controller
    /// unborrowed; the self-test sets `export_path` instead), then `export_clip`.
    pub fn export_clip_request(&mut self, id: ClipId) {
        if let Some(why) = self.render_block(id, true) {
            return self.set_status(format!("Export: {why}"), 4);
        }
        match self.clip_render.export_path.take() {
            Some(path) => self.export_clip(id, &path, false),
            None => self.clip_render.dialog_request = Some((id, self.export_clip_name(id))),
        }
    }

    /// Writes clip `id`'s range, as it sounds on its track (devices, fader, pan, with the tail of
    /// a ringing track), to `path` as a stereo WAV at the export dialog's bit depth.
    pub fn export_clip(&mut self, id: ClipId, path: &Path, reveal: bool) {
        if self.clip_render.busy {
            return self.set_status("Still rendering. One moment.", 3);
        }
        let Some(span) = render_plan::clip_span(self.project(), id) else { return };
        let file = match std::fs::File::create(path) {
            Ok(f) => f,
            Err(e) => return self.set_status(format!("Export failed: {}: {e}", path.display()), 6),
        };
        self.set_status("Rendering…", 30);
        match self.render_span(&span, RenderMode::Stem) {
            Ok(r) => {
                let bits = if self.export.bits24 { 24 } else { 16 };
                self.spawn_clip_render(ClipOp::Export { reveal }, r, path.to_path_buf(), file, bits);
            }
            Err(e) => {
                let _ = std::fs::remove_file(path);
                self.set_status(format!("Export failed: {e}"), 6);
            }
        }
    }

    // ---------------------------------------------------------------- landing

    /// A worker finished: put the render in the project (one Batch) or report the failure.
    pub(crate) fn finish_clip_render(&mut self, r: ClipRendered) {
        self.clip_render.busy = false;
        let ClipRendered { doc_epoch, revision, op, path, result } = r;
        let (info, pyr) = match (result, &op) {
            (Err(e), _) => {
                let _ = std::fs::remove_file(&path);
                let what = if matches!(op, ClipOp::Export { .. }) { "Export" } else { "Render" };
                return self.set_status(format!("{what} failed: {e}"), 6);
            }
            (Ok(None), ClipOp::Export { reveal: show }) => {
                self.set_status(format!("Exported {}", path.display()), 5);
                if *show {
                    let _ = reveal(&path.to_string_lossy());
                }
                return;
            }
            (Ok(None), _) => return,
            (Ok(Some(loaded)), _) => loaded,
        };
        if doc_epoch != self.doc_epoch || revision != self.doc.revision() {
            let _ = self.engine.release_asset(info.asset_id);
            let _ = std::fs::remove_file(&path);
            if doc_epoch == self.doc_epoch {
                self.set_status("Render cancelled: the project changed while it rendered", 5);
            }
            return;
        }
        let p = self.project();
        let mut next = p.next_id;
        let mut alloc = || {
            next += 1;
            next - 1
        };
        let asset_id = AssetId(alloc());
        let native_frames = (info.frames as f64 * info.source_sample_rate / self.rate).round().max(1.0) as u64;
        let asset = Asset {
            id: asset_id,
            relative_path: None,
            absolute_path: std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone()).to_string_lossy().into_owned(),
            sample_rate: info.source_sample_rate,
            channels: info.channels,
            frames: native_frames,
        };
        let clip_id = ClipId(alloc());
        let new_clip = |name: String, start: saine_core::Ticks| Clip { id: clip_id, asset: asset_id, name, start: start.max(0), source_offset: 0, source_length: native_frames, gain: 1.0, warp: Default::default(), disabled: false };
        let mut cmds = vec![Command::AddAsset { asset }];
        let (label, track, status) = match &op {
            ClipOp::Consolidate { span } => {
                cmds.extend(span.clips.iter().map(|&clip| Command::RemoveClip { clip }));
                let first = p.clip(span.clips[0]).map(|c| c.name.clone()).unwrap_or_default();
                cmds.push(Command::AddClip { track: span.track, clip: new_clip(render_plan::merged_name(&first), span.start) });
                let n = span.clips.len();
                ("Consolidate Clips", span.track, format!("Consolidated {n} clip{}", if n == 1 { "" } else { "s" }))
            }
            ClipOp::Bounce { source, span } => {
                let Some((src, index)) = p.track(span.track).zip(p.track_index(span.track)) else {
                    let _ = self.engine.release_asset(info.asset_id);
                    return self.set_status("Bounce: the track is gone", 4);
                };
                let name = p.clip(*source).map(|c| c.name.clone()).or_else(|| p.midi_clip(*source).map(|c| c.name.clone())).unwrap_or_default();
                let id = TrackId(alloc());
                let mut t = Track::new(id, TrackKind::Audio, &format!("{} (Bounce)", src.name.trim()), src.color);
                (t.gain, t.pan) = (src.gain, src.pan);
                t.clips.push(new_clip(name, span.start));
                cmds.push(Command::AddTrack { index: Some(index + 1), track: t });
                cmds.push(Command::SetClipEnabled { clip: *source, enabled: false });
                ("Bounce to New Track", id, "Bounced to a new track".to_string())
            }
            ClipOp::Export { .. } => return,
        };
        self.peaks.insert(info.asset_id, pyr);
        self.asset_map.insert(asset_id, info.asset_id);
        if self.execute(Command::Batch { label: label.into(), commands: cmds }) {
            self.select_track(track);
            self.select_clip(Some(clip_id));
            self.set_status(status, 3);
        } else {
            self.asset_map.remove(&asset_id);
            let _ = self.engine.release_asset(info.asset_id);
        }
    }
}
