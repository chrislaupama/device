//! Safe wrapper around the C++ audio engine.
//!
//! The [`Engine`] handle is `Send + Sync`: every C entry point is documented
//! as callable from any non-real-time thread. The real-time audio callback
//! lives entirely inside the C++ library.

pub mod ffi;

use saine_core::instrument::{FilterMode, SynthParams, Waveform};
use saine_core::snapshot::SessionSnapshot;
use std::ffi::{CStr, CString};
use std::path::{Path, PathBuf};
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

    /// Publish an immutable session snapshot to the audio thread
    /// (`se_session_publish_v3`: audio clips plus, on instrument tracks,
    /// MIDI notes rendered by the engine's built-in synth with the track's
    /// [`SynthParams`]; `synth: None` uses the engine defaults, which equal
    /// `SynthParams::default()`).
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
        let note_descs: Vec<Vec<ffi::se_note_desc>> = s
            .tracks
            .iter()
            .map(|t| {
                if !t.instrument {
                    return Vec::new();
                }
                t.notes.iter().map(|n| ffi::se_note_desc { start: n.start, end: n.end, pitch: n.pitch, velocity: n.velocity, _pad: [0; 6] }).collect()
            })
            .collect();
        // Parameters only matter on instrument tracks; the engine copies them.
        let synth_descs: Vec<Option<ffi::se_synth_params>> =
            s.tracks.iter().map(|t| if t.instrument { t.synth.as_ref().map(synth_params_to_ffi) } else { None }).collect();
        let tracks: Vec<ffi::se_track_desc_v3> = s
            .tracks
            .iter()
            .zip(clip_descs.iter().zip(note_descs.iter().zip(&synth_descs)))
            .map(|(t, (clips, (notes, synth)))| ffi::se_track_desc_v3 {
                track_id: t.track_id,
                gain: t.gain,
                pan: t.pan,
                mute: t.mute as u8,
                solo: t.solo as u8,
                instrument: t.instrument as u8,
                _pad: [0; 5],
                clips: clips.as_ptr(),
                clip_count: clips.len() as u32,
                note_count: notes.len() as u32,
                notes: notes.as_ptr(),
                synth: synth.as_ref().map_or(std::ptr::null(), |p| p as *const ffi::se_synth_params),
            })
            .collect();
        let desc = ffi::se_session_desc_v3 { tracks: tracks.as_ptr(), track_count: tracks.len() as u32, master_gain: s.master_gain };
        // SAFETY: all pointers stay valid for the duration of the call; the engine copies.
        check(unsafe { ffi::se_session_publish_v3(self.raw, &desc) })
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

fn wave_to_ffi(w: Waveform) -> u8 {
    match w {
        Waveform::Saw => ffi::SE_WAVE_SAW,
        Waveform::Square => ffi::SE_WAVE_SQUARE,
        Waveform::Triangle => ffi::SE_WAVE_TRIANGLE,
        Waveform::Sine => ffi::SE_WAVE_SINE,
    }
}

/// Maps the model's [`SynthParams`] onto the engine's flat C struct (the
/// preset name is display-only and not sent). Values are passed through
/// unchanged; the engine validates the documented ranges on publish.
pub fn synth_params_to_ffi(p: &SynthParams) -> ffi::se_synth_params {
    let [o1, o2] = &p.osc;
    ffi::se_synth_params {
        osc1_wave: wave_to_ffi(o1.wave),
        osc1_octave: o1.octave,
        osc1_semitones: o1.semitones,
        osc2_wave: wave_to_ffi(o2.wave),
        osc2_octave: o2.octave,
        osc2_semitones: o2.semitones,
        filter_mode: match p.filter.mode {
            FilterMode::LowPass => ffi::SE_FILTER_LOWPASS,
            FilterMode::HighPass => ffi::SE_FILTER_HIGHPASS,
            FilterMode::BandPass => ffi::SE_FILTER_BANDPASS,
        },
        lfo_wave: wave_to_ffi(p.lfo.wave),
        polyphony: p.voice.polyphony,
        unison: p.voice.unison,
        _pad: [0; 6],
        osc1_shape: o1.shape,
        osc1_fine_cents: o1.fine_cents,
        osc1_level: o1.level,
        osc2_shape: o2.shape,
        osc2_fine_cents: o2.fine_cents,
        osc2_level: o2.level,
        cross_fm: p.cross_fm,
        sub_level: p.sub_level,
        noise_level: p.noise_level,
        filter_cutoff_hz: p.filter.cutoff_hz,
        filter_resonance: p.filter.resonance,
        filter_key_track: p.filter.key_track,
        filter_env_amount: p.filter.env_amount,
        amp_attack: p.amp_env.attack,
        amp_decay: p.amp_env.decay,
        amp_sustain: p.amp_env.sustain,
        amp_release: p.amp_env.release,
        filter_attack: p.filter_env.attack,
        filter_decay: p.filter_env.decay,
        filter_sustain: p.filter_env.sustain,
        filter_release: p.filter_env.release,
        lfo_rate_hz: p.lfo.rate_hz,
        lfo_to_pitch_cents: p.lfo.to_pitch_cents,
        lfo_to_cutoff_oct: p.lfo.to_cutoff_oct,
        lfo_to_amp: p.lfo.to_amp,
        glide: p.voice.glide,
        unison_detune_cents: p.voice.unison_detune_cents,
        output: p.output,
    }
}

// ---------------------------------------------------------------- devices & recording
//
// Contract for recording/input (docs/adr/0002-recording.md). The app codes
// against these signatures. Bodies marked STUB are replaced by the engine
// implementation; until then they return `unsupported()`.

/// An audio device as reported by the platform backend.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioDeviceInfo {
    /// Stable identifier (Core Audio device UID on macOS).
    pub id: String,
    pub name: String,
    pub input_channels: u32,
    pub output_channels: u32,
    pub sample_rates: Vec<f64>,
    pub is_default_input: bool,
    pub is_default_output: bool,
}

/// Requested device setup (Settings › Audio). `None`/0 = system default.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DeviceConfig {
    pub output: Option<String>,
    pub input: Option<String>,
    pub sample_rate: f64,
    pub buffer_size: u32,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct DeviceStatus {
    pub running: bool,
    pub output_name: String,
    pub input_name: Option<String>,
    pub sample_rate: f64,
    pub buffer_size: u32,
    pub input_channels: u32,
    pub output_channels: u32,
    /// Round-trip compensation applied to recorded takes, in frames.
    pub input_latency_frames: u32,
    pub output_latency_frames: u32,
}

/// A finished recording on one track.
#[derive(Debug, Clone, PartialEq)]
pub struct RecordedTake {
    pub track_id: u64,
    /// 32-bit float WAV at the engine sample rate.
    pub path: PathBuf,
    /// Timeline frame of the first sample (latency compensated).
    pub start_frame: i64,
    pub frames: u64,
    pub channels: u32,
    pub sample_rate: f64,
}

/// A take still being recorded.
#[derive(Debug, Clone, PartialEq)]
pub struct LiveTake {
    pub track_id: u64,
    pub start_frame: i64,
    pub frames: u64,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RecordingState {
    pub active: bool,
    pub takes: Vec<LiveTake>,
}

fn unsupported(what: &str) -> EngineError {
    EngineError { code: ffi::SE_ERR_UNSUPPORTED_FORMAT, kind: "unsupported", message: format!("{what} is not implemented yet") }
}

impl Engine {
    /// STUB. Enumerate audio devices.
    pub fn list_devices(&self) -> Result<Vec<AudioDeviceInfo>, EngineError> {
        Err(unsupported("device enumeration"))
    }

    /// STUB. Apply a device configuration (restarts the device if running).
    pub fn configure_device(&self, _cfg: &DeviceConfig) -> Result<(), EngineError> {
        Err(unsupported("device configuration"))
    }

    /// STUB. Current device status.
    pub fn device_status(&self) -> DeviceStatus {
        DeviceStatus { running: self.device_running(), sample_rate: self.sample_rate(), ..Default::default() }
    }

    /// STUB. Silence all device input (Settings › "Mute audio input").
    pub fn set_input_muted(&self, _muted: bool) {}

    /// STUB. Start capturing every armed track that has an input, from the
    /// current transport position (starts the transport if stopped). Takes
    /// are written to `dir` by a non-real-time writer thread.
    pub fn start_recording(&self, _dir: &Path) -> Result<(), EngineError> {
        Err(unsupported("recording"))
    }

    /// STUB. Stop capturing (the transport keeps its state) and return the
    /// finalized takes.
    pub fn stop_recording(&self) -> Result<Vec<RecordedTake>, EngineError> {
        Err(unsupported("recording"))
    }

    /// STUB. Live recording state for drawing growing clips.
    pub fn recording_state(&self) -> RecordingState {
        RecordingState::default()
    }

    /// STUB. Min/max peaks of a take in progress (mono mix), for the live waveform.
    pub fn recording_peaks(&self, _track_id: u64, _frames_per_peak: u32, _max_pairs: usize) -> Vec<[f32; 2]> {
        Vec::new()
    }

    /// STUB. Offline render with injected device input (tests): `input` is
    /// interleaved with `input_channels` channels and the same frame count as `out`/2.
    pub fn render_offline_with_input(&self, _input: &[f32], _input_channels: u32, _out: &mut [f32]) -> Result<(), EngineError> {
        Err(unsupported("input injection"))
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        // SAFETY: we own the handle exclusively here.
        unsafe { ffi::se_engine_destroy(self.raw) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use saine_core::instrument::{factory_presets, Adsr, Filter, Lfo, Osc, Voice};

    #[test]
    fn default_synth_params_map_to_engine_defaults() {
        let mut engine_default = ffi::se_synth_params::default();
        // SAFETY: valid out pointer.
        unsafe { ffi::se_synth_params_default(&mut engine_default) };
        assert_eq!(synth_params_to_ffi(&SynthParams::default()), engine_default);
    }

    #[test]
    fn synth_params_mapping_covers_every_field() {
        // Distinct, non-default values everywhere so a swapped field shows.
        let p = SynthParams {
            osc: [
                Osc { wave: Waveform::Triangle, shape: 0.11, octave: -2, semitones: 7, fine_cents: -12.5, level: 0.21 },
                Osc { wave: Waveform::Sine, shape: 0.12, octave: 3, semitones: -11, fine_cents: 33.0, level: 0.22 },
            ],
            cross_fm: 0.31,
            sub_level: 0.32,
            noise_level: 0.33,
            filter: Filter { mode: FilterMode::BandPass, cutoff_hz: 1234.0, resonance: 0.41, key_track: 0.42, env_amount: -0.43 },
            amp_env: Adsr { attack: 0.051, decay: 0.052, sustain: 0.53, release: 0.054 },
            filter_env: Adsr { attack: 0.061, decay: 0.062, sustain: 0.63, release: 0.064 },
            lfo: Lfo { wave: Waveform::Square, rate_hz: 7.1, to_pitch_cents: 72.0, to_cutoff_oct: 0.73, to_amp: 0.74 },
            voice: Voice { polyphony: 3, glide: 0.81, unison: 4, unison_detune_cents: 8.2 },
            output: 1.9,
            preset: "Mapping".into(),
        };
        assert!(p.validate().is_ok());
        let c = synth_params_to_ffi(&p);
        assert_eq!((c.osc1_wave, c.osc1_octave, c.osc1_semitones), (ffi::SE_WAVE_TRIANGLE, -2, 7));
        assert_eq!((c.osc2_wave, c.osc2_octave, c.osc2_semitones), (ffi::SE_WAVE_SINE, 3, -11));
        assert_eq!((c.osc1_shape, c.osc1_fine_cents, c.osc1_level), (0.11, -12.5, 0.21));
        assert_eq!((c.osc2_shape, c.osc2_fine_cents, c.osc2_level), (0.12, 33.0, 0.22));
        assert_eq!((c.cross_fm, c.sub_level, c.noise_level), (0.31, 0.32, 0.33));
        assert_eq!(c.filter_mode, ffi::SE_FILTER_BANDPASS);
        assert_eq!((c.filter_cutoff_hz, c.filter_resonance, c.filter_key_track, c.filter_env_amount), (1234.0, 0.41, 0.42, -0.43));
        assert_eq!((c.amp_attack, c.amp_decay, c.amp_sustain, c.amp_release), (0.051, 0.052, 0.53, 0.054));
        assert_eq!((c.filter_attack, c.filter_decay, c.filter_sustain, c.filter_release), (0.061, 0.062, 0.63, 0.064));
        assert_eq!(c.lfo_wave, ffi::SE_WAVE_SQUARE);
        assert_eq!((c.lfo_rate_hz, c.lfo_to_pitch_cents, c.lfo_to_cutoff_oct, c.lfo_to_amp), (7.1, 72.0, 0.73, 0.74));
        assert_eq!((c.polyphony, c.glide, c.unison, c.unison_detune_cents), (3, 0.81, 4, 8.2));
        assert_eq!(c.output, 1.9);
        assert_eq!(c._pad, [0; 6]);
        for (w, code) in [(Waveform::Saw, 0), (Waveform::Square, 1), (Waveform::Triangle, 2), (Waveform::Sine, 3)] {
            assert_eq!(wave_to_ffi(w), code);
        }
    }

    #[test]
    fn every_factory_preset_is_accepted_by_the_engine() {
        let e = Engine::new(Config::default()).unwrap();
        for p in factory_presets() {
            let snap = SessionSnapshot {
                tracks: vec![saine_core::snapshot::TrackSnapshot {
                    track_id: 1,
                    gain: 1.0,
                    pan: 0.0,
                    mute: false,
                    solo: false,
                    clips: vec![],
                    instrument: true,
                    notes: vec![],
                    synth: Some(p.clone()),
                }],
                master_gain: 1.0,
            };
            e.publish(&snap).unwrap_or_else(|err| panic!("{}: {err}", p.preset));
        }
        // Out-of-range values are rejected by the engine.
        let mut bad = SynthParams::default();
        bad.voice.unison = 9;
        let snap = SessionSnapshot {
            tracks: vec![saine_core::snapshot::TrackSnapshot {
                track_id: 1,
                gain: 1.0,
                pan: 0.0,
                mute: false,
                solo: false,
                clips: vec![],
                instrument: true,
                notes: vec![],
                synth: Some(bad),
            }],
            master_gain: 1.0,
        };
        assert_eq!(e.publish(&snap).unwrap_err().code, ffi::SE_ERR_INVALID_ARG);
    }
}
