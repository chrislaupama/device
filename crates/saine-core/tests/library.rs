//! The clip sidebar's pattern library: SetComposer edits (Create, Load, Original,
//! Type, Genre) with undo, persistence, and the library's own guarantees.

use saine_core::command::ComposerEdit;
use saine_core::library::{self, Composer, MadeSet, PartType};
use saine_core::*;

fn doc() -> (Document, ClipId) {
    let mut d = Document::new(Project::default());
    let inst = d.project().tracks[1].id;
    let mut clip = MidiClip::empty(0, 4 * PPQ);
    clip.notes.push(Note { id: 1, pitch: 60, start: 0, length: PPQ, velocity: 90 });
    d.execute(Command::AddMidiClip { track: inst, clip }, Origin::User).unwrap();
    let c = d.project().tracks[1].midi_clips[0].id;
    (d, c)
}

fn clip(d: &Document, c: ClipId) -> MidiClip {
    d.project().midi_clip(c).unwrap().clone()
}

/// What the sidebar's Create does: keep the clip's notes, make set 0, play row 0.
fn create(d: &mut Document, c: ClipId, part: PartType, genre: u8) {
    let m = clip(d, c);
    let made = MadeSet { part: Some(part), genre, set: 0, last: 0 };
    let comp = Composer { part, genre, made: vec![made], playing: Some(0), kept: m.notes.clone(), ..Default::default() };
    let first = library::patterns_of(made, None, m.length, 4 * PPQ).remove(0);
    d.execute(Command::SetComposer { clip: c, edit: ComposerEdit::Create, composer: Some(comp), notes: first.notes, pattern: first.pattern }, Origin::User).unwrap();
}

#[test]
fn create_plays_a_library_pattern_and_undo_restores_the_clip() {
    let (mut d, c) = doc();
    let before = clip(&d, c);
    create(&mut d, c, PartType::Bass, 3);
    assert_eq!(d.undo_label(), Some("Create Patterns"));
    let m = clip(&d, c);
    assert_ne!(m.notes, before.notes);
    assert!(!m.notes.is_empty());
    assert_eq!(m.composer.as_ref().unwrap().kept, before.notes, "the clip's own notes are kept");
    d.undo().unwrap();
    assert_eq!(clip(&d, c), before);
}

#[test]
fn original_brings_the_kept_notes_back() {
    let (mut d, c) = doc();
    let before = clip(&d, c).notes;
    create(&mut d, c, PartType::Chords, 0);
    let mut comp = clip(&d, c).composer.unwrap();
    comp.playing = None;
    let kept = std::mem::take(&mut comp.kept);
    d.execute(Command::SetComposer { clip: c, edit: ComposerEdit::Original, composer: Some(comp), notes: kept, pattern: None }, Origin::User).unwrap();
    assert_eq!(d.undo_label(), Some("Load Original"));
    assert_eq!(clip(&d, c).notes, before);
}

#[test]
fn composer_state_survives_save_and_load_and_bad_state_is_dropped() {
    let (mut d, c) = doc();
    create(&mut d, c, PartType::Arp, 5);
    let dir = tempfile::tempdir().unwrap();
    persist::save(d.project(), dir.path()).unwrap();
    let loaded = persist::load(dir.path()).unwrap().project;
    assert_eq!(loaded.midi_clip(c), d.project().midi_clip(c));
    let file = dir.path().join(persist::PROJECT_FILE);
    let json = std::fs::read_to_string(&file).unwrap().replacen("\"genre\": 5", "\"genre\": 99", 1);
    std::fs::write(&file, json).unwrap();
    let m = persist::load(dir.path()).unwrap().project.midi_clip(c).unwrap().clone();
    assert!(m.composer.is_none());
    assert_eq!(m.notes, clip(&d, c).notes, "the playing notes stay");
}

#[test]
fn invalid_composer_edits_are_refused() {
    let (mut d, c) = doc();
    let bad = Composer { genre: 200, ..Composer::default() };
    assert!(d.execute(Command::SetComposer { clip: c, edit: ComposerEdit::Genre, composer: Some(bad), notes: Vec::new(), pattern: None }, Origin::User).is_err());
}

#[test]
fn every_part_and_genre_makes_six_playable_patterns() {
    for part in PartType::ALL {
        for genre in 0..library::GENRES.len() {
            let set = library::create_set(part, genre, None, 0, 4 * PPQ, 4 * PPQ);
            assert_eq!(set.len(), library::SET_SIZE);
            for p in &set {
                assert!(!p.notes.is_empty(), "{part:?} {genre} {}", p.name);
                midi::validate_notes(&p.notes).unwrap();
                assert!(p.notes.iter().all(|n| n.start + n.length <= 4 * PPQ));
            }
        }
    }
    for genre in 0..library::GENRES.len() {
        for p in library::create_drum_set(genre, 2) {
            p.pattern.as_ref().unwrap().validate().unwrap();
        }
    }
}
