//! Track menu commands: moving, recolouring and duplicating tracks.

use dstudio_core::builtin;
use dstudio_core::modulation::LFO_ID;
use dstudio_core::*;

fn kinds(d: &Document) -> Vec<TrackKind> {
    d.project().tracks.iter().map(|t| t.kind).collect()
}

#[test]
fn tracks_move_within_their_group_and_undo_puts_them_back() {
    let mut d = Document::new(Project::default());
    let (a, b) = (d.project().tracks[0].id, d.project().tracks[1].id);
    let r = Track::new(TrackId(0), TrackKind::Return, "Return A", ColorIndex(9));
    d.execute(Command::AddTrack { index: None, track: r }, Origin::User).unwrap();
    d.execute(Command::MoveTrack { track: a, index: 1 }, Origin::User).unwrap();
    assert_eq!(d.undo_label(), Some("Move Track"));
    assert_eq!(d.project().track_index(a), Some(1));
    assert_eq!(d.project().track_index(b), Some(0));
    // An ordinary track never moves among the returns, nor a return among the ordinary tracks.
    assert!(d.execute(Command::MoveTrack { track: a, index: 2 }, Origin::User).is_err());
    let ret = d.project().tracks[2].id;
    assert!(d.execute(Command::MoveTrack { track: ret, index: 0 }, Origin::User).is_err());
    assert_eq!(kinds(&d), [TrackKind::Instrument, TrackKind::Audio, TrackKind::Return]);
    d.undo().unwrap();
    assert_eq!(d.project().track_index(a), Some(0));
}

#[test]
fn colour_changes_are_one_undo_step() {
    let mut d = Document::new(Project::default());
    let a = d.project().tracks[0].id;
    let old = d.project().tracks[0].color;
    d.execute(Command::SetTrackColor { track: a, color: ColorIndex(7) }, Origin::User).unwrap();
    assert_eq!(d.project().tracks[0].color, ColorIndex(7));
    assert_eq!(d.undo_label(), Some("Track Color Change"));
    d.undo().unwrap();
    assert_eq!(d.project().tracks[0].color, old);
}

#[test]
fn a_duplicated_track_gets_fresh_ids_and_its_links_follow_its_own_devices() {
    let mut d = Document::new(Project::default());
    let track = d.project().tracks[1].id;
    d.execute(Command::AddMidiClip { track, clip: MidiClip::empty(0, PPQ * 4) }, Origin::User).unwrap();
    for dev in [builtin::find(LFO_ID).unwrap().instantiate(), builtin::find("compressor").unwrap().instantiate()] {
        d.execute(Command::AddDevice { track, index: None, device: dev }, Origin::User).unwrap();
    }
    let t = d.project().track(track).unwrap();
    let n = t.devices.len();
    let (lfo, comp) = (t.devices[n - 2].id, t.devices[n - 1].id);
    let link = ModLink { slot: 0, target: comp, param: ModParam::Builtin("threshold".into()), min: -0.2, max: 0.2 };
    d.execute(Command::AddModLink { source: lfo, index: None, link }, Origin::User).unwrap();
    let lane = AutomationLane {
        target: AutomationTarget::Builtin { device: comp, device_type: "compressor".into(), param: "ratio".into() },
        enabled: true,
        points: vec![],
    };
    d.execute(Command::SetAutomationLane { track: Some(track), target: lane.target.clone(), lane: Some(lane) }, Origin::User).unwrap();

    let copy = d.project().track_copy(track, true).unwrap();
    let src = d.project().track(track).unwrap();
    assert_ne!(copy.id, src.id);
    assert_eq!(copy.name, src.name);
    assert_eq!(copy.color, src.color);
    assert_eq!(copy.midi_clips.len(), 1);
    assert_ne!(copy.midi_clips[0].id, src.midi_clips[0].id);
    let (new_lfo, new_comp) = (copy.devices[n - 2].id, copy.devices[n - 1].id);
    assert!(new_lfo != lfo && new_comp != comp);
    assert_eq!(copy.devices[n - 2].links[0].target, new_comp);
    assert!(matches!(&copy.automation[0].target, AutomationTarget::Builtin { device, .. } if *device == new_comp));
    // The copy goes in right after the original, and is one undo step.
    let at = d.project().track_index(track).unwrap() + 1;
    d.execute(Command::AddTrack { index: Some(at), track: copy.clone() }, Origin::User).unwrap();
    assert_eq!(d.project().tracks[at].id, copy.id);
    assert_eq!(d.project().tracks.len(), 3);
    d.undo().unwrap();
    assert_eq!(d.project().tracks.len(), 2);

    // Without clips: devices and settings only.
    let bare = d.project().track_copy(track, false).unwrap();
    assert!(bare.midi_clips.is_empty() && bare.clips.is_empty());
    assert_eq!(bare.devices.len(), n);
}
