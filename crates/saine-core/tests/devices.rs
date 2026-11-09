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


// ---------------------------------------------------------------- built-in devices

fn comp() -> &'static builtin::DeviceDef {
    builtin::find("compressor").unwrap()
}

fn add_comp(d: &mut Document, t: TrackId) -> DeviceId {
    d.execute(Command::AddDevice { track: t, index: None, device: comp().instantiate() }, Origin::User).unwrap();
    d.project().track(t).unwrap().devices.last().unwrap().id
}

#[test]
fn builtin_params_are_validated_undoable_and_coalesced() {
    let mut d = Document::new(Project::default());
    let a = d.project().tracks[0].id;
    let id = add_comp(&mut d, a);
    assert_eq!(d.undo_label(), Some("Add Compressor"));
    assert_eq!(d.project().device(id).unwrap().params.len(), comp().params.len(), "stored state is complete");

    let t0 = std::time::Instant::now();
    for (i, v) in [-25.0, -27.5, -30.0].into_iter().enumerate() {
        d.execute_at(Command::SetDeviceParam { device: id, param: "threshold".into(), value: v }, t0 + std::time::Duration::from_millis(100 * i as u64)).unwrap();
    }
    assert_eq!(d.project().device(id).unwrap().params["threshold"], -30.0);
    assert_eq!(d.undo_label(), Some("Device Parameter Change"));
    d.undo().unwrap();
    assert_eq!(d.project().device(id).unwrap().params["threshold"], -24.0, "a drag is one undo step");
    d.redo().unwrap();

    for bad in [
        Command::SetDeviceParam { device: id, param: "threshold".into(), value: 6.0 },
        Command::SetDeviceParam { device: id, param: "threshold".into(), value: f32::NAN },
        Command::SetDeviceParam { device: id, param: "nope".into(), value: 0.0 },
        Command::SetDeviceParam { device: DeviceId(9999), param: "threshold".into(), value: -1.0 },
    ] {
        let before = d.project().clone();
        assert!(d.execute(bad, Origin::User).is_err());
        assert_eq!(d.project(), &before);
    }
    // Discrete values snap to an index.
    d.execute(Command::SetDeviceParam { device: id, param: "style".into(), value: 1.8 }, Origin::User).unwrap();
    assert_eq!(d.project().device(id).unwrap().params["style"], 2.0);
}

#[test]
fn presets_load_in_one_step_and_name_the_card() {
    let mut d = Document::new(Project::default());
    let a = d.project().tracks[0].id;
    let id = add_comp(&mut d, a);
    let preset = builtin::DevicePreset::factory(comp(), &comp().factory[0]);
    d.execute(Command::SetDeviceParams { device: id, params: preset.load_values(comp()), preset: Some(preset.name.clone()) }, Origin::User).unwrap();
    let dev = d.project().device(id).unwrap().clone();
    assert_eq!(dev.preset.as_deref(), Some(comp().factory[0].name));
    assert_eq!(dev.params, preset.values);
    assert_eq!(d.undo_label(), Some(format!("Load Preset {}", preset.name).as_str()));
    d.undo().unwrap();
    let back = d.project().device(id).unwrap();
    assert_eq!((back.preset.clone(), back.params.clone()), (None, comp().defaults()));
    // A preset with one bad value changes nothing.
    let mut bad = comp().defaults();
    bad.insert("ratio".into(), 100.0);
    assert!(d.execute(Command::SetDeviceParams { device: id, params: bad, preset: None }, Origin::User).is_err());
    assert_eq!(d.project().device(id).unwrap().params, comp().defaults());
}

#[test]
fn builtin_chain_ops_bypass_sidechain_persist_and_snapshot() {
    let mut d = Document::new(Project::default());
    let (a, i) = (d.project().tracks[0].id, d.project().tracks[1].id);
    let c1 = add_comp(&mut d, a);
    let c2 = add_comp(&mut d, a);
    d.execute(Command::MoveDevice { device: c2, track: a, index: 0 }, Origin::User).unwrap();
    d.execute(Command::SetDeviceEnabled { device: c1, enabled: false }, Origin::User).unwrap();
    d.execute(Command::SetDeviceParam { device: c1, param: "ratio".into(), value: 8.0 }, Origin::User).unwrap();
    assert!(d.execute(Command::SetDeviceSidechain { device: c1, source: Some(a) }, Origin::User).is_err(), "own track");
    d.execute(Command::SetDeviceSidechain { device: c1, source: Some(i) }, Origin::User).unwrap();
    assert_eq!(d.project().device(c1).unwrap().sidechain, Some(i));

    // Unknown definitions and sanitising on the way in.
    let mut odd = comp().instantiate();
    odd.params.insert("threshold".into(), -400.0);
    odd.params.insert("ghost".into(), 1.0);
    odd.params.remove("mix");
    d.execute(Command::AddDevice { track: a, index: None, device: odd }, Origin::User).unwrap();
    let c3 = d.project().track(a).unwrap().devices[2].clone();
    assert_eq!((c3.params["threshold"], c3.params["mix"], c3.params.contains_key("ghost")), (-60.0, 100.0, false));
    let mut unknown = comp().instantiate();
    unknown.plugin.id = "no-such-device".into();
    assert!(d.execute(Command::AddDevice { track: a, index: None, device: unknown }, Origin::User).is_err());

    let (snap, _) = snapshot::build(d.project(), 48_000.0, &HashMap::new());
    let ids: Vec<_> = snap.tracks[0].devices.iter().map(|x| x.id).collect();
    assert_eq!(ids, [c2, c1, c3.id]);
    let v = comp().values(&snap.tracks[0].devices[1]);
    assert_eq!(v[comp().index_of("ratio").unwrap()], 8.0);
    assert!(!snap.tracks[0].devices[1].enabled);

    let dir = tempfile::tempdir().unwrap();
    persist::save(d.project(), dir.path()).unwrap();
    let l = persist::load(dir.path()).unwrap();
    assert_eq!(l.project.track(a).unwrap().devices, d.project().track(a).unwrap().devices);
}

#[test]
fn builtin_automation_targets_use_plain_values() {
    let mut d = Document::new(Project::default());
    let (a, i) = (d.project().tracks[0].id, d.project().tracks[1].id);
    let id = add_comp(&mut d, a);
    let target = AutomationTarget::Builtin { device: id, device_type: "compressor".into(), param: "attack".into() };
    assert_eq!(target.range(), (1.0, 1000.0));
    assert!(target.logarithmic());
    assert_eq!(target.labels(), ("Compressor".to_string(), "Attack".to_string()));
    let mut lane = AutomationLane::new(target.clone());
    lane.points = vec![AutoPoint { time: 0, value: 1.0, curve: 0.0 }, AutoPoint { time: 960, value: 100.0, curve: 0.0 }];
    assert!((lane.value_at(480).unwrap() - 10.0).abs() < 1e-3, "geometric interpolation");
    d.execute(Command::SetAutomationLane { track: Some(a), target: target.clone(), lane: Some(lane.clone()) }, Origin::User).unwrap();
    assert!(d.execute(Command::SetAutomationLane { track: Some(i), target: target.clone(), lane: Some(lane.clone()) }, Origin::User).is_err(), "other track");
    let hidden = AutomationTarget::Builtin { device: id, device_type: "compressor".into(), param: "sidechainEnable".into() };
    assert!(d.execute(Command::SetAutomationLane { track: Some(a), target: hidden.clone(), lane: Some(AutomationLane::new(hidden)) }, Origin::User).is_err(), "not automatable");
    let mut wrong = lane;
    wrong.points[0].value = 5000.0;
    assert!(d.execute(Command::SetAutomationLane { track: Some(a), target, lane: Some(wrong) }, Origin::User).is_err(), "out of range");

    let targets = builtin::mod_targets(d.project().track(a).unwrap());
    assert_eq!(targets.len(), comp().params.iter().filter(|p| p.automatable).count());
    assert!(targets.iter().all(|t| t.device == id && t.param != "sidechainEnable"));
}

#[test]
fn builtin_commands_round_trip_as_json() {
    let cmd = Command::SetDeviceParam { device: DeviceId(7), param: "ratio".into(), value: 3.0 };
    let json = serde_json::to_string(&cmd).unwrap();
    assert_eq!(json, r#"{"op":"set_device_param","device":7,"param":"ratio","value":3.0}"#);
    assert_eq!(serde_json::from_str::<Command>(&json).unwrap(), cmd);
}
