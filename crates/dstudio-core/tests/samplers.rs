//! Built-in samplers: instruments in the instrument slot, sample slots with
//! undo, slices, persistence, snapshot asset mapping and value formats.

use dstudio_core::builtin::{self, Format};
use dstudio_core::*;
use std::collections::HashMap;

fn add_asset(d: &mut Document, id: u64) -> AssetId {
    let asset = Asset {
        id: AssetId(id),
        relative_path: None,
        absolute_path: format!("/tmp/s{id}.wav"),
        sample_rate: 48_000.0,
        channels: 2,
        frames: 48_000,
    };
    d.execute(Command::AddAsset { asset }, Origin::User).unwrap();
    AssetId(id)
}

fn sample(slot: u32, asset: AssetId) -> DeviceSample {
    DeviceSample { slot, asset, name: format!("s{slot}"), slices: vec![] }
}

#[test]
fn instruments_go_in_the_instrument_slot_not_the_chain() {
    let mut d = Document::new(Project::default());
    let (a, i) = (d.project().tracks[0].id, d.project().tracks[1].id);
    for id in ["sampler-steam", "pad-sampler", "drum-rack"] {
        let def = builtin::find(id).unwrap();
        assert_eq!(def.kind, PluginKind::Instrument);
        assert!(d.execute(Command::AddDevice { track: a, index: None, device: def.instantiate() }, Origin::User).is_err());
        assert!(
            d.execute(Command::SetInstrumentPlugin { track: a, device: Some(def.instantiate()) }, Origin::User).is_err(),
            "audio track"
        );
        d.execute(Command::SetInstrumentPlugin { track: i, device: Some(def.instantiate()) }, Origin::User).unwrap();
        assert_eq!(d.project().track(i).unwrap().instrument_plugin.as_ref().unwrap().plugin.id, id);
    }
    assert_eq!(d.undo_label(), Some("Load Drum Rack"));
}

#[test]
fn sample_slots_load_replace_clear_with_undo_and_guard_assets() {
    let mut d = Document::new(Project::default());
    let i = d.project().tracks[1].id;
    let (s1, s2) = (add_asset(&mut d, 500), add_asset(&mut d, 501));
    d.execute(Command::SetInstrumentPlugin { track: i, device: Some(builtin::find("drum-rack").unwrap().instantiate()) }, Origin::User)
        .unwrap();
    let dev = d.project().track(i).unwrap().instrument_plugin.as_ref().unwrap().id;

    d.execute(Command::SetDeviceSample { device: dev, slot: 9, sample: Some(sample(9, s1)) }, Origin::User).unwrap();
    d.execute(Command::SetDeviceSample { device: dev, slot: 0, sample: Some(sample(0, s2)) }, Origin::User).unwrap();
    let slots = |d: &Document| d.project().device(dev).unwrap().samples.iter().map(|s| (s.slot, s.asset)).collect::<Vec<_>>();
    assert_eq!(slots(&d), [(0, s2), (9, s1)], "kept in slot order");

    for bad in [
        Command::SetDeviceSample { device: dev, slot: 128, sample: Some(sample(128, s1)) },
        Command::SetDeviceSample { device: dev, slot: 3, sample: Some(sample(4, s1)) },
        Command::SetDeviceSample { device: dev, slot: 3, sample: Some(sample(3, AssetId(999))) },
        Command::SetDeviceSample {
            device: dev,
            slot: 3,
            sample: Some(DeviceSample { slices: vec![Slice::at(0.5), Slice::at(0.2)], ..sample(3, s1) }),
        },
    ] {
        let before = d.project().clone();
        assert!(d.execute(bad, Origin::User).is_err());
        assert_eq!(d.project(), &before);
    }

    assert!(d.execute(Command::RemoveAsset { asset: s1 }, Origin::User).is_err(), "played by the rack");
    d.execute(Command::SetDeviceSample { device: dev, slot: 9, sample: None }, Origin::User).unwrap();
    assert_eq!(d.undo_label(), Some("Remove Sample"));
    d.execute(Command::RemoveAsset { asset: s1 }, Origin::User).unwrap();
    d.undo().unwrap();
    d.undo().unwrap();
    assert_eq!(slots(&d), [(0, s2), (9, s1)]);
    d.undo().unwrap();
    assert_eq!(slots(&d), [(9, s1)]);

    // Snapshot: engine assets for the samples that are loaded.
    let (snap, _) = snapshot::build(d.project(), 48_000.0, &HashMap::from([(s1, 77), (s2, 78)]));
    assert_eq!(snap.device_assets, HashMap::from([(s1, 77)]));

    // Slices and samples persist.
    let sl = vec![Slice::at(0.0), Slice { start: 0.25, gain: -6.0, pan: 0.5, pitch: 3.0 }];
    d.execute(
        Command::SetDeviceSample { device: dev, slot: 9, sample: Some(DeviceSample { slices: sl.clone(), ..sample(9, s1) }) },
        Origin::User,
    )
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    persist::save(d.project(), dir.path()).unwrap();
    let l = persist::load(dir.path()).unwrap();
    assert_eq!(l.project.device(dev).unwrap().samples, d.project().device(dev).unwrap().samples);
    assert_eq!(l.project.device(dev).unwrap().samples[0].slices, sl);

    // Effects and plugins hold no samples.
    let mut comp = builtin::find("compressor").unwrap().instantiate();
    comp.samples.push(sample(0, s1));
    assert!(d.execute(Command::AddDevice { track: d.project().tracks[0].id, index: None, device: comp }, Origin::User).is_err());
}

#[test]
fn new_value_formats_print_and_parse() {
    let def = builtin::find("sampler-steam").unwrap();
    let p = |id| def.param(id).unwrap();
    assert_eq!(p("root").format(60.0), "C3");
    assert_eq!(p("root").parse("A3"), Some(69.0));
    assert_eq!(p("pan").format(0.0), "C");
    assert_eq!(p("pan").format(-0.25), "25L");
    assert_eq!(p("pan").parse("40R"), Some(0.4));
    assert_eq!(p("transpose").format(7.0), "+7 st");
    assert!(matches!(p("envDepth").format, Format::Signed { scale: 48, .. }));
    assert_eq!(builtin::find("drum-rack").unwrap().params.len(), 16 * 11);
}
