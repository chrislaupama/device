//! Modulation sources and chains: the LFO device, chain (rack) heads with
//! eight macros, and the links that let them move other parameters
//! (builtin::lfo, builtin::rack).
//!
//! * A source device owns its links (`Device::links`). An LFO adds a
//!   bipolar offset to each target's normalized value; a macro sets the
//!   target's normalized value from its own position. Offsets from several
//!   sources add and the result is clamped to 0..1. A parameter takes at
//!   most [MAX_SOURCES] sources.
//! * A chain is a head device (`builtin::rack`) followed by its members
//!   (`Device::chain` = the head's id). Members are contiguous right after
//!   the head; a device whose `chain` names a head it does not follow is
//!   treated as outside any chain. Chains don't nest and hold no LFOs.
//! * Links whose target is gone (removed device, other track) are inert, so
//!   undoing a removal brings the modulation back.
//!
//! The engine evaluates the waves and macros (engine/src/devices/lfo.cpp,
//! rack.cpp) from [routes]; [lfo_wave] mirrors its shapes for drawing.

use crate::builtin::{self, ParamDef, Taper};
use crate::device::{Device, DeviceId, ModLink, ModParam, PluginFormat};
use crate::model::Track;
use std::ops::Range;

pub const LFO_ID: &str = "lfo";
pub const RACK_ID: &str = "device-rack";
/// Sources (LFO or macro) one parameter accepts.
pub const MAX_SOURCES: usize = 3;
/// Amount of a link created from a knob's Modulators menu.
pub const DEFAULT_AMOUNT: f32 = 0.25;

fn is_builtin(d: &Device, id: &str) -> bool {
    d.plugin.format == PluginFormat::Builtin && d.plugin.id == id
}

pub fn is_lfo(d: &Device) -> bool {
    is_builtin(d, LFO_ID)
}

pub fn is_rack(d: &Device) -> bool {
    is_builtin(d, RACK_ID)
}

/// Modulation outputs of a device: 1 for an LFO, 8 macros for a chain head, else 0.
pub fn outputs(d: &Device) -> usize {
    if is_lfo(d) {
        1
    } else if is_rack(d) {
        builtin::rack::MACROS
    } else {
        0
    }
}

/// Members of the chain whose head is `devices[head]` (empty when it isn't a head).
pub fn members(devices: &[Device], head: usize) -> Range<usize> {
    let Some(h) = devices.get(head).filter(|d| is_rack(d)) else { return head..head };
    let end = devices[head + 1..].iter().position(|d| d.chain != Some(h.id)).map_or(devices.len(), |n| head + 1 + n);
    head + 1..end
}

/// Index of the chain head `devices[i]` belongs to, if any.
pub fn head_of(devices: &[Device], i: usize) -> Option<usize> {
    let want = devices.get(i)?.chain?;
    let h = devices[..i].iter().rposition(|d| d.id == want)?;
    members(devices, h).contains(&i).then_some(h)
}

/// Whether `target` is a member of the chain headed by `head` (by position).
pub fn is_member(devices: &[Device], head: DeviceId, target: DeviceId) -> bool {
    devices.iter().position(|d| d.id == target).and_then(|i| head_of(devices, i)).is_some_and(|h| devices[h].id == head)
}

/// Whether `devices[i]` processes: its own switch and its chain head's.
pub fn effectively_enabled(devices: &[Device], i: usize) -> bool {
    devices[i].enabled && head_of(devices, i).is_none_or(|h| devices[h].enabled)
}

/// Every link on the track that moves (`target`, `param`): (source id, link index).
pub fn sources_of(track: &Track, target: DeviceId, param: &ModParam) -> Vec<(DeviceId, usize)> {
    track
        .devices
        .iter()
        .flat_map(|s| s.links.iter().enumerate().filter(|(_, l)| l.target == target && l.param == *param).map(move |(i, _)| (s.id, i)))
        .collect()
}

/// A device on the track or its instrument plugin.
pub fn track_device(track: &Track, id: DeviceId) -> Option<&Device> {
    track.devices.iter().chain(track.instrument_plugin.iter()).find(|d| d.id == id)
}

/// Why `source` (slot `slot`) can't move (`target`, `param`), or None when it can.
pub fn link_refusal(track: &Track, source: &Device, slot: u8, target: DeviceId, param: &ModParam) -> Option<String> {
    if (slot as usize) >= outputs(source) {
        return Some(format!("{} has no modulation output {slot}", source.display_name()));
    }
    let Some(t) = track_device(track, target) else { return Some("the target must be on the same track".into()) };
    if outputs(t) > 0 {
        return Some("LFOs and chains can't be modulated".into());
    }
    match param {
        ModParam::Builtin(id) => {
            let Some(def) = builtin::def_of(t) else { return Some(format!("{} has no built-in parameter {id:?}", t.display_name())) };
            if !def.param(id).is_some_and(|p| p.automatable) {
                return Some(format!("{} has no modulatable parameter {id:?}", def.name));
            }
        }
        ModParam::Plugin(_) if t.plugin.format == PluginFormat::Builtin => return Some("built-in parameters are addressed by id".into()),
        ModParam::Plugin(_) => {}
    }
    if is_rack(source) && !is_member(&track.devices, source.id, target) {
        return Some("a macro moves parameters inside its own chain".into());
    }
    if source.links.iter().any(|l| l.slot == slot && l.target == target && l.param == *param) {
        return Some("already modulated by this source".into());
    }
    if sources_of(track, target, param).len() >= MAX_SOURCES {
        return Some(format!("a parameter takes at most {MAX_SOURCES} modulation sources"));
    }
    None
}

/// Clamp a link's range to its source kind: LFO -1..1, macro 0..1.
pub fn clamp_range(source: &Device, min: f32, max: f32) -> Option<(f32, f32)> {
    if !min.is_finite() || !max.is_finite() {
        return None;
    }
    let lo = if is_lfo(source) { -1.0 } else { 0.0 };
    Some((min.clamp(lo, 1.0), max.clamp(lo, 1.0)))
}

/// First free name in "base", "base 2", "base 3"... among the track's device labels.
pub fn next_name(track: &Track, base: &str) -> String {
    let taken = |n: &str| track.devices.iter().any(|d| d.display_name() == n);
    if !taken(base) {
        return base.to_string();
    }
    (2..).map(|i| format!("{base} {i}")).find(|n| !taken(n)).expect("a free name")
}

/// Name of macro `i` of a chain head.
pub fn macro_name(head: &Device, i: usize) -> String {
    match head.macro_names.get(i).map(|s| s.trim()) {
        Some(n) if !n.is_empty() => n.to_string(),
        _ => format!("Macro {}", i + 1),
    }
}

/// Position of macro `slot` on a chain head, 0..1.
pub fn macro_position(head: &Device, slot: u8) -> f32 {
    let def = &builtin::rack::RACK;
    def.params.get(slot as usize).map_or(0.0, |p| def.value(head, p) / 100.0)
}

/// Normalized value of a link's target for macro position `m` (0..1).
pub fn macro_target(link: &ModLink, m: f32) -> f32 {
    (link.min + (link.max - link.min) * m).clamp(0.0, 1.0)
}

/// The value a built-in parameter rests at once macros are applied (the
/// knob's displayed value; LFOs move around it). None when no macro maps it.
pub fn macro_value(track: &Track, target: DeviceId, param: &ParamDef) -> Option<f32> {
    let key = ModParam::Builtin(param.id.to_string());
    let mut out = None;
    for s in track.devices.iter().filter(|d| is_rack(d)) {
        for l in s.links.iter().filter(|l| l.target == target && l.param == key) {
            out = Some(param.from_normalized(macro_target(l, macro_position(s, l.slot))));
        }
    }
    out
}

// ---------------------------------------------------------------- LFO shapes

/// Per-cycle random value in -1..1 (deterministic, repeats every 4096 cycles).
pub fn cycle_random(cycle: i64) -> f32 {
    let n = cycle.rem_euclid(4096) as u32;
    let mut x = n.wrapping_mul(0x9E37_79B1) ^ 0x85EB_CA6B;
    x ^= x >> 15;
    x = x.wrapping_mul(0x2C1B_3C6D);
    x ^= x >> 12;
    x = x.wrapping_mul(0x297A_2D39);
    x ^= x >> 15;
    (x >> 8) as f32 / 8_388_608.0 - 1.0
}

/// Bipolar wave value (-1..1) of shape `shape` (builtin::lfo::SHAPES order) at
/// cycle position `x` (cycles since the start; the fraction is the phase).
pub fn lfo_wave(shape: usize, x: f64) -> f32 {
    let c = x.floor();
    let p = (x - c) as f32;
    match shape {
        0 => (std::f32::consts::TAU * p).sin(),
        1 => {
            if p < 0.25 {
                4.0 * p
            } else if p < 0.75 {
                2.0 - 4.0 * p
            } else {
                4.0 * p - 4.0
            }
        }
        2 => 2.0 * p - 1.0,
        3 => 1.0 - 2.0 * p,
        4 => {
            if p < 0.5 {
                1.0
            } else {
                -1.0
            }
        }
        5 => cycle_random(c as i64),
        _ => {
            let (a, b) = (cycle_random(c as i64), cycle_random(c as i64 + 1));
            a + (b - a) * (1.0 - (std::f32::consts::PI * p).cos()) * 0.5
        }
    }
}

// ---------------------------------------------------------------- engine routes

/// One link resolved for the engine (se_mod_route).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModRoute {
    pub target: DeviceId,
    /// Built-in: parameter index in definition order. Plugin: format parameter id.
    pub param: u32,
    pub plugin: bool,
    pub slot: u8,
    /// Macro: sets the normalized value instead of adding to it.
    pub absolute: bool,
    pub min: f32,
    pub max: f32,
    /// Built-in targets: the parameter's plain range and taper (normalisation in the engine).
    pub p_min: f32,
    pub p_max: f32,
    pub p_log: bool,
    pub p_skew: f32,
    pub p_step: f32,
}

/// Engine routes of `source` on a track (`devices` = its chain,
/// `instrument` = its instrument plugin). Inert links are left out.
pub fn routes(devices: &[Device], instrument: Option<&Device>, source: &Device) -> Vec<ModRoute> {
    if outputs(source) == 0 {
        return Vec::new();
    }
    let rack = is_rack(source);
    source
        .links
        .iter()
        .filter(|l| (l.slot as usize) < outputs(source))
        .filter_map(|l| {
            let t = devices.iter().chain(instrument).find(|d| d.id == l.target && outputs(d) == 0)?;
            if rack && !is_member(devices, source.id, t.id) {
                return None;
            }
            let mut r = ModRoute {
                target: t.id,
                param: 0,
                plugin: false,
                slot: l.slot,
                absolute: rack,
                min: l.min,
                max: l.max,
                p_min: 0.0,
                p_max: 1.0,
                p_log: false,
                p_skew: 1.0,
                p_step: 0.0,
            };
            match &l.param {
                ModParam::Builtin(id) => {
                    let def = builtin::def_of(t)?;
                    let i = def.index_of(id)?;
                    let p = &def.params[i];
                    r.param = i as u32;
                    r.p_min = p.min;
                    r.p_max = p.max;
                    match p.taper {
                        Taper::Log => r.p_log = true,
                        Taper::Linear { skew } => r.p_skew = skew,
                    }
                    r.p_step = if p.taper == Taper::Log { 0.0 } else { p.step };
                }
                ModParam::Plugin(id) if t.plugin.format != PluginFormat::Builtin => {
                    r.param = *id;
                    r.plugin = true;
                }
                ModParam::Plugin(_) => return None,
            }
            Some(r)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ColorIndex, TrackId, TrackKind};

    fn track() -> Track {
        let mut t = Track::new(TrackId(1), TrackKind::Audio, "A", ColorIndex(0));
        let mut comp = builtin::find("compressor").unwrap().instantiate();
        comp.id = DeviceId(10);
        let mut lfo = builtin::find(LFO_ID).unwrap().instantiate();
        lfo.id = DeviceId(11);
        let mut rack = builtin::find(RACK_ID).unwrap().instantiate();
        rack.id = DeviceId(12);
        let mut member = builtin::find("compressor").unwrap().instantiate();
        member.id = DeviceId(13);
        member.chain = Some(DeviceId(12));
        t.devices = vec![lfo, comp, rack, member];
        t
    }

    #[test]
    fn chains_are_contiguous_runs_after_their_head() {
        let mut t = track();
        assert_eq!(members(&t.devices, 2), 3..4);
        assert_eq!(head_of(&t.devices, 3), Some(2));
        assert_eq!(head_of(&t.devices, 1), None);
        t.devices[2].enabled = false;
        assert!(!effectively_enabled(&t.devices, 3));
        assert!(effectively_enabled(&t.devices, 1));
        // A member separated from its head is outside the chain.
        let stray = t.devices.remove(3);
        t.devices.insert(1, stray);
        assert_eq!(head_of(&t.devices, 1), None);
        assert_eq!(members(&t.devices, 3), 4..4);
    }

    #[test]
    fn link_rules() {
        let t = track();
        let lfo = &t.devices[0];
        let rack = &t.devices[2];
        let th = ModParam::Builtin("threshold".into());
        assert_eq!(link_refusal(&t, lfo, 0, DeviceId(10), &th), None);
        assert!(link_refusal(&t, lfo, 1, DeviceId(10), &th).is_some(), "an LFO has one output");
        assert!(link_refusal(&t, lfo, 0, DeviceId(12), &ModParam::Builtin("macro1".into())).is_some(), "no modulating a chain");
        assert!(link_refusal(&t, rack, 0, DeviceId(10), &th).is_some(), "macros stay in their chain");
        assert_eq!(link_refusal(&t, rack, 7, DeviceId(13), &th), None);
        assert!(link_refusal(&t, lfo, 0, DeviceId(10), &ModParam::Builtin("nope".into())).is_some());
        let mut full = t.clone();
        for slot in 0..3u8 {
            let target = DeviceId(13);
            full.devices[2].links.push(ModLink { slot, target, param: th.clone(), min: 0.0, max: 1.0 });
        }
        assert!(link_refusal(&full, &full.devices[0].clone(), 0, DeviceId(13), &th).unwrap().contains("at most 3"));
        // A built-in parameter id on a third-party plugin.
        let mut plug = t.clone();
        let mut p = Device::new(crate::device::PluginRef { format: PluginFormat::Clap, ..t.devices[1].plugin.clone() });
        p.id = DeviceId(14);
        plug.devices.push(p);
        assert!(link_refusal(&plug, lfo, 0, DeviceId(14), &th).is_some());
        // A member moved away from its head is outside the chain for macros too.
        let mut stray = t.clone();
        let m = stray.devices.remove(3);
        stray.devices.insert(0, m);
        assert!(link_refusal(&stray, &stray.devices[3].clone(), 0, DeviceId(13), &th).is_some());
        let mut rack = stray.devices[3].clone();
        rack.links.push(ModLink { slot: 0, target: DeviceId(13), param: th.clone(), min: 0.0, max: 1.0 });
        assert!(routes(&stray.devices, None, &rack).is_empty());
    }

    #[test]
    fn names_count_up() {
        let mut t = track();
        assert_eq!(next_name(&t, "LFO"), "LFO 2");
        t.devices[0].label = Some("LFO 2".into());
        assert_eq!(next_name(&t, "LFO"), "LFO");
        assert_eq!(macro_name(&t.devices[2], 3), "Macro 4");
    }

    #[test]
    fn shapes_follow_the_spec() {
        let close = |a: f32, b: f32| (a - b).abs() < 1e-5;
        assert!(close(lfo_wave(0, 0.25), 1.0) && close(lfo_wave(0, 0.0), 0.0));
        assert!(close(lfo_wave(1, 0.25), 1.0) && close(lfo_wave(1, 0.5), 0.0) && close(lfo_wave(1, 0.75), -1.0));
        assert!(close(lfo_wave(2, 0.0), -1.0) && close(lfo_wave(3, 0.0), 1.0));
        assert!(close(lfo_wave(4, 0.2), 1.0) && close(lfo_wave(4, 0.7), -1.0));
        // Random holds per cycle and repeats every 4096 cycles; smooth random joins the holds.
        assert_eq!(lfo_wave(5, 3.1), lfo_wave(5, 3.9));
        assert_eq!(lfo_wave(5, 3.5), lfo_wave(5, 4096.0 + 3.5));
        assert!(close(lfo_wave(6, 7.0), cycle_random(7)) && close(lfo_wave(6, 7.999_999), cycle_random(8)));
        for c in 0..4096 {
            let v = cycle_random(c);
            assert!((-1.0..=1.0).contains(&v));
        }
    }

    #[test]
    fn routes_resolve_targets_and_tapers() {
        let mut t = track();
        t.devices[0].links.push(ModLink {
            slot: 0,
            target: DeviceId(10),
            param: ModParam::Builtin("attack".into()),
            min: -0.25,
            max: 0.25,
        });
        t.devices[0].links.push(ModLink {
            slot: 0,
            target: DeviceId(99),
            param: ModParam::Builtin("attack".into()),
            min: -0.25,
            max: 0.25,
        });
        t.devices[2].links.push(ModLink { slot: 2, target: DeviceId(13), param: ModParam::Builtin("ratio".into()), min: 0.2, max: 0.8 });
        let r = routes(&t.devices, None, &t.devices[0]);
        assert_eq!(r.len(), 1, "a missing target is inert");
        assert!(r[0].p_log && !r[0].absolute);
        assert_eq!(Some(r[0].param as usize), builtin::find("compressor").unwrap().index_of("attack"));
        let r = routes(&t.devices, None, &t.devices[2]);
        assert!(r[0].absolute && r[0].slot == 2);
        assert!(macro_value(&t, DeviceId(13), builtin::find("compressor").unwrap().param("ratio").unwrap()).is_some());
    }
}
