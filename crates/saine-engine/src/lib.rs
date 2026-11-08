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
    /// (`se_session_publish_v4`: audio clips plus, on instrument tracks,
    /// MIDI notes rendered by the engine's built-in synth with the track's
    /// [`SynthParams`]; `synth: None` uses the engine defaults, which equal
    /// `SynthParams::default()`; plus each track's record arm, input
    /// monitoring and input routing).
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
        let tracks: Vec<ffi::se_track_desc_v4> = s
            .tracks
            .iter()
            .zip(clip_descs.iter().zip(note_descs.iter().zip(&synth_descs)))
            .map(|(t, (clips, (notes, synth)))| ffi::se_track_desc_v4 {
                track_id: t.track_id,
                gain: t.gain,
                pan: t.pan,
                mute: t.mute as u8,
                solo: t.solo as u8,
                instrument: t.instrument as u8,
                arm: t.arm as u8,
                monitor: t.monitor as u8,
                input_mode: match t.input {
                    None => ffi::SE_INPUT_NONE,
                    Some(i) if i.stereo => ffi::SE_INPUT_STEREO,
                    Some(_) => ffi::SE_INPUT_MONO,
                },
                input_channel: t.input.map_or(0, |i| i.channel),
                clips: clips.as_ptr(),
                clip_count: clips.len() as u32,
                note_count: notes.len() as u32,
                notes: notes.as_ptr(),
                synth: synth.as_ref().map_or(std::ptr::null(), |p| p as *const ffi::se_synth_params),
            })
            .collect();
        let desc = ffi::se_session_desc_v4 { tracks: tracks.as_ptr(), track_count: tracks.len() as u32, master_gain: s.master_gain };
        // SAFETY: all pointers stay valid for the duration of the call; the engine copies.
        check(unsafe { ffi::se_session_publish_v4(self.raw, &desc) })
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
// Contract for recording/input (docs/adr/0002-recording.md, engine/README.md
// "Input, monitoring and recording").

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

/// Requested device setup (Settings › Audio). `None`/0 = system default,
/// except `input`:
///
/// `input`: `None` = **no input** (output only; nothing ever opens the
/// microphone), `Some(id)` = that input device, `Some(DEFAULT_INPUT)` (`""`)
/// = the system default input. An input is opened only when one is named
/// here, so a fresh engine or an output-only change never triggers the macOS
/// microphone prompt. `sample_rate` is the device's nominal rate: with an
/// input it must be 0 or the engine rate (the input path does not resample);
/// output-only, 0 leaves the device alone and another rate is converted by
/// the output unit. The engine rate itself never changes.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DeviceConfig {
    pub output: Option<String>,
    pub input: Option<String>,
    pub sample_rate: f64,
    pub buffer_size: u32,
}

impl DeviceConfig {
    /// `input: Some(DEFAULT_INPUT.into())` selects the system default input.
    pub const DEFAULT_INPUT: &'static str = "";
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
    /// Input-side latency in engine frames (device + safety offset + stream
    /// + one buffer). Recorded takes are compensated by
    /// `input_latency_frames + output_latency_frames`.
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

/// Recording health counters ([`Engine::recording_stats`]).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RecordingStats {
    /// Audio blocks dropped because a track's ring was full (writer too slow).
    pub overruns: u64,
    pub dropped_frames: u64,
    /// Takes that could not be written completely (left out of the result).
    pub write_errors: u64,
    pub takes: u64,
}

fn c_chars_to_string(chars: &[std::ffi::c_char]) -> String {
    let bytes: Vec<u8> = chars.iter().take_while(|&&c| c != 0).map(|&c| c as u8).collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

impl Engine {
    /// Enumerate audio devices (reads properties only; never opens a device).
    pub fn list_devices(&self) -> Result<Vec<AudioDeviceInfo>, EngineError> {
        let mut total = 0usize;
        check(unsafe { ffi::se_device_list(std::ptr::null_mut(), 0, &mut total) })?;
        loop {
            let mut raw = vec![ffi::se_device_info::default(); total];
            let mut n = 0usize;
            check(unsafe { ffi::se_device_list(raw.as_mut_ptr(), raw.len(), &mut n) })?;
            if n > raw.len() {
                total = n; // a device appeared in between: retry
                continue;
            }
            return Ok(raw[..n]
                .iter()
                .map(|d| AudioDeviceInfo {
                    id: c_chars_to_string(&d.uid),
                    name: c_chars_to_string(&d.name),
                    input_channels: d.input_channels,
                    output_channels: d.output_channels,
                    sample_rates: d.sample_rates[..(d.sample_rate_count as usize).min(ffi::SE_DEVICE_MAX_RATES)].to_vec(),
                    is_default_input: d.is_default_input != 0,
                    is_default_output: d.is_default_output != 0,
                })
                .collect());
        }
    }

    /// Apply a device configuration. Everything is validated first without
    /// opening anything (unknown id → `SE_ERR_NOT_FOUND`, unsupported
    /// rate/buffer/combination → `SE_ERR_INVALID_ARG`). If the device is
    /// stopped the setup is used by the next [`Engine::start_device`]; if it
    /// runs it is restarted. If the output opens but the input cannot (e.g.
    /// microphone permission denied), output keeps running, `device_status()
    /// .input_name` is `None`, [`Engine::input_error`] says why, and a
    /// running restart returns `SE_ERR_DEVICE` with that message.
    pub fn configure_device(&self, cfg: &DeviceConfig) -> Result<(), EngineError> {
        let out = cfg.output.as_deref().map(|s| CString::new(s)).transpose().map_err(|_| nul_error())?;
        let input_enabled = cfg.input.is_some();
        let inp = cfg.input.as_deref().filter(|s| !s.is_empty()).map(CString::new).transpose().map_err(|_| nul_error())?;
        let c = ffi::se_device_config {
            output_uid: out.as_ref().map_or(std::ptr::null(), |s| s.as_ptr()),
            input_uid: inp.as_ref().map_or(std::ptr::null(), |s| s.as_ptr()),
            sample_rate: cfg.sample_rate,
            buffer_size: cfg.buffer_size,
            input_enabled: input_enabled as u8,
            _pad: [0; 3],
        };
        // SAFETY: the strings outlive the call; the engine copies them.
        check(unsafe { ffi::se_device_configure(self.raw, &c) })
    }

    /// Current device status.
    pub fn device_status(&self) -> DeviceStatus {
        let mut st = ffi::se_device_status::default();
        if unsafe { ffi::se_device_status_get(self.raw, &mut st) } != ffi::SE_OK {
            return DeviceStatus { running: self.device_running(), sample_rate: self.sample_rate(), ..Default::default() };
        }
        DeviceStatus {
            running: st.running != 0,
            output_name: c_chars_to_string(&st.output_name),
            input_name: (st.input_open != 0).then(|| c_chars_to_string(&st.input_name)),
            sample_rate: st.sample_rate,
            buffer_size: st.buffer_size,
            input_channels: st.input_channels,
            output_channels: st.output_channels,
            input_latency_frames: st.input_latency_frames,
            output_latency_frames: st.output_latency_frames,
        }
    }

    /// Why the configured input is not open (permission denied, device
    /// gone, ...), if it is not.
    pub fn input_error(&self) -> Option<String> {
        let mut st = ffi::se_device_status::default();
        unsafe { ffi::se_device_status_get(self.raw, &mut st) };
        Some(c_chars_to_string(&st.input_error)).filter(|s| !s.is_empty())
    }

    /// Silence all device input (Settings › "Mute audio input"): monitoring,
    /// input meters and recordings get zeros (recordings keep their timing).
    pub fn set_input_muted(&self, muted: bool) {
        unsafe { ffi::se_input_set_muted(self.raw, muted as u8) };
    }

    /// Start capturing every armed track that has an input (as published),
    /// from the current transport position at the next audio block; starts
    /// the transport if stopped. Takes are written to `dir` (created if
    /// missing) by a non-real-time writer thread. Errors with
    /// `SE_ERR_STATE` when already recording, when no armed track has an
    /// input, or when the device runs without an open input.
    pub fn start_recording(&self, dir: &Path) -> Result<(), EngineError> {
        let p = cpath(dir)?;
        check(unsafe { ffi::se_record_start(self.raw, p.as_ptr()) })
    }

    /// Stop capturing (the transport keeps its state) and return the
    /// finalized takes. A seek/stop during recording ends a take and starts
    /// a new one at the new position; looping records linearly (one take).
    pub fn stop_recording(&self) -> Result<Vec<RecordedTake>, EngineError> {
        let mut n = 0usize;
        check(unsafe { ffi::se_record_stop(self.raw, &mut n) })?;
        let mut takes = Vec::with_capacity(n);
        for i in 0..n {
            let mut t = ffi::se_recorded_take::default();
            check(unsafe { ffi::se_record_get_take(self.raw, i, &mut t) })?;
            takes.push(RecordedTake {
                track_id: t.track_id,
                path: PathBuf::from(c_chars_to_string(&t.path)),
                start_frame: t.start_frame,
                frames: t.frames,
                channels: t.channels,
                sample_rate: t.sample_rate,
            });
        }
        Ok(takes)
    }

    /// Live recording state for drawing growing clips (cheap; any thread).
    pub fn recording_state(&self) -> RecordingState {
        let mut active = 0u8;
        let mut n = 0usize;
        if unsafe { ffi::se_record_state(self.raw, &mut active, std::ptr::null_mut(), 0, &mut n) } != ffi::SE_OK {
            return RecordingState::default();
        }
        let mut raw = vec![ffi::se_live_take::default(); n + 8]; // slack for takes started in between
        let mut m = 0usize;
        unsafe { ffi::se_record_state(self.raw, &mut active, raw.as_mut_ptr(), raw.len(), &mut m) };
        raw.truncate(m.min(raw.len()));
        RecordingState {
            active: active != 0,
            takes: raw.iter().map(|t| LiveTake { track_id: t.track_id, start_frame: t.start_frame, frames: t.frames }).collect(),
        }
    }

    /// Min/max peaks of a take in progress (mono mix), for the live
    /// waveform: the newest take of `track_id`, from its first frame, at
    /// 64-frame resolution (`frames_per_peak` is rounded to multiples of 64).
    pub fn recording_peaks(&self, track_id: u64, frames_per_peak: u32, max_pairs: usize) -> Vec<[f32; 2]> {
        let mut buf = vec![0f32; max_pairs * 2];
        let mut n = 0usize;
        if unsafe { ffi::se_record_peaks(self.raw, track_id, frames_per_peak.max(1), buf.as_mut_ptr(), max_pairs, &mut n) } != ffi::SE_OK {
            return Vec::new();
        }
        buf.truncate(n * 2);
        buf.chunks_exact(2).map(|c| [c[0], c[1]]).collect()
    }

    /// Overrun / error counters of the current recording (or of the last one
    /// after [`Engine::stop_recording`]).
    pub fn recording_stats(&self) -> RecordingStats {
        let mut s = ffi::se_record_stats::default();
        unsafe { ffi::se_record_stats_get(self.raw, &mut s) };
        RecordingStats { overruns: s.overruns, dropped_frames: s.dropped_frames, write_errors: s.write_errors, takes: s.takes }
    }

    /// Offline render with injected device input (tests): `input` is
    /// interleaved with `input_channels` channels and the same frame count as `out`/2.
    pub fn render_offline_with_input(&self, input: &[f32], input_channels: u32, out: &mut [f32]) -> Result<(), EngineError> {
        let frames = out.len() / 2;
        if input_channels == 0 || input.len() != frames * input_channels as usize {
            return Err(EngineError { code: ffi::SE_ERR_INVALID_ARG, kind: "invalid argument", message: "input length must be frames * input_channels".into() });
        }
        check(unsafe { ffi::se_render_offline_with_input(self.raw, input.as_ptr(), input_channels, out.as_mut_ptr(), frames as u32) })
    }
}

fn nul_error() -> EngineError {
    EngineError { code: ffi::SE_ERR_INVALID_ARG, kind: "invalid argument", message: "string contains NUL".into() }
}

// ---------------------------------------------------------------- plugins

fn unsupported(what: &str) -> EngineError {
    EngineError { code: ffi::SE_ERR_UNSUPPORTED_FORMAT, kind: "unsupported", message: format!("{what} is not implemented yet") }
}
//
// Contract for plugin hosting (docs/adr/0003-plugins.md). The app codes
// against these signatures. Bodies marked STUB are replaced by the engine
// implementation; until then they return `unsupported()`.

/// One scanned plugin.
#[derive(Debug, Clone, PartialEq)]
pub struct PluginInfo {
    pub plugin: saine_core::PluginRef,
    pub has_editor: bool,
    pub audio_inputs: u32,
    pub audio_outputs: u32,
}

/// Result of a scan. `failed` lists plugins that crashed, timed out or
/// failed validation in the out-of-process scanner (they are blocklisted
/// until a forced rescan).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ScanReport {
    pub plugins: Vec<PluginInfo>,
    pub failed: Vec<(String, String)>,
    pub from_cache: bool,
}

/// STUB. Scan AU/VST3/CLAP plugins in the standard locations using the
/// out-of-process scanner, caching results in `cache_path` (JSON). With
/// `rescan == false` unchanged bundles come from the cache. Blocking: call
/// from a worker thread. `progress` receives a short status per plugin.
pub fn scan_plugins(_cache_path: &Path, _rescan: bool, _progress: &mut dyn FnMut(&str)) -> Result<ScanReport, EngineError> {
    Err(unsupported("plugin scanning"))
}

impl Engine {
    /// STUB. Current state blobs (base64) of all live plugin instances, for
    /// `Document::sync_device_states` before saving.
    pub fn plugin_states(&self) -> Vec<(saine_core::DeviceId, String)> {
        Vec::new()
    }

    /// STUB. Devices whose plugin could not be instantiated (missing, failed
    /// to load, wrong format) with a reason. They are passed through silently.
    pub fn plugin_load_errors(&self) -> Vec<(saine_core::DeviceId, String)> {
        Vec::new()
    }

    /// STUB. Open the plugin's native editor in its own window. MAIN THREAD ONLY.
    pub fn open_plugin_editor(&self, _device: saine_core::DeviceId) -> Result<(), EngineError> {
        Err(unsupported("plugin editors"))
    }

    /// STUB. Close the editor window if open. MAIN THREAD ONLY.
    pub fn close_plugin_editor(&self, _device: saine_core::DeviceId) {}

    /// STUB. Whether the device's editor window is open.
    pub fn plugin_editor_is_open(&self, _device: saine_core::DeviceId) -> bool {
        false
    }

    /// STUB. Reported processing latency of a device, in frames (0 if unknown).
    pub fn plugin_latency(&self, _device: saine_core::DeviceId) -> u32 {
        0
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
                    arm: false,
                    monitor: false,
                    input: None,
                    synth: Some(p.clone()),
                    devices: vec![],
                    instrument_plugin: None,
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
                arm: false,
                monitor: false,
                input: None,
                synth: Some(bad),
                devices: vec![],
                instrument_plugin: None,
            }],
            master_gain: 1.0,
        };
        assert_eq!(e.publish(&snap).unwrap_err().code, ffi::SE_ERR_INVALID_ARG);
    }
}
