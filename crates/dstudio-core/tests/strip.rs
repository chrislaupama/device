//! Channel-strip trim, width and mono (spec 004): commands, undo, coalescing,
//! validation, persistence and the snapshot the engine receives.
use dstudio_core::*;
use std::collections::HashMap;
use std::time::{Duration, Instant};

fn doc() -> (Document, TrackId) {
    let d = Document::new(Project::default());
    let track = d.project().tracks[0].id;
    (d, track)
}

#[test]
fn strip_commands_apply_undo_and_label() {
    let (mut d, track) = doc();
    d.execute(Command::SetTrackTrim { track, trim_db: -6.0 }, Origin::User).unwrap();
    assert_eq!(d.undo_label(), Some("Trim Change"));
    d.seal_coalescing();
    d.execute(Command::SetTrackWidth { track, width: 1.5 }, Origin::User).unwrap();
    assert_eq!(d.undo_label(), Some("Width Change"));
    d.execute(Command::SetTrackMono { track, mono: true }, Origin::User).unwrap();
    assert_eq!(d.undo_label(), Some("Mono Change"));
    let t = d.project().track(track).unwrap();
    assert_eq!((t.trim_db, t.width, t.mono), (-6.0, 1.5, true));
    for _ in 0..3 {
        d.undo().unwrap();
    }
    let t = d.project().track(track).unwrap();
    assert_eq!((t.trim_db, t.width, t.mono), (0.0, 1.0, false), "undo restores the neutral strip");
    d.redo().unwrap();
    assert_eq!(d.project().track(track).unwrap().trim_db, -6.0);
}

#[test]
fn strip_values_out_of_range_are_rejected_and_change_nothing() {
    let (mut d, track) = doc();
    let before = d.project().clone();
    for cmd in [
        Command::SetTrackTrim { track, trim_db: 24.5 },
        Command::SetTrackTrim { track, trim_db: f32::NAN },
        Command::SetTrackWidth { track, width: 2.01 },
        Command::SetTrackWidth { track, width: -0.1 },
        Command::SetTrackMono { track: TrackId(999), mono: true },
    ] {
        assert!(d.execute(cmd, Origin::User).is_err());
    }
    assert_eq!(*d.project(), before);
}

#[test]
fn a_knob_drag_is_one_undo_step() {
    let (mut d, track) = doc();
    let t0 = Instant::now();
    // Pressed: the first move opens the step and the hold keeps it open across pauses.
    d.execute_at(Command::SetTrackWidth { track, width: 1.05 }, t0).unwrap();
    d.hold_coalescing();
    for i in 2..=12 {
        d.execute_at(Command::SetTrackWidth { track, width: 1.0 + i as f32 * 0.05 }, t0 + Duration::from_secs(i)).unwrap();
    }
    d.seal_coalescing();
    d.undo().unwrap();
    assert_eq!(d.project().track(track).unwrap().width, 1.0);
}

#[test]
fn strip_persists_and_old_projects_load_neutral() {
    let dir = tempfile::tempdir().unwrap();
    let (mut d, track) = doc();
    d.execute(Command::SetTrackTrim { track, trim_db: 3.5 }, Origin::User).unwrap();
    d.execute(Command::SetTrackWidth { track, width: 0.25 }, Origin::User).unwrap();
    d.execute(Command::SetTrackMono { track, mono: true }, Origin::User).unwrap();
    persist::save(d.project(), dir.path()).unwrap();
    let t = persist::load(dir.path()).unwrap().project.track(track).unwrap().clone();
    assert_eq!((t.trim_db, t.width, t.mono), (3.5, 0.25, true));

    let json = serde_json::to_string(&Project::default()).unwrap();
    assert!(!json.contains("\"trim\"") && !json.contains("\"width\"") && !json.contains("\"mono\""), "defaults are not written");
    let old: Project = serde_json::from_str(&json).unwrap();
    assert!(old.tracks.iter().all(Track::strip_is_neutral));
}

#[test]
fn snapshot_carries_linear_trim_width_and_mono() {
    let (mut d, track) = doc();
    d.execute(Command::SetTrackTrim { track, trim_db: 20.0 }, Origin::User).unwrap();
    d.execute(Command::SetTrackMono { track, mono: true }, Origin::User).unwrap();
    let (snap, _) = snapshot::build(d.project(), 48_000.0, &HashMap::new());
    let t = snap.tracks.iter().find(|t| t.track_id == track.0).unwrap();
    assert!((t.trim - 10.0).abs() < 1e-5 && t.width == 1.0 && t.mono);
}
