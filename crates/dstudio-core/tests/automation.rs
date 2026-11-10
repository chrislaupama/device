use dstudio_core::*;
use std::collections::HashMap;

fn pts(v: &[(Ticks, f32)]) -> Vec<AutoPoint> {
    v.iter().map(|&(time, value)| AutoPoint { time, value, curve: 0.0 }).collect()
}

#[test]
fn lane_create_edit_delete_undo() {
    let mut d = Document::new(Project::default());
    let a = d.project().tracks[0].id;
    let lane = AutomationLane { target: AutomationTarget::Volume, enabled: true, points: pts(&[(0, -12.0), (4 * PPQ, 0.0)]) };
    d.execute(Command::SetAutomationLane { track: Some(a), target: AutomationTarget::Volume, lane: Some(lane.clone()) }, Origin::User).unwrap();
    assert_eq!(d.undo_label(), Some("Edit Volume Automation"));
    let mut edited = lane.clone();
    edited.points.push(AutoPoint { time: 8 * PPQ, value: -6.0, curve: 0.5 });
    d.execute(Command::SetAutomationLane { track: Some(a), target: AutomationTarget::Volume, lane: Some(edited.clone()) }, Origin::User).unwrap();
    assert_eq!(d.project().track(a).unwrap().automation, vec![edited]);
    d.execute(Command::SetAutomationLane { track: Some(a), target: AutomationTarget::Volume, lane: None }, Origin::User).unwrap();
    assert!(d.project().track(a).unwrap().automation.is_empty());
    d.undo().unwrap();
    d.undo().unwrap();
    assert_eq!(d.project().track(a).unwrap().automation, vec![lane]);
    d.undo().unwrap();
    assert!(d.project().track(a).unwrap().automation.is_empty());
}

#[test]
fn targets_are_validated() {
    let mut d = Document::new(Project::default());
    let (a, i) = (d.project().tracks[0].id, d.project().tracks[1].id);
    let synth = AutomationTarget::Synth { param: SynthParam::FilterCutoff };
    let cut = AutomationLane { target: synth.clone(), enabled: true, points: pts(&[(0, 500.0)]) };
    assert!(d.execute(Command::SetAutomationLane { track: Some(a), target: synth.clone(), lane: Some(cut.clone()) }, Origin::User).is_err());
    d.execute(Command::SetAutomationLane { track: Some(i), target: synth.clone(), lane: Some(cut) }, Origin::User).unwrap();
    let plug = AutomationTarget::Plugin { device: DeviceId(999), param: 0, name: "Gain".into() };
    assert!(d.execute(Command::SetAutomationLane { track: Some(a), target: plug.clone(), lane: Some(AutomationLane::new(plug)) }, Origin::User).is_err(), "device not on track");
    assert!(d.execute(Command::SetAutomationLane { track: None, target: AutomationTarget::Pan, lane: Some(AutomationLane::new(AutomationTarget::Pan)) }, Origin::User).is_err(), "master is volume-only");
    let bad = AutomationLane { target: AutomationTarget::Volume, enabled: true, points: pts(&[(0, 20.0)]) };
    assert!(d.execute(Command::SetAutomationLane { track: Some(a), target: AutomationTarget::Volume, lane: Some(bad) }, Origin::User).is_err());
    let mismatched = AutomationLane::new(AutomationTarget::Pan);
    assert!(d.execute(Command::SetAutomationLane { track: Some(a), target: AutomationTarget::Volume, lane: Some(mismatched) }, Origin::User).is_err());
}

#[test]
fn snapshot_has_enabled_nonempty_lanes_in_frames_and_master() {
    let mut d = Document::new(Project::default());
    let a = d.project().tracks[0].id;
    d.execute(Command::SetAutomationLane { track: Some(a), target: AutomationTarget::Pan, lane: Some(AutomationLane { target: AutomationTarget::Pan, enabled: true, points: pts(&[(PPQ, -1.0), (2 * PPQ, 1.0)]) }) }, Origin::User).unwrap();
    d.execute(Command::SetAutomationLane { track: Some(a), target: AutomationTarget::Volume, lane: Some(AutomationLane { target: AutomationTarget::Volume, enabled: false, points: pts(&[(0, -6.0)]) }) }, Origin::User).unwrap();
    d.execute(Command::SetAutomationLane { track: None, target: AutomationTarget::Volume, lane: Some(AutomationLane { target: AutomationTarget::Volume, enabled: true, points: pts(&[(0, -3.0)]) }) }, Origin::User).unwrap();
    let (snap, _) = snapshot::build(d.project(), 48_000.0, &HashMap::new());
    let t = &snap.tracks[0];
    assert_eq!(t.automation.len(), 1, "disabled lane is not sent");
    assert_eq!(t.automation[0].points, vec![(24_000, -1.0, 0.0), (48_000, 1.0, 0.0)]);
    assert_eq!(snap.master_automation[0].points, vec![(0, -3.0, 0.0)]);
    // persists
    let dir = tempfile::tempdir().unwrap();
    persist::save(d.project(), dir.path()).unwrap();
    assert_eq!(persist::load(dir.path()).unwrap().project, *d.project());
}
