use saine_core::*;
use std::collections::HashMap;

fn plug(name: &str, kind: PluginKind) -> PluginRef {
    PluginRef { format: PluginFormat::AudioUnit, id: format!("aufx {name} test"), path: None, name: name.into(), vendor: "Test".into(), version: "1".into(), kind }
}

#[test]
fn insert_chain_add_move_bypass_remove_with_undo() {
    let mut d = Document::new(Project::default());
    let (a, i) = (d.project().tracks[0].id, d.project().tracks[1].id);
    d.execute(Command::AddDevice { track: a, index: None, device: Device::new(plug("dely", PluginKind::Effect)) }, Origin::User).unwrap();
    d.execute(Command::AddDevice { track: a, index: Some(0), device: Device::new(plug("filt", PluginKind::Effect)) }, Origin::User).unwrap();
    let names = |d: &Document, t: TrackId| d.project().track(t).unwrap().devices.iter().map(|x| x.plugin.name.clone()).collect::<Vec<_>>();
    assert_eq!(names(&d, a), ["filt", "dely"]);
    assert_eq!(d.undo_label(), Some("Add filt"));
    let delay = d.project().track(a).unwrap().devices[1].id;
    d.execute(Command::MoveDevice { device: delay, track: i, index: 0 }, Origin::User).unwrap();
    assert_eq!((names(&d, a), names(&d, i)), (vec!["filt".to_string()], vec!["dely".to_string()]));
    d.execute(Command::SetDeviceEnabled { device: delay, enabled: false }, Origin::User).unwrap();
    assert!(!d.project().device(delay).unwrap().enabled);
    d.execute(Command::RemoveDevice { device: delay }, Origin::User).unwrap();
    assert!(d.project().device(delay).is_none());
    for _ in 0..3 {
        d.undo().unwrap();
    }
    assert_eq!(names(&d, a), ["filt", "dely"], "back on the original track and position");
    assert!(d.project().device(delay).unwrap().enabled);
    d.redo().unwrap();
    assert_eq!(d.project().find_device(delay).unwrap().0, 1, "redo moves it again with the same id");
}

#[test]
fn kinds_and_tracks_are_validated() {
    let mut d = Document::new(Project::default());
    let (a, i) = (d.project().tracks[0].id, d.project().tracks[1].id);
    assert!(d.execute(Command::AddDevice { track: a, index: None, device: Device::new(plug("samp", PluginKind::Instrument)) }, Origin::User).is_err());
    assert!(d.execute(Command::SetInstrumentPlugin { track: a, device: Some(Device::new(plug("samp", PluginKind::Instrument))) }, Origin::User).is_err());
    assert!(d.execute(Command::SetInstrumentPlugin { track: i, device: Some(Device::new(plug("dely", PluginKind::Effect))) }, Origin::User).is_err());
    d.execute(Command::SetInstrumentPlugin { track: i, device: Some(Device::new(plug("samp", PluginKind::Instrument))) }, Origin::User).unwrap();
    let inst = d.project().track(i).unwrap().instrument_plugin.clone().unwrap();
    assert_eq!(d.undo_label(), Some("Load samp"));
    // Removing the instrument plugin restores the built-in synth; undo brings it back.
    d.execute(Command::RemoveDevice { device: inst.id }, Origin::User).unwrap();
    assert!(d.project().track(i).unwrap().instrument_plugin.is_none());
    d.undo().unwrap();
    assert_eq!(d.project().track(i).unwrap().instrument_plugin.as_ref().unwrap().id, inst.id);
}

#[test]
fn states_sync_without_history_and_persist() {
    let mut d = Document::new(Project::default());
    let a = d.project().tracks[0].id;
    d.execute(Command::AddDevice { track: a, index: None, device: Device::new(plug("dely", PluginKind::Effect)) }, Origin::User).unwrap();
    let id = d.project().track(a).unwrap().devices[0].id;
    let (rev, n) = (d.revision(), d.history_labels().len());
    let blob = device::b64::encode(&[1, 2, 3, 250]);
    d.sync_device_states(&[(id, blob.clone())]);
    assert_eq!((d.revision(), d.history_labels().len()), (rev, n));
    assert_eq!(d.project().device(id).unwrap().state.as_deref(), Some(blob.as_str()));
    let (snap, _) = snapshot::build(d.project(), 48_000.0, &HashMap::new());
    assert_eq!(snap.tracks[0].devices[0].state.as_deref(), Some(blob.as_str()));
    assert!(snap.tracks[1].instrument_plugin.is_none());

    let dir = tempfile::tempdir().unwrap();
    persist::save(d.project(), dir.path()).unwrap();
    let l = persist::load(dir.path()).unwrap();
    assert_eq!(l.project.device(id).unwrap(), d.project().device(id).unwrap());
}
