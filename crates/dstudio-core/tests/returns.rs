//! Return tracks, sends and output routing (spec 002).

use dstudio_core::*;
use std::time::{Duration, Instant};

fn add_return(d: &mut Document) -> TrackId {
    let name = d.project().next_return_name();
    let t = Track::new(TrackId(0), TrackKind::Return, &name, ColorIndex(9));
    d.execute(Command::AddTrack { index: None, track: t }, Origin::User).unwrap();
    d.project().tracks.last().unwrap().id
}

fn add_send(d: &mut Document, track: TrackId, target: TrackId) {
    let mut sends = d.project().track(track).unwrap().sends.clone();
    sends.push(TrackSend::new(target));
    let cmd = Command::SetSends { track, sends };
    d.execute(Command::Batch { label: "Add Send".into(), commands: vec![cmd] }, Origin::User).unwrap();
}

#[test]
fn returns_stay_after_ordinary_tracks() {
    let mut d = Document::new(Project::default());
    let a = add_return(&mut d);
    assert_eq!(d.undo_label(), Some("Add Return Track"));
    assert_eq!(d.project().track(a).unwrap().name, "Return A");
    assert_eq!(d.project().next_return_name(), "Return B");
    // A new ordinary track goes before the returns, and can't be placed among them.
    let audio = Track::new(TrackId(0), TrackKind::Audio, "Vox", ColorIndex(2));
    d.execute(Command::AddTrack { index: None, track: audio.clone() }, Origin::User).unwrap();
    assert_eq!(
        d.project().tracks.iter().map(|t| t.kind).collect::<Vec<_>>(),
        [TrackKind::Audio, TrackKind::Instrument, TrackKind::Audio, TrackKind::Return]
    );
    assert!(d.execute(Command::AddTrack { index: Some(4), track: audio }, Origin::User).is_err());
    let r = Track::new(TrackId(0), TrackKind::Return, "R", ColorIndex(9));
    assert!(d.execute(Command::AddTrack { index: Some(0), track: r }, Origin::User).is_err());
    assert_eq!(d.project().ordinary_track_count(), 3);
    assert_eq!(d.project().return_index(a), Some(0));
    // The cap.
    for _ in 1..MAX_RETURNS {
        add_return(&mut d);
    }
    let before = d.project().clone();
    let r = Track::new(TrackId(0), TrackKind::Return, "R", ColorIndex(9));
    assert!(d.execute(Command::AddTrack { index: None, track: r }, Origin::User).is_err());
    assert_eq!(d.project(), &before);
}

#[test]
fn return_tracks_refuse_clips_input_and_sends() {
    let mut d = Document::new(Project::default());
    let r = add_return(&mut d);
    d.execute(
        Command::AddAsset {
            asset: Asset {
                id: AssetId(0),
                relative_path: None,
                absolute_path: "/x.wav".into(),
                sample_rate: 48_000.0,
                channels: 2,
                frames: 48_000,
            },
        },
        Origin::User,
    )
    .unwrap();
    let asset = d.project().assets[0].id;
    let before = d.project().clone();
    let clip = Clip {
        id: ClipId(0),
        asset,
        name: "c".into(),
        start: 0,
        source_offset: 0,
        source_length: 1000,
        gain: 1.0,
        warp: Default::default(),
        disabled: false,
        clip_loop: None,
        fades: Default::default(),
    };
    let audio = d.project().tracks[0].id;
    for cmd in [
        Command::AddClip { track: r, clip },
        Command::SetTrackArm { track: r, arm: true },
        Command::SetTrackMonitor { track: r, monitor: true },
        Command::SetTrackInput { track: r, input: Some(InputSource { channel: 0, stereo: false }) },
        Command::AddMidiClip { track: r, clip: MidiClip::empty(0, PPQ) },
        // Returns have no sends and always go to the master.
        Command::SetSends { track: r, sends: vec![TrackSend::new(r)] },
        Command::SetTrackOutput { track: r, output: Some(r) },
        // Sends and outputs only into return tracks, one send per return, levels in range.
        Command::SetSends { track: audio, sends: vec![TrackSend::new(audio)] },
        Command::SetSends { track: audio, sends: vec![TrackSend::new(r), TrackSend::new(r)] },
        Command::SetSends { track: audio, sends: vec![TrackSend { level: 3.0, ..TrackSend::new(r) }] },
        Command::SetSends { track: audio, sends: vec![TrackSend { level: f32::NAN, ..TrackSend::new(r) }] },
        Command::SetTrackOutput { track: audio, output: Some(d.project().tracks[1].id) },
    ] {
        assert!(d.execute(cmd.clone(), Origin::User).is_err(), "{cmd:?} must be refused");
        assert_eq!(d.project(), &before);
    }
}

#[test]
fn sends_and_output_undo_and_coalesce() {
    let mut d = Document::new(Project::default());
    let r = add_return(&mut d);
    let audio = d.project().tracks[0].id;
    add_send(&mut d, audio, r);
    assert_eq!(d.undo_label(), Some("Add Send"));
    assert_eq!(d.project().track(audio).unwrap().sends, vec![TrackSend { target: r, level: 1.0, pre_fader: false }]);
    // A level drag is one undo step.
    let t0 = Instant::now();
    for i in 1..=10 {
        let sends = vec![TrackSend { level: 1.0 - i as f32 * 0.05, ..TrackSend::new(r) }];
        d.execute_at(Command::SetSends { track: audio, sends }, t0 + Duration::from_millis(i * 16)).unwrap();
    }
    assert_eq!(d.undo_label(), Some("Send Level Change"));
    d.undo().unwrap();
    assert_eq!(d.project().track(audio).unwrap().sends[0].level, 1.0);
    d.execute(Command::SetTrackOutput { track: audio, output: Some(r) }, Origin::User).unwrap();
    assert_eq!(d.undo_label(), Some("Output Change"));
    d.undo().unwrap();
    assert_eq!(d.project().track(audio).unwrap().output, None);
    d.redo().unwrap();
    assert_eq!(d.project().track(audio).unwrap().output, Some(r));
}

#[test]
fn removing_a_return_takes_its_sends_and_undo_restores_them() {
    let mut d = Document::new(Project::default());
    let a = add_return(&mut d);
    let b = add_return(&mut d);
    let (t0, t1) = (d.project().tracks[0].id, d.project().tracks[1].id);
    add_send(&mut d, t0, b);
    add_send(&mut d, t0, a);
    add_send(&mut d, t1, a);
    d.execute(Command::SetTrackOutput { track: t1, output: Some(a) }, Origin::User).unwrap();
    let before = d.project().clone();
    d.execute(Command::RemoveTrack { track: a }, Origin::User).unwrap();
    assert_eq!(d.undo_label(), Some("Delete Track"));
    let p = d.project();
    assert!(p.track(a).is_none());
    assert_eq!(p.track(t0).unwrap().sends, vec![TrackSend::new(b)], "the send to B stays");
    assert!(p.track(t1).unwrap().sends.is_empty());
    assert_eq!(p.track(t1).unwrap().output, None);
    d.undo().unwrap();
    assert_eq!(d.project(), &before, "undo restores the return, the sends in order and the output");
    d.redo().unwrap();
    assert!(d.project().track(a).is_none());
}

#[test]
fn routing_round_trips_and_old_projects_load() {
    let mut d = Document::new(Project::default());
    let r = add_return(&mut d);
    let audio = d.project().tracks[0].id;
    add_send(&mut d, audio, r);
    d.execute(Command::SetSends { track: audio, sends: vec![TrackSend { level: 0.5, pre_fader: true, target: r }] }, Origin::User).unwrap();
    d.execute(Command::SetTrackOutput { track: audio, output: Some(r) }, Origin::User).unwrap();
    let dir = std::env::temp_dir().join(format!("dstudio-returns-{}", std::process::id()));
    persist::save(d.project(), &dir).unwrap();
    let loaded = persist::load(&dir).unwrap().project;
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(&loaded, d.project());
    // Tracks saved before routing existed have no "sends" or "output" keys.
    let json = serde_json::to_value(&Project::default().tracks[0]).unwrap();
    assert!(json.get("sends").is_none() && json.get("output").is_none());
    let t: Track = serde_json::from_value(json).unwrap();
    assert!(t.sends.is_empty() && t.output.is_none());
}

#[test]
fn damaged_routing_is_repaired_on_load() {
    let mut p = Project::default();
    let rid = TrackId(p.alloc_id());
    let mut r = Track::new(rid, TrackKind::Return, "Return A", ColorIndex(9));
    r.sends.push(TrackSend::new(rid));
    p.tracks.insert(0, r); // a return before the ordinary tracks
    p.tracks[1].sends = vec![TrackSend { level: 9.0, ..TrackSend::new(rid) }, TrackSend::new(rid), TrackSend::new(TrackId(999))];
    p.tracks[2].output = Some(p.tracks[1].id);
    let dir = tempfile::tempdir().unwrap();
    persist::save(&p, dir.path()).unwrap();
    let p = persist::load(dir.path()).unwrap().project;
    assert_eq!(p.tracks.iter().map(|t| t.kind).collect::<Vec<_>>(), [TrackKind::Audio, TrackKind::Instrument, TrackKind::Return]);
    assert_eq!(p.tracks[0].sends, vec![TrackSend { level: TrackSend::MAX_LEVEL, ..TrackSend::new(rid) }]);
    assert_eq!(p.tracks[1].output, None);
    assert!(p.tracks[2].sends.is_empty());
}

#[test]
fn snapshot_carries_routing_and_track_renders_are_dry() {
    let mut d = Document::new(Project::default());
    let r = add_return(&mut d);
    let audio = d.project().tracks[0].id;
    d.execute(Command::SetSends { track: audio, sends: vec![TrackSend { level: 0.5, pre_fader: true, target: r }] }, Origin::User).unwrap();
    d.execute(Command::SetTrackOutput { track: audio, output: Some(r) }, Origin::User).unwrap();
    let (s, _) = snapshot::build(d.project(), 48_000.0, &Default::default());
    let t = &s.tracks[0];
    assert_eq!((t.is_return, t.output), (false, Some(r.0)));
    assert_eq!(t.sends, vec![snapshot::SendSnapshot { target: r.0, gain: 0.5, pre_fader: true }]);
    assert!(s.tracks[2].is_return);
    let span = render_plan::ClipSpan { track: audio, kind: TrackKind::Audio, clips: Vec::new(), start: 0, end: PPQ };
    let rp = render_plan::render_project(d.project(), &span, render_plan::RenderMode::Stem);
    assert!(rp.track(audio).unwrap().sends.is_empty() && rp.track(audio).unwrap().output.is_none());
}

#[test]
fn a_saved_return_holding_clips_loads_as_an_ordinary_track_and_can_be_removed_and_restored() {
    let mut p = Project::default();
    let rid = TrackId(p.alloc_id());
    let mut r = Track::new(rid, TrackKind::Return, "Return A", ColorIndex(9));
    r.midi_clips.push(MidiClip::empty(0, PPQ));
    r.arm = true;
    p.tracks.push(r);
    let empty = TrackId(p.alloc_id());
    let mut e = Track::new(empty, TrackKind::Return, "Return B", ColorIndex(9));
    e.arm = true;
    e.monitor = true;
    e.input = Some(InputSource { channel: 0, stereo: false });
    p.tracks.push(e);
    p.tracks[0].sends = vec![TrackSend::new(rid), TrackSend::new(empty)];
    let dir = tempfile::tempdir().unwrap();
    persist::save(&p, dir.path()).unwrap();
    let p = persist::load(dir.path()).unwrap().project;
    // The return with MIDI keeps its clip as an instrument track; sends into it are gone.
    let t = &p.tracks[p.track_index(rid).unwrap()];
    assert_eq!(t.kind, TrackKind::Instrument);
    assert_eq!(t.midi_clips.len(), 1);
    assert_eq!(p.tracks[0].sends, vec![TrackSend::new(empty)]);
    // The empty return stays a return without input, arm or monitor, after the ordinary tracks.
    let last = p.tracks.last().unwrap();
    assert!(last.id == empty && last.is_return() && !last.arm && !last.monitor && last.input.is_none());
    // Every track can be removed and the undo puts it back exactly.
    let mut d = Document::new(p);
    for id in [rid, empty] {
        let before = d.project().clone();
        d.execute(Command::RemoveTrack { track: id }, Origin::User).unwrap();
        d.undo().unwrap();
        assert_eq!(d.project(), &before);
    }
}

#[test]
fn returns_beyond_the_cap_load_as_audio_tracks() {
    let mut p = Project::default();
    let ids: Vec<TrackId> = (0..MAX_RETURNS + 2)
        .map(|i| {
            let id = TrackId(p.alloc_id());
            p.tracks.push(Track::new(id, TrackKind::Return, &format!("R{i}"), ColorIndex(9)));
            id
        })
        .collect();
    p.sanitize_routing();
    assert_eq!(p.returns().count(), MAX_RETURNS);
    assert_eq!(p.returns().map(|t| t.id).collect::<Vec<_>>(), ids[..MAX_RETURNS]);
    for id in &ids[MAX_RETURNS..] {
        let t = &p.tracks[p.track_index(*id).unwrap()];
        assert_eq!(t.kind, TrackKind::Audio);
    }
    assert!(p.tracks.iter().skip_while(|t| !t.is_return()).all(|t| t.is_return()));
}
