//! Chain (device rack) head: groups the effects that follow it and carries
//! eight macros (handoffs/device-specs/chain-head.md).
//!
//! It passes audio through. Its power switch bypasses every member; each
//! macro moves the member parameters in its `Device::links` (slot = macro
//! index, crate::modulation). The panel is the custom "chain" view (macro
//! grid, name editor, targets and the Map pill).

use super::{DeviceDef, FactoryPreset, Format, Layout, ParamDef, View, LINEAR};
use crate::device::PluginKind;

/// Number of macros on a chain head.
pub const MACROS: usize = 8;

/// Default macro colours, cycled (reference values).
pub const MACRO_COLORS: [u32; 4] = [0xFFEF7A, 0x439B94, 0x64B5F6, 0xBA68C8];

const fn m(id: &'static str, name: &'static str) -> ParamDef {
    ParamDef::new(id, name, 0.0, 100.0, 0.0, LINEAR, "%", Format::Percent).help("Moves the parameters mapped to this macro.")
}

const PARAMS: &[ParamDef] = &[
    m("macro1", "Macro 1"),
    m("macro2", "Macro 2"),
    m("macro3", "Macro 3"),
    m("macro4", "Macro 4"),
    m("macro5", "Macro 5"),
    m("macro6", "Macro 6"),
    m("macro7", "Macro 7"),
    m("macro8", "Macro 8"),
];

const FACTORY: &[FactoryPreset] = &[];

pub static RACK: DeviceDef = DeviceDef {
    id: "device-rack",
    name: "Chain",
    kind: PluginKind::Effect,
    category: "Utility",
    purpose: "Groups effects into one chain with a shared power switch and eight macros.",
    params: PARAMS,
    layout: Layout { picker: &[], view: Some(View::Custom("chain")), sliders: &[], clusters: &[], faces: &[], min_height: 0.0, shown_when: &[], fixed_width: 0.0 },
    factory: FACTORY,
    sidechain: false,
    samples: 0,
};
