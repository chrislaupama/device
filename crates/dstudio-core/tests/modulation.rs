//! Modulation links, chains and the device header commands (crate::modulation).

use dstudio_core::builtin;
use dstudio_core::modulation::{self, LFO_ID, RACK_ID};
use dstudio_core::*;
use std::collections::HashMap;

fn add(d: &mut Document, track: TrackId, index: Option<usize>, dev: Device) -> DeviceId {
    d.execute(Command::AddDevice { track, index, device: dev }, Origin::User).unwrap();
    let t = d.project().track(track).unwrap();
    t.devices[index.unwrap_or(t.devices.len() - 1)].id
}

fn setup() -> (Document, TrackId, DeviceId, DeviceId) {
    let mut d = Document::new(Project::default());
    let track = d.project().tracks[0].id;
    let comp = add(&mut d, track, None, builtin::find("compressor").unwrap().instantiate());
    let lfo = add(&mut d, track, Some(0), builtin::find(LFO_ID).unwrap().instantiate());
    (d, track, comp, lfo)
}

fn link(target: DeviceId, param: &str, slot: u8, min: f32, max: f32) -> ModLink {
    ModLink { slot, target, param: ModParam::Builtin(param.into()), min, max }
}

#[test]
fn lfo_links_add_edit_remove_with_undo() {
    let (mut d, _, comp, lfo) = setup();
    d.execute(Command::AddModLink { source: lfo, index: None, link: link(comp, "threshold", 0, -0.25, 0.25) }, Origin::User).unwrap();
    assert_eq!(d.undo_label(), Some("Add Modulation"));
    assert_eq!(d.project().device(lfo).unwrap().links.len(), 1);
    // Ranges are clamped to the source kind.
    d.execute(Command::SetModLink { source: lfo, index: 0, min: -3.0, max: 0.5 }, Origin::User).unwrap();
    assert_eq!(d.project().device(lfo).unwrap().links[0].min, -1.0);
    // Amount drags coalesce into one step.
    d.execute(Command::SetModLink { source: lfo, index: 0, min: -0.4, max: 0.4 }, Origin::User).unwrap();
    d.undo().unwrap();
    assert_eq!(d.project().device(lfo).unwrap().links[0].max, 0.25);
    d.execute(Command::RemoveModLink { source: lfo, index: 0 }, Origin::User).unwrap();
    assert!(d.project().device(lfo).unwrap().links.is_empty());
    d.undo().unwrap();
    assert_eq!(d.project().device(lfo).unwrap().links[0].param, ModParam::Builtin("threshold".into()));
}

#[test]
fn a_parameter_takes_three_sources() {
    let (mut d, track, comp, lfo) = setup();
    let lfo2 = add(&mut d, track, Some(0), builtin::find(LFO_ID).unwrap().instantiate());
    let lfo3 = add(&mut d, track, Some(0), builtin::find(LFO_ID).unwrap().instantiate());
    let lfo4 = add(&mut d, track, Some(0), builtin::find(LFO_ID).unwrap().instantiate());
    for s in [lfo, lfo2, lfo3] {
        d.execute(Command::AddModLink { source: s, index: None, link: link(comp, "ratio", 0, -0.1, 0.1) }, Origin::User).unwrap();
    }
    let err = d.execute(Command::AddModLink { source: lfo4, index: None, link: link(comp, "ratio", 0, -0.1, 0.1) }, Origin::User).unwrap_err();
    assert!(err.to_string().contains("at most 3"), "{err}");
    // The same source twice on one parameter is refused, other parameters are fine.
    assert!(d.execute(Command::AddModLink { source: lfo, index: None, link: link(comp, "ratio", 0, 0.0, 0.1) }, Origin::User).is_err());
    d.execute(Command::AddModLink { source: lfo4, index: None, link: link(comp, "attack", 0, -0.1, 0.1) }, Origin::User).unwrap();
}

#[test]
fn chains_group_bypass_and_keep_members_together() {
    let (mut d, track, comp, _lfo) = setup();
    // Chain head before the compressor, then the compressor joins it.
    let head = add(&mut d, track, Some(1), builtin::find(RACK_ID).unwrap().instantiate());
    d.execute(Command::SetDeviceChain { device: comp, chain: Some(head) }, Origin::User).unwrap();
    let mut member = builtin::find("compressor").unwrap().instantiate();
    member.chain = Some(head);
    let m2 = add(&mut d, track, Some(3), member);
    let t = d.project().track(track).unwrap();
    assert_eq!(modulation::members(&t.devices, 1), 2..4);
    // A member can't be added away from its chain, and chains don't hold LFOs.
    let mut stray = builtin::find("compressor").unwrap().instantiate();
    stray.chain = Some(head);
    assert!(d.execute(Command::AddDevice { track, index: Some(0), device: stray }, Origin::User).is_err());
    let lfo_id = d.project().track(track).unwrap().devices[0].id;
    assert!(d.execute(Command::SetDeviceChain { device: lfo_id, chain: Some(head) }, Origin::User).is_err());
    // Macro links stay inside the chain.
    d.execute(Command::AddModLink { source: head, index: None, link: link(m2, "threshold", 3, 0.2, 0.9) }, Origin::User).unwrap();
    assert!(d.execute(Command::AddModLink { source: head, index: None, link: link(comp, "threshold", 9, 0.2, 0.9) }, Origin::User).is_err());
    // The head's switch bypasses every member in the engine snapshot.
    d.execute(Command::SetDeviceEnabled { device: head, enabled: false }, Origin::User).unwrap();
    let (snap, _) = snapshot::build(d.project(), 48_000.0, &HashMap::new());
    let flags: Vec<bool> = snap.tracks[0].devices.iter().map(|x| x.enabled).collect();
    assert_eq!(flags, [true, false, false, false]);
    assert!(d.project().track(track).unwrap().devices[2].enabled, "members keep their own switch");
}

#[test]
fn header_commands_round_trip() {
    let (mut d, track, comp, lfo) = setup();
    d.execute(Command::SetDeviceLabel { device: lfo, label: Some("  Wobble ".into()) }, Origin::User).unwrap();
    assert_eq!(d.project().device(lfo).unwrap().display_name(), "Wobble");
    d.execute(Command::SetDeviceView { device: comp, folded: true, collapsed: false }, Origin::User).unwrap();
    assert!(d.project().device(comp).unwrap().folded);
    d.undo().unwrap();
    assert!(!d.project().device(comp).unwrap().folded);
    let head = add(&mut d, track, None, builtin::find(RACK_ID).unwrap().instantiate());
    d.execute(Command::SetMacroName { device: head, index: 2, name: "Air".into() }, Origin::User).unwrap();
    let h = d.project().device(head).unwrap();
    assert_eq!(modulation::macro_name(h, 2), "Air");
    assert_eq!(modulation::macro_name(h, 0), "Macro 1");
    assert!(d.execute(Command::SetMacroName { device: comp, index: 0, name: "x".into() }, Origin::User).is_err());
    // Swap keeps the slot; links that targeted the old device become inert.
    d.execute(Command::AddModLink { source: lfo, index: None, link: link(comp, "threshold", 0, -0.2, 0.2) }, Origin::User).unwrap();
    let i = d.project().track(track).unwrap().devices.iter().position(|x| x.id == comp).unwrap();
    d.execute(Command::ReplaceDevice { device: comp, with: builtin::find("compressor").unwrap().instantiate() }, Origin::User).unwrap();
    let t = d.project().track(track).unwrap();
    assert_ne!(t.devices[i].id, comp);
    assert!(modulation::routes(&t.devices, None, &t.devices[0]).is_empty());
    d.undo().unwrap();
    let t = d.project().track(track).unwrap();
    assert_eq!(t.devices[i].id, comp);
    assert_eq!(modulation::routes(&t.devices, None, &t.devices[0]).len(), 1);
}

#[test]
fn links_and_chains_persist() {
    let (mut d, track, comp, lfo) = setup();
    d.execute(Command::AddModLink { source: lfo, index: None, link: link(comp, "threshold", 0, -0.25, 0.25) }, Origin::User).unwrap();
    let head = add(&mut d, track, None, builtin::find(RACK_ID).unwrap().instantiate());
    d.execute(Command::SetMacroName { device: head, index: 0, name: "Drive".into() }, Origin::User).unwrap();
    let json = serde_json::to_string(d.project()).unwrap();
    let back: Project = serde_json::from_str(&json).unwrap();
    assert_eq!(&back, d.project());
    // Old projects without the new fields still load.
    let plain = serde_json::to_string(&Device::new(builtin::find("compressor").unwrap().plugin_ref())).unwrap();
    assert!(!plain.contains("links") && !plain.contains("folded"));
}
