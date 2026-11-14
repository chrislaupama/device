//! End-to-end milestone-1 test: project model → commands → snapshot → C++
//! engine offline render, without an audio device.

use dstudio_core::*;
use dstudio_engine::{Config, Engine};
use std::collections::HashMap;
use std::path::Path;

/// Write a 16-bit PCM stereo WAV with a constant value per channel.
fn write_dc_wav(path: &Path, rate: u32, frames: u32, l: i16, r: i16) {
    let data_len = frames * 4;
    let mut b = Vec::with_capacity(44 + data_len as usize);
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data_len).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes()); // PCM
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&rate.to_le_bytes());
    b.extend_from_slice(&(rate * 4).to_le_bytes());
    b.extend_from_slice(&4u16.to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data_len.to_le_bytes());
    for _ in 0..frames {
        b.extend_from_slice(&l.to_le_bytes());
        b.extend_from_slice(&r.to_le_bytes());
    }
    std::fs::write(path, b).unwrap();
}

struct Slice {
    engine: Engine,
    doc: Document,
    map: HashMap<AssetId, u64>,
}

impl Slice {
    fn import(&mut self, path: &Path) -> AssetId {
        let info = self.engine.load_file(path).unwrap();
        self.doc
            .execute(
                Command::AddAsset {
                    asset: Asset {
                        id: AssetId(0),
                        relative_path: None,
                        absolute_path: path.to_string_lossy().into(),
                        sample_rate: info.source_sample_rate,
                        channels: info.channels,
                        frames: (info.frames as f64 * info.source_sample_rate / self.engine.sample_rate()).round() as u64,
                    },
                },
                Origin::User,
            )
            .unwrap();
        let id = self.doc.project().assets.last().unwrap().id;
        self.map.insert(id, info.asset_id);
        id
    }
    fn publish(&self) {
        let (snap, skipped) = snapshot::build(self.doc.project(), self.engine.sample_rate(), &self.map);
        assert!(skipped.is_empty());
        self.engine.publish(&snap).unwrap();
    }
    fn render(&self, frames: usize) -> Vec<f32> {
        let mut out = vec![0f32; frames * 2];
        // Render in odd-sized chunks to exercise block boundaries.
        let mut done = 0;
        for chunk in [127usize, 512, 1000, 33].iter().cycle() {
            if done >= frames {
                break;
            }
            let n = (*chunk).min(frames - done);
            self.engine.render_offline(&mut out[done * 2..(done + n) * 2]).unwrap();
            done += n;
        }
        out
    }
}

fn setup() -> (Slice, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::new(Config { sample_rate: 48_000.0, max_block_size: 1024 }).unwrap();
    assert_eq!(engine.sample_rate(), 48_000.0);
    (Slice { engine, doc: Document::new(Project::default()), map: HashMap::new() }, dir)
}

const HALF: i16 = 16384; // 0.5 full scale

#[test]
fn two_tracks_sum_with_sample_accurate_placement() {
    let (mut s, dir) = setup();
    let a = dir.path().join("a.wav");
    let b = dir.path().join("b.wav");
    write_dc_wav(&a, 48_000, 4800, HALF, HALF);
    write_dc_wav(&b, 48_000, 4800, HALF / 2, HALF / 2);
    let aa = s.import(&a);
    let ab = s.import(&b);
    let t1 = s.doc.project().tracks[0].id;
    s.doc
        .execute(Command::AddTrack { index: None, track: Track::new(TrackId(0), TrackKind::Audio, "Audio 2", ColorIndex(2)) }, Origin::User)
        .unwrap();
    let t2 = s.doc.project().tracks[2].id;
    // Clip A at bar 1 beat 1; clip B one beat later (0.5 s = 24000 frames at 120 BPM).
    s.doc
        .execute(
            Command::AddClip {
                track: t1,
                clip: Clip {
                    id: ClipId(0),
                    asset: aa,
                    name: "a".into(),
                    start: 0,
                    source_offset: 0,
                    source_length: 4800,
                    gain: 1.0,
                    warp: Default::default(),
                    disabled: false,
                    clip_loop: None,
                    fades: Default::default(),
                },
            },
            Origin::User,
        )
        .unwrap();
    s.doc
        .execute(
            Command::AddClip {
                track: t2,
                clip: Clip {
                    id: ClipId(0),
                    asset: ab,
                    name: "b".into(),
                    start: PPQ / 8,
                    source_offset: 0,
                    source_length: 4800,
                    gain: 1.0,
                    warp: Default::default(),
                    disabled: false,
                    clip_loop: None,
                    fades: Default::default(),
                },
            },
            Origin::User,
        )
        .unwrap();
    s.publish();
    s.engine.play().unwrap();
    let out = s.render(12_000);
    // PPQ/8 ticks = 1/32 note = 62.5 ms = 3000 frames.
    let pan_c = std::f32::consts::FRAC_1_SQRT_2; // constant-power centre
    let l = |i: usize| out[i * 2];
    assert!((l(0) - 0.5 * pan_c).abs() < 1e-3, "track A alone at frame 0: {}", l(0));
    assert!((l(2999) - 0.5 * pan_c).abs() < 1e-3);
    assert!((l(3000) - 0.75 * pan_c).abs() < 1e-3, "both tracks summed from frame 3000: {}", l(3000));
    assert!((l(4799) - 0.75 * pan_c).abs() < 1e-3);
    assert!((l(4800) - 0.25 * pan_c).abs() < 1e-3, "A ended at 4800");
    assert!(l(7800).abs() < 1e-6, "B ended at 7800");
    assert_eq!(s.engine.transport().position, 12_000);
}

#[test]
fn mute_solo_volume_and_pan_reach_the_engine() {
    let (mut s, dir) = setup();
    let a = dir.path().join("a.wav");
    write_dc_wav(&a, 48_000, 48_000, HALF, HALF);
    let aa = s.import(&a);
    let t1 = s.doc.project().tracks[0].id;
    s.doc
        .execute(
            Command::AddClip {
                track: t1,
                clip: Clip {
                    id: ClipId(0),
                    asset: aa,
                    name: "a".into(),
                    start: 0,
                    source_offset: 0,
                    source_length: 48_000,
                    gain: 1.0,
                    warp: Default::default(),
                    disabled: false,
                    clip_loop: None,
                    fades: Default::default(),
                },
            },
            Origin::User,
        )
        .unwrap();
    s.engine.play().unwrap();

    s.doc.execute(Command::SetTrackPan { track: t1, pan: 1.0 }, Origin::User).unwrap();
    s.publish();
    let out = s.render(256);
    assert!(out[200].abs() < 1e-4 && (out[201] - 0.5).abs() < 1e-3, "hard right: L={} R={}", out[200], out[201]);

    s.doc.execute(Command::SetTrackGain { track: t1, gain: db_to_gain(-6.0206) }, Origin::User).unwrap();
    s.publish();
    let out = s.render(256);
    assert!((out[201] - 0.25).abs() < 1e-3, "-6 dB: {}", out[201]);

    s.doc.execute(Command::SetTrackMute { track: t1, mute: true }, Origin::User).unwrap();
    s.publish();
    assert!(s.render(256).iter().all(|v| v.abs() < 1e-6), "muted");

    s.doc.execute(Command::SetTrackMute { track: t1, mute: false }, Origin::User).unwrap();
    let t2 = s.doc.project().tracks[1].id;
    s.doc.execute(Command::SetTrackSolo { track: t2, solo: true }, Origin::User).unwrap();
    s.publish();
    assert!(s.render(256).iter().all(|v| v.abs() < 1e-6), "another track soloed → silent");

    // Undo the solo through the history; the engine follows the model.
    s.doc.undo().unwrap();
    s.publish();
    assert!(s.render(256)[201] > 0.2);
}

#[test]
fn transport_seek_stop_and_trim() {
    let (mut s, dir) = setup();
    let a = dir.path().join("ramp.wav");
    write_dc_wav(&a, 48_000, 48_000, HALF, HALF);
    let aa = s.import(&a);
    let t1 = s.doc.project().tracks[0].id;
    s.doc
        .execute(
            Command::AddClip {
                track: t1,
                clip: Clip {
                    id: ClipId(0),
                    asset: aa,
                    name: "a".into(),
                    start: 0,
                    source_offset: 0,
                    source_length: 48_000,
                    gain: 1.0,
                    warp: Default::default(),
                    disabled: false,
                    clip_loop: None,
                    fades: Default::default(),
                },
            },
            Origin::User,
        )
        .unwrap();
    let c = s.doc.project().tracks[0].clips[0].id;
    // Trim to 1000 frames starting at source frame 100.
    s.doc.execute(Command::TrimClip { clip: c, start: 0, source_offset: 100, source_length: 1000 }, Origin::User).unwrap();
    s.publish();
    s.engine.seek(990).unwrap();
    s.engine.play().unwrap();
    let out = s.render(20);
    assert!(out[0].abs() > 0.1 && out[9 * 2].abs() > 0.1, "inside clip");
    assert!(out[10 * 2].abs() < 1e-6, "trim end at timeline frame 1000");
    s.engine.stop().unwrap();
    // Command application is block-synchronous; render once to apply.
    s.render(1);
    let t = s.engine.transport();
    assert!(!t.playing);
    assert_eq!(t.position, 0);
}

#[test]
fn waveform_peaks_and_export_round_trip() {
    let (mut s, dir) = setup();
    let a = dir.path().join("a.wav");
    write_dc_wav(&a, 44_100, 44_100, HALF, -HALF);
    let aa = s.import(&a);
    let eid = s.map[&aa];
    let info = s.engine.asset_info(eid).unwrap();
    assert_eq!(info.frames, 48_000, "resampled 44.1k → 48k keeps duration");
    let peaks = s.engine.peaks(eid, 0, 4800, 100).unwrap();
    assert_eq!(peaks.len(), 10);
    // L=+0.5, R=-0.5 mixed to mono → ~0 (min/max straddle 0 only at edges).
    assert!(peaks[5][0] <= peaks[5][1]);

    let t1 = s.doc.project().tracks[0].id;
    let len = s.doc.project().asset(aa).unwrap().frames;
    s.doc
        .execute(
            Command::AddClip {
                track: t1,
                clip: Clip {
                    id: ClipId(0),
                    asset: aa,
                    name: "a".into(),
                    start: 0,
                    source_offset: 0,
                    source_length: len,
                    gain: 1.0,
                    warp: Default::default(),
                    disabled: false,
                    clip_loop: None,
                    fades: Default::default(),
                },
            },
            Origin::User,
        )
        .unwrap();
    s.publish();
    let out_path = dir.path().join("export.wav");
    s.engine.export_wav(&out_path, 0, 4800).unwrap();
    let bytes = std::fs::read(&out_path).unwrap();
    assert_eq!(&bytes[0..4], b"RIFF");
    assert!(bytes.len() >= 44 + 4800 * 2 * 4, "32-bit float stereo, 4800 frames");
}
