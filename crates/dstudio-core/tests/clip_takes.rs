//! Takes from a looped recording: choosing a take, undo, splitting (comping) and saving.

use dstudio_core::*;

/// A 2 s loop recorded three times round (a 6 s file): the clip at beat 4 plays the last pass.
fn doc() -> (Document, ClipId) {
    let mut d = Document::new(Project::default());
    let track = d.project().tracks[0].id;
    let asset = Asset {
        id: AssetId(0),
        relative_path: None,
        absolute_path: "/nonexistent/take.wav".into(),
        sample_rate: 48_000.0,
        channels: 1,
        frames: 48_000 * 6,
    };
    d.execute(Command::AddAsset { asset }, Origin::User).unwrap();
    let asset = d.project().assets[0].id;
    let clip = Clip {
        id: ClipId(0),
        asset,
        name: "Audio".into(),
        start: 4 * PPQ,
        source_offset: 2 * 96_000,
        source_length: 96_000,
        gain: 1.0,
        warp: Default::default(),
        disabled: false,
        clip_loop: None,
        fades: Default::default(),
        takes: vec![-192_000, -96_000, 0],
    };
    d.execute(Command::AddClip { track, clip }, Origin::User).unwrap();
    let c = d.project().tracks[0].clips[0].id;
    (d, c)
}

fn clip(d: &Document) -> &Clip {
    &d.project().tracks[0].clips[0]
}

#[test]
fn choosing_a_take_plays_its_pass_and_undoes() {
    let (mut d, c) = doc();
    assert_eq!(clip(&d).current_take(), Some(2));
    d.execute(Command::SetClipTake { clip: c, take: 0 }, Origin::User).unwrap();
    assert_eq!(d.undo_label(), Some("Choose Take"));
    assert_eq!(clip(&d).source_offset, 0, "take 1 is the first pass");
    assert_eq!(clip(&d).takes, vec![0, 96_000, 192_000]);
    assert_eq!(clip(&d).current_take(), Some(0));
    d.execute(Command::SetClipTake { clip: c, take: 1 }, Origin::User).unwrap();
    assert_eq!(clip(&d).source_offset, 96_000);
    d.undo().unwrap();
    d.undo().unwrap();
    assert_eq!(clip(&d).source_offset, 192_000);
    assert_eq!(clip(&d).takes, vec![-192_000, -96_000, 0]);
}

#[test]
fn a_missing_take_or_a_clip_without_takes_is_refused() {
    let (mut d, c) = doc();
    assert!(d.execute(Command::SetClipTake { clip: c, take: 3 }, Origin::User).is_err());
    let mut plain = clip(&d).clone();
    plain.id = ClipId(0);
    plain.takes.clear();
    let track = d.project().tracks[0].id;
    d.execute(Command::AddClip { track, clip: plain }, Origin::User).unwrap();
    let id = d.project().tracks[0].clips.iter().find(|c| c.takes.is_empty()).unwrap().id;
    assert!(d.execute(Command::SetClipTake { clip: id, take: 0 }, Origin::User).is_err());
}

#[test]
fn a_take_outside_the_recording_is_refused() {
    let (d, _) = doc();
    // Lengthened to 4 s from 2 s in: take 3 would need the file to run on to 8 s.
    let mut long = clip(&d).clone();
    long.source_length = 2 * 96_000;
    long.source_offset = 96_000;
    long.takes = vec![-96_000, 0, 96_000];
    assert!(long.with_take(2, 48_000 * 6).is_err(), "take 3 would run past the end of the file");
    assert!(long.with_take(0, 48_000 * 6).is_ok());
}

#[test]
fn splitting_keeps_the_takes_in_step_for_comping() {
    let (mut d, c) = doc();
    // Split at beat 6 (1 s into the clip), then play take 1 on the right part only.
    d.execute(Command::SplitClip { clip: c, at: 6 * PPQ, new_id: ClipId(0) }, Origin::User).unwrap();
    let right = d.project().tracks[0].clips[1].clone();
    assert_eq!(right.source_offset, 192_000 + 48_000);
    assert_eq!(right.takes, vec![-192_000, -96_000, 0]);
    d.execute(Command::SetClipTake { clip: right.id, take: 0 }, Origin::User).unwrap();
    let right = &d.project().tracks[0].clips[1];
    assert_eq!(right.source_offset, 48_000, "the second half of the first pass");
    assert_eq!(clip(&d).source_offset, 192_000, "the left part still plays take 3");
}

#[test]
fn takes_save_only_when_present() {
    let (d, _) = doc();
    let json = serde_json::to_string(clip(&d)).unwrap();
    assert!(json.contains("\"takes\":[-192000,-96000,0]"));
    let back: Clip = serde_json::from_str(&json).unwrap();
    assert_eq!(&back, clip(&d));
    let mut plain = clip(&d).clone();
    plain.takes.clear();
    let json = serde_json::to_string(&plain).unwrap();
    assert!(!json.contains("takes"));
    let back: Clip = serde_json::from_str(&json).unwrap();
    assert!(back.takes.is_empty(), "clips saved without takes load without them");
}
