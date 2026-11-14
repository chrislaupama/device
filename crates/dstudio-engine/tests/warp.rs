//! Clip warping end to end: `Command::SetClipWarp` → snapshot → `se_session_publish_v7`
//! → offline render, plus onset detection through `Engine::detect_onsets`.

use dstudio_core::warp::{ClipWarp, Transient, WarpMode};
use dstudio_core::*;
use dstudio_engine::{Config, Engine};
use std::collections::HashMap;
use std::path::Path;

const RATE: f64 = 48_000.0;

/// 16-bit PCM stereo WAV from a sample function.
fn write_wav(path: &Path, frames: u32, f: impl Fn(u32) -> f64) {
    let data_len = frames * 4;
    let mut b = Vec::with_capacity(44 + data_len as usize);
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data_len).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    for v in [
        16u32.to_le_bytes().as_slice(),
        &1u16.to_le_bytes(),
        &2u16.to_le_bytes(),
        &48_000u32.to_le_bytes(),
        &(48_000u32 * 4).to_le_bytes(),
        &4u16.to_le_bytes(),
        &16u16.to_le_bytes(),
    ] {
        b.extend_from_slice(v);
    }
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data_len.to_le_bytes());
    for i in 0..frames {
        let s = (f(i).clamp(-1.0, 1.0) * 32767.0) as i16;
        b.extend_from_slice(&s.to_le_bytes());
        b.extend_from_slice(&s.to_le_bytes());
    }
    std::fs::write(path, b).unwrap();
}

struct Rig {
    engine: Engine,
    doc: Document,
    map: HashMap<AssetId, u64>,
    clip: ClipId,
    asset: AssetId,
}

impl Rig {
    fn new(path: &Path) -> Rig {
        let engine = Engine::new(Config { sample_rate: RATE, max_block_size: 512 }).unwrap();
        let mut doc = Document::new(Project::default());
        let info = engine.load_file(path).unwrap();
        let frames = info.frames;
        doc.execute(
            Command::AddAsset {
                asset: Asset {
                    id: AssetId(0),
                    relative_path: None,
                    absolute_path: path.to_string_lossy().into(),
                    sample_rate: RATE,
                    channels: 2,
                    frames,
                },
            },
            Origin::User,
        )
        .unwrap();
        let asset = doc.project().assets[0].id;
        let track = doc.project().tracks[0].id;
        let clip = Clip {
            id: ClipId(0),
            asset,
            name: "tone".into(),
            start: 0,
            source_offset: 0,
            source_length: frames,
            gain: 1.0,
            warp: ClipWarp::default(),
            disabled: false,
            clip_loop: None,
            fades: Default::default(),
        };
        doc.execute(Command::AddClip { track, clip }, Origin::User).unwrap();
        let clip = doc.project().tracks[0].clips[0].id;
        Rig { engine, doc, map: HashMap::from([(asset, info.asset_id)]), clip, asset }
    }

    fn warp(&mut self, w: ClipWarp) {
        self.doc.execute(Command::SetClipWarp { clip: self.clip, warp: w, label: "Set clip speed".into() }, Origin::User).unwrap();
    }

    fn render(&self, frames: usize, transients: &HashMap<AssetId, Vec<Transient>>) -> Vec<f32> {
        let (snap, skipped) = snapshot::build_with_transients(self.doc.project(), RATE, &self.map, transients);
        assert!(skipped.is_empty());
        self.engine.publish(&snap).unwrap();
        self.engine.seek(0).unwrap();
        self.engine.play().unwrap();
        let mut out = vec![0f32; frames * 2];
        for chunk in out.chunks_mut(512 * 2) {
            self.engine.render_offline(chunk).unwrap();
        }
        self.engine.stop().unwrap();
        out.chunks_exact(2).map(|c| c[0]).collect()
    }
}

fn rms(x: &[f32]) -> f64 {
    (x.iter().map(|&v| (v as f64).powi(2)).sum::<f64>() / x.len() as f64).sqrt()
}

/// Frequency from rising zero crossings.
fn frequency(x: &[f32]) -> f64 {
    let ups: Vec<usize> = (1..x.len()).filter(|&i| x[i - 1] < 0.0 && x[i] >= 0.0).collect();
    (ups.len() - 1) as f64 * RATE / (ups[ups.len() - 1] - ups[0]) as f64
}

fn tone_rig() -> (Rig, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("tone.wav");
    write_wav(&path, 96_000, |i| 0.5 * (2.0 * std::f64::consts::PI * 440.0 * i as f64 / RATE).sin());
    (Rig::new(&path), dir)
}

#[test]
fn double_speed_halves_the_clip_and_the_mode_decides_the_pitch() {
    let (mut rig, _dir) = tone_rig();
    let none = HashMap::new();
    let plain = rig.render(120_000, &none);
    assert!(rms(&plain[80_000..90_000]) > 0.2, "unwarped: 2 s of tone");
    assert!((frequency(&plain[10_000..80_000]) - 440.0).abs() < 1.0);

    let on = ClipWarp { enabled: true, original_bpm: 120.0, speed: 2.0, ..ClipWarp::default() };
    for (mode, pitch) in [(WarpMode::Repitch, 880.0), (WarpMode::Melody, 440.0), (WarpMode::Vocal, 440.0)] {
        rig.warp(ClipWarp { mode, ..on.clone() });
        assert_eq!(rig.doc.project().clip_length_ticks(rig.doc.project().clip(rig.clip).unwrap()), 2 * PPQ);
        let out = rig.render(72_000, &none);
        assert!(rms(&out[8_000..40_000]) > 0.2, "{mode:?}: the first second plays");
        assert!(rms(&out[52_000..72_000]) < 1e-3, "{mode:?}: and then it ends");
        let f = frequency(&out[8_000..40_000]);
        assert!((f - pitch).abs() < pitch * 0.01, "{mode:?}: {f} Hz, want {pitch}");
    }

    // Undo back to the unwarped clip.
    while rig.doc.project().clip(rig.clip).unwrap().warp.enabled {
        rig.doc.undo().unwrap();
    }
    let back = rig.render(120_000, &none);
    assert!(rms(&back[80_000..90_000]) > 0.2);
}

#[test]
fn background_renders_attach_and_play_the_same_stretch() {
    let (mut rig, _dir) = tone_rig();
    rig.warp(ClipWarp { enabled: true, original_bpm: 120.0, speed: 0.8, mode: WarpMode::Melody, ..ClipWarp::default() });
    let none = HashMap::new();
    let live = rig.render(96_000, &none);
    for _ in 0..500 {
        if rig.engine.warp_renders_pending() == 0 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(rig.engine.warp_renders_pending(), 0);
    rig.engine.plugin_idle();
    let rendered = rig.render(96_000, &none);
    assert!((frequency(&rendered[10_000..90_000]) - 440.0).abs() < 2.0);
    assert!((rms(&rendered[10_000..90_000]) - rms(&live[10_000..90_000])).abs() < 0.02);
}

#[test]
fn onsets_drive_drum_slices() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("hits.wav");
    let hits = [4_800u32, 28_800, 52_800, 76_800];
    write_wav(&path, 96_000, |i| {
        hits.iter()
            .filter(|&&h| i >= h)
            .map(|&h| (-((i - h) as f64) / 800.0).exp() * (2.0 * std::f64::consts::PI * 180.0 * (i - h) as f64 / RATE).sin() * 0.8)
            .sum()
    });
    let mut rig = Rig::new(&path);
    let onsets = rig.engine.detect_onsets(rig.map[&rig.asset]).unwrap();
    assert_eq!(onsets.len(), hits.len(), "{onsets:?}");
    for (o, &h) in onsets.iter().zip(&hits) {
        assert!((o.frame - h as i64).abs() <= 144 && o.percussive, "{o:?} vs {h}");
    }
    let transients = HashMap::from([(
        rig.asset,
        onsets.iter().map(|o| Transient { source: o.frame as u64, strength: o.strength, percussive: o.percussive }).collect(),
    )]);
    // A 120 BPM loop played at 0.5x: the hits move to twice their times, still sharp.
    rig.warp(ClipWarp { enabled: true, original_bpm: 120.0, speed: 0.5, mode: WarpMode::Drums, ..ClipWarp::default() });
    let out = rig.render(192_000, &transients);
    for &h in &hits {
        let at = 2 * h as usize;
        assert!(rms(&out[at.saturating_sub(2_000)..at - 300]) < 0.02, "quiet before the hit at {at}");
        assert!(rms(&out[at..at + 800]) > 0.1, "hit at {at}");
    }
}

#[test]
fn reversed_clip_plays_back_to_front() {
    // 1 s of 440 Hz then 1 s of silence; reversed, the silence comes first.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("half.wav");
    write_wav(&path, 96_000, |i| if i < 48_000 { 0.5 * (2.0 * std::f64::consts::PI * 440.0 * i as f64 / RATE).sin() } else { 0.0 });
    let mut rig = Rig::new(&path);
    rig.warp(ClipWarp { reverse: true, ..ClipWarp::default() });
    let ea = rig.map[&rig.asset];
    let rev = rig.engine.reverse_asset(ea).unwrap();
    let (snap, skipped) =
        snapshot::build_full(rig.doc.project(), RATE, &rig.map, &HashMap::from([(rig.asset, rev.asset_id)]), &HashMap::new());
    assert!(skipped.is_empty());
    rig.engine.publish(&snap).unwrap();
    rig.engine.seek(0).unwrap();
    rig.engine.play().unwrap();
    let mut out = vec![0f32; 96_000 * 2];
    for chunk in out.chunks_mut(512 * 2) {
        rig.engine.render_offline(chunk).unwrap();
    }
    rig.engine.stop().unwrap();
    let l: Vec<f32> = out.chunks_exact(2).map(|c| c[0]).collect();
    assert!(rms(&l[2_000..46_000]) < 1e-4, "silence first");
    assert!(rms(&l[50_000..94_000]) > 0.2, "then the tone");
    assert!((frequency(&l[50_000..94_000]) - 440.0).abs() < 1.0);
}

#[test]
fn transpose_shifts_the_pitch_and_varispeeds_unwarped_clips() {
    let (mut rig, _dir) = tone_rig();
    let none = HashMap::new();
    // Warp off, +12: an octave up and twice as fast (1 s of the 2 s tone).
    rig.warp(ClipWarp { transpose: 12, ..ClipWarp::default() });
    assert_eq!(rig.doc.project().clip_length_ticks(rig.doc.project().clip(rig.clip).unwrap()), 2 * PPQ);
    let out = rig.render(72_000, &none);
    assert!(rms(&out[8_000..40_000]) > 0.2);
    assert!(rms(&out[52_000..72_000]) < 1e-3, "and then it ends");
    let f = frequency(&out[8_000..40_000]);
    assert!((f - 880.0).abs() < 8.8, "{f} Hz, want 880");
    // Warp on (Melody), -12: an octave down, same length.
    rig.warp(ClipWarp { enabled: true, original_bpm: 120.0, mode: WarpMode::Melody, transpose: -12, ..ClipWarp::default() });
    assert_eq!(rig.doc.project().clip_length_ticks(rig.doc.project().clip(rig.clip).unwrap()), 4 * PPQ);
    let out = rig.render(120_000, &none);
    assert!(rms(&out[80_000..90_000]) > 0.2, "still 2 s long");
    let f = frequency(&out[10_000..80_000]);
    assert!((f - 220.0).abs() < 2.2, "{f} Hz, want 220");
}

#[test]
fn tempo_is_detected_from_the_engine_onsets() {
    // Kick-like hits on every beat and softer ones on the off-beats at 128 BPM.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("beat.wav");
    let beat = RATE * 60.0 / 128.0;
    write_wav(&path, (RATE * 12.0) as u32, |i| {
        let half = beat / 2.0;
        let k = (i as f64 / half).floor();
        let t = (i as f64 - k * half) / RATE;
        let amp = if k as i64 % 2 == 0 { 0.8 } else { 0.35 };
        amp * (-t * 40.0).exp() * (2.0 * std::f64::consts::PI * 70.0 * t).sin()
    });
    let rig = Rig::new(&path);
    let onsets = rig.engine.detect_onsets(rig.map[&rig.asset]).unwrap();
    let tr: Vec<Transient> =
        onsets.iter().map(|o| Transient { source: o.frame as u64, strength: o.strength, percussive: o.percussive }).collect();
    assert_eq!(dstudio_core::warp::estimate_bpm(&tr, RATE), Some(128.0), "{} onsets", tr.len());
}
