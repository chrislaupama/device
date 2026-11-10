//! Arm / monitor / input routing, the record transport and the live recording
//! clip (docs/adr/0002-recording.md).
//!
//! Every project change is a `saine_core::Command`: R = `SetTrackArm`,
//! I = `SetTrackMonitor`, the inspector "In" popup = `SetTrackInput`, and a
//! finished recording pass = ONE `Batch` "Record" (`AddAsset` + `AddClip` per
//! take; with one armed track that is exactly one take). Device I/O goes through
//! the engine's recording API; while it is unimplemented, every engine error is
//! shown as a status message.
//!
//! Where takes are written (`Controller::recording_dir`): `<project dir>/media/`
//! when the project has been saved, else `<system temp dir>/Saine Studio/Unsaved
//! recordings/`. Note that Save does not move unsaved recordings into the project
//! (core `persist` has no media consolidation yet), and macOS may purge the temp
//! directory after a few days.

use crate::controller::{Controller, JobResult, load_and_analyse, sync_model};
use crate::peaks::Pyramid;
use crate::timeline::ClipVis;
use crate::{ChoiceItem, Studio};
use saine_core::{Asset, AssetId, Clip, ClipId, Command, InputSource, PPQ, TrackId, TrackKind};
use saine_engine::{AssetInfo, AudioDeviceInfo, RecordedTake};
use std::path::PathBuf;

/// Where recorded input may come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InputPolicy {
    /// Normal app: record from the configured input device.
    #[default]
    Device,
    /// `--self-test`: input is injected with `Engine::render_offline_with_input`;
    /// no real input device is ever opened or required.
    InjectedOnly,
}

#[derive(Debug, Default)]
pub struct RecState {
    pub recording: bool,
    pub policy: InputPolicy,
    /// Overrides `recording_dir()` (self-test scratch directory).
    pub dir_override: Option<PathBuf>,
    /// The "In" popup, while open: its entries (label, source).
    pub input_menu: Option<Vec<(String, Option<InputSource>)>>,
    /// Hint row under "None" when no device inputs can be listed.
    pub input_hint: String,
    /// Live takes drawn on the timeline (rebuilt every frame while recording).
    pub live_vis: Vec<ClipVis>,
    /// Label of the last finished pass (self-test reads it).
    pub last_pass: Option<String>,
}

/// Rows of the "In" selector: "None", then "Input k (Mono)" per device input
/// channel, then "Input k/k+1 (Stereo)" pairs when the device has ≥ 2 inputs.
pub fn input_choices(dev: Option<&AudioDeviceInfo>) -> Vec<(String, Option<InputSource>)> {
    let mut v = vec![("None".to_string(), None)];
    if let Some(d) = dev {
        let n = d.input_channels.min(u16::MAX as u32) as u16;
        for c in 0..n {
            v.push((input_label(Some(InputSource { channel: c, stereo: false })), Some(InputSource { channel: c, stereo: false })));
        }
        for c in (0..n.saturating_sub(1)).step_by(2) {
            v.push((input_label(Some(InputSource { channel: c, stereo: true })), Some(InputSource { channel: c, stereo: true })));
        }
    }
    v
}

/// Inspector "In" field text.
pub fn input_label(src: Option<InputSource>) -> String {
    match src {
        None => "Select audio input".into(),
        Some(InputSource { channel, stereo: false }) => format!("Input {} (Mono)", channel as u32 + 1),
        Some(InputSource { channel, stereo: true }) => format!("Input {}/{} (Stereo)", channel as u32 + 1, channel as u32 + 2),
    }
}

/// SCREENSHOT-ONLY fake device listing, used exclusively by `--state
/// reference-input-menu` and `--state reference-settings-audio` so those
/// captures are deterministic. Never configured on the engine.
pub fn screenshot_fake_devices() -> Vec<AudioDeviceInfo> {
    vec![
        AudioDeviceInfo {
            id: "fake:speakers".into(),
            name: "Built-in Speakers".into(),
            input_channels: 0,
            output_channels: 2,
            sample_rates: vec![44_100.0, 48_000.0],
            is_default_input: false,
            is_default_output: true,
        },
        AudioDeviceInfo {
            id: "fake:microphone".into(),
            name: "Built-in Microphone".into(),
            input_channels: 1,
            output_channels: 0,
            sample_rates: vec![44_100.0, 48_000.0],
            is_default_input: true,
            is_default_output: false,
        },
    ]
}

/// Popup geometry (window points): rows 21 pt, 3 pt padding, hint rows 30 pt.
pub const IN_ROW: f32 = 21.0;
pub const IN_HINT_ROW: f32 = 30.0;
pub const IN_PAD: f32 = 3.0;

impl Controller {
    // ---------------------------------------------------------------- devices

    /// Device listing: the SCREENSHOT fake list in `--state` captures, else the engine's.
    pub fn device_list(&self) -> Result<Vec<AudioDeviceInfo>, String> {
        if self.settings.fake_devices {
            return Ok(screenshot_fake_devices());
        }
        self.engine.list_devices().map_err(|e| e.message.clone())
    }

    /// The input device the "In" selector lists channels for: the one chosen in
    /// Settings › Audio (preference), else the one the engine reports open.
    /// No fallback to the system default: an input is only ever opened after the
    /// user picks it deliberately (macOS microphone prompt).
    pub fn current_input_device(&self) -> Result<Option<AudioDeviceInfo>, String> {
        let devs = self.device_list()?;
        if let Some(id) = &self.settings.prefs.audio.input
            && let Some(d) = devs.iter().find(|d| &d.id == id)
        {
            return Ok(Some(d.clone()));
        }
        if !self.settings.fake_devices
            && let Some(name) = self.engine.device_status().input_name
        {
            return Ok(devs.into_iter().find(|d| d.name == name && d.input_channels > 0));
        }
        Ok(None)
    }

    // ---------------------------------------------------------------- arm / monitor / input

    fn audio_track(&self, id: TrackId) -> Option<&saine_core::Track> {
        self.project().track(id).filter(|t| t.kind == TrackKind::Audio)
    }

    /// R button: `SetTrackArm` on audio tracks (one undo step). Instrument tracks:
    /// PLACEHOLDER (no MIDI recording).
    pub fn toggle_arm(&mut self, id: TrackId) {
        match self.project().track(id) {
            Some(t) if t.kind == TrackKind::Audio => {
                let arm = !t.arm;
                self.execute(Command::SetTrackArm { track: id, arm });
            }
            Some(_) => self.set_status("MIDI recording is not implemented yet", 3),
            None => {}
        }
    }

    /// I button: `SetTrackMonitor` on audio tracks. Instrument tracks: PLACEHOLDER.
    pub fn toggle_monitor(&mut self, id: TrackId) {
        match self.project().track(id) {
            Some(t) if t.kind == TrackKind::Audio => {
                let monitor = !t.monitor;
                self.execute(Command::SetTrackMonitor { track: id, monitor });
            }
            Some(_) => self.set_status("MIDI recording is not implemented yet (no input monitoring on instrument tracks)", 3),
            None => {}
        }
    }

    pub fn set_track_input(&mut self, id: TrackId, input: Option<InputSource>) {
        if self.audio_track(id).is_some_and(|t| t.input != input) {
            self.execute(Command::SetTrackInput { track: id, input });
        }
    }

    /// Open the inspector "In" popup for the selected audio track.
    pub fn open_input_menu(&mut self) {
        let Some(t) = self.selected_track.and_then(|t| self.audio_track(t)).map(|t| t.id) else { return };
        let _ = t;
        let (dev, hint) = match self.current_input_device() {
            Ok(Some(d)) => (Some(d), String::new()),
            Ok(None) => (None, "No input device. Choose one in Settings › Audio.".to_string()),
            Err(e) => (None, format!("Inputs unavailable: {e}")),
        };
        self.rec.input_menu = Some(input_choices(dev.as_ref()));
        self.rec.input_hint = hint;
        self.ui_dirty = true;
    }

    pub fn input_menu_close(&mut self) {
        self.rec.input_menu = None;
        self.ui_dirty = true;
    }

    pub fn input_menu_pick(&mut self, i: usize) {
        let Some(items) = self.rec.input_menu.take() else { return };
        if let (Some((_, src)), Some(t)) = (items.get(i), self.selected_track) {
            self.set_track_input(t, *src);
        }
        self.ui_dirty = true;
    }

    // ---------------------------------------------------------------- record transport

    /// Armed audio tracks that have an input (the ones a recording captures).
    pub fn armed_with_input(&self) -> Vec<TrackId> {
        self.project().tracks.iter().filter(|t| t.kind == TrackKind::Audio && t.arm && t.input.is_some()).map(|t| t.id).collect()
    }

    /// Takes go to `<project>/media/` once saved, else to a temp "Unsaved recordings" dir.
    pub fn recording_dir(&self) -> PathBuf {
        if let Some(d) = &self.rec.dir_override {
            return d.clone();
        }
        match &self.project_dir {
            Some(p) => p.join("media"),
            None => std::env::temp_dir().join("Saine Studio").join("Unsaved recordings"),
        }
    }

    /// Header record button / `R` (transport.record).
    pub fn record_toggle(&mut self) {
        if self.rec.recording { self.stop_recording() } else { self.start_recording() }
    }

    pub fn start_recording(&mut self) {
        if self.armed_with_input().is_empty() {
            self.set_status("Nothing to record: arm an audio track (R) and choose its input (In)", 5);
            return;
        }
        match self.rec.policy {
            InputPolicy::Device => {
                if !self.engine.device_running() {
                    self.set_status("Can't record: no audio device (offline mode)", 5);
                    return;
                }
                if self.engine.device_status().input_name.is_none() {
                    let why = self.engine.input_error().map(|e| format!(" ({e})")).unwrap_or_default();
                    self.set_status(format!("Can't record: no audio input device{why}. Choose one in Settings › Audio"), 6);
                    return;
                }
            }
            InputPolicy::InjectedOnly => {
                if self.engine.device_running() {
                    self.set_status("Can't record: injected-input recording needs the offline engine (--no-audio)", 5);
                    return;
                }
            }
        }
        let dir = self.recording_dir();
        if let Err(e) = std::fs::create_dir_all(&dir) {
            self.set_status(format!("Can't record: {}: {e}", dir.display()), 6);
            return;
        }
        match self.engine.start_recording(&dir) {
            Ok(()) => {
                self.rec.recording = true;
                self.set_status("Recording…", 2);
            }
            Err(e) => self.set_status(format!("Recording failed: {e}"), 6),
        }
        self.ui_dirty = true;
    }

    pub fn stop_recording(&mut self) {
        self.rec.recording = false;
        self.rec.live_vis.clear();
        self.ui_dirty = true;
        let takes = match self.engine.stop_recording() {
            Ok(t) => t,
            Err(e) => {
                self.set_status(format!("Stopping the recording failed: {e}"), 6);
                return;
            }
        };
        if takes.is_empty() {
            self.set_status("Recording stopped: no takes", 3);
            return;
        }
        // Decode + analyse on a worker thread, as imports do; the "Record" batch is
        // executed when the engine assets exist (JobResult::Recorded).
        let engine = self.engine.clone();
        let tx = self.jobs_tx.clone();
        let epoch = self.doc_epoch;
        self.pending_jobs += 1;
        std::thread::Builder::new()
            .name("recorded-takes".into())
            .spawn(move || {
                let takes = takes.into_iter().map(|t| { let r = load_and_analyse(&engine, &t.path); (t, r) }).collect();
                let _ = tx.send(JobResult::Recorded { doc_epoch: epoch, takes });
            })
            .expect("spawn take loader");
        self.set_status("Loading recording…", 30);
    }

    /// Worker finished loading the takes of one pass: ONE undoable `Batch` "Record".
    pub(crate) fn finish_recorded(&mut self, doc_epoch: u64, takes: Vec<(RecordedTake, Result<(AssetInfo, Pyramid), String>)>) {
        if doc_epoch != self.doc_epoch {
            for (_, r) in takes {
                if let Ok((info, _)) = r {
                    let _ = self.engine.release_asset(info.asset_id);
                }
            }
            return;
        }
        let mut next = self.project().next_id;
        let mut alloc = || {
            let id = next;
            next += 1;
            id
        };
        let mut cmds = Vec::new();
        let mut loaded = Vec::new();
        let mut errors = Vec::new();
        for (take, res) in takes {
            let (info, pyr) = match res {
                Ok(v) => v,
                Err(e) => {
                    errors.push(e);
                    continue;
                }
            };
            let track = TrackId(take.track_id);
            let Some(name) = self.audio_track(track).map(|t| t.name.clone()) else {
                let _ = self.engine.release_asset(info.asset_id);
                errors.push(format!("track {} is gone", take.track_id));
                continue;
            };
            let rate = if take.sample_rate > 0.0 { take.sample_rate } else { info.source_sample_rate };
            let frames = if take.frames > 0 { take.frames } else { (info.frames as f64 * info.source_sample_rate / self.rate).round() as u64 };
            if frames == 0 {
                let _ = self.engine.release_asset(info.asset_id);
                continue;
            }
            // Latency compensation can put the first sample before 0: trim it off.
            let skip = (-take.start_frame).max(0) as u64;
            if skip >= frames {
                let _ = self.engine.release_asset(info.asset_id);
                continue;
            }
            let asset_id = AssetId(alloc());
            let asset = Asset {
                id: asset_id,
                relative_path: None,
                absolute_path: std::fs::canonicalize(&take.path).unwrap_or_else(|_| take.path.clone()).to_string_lossy().into_owned(),
                sample_rate: rate,
                channels: take.channels.max(1),
                frames,
            };
            let start = self.project().frames_to_ticks(take.start_frame.max(0), rate);
            let clip = Clip { id: ClipId(alloc()), asset: asset_id, name, start, source_offset: skip, source_length: frames - skip, gain: 1.0, warp: Default::default() };
            cmds.push(Command::AddAsset { asset });
            cmds.push(Command::AddClip { track, clip });
            loaded.push((asset_id, info.asset_id, pyr));
        }
        if cmds.is_empty() {
            self.set_status(format!("Recording produced no usable takes{}", if errors.is_empty() { String::new() } else { format!(": {}", errors.join("; ")) }), 6);
            return;
        }
        for (asset_id, ea, pyr) in &loaded {
            self.peaks.insert(*ea, pyr.clone());
            self.asset_map.insert(*asset_id, *ea);
        }
        if self.execute(Command::Batch { label: "Record".into(), commands: cmds }) {
            self.rec.last_pass = Some("Record".into());
            if errors.is_empty() {
                self.set_status(format!("Recorded {} take(s)", loaded.len()), 3);
            } else {
                self.set_status(format!("Recorded {} take(s); skipped: {}", loaded.len(), errors.join("; ")), 6);
            }
        } else {
            for (asset_id, ea, _) in loaded {
                self.asset_map.remove(&asset_id);
                let _ = self.engine.release_asset(ea);
            }
        }
    }

    // ---------------------------------------------------------------- live clip

    /// Growing clips of the takes in progress (from `recording_state()` and
    /// `recording_peaks()`), plus follow-playhead while recording.
    pub fn rebuild_live(&mut self) {
        self.rec.live_vis.clear();
        if !self.rec.recording {
            return;
        }
        let st = self.engine.recording_state();
        let p = self.doc.project();
        let fpb = self.rate * 60.0 / p.transport.bpm;
        // About one peak pair per logical point at the current zoom; the engine works in
        // multiples of 64 frames, so round to that and scale the pair width to match.
        let frames_per_pt = fpb / self.view.px_per_beat;
        let fpp = ((frames_per_pt / 64.0).round().max(1.0) * 64.0) as u32;
        for (k, t) in st.takes.iter().enumerate() {
            let Some(ti) = p.track_index(TrackId(t.track_id)) else { continue };
            let start_beats = p.frames_to_ticks(t.start_frame, self.rate) as f64 / PPQ as f64;
            let len_beats = t.frames as f64 / fpb;
            let max_pairs = ((t.frames as f64 / fpp as f64).ceil() as usize + 1).min(8192);
            // recording_peaks() serves the newest take of a track (a seek starts a new take).
            let newest = !st.takes[k + 1..].iter().any(|n| n.track_id == t.track_id);
            let peaks = if newest { self.engine.recording_peaks(t.track_id, fpp, max_pairs) } else { Vec::new() };
            self.rec.live_vis.push(ClipVis {
                track_index: ti,
                start_beats,
                len_beats: len_beats.max(0.0),
                color: crate::controller::RECORD_RED,
                selected: false,
                muted: false,
                name: "Recording".into(),
                wave: None,
                midi: None,
                live: Some((peaks, (fpp as f64 / frames_per_pt) as f32)),
            });
        }
        // Follow the playhead: page forward when it nears the right edge.
        let ph = self.playhead_ticks() as f64 / PPQ as f64;
        let x = self.view.x_of(ph);
        let w = self.lanes_size.0;
        if x > w * 0.92 || x < 0.0 {
            self.view.start_beats = (ph - (w as f64 * 0.1) / self.view.px_per_beat).max(0.0);
        }
    }

    pub(crate) fn push_recording(&self, s: &Studio) {
        s.set_recording(self.rec.recording);
        s.set_record_ready(!self.armed_with_input().is_empty());
        let sel = self.selected_track.and_then(|t| self.project().track(t));
        s.set_sel_arm(sel.is_some_and(|t| t.arm));
        s.set_sel_monitor(sel.is_some_and(|t| t.monitor));
        s.set_sel_input_text(input_label(sel.and_then(|t| t.input)).into());
        match &self.rec.input_menu {
            Some(items) => {
                let cur = sel.and_then(|t| t.input);
                let mut y = IN_PAD;
                let mut rows = Vec::new();
                for (i, (label, src)) in items.iter().enumerate() {
                    rows.push(ChoiceItem { label: label.clone().into(), checked: *src == cur, hint: false, y, h: IN_ROW });
                    y += IN_ROW;
                    if i == 0 && !self.rec.input_hint.is_empty() {
                        rows.push(ChoiceItem { label: self.rec.input_hint.clone().into(), checked: false, hint: true, y, h: IN_HINT_ROW });
                        y += IN_HINT_ROW;
                    }
                }
                s.set_input_menu_h(y + IN_PAD);
                s.set_input_menu_w(if self.rec.input_hint.is_empty() { 136.0 } else { 200.0 });
                sync_model(&self.input_menu_model, rows);
                s.set_input_menu_open(true);
            }
            None => s.set_input_menu_open(false),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn dev(ch: u32) -> AudioDeviceInfo {
        AudioDeviceInfo { id: "x".into(), name: "X".into(), input_channels: ch, output_channels: 0, sample_rates: vec![], is_default_input: false, is_default_output: false }
    }
    #[test]
    fn choices() {
        let labels = |d: Option<&AudioDeviceInfo>| input_choices(d).into_iter().map(|(l, _)| l).collect::<Vec<_>>();
        assert_eq!(labels(None), ["None"]);
        assert_eq!(labels(Some(&dev(1))), ["None", "Input 1 (Mono)"]);
        assert_eq!(labels(Some(&dev(2))), ["None", "Input 1 (Mono)", "Input 2 (Mono)", "Input 1/2 (Stereo)"]);
        assert_eq!(labels(Some(&dev(5))).len(), 1 + 5 + 2);
        let c = input_choices(Some(&dev(4)));
        assert_eq!(c.last().unwrap().1, Some(InputSource { channel: 2, stereo: true }));
        assert_eq!(input_label(None), "Select audio input");
    }
}
