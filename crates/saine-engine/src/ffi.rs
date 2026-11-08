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
    }

    #[test]
    fn abi_version_matches() {
        assert_eq!(unsafe { se_abi_version() }, SE_ABI_VERSION);
    }
}
