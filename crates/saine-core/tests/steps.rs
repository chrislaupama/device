//! Pattern clips: step commands, undo, piano-roll edits, conversion to and
//! from plain MIDI, persistence and playback (snapshot repeats).

use saine_core::steps::{self, StepLane, StepPattern, StepRate, StepRows};
use saine_core::*;
use std::collections::HashMap;

const KICK: u8 = 36;
const SNARE: u8 = 37;

fn doc_with_pattern_clip() -> (Document, ClipId) {
    let mut d = Document::new(Project::default());
    let inst = d.project().tracks[1].id;
    d.execute(Command::AddMidiClip { track: inst, clip: MidiClip::empty_pattern(0, StepRows::Pads) }, Origin::User).unwrap();
    let c = d.project().tracks[1].midi_clips[0].id;
    (d, c)
}

fn beat() -> StepPattern {
    let mut p = StepPattern::default();
    for s in [0, 4, 8, 12] {
        p = p.toggled(KICK, s);
    }
    p.toggled(SNARE, 4).toggled(SNARE, 12)
}

fn clip(d: &Document, c: ClipId) -> MidiClip {
    d.project().midi_clip(c).unwrap().clone()
}

#[test]
fn new_pattern_clip_is_one_bar_of_sixteenths() {
    let (d, c) = doc_with_pattern_clip();
    let m = clip(&d, c);
    assert_eq!((m.name.as_str(), m.length), ("Pattern", 4 * PPQ));
    let p = m.pattern.unwrap();
    assert_eq!((p.rate, p.steps, p.swing), (StepRate::Sixteenth, 16, 0.0));
}

#[test]
fn step_edits_bake_notes_and_undo() {
    let (mut d, c) = doc_with_pattern_clip();
    d.execute(Command::SetStepPattern { clip: c, pattern: Some(beat()) }, Origin::User).unwrap();
    assert_eq!(d.undo_label(), Some("Edit Step Sequencer"));
    assert_eq!(clip(&d, c).notes.len(), 6);
    let louder = beat().with_lane(SNARE, 4, StepLane::Velocity, 127);
    d.execute(Command::SetStepPattern { clip: c, pattern: Some(louder.clone()) }, Origin::User).unwrap();
    assert_eq!(clip(&d, c).notes.iter().find(|n| n.pitch == SNARE).unwrap().velocity, 127);
    d.undo().unwrap();
    assert_eq!(clip(&d, c).pattern, Some(beat()));
    assert!(clip(&d, c).notes.iter().all(|n| n.velocity == 100));
    d.redo().unwrap();
    assert_eq!(clip(&d, c).pattern, Some(louder));
    d.undo().unwrap();
    d.undo().unwrap();
    assert!(clip(&d, c).notes.is_empty());
}

#[test]
fn invalid_patterns_are_refused() {
    let (mut d, c) = doc_with_pattern_clip();
    let mut bad = beat();
    bad.cells[0].step = 16;
    assert!(d.execute(Command::SetStepPattern { clip: c, pattern: Some(bad) }, Origin::User).is_err());
    let mut off_rows = StepPattern::default();
    off_rows.cells.push(steps::StepCell::new(60, 0));
    assert!(d.execute(Command::SetStepPattern { clip: c, pattern: Some(off_rows) }, Origin::User).is_err());
    let mut swing = beat();
    swing.swing = 1.5;
    assert!(d.execute(Command::SetStepPattern { clip: c, pattern: Some(swing) }, Origin::User).is_err());
    assert!(clip(&d, c).notes.is_empty());
}

#[test]
fn piano_roll_edit_rereads_only_the_edited_row() {
    let (mut d, c) = doc_with_pattern_clip();
    let p = beat().with_lane(KICK, 8, StepLane::Repeat, 3);
    d.execute(Command::SetStepPattern { clip: c, pattern: Some(p.clone()) }, Origin::User).unwrap();
    let mut notes = clip(&d, c).notes;
    notes.iter_mut().filter(|n| n.pitch == SNARE).for_each(|n| n.start += 240);
    d.execute(Command::SetMidiNotes { clip: c, notes }, Origin::User).unwrap();
    let q = clip(&d, c).pattern.unwrap();
    assert!(q.cell(SNARE, 5).is_some() && q.cell(SNARE, 13).is_some() && q.cell(SNARE, 4).is_none());
    assert_eq!(q.cell(KICK, 8).unwrap().repeat, 3, "the kick row was not touched");
    d.undo().unwrap();
    assert_eq!(clip(&d, c).pattern, Some(p));
}

#[test]
fn convert_to_plain_midi_and_back() {
    let (mut d, c) = doc_with_pattern_clip();
    d.execute(Command::SetStepPattern { clip: c, pattern: Some(beat()) }, Origin::User).unwrap();
    d.execute(Command::ResizeMidiClip { clip: c, start: 0, length: 8 * PPQ }, Origin::User).unwrap();
    d.execute(Command::SetStepPattern { clip: c, pattern: None }, Origin::User).unwrap();
    assert_eq!(d.undo_label(), Some("Convert to MIDI Clip"));
    let m = clip(&d, c);
    assert!(m.pattern.is_none());
    assert_eq!(m.notes.len(), 12, "the repeat became notes");
    let back = StepPattern::from_notes(&m.notes, m.length, StepRows::Pads);
    assert_eq!(back.steps, 32);
    assert_eq!(back.resized(16), beat());
    d.undo().unwrap();
    assert_eq!(clip(&d, c).pattern, Some(beat()));
    assert_eq!(clip(&d, c).notes.len(), 6);
}

#[test]
fn snapshot_repeats_the_pattern_over_the_clip() {
    let (mut d, c) = doc_with_pattern_clip();
    d.execute(Command::SetStepPattern { clip: c, pattern: Some(beat()) }, Origin::User).unwrap();
    d.execute(Command::ResizeMidiClip { clip: c, start: 4 * PPQ, length: 10 * PPQ }, Origin::User).unwrap();
    let (snap, _) = snapshot::build(d.project(), 48_000.0, &HashMap::new());
    let notes = &snap.tracks[1].notes;
    assert_eq!(notes.len(), 6 + 6 + 3);
    let frames_per_beat = 24_000; // 120 BPM at 48 kHz
    assert_eq!(notes[0].start, 4 * frames_per_beat);
    assert!(notes.iter().any(|n| n.start == 8 * frames_per_beat && n.pitch == KICK), "second repeat");
    assert!(notes.iter().all(|n| n.end <= 14 * frames_per_beat));
}

#[test]
fn pattern_clips_survive_save_and_load() {
    let (mut d, c) = doc_with_pattern_clip();
    let mut p = beat().with_lane(KICK, 0, StepLane::Tie, 2).with_lane(SNARE, 12, StepLane::Offset, -10);
    p.swing = 0.25;
    p.rate = StepRate::SixteenthTriplet;
    d.execute(Command::SetStepPattern { clip: c, pattern: Some(p.clone()) }, Origin::User).unwrap();
    let dir = tempfile::tempdir().unwrap();
    persist::save(d.project(), dir.path()).unwrap();
    let loaded = persist::load(dir.path()).unwrap();
    let m = loaded.project.midi_clip(c).unwrap();
    assert_eq!(m.pattern, Some(p));
    assert_eq!(m.notes, clip(&d, c).notes);
}

#[test]
fn plain_midi_clips_keep_their_file_format() {
    let mut d = Document::new(Project::default());
    let inst = d.project().tracks[1].id;
    d.execute(Command::AddMidiClip { track: inst, clip: MidiClip::empty(0, PPQ) }, Origin::User).unwrap();
    let json = serde_json::to_string(&d.project().tracks[1].midi_clips[0]).unwrap();
    assert!(!json.contains("pattern"), "no pattern field on plain clips: {json}");
    let old = r#"{"id":7,"name":"MIDI","start":0,"length":960,"notes":[]}"#;
    let m: MidiClip = serde_json::from_str(old).unwrap();
    assert!(m.pattern.is_none());
}

#[test]
fn step_commands_round_trip_as_json() {
    let cmd = Command::SetStepPattern { clip: ClipId(3), pattern: Some(beat()) };
    let json = serde_json::to_string(&cmd).unwrap();
    assert!(json.contains("\"1/16\""));
    assert_eq!(serde_json::from_str::<Command>(&json).unwrap(), cmd);
}
