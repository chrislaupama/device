//! Safe wrapper around the C++ audio engine.
//!
//! The [`Engine`] handle is `Send + Sync`: every C entry point is documented
//! as callable from any non-real-time thread. The real-time audio callback
//! lives entirely inside the C++ library.

pub mod ffi;

use saine_core::snapshot::SessionSnapshot;
use std::ffi::{CStr, CString};
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq)]
#[error("{kind}: {message}")]
pub struct EngineError {
    pub code: i32,
    pub kind: &'static str,
    pub message: String,
}

fn kind_name(code: i32) -> &'static str {
    match code {
        ffi::SE_ERR_INVALID_ARG => "invalid argument",
        ffi::SE_ERR_IO => "i/o error",
        ffi::SE_ERR_UNSUPPORTED_FORMAT => "unsupported format",
        ffi::SE_ERR_DEVICE => "audio device error",
        ffi::SE_ERR_NOT_FOUND => "not found",
        ffi::SE_ERR_STATE => "invalid state",
        ffi::SE_ERR_OUT_OF_MEMORY => "out of memory",
        _ => "engine error",
    }
}

fn check(code: i32) -> Result<(), EngineError> {
    if code == ffi::SE_OK {
        return Ok(());
    }
    // SAFETY: se_last_error returns a thread-local NUL-terminated string or NULL.
    let message = unsafe {
        let p = ffi::se_last_error();
        if p.is_null() { String::new() } else { CStr::from_ptr(p).to_string_lossy().into_owned() }
    };
    Err(EngineError { code, kind: kind_name(code), message })
}

fn cpath(p: &Path) -> Result<CString, EngineError> {
    CString::new(p.to_string_lossy().as_bytes()).map_err(|_| EngineError { code: ffi::SE_ERR_INVALID_ARG, kind: "invalid argument", message: "path contains NUL".into() })
}

#[derive(Debug, Clone, Copy)]
pub struct Config {
    pub sample_rate: f64,
    pub max_block_size: u32,
}

impl Default for Config {
    fn default() -> Self {
        Config { sample_rate: 48_000.0, max_block_size: 2048 }
    }
}

pub type AssetInfo = ffi::se_asset_info;
pub type Stats = ffi::se_stats;

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct TransportState {
    pub position: i64,
    pub playing: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Meter {
    pub track_id: u64,
    pub peak_l: f32,
    pub peak_r: f32,
}

/// Metronome settings for [`Engine::set_metronome`]. Beat `k` clicks at
/// timeline frame `round(k * frames_per_beat)`; every `beats_per_bar`-th
/// beat (from beat 0 at frame 0) is a downbeat. `gain` is linear and applied
/// after master gain. Not included in [`Engine::export_wav`] or the meters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MetronomeConfig {
    pub enabled: bool,
    pub gain: f32,
    pub frames_per_beat: f64,
    pub beats_per_bar: u32,
}

pub struct Engine {
    raw: *mut ffi::se_engine,
}

// SAFETY: the C ABI is documented thread-safe for non-RT callers; the handle
// is only destroyed in Drop, which requires exclusive ownership.
unsafe impl Send for Engine {}
unsafe impl Sync for Engine {}

impl Engine {
    pub fn new(cfg: Config) -> Result<Self, EngineError> {
        let c = ffi::se_config { abi_version: ffi::SE_ABI_VERSION, sample_rate: cfg.sample_rate, max_block_size: cfg.max_block_size, output_channels: 2 };
        // SAFETY: valid pointer to a properly initialised config.
        let raw = unsafe { ffi::se_engine_create(&c) };
        if raw.is_null() {
            check(ffi::SE_ERR_STATE)?;
        }
        Ok(Engine { raw })
    }

    pub fn abi_version() -> u32 {
        unsafe { ffi::se_abi_version() }
    }

    pub fn start_device(&self) -> Result<(), EngineError> {
        check(unsafe { ffi::se_device_start(self.raw) })
    }

    pub fn stop_device(&self) -> Result<(), EngineError> {
        check(unsafe { ffi::se_device_stop(self.raw) })
    }

    pub fn device_running(&self) -> bool {
        unsafe { ffi::se_device_is_running(self.raw) != 0 }
    }

    pub fn sample_rate(&self) -> f64 {
        unsafe { ffi::se_engine_sample_rate(self.raw) }
    }

    /// Blocking decode + resample. Call from a worker thread.
    pub fn load_file(&self, path: &Path) -> Result<AssetInfo, EngineError> {
        let p = cpath(path)?;
        let mut info = AssetInfo::default();
        check(unsafe { ffi::se_asset_load_file(self.raw, p.as_ptr(), &mut info) })?;
        Ok(info)
    }

    pub fn release_asset(&self, id: u64) -> Result<(), EngineError> {
        check(unsafe { ffi::se_asset_release(self.raw, id) })
    }

    pub fn asset_info(&self, id: u64) -> Result<AssetInfo, EngineError> {
        let mut info = AssetInfo::default();
        check(unsafe { ffi::se_asset_get_info(self.raw, id, &mut info) })?;
        Ok(info)
    }

    /// Min/max pairs (mono mix) for waveform display.
    pub fn peaks(&self, id: u64, start_frame: u64, frames_per_peak: u32, max_pairs: usize) -> Result<Vec<[f32; 2]>, EngineError> {
        let mut buf = vec![0f32; max_pairs * 2];
        let mut n = 0usize;
        check(unsafe { ffi::se_asset_compute_peaks(self.raw, id, start_frame, frames_per_peak, buf.as_mut_ptr(), max_pairs, &mut n) })?;
        buf.truncate(n * 2);
        Ok(buf.chunks_exact(2).map(|c| [c[0], c[1]]).collect())
    }

    /// Publish an immutable session snapshot to the audio thread.
    pub fn publish(&self, s: &SessionSnapshot) -> Result<(), EngineError> {
        let clip_descs: Vec<Vec<ffi::se_clip_desc>> = s
            .tracks
            .iter()
            .map(|t| {
                t.clips
                    .iter()
                    .map(|c| ffi::se_clip_desc { clip_id: c.clip_id, asset_id: c.engine_asset, timeline_start: c.timeline_start, source_offset: c.source_offset, length: c.length, gain: c.gain })
                    .collect()
            })
            .collect();
        let tracks: Vec<ffi::se_track_desc> = s
            .tracks
            .iter()
            .zip(&clip_descs)
            .map(|(t, clips)| ffi::se_track_desc {
                track_id: t.track_id,
                gain: t.gain,
                pan: t.pan,
                mute: t.mute as u8,
                solo: t.solo as u8,
                _pad: [0; 6],
                clips: clips.as_ptr(),
                clip_count: clips.len() as u32,
            })
            .collect();
        let desc = ffi::se_session_desc { tracks: tracks.as_ptr(), track_count: tracks.len() as u32, master_gain: s.master_gain };
        // SAFETY: all pointers stay valid for the duration of the call; the engine copies.
        check(unsafe { ffi::se_session_publish(self.raw, &desc) })
    }

    pub fn play(&self) -> Result<(), EngineError> {
        check(unsafe { ffi::se_transport_play(self.raw) })
    }

    pub fn pause(&self) -> Result<(), EngineError> {
        check(unsafe { ffi::se_transport_pause(self.raw) })
    }

    pub fn stop(&self) -> Result<(), EngineError> {
        check(unsafe { ffi::se_transport_stop(self.raw) })
    }

    pub fn seek(&self, frame: i64) -> Result<(), EngineError> {
        check(unsafe { ffi::se_transport_seek(self.raw, frame) })
    }

    pub fn transport(&self) -> TransportState {
        let mut t = ffi::se_transport_info::default();
        unsafe { ffi::se_transport_get(self.raw, &mut t) };
        TransportState { position: t.position, playing: t.playing != 0 }
    }

    pub fn meters(&self, out: &mut Vec<Meter>, capacity: usize) {
        let mut raw = vec![ffi::se_meter::default(); capacity];
        let mut n = 0usize;
        if unsafe { ffi::se_meters_read(self.raw, raw.as_mut_ptr(), capacity, &mut n) } == ffi::SE_OK {
            out.clear();
            out.extend(raw[..n].iter().map(|m| Meter { track_id: m.track_id, peak_l: m.peak_l, peak_r: m.peak_r }));
        }
    }

    /// Non-destructive meter read: per meter (master first, then tracks in
    /// session order) the peak over the last ~50 ms of rendered audio.
    /// Unlike [`Engine::meters`] this resets nothing, so it can be polled at
    /// any rate.
    pub fn peek_meters(&self, out: &mut Vec<Meter>, capacity: usize) {
        let mut raw = vec![ffi::se_meter::default(); capacity];
        let mut n = 0usize;
        if unsafe { ffi::se_meters_peek(self.raw, raw.as_mut_ptr(), capacity, &mut n) } == ffi::SE_OK {
            out.clear();
            out.extend(raw[..n].iter().map(|m| Meter { track_id: m.track_id, peak_l: m.peak_l, peak_r: m.peak_r }));
        }
    }

    /// Loop playback over `[start, end)` frames. Enabling requires
    /// `start < end`; when disabling, `start`/`end` are ignored.
    pub fn set_loop(&self, enabled: bool, start: i64, end: i64) -> Result<(), EngineError> {
        check(unsafe { ffi::se_transport_set_loop(self.raw, enabled as u8, start, end) })
    }

    pub fn set_metronome(&self, cfg: MetronomeConfig) -> Result<(), EngineError> {
        let d = ffi::se_metronome_desc {
            enabled: cfg.enabled as u8,
            _pad: [0; 3],
            gain: cfg.gain,
            frames_per_beat: cfg.frames_per_beat,
            beats_per_bar: cfg.beats_per_bar,
            _pad2: 0,
        };
        // SAFETY: valid pointer for the duration of the call; the engine copies.
        check(unsafe { ffi::se_metronome_set(self.raw, &d) })
    }

    pub fn stats(&self) -> Stats {
        let mut s = Stats::default();
        unsafe { ffi::se_stats_get(self.raw, &mut s) };
        s
    }

    pub fn reset_stats(&self) {
        unsafe { ffi::se_stats_reset(self.raw) }
    }

    /// Render interleaved stereo offline (device must be stopped).
    pub fn render_offline(&self, out: &mut [f32]) -> Result<(), EngineError> {
        let frames = (out.len() / 2) as u32;
        check(unsafe { ffi::se_render_offline(self.raw, out.as_mut_ptr(), frames) })
    }

    pub fn export_wav(&self, path: &Path, start: i64, end: i64) -> Result<(), EngineError> {
        let p = cpath(path)?;
        check(unsafe { ffi::se_export_wav(self.raw, p.as_ptr(), start, end) })
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        // SAFETY: we own the handle exclusively here.
        unsafe { ffi::se_engine_destroy(self.raw) }
    }
}
