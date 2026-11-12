//! Automation end to end (docs/adr/0005-automation.md): the engine's lane
//! evaluation against `AutomationLane::value_at` (the contract), and a
//! `Command::SetAutomationLane` volume lane on a DC clip → snapshot →
//! `se_session_publish_v6` → offline render → undo.

use dstudio_core::automation::{AutoPoint, AutomationLane, AutomationTarget, SynthParam};
use dstudio_core::snapshot::LaneSnapshot;
use dstudio_core::*;
use dstudio_engine::{Config, Engine, engine_eval_lane};
use std::collections::HashMap;
use std::path::Path;

/// Deterministic pseudo-random numbers (no extra dependency).
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn unit(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }
}

fn random_lane(rng: &mut Rng, target: AutomationTarget) -> AutomationLane {
    let (lo, hi) = target.range();
    let mut t = (rng.next() % 50) as i64;
    let n = 2 + (rng.next() % 12) as usize;
    let mut points = Vec::new();
    for i in 0..n {
        // Vertical steps (equal times) now and then, including at the first point.
        if i > 0 && rng.next() % 4 != 0 {
            t += 1 + (rng.next() % 5000) as i64;
        }
        let curve = match rng.next() % 4 {
            0 => 0.0,
            1 => 1.0 - 2.0 * rng.unit(),
            2 => {
                if rng.next() % 2 == 0 {
                    1.0
                } else {
                    -1.0
                }
            }
            _ => 1e-7,
        };
        points.push(AutoPoint { time: t, value: lo + (hi - lo) * rng.unit(), curve });
    }
    AutomationLane { target, enabled: true, points }
}

/// Points as frames 1:1 (ticks and frames are both integers; the contract
/// is the same formula over either).
fn as_snapshot(l: &AutomationLane) -> LaneSnapshot {
    LaneSnapshot { target: l.target.clone(), points: l.points.iter().map(|q| (q.time, q.value, q.curve)).collect() }
}

#[test]
fn engine_evaluation_matches_value_at() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let targets = [
        AutomationTarget::Volume,
        AutomationTarget::Pan,
        AutomationTarget::Synth { param: SynthParam::FilterCutoff }, // geometric
        AutomationTarget::Synth { param: SynthParam::LfoRate },      // geometric
        AutomationTarget::Synth { param: SynthParam::Osc1Fine },
        AutomationTarget::Plugin { device: DeviceId(7), param: 3, name: "P".into() },
    ];
    let (mut compared, mut identical, mut worst) = (0usize, 0usize, 0f64);
    for round in 0..200 {
        let lane = random_lane(&mut rng, targets[round % targets.len()].clone());
        lane.validate().unwrap();
        let first = lane.points[0].time;
        let last = lane.points.last().unwrap().time;
        let mut frames: Vec<i64> = (first - 20..=last + 20).step_by(1 + (last - first) as usize / 700).collect();
        for q in &lane.points {
            frames.extend([q.time - 1, q.time, q.time + 1]);
        }
        let got = engine_eval_lane(&as_snapshot(&lane), &frames).unwrap();
        for (&f, &g) in frames.iter().zip(&got) {
            let want = lane.value_at(f).unwrap();
            compared += 1;
            if g.to_bits() == want.to_bits() {
                identical += 1;
            }
            let rel = ((g - want).abs() as f64) / (want.abs() as f64).max(1.0);
            worst = worst.max(rel);
            assert!(rel <= 1e-5, "{:?} at {f}: engine {g} vs core {want}", lane.target);
        }
    }
    eprintln!("{compared} values compared, {identical} bit-identical, worst relative difference {worst:e}");
}

#[test]
fn engine_rejects_lanes_core_rejects() {
    let bad = LaneSnapshot { target: AutomationTarget::Volume, points: vec![(10, 0.0, 0.0), (5, 0.0, 0.0)] };
    assert!(engine_eval_lane(&bad, &[0]).is_err());
    let bad = LaneSnapshot { target: AutomationTarget::Synth { param: SynthParam::FilterCutoff }, points: vec![(0, 5.0, 0.0)] };
    assert!(engine_eval_lane(&bad, &[0]).is_err());
}

/// 16-bit PCM mono WAV with a constant value.
fn write_dc_wav(path: &Path, frames: u32, v: i16) {
    let data_len = frames * 2;
    let mut b = Vec::new();
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data_len).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&48_000u32.to_le_bytes());
    b.extend_from_slice(&96_000u32.to_le_bytes());
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data_len.to_le_bytes());
    for _ in 0..frames {
        b.extend_from_slice(&v.to_le_bytes());
    }
    std::fs::write(path, b).unwrap();
}

fn db_gain(db: f32) -> f64 {
    if db <= -60.0 { 0.0 } else { 10f64.powf(db as f64 / 20.0) }
}

#[test]
fn volume_lane_from_a_document_shapes_the_output_and_undo_restores_it() {
    let dir = tempfile::tempdir().unwrap();
    let wav = dir.path().join("dc.wav");
    write_dc_wav(&wav, 72_000, 16384); // 0.5
    let engine = Engine::new(Config { sample_rate: 48_000.0, max_block_size: 1024 }).unwrap();
    let info = engine.load_file(&wav).unwrap();
    let mut doc = Document::new(Project::default());
    doc.execute(
        Command::AddAsset {
            asset: Asset {
                id: AssetId(0),
                relative_path: None,
                absolute_path: wav.to_string_lossy().into(),
                sample_rate: 48_000.0,
                channels: 1,
                frames: 72_000,
            },
        },
        Origin::User,
    )
    .unwrap();
    let asset = doc.project().assets.last().unwrap().id;
    let map = HashMap::from([(asset, info.asset_id)]);
    let track = doc.project().tracks.iter().find(|t| t.kind == TrackKind::Audio).unwrap().id;
    doc.execute(
        Command::AddClip {
            track,
            clip: Clip {
                id: ClipId(0),
                asset,
                name: "dc".into(),
                start: 0,
                source_offset: 0,
                source_length: 72_000,
                gain: 1.0,
                warp: Default::default(),
                disabled: false,
                clip_loop: None,
            },
        },
        Origin::User,
    )
    .unwrap();
    doc.execute(Command::SetTrackPan { track, pan: -1.0 }, Origin::User).unwrap(); // left = the fader gain

    let render = |doc: &Document, frames: usize| -> Vec<f32> {
        let (snap, skipped) = snapshot::build(doc.project(), engine.sample_rate(), &map);
        assert!(skipped.is_empty());
        engine.publish(&snap).unwrap();
        engine.seek(0).unwrap();
        engine.play().unwrap();
        let mut out = vec![0f32; frames * 2];
        for chunk in out.chunks_mut(2 * 777) {
            engine.render_offline(chunk).unwrap();
        }
        out.iter().step_by(2).copied().collect()
    };
    let static_gain = doc.project().track(track).unwrap().gain;
    let before = render(&doc, 60_000);

    // -60 dB at 0 rising to 0 dB at 1 s (1920 ticks at 120 BPM = 48000 frames), curved, then a step.
    let lane = AutomationLane {
        target: AutomationTarget::Volume,
        enabled: true,
        points: vec![
            AutoPoint { time: 0, value: -60.0, curve: 0.3 },
            AutoPoint { time: 1920, value: 0.0, curve: 0.0 },
            AutoPoint { time: 2000, value: 0.0, curve: 0.0 },
            AutoPoint { time: 2000, value: -12.0, curve: 0.0 },
        ],
    };
    doc.execute(
        Command::SetAutomationLane { track: Some(track), target: AutomationTarget::Volume, lane: Some(lane.clone()) },
        Origin::User,
    )
    .unwrap();
    let out = render(&doc, 60_000);
    // One tick = 25 frames: compare at tick positions against value_at.
    let mut worst = 0f64;
    for k in 0..2400i64 {
        let want = 0.5 * db_gain(lane.value_at(k).unwrap());
        let got = out[(k * 25) as usize] as f64;
        worst = worst.max((got - want).abs());
    }
    eprintln!("document volume lane: worst error {worst:e}");
    assert!(worst <= 1e-4, "envelope must follow value_at: {worst}");
    assert_eq!(out[0], 0.0, "-60 dB is silence");
    assert!((out[50_000] as f64 - 0.5 * db_gain(-12.0)).abs() < 1e-6);

    // A disabled lane is kept in the model but not played.
    let mut disabled = lane.clone();
    disabled.enabled = false;
    doc.execute(Command::SetAutomationLane { track: Some(track), target: AutomationTarget::Volume, lane: Some(disabled) }, Origin::User)
        .unwrap();
    assert_eq!(render(&doc, 60_000), before);
    doc.undo().unwrap();

    // Undo removes the lane: the static fader again, bit for bit.
    doc.undo().unwrap();
    assert!(doc.project().track(track).unwrap().automation.is_empty());
    let after = render(&doc, 60_000);
    assert_eq!(after, before);
    assert!((after[30_000] - 0.5 * static_gain).abs() < 1e-6);
}

#[test]
fn master_volume_lane_reaches_the_engine() {
    let dir = tempfile::tempdir().unwrap();
    let wav = dir.path().join("dc.wav");
    write_dc_wav(&wav, 24_000, 16384);
    let engine = Engine::new(Config { sample_rate: 48_000.0, max_block_size: 512 }).unwrap();
    let info = engine.load_file(&wav).unwrap();
    let mut doc = Document::new(Project::default());
    doc.execute(
        Command::AddAsset {
            asset: Asset {
                id: AssetId(0),
                relative_path: None,
                absolute_path: wav.to_string_lossy().into(),
                sample_rate: 48_000.0,
                channels: 1,
                frames: 24_000,
            },
        },
        Origin::User,
    )
    .unwrap();
    let asset = doc.project().assets.last().unwrap().id;
    let track = doc.project().tracks.iter().find(|t| t.kind == TrackKind::Audio).unwrap().id;
    doc.execute(
        Command::AddClip {
            track,
            clip: Clip {
                id: ClipId(0),
                asset,
                name: "dc".into(),
                start: 0,
                source_offset: 0,
                source_length: 24_000,
                gain: 1.0,
                warp: Default::default(),
                disabled: false,
                clip_loop: None,
            },
        },
        Origin::User,
    )
    .unwrap();
    doc.execute(Command::SetTrackPan { track, pan: -1.0 }, Origin::User).unwrap();
    let lane =
        AutomationLane { target: AutomationTarget::Volume, enabled: true, points: vec![AutoPoint { time: 0, value: -6.0, curve: 0.0 }] };
    doc.execute(Command::SetAutomationLane { track: None, target: AutomationTarget::Volume, lane: Some(lane) }, Origin::User).unwrap();
    let (snap, _) = snapshot::build(doc.project(), 48_000.0, &HashMap::from([(asset, info.asset_id)]));
    engine.publish(&snap).unwrap();
    engine.play().unwrap();
    let mut out = vec![0f32; 2 * 512];
    engine.render_offline(&mut out).unwrap();
    let track_gain = doc.project().track(track).unwrap().gain as f64;
    assert!((out[200] as f64 - 0.5 * track_gain * db_gain(-6.0)).abs() < 1e-6, "{}", out[200]);
}

#[test]
fn plugin_lane_and_parameter_queries_through_the_document() {
    let dir = tempfile::tempdir().unwrap();
    let wav = dir.path().join("dc.wav");
    write_dc_wav(&wav, 24_000, 16384);
    let engine = Engine::new(Config { sample_rate: 48_000.0, max_block_size: 256 }).unwrap();
    let info = engine.load_file(&wav).unwrap();
    let mut doc = Document::new(Project::default());
    doc.execute(
        Command::AddAsset {
            asset: Asset {
                id: AssetId(0),
                relative_path: None,
                absolute_path: wav.to_string_lossy().into(),
                sample_rate: 48_000.0,
                channels: 1,
                frames: 24_000,
            },
        },
        Origin::User,
    )
    .unwrap();
    let asset = doc.project().assets.last().unwrap().id;
    let map = HashMap::from([(asset, info.asset_id)]);
    let track = doc.project().tracks.iter().find(|t| t.kind == TrackKind::Audio).unwrap().id;
    doc.execute(
        Command::AddClip {
            track,
            clip: Clip {
                id: ClipId(0),
                asset,
                name: "dc".into(),
                start: 0,
                source_offset: 0,
                source_length: 24_000,
                gain: 1.0,
                warp: Default::default(),
                disabled: false,
                clip_loop: None,
            },
        },
        Origin::User,
    )
    .unwrap();
    doc.execute(Command::SetTrackPan { track, pan: -1.0 }, Origin::User).unwrap();
    let gain = PluginRef {
        format: PluginFormat::Clap,
        id: "studio.dstudio.test.gain".into(),
        path: Some(dstudio_engine::test_plugin_dir().join("DStudioTestGain.clap").to_string_lossy().into_owned()),
        name: "DStudio Test Gain".into(),
        vendor: "DStudio".into(),
        version: "1.0.0".into(),
        kind: PluginKind::Effect,
    };
    doc.execute(Command::AddDevice { track, index: None, device: Device::new(gain) }, Origin::User).unwrap();
    let dev = doc.project().track(track).unwrap().devices[0].id;
    let publish = |doc: &Document| {
        let (snap, _) = snapshot::build(doc.project(), 48_000.0, &map);
        engine.publish(&snap).unwrap();
    };
    publish(&doc);
    assert!(engine.plugin_load_errors().is_empty());

    let params = engine.plugin_params(dev);
    assert_eq!(params.len(), 1);
    assert_eq!((params[0].id, params[0].name.as_str(), params[0].steps, params[0].automatable), (0, "Gain", 0, true));
    assert!((params[0].default_normalized - 0.5).abs() < 1e-6);
    assert_eq!(engine.plugin_param_value(dev, 0), Some(0.5));
    assert_eq!(engine.plugin_param_text(dev, 0, 0.25).as_deref(), Some("0.250"));
    assert_eq!(engine.plugin_param_value(dev, 99), None);
    assert!(engine.plugin_params(DeviceId(12345)).is_empty());
    assert!(engine.take_plugin_param_changes().is_empty());

    // A plugin lane: gain 0 -> 1 over 960 ticks (24000 frames).
    let target = AutomationTarget::Plugin { device: dev, param: 0, name: "Gain".into() };
    let lane = AutomationLane {
        target: target.clone(),
        enabled: true,
        points: vec![AutoPoint { time: 0, value: 0.0, curve: 0.0 }, AutoPoint { time: 960, value: 1.0, curve: 0.0 }],
    };
    doc.execute(Command::SetAutomationLane { track: Some(track), target, lane: Some(lane.clone()) }, Origin::User).unwrap();
    publish(&doc);
    engine.seek(0).unwrap();
    engine.play().unwrap();
    let mut out = vec![0f32; 2 * 20_000];
    for chunk in out.chunks_mut(2 * 256) {
        engine.render_offline(chunk).unwrap();
    }
    // Events every 64 frames: exact there (one tick = 25 frames; 1600 frames = 64 ticks).
    for k in [0usize, 1600, 3200, 6400, 12800, 19200] {
        let want = 0.5 * lane.value_at((k / 25) as i64).unwrap() as f64;
        assert!((out[2 * k] as f64 - want).abs() < 1e-6, "frame {k}: {} vs {want}", out[2 * k]);
    }
    // Automation is not an editor change.
    assert!(engine.take_plugin_param_changes().is_empty());
}
