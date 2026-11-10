//! Clip menu renders (Consolidate, Bounce, Export) end to end: `render_plan::render_project` →
//! `saine_engine::offline::render` (a private engine on the calling thread, as src/clip_render.rs
//! runs it on its worker). `bench_four_bar_clip_render` (ignored; `cargo test --release -p
//! saine-engine --test clip_render -- --ignored --nocapture`) prints how long a 4-bar clip takes,
//! engine start and file loading included.

use saine_core::render_plan::{self, RenderMode};
use saine_core::*;
use saine_engine::offline::{self, OfflineError, OfflineRender};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Instant;

const RATE: f64 = 48_000.0;

/// 16-bit mono WAV of a sine.
fn write_sine(path: &std::path::Path, secs: f64, hz: f64, amp: f64) {
    let n = (RATE * secs) as usize;
    let mut b = Vec::new();
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + 2 * n as u32).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    for v in [16u32, 0x0001_0001, RATE as u32, RATE as u32 * 2, 0x0010_0002] {
        b.extend_from_slice(&v.to_le_bytes());
    }
    b.extend_from_slice(b"data");
    b.extend_from_slice(&(2 * n as u32).to_le_bytes());
    for i in 0..n {
        let v = amp * (std::f64::consts::TAU * hz * i as f64 / RATE).sin();
        b.extend_from_slice(&((v * 32767.0) as i16).to_le_bytes());
    }
    std::fs::write(path, b).unwrap();
}

struct Rig {
    doc: Document,
    wav: std::path::PathBuf,
    _dir: tempfile::TempDir,
}

/// Default project (audio track 0, instrument track 1) with a sine asset.
fn rig(secs: f64) -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let wav = dir.path().join("s.wav");
    write_sine(&wav, secs, 440.0, 0.3);
    let mut doc = Document::new(Project::default());
    let asset = Asset { id: AssetId(0), relative_path: None, absolute_path: wav.to_string_lossy().into(), sample_rate: RATE, channels: 1, frames: (RATE * secs) as u64 };
    doc.execute(Command::AddAsset { asset }, Origin::User).unwrap();
    Rig { doc, wav, _dir: dir }
}

impl Rig {
    fn job(&self, span: &render_plan::ClipSpan, mode: RenderMode, tail_s: f64) -> OfflineRender {
        let p = self.doc.project();
        let start = p.ticks_to_frames(span.start, RATE);
        let body = (p.ticks_to_frames(span.end, RATE) - start) as usize + (tail_s * RATE) as usize;
        OfflineRender { project: render_plan::render_project(p, span, mode), rate: RATE, transients: HashMap::new(), start, frames: body }
    }

    fn render(&self, span: &render_plan::ClipSpan, mode: RenderMode, tail_s: f64) -> Vec<f32> {
        offline::render(&self.job(span, mode, tail_s), &AtomicBool::new(false), &AtomicUsize::new(0)).unwrap()
    }
}

fn peak(x: &[f32]) -> f32 {
    x.iter().fold(0.0, |m, v| m.max(v.abs()))
}

fn audio_clip(asset: AssetId, id: u64, start_beat: i64, secs: f64) -> Clip {
    Clip { id: ClipId(id), asset, name: format!("c{id}"), start: start_beat * PPQ, source_offset: 0, source_length: (RATE * secs) as u64, gain: 1.0, warp: Default::default(), disabled: false }
}

fn midi_clip(start_beat: i64, beats: i64, pitch: u8) -> MidiClip {
    let mut c = MidiClip::empty(start_beat * PPQ, beats * PPQ);
    c.notes = (0..beats * 4).map(|i| Note { id: i as u32 + 1, pitch: pitch + ((i * 5) % 24) as u8, start: i * PPQ / 4, length: PPQ / 2, velocity: 100 }).collect();
    c
}

#[test]
fn consolidate_render_is_unity_level_and_plays_only_the_group() {
    let mut r = rig(2.0);
    let (a, asset) = (r.doc.project().tracks[0].id, r.doc.project().assets[0].id);
    // Fader and pan must not leak into the material: the track is at -6 dB, panned left.
    r.doc.execute(Command::SetTrackGain { track: a, gain: 0.5 }, Origin::User).unwrap();
    r.doc.execute(Command::SetTrackPan { track: a, pan: -0.7 }, Origin::User).unwrap();
    for (id, beat) in [(101, 2), (102, 12)] {
        r.doc.execute(Command::AddClip { track: a, clip: audio_clip(asset, id, beat, 2.0) }, Origin::User).unwrap();
    }
    let span = render_plan::consolidate_group(r.doc.project(), ClipId(101)).unwrap();
    assert_eq!(span.clips, vec![ClipId(101)], "the clip at beat 12 is apart");
    let out = r.render(&span, RenderMode::Material, 0.0);
    assert_eq!(out.len() / 2, 96_000);
    let (l, rt): (Vec<f32>, Vec<f32>) = (out.iter().step_by(2).copied().collect(), out.iter().skip(1).step_by(2).copied().collect());
    assert!((peak(&l) - 0.3).abs() < 0.01 && (peak(&rt) - 0.3).abs() < 0.01, "source level on both channels: {} {}", peak(&l), peak(&rt));
    let stem = r.render(&span, RenderMode::Stem, 0.0);
    assert!(peak(&stem) < 0.2 && peak(&stem) > 0.01, "a stem keeps fader and pan: {}", peak(&stem));
}

#[test]
fn bounce_of_a_midi_clip_plays_that_clip_only_and_rings_into_its_tail() {
    let mut r = rig(1.0);
    let inst = r.doc.project().tracks[1].id;
    let mk = |start_beat: i64, pitch: u8| {
        let mut c = MidiClip::empty(start_beat * PPQ, PPQ);
        c.notes = vec![Note { id: 1, pitch, start: 0, length: PPQ, velocity: 110 }];
        c
    };
    r.doc.execute(Command::AddMidiClip { track: inst, clip: mk(0, 60) }, Origin::User).unwrap();
    r.doc.execute(Command::AddMidiClip { track: inst, clip: mk(8, 72) }, Origin::User).unwrap();
    let id = r.doc.project().tracks[1].midi_clips[0].id;
    let span = render_plan::clip_span(r.doc.project(), id).unwrap();
    assert!(render_plan::rings_out(r.doc.project(), inst, RenderMode::Bounce));
    let out = r.render(&span, RenderMode::Bounce, 6.0);
    let (body, beat8) = (24_000usize, 8 * 24_000usize);
    assert_eq!(out.len() / 2, body + 6 * 48_000);
    assert!(peak(&out[..body * 2]) > 0.02, "the clip sounds");
    assert!(peak(&out[body * 2..2 * (body + 24_000)]) > 0.0, "the release rings past the clip end");
    assert!(peak(&out[2 * beat8..2 * (beat8 + 24_000)]) < 1e-3, "nothing of the clip at beat 8 leaks in");
    let other = render_plan::clip_span(r.doc.project(), r.doc.project().tracks[1].midi_clips[1].id).unwrap();
    assert!(peak(&r.render(&other, RenderMode::Bounce, 0.0)[..48_000]) > 0.02, "the clip at beat 8 does sound when it is the one rendered");
}

#[test]
fn a_finished_render_reports_every_frame_done() {
    let mut r = rig(2.0);
    let (a, asset) = (r.doc.project().tracks[0].id, r.doc.project().assets[0].id);
    r.doc.execute(Command::AddClip { track: a, clip: audio_clip(asset, 101, 0, 2.0) }, Origin::User).unwrap();
    let span = render_plan::clip_span(r.doc.project(), ClipId(101)).unwrap();
    let job = r.job(&span, RenderMode::Material, 0.0);
    let done = AtomicUsize::new(0);
    let out = offline::render(&job, &AtomicBool::new(false), &done).unwrap();
    assert_eq!((out.len() / 2, done.load(Ordering::Relaxed)), (job.frames, job.frames));
}

#[test]
fn a_cancelled_render_returns_cancelled_and_stops_early() {
    let mut r = rig(2.0);
    let inst = r.doc.project().tracks[1].id;
    let reverb = saine_core::builtin::find("reverb").unwrap().instantiate();
    r.doc.execute(Command::AddDevice { track: inst, index: None, device: reverb }, Origin::User).unwrap();
    r.doc.execute(Command::AddMidiClip { track: inst, clip: midi_clip(0, 16, 48) }, Origin::User).unwrap();
    let id = r.doc.project().tracks[1].midi_clips[0].id;
    let span = render_plan::clip_span(r.doc.project(), id).unwrap();
    // Not even started: the cancel flag is seen before an engine is made.
    let job = r.job(&span, RenderMode::Bounce, 2.0);
    let done = AtomicUsize::new(0);
    assert_eq!(offline::render(&job, &AtomicBool::new(true), &done), Err(OfflineError::Cancelled));
    assert_eq!(done.load(Ordering::Relaxed), 0);
    // Cancelled once a tenth of the audio is rendered.
    let (cancel, done, finished) = (AtomicBool::new(false), AtomicUsize::new(0), AtomicBool::new(false));
    let result = std::thread::scope(|s| {
        s.spawn(|| {
            while done.load(Ordering::Relaxed) < job.frames / 10 && !finished.load(Ordering::Relaxed) {
                std::thread::yield_now();
            }
            cancel.store(true, Ordering::Relaxed);
        });
        let r = offline::render(&job, &cancel, &done);
        finished.store(true, Ordering::Relaxed);
        r
    });
    assert_eq!(result, Err(OfflineError::Cancelled));
    assert!(done.load(Ordering::Relaxed) < job.frames, "stopped before the end: {} of {}", done.load(Ordering::Relaxed), job.frames);
}

#[test]
fn a_missing_audio_file_fails_the_render_and_names_the_file() {
    let mut r = rig(1.0);
    let (a, asset) = (r.doc.project().tracks[0].id, r.doc.project().assets[0].id);
    r.doc.execute(Command::AddClip { track: a, clip: audio_clip(asset, 101, 0, 1.0) }, Origin::User).unwrap();
    let span = render_plan::clip_span(r.doc.project(), ClipId(101)).unwrap();
    let job = r.job(&span, RenderMode::Material, 0.0);
    std::fs::remove_file(&r.wav).unwrap();
    match offline::render(&job, &AtomicBool::new(false), &AtomicUsize::new(0)) {
        Err(OfflineError::Failed(msg)) => assert!(msg.contains("s.wav"), "{msg}"),
        other => panic!("expected a failure, got {:?}", other.map(|v| v.len())),
    }
}

#[test]
fn a_track_with_a_plugin_device_is_refused_before_any_engine_is_made() {
    let mut r = rig(1.0);
    let inst = r.doc.project().tracks[1].id;
    let clap = PluginRef { format: PluginFormat::Clap, id: "test.plugin".into(), path: None, name: "Test".into(), vendor: String::new(), version: String::new(), kind: PluginKind::Effect };
    r.doc.execute(Command::AddDevice { track: inst, index: None, device: Device::new(clap) }, Origin::User).unwrap();
    r.doc.execute(Command::AddMidiClip { track: inst, clip: midi_clip(0, 1, 60) }, Origin::User).unwrap();
    let id = r.doc.project().tracks[1].midi_clips[0].id;
    let span = render_plan::clip_span(r.doc.project(), id).unwrap();
    let job = r.job(&span, RenderMode::Bounce, 0.0);
    assert_eq!(offline::render(&job, &AtomicBool::new(false), &AtomicUsize::new(0)), Err(OfflineError::Failed(offline::PLUGINS_REFUSED.into())));
    let material = r.job(&span, RenderMode::Material, 0.0);
    assert!(material.project.track(inst).unwrap().devices.is_empty(), "Material renders without devices, so it is not refused");
}

#[test]
fn a_plugin_on_another_track_does_not_stop_a_render() {
    let mut r = rig(1.0);
    let (a, inst) = (r.doc.project().tracks[0].id, r.doc.project().tracks[1].id);
    let clap = PluginRef { format: PluginFormat::Clap, id: "test.plugin".into(), path: None, name: "Test".into(), vendor: String::new(), version: String::new(), kind: PluginKind::Effect };
    r.doc.execute(Command::AddDevice { track: a, index: None, device: Device::new(clap) }, Origin::User).unwrap();
    r.doc.execute(Command::AddMidiClip { track: inst, clip: midi_clip(0, 1, 60) }, Origin::User).unwrap();
    let id = r.doc.project().tracks[1].midi_clips[0].id;
    let span = render_plan::clip_span(r.doc.project(), id).unwrap();
    assert!(peak(&r.render(&span, RenderMode::Bounce, 0.0)) > 0.02);
}

#[test]
#[ignore]
fn bench_four_bar_clip_render() {
    let mut r = rig(8.0);
    let (a, inst, asset) = (r.doc.project().tracks[0].id, r.doc.project().tracks[1].id, r.doc.project().assets[0].id);
    let reverb = saine_core::builtin::find("reverb").unwrap().instantiate();
    let delay = saine_core::builtin::find("delay").map(|d| d.instantiate());
    r.doc.execute(Command::AddDevice { track: a, index: None, device: reverb.clone() }, Origin::User).unwrap();
    r.doc.execute(Command::AddDevice { track: inst, index: None, device: reverb }, Origin::User).unwrap();
    if let Some(d) = delay {
        r.doc.execute(Command::AddDevice { track: inst, index: None, device: d }, Origin::User).unwrap();
    }
    r.doc.execute(Command::AddClip { track: a, clip: audio_clip(asset, 101, 0, 8.0) }, Origin::User).unwrap();
    r.doc.execute(Command::AddMidiClip { track: inst, clip: midi_clip(0, 16, 48) }, Origin::User).unwrap();
    let midi = r.doc.project().tracks[1].midi_clips[0].id;
    let cases = [("audio 4 bars, Material (consolidate)", ClipId(101), RenderMode::Material, 0.0), ("audio 4 bars + reverb, Bounce", ClipId(101), RenderMode::Bounce, 2.0), ("MIDI 4 bars, synth + reverb + delay, Bounce", midi, RenderMode::Bounce, 2.0), ("MIDI 4 bars, Stem (export)", midi, RenderMode::Stem, 2.0)];
    for (name, id, mode, tail) in cases {
        let span = render_plan::clip_span(r.doc.project(), id).unwrap();
        let mut ms: Vec<f64> = (0..9)
            .map(|_| {
                let t = Instant::now();
                let out = r.render(&span, mode, tail);
                assert!(peak(&out) > 0.0);
                t.elapsed().as_secs_f64() * 1e3
            })
            .collect();
        ms.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let audio = 8.0 + tail;
        println!("BENCH {name}: median {:.1} ms, min {:.1} ms for {audio:.0} s of audio ({:.0}x real time)", ms[4], ms[0], audio * 1e3 / ms[4]);
    }
}
