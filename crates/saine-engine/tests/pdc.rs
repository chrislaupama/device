//! End-to-end plugin delay compensation from the project model: two audio
//! tracks play the same impulse clip; `AddDevice` puts SaineTestLatency
//! (CLAP, N = 1000 frames from its state) on one of them → snapshot →
//! publish → offline render. With compensation the impulses line up (one
//! impulse of double amplitude, L = 1000 frames late); switched off they
//! are N frames apart, as before compensation existed.

use saine_core::*;
use saine_engine::{Config, Engine, LatencyReport};
use std::collections::HashMap;
use std::path::Path;

const N: u32 = 1000;
const IMPULSE_AT: usize = 4800;
const PAN_C: f32 = std::f32::consts::FRAC_1_SQRT_2;

/// Stereo 16-bit WAV, silent except one sample of 0.5 at `at`.
fn write_impulse_wav(path: &Path, frames: u32, at: u32) {
    let data_len = frames * 4;
    let mut b = Vec::new();
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data_len).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&48_000u32.to_le_bytes());
    b.extend_from_slice(&(48_000u32 * 4).to_le_bytes());
    b.extend_from_slice(&4u16.to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data_len.to_le_bytes());
    for i in 0..frames {
        let v: i16 = if i == at { 16384 } else { 0 };
        b.extend_from_slice(&v.to_le_bytes());
        b.extend_from_slice(&v.to_le_bytes());
    }
    std::fs::write(path, b).unwrap();
}

fn latency_plugin() -> PluginRef {
    PluginRef {
        format: PluginFormat::Clap,
        id: "studio.saine.test.latency".into(),
        path: Some(saine_engine::test_plugin_dir().join("SaineTestLatency.clap").to_string_lossy().into_owned()),
        name: "Saine Test Latency".into(),
        vendor: "Saine".into(),
        version: "1.0.0".into(),
        kind: PluginKind::Effect,
    }
}

fn nonzero(v: &[f32]) -> Vec<usize> {
    v.iter().enumerate().filter(|(_, x)| **x != 0.0).map(|(i, _)| i).collect()
}

#[test]
fn latency_plugin_on_one_of_two_tracks_is_compensated_and_toggle_off_restores_the_offset() {
    let dir = tempfile::tempdir().unwrap();
    let wav = dir.path().join("impulse.wav");
    write_impulse_wav(&wav, 9600, IMPULSE_AT as u32);

    let engine = Engine::new(Config { sample_rate: 48_000.0, max_block_size: 512 }).unwrap();
    assert!(engine.delay_compensation_enabled(), "on by default");
    let mut doc = Document::new(Project::default());
    let info = engine.load_file(&wav).unwrap();
    doc.execute(
        Command::AddAsset { asset: Asset { id: AssetId(0), relative_path: None, absolute_path: wav.to_string_lossy().into(), sample_rate: 48_000.0, channels: 2, frames: info.frames } },
        Origin::User,
    )
    .unwrap();
    let asset = doc.project().assets.last().unwrap().id;
    let mut map = HashMap::new();
    map.insert(asset, info.asset_id);

    // A second audio track; both get the same clip.
    doc.execute(Command::AddTrack { index: None, track: Track::new(TrackId(0), TrackKind::Audio, "Audio 2", ColorIndex(2)) }, Origin::User).unwrap();
    let audio: Vec<TrackId> = doc.project().tracks.iter().filter(|t| t.kind == TrackKind::Audio).map(|t| t.id).collect();
    assert_eq!(audio.len(), 2);
    for &track in &audio {
        doc.execute(Command::AddClip { track, clip: Clip { id: ClipId(0), asset, name: "impulse".into(), start: 0, source_offset: 0, source_length: 9600, gain: 1.0, warp: Default::default() } }, Origin::User).unwrap();
    }
    let mut dev = Device::new(latency_plugin());
    dev.state = Some(device::b64::encode(&N.to_le_bytes()));
    doc.execute(Command::AddDevice { track: audio[0], index: None, device: dev }, Origin::User).unwrap();
    let dev_id = doc.project().tracks.iter().find(|t| t.id == audio[0]).unwrap().devices[0].id;

    let (snap, skipped) = snapshot::build(doc.project(), engine.sample_rate(), &map);
    assert!(skipped.is_empty());
    engine.publish(&snap).unwrap();
    assert!(engine.plugin_load_errors().is_empty(), "{:?}", engine.plugin_load_errors());
    assert_eq!(engine.plugin_latency(dev_id), N);

    let render_left = |frames: usize| -> Vec<f32> {
        engine.seek(0).unwrap();
        engine.play().unwrap();
        let mut out = vec![0f32; frames * 2];
        for chunk in out.chunks_mut(2 * 300) {
            engine.render_offline(chunk).unwrap();
        }
        out.iter().step_by(2).copied().collect()
    };

    // Compensated: one impulse of double amplitude at t + L.
    let report = engine.latency_report();
    let expected_tracks: Vec<(u64, u32)> =
        snap.tracks.iter().map(|t| (t.track_id, if t.track_id == audio[0].0 { N } else { 0 })).collect();
    assert_eq!(report, LatencyReport { total_frames: N, tracks: expected_tracks.clone(), clamped: false });
    let on = render_left(9600);
    assert_eq!(nonzero(&on), vec![IMPULSE_AT + N as usize]);
    assert!((on[IMPULSE_AT + N as usize] - 2.0 * 0.5 * PAN_C).abs() < 1e-6, "{}", on[IMPULSE_AT + N as usize]);
    assert_eq!(engine.transport().position, 9600);
    assert_eq!(engine.audible_position(), 9600 - N as i64);

    // Off: misaligned by N, exactly like the engine without compensation.
    engine.set_delay_compensation(false);
    assert!(!engine.delay_compensation_enabled());
    assert_eq!(engine.latency_report(), LatencyReport { total_frames: 0, tracks: expected_tracks, clamped: false });
    let off = render_left(9600);
    assert_eq!(nonzero(&off), vec![IMPULSE_AT, IMPULSE_AT + N as usize]);
    assert!((off[IMPULSE_AT] - 0.5 * PAN_C).abs() < 1e-6);
    assert_eq!(engine.audible_position(), 9600);

    // Bypassing the device keeps the compensated timing (the slot delays by N).
    engine.set_delay_compensation(true);
    doc.execute(Command::SetDeviceEnabled { device: dev_id, enabled: false }, Origin::User).unwrap();
    let (snap, _) = snapshot::build(doc.project(), engine.sample_rate(), &map);
    engine.publish(&snap).unwrap();
    assert_eq!(engine.latency_report().total_frames, N);
    assert_eq!(nonzero(&render_left(9600)), vec![IMPULSE_AT + N as usize]);
}
