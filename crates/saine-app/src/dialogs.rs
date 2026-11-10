//! Export dialog (mixdown to WAV) and the unsaved-changes prompt shown before
//! File › New Project. Layout values come from the reference app's export and unsaved-changes
//! modals (reference/research-findings.md, W2-D rows); Saine renders the mixdown with
//! the engine's offline render at the engine sample rate.
use crate::Studio;
use crate::controller::Controller;
use crate::wavgen;
use saine_core::{Project, Ticks};
use std::path::PathBuf;

/// Silence rendered after the last bar so reverb and delay tails ring out.
pub const TAIL_S: f64 = 2.0;
const CHUNK: usize = 4096;

pub struct ExportUi {
    pub open: bool,
    pub bits24: bool,
    pub name: String,
    pub dir: PathBuf,
}

impl Default for ExportUi {
    fn default() -> Self {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
        ExportUi { open: false, bits24: true, name: "Untitled.wav".into(), dir: home.join("Music").join("Saine Exports") }
    }
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

/// "3.7 MB • 0:14" for `frames` of stereo PCM at `bits`.
pub fn estimate_text(frames: u64, rate: f64, bits: u32) -> String {
    let mb = frames as f64 * 2.0 * (bits / 8) as f64 / 1e6;
    let secs = (frames as f64 / rate).round() as u64;
    format!("{mb:.1} MB • {}:{:02}", secs / 60, secs % 60)
}

impl Controller {
    pub fn export_frames(&self) -> u64 {
        let p = self.project();
        let secs = p.ticks_to_seconds(export_bars(p) * p.ticks_per_bar()) + TAIL_S;
        (secs * self.engine.sample_rate()).round() as u64
    }

    pub fn export_open(&mut self) {
        let stem = self.project_dir.as_ref().and_then(|d| d.file_stem()).map(|s| s.to_string_lossy().into_owned());
        self.export.name = format!("{}.wav", stem.as_deref().unwrap_or("Untitled"));
        self.export.open = true;
        self.ui_dirty = true;
    }

    /// Close the export dialog and the unsaved-changes prompt (Close, ×, Cancel, Escape).
    pub fn dialogs_close(&mut self) {
        self.export.open = false;
        self.unsaved_open = false;
        self.ui_dirty = true;
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

    /// Render the whole project offline and write it as stereo PCM WAV. The audio
    /// device is paused for the render and restarted afterwards.
    pub fn export_run(&mut self) {
        let name = if self.export.name.to_lowercase().ends_with(".wav") { self.export.name.clone() } else { format!("{}.wav", self.export.name) };
        self.ui_dirty = true;
        match self.render_mixdown(&name) {
            Ok((path, restart)) => {
                self.export.open = false;
                match restart {
                    Ok(()) => self.set_status(format!("Exported {}", path.display()), 4),
                    Err(e) => self.set_status(format!("Exported {}, but the audio device did not restart: {e}", path.display()), 8),
                }
            }
            Err(e) => self.set_status(format!("Export failed: {e}"), 6),
        }
    }

    /// Returns the written file (never an existing one: a taken name gets " 2", " 3", …)
    /// and the device restart result, which does not undo a written export.
    fn render_mixdown(&mut self, name: &str) -> Result<(PathBuf, Result<(), String>), String> {
        std::fs::create_dir_all(&self.export.dir).map_err(|e| e.to_string())?;
        let frames = self.export_frames() as usize;
        // WAV sizes are 32-bit: refuse before rendering rather than after.
        if frames as u64 * 2 * if self.export.bits24 { 3 } else { 2 } > (u32::MAX - 36) as u64 {
            return Err("too long for a WAV file (4 GB limit)".into());
        }
        let rate = self.engine.sample_rate();
        let restore = self.project().ticks_to_frames(self.playhead_ticks(), rate);
        let e = self.engine.clone();
        let was_running = e.device_running();
        if was_running {
            e.stop_device().map_err(|e| e.to_string())?;
        }
        // The mixdown plays the arrangement once: no loop, no click. Restored below.
        let metronome_was = std::mem::replace(&mut self.metronome_on, false);
        self.sync_loop_and_click();
        let mut out = vec![0f32; frames * 2];
        let rendered = (|| {
            e.set_loop(false, 0, 0).map_err(|e| e.to_string())?;
            e.stop().map_err(|e| e.to_string())?;
            e.seek(0).map_err(|e| e.to_string())?;
            e.play().map_err(|e| e.to_string())?;
            for chunk in out.chunks_mut(CHUNK * 2) {
                e.render_offline(chunk).map_err(|e| e.to_string())?;
            }
            Ok::<(), String>(())
        })();
        let _ = e.stop();
        let _ = e.seek(restore);
        self.metronome_on = metronome_was;
        self.sync_loop_and_click();
        let restarted = if was_running { e.start_device().map_err(|e| e.to_string()) } else { Ok(()) };
        match (self.write_mixdown(name, rendered.map(|_| out)), restarted) {
            (Ok(path), restarted) => Ok((path, restarted)),
            (Err(e), Ok(())) => Err(e),
            (Err(e), Err(r)) => Err(format!("{e}; the audio device also did not restart: {r}")),
        }
    }

    fn write_mixdown(&self, name: &str, rendered: Result<Vec<f32>, String>) -> Result<PathBuf, String> {
        let out = rendered?;
        let rate = self.engine.sample_rate();
        let stem = name.strip_suffix(".wav").or_else(|| name.strip_suffix(".WAV")).unwrap_or(name);
        let (path, file) = (1..1000)
            .find_map(|n| {
                let path = self.export.dir.join(if n == 1 { format!("{stem}.wav") } else { format!("{stem} {n}.wav") });
                match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
                    Ok(f) => Some(Ok((path, f))),
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => None,
                    Err(e) => Some(Err(e.to_string())),
                }
            })
            .unwrap_or_else(|| Err("no free file name".into()))?;
        if let Err(e) = wavgen::encode_wav_pcm(file, rate.round() as u32, 2, if self.export.bits24 { 24 } else { 16 }, &out) {
            let _ = std::fs::remove_file(&path);
            return Err(e.to_string());
        }
        Ok(path)
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
        if !self.export.open {
            return;
        }
        let bits = if self.export.bits24 { 24 } else { 16 };
        let rate = self.engine.sample_rate();
        s.set_export_bits24(self.export.bits24);
        s.set_export_rate48(rate >= 47_000.0);
        s.set_export_recipe_text(format!("WAV {bits}-bit").into());
        s.set_export_bars_text(format!("Bars 1–{}", export_bars(self.project())).into());
        s.set_export_name(self.export.name.clone().into());
        s.set_export_dir(self.export.dir.display().to_string().into());
        s.set_export_estimate(estimate_text(self.export_frames(), rate, bits).into());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimate_matches_the_reference_readout() {
        // The reference app: 6 bars at 120 BPM + tail, 44.1 kHz, 24-bit → "3.7 MB • 0:14".
        assert_eq!(estimate_text(14 * 44_100, 44_100.0, 24), "3.7 MB • 0:14");
    }

    #[test]
    fn pcm24_round_trips() {
        let path = std::env::temp_dir().join(format!("saine-pcm24-{}.wav", std::process::id()));
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
