use dstudio_core::*;
use std::collections::HashMap;

fn plug(name: &str, kind: PluginKind) -> PluginRef {
    PluginRef {
        format: PluginFormat::AudioUnit,
        id: format!("aufx {name} test"),
        path: None,
        name: name.into(),
        vendor: "Test".into(),
        version: "1".into(),
        kind,
    }
}

#[test]
fn insert_chain_add_move_bypass_remove_with_undo() {
    let mut d = Document::new(Project::default());
    let (a, i) = (d.project().tracks[0].id, d.project().tracks[1].id);
    d.execute(Command::AddDevice { track: a, index: None, device: Device::new(plug("dely", PluginKind::Effect)) }, Origin::User).unwrap();
    d.execute(Command::AddDevice { track: a, index: Some(0), device: Device::new(plug("filt", PluginKind::Effect)) }, Origin::User)
        .unwrap();
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
    assert!(
        d.execute(Command::AddDevice { track: a, index: None, device: Device::new(plug("samp", PluginKind::Instrument)) }, Origin::User)
            .is_err()
    );
    assert!(
        d.execute(Command::SetInstrumentPlugin { track: a, device: Some(Device::new(plug("samp", PluginKind::Instrument))) }, Origin::User)
            .is_err()
    );
    assert!(
        d.execute(Command::SetInstrumentPlugin { track: i, device: Some(Device::new(plug("dely", PluginKind::Effect))) }, Origin::User)
            .is_err()
    );
    d.execute(Command::SetInstrumentPlugin { track: i, device: Some(Device::new(plug("samp", PluginKind::Instrument))) }, Origin::User)
        .unwrap();
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
    assert_eq!(
        snap.tracks[1].instrument_plugin.as_ref().map(|d| d.plugin.id.as_str()),
        Some("synth-v2"),
        "instrument tracks play the built-in Synth"
    );

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
        d.execute_at(
            Command::SetDeviceParam { device: id, param: "threshold".into(), value: v },
            t0 + std::time::Duration::from_millis(100 * i as u64),
        )
        .unwrap();
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
    d.execute(Command::SetDeviceParams { device: id, params: preset.load_values(comp()), preset: Some(preset.name.clone()) }, Origin::User)
        .unwrap();
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
    assert!(
        d.execute(Command::SetAutomationLane { track: Some(i), target: target.clone(), lane: Some(lane.clone()) }, Origin::User).is_err(),
        "other track"
    );
    let hidden = AutomationTarget::Builtin { device: id, device_type: "compressor".into(), param: "sidechainEnable".into() };
    assert!(
        d.execute(
            Command::SetAutomationLane { track: Some(a), target: hidden.clone(), lane: Some(AutomationLane::new(hidden)) },
            Origin::User
        )
        .is_err(),
        "not automatable"
    );
    let mut wrong = lane;
    wrong.points[0].value = 5000.0;
    assert!(d.execute(Command::SetAutomationLane { track: Some(a), target, lane: Some(wrong) }, Origin::User).is_err(), "out of range");

    let targets = builtin::mod_targets(d.project().track(a).unwrap());
    assert_eq!(targets.len(), comp().params.iter().filter(|p| p.automatable).count());
    assert!(targets.iter().all(|t| t.device == id && t.param != "sidechainEnable"));
}

#[test]
fn switch_lanes_validate_their_device_and_reach_the_snapshot() {
    let mut d = Document::new(Project::default());
    let (a, i) = (d.project().tracks[0].id, d.project().tracks[1].id);
    let id = add_comp(&mut d, a);
    let on = AutomationTarget::DeviceOn { device: id, name: "Compressor".into() };
    let mut lane = AutomationLane::new(on.clone());
    lane.points = vec![AutoPoint { time: 0, value: 1.0, curve: 0.0 }, AutoPoint { time: 960, value: 0.0, curve: 0.0 }];
    assert!(
        d.execute(Command::SetAutomationLane { track: Some(i), target: on.clone(), lane: Some(lane.clone()) }, Origin::User).is_err(),
        "the device is on another track"
    );
    d.execute(Command::SetAutomationLane { track: Some(a), target: on.clone(), lane: Some(lane.clone()) }, Origin::User).unwrap();
    let mut mute = AutomationLane::new(AutomationTarget::Mute);
    mute.points = vec![AutoPoint { time: 480, value: 1.0, curve: 0.0 }];
    d.execute(Command::SetAutomationLane { track: Some(a), target: AutomationTarget::Mute, lane: Some(mute) }, Origin::User).unwrap();
    assert!(
        d.execute(
            Command::SetAutomationLane {
                track: None,
                target: AutomationTarget::Mute,
                lane: Some(AutomationLane::new(AutomationTarget::Mute))
            },
            Origin::User
        )
        .is_err(),
        "the master only has volume automation"
    );
    let (snap, _) = snapshot::build(d.project(), 48_000.0, &HashMap::new());
    let targets: Vec<_> = snap.tracks[0].automation.iter().map(|l| l.target.clone()).collect();
    assert_eq!(targets, vec![on, AutomationTarget::Mute]);
    // Saved and loaded unchanged.
    let dir = tempfile::tempdir().unwrap();
    persist::save(d.project(), dir.path()).unwrap();
    let l = persist::load(dir.path()).unwrap();
    assert_eq!(l.project.track(a).unwrap().automation, d.project().track(a).unwrap().automation);
}

#[test]
fn builtin_commands_round_trip_as_json() {
    let cmd = Command::SetDeviceParam { device: DeviceId(7), param: "ratio".into(), value: 3.0 };
    let json = serde_json::to_string(&cmd).unwrap();
    assert_eq!(json, r#"{"op":"set_device_param","device":7,"param":"ratio","value":3.0}"#);
    assert_eq!(serde_json::from_str::<Command>(&json).unwrap(), cmd);
}

#[test]
fn pitch_corrector_follows_the_project_key_undoably_and_persists_it() {
    let pc = builtin::find("pitch-corrector").unwrap();
    let mut d = Document::new(Project::default());
    let a = d.project().tracks[0].id;
    d.execute(Command::AddDevice { track: a, index: None, device: pc.instantiate() }, Origin::User).unwrap();
    let dev = d.project().track(a).unwrap().devices[0].id;
    d.execute(Command::SetDeviceParam { device: dev, param: "key".into(), value: 4.0 }, Origin::User).unwrap();
    let values = |d: &Document| {
        let (snap, _) = snapshot::build(d.project(), 48_000.0, &HashMap::new());
        let v = pc.values(&snap.tracks[0].devices[0]);
        (v[pc.index_of("key").unwrap()], v[pc.index_of("scale").unwrap()])
    };
    assert_eq!(values(&d), (4.0, 0.0), "no project key: the stored key and scale run");

    assert!(d.execute(Command::SetProjectKey { key: Some(ProjectKey { root: 12, minor: false }) }, Origin::User).is_err());
    d.execute(Command::SetProjectKey { key: Some(ProjectKey { root: 9, minor: true }) }, Origin::User).unwrap();
    assert_eq!(d.undo_label(), Some("Key Change"));
    assert_eq!(values(&d), (9.0, 2.0), "Follow = Project (default) runs A minor");
    assert_eq!(d.project().device(dev).unwrap().params["key"], 4.0, "stored key is kept");

    d.execute(Command::SetDeviceParam { device: dev, param: "key_follow".into(), value: 0.0 }, Origin::User).unwrap();
    assert_eq!(values(&d), (4.0, 0.0), "Manual ignores the project key");
    d.undo().unwrap();
    assert_eq!(values(&d), (9.0, 2.0));

    let dir = tempfile::tempdir().unwrap();
    persist::save(d.project(), dir.path()).unwrap();
    assert_eq!(persist::load(dir.path()).unwrap().project.transport.key, Some(ProjectKey { root: 9, minor: true }));
    d.undo().unwrap();
    assert_eq!(d.project().transport.key, None);
    assert_eq!(values(&d), (4.0, 0.0));
}

#[test]
fn analyzer_formats_and_tuner_strings() {
    let sp = builtin::find("spectrum").unwrap();
    let slope = sp.param("slope").unwrap();
    assert_eq!(slope.format(4.5), "+4.5 dB/oct");
    assert_eq!(slope.format(-3.0), "\u{2212}3.0 dB/oct");
    assert_eq!(slope.parse("-1.5"), Some(-1.5));
    let tu = builtin::find("tuner").unwrap();
    assert_eq!(tu.param("tuning").unwrap().choices.len(), builtin::TUNING_STRINGS.len());
}

// ---------------------------------------------------------------- time and pitch built-ins

fn time_fx(id: &str) -> &'static builtin::DeviceDef {
    builtin::find(id).unwrap_or_else(|| panic!("{id} is registered"))
}

#[test]
fn time_fx_definitions_are_complete_and_presets_valid() {
    for id in ["delay", "echo", "reverb", "pitch-shifter", "grains"] {
        let def = time_fx(id);
        let d = def.instantiate();
        assert_eq!(d.params.len(), def.params.len(), "{id}: stored state is complete");
        for p in def.params {
            assert!(p.default >= p.min && p.default <= p.max, "{id}.{}: default in range", p.id);
            assert!(!p.format(p.default).is_empty());
        }
        for c in def.layout.controls() {
            if let Some(p) = c.param() {
                assert!(def.param(p).is_some(), "{id}: layout names unknown parameter {p}");
            }
        }
        assert!(!def.factory.is_empty(), "{id}: has factory presets");
        for f in def.factory {
            for (p, v) in f.values {
                builtin::validate_value(def, p, *v).unwrap_or_else(|e| panic!("{id} preset {}: {e}", f.name));
            }
        }
    }
}

#[test]
fn custom_formats_show_and_read_values() {
    let delay = time_fx("delay");
    let p = |def: &builtin::DeviceDef, id: &str| def.param(id).unwrap();
    // Typed text is read in the device's state (Behavior::read), as the panel does.
    let read = |def: &builtin::DeviceDef, id: &str, t: &str| def.parse_of(&def.instantiate(), def.param(id).unwrap(), t);
    assert_eq!(p(delay, "timeMs").format(1500.0), "1.50 s");
    assert_eq!(p(delay, "timeMs").format(4.5), "4.5 ms");
    assert_eq!(read(delay, "timeMs", "1.2 s"), Some(1200.0));
    assert_eq!(p(delay, "stereoWidth").format(140.0), "Ping 40%");
    assert_eq!(p(delay, "stereoWidth").format(0.0), "Mono");
    assert_eq!(read(delay, "stereoWidth", "ping 40"), Some(140.0));
    let reverb = time_fx("reverb");
    assert_eq!(p(reverb, "highcut2").format(12000.0), "12.0k");
    assert_eq!(read(reverb, "highcut2", "8k"), Some(8000.0));
    assert_eq!(p(reverb, "decay").format(0.45), "0.45 s");
    let pitch = time_fx("pitch-shifter");
    assert_eq!(p(pitch, "semitones").format(-12.0), "\u{2212}12 st");
    assert_eq!(builtin::pitch_shifter::interval_text(7.0, 0.0), "Fifth up");
    assert_eq!(builtin::pitch_shifter::interval_text(-12.0, 0.0), "Octave down");
    assert_eq!(builtin::pitch_shifter::interval_text(0.0, 0.0), "Unison");
}

#[test]
fn delay_names_and_inert_dials_follow_link_and_sync() {
    let def = time_fx("delay");
    let mut d = def.instantiate();
    let note = def.param("timeDivision").unwrap();
    assert_eq!(def.label_of(&d, note), "Note");
    assert!(def.is_inert(&d, "timeDivisionR"));
    assert!(def.is_shown(&d, "timeDivision") && !def.is_shown(&d, "timeMs"));
    d.params.insert("link".into(), 0.0);
    d.params.insert("sync".into(), 0.0);
    assert_eq!(def.label_of(&d, def.param("timeMs").unwrap()), "Time L");
    assert!(!def.is_inert(&d, "timeMsR"));
    assert!(def.is_shown(&d, "timeMs") && !def.is_shown(&d, "timeDivision"));
    assert!(note.reversed && def.param("sync").unwrap().latch == Some(("Free", "Sync")));
}

#[test]
fn reverb_dials_follow_the_mode() {
    let def = time_fx("reverb");
    let shown = |mode: f32| {
        let mut d = def.instantiate();
        d.params.insert("mode2".into(), mode);
        def.layout
            .clusters
            .iter()
            .map(|c| c.iter().filter_map(|c| c.param()).filter(|p| def.is_shown(&d, p)).count())
            .filter(|n| *n > 0)
            .collect::<Vec<_>>()
    };
    assert_eq!(shown(1.0), [3, 3, 1], "Hall");
    assert_eq!(shown(4.0), [1, 2, 3, 1], "Shimmer");
    assert_eq!(shown(5.0), [3, 2, 1, 1], "Haze");
}

#[test]
fn echo_picture_follows_the_machine_model() {
    let def = time_fx("echo");
    let d = def.instantiate();
    // Tape loop at the default Motor (260 mm/s), mode Heads 2+3.
    let pic = builtin::echo::picture(def, &d, 120.0);
    assert_eq!(pic.heads.len(), 3);
    let on: Vec<f32> = pic.heads.iter().filter(|h| h.on).map(|h| h.ms).collect();
    assert_eq!(on.len(), 2);
    assert!((on[0] - 52.4 / 260.0 * 1000.0).abs() < 0.1 && (on[1] - 75.6 / 260.0 * 1000.0).abs() < 0.1, "{on:?}");
    assert!(pic.pass_gain > 0.0 && pic.pass_gain < 1.0, "Intensity 5 is below self-oscillation");

    // Dragging head 2 authors the heads: the others keep their delay, the dragged one moves.
    let mut authored = d.clone();
    for (id, v) in builtin::echo::authored_patch(&pic, 1, 300.0) {
        authored.params.insert(id.into(), v);
    }
    let moved = builtin::echo::picture(def, &authored, 120.0);
    assert!(builtin::echo::authored(0, &|id: &str| def.value(&authored, def.param(id).unwrap())));
    assert!((moved.heads[1].ms - 300.0).abs() < 0.5, "{}", moved.heads[1].ms);
    assert!((moved.heads[2].ms - pic.heads[2].ms).abs() < 0.5);
    assert_eq!(moved.heads.iter().map(|h| h.on).collect::<Vec<_>>(), pic.heads.iter().map(|h| h.on).collect::<Vec<_>>());

    // Cartridge: Motor slides the record head; sync lands the first head on its note.
    let mut cart = d.clone();
    cart.params.insert("machine".into(), 2.0);
    cart.params.insert("motor".into(), 0.0);
    assert!((builtin::echo::picture(def, &cart, 120.0).heads[0].ms - builtin::echo::cartridge_ms(0.0)).abs() < 0.01);
    let mut synced = d.clone();
    synced.params.insert("sync".into(), 1.0);
    let first =
        builtin::echo::picture(def, &synced, 120.0).heads.iter().filter(|h| h.on && h.pan <= 0.0).map(|h| h.ms).fold(f32::MAX, f32::min);
    assert!((first - 500.0).abs() < 0.5, "Note L 1/4 at 120 BPM: {first}");
    // A note slower than the motor allows still lands exactly (speed clamped before the scale).
    synced.params.insert("machine".into(), 1.0);
    synced.params.insert("note_l".into(), 0.0);
    let slow =
        builtin::echo::picture(def, &synced, 60.0).heads.iter().filter(|h| h.on && h.pan <= 0.0).map(|h| h.ms).fold(f32::MAX, f32::min);
    assert!((slow - 12000.0).abs() < 2.0, "Note L 2/1. at 60 BPM: {slow}");
}

#[test]
fn echo_reads_back_the_text_it_shows() {
    let def = time_fx("echo");
    let generations = def.param("generations").unwrap();
    let motor = def.param("motor").unwrap();
    let mut d = def.instantiate();
    assert_eq!(def.parse_of(&d, generations, "45%"), Some(0.45));
    assert_eq!(def.parse_of(&d, generations, "45"), Some(0.45));
    for (machine, mains) in [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (2.0, 0.0)] {
        d.params.insert("machine".into(), machine);
        d.params.insert("drum_mains_60".into(), mains);
        let shown = def.text(&d, motor, 0.3);
        let read = def.parse_of(&d, motor, &shown).unwrap_or_else(|| panic!("machine {machine}: can't read {shown}"));
        assert!((read - 0.3).abs() < 0.01, "machine {machine}: {shown} -> {read}");
    }
}

#[test]
fn mix_lock_keeps_mix_across_presets() {
    let def = time_fx("grains");
    let mut d = def.instantiate();
    d.params.insert("mix".into(), 12.0);
    d.params.insert("mix_lock".into(), 1.0);
    let preset = builtin::DevicePreset::factory(def, &def.factory[0]);
    let v = def.preset_load_values(&d, &preset);
    assert_eq!(v["mix"], 12.0);
    d.params.insert("mix_lock".into(), 0.0);
    assert_ne!(def.preset_load_values(&d, &preset)["mix"], 12.0);
    assert!(time_fx("echo").param("throw").unwrap().momentary);
}
