//! Audio clip fades: the command, clamping, splitting, saving and the engine snapshot.

use dstudio_core::*;
use std::collections::HashMap;

/// A 4 s clip (8 beats at 120 bpm) at beat 4 on the first track of a new project.
fn doc() -> (Document, ClipId) {
    let mut d = Document::new(Project::default());
    let track = d.project().tracks[0].id;
    let asset = Asset {
        id: AssetId(0),
        relative_path: None,
        absolute_path: "/nonexistent/t.wav".into(),
        sample_rate: 48_000.0,
        channels: 2,
        frames: 48_000 * 10,
    };
    d.execute(Command::AddAsset { asset }, Origin::User).unwrap();
    let asset = d.project().assets[0].id;
    let clip = Clip {
        id: ClipId(0),
        asset,
        name: "pad".into(),
        start: 4 * PPQ,
        source_offset: 0,
        source_length: 48_000 * 4,
        gain: 1.0,
        warp: Default::default(),
        disabled: false,
        clip_loop: None,
        fades: Default::default(),
        takes: Vec::new(),
    };
    d.execute(Command::AddClip { track, clip }, Origin::User).unwrap();
    let c = d.project().tracks[0].clips[0].id;
    (d, c)
}

fn fades(d: &Document) -> ClipFades {
    d.project().tracks[0].clips[0].fades
}

#[test]
fn set_fades_undoes_and_rejects_negative_lengths() {
    let (mut d, c) = doc();
    let f = ClipFades { fade_in: PPQ, fade_out: 2 * PPQ };
    d.execute(Command::SetClipFades { clip: c, fades: f }, Origin::User).unwrap();
    assert_eq!(fades(&d), f);
    assert_eq!(d.undo_label(), Some("Clip Fade Change"));
    d.undo().unwrap();
    assert_eq!(fades(&d), ClipFades::default());
    let bad = ClipFades { fade_in: -1, fade_out: 0 };
    assert!(d.execute(Command::SetClipFades { clip: c, fades: bad }, Origin::User).is_err());
}

#[test]
fn fades_clamp_to_the_clip_without_overlapping() {
    let f = ClipFades { fade_in: 6 * PPQ, fade_out: 6 * PPQ };
    assert_eq!(f.clamped(8 * PPQ), ClipFades { fade_in: 6 * PPQ, fade_out: 2 * PPQ });
    assert_eq!(f.clamped(4 * PPQ), ClipFades { fade_in: 4 * PPQ, fade_out: 0 });
    assert_eq!(ClipFades { fade_in: PPQ, fade_out: PPQ }.clamped(8 * PPQ), ClipFades { fade_in: PPQ, fade_out: PPQ });
}

#[test]
fn split_gives_the_fade_in_to_the_left_part_and_the_fade_out_to_the_right() {
    let (mut d, c) = doc();
    let f = ClipFades { fade_in: PPQ, fade_out: 2 * PPQ };
    d.execute(Command::SetClipFades { clip: c, fades: f }, Origin::User).unwrap();
    d.execute(Command::SplitClip { clip: c, at: 8 * PPQ, new_id: ClipId(0) }, Origin::User).unwrap();
    let clips = &d.project().tracks[0].clips;
    assert_eq!(clips[0].fades, ClipFades { fade_in: PPQ, fade_out: 0 });
    assert_eq!(clips[1].fades, ClipFades { fade_in: 0, fade_out: 2 * PPQ });
    d.undo().unwrap();
    assert_eq!(d.project().tracks[0].clips.len(), 1);
    assert_eq!(fades(&d), f);
}

#[test]
fn fades_save_only_when_set_and_old_projects_load_without_them() {
    let (mut d, c) = doc();
    let plain = serde_json::to_string(&d.project().tracks[0].clips[0]).unwrap();
    assert!(!plain.contains("fades"), "{plain}");
    let back: Clip = serde_json::from_str(&plain).unwrap();
    assert_eq!(back.fades, ClipFades::default());
    let f = ClipFades { fade_in: PPQ / 2, fade_out: 0 };
    d.execute(Command::SetClipFades { clip: c, fades: f }, Origin::User).unwrap();
    let json = serde_json::to_string(&d.project().tracks[0].clips[0]).unwrap();
    let back: Clip = serde_json::from_str(&json).unwrap();
    assert_eq!(back.fades, f);
}

#[test]
fn snapshot_carries_fades_in_timeline_frames() {
    let (mut d, c) = doc();
    let assets: HashMap<AssetId, u64> = d.project().assets.iter().map(|a| (a.id, 7)).collect();
    let (s, _) = snapshot::build(d.project(), 48_000.0, &assets);
    assert_eq!(s.tracks[0].clips[0].fades, (0, 0));
    // One beat in (0.5 s at 120 bpm), two beats out; then a fade-out longer than what is left.
    d.execute(Command::SetClipFades { clip: c, fades: ClipFades { fade_in: PPQ, fade_out: 2 * PPQ } }, Origin::User).unwrap();
    let (s, _) = snapshot::build(d.project(), 48_000.0, &assets);
    assert_eq!(s.tracks[0].clips[0].fades, (24_000, 48_000));
    d.execute(Command::SetClipFades { clip: c, fades: ClipFades { fade_in: 6 * PPQ, fade_out: 6 * PPQ } }, Origin::User).unwrap();
    let (s, _) = snapshot::build(d.project(), 48_000.0, &assets);
    let cs = &s.tracks[0].clips[0];
    assert_eq!(cs.fades, (144_000, 48_000));
    assert!(cs.fades.0 + cs.fades.1 <= cs.length);
}
