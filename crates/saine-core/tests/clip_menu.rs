use saine_core::midi::Note;
use saine_core::*;
use std::collections::HashMap;

fn audio_doc() -> (Document, TrackId, ClipId) {
    let mut d = Document::new(Project::default());
    let track = d.project().tracks[0].id;
    let asset = Asset { id: AssetId(0), relative_path: None, absolute_path: "/nonexistent/t.wav".into(), sample_rate: 48_000.0, channels: 2, frames: 48_000 * 10 };
    d.execute(Command::AddAsset { asset }, Origin::User).unwrap();
    let a = d.project().assets[0].id;
    let clip = Clip { id: ClipId(0), asset: a, name: "loop".into(), start: 4 * PPQ, source_offset: 0, source_length: 48_000 * 4, gain: 1.0, warp: Default::default(), disabled: false };
    d.execute(Command::AddClip { track, clip }, Origin::User).unwrap();
    let c = d.project().tracks[0].clips[0].id;
    (d, track, c)
}

fn midi_doc() -> (Document, ClipId) {
    let mut d = Document::new(Project::default());
    let inst = d.project().tracks[1].id;
    let mut clip = MidiClip::empty(PPQ, 4 * PPQ);
    clip.notes = vec![Note { id: 1, pitch: 60, start: 0, length: PPQ, velocity: 100 }, Note { id: 2, pitch: 64, start: PPQ, length: 2 * PPQ, velocity: 90 }, Note { id: 3, pitch: 67, start: 3 * PPQ, length: PPQ, velocity: 80 }];
    d.execute(Command::AddMidiClip { track: inst, clip }, Origin::User).unwrap();
    let c = d.project().tracks[1].midi_clips[0].id;
    (d, c)
}

#[test]
fn split_audio_clip_divides_the_source_and_undoes() {
    let (mut d, _, c) = audio_doc();
    let before = d.project().tracks[0].clips[0].clone();
    d.execute(Command::SplitClip { clip: c, at: 4 * PPQ + 2 * PPQ, new_id: ClipId(0) }, Origin::User).unwrap();
    let clips = &d.project().tracks[0].clips;
    assert_eq!(clips.len(), 2);
    assert_eq!((clips[0].id, clips[0].start, clips[0].source_offset, clips[0].source_length), (c, 4 * PPQ, 0, 48_000));
    assert_eq!((clips[1].start, clips[1].source_offset, clips[1].source_length), (6 * PPQ, 48_000, 48_000 * 3));
    assert_eq!(clips[1].name, "loop");
    assert_eq!(d.undo_label(), Some("Split Clip"));
    d.undo().unwrap();
    assert_eq!(d.project().tracks[0].clips, vec![before]);
    d.redo().unwrap();
    assert_eq!(d.project().tracks[0].clips.len(), 2);
}

#[test]
fn split_audio_clip_refuses_the_edges_and_warped_clips() {
    let (mut d, _, c) = audio_doc();
    for at in [4 * PPQ, 5 * PPQ + 4 * 2 * PPQ, 0] {
        assert!(d.execute(Command::SplitClip { clip: c, at, new_id: ClipId(0) }, Origin::User).is_err(), "at {at}");
    }
    let warp = ClipWarp { reverse: true, ..ClipWarp::default() };
    d.execute(Command::SetClipWarp { clip: c, warp, label: "Reverse".into() }, Origin::User).unwrap();
    assert!(d.execute(Command::SplitClip { clip: c, at: 6 * PPQ, new_id: ClipId(0) }, Origin::User).is_err());
    assert_eq!(d.project().tracks[0].clips.len(), 1);
}

#[test]
fn split_midi_clip_cuts_crossing_notes_and_undoes() {
    let (mut d, c) = midi_doc();
    let before = d.project().tracks[1].midi_clips[0].clone();
    d.execute(Command::SplitClip { clip: c, at: PPQ + 2 * PPQ, new_id: ClipId(0) }, Origin::User).unwrap();
    let clips = &d.project().tracks[1].midi_clips;
    assert_eq!(clips.len(), 2);
    let (l, r) = (&clips[0], &clips[1]);
    assert_eq!((l.id, l.start, l.length), (c, PPQ, 2 * PPQ));
    assert_eq!((r.start, r.length), (3 * PPQ, 2 * PPQ));
    assert_eq!(l.notes.iter().map(|n| (n.pitch, n.start, n.length)).collect::<Vec<_>>(), vec![(60, 0, PPQ), (64, PPQ, PPQ)]);
    assert_eq!(r.notes.iter().map(|n| (n.pitch, n.start, n.length)).collect::<Vec<_>>(), vec![(64, 0, PPQ), (67, PPQ, PPQ)]);
    d.undo().unwrap();
    assert_eq!(d.project().tracks[1].midi_clips, vec![before]);
}

#[test]
fn split_refuses_pattern_clips_and_taken_ids() {
    let (mut d, c) = midi_doc();
    assert!(d.execute(Command::SplitClip { clip: c, at: 2 * PPQ, new_id: c }, Origin::User).is_err());
    assert!(d.execute(Command::SplitClip { clip: ClipId(9999), at: 2 * PPQ, new_id: ClipId(0) }, Origin::User).is_err());
    let mut p = Project::default();
    let inst = p.tracks[1].id;
    let pat = MidiClip::empty_pattern(0, StepRows::default());
    let mut d = Document::new(std::mem::take(&mut p));
    d.execute(Command::AddMidiClip { track: inst, clip: pat }, Origin::User).unwrap();
    let id = d.project().tracks[1].midi_clips[0].id;
    assert!(d.execute(Command::SplitClip { clip: id, at: PPQ, new_id: ClipId(0) }, Origin::User).is_err());
}

#[test]
fn disable_clip_toggles_and_undoes_for_audio_and_midi() {
    let (mut d, _, c) = audio_doc();
    d.execute(Command::SetClipEnabled { clip: c, enabled: false }, Origin::User).unwrap();
    assert!(d.project().tracks[0].clips[0].disabled);
    assert_eq!(d.undo_label(), Some("Disable Clip"));
    d.undo().unwrap();
    assert!(!d.project().tracks[0].clips[0].disabled);
    let (mut d, m) = midi_doc();
    d.execute(Command::SetClipEnabled { clip: m, enabled: false }, Origin::User).unwrap();
    assert!(d.project().midi_clip(m).unwrap().disabled);
    d.execute(Command::SetClipEnabled { clip: m, enabled: true }, Origin::User).unwrap();
    assert_eq!(d.undo_label(), Some("Enable Clip"));
    assert!(d.execute(Command::SetClipEnabled { clip: ClipId(9999), enabled: false }, Origin::User).is_err());
}

#[test]
fn disabled_clips_are_left_out_of_the_session_snapshot_and_persist() {
    let (mut d, _, c) = audio_doc();
    let a = d.project().assets[0].id;
    let engine_assets = HashMap::from([(a, 1u64)]);
    let (s, _) = snapshot::build(d.project(), 48_000.0, &engine_assets);
    assert_eq!(s.tracks[0].clips.len(), 1);
    d.execute(Command::SetClipEnabled { clip: c, enabled: false }, Origin::User).unwrap();
    let (s, _) = snapshot::build(d.project(), 48_000.0, &engine_assets);
    assert!(s.tracks[0].clips.is_empty());
    let json = serde_json::to_string(d.project()).unwrap();
    let back: Project = serde_json::from_str(&json).unwrap();
    assert!(back.tracks[0].clips[0].disabled);

    let (mut d, m) = midi_doc();
    let (s, _) = snapshot::build(d.project(), 48_000.0, &HashMap::new());
    assert_eq!(s.tracks[1].notes.len(), 3);
    d.execute(Command::SetClipEnabled { clip: m, enabled: false }, Origin::User).unwrap();
    let (s, _) = snapshot::build(d.project(), 48_000.0, &HashMap::new());
    assert!(s.tracks[1].notes.is_empty());
}
