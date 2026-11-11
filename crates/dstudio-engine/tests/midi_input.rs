//! End-to-end MIDI input (specs/003-midi-input-and-recording): an armed
//! instrument track (document command → snapshot → publish) plays injected
//! MIDI, and the capture ring reports where the notes were heard.

use dstudio_core::*;
use dstudio_engine::{Config, Engine};
use std::collections::HashMap;

const RATE: f64 = 48_000.0;

fn publish(engine: &Engine, doc: &Document) {
    let (snap, skipped) = snapshot::build(doc.project(), engine.sample_rate(), &HashMap::new());
    assert!(skipped.is_empty());
    engine.publish(&snap).unwrap();
}

fn peak(e: &Engine, frames: usize) -> f32 {
    let mut out = vec![0f32; frames * 2];
    for chunk in out.chunks_mut(512 * 2) {
        e.render_offline(chunk).unwrap();
    }
    out.iter().fold(0f32, |m, v| m.max(v.abs()))
}

#[test]
fn armed_instrument_track_plays_and_captures_midi_input() {
    let engine = Engine::new(Config { sample_rate: RATE, max_block_size: 512 }).unwrap();
    let mut doc = Document::new(Project::default());
    let inst = doc.project().tracks.iter().find(|t| t.kind == TrackKind::Instrument).unwrap().id;
    publish(&engine, &doc);
    assert!(engine.midi_inputs().is_ok(), "listing works without MIDI hardware");

    // Not armed, monitoring or focused: the keyboard is not heard.
    engine.midi_inject(0x90, 60, 100).unwrap();
    assert_eq!(peak(&engine, 4096), 0.0);
    engine.midi_inject(0x80, 60, 0).unwrap();
    peak(&engine, 512);

    doc.execute(Command::SetTrackArm { track: inst, arm: true }, Origin::User).unwrap();
    publish(&engine, &doc);
    engine.set_midi_capture(true);
    engine.play().unwrap();
    peak(&engine, 4096); // 8 blocks of 512
    engine.midi_inject(0x90, 64, 90).unwrap();
    assert!(peak(&engine, 9216) > 0.01, "the armed track plays the note");
    engine.midi_inject(0x80, 64, 0).unwrap();
    peak(&engine, 512);
    let got = engine.read_midi_capture();
    assert_eq!(got.len(), 2, "{got:?}");
    assert!(got[0].on && got[0].pitch == 64 && got[0].velocity == 90 && got[0].frame == 4096);
    assert!(!got[1].on && got[1].frame == 4096 + 9216);
    assert!(engine.read_midi_capture().is_empty());
    assert_eq!(engine.midi_stats().dropped, 0);
}

#[test]
fn capture_reports_a_loop_wrap_but_not_a_seek_back() {
    let engine = Engine::new(Config { sample_rate: RATE, max_block_size: 512 }).unwrap();
    let doc = Document::new(Project::default());
    publish(&engine, &doc);
    assert!(!engine.midi_capture_wrapped());
    engine.set_midi_capture(true);
    engine.seek(8192).unwrap();
    engine.play().unwrap();
    peak(&engine, 1024);
    engine.seek(0).unwrap();
    peak(&engine, 1024);
    assert!(!engine.midi_capture_wrapped(), "a seek back is not a wrap");
    engine.set_loop(true, 1024, 2048).unwrap();
    peak(&engine, 2048);
    assert!(engine.midi_capture_wrapped());
    engine.set_midi_capture(false);
    assert!(engine.midi_capture_wrapped(), "still answers after the pass");
    engine.set_midi_capture(true);
    assert!(!engine.midi_capture_wrapped(), "a new pass starts unwrapped");
}

#[test]
fn focus_track_plays_until_another_track_is_armed() {
    let engine = Engine::new(Config { sample_rate: RATE, max_block_size: 512 }).unwrap();
    let mut doc = Document::new(Project::default());
    let inst = doc.project().tracks.iter().find(|t| t.kind == TrackKind::Instrument).unwrap().id;
    let mut other = Track::new(TrackId(0), TrackKind::Instrument, "Other", ColorIndex::FIRST);
    other.instrument_plugin = None; // the built-in Synth
    doc.execute(Command::AddTrack { index: None, track: other }, Origin::User).unwrap();
    let other = doc.project().tracks.last().unwrap().id;
    doc.execute(Command::SetTrackMute { track: inst, mute: true }, Origin::User).unwrap();
    publish(&engine, &doc);
    engine.set_midi_focus(Some(other.0));
    engine.midi_inject(0x90, 60, 100).unwrap();
    assert!(peak(&engine, 4096) > 0.01, "the focus track (built-in Synth) plays");
    engine.midi_inject(0x80, 60, 0).unwrap();
    peak(&engine, 96_000);
    // Arming the muted track takes the input away from the focus track.
    doc.execute(Command::SetTrackArm { track: inst, arm: true }, Origin::User).unwrap();
    publish(&engine, &doc);
    engine.midi_inject(0x90, 60, 100).unwrap();
    assert!(peak(&engine, 4096) < 1e-3, "only what is left of the earlier note's tail");
}

#[test]
fn opening_a_midi_input_id_with_nul_is_an_invalid_argument() {
    let engine = Engine::new(Config { sample_rate: RATE, max_block_size: 512 }).unwrap();
    let err = engine.open_midi_inputs(&["keys\0x".to_string()]).unwrap_err();
    assert_eq!(err.kind, "invalid argument");
    engine.open_midi_inputs(&["not present".to_string()]).unwrap();
}
