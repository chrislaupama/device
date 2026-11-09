//! Projects saved before the Synth became a built-in device kept the
//! instrument settings in Track::synth (saine_core::instrument::SynthParams)
//! and played them on the engine's classic synth. [upgrade_project] gives
//! every such instrument track a Synth device with the closest settings, and
//! moves its synth automation lanes onto the device's parameters.

use super::{fx_param, SYNTH_V2};
use crate::automation::{AutomationLane, AutomationTarget, SynthParam};
use crate::builtin::DeviceDef;
use crate::device::{Device, DeviceId};
use crate::instrument::{FilterMode, SynthParams, Waveform};
use crate::model::{Project, TrackKind};
use std::collections::BTreeMap;

fn wave(w: Waveform) -> f32 {
    match w {
        Waveform::Sine => 0.0,
        Waveform::Triangle => 1.0,
        Waveform::Saw => 2.0,
        Waveform::Square => 3.0,
    }
}

fn lfo_shape(w: Waveform) -> f32 {
    match w {
        Waveform::Sine => 0.0,
        Waveform::Triangle => 1.0,
        Waveform::Saw => 2.0,
        Waveform::Square => 4.0,
    }
}

/// Classic unison spread (cents) -> Unison Detune (the engine's spread is detune² × 50 cents).
fn detune(cents: f32) -> f32 {
    (cents / 50.0).clamp(0.0, 1.0).sqrt()
}

fn volume_db(output: f32) -> f32 {
    if output <= 1e-6 { -36.0 } else { 20.0 * output.log10() }
}

/// Classic filter envelope depth was ±6 octaves; ours is ±7.
const ENV_SCALE: f32 = 6.0 / 7.0;

/// Route slots used for the classic LFO's fixed destinations.
const LFO_PITCH_SLOT: usize = 1;
const LFO_CUTOFF_SLOT: usize = 2;
const LFO_AMP_SLOT: usize = 3;

fn lfo_pitch_amount(cents: f32) -> f32 {
    (cents / 100.0 / 24.0).clamp(-1.0, 1.0)
}
fn lfo_cutoff_amount(oct: f32) -> f32 {
    (oct / 8.0).clamp(-1.0, 1.0)
}
fn lfo_amp_amount(depth: f32) -> f32 {
    (-0.5 * depth).clamp(-1.0, 1.0)
}

/// Synth device parameter id and value conversion for a classic automation target.
fn lane_param(p: SynthParam) -> (String, fn(f32) -> f32) {
    use SynthParam::*;
    let same: fn(f32) -> f32 = |v| v;
    let route = |slot: usize| format!("mod{slot}_amount");
    match p {
        Osc1Level => ("osc1_level".into(), same),
        Osc1Shape => ("osc1_shape".into(), same),
        Osc1Fine => ("osc1_fine".into(), |v| v.clamp(-50.0, 50.0)),
        Osc2Level => ("osc2_level".into(), same),
        Osc2Shape => ("osc2_shape".into(), same),
        Osc2Fine => ("osc2_fine".into(), |v| v.clamp(-50.0, 50.0)),
        CrossFm => ("cross_fm".into(), same),
        SubLevel => ("sub_level".into(), same),
        NoiseLevel => ("noise_level".into(), same),
        FilterCutoff => ("cutoff".into(), same),
        FilterResonance => ("resonance".into(), same),
        FilterKeyTrack => ("key_track".into(), same),
        FilterEnvAmount => ("filter_env".into(), |v| (v * ENV_SCALE).clamp(-1.0, 1.0)),
        LfoRate => ("lfo1_rate".into(), |v| v.clamp(0.02, 40.0)),
        LfoToPitch => (route(LFO_PITCH_SLOT), lfo_pitch_amount),
        LfoToCutoff => (route(LFO_CUTOFF_SLOT), lfo_cutoff_amount),
        LfoToAmp => (route(LFO_AMP_SLOT), lfo_amp_amount),
        Glide => ("glide".into(), same),
        UnisonDetune => ("unison_detune".into(), detune),
        Output => ("volume".into(), |v| volume_db(v).clamp(-36.0, 12.0)),
    }
}

/// Synth device values closest to a classic synth setting. "lanes" are the
/// track's classic automation lanes: an LFO destination that is automated
/// keeps its route even when its static depth is zero.
pub fn values_from_legacy(sp: &SynthParams, lanes: &[SynthParam]) -> BTreeMap<String, f32> {
    let def: &DeviceDef = &SYNTH_V2;
    let mut v = def.defaults();
    let mut set = |id: &str, x: f32| {
        if let Some(p) = def.param(id) {
            v.insert(id.to_string(), p.snap(x));
        }
    };
    for (i, o) in sp.osc.iter().enumerate() {
        let n = i + 1;
        set(&format!("osc{n}_wave"), wave(o.wave));
        set(&format!("osc{n}_shape"), o.shape);
        set(&format!("osc{n}_octave"), (o.octave as f32 + 2.0).clamp(0.0, 4.0));
        set(&format!("osc{n}_semi"), o.semitones as f32 + 12.0);
        set(&format!("osc{n}_fine"), o.fine_cents.clamp(-50.0, 50.0));
        set(&format!("osc{n}_level"), o.level);
    }
    set("cross_fm", sp.cross_fm);
    set("sub_level", sp.sub_level);
    set("sub_waveform", 0.0);
    set("sub_tune", 12.0);
    set("noise_level", sp.noise_level);
    set("noise_type", 0.0);
    set(
        "filter_type",
        match sp.filter.mode {
            FilterMode::LowPass => 1.0,
            FilterMode::BandPass => 2.0,
            FilterMode::HighPass => 3.0,
        },
    );
    set("cutoff", sp.filter.cutoff_hz);
    set("resonance", sp.filter.resonance);
    set("filter_drive", 0.0);
    set("key_track", sp.filter.key_track);
    set("filter_env", sp.filter.env_amount * ENV_SCALE);
    set("filter_velocity", 0.0);
    for (prefix, e) in [("amp", &sp.amp_env), ("env2", &sp.filter_env)] {
        set(&format!("{prefix}_attack"), e.attack);
        set(&format!("{prefix}_decay"), e.decay);
        set(&format!("{prefix}_sustain"), e.sustain);
        set(&format!("{prefix}_release"), e.release);
    }
    set("amp_velocity", 1.0);
    set("lfo1_shape", lfo_shape(sp.lfo.wave));
    set("lfo1_rate", sp.lfo.rate_hz);
    set("lfo1_depth", 1.0);
    set("lfo1_retrigger", 1.0);
    for n in 1..=super::MOD_SLOTS {
        set(&format!("mod{n}_source"), 0.0);
        set(&format!("mod{n}_target"), 0.0);
        set(&format!("mod{n}_amount"), 0.0);
    }
    let routes = [
        (LFO_PITCH_SLOT, 1.0, lfo_pitch_amount(sp.lfo.to_pitch_cents), SynthParam::LfoToPitch),
        (LFO_CUTOFF_SLOT, 11.0, lfo_cutoff_amount(sp.lfo.to_cutoff_oct), SynthParam::LfoToCutoff),
        (LFO_AMP_SLOT, 15.0, lfo_amp_amount(sp.lfo.to_amp), SynthParam::LfoToAmp),
    ];
    for (slot, target, amount, lane) in routes {
        if amount != 0.0 || lanes.contains(&lane) {
            set(&format!("mod{slot}_source"), super::source::LFO1 as f32);
            set(&format!("mod{slot}_target"), target);
            set(&format!("mod{slot}_amount"), amount);
        }
    }
    set("voice_mode", if sp.voice.polyphony <= 1 { 2.0 } else { 0.0 });
    set("glide", sp.voice.glide);
    set("unison_voices", (sp.voice.unison.clamp(1, 8) - 1) as f32);
    set("unison_detune", detune(sp.voice.unison_detune_cents));
    set("unison_spread", 0.0);
    set("drift", 0.0);
    set("volume", volume_db(sp.output));
    for slot in 1..=4 {
        set(&fx_param(slot, "on"), 0.0);
    }
    v
}

/// A Synth device carrying a classic synth setting (id 0: allocate on insert).
pub fn device_from_legacy(sp: &SynthParams, lanes: &[SynthParam]) -> Device {
    let mut d = SYNTH_V2.instantiate();
    d.params = values_from_legacy(sp, lanes);
    d
}

/// Give every instrument track without an instrument a Synth device made
/// from its classic settings (the Synth's defaults when they were never edited), and move its classic synth lanes onto the
/// device. Returns whether anything changed.
pub fn upgrade_project(p: &mut Project) -> bool {
    let mut changed = false;
    for ti in 0..p.tracks.len() {
        changed |= upgrade_track(p, ti);
    }
    changed
}

/// [upgrade_project] for the track at index ti.
pub fn upgrade_track(p: &mut Project, ti: usize) -> bool {
    if p.tracks.get(ti).is_none_or(|t| t.kind != TrackKind::Instrument || t.instrument_plugin.is_some()) {
        return false;
    }
    let id = DeviceId(p.alloc_id());
    let t = &mut p.tracks[ti];
    let lanes: Vec<SynthParam> = t.automation.iter().filter_map(|l| if let AutomationTarget::Synth { param } = l.target { Some(param) } else { None }).collect();
    // Untouched classic settings (every new track) start from the Synth's
    // own defaults; edited ones carry over.
    let mut d = if lanes.is_empty() && t.synth == SynthParams::default() { SYNTH_V2.instantiate() } else { device_from_legacy(&t.synth, &lanes) };
    d.id = id;
    for lane in &mut t.automation {
        if let AutomationTarget::Synth { param } = lane.target {
            *lane = convert_lane(lane, param, id);
        }
    }
    t.instrument_plugin = Some(d);
    true
}

fn convert_lane(lane: &AutomationLane, param: SynthParam, device: DeviceId) -> AutomationLane {
    let (id, f) = lane_param(param);
    let q = SYNTH_V2.param(&id).expect("legacy lane maps to a Synth parameter");
    let mut out = lane.clone();
    out.target = AutomationTarget::Builtin { device, device_type: SYNTH_V2.id.into(), param: id };
    for pt in &mut out.points {
        pt.value = q.snap(f(pt.value));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::automation::AutoPoint;

    #[test]
    fn classic_settings_carry_over() {
        let mut sp = SynthParams::default();
        sp.osc[1].wave = Waveform::Square;
        sp.osc[0].octave = -1;
        sp.filter.mode = FilterMode::HighPass;
        sp.lfo.to_cutoff_oct = 2.0;
        sp.voice.polyphony = 1;
        sp.voice.unison = 3;
        let v = values_from_legacy(&sp, &[]);
        assert_eq!(v["osc2_wave"], 3.0);
        assert_eq!(v["osc1_octave"], 1.0);
        assert_eq!(v["filter_type"], 3.0);
        assert_eq!(v["cutoff"], sp.filter.cutoff_hz);
        assert_eq!(v["voice_mode"], 2.0);
        assert_eq!(v["unison_voices"], 2.0);
        assert_eq!((v["mod2_source"], v["mod2_target"], v["mod2_amount"]), (3.0, 11.0, 0.25));
        assert_eq!(v["mod1_source"], 0.0, "unused LFO destinations get no route");
        assert_eq!(v["fx2_on"], 0.0);
        assert_eq!(v.len(), SYNTH_V2.params.len());
    }

    #[test]
    fn projects_upgrade_once_with_their_lanes() {
        let mut p = Project::default();
        let ti = p.tracks.iter().position(|t| t.kind == TrackKind::Instrument).unwrap();
        p.tracks[ti].instrument_plugin = None;
        p.tracks[ti].automation.push(AutomationLane {
            target: AutomationTarget::Synth { param: SynthParam::FilterEnvAmount },
            enabled: true,
            points: vec![AutoPoint { time: 0, value: 0.7, curve: 0.0 }],
        });
        assert!(upgrade_project(&mut p));
        let d = p.tracks[ti].instrument_plugin.clone().unwrap();
        assert_eq!(d.plugin.id, "synth-v2");
        assert!(d.id.0 > 0);
        let lane = &p.tracks[ti].automation[0];
        assert_eq!(lane.target, AutomationTarget::Builtin { device: d.id, device_type: "synth-v2".into(), param: "filter_env".into() });
        assert!((lane.points[0].value - 0.6).abs() < 1e-6);
        assert!(!upgrade_project(&mut p), "already upgraded");
    }
}
