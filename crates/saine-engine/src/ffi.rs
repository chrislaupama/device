//! Raw declarations mirroring `engine/include/saine_engine.h` (ABI v1).
//! Hand-written to avoid a bindgen/libclang build dependency; layout is
//! checked by the `abi_layout` test.
#![allow(non_camel_case_types, dead_code)]

use std::ffi::c_char;

pub const SE_ABI_VERSION: u32 = 1;

pub const SE_OK: i32 = 0;
pub const SE_ERR_INVALID_ARG: i32 = -1;
pub const SE_ERR_IO: i32 = -2;
pub const SE_ERR_UNSUPPORTED_FORMAT: i32 = -3;
pub const SE_ERR_DEVICE: i32 = -4;
pub const SE_ERR_NOT_FOUND: i32 = -5;
pub const SE_ERR_STATE: i32 = -6;
pub const SE_ERR_OUT_OF_MEMORY: i32 = -7;

#[repr(C)]
pub struct se_engine {
    _private: [u8; 0],
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct se_config {
    pub abi_version: u32,
    pub sample_rate: f64,
    pub max_block_size: u32,
    pub output_channels: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct se_asset_info {
    pub asset_id: u64,
    pub frames: u64,
    pub channels: u32,
    pub source_sample_rate: f64,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct se_clip_desc {
    pub clip_id: u64,
    pub asset_id: u64,
    pub timeline_start: i64,
    pub source_offset: i64,
    pub length: i64,
    pub gain: f32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct se_track_desc {
    pub track_id: u64,
    pub gain: f32,
    pub pan: f32,
    pub mute: u8,
    pub solo: u8,
    pub _pad: [u8; 6],
    pub clips: *const se_clip_desc,
    pub clip_count: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct se_session_desc {
    pub tracks: *const se_track_desc,
    pub track_count: u32,
    pub master_gain: f32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct se_transport_info {
    pub position: i64,
    pub playing: u8,
    pub _pad: [u8; 7],
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct se_meter {
    pub track_id: u64,
    pub peak_l: f32,
    pub peak_r: f32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct se_stats {
    pub callbacks: u64,
    pub xruns_or_overloads: u64,
    pub last_callback_us: f64,
    pub max_callback_us: f64,
    pub avg_callback_us: f64,
    pub block_deadline_us: f64,
}

/// Appended to ABI v1 (see the "additions" section of saine_engine.h).
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct se_metronome_desc {
    pub enabled: u8,
    pub _pad: [u8; 3],
    pub gain: f32,
    pub frames_per_beat: f64,
    pub beats_per_bar: u32,
    pub _pad2: u32,
}

/// MIDI note for [`se_track_desc_v2`] (ABI v1 addition).
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct se_note_desc {
    pub start: i64,
    pub end: i64,
    pub pitch: u8,
    pub velocity: u8,
    pub _pad: [u8; 6],
}

/// v1 track fields plus the built-in instrument flag and notes.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct se_track_desc_v2 {
    pub track_id: u64,
    pub gain: f32,
    pub pan: f32,
    pub mute: u8,
    pub solo: u8,
    pub instrument: u8,
    pub _pad: [u8; 5],
    pub clips: *const se_clip_desc,
    pub clip_count: u32,
    pub note_count: u32,
    pub notes: *const se_note_desc,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct se_session_desc_v2 {
    pub tracks: *const se_track_desc_v2,
    pub track_count: u32,
    pub master_gain: f32,
}

/// Built-in synth parameters (ABI v1 addition), see `se_synth_params` in
/// saine_engine.h. Waveforms: 0 saw, 1 square, 2 triangle, 3 sine; filter
/// mode: 0 low-pass, 1 high-pass, 2 band-pass.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct se_synth_params {
    pub osc1_wave: u8,
    pub osc1_octave: i8,
    pub osc1_semitones: i8,
    pub osc2_wave: u8,
    pub osc2_octave: i8,
    pub osc2_semitones: i8,
    pub filter_mode: u8,
    pub lfo_wave: u8,
    pub polyphony: u8,
    pub unison: u8,
    pub _pad: [u8; 6],
    pub osc1_shape: f32,
    pub osc1_fine_cents: f32,
    pub osc1_level: f32,
    pub osc2_shape: f32,
    pub osc2_fine_cents: f32,
    pub osc2_level: f32,
    pub cross_fm: f32,
    pub sub_level: f32,
    pub noise_level: f32,
    pub filter_cutoff_hz: f32,
    pub filter_resonance: f32,
    pub filter_key_track: f32,
    pub filter_env_amount: f32,
    pub amp_attack: f32,
    pub amp_decay: f32,
    pub amp_sustain: f32,
    pub amp_release: f32,
    pub filter_attack: f32,
    pub filter_decay: f32,
    pub filter_sustain: f32,
    pub filter_release: f32,
    pub lfo_rate_hz: f32,
    pub lfo_to_pitch_cents: f32,
    pub lfo_to_cutoff_oct: f32,
    pub lfo_to_amp: f32,
    pub glide: f32,
    pub unison_detune_cents: f32,
    pub output: f32,
}

pub const SE_WAVE_SAW: u8 = 0;
pub const SE_WAVE_SQUARE: u8 = 1;
pub const SE_WAVE_TRIANGLE: u8 = 2;
pub const SE_WAVE_SINE: u8 = 3;
pub const SE_FILTER_LOWPASS: u8 = 0;
pub const SE_FILTER_HIGHPASS: u8 = 1;
pub const SE_FILTER_BANDPASS: u8 = 2;

/// v2 track fields plus a pointer to the track's synth parameters (NULL = defaults).
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct se_track_desc_v3 {
    pub track_id: u64,
    pub gain: f32,
    pub pan: f32,
    pub mute: u8,
    pub solo: u8,
    pub instrument: u8,
    pub _pad: [u8; 5],
    pub clips: *const se_clip_desc,
    pub clip_count: u32,
    pub note_count: u32,
    pub notes: *const se_note_desc,
    pub synth: *const se_synth_params,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct se_session_desc_v3 {
    pub tracks: *const se_track_desc_v3,
    pub track_count: u32,
    pub master_gain: f32,
}

// ---- audio input, monitoring, recording, devices (ABI v1 addition)

pub const SE_INPUT_NONE: u8 = 0;
pub const SE_INPUT_MONO: u8 = 1;
pub const SE_INPUT_STEREO: u8 = 2;

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct se_track_desc_v4 {
    pub track_id: u64,
    pub gain: f32,
    pub pan: f32,
    pub mute: u8,
    pub solo: u8,
    pub instrument: u8,
    pub arm: u8,
    pub monitor: u8,
    pub input_mode: u8,
    pub input_channel: u16,
    pub clips: *const se_clip_desc,
    pub clip_count: u32,
    pub note_count: u32,
    pub notes: *const se_note_desc,
    pub synth: *const se_synth_params,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct se_session_desc_v4 {
    pub tracks: *const se_track_desc_v4,
    pub track_count: u32,
    pub master_gain: f32,
}

pub const SE_DEVICE_STRING_MAX: usize = 256;
pub const SE_DEVICE_MAX_RATES: usize = 16;
pub const SE_PATH_MAX: usize = 1024;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct se_device_info {
    pub uid: [c_char; SE_DEVICE_STRING_MAX],
    pub name: [c_char; SE_DEVICE_STRING_MAX],
    pub input_channels: u32,
    pub output_channels: u32,
    pub is_default_input: u8,
    pub is_default_output: u8,
    pub _pad: [u8; 2],
    pub sample_rate_count: u32,
    pub sample_rates: [f64; SE_DEVICE_MAX_RATES],
}

impl Default for se_device_info {
    fn default() -> Self {
        // SAFETY: plain-old-data; all-zero is a valid value.
        unsafe { std::mem::zeroed() }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct se_device_config {
    pub output_uid: *const c_char,
    pub input_uid: *const c_char,
    pub sample_rate: f64,
    pub buffer_size: u32,
    pub input_enabled: u8,
    pub _pad: [u8; 3],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct se_device_status {
    pub output_name: [c_char; SE_DEVICE_STRING_MAX],
    pub input_name: [c_char; SE_DEVICE_STRING_MAX],
    pub input_error: [c_char; SE_DEVICE_STRING_MAX],
    pub running: u8,
    pub input_open: u8,
    pub _pad: [u8; 6],
    pub sample_rate: f64,
    pub buffer_size: u32,
    pub input_channels: u32,
    pub output_channels: u32,
    pub input_latency_frames: u32,
    pub output_latency_frames: u32,
    pub _pad2: u32,
}

impl Default for se_device_status {
    fn default() -> Self {
        // SAFETY: plain-old-data; all-zero is a valid value.
        unsafe { std::mem::zeroed() }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct se_recorded_take {
    pub track_id: u64,
    pub start_frame: i64,
    pub frames: u64,
    pub channels: u32,
    pub _pad: u32,
    pub sample_rate: f64,
    pub path: [c_char; SE_PATH_MAX],
}

impl Default for se_recorded_take {
    fn default() -> Self {
        // SAFETY: plain-old-data; all-zero is a valid value.
        unsafe { std::mem::zeroed() }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct se_live_take {
    pub track_id: u64,
    pub start_frame: i64,
    pub frames: u64,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct se_record_stats {
    pub overruns: u64,
    pub dropped_frames: u64,
    pub write_errors: u64,
    pub takes: u64,
}

unsafe extern "C" {
    pub fn se_abi_version() -> u32;
    pub fn se_engine_create(config: *const se_config) -> *mut se_engine;
    pub fn se_engine_destroy(engine: *mut se_engine);
    pub fn se_last_error() -> *const c_char;

    pub fn se_device_start(engine: *mut se_engine) -> i32;
    pub fn se_device_stop(engine: *mut se_engine) -> i32;
    pub fn se_device_is_running(engine: *const se_engine) -> i32;
    pub fn se_engine_sample_rate(engine: *const se_engine) -> f64;

    pub fn se_asset_load_file(engine: *mut se_engine, path: *const c_char, out: *mut se_asset_info) -> i32;
    pub fn se_asset_release(engine: *mut se_engine, asset_id: u64) -> i32;
    pub fn se_asset_get_info(engine: *const se_engine, asset_id: u64, out: *mut se_asset_info) -> i32;
    pub fn se_asset_compute_peaks(
        engine: *const se_engine,
        asset_id: u64,
        start_frame: u64,
        frames_per_peak: u32,
        out_minmax: *mut f32,
        capacity: usize,
        out_count: *mut usize,
    ) -> i32;

    pub fn se_session_publish(engine: *mut se_engine, session: *const se_session_desc) -> i32;

    pub fn se_transport_play(engine: *mut se_engine) -> i32;
    pub fn se_transport_pause(engine: *mut se_engine) -> i32;
    pub fn se_transport_stop(engine: *mut se_engine) -> i32;
    pub fn se_transport_seek(engine: *mut se_engine, frame: i64) -> i32;
    pub fn se_transport_get(engine: *const se_engine, out: *mut se_transport_info) -> i32;

    pub fn se_meters_read(engine: *mut se_engine, out: *mut se_meter, capacity: usize, out_count: *mut usize) -> i32;
    pub fn se_stats_get(engine: *const se_engine, out: *mut se_stats) -> i32;
    pub fn se_stats_reset(engine: *mut se_engine);

    pub fn se_render_offline(engine: *mut se_engine, out_interleaved: *mut f32, frames: u32) -> i32;
    pub fn se_export_wav(engine: *mut se_engine, path: *const c_char, start_frame: i64, end_frame: i64) -> i32;

    // ---- additions (ABI v1, append-only)
    pub fn se_meters_peek(engine: *mut se_engine, out: *mut se_meter, capacity: usize, out_count: *mut usize) -> i32;
    pub fn se_transport_set_loop(engine: *mut se_engine, enabled: u8, start_frame: i64, end_frame: i64) -> i32;
    pub fn se_metronome_set(engine: *mut se_engine, desc: *const se_metronome_desc) -> i32;
    pub fn se_session_publish_v2(engine: *mut se_engine, session: *const se_session_desc_v2) -> i32;
    pub fn se_session_publish_v3(engine: *mut se_engine, session: *const se_session_desc_v3) -> i32;
    pub fn se_synth_params_default(out: *mut se_synth_params);
    pub fn se_session_publish_v4(engine: *mut se_engine, session: *const se_session_desc_v4) -> i32;
    pub fn se_device_list(out: *mut se_device_info, capacity: usize, out_count: *mut usize) -> i32;
    pub fn se_device_configure(engine: *mut se_engine, config: *const se_device_config) -> i32;
    pub fn se_device_status_get(engine: *mut se_engine, out: *mut se_device_status) -> i32;
    pub fn se_input_set_muted(engine: *mut se_engine, muted: u8) -> i32;
    pub fn se_record_start(engine: *mut se_engine, utf8_dir: *const c_char) -> i32;
    pub fn se_record_stop(engine: *mut se_engine, out_take_count: *mut usize) -> i32;
    pub fn se_record_get_take(engine: *mut se_engine, index: usize, out: *mut se_recorded_take) -> i32;
    pub fn se_record_state(engine: *mut se_engine, out_active: *mut u8, out: *mut se_live_take, capacity: usize, out_count: *mut usize) -> i32;
    pub fn se_record_peaks(engine: *mut se_engine, track_id: u64, frames_per_peak: u32, out_minmax: *mut f32, capacity: usize, out_count: *mut usize) -> i32;
    pub fn se_record_stats_get(engine: *mut se_engine, out: *mut se_record_stats) -> i32;
    pub fn se_render_offline_with_input(engine: *mut se_engine, input_interleaved: *const f32, input_channels: u32, out_interleaved: *mut f32, frames: u32) -> i32;
}

/// Guards the hand-written declarations against drift from saine_engine.h.
/// Expected values were produced by compiling the header with clang (LP64).
#[cfg(all(test, target_pointer_width = "64"))]
mod abi_layout {
    use super::*;
    use std::mem::{align_of, offset_of, size_of};

    #[test]
    fn struct_layouts_match_c_header() {
        assert_eq!((size_of::<se_config>(), align_of::<se_config>()), (24, 8));
        assert_eq!((size_of::<se_asset_info>(), align_of::<se_asset_info>()), (32, 8));
        assert_eq!((size_of::<se_clip_desc>(), align_of::<se_clip_desc>()), (48, 8));
        assert_eq!((size_of::<se_track_desc>(), align_of::<se_track_desc>()), (40, 8));
        assert_eq!((offset_of!(se_track_desc, clips), offset_of!(se_track_desc, clip_count)), (24, 32));
        assert_eq!(size_of::<se_session_desc>(), 16);
        assert_eq!(size_of::<se_transport_info>(), 16);
        assert_eq!(size_of::<se_meter>(), 16);
        assert_eq!(size_of::<se_stats>(), 48);
        // Verified with clang against saine_engine.h: size 24, align 8,
        // offsets enabled 0, gain 4, frames_per_beat 8, beats_per_bar 16, _pad2 20.
        assert_eq!((size_of::<se_metronome_desc>(), align_of::<se_metronome_desc>()), (24, 8));
        assert_eq!(
            (
                offset_of!(se_metronome_desc, enabled),
                offset_of!(se_metronome_desc, gain),
                offset_of!(se_metronome_desc, frames_per_beat),
                offset_of!(se_metronome_desc, beats_per_bar),
                offset_of!(se_metronome_desc, _pad2)
            ),
            (0, 4, 8, 16, 20)
        );
        // MIDI additions, verified with clang (-Wpadded: no implicit padding):
        // se_note_desc 24/8: start 0, end 8, pitch 16, velocity 17, _pad 18.
        assert_eq!((size_of::<se_note_desc>(), align_of::<se_note_desc>()), (24, 8));
        assert_eq!(
            (offset_of!(se_note_desc, start), offset_of!(se_note_desc, end), offset_of!(se_note_desc, pitch), offset_of!(se_note_desc, velocity), offset_of!(se_note_desc, _pad)),
            (0, 8, 16, 17, 18)
        );
        // se_track_desc_v2 48/8: track_id 0, gain 8, pan 12, mute 16, solo 17,
        // instrument 18, _pad 19, clips 24, clip_count 32, note_count 36, notes 40.
        assert_eq!((size_of::<se_track_desc_v2>(), align_of::<se_track_desc_v2>()), (48, 8));
        assert_eq!(
            (
                offset_of!(se_track_desc_v2, track_id),
                offset_of!(se_track_desc_v2, gain),
                offset_of!(se_track_desc_v2, pan),
                offset_of!(se_track_desc_v2, mute),
                offset_of!(se_track_desc_v2, solo),
                offset_of!(se_track_desc_v2, instrument),
                offset_of!(se_track_desc_v2, _pad),
                offset_of!(se_track_desc_v2, clips),
                offset_of!(se_track_desc_v2, clip_count),
                offset_of!(se_track_desc_v2, note_count),
                offset_of!(se_track_desc_v2, notes)
            ),
            (0, 8, 12, 16, 17, 18, 19, 24, 32, 36, 40)
        );
        // se_session_desc_v2 16/8: tracks 0, track_count 8, master_gain 12.
        assert_eq!((size_of::<se_session_desc_v2>(), align_of::<se_session_desc_v2>()), (16, 8));
        assert_eq!(
            (offset_of!(se_session_desc_v2, tracks), offset_of!(se_session_desc_v2, track_count), offset_of!(se_session_desc_v2, master_gain)),
            (0, 8, 12)
        );
    }

    #[test]
    fn synth_params_layout_matches_c_header() {
        // Verified with clang -Wpadded against saine_engine.h (no implicit
        // padding): se_synth_params 128/4, se_track_desc_v3 56/8,
        // se_session_desc_v3 16/8.
        assert_eq!((size_of::<se_synth_params>(), align_of::<se_synth_params>()), (128, 4));
        assert_eq!(
            (
                offset_of!(se_synth_params, osc1_wave),
                offset_of!(se_synth_params, osc1_octave),
                offset_of!(se_synth_params, osc1_semitones),
                offset_of!(se_synth_params, osc2_wave),
                offset_of!(se_synth_params, osc2_octave),
                offset_of!(se_synth_params, osc2_semitones),
                offset_of!(se_synth_params, filter_mode),
                offset_of!(se_synth_params, lfo_wave),
                offset_of!(se_synth_params, polyphony),
                offset_of!(se_synth_params, unison),
                offset_of!(se_synth_params, _pad),
            ),
            (0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10)
        );
        assert_eq!(
            [
                offset_of!(se_synth_params, osc1_shape),
                offset_of!(se_synth_params, osc1_fine_cents),
                offset_of!(se_synth_params, osc1_level),
                offset_of!(se_synth_params, osc2_shape),
                offset_of!(se_synth_params, osc2_fine_cents),
                offset_of!(se_synth_params, osc2_level),
                offset_of!(se_synth_params, cross_fm),
                offset_of!(se_synth_params, sub_level),
                offset_of!(se_synth_params, noise_level),
                offset_of!(se_synth_params, filter_cutoff_hz),
                offset_of!(se_synth_params, filter_resonance),
                offset_of!(se_synth_params, filter_key_track),
                offset_of!(se_synth_params, filter_env_amount),
                offset_of!(se_synth_params, amp_attack),
            ],
            [16, 20, 24, 28, 32, 36, 40, 44, 48, 52, 56, 60, 64, 68]
        );
        assert_eq!(
            (
                offset_of!(se_synth_params, amp_release),
                offset_of!(se_synth_params, filter_attack),
                offset_of!(se_synth_params, filter_release),
                offset_of!(se_synth_params, lfo_rate_hz),
                offset_of!(se_synth_params, lfo_to_pitch_cents),
                offset_of!(se_synth_params, lfo_to_cutoff_oct),
                offset_of!(se_synth_params, lfo_to_amp),
                offset_of!(se_synth_params, glide),
                offset_of!(se_synth_params, unison_detune_cents),
                offset_of!(se_synth_params, output),
            ),
            (80, 84, 96, 100, 104, 108, 112, 116, 120, 124)
        );
        assert_eq!((size_of::<se_track_desc_v3>(), align_of::<se_track_desc_v3>()), (56, 8));
        assert_eq!(
            (
                offset_of!(se_track_desc_v3, track_id),
                offset_of!(se_track_desc_v3, gain),
                offset_of!(se_track_desc_v3, pan),
                offset_of!(se_track_desc_v3, mute),
                offset_of!(se_track_desc_v3, solo),
                offset_of!(se_track_desc_v3, instrument),
                offset_of!(se_track_desc_v3, _pad),
                offset_of!(se_track_desc_v3, clips),
                offset_of!(se_track_desc_v3, clip_count),
                offset_of!(se_track_desc_v3, note_count),
                offset_of!(se_track_desc_v3, notes),
                offset_of!(se_track_desc_v3, synth)
            ),
            (0, 8, 12, 16, 17, 18, 19, 24, 32, 36, 40, 48)
        );
        assert_eq!((size_of::<se_session_desc_v3>(), align_of::<se_session_desc_v3>()), (16, 8));
        assert_eq!(
            (offset_of!(se_session_desc_v3, tracks), offset_of!(se_session_desc_v3, track_count), offset_of!(se_session_desc_v3, master_gain)),
            (0, 8, 12)
        );
    }

    #[test]
    fn recording_layout_matches_c_header() {
        // Verified with clang -Wpadded against saine_engine.h (no implicit
        // padding) and by static_asserts in engine/src/c_api.cpp.
        assert_eq!((size_of::<se_track_desc_v4>(), align_of::<se_track_desc_v4>()), (56, 8));
        assert_eq!(
            (
                offset_of!(se_track_desc_v4, track_id),
                offset_of!(se_track_desc_v4, gain),
                offset_of!(se_track_desc_v4, pan),
                offset_of!(se_track_desc_v4, mute),
                offset_of!(se_track_desc_v4, solo),
                offset_of!(se_track_desc_v4, instrument),
                offset_of!(se_track_desc_v4, arm),
                offset_of!(se_track_desc_v4, monitor),
                offset_of!(se_track_desc_v4, input_mode),
                offset_of!(se_track_desc_v4, input_channel),
                offset_of!(se_track_desc_v4, clips),
                offset_of!(se_track_desc_v4, clip_count),
            ),
            (0, 8, 12, 16, 17, 18, 19, 20, 21, 22, 24, 32)
        );
        assert_eq!((offset_of!(se_track_desc_v4, note_count), offset_of!(se_track_desc_v4, notes), offset_of!(se_track_desc_v4, synth)), (36, 40, 48));
        assert_eq!((size_of::<se_session_desc_v4>(), align_of::<se_session_desc_v4>()), (16, 8));
        assert_eq!((size_of::<se_device_info>(), align_of::<se_device_info>()), (656, 8));
        assert_eq!(
            (
                offset_of!(se_device_info, name),
                offset_of!(se_device_info, input_channels),
                offset_of!(se_device_info, output_channels),
                offset_of!(se_device_info, is_default_input),
                offset_of!(se_device_info, is_default_output),
                offset_of!(se_device_info, sample_rate_count),
                offset_of!(se_device_info, sample_rates)
            ),
            (256, 512, 516, 520, 521, 524, 528)
        );
        assert_eq!((size_of::<se_device_config>(), align_of::<se_device_config>()), (32, 8));
        assert_eq!(
            (offset_of!(se_device_config, input_uid), offset_of!(se_device_config, sample_rate), offset_of!(se_device_config, buffer_size), offset_of!(se_device_config, input_enabled)),
            (8, 16, 24, 28)
        );
        assert_eq!((size_of::<se_device_status>(), align_of::<se_device_status>()), (808, 8));
        assert_eq!(
            (
                offset_of!(se_device_status, input_name),
                offset_of!(se_device_status, input_error),
                offset_of!(se_device_status, running),
                offset_of!(se_device_status, input_open),
                offset_of!(se_device_status, sample_rate),
                offset_of!(se_device_status, buffer_size),
                offset_of!(se_device_status, input_channels),
                offset_of!(se_device_status, output_channels),
                offset_of!(se_device_status, input_latency_frames),
                offset_of!(se_device_status, output_latency_frames),
                offset_of!(se_device_status, _pad2)
            ),
            (256, 512, 768, 769, 776, 784, 788, 792, 796, 800, 804)
        );
        assert_eq!((size_of::<se_recorded_take>(), align_of::<se_recorded_take>()), (1064, 8));
        assert_eq!(
            (offset_of!(se_recorded_take, start_frame), offset_of!(se_recorded_take, frames), offset_of!(se_recorded_take, channels), offset_of!(se_recorded_take, sample_rate), offset_of!(se_recorded_take, path)),
            (8, 16, 24, 32, 40)
        );
        assert_eq!((size_of::<se_live_take>(), align_of::<se_live_take>()), (24, 8));
        assert_eq!((size_of::<se_record_stats>(), align_of::<se_record_stats>()), (32, 8));
    }

    #[test]
    fn abi_version_matches() {
        assert_eq!(unsafe { se_abi_version() }, SE_ABI_VERSION);
    }
}
