use saine_core::*;
use std::collections::HashMap;
use std::time::{Duration, Instant};

fn asset(frames: u64, rate: f64) -> Asset {
    Asset { id: AssetId(0), relative_path: None, absolute_path: "/nonexistent/test.wav".into(), sample_rate: rate, channels: 2, frames }
}

fn clip(asset: AssetId, start: Ticks, len: u64) -> Clip {
    Clip { id: ClipId(0), asset, name: "c".into(), start, source_offset: 0, source_length: len, gain: 1.0, warp: Default::default() }
}

fn doc_with_asset() -> (Document, TrackId, AssetId) {
    let mut d = Document::new(Project::default());
    let track = d.project().tracks[0].id;
    d.execute(Command::AddAsset { asset: asset(48_000 * 10, 48_000.0) }, Origin::User).unwrap();
    let a = d.project().assets[0].id;
    (d, track, a)
}

#[test]
fn default_project_matches_reference() {
    let p = Project::default();
    assert_eq!(p.tracks.len(), 2);
    assert_eq!(p.tracks[0].name, "Audio");
    assert_eq!(p.tracks[1].kind, TrackKind::Instrument);
    assert_eq!(p.transport.bpm, 120.0);
    assert_eq!(p.ticks_per_bar(), 4 * PPQ);
}

#[test]
fn add_and_delete_track_undo_redo() {
    let mut d = Document::new(Project::default());
    let t = Track::new(TrackId(0), TrackKind::Audio, "Drums", ColorIndex(2));
    d.execute(Command::AddTrack { index: None, track: t }, Origin::User).unwrap();
    assert_eq!(d.project().tracks.len(), 3);
    let id = d.project().tracks[2].id;
    assert_eq!(d.undo_label(), Some("Add Audio Track"));
    d.undo().unwrap();
    assert_eq!(d.project().tracks.len(), 2);
    d.redo().unwrap();
    assert_eq!(d.project().tracks[2].id, id, "redo must recreate the same id");

    d.execute(Command::RemoveTrack { track: d.project().tracks[0].id }, Origin::User).unwrap();
    assert_eq!(d.project().tracks[0].name, "Instrument");
    d.undo().unwrap();
    assert_eq!(d.project().tracks[0].name, "Audio", "undo restores at the original index");
}

#[test]
fn validation_rejects_and_leaves_state_unchanged() {
    let (mut d, track, a) = doc_with_asset();
    let before = d.project().clone();
    let rev = d.revision();
    assert!(d.execute(Command::SetTrackGain { track, gain: 100.0 }, Origin::User).is_err());
    assert!(d.execute(Command::SetTrackPan { track, pan: f32::NAN }, Origin::User).is_err());
    assert!(d.execute(Command::SetTempo { bpm: 5.0 }, Origin::User).is_err());
    assert!(d.execute(Command::AddClip { track, clip: clip(a, 0, 48_000 * 11) }, Origin::User).is_err(), "longer than asset");
    assert!(d.execute(Command::AddClip { track: TrackId(999), clip: clip(a, 0, 10) }, Origin::User).is_err());
    let inst = d.project().tracks[1].id;
    d.execute(Command::AddClip { track, clip: clip(a, 0, 100) }, Origin::User).unwrap();
    let c = d.project().tracks[0].clips[0].id;
    assert!(d.execute(Command::MoveClip { clip: c, track: inst, start: 0 }, Origin::User).is_err(), "audio clip onto instrument track");
    d.undo().unwrap();
    // Ids are never recycled after undo, so only next_id may differ.
    let mut now = d.project().clone();
    now.next_id = before.next_id;
    assert_eq!(&now, &before);
    assert!(d.revision() > rev);
}

#[test]
fn clip_move_trim_undo() {
    let (mut d, track, a) = doc_with_asset();
    d.execute(Command::AddClip { track, clip: clip(a, 0, 48_000) }, Origin::User).unwrap();
    let c = d.project().tracks[0].clips[0].id;
    d.execute(Command::MoveClip { clip: c, track, start: 4 * PPQ }, Origin::User).unwrap();
    d.execute(Command::TrimClip { clip: c, start: 5 * PPQ, source_offset: 24_000, source_length: 24_000 }, Origin::User).unwrap();
    let cl = d.project().clip(c).unwrap();
    assert_eq!((cl.start, cl.source_offset, cl.source_length), (5 * PPQ, 24_000, 24_000));
    d.undo().unwrap();
    let cl = d.project().clip(c).unwrap();
    assert_eq!((cl.start, cl.source_offset, cl.source_length), (4 * PPQ, 0, 48_000));
    d.undo().unwrap();
    assert_eq!(d.project().clip(c).unwrap().start, 0);
    d.redo().unwrap();
    d.redo().unwrap();
    assert_eq!(d.project().clip(c).unwrap().source_offset, 24_000);
}

#[test]
fn fader_drag_coalesces_into_one_undo_step() {
    let (mut d, track, _) = doc_with_asset();
    let t0 = Instant::now();
    for i in 1..=20 {
        d.execute_at(Command::SetTrackGain { track, gain: i as f32 * 0.05 }, t0 + Duration::from_millis(i * 16)).unwrap();
    }
    assert_eq!(d.project().track(track).unwrap().gain, 1.0);
    d.undo().unwrap();
    assert_eq!(d.project().track(track).unwrap().gain, 1.0, "back to the value before the drag (default 1.0)");
    d.redo().unwrap();
    assert_eq!(d.project().track(track).unwrap().gain, 1.0);
    // A pause longer than the window starts a new step.
    d.execute_at(Command::SetTrackGain { track, gain: 0.5 }, t0 + Duration::from_secs(5)).unwrap();
    d.execute_at(Command::SetTrackGain { track, gain: 0.25 }, t0 + Duration::from_secs(10)).unwrap();
    d.undo().unwrap();
    assert_eq!(d.project().track(track).unwrap().gain, 0.5);
}

#[test]
fn batch_is_atomic_and_redo_reuses_ids() {
    let (mut d, track, a) = doc_with_asset();
    let before = d.project().clone();
    let bad = Command::Batch {
        label: "Bad".into(),
        commands: vec![Command::AddClip { track, clip: clip(a, 0, 10) }, Command::SetTempo { bpm: 0.0 }],
    };
    assert!(d.execute(bad, Origin::Assistant).is_err());
    assert_eq!(d.project(), &before, "failed batch rolls back fully (including next_id)");

    let good = Command::Batch {
        label: "Add Two Clips".into(),
        commands: vec![Command::AddClip { track, clip: clip(a, 0, 10) }, Command::AddClip { track, clip: clip(a, PPQ, 10) }],
    };
    d.execute(good, Origin::Assistant).unwrap();
    let ids: Vec<_> = d.project().tracks[0].clips.iter().map(|c| c.id).collect();
    assert_eq!(d.undo_label(), Some("Add Two Clips"));
    d.undo().unwrap();
    assert!(d.project().tracks[0].clips.is_empty());
    d.redo().unwrap();
    let ids2: Vec<_> = d.project().tracks[0].clips.iter().map(|c| c.id).collect();
    assert_eq!(ids, ids2);
}

#[test]
fn commands_round_trip_as_json_for_ai_and_scripting() {
    let json = r#"{"op":"set_tempo","bpm":128.0}"#;
    let cmd: Command = serde_json::from_str(json).unwrap();
    assert_eq!(cmd, Command::SetTempo { bpm: 128.0 });
    let cmd = Command::SetTrackPan { track: TrackId(3), pan: -0.5 };
    let back: Command = serde_json::from_str(&serde_json::to_string(&cmd).unwrap()).unwrap();
    assert_eq!(cmd, back);
}

#[test]
fn save_and_reopen_project() {
    let dir = tempfile::tempdir().unwrap();
    let media = dir.path().join("media");
    std::fs::create_dir_all(&media).unwrap();
    let wav = media.join("kick.wav");
    std::fs::write(&wav, b"stub").unwrap();

    let mut d = Document::new(Project::default());
    let track = d.project().tracks[0].id;
    let mut a = asset(1000, 44_100.0);
    a.absolute_path = wav.to_string_lossy().into_owned();
    d.execute(Command::AddAsset { asset: a }, Origin::User).unwrap();
    let aid = d.project().assets[0].id;
    d.execute(Command::AddClip { track, clip: clip(aid, 2 * PPQ, 500) }, Origin::User).unwrap();
    d.execute(Command::SetTempo { bpm: 97.5 }, Origin::User).unwrap();

    let proj_dir = dir.path().join("Song.saineproj");
    persist::save(d.project(), &proj_dir).unwrap();
    let loaded = persist::load(&proj_dir).unwrap();
    assert!(loaded.missing_media.is_empty());
    let mut expected = d.project().clone();
    expected.assets[0].relative_path = Some("../media/kick.wav".into());
    // macOS temp dirs live behind the /var -> /private/var symlink; compare canonically.
    let canon = |s: &str| std::fs::canonicalize(s).unwrap();
    assert_eq!(canon(&loaded.project.assets[0].absolute_path), canon(&expected.assets[0].absolute_path));
    let mut loaded_cmp = loaded.project.clone();
    loaded_cmp.assets[0].absolute_path = expected.assets[0].absolute_path.clone();
    assert_eq!(loaded_cmp, expected);

    // Move project + media together: relative path wins.
    let moved = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(moved.path().join("media")).unwrap();
    std::fs::copy(&wav, moved.path().join("media/kick.wav")).unwrap();
    let moved_proj = moved.path().join("Song.saineproj");
    std::fs::create_dir_all(&moved_proj).unwrap();
    std::fs::copy(proj_dir.join("project.json"), moved_proj.join("project.json")).unwrap();
    std::fs::remove_file(&wav).unwrap();
    let l2 = persist::load(&moved_proj).unwrap();
    assert!(l2.missing_media.is_empty());
    assert!(l2.project.assets[0].absolute_path.starts_with(&*moved.path().to_string_lossy()));
}

#[test]
fn load_rejects_newer_schema_and_foreign_files() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("project.json"), r#"{"format":"saine-project","schema_version":99,"saved_with":"x","project":{}}"#).unwrap();
    assert!(matches!(persist::load(dir.path()), Err(persist::PersistError::TooNew { .. })));
    std::fs::write(dir.path().join("project.json"), r#"{"format":"other","schema_version":1,"saved_with":"x","project":{}}"#).unwrap();
    assert!(matches!(persist::load(dir.path()), Err(persist::PersistError::WrongFormat(_))));
}

#[test]
fn snapshot_converts_musical_time_to_engine_frames() {
    let (mut d, track, a) = doc_with_asset();
    // Asset at 44.1k played by a 48k engine.
    d.execute(Command::AddAsset { asset: asset(44_100, 44_100.0) }, Origin::User).unwrap();
    let a2 = d.project().assets[1].id;
    d.execute(Command::AddClip { track, clip: Clip { source_offset: 22_050, ..clip(a2, 4 * PPQ, 22_050) } }, Origin::User).unwrap();
    d.execute(Command::AddClip { track, clip: clip(a, 0, 100) }, Origin::User).unwrap();
    let mut map = HashMap::new();
    map.insert(a2, 7u64);
    let (snap, skipped) = snapshot::build(d.project(), 48_000.0, &map);
    assert_eq!(skipped.len(), 1, "clip of unloaded asset is skipped");
    let c = &snap.tracks[0].clips[0];
    // Bar 2 at 120 BPM = 2 s = 96000 frames.
    assert_eq!(c.timeline_start, 96_000);
    assert_eq!(c.source_offset, 24_000);
    assert_eq!(c.length, 24_000);
    assert_eq!(c.engine_asset, 7);
}

#[test]
fn db_conversions() {
    assert!((db_to_gain(-6.0) - 0.501).abs() < 1e-3);
    assert!((gain_to_db(0.5) + 6.02).abs() < 1e-2);
    assert_eq!(db_to_gain(-200.0), 0.0);
}

#[test]
fn seal_coalescing_starts_a_new_undo_step() {
    let (mut d, track, _) = doc_with_asset();
    let t0 = Instant::now();
    d.execute_at(Command::SetTrackGain { track, gain: 0.5 }, t0).unwrap();
    d.seal_coalescing();
    d.execute_at(Command::SetTrackGain { track, gain: 0.25 }, t0 + Duration::from_millis(10)).unwrap();
    d.undo().unwrap();
    assert_eq!(d.project().track(track).unwrap().gain, 0.5, "second gesture is its own step");
}

#[test]
fn track_and_panel_heights_are_view_changes_not_undo_steps() {
    let mut d = Document::new(Project::default());
    let track = d.project().tracks[0].id;
    assert!(d.set_track_height(track, 0.1).is_err());
    assert!(d.set_track_height(track, f32::NAN).is_err());
    assert!(d.set_track_height(TrackId(999), 1.0).is_err());
    let rev = d.revision();
    d.set_track_height(track, 2.0).unwrap();
    assert_eq!(d.project().track(track).unwrap().height, 2.0);
    assert!(d.revision() > rev, "a track height marks the project dirty");
    assert!(!d.can_undo(), "but is not an undo step");

    let rev = d.revision();
    d.set_bottom_panel_height("devices", Some(420.0));
    assert!(d.revision() > rev, "a panel height marks the project dirty");
    assert!(!d.can_undo(), "but is not an undo step");
    d.set_bottom_panel_height("devices", None);
    assert!(d.project().bottom_panel_heights.is_empty());
}

#[test]
fn track_and_panel_heights_persist_and_default_when_absent() {
    let dir = tempfile::tempdir().unwrap();
    let mut d = Document::new(Project::default());
    let track = d.project().tracks[1].id;
    d.set_track_height(track, 0.5).unwrap();
    d.set_bottom_panel_height("mixer", Some(500.0));
    persist::save(d.project(), dir.path()).unwrap();
    let loaded = persist::load(dir.path()).unwrap().project;
    assert_eq!(loaded.track(track).unwrap().height, 0.5);
    assert_eq!(loaded.bottom_panel_heights.get("mixer"), Some(&500.0));

    let json = serde_json::to_string(&Project::default()).unwrap();
    assert!(!json.contains("\"height\"") && !json.contains("bottom_panel_heights"), "defaults are not written");
    let old: Project = serde_json::from_str(&json).unwrap();
    assert!(old.tracks.iter().all(|t| t.height == 1.0));
}

#[test]
fn arm_monitor_input_commands() {
    let mut d = Document::new(Project::default());
    let (audio, inst) = (d.project().tracks[0].id, d.project().tracks[1].id);
    let input = InputSource { channel: 0, stereo: false };
    d.execute(Command::SetTrackInput { track: audio, input: Some(input) }, Origin::User).unwrap();
    d.execute(Command::SetTrackArm { track: audio, arm: true }, Origin::User).unwrap();
    d.execute(Command::SetTrackMonitor { track: audio, monitor: true }, Origin::User).unwrap();
    assert!(d.execute(Command::SetTrackArm { track: inst, arm: true }, Origin::User).is_err());
    assert!(d.execute(Command::SetTrackInput { track: inst, input: Some(input) }, Origin::User).is_err());
    let (snap, _) = snapshot::build(d.project(), 48_000.0, &HashMap::new());
    assert!(snap.tracks[0].arm && snap.tracks[0].monitor && snap.tracks[0].input == Some(input));
    assert_eq!(d.undo_label(), Some("Input Monitoring Change"));
    d.undo().unwrap();
    d.undo().unwrap();
    d.undo().unwrap();
    let t = d.project().track(audio).unwrap();
    assert!(!t.arm && !t.monitor && t.input.is_none());
}
