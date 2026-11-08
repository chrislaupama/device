use saine_core::midi::{self, Note};
use saine_core::*;
use std::collections::HashMap;

fn note(id: u32, pitch: u8, start: Ticks, length: Ticks) -> Note {
    Note { id, pitch, start, length, velocity: 100 }
}

fn doc_with_midi_clip() -> (Document, TrackId, ClipId) {
    let mut d = Document::new(Project::default());
    let inst = d.project().tracks[1].id;
    d.execute(Command::AddMidiClip { track: inst, clip: MidiClip::empty(4 * PPQ, 4 * PPQ) }, Origin::User).unwrap();
    let c = d.project().tracks[1].midi_clips[0].id;
    (d, inst, c)
}

#[test]
fn midi_clips_only_on_instrument_tracks() {
    let mut d = Document::new(Project::default());
    let audio = d.project().tracks[0].id;
    assert!(d.execute(Command::AddMidiClip { track: audio, clip: MidiClip::empty(0, PPQ) }, Origin::User).is_err());
    let (mut d, _, c) = doc_with_midi_clip();
    let audio = d.project().tracks[0].id;
    assert!(d.execute(Command::MoveMidiClip { clip: c, track: audio, start: 0 }, Origin::User).is_err());
}

#[test]
fn note_edits_are_single_undo_steps() {
    let (mut d, _, c) = doc_with_midi_clip();
    let notes = vec![note(1, 60, 0, 240), note(2, 64, 480, 240)];
    d.execute(Command::SetMidiNotes { clip: c, notes: notes.clone() }, Origin::User).unwrap();
    let clip = d.project().midi_clip(c).unwrap().clone();
    let q = midi::transpose(&clip.notes, 12).unwrap();
    d.execute(Command::SetMidiNotes { clip: c, notes: q }, Origin::User).unwrap();
    assert_eq!(d.project().midi_clip(c).unwrap().notes[0].pitch, 72);
    assert_eq!(d.undo_label(), Some("Edit Notes"));
    d.undo().unwrap();
    assert_eq!(d.project().midi_clip(c).unwrap().notes, notes);
    d.undo().unwrap();
    assert!(d.project().midi_clip(c).unwrap().notes.is_empty());
    // invalid notes rejected, state unchanged
    assert!(d.execute(Command::SetMidiNotes { clip: c, notes: vec![Note { velocity: 0, ..note(1, 60, 0, 10) }] }, Origin::User).is_err());
}

#[test]
fn clip_move_resize_delete_undo_and_ids() {
    let (mut d, inst, c) = doc_with_midi_clip();
    d.execute(Command::MoveMidiClip { clip: c, track: inst, start: 8 * PPQ }, Origin::User).unwrap();
    d.execute(Command::ResizeMidiClip { clip: c, start: 8 * PPQ, length: 8 * PPQ }, Origin::User).unwrap();
    assert_eq!(d.project().midi_clip(c).unwrap().end(), 16 * PPQ);
    d.execute(Command::RemoveMidiClip { clip: c }, Origin::User).unwrap();
    assert!(d.project().midi_clip(c).is_none());
    d.undo().unwrap();
    d.undo().unwrap();
    d.undo().unwrap();
    assert_eq!(d.project().midi_clip(c).unwrap().start, 4 * PPQ);
    // ids are shared with audio clips: a MIDI clip id can't be reused by AddClip
    d.execute(Command::AddAsset { asset: Asset { id: AssetId(0), relative_path: None, absolute_path: "/x.wav".into(), sample_rate: 48_000.0, channels: 1, frames: 100 } }, Origin::User).unwrap();
    let a = d.project().assets[0].id;
    let audio = d.project().tracks[0].id;
    let dup = Clip { id: c, asset: a, name: "x".into(), start: 0, source_offset: 0, source_length: 10, gain: 1.0 };
    assert_eq!(d.execute(Command::AddClip { track: audio, clip: dup }, Origin::User), Err(EditError::DuplicateId(c.0)));
}

#[test]
fn snapshot_emits_notes_in_frames_cut_at_clip_end() {
    let (mut d, _, c) = doc_with_midi_clip(); // clip at beat 4 (2 s at 120 BPM), 4 beats long
    d.execute(
        Command::SetMidiNotes { clip: c, notes: vec![note(1, 60, 0, PPQ), note(2, 67, 3 * PPQ, 2 * PPQ), note(3, 72, 5 * PPQ, PPQ)] },
        Origin::User,
    )
    .unwrap();
    let (snap, _) = snapshot::build(d.project(), 48_000.0, &HashMap::new());
    let t = &snap.tracks[1];
    assert!(t.instrument && !snap.tracks[0].instrument);
    assert_eq!(t.notes.len(), 2, "note starting after the clip end is dropped");
    assert_eq!((t.notes[0].start, t.notes[0].end, t.notes[0].pitch), (96_000, 120_000, 60));
    // second note runs past the clip end (beat 8) and is cut there
    assert_eq!((t.notes[1].start, t.notes[1].end), (168_000, 192_000));
}

#[test]
fn v1_project_files_without_midi_still_load() {
    let dir = tempfile::tempdir().unwrap();
    let mut p = Project::default();
    p.name = "Old".into();
    persist::save(&p, dir.path()).unwrap();
    // Strip the midi_clips field to simulate a file written before MIDI existed.
    let path = dir.path().join("project.json");
    let txt = std::fs::read_to_string(&path).unwrap().replace("\"midi_clips\": [],", "").replace(",\n        \"midi_clips\": []", "");
    assert!(!txt.contains("midi_clips"));
    std::fs::write(&path, txt).unwrap();
    let l = persist::load(dir.path()).unwrap();
    assert!(l.project.tracks.iter().all(|t| t.midi_clips.is_empty()));
}
