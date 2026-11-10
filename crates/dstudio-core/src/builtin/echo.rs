//! Echo (reference: the reference app's built-in "Echo"): a tape-loop, magnetic-drum
//! and cartridge echo with several playback heads. Parameter ids, ranges,
//! defaults, tapers, the machine constants and the panel arrangement are
//! taken from the reference as facts; factory preset names and values are our own.

use super::*;
use crate::device::PluginKind;

pub const MACHINES: &[&str] = &["Tape Loop", "Drum", "Cartridge"];
pub const LOOP_MODES: &[&str] = &[
    "1 · Head 1", "2 · Head 2", "3 · Head 3", "4 · Heads 2+3", "5 · Head 1 + spring", "6 · Head 2 + spring", "7 · Head 3 + spring",
    "8 · Heads 1+2 + spring", "9 · Heads 2+3 + spring", "10 · Heads 1+3 + spring", "11 · All heads + spring", "12 · Spring only",
];
/// Heads (bit 0 = head 1) each Tape Loop mode plays; modes 5..=12 add the spring.
pub const LOOP_MASKS: [u8; 12] = [1, 2, 4, 6, 1, 2, 4, 3, 6, 5, 7, 0];
pub const DRUM_MODES: &[&str] = &["Echo", "Repeat", "Swell"];
pub const CARTRIDGE_MODES: &[&str] = &["Echo", "Sound-on-Sound"];
pub const DRUM_SWITCH: &[&str] = &[
    "1 · Head 1", "2 · Head 2", "3 · Head 3", "4 · Head 4", "5 · Heads 1+2", "6 · Heads 2+3", "7 · Heads 3+4", "8 · Heads 1+3", "9 · Heads 2+4",
    "10 · Heads 1+2+3", "11 · Heads 2+3+4", "12 · All four",
];
pub const DRUM_MASKS: [u8; 12] = [1, 2, 4, 8, 3, 6, 12, 5, 10, 7, 14, 15];
/// Note values (longest first) and their length in quarter notes.
pub const NOTES: &[&str] = &[
    "2/1.", "2/1", "2/1T", "1/1.", "1/1", "1/1T", "1/2.", "1/2", "1/2T", "1/4.", "1/4", "1/4T", "1/8.", "1/8", "1/8T", "1/16.", "1/16", "1/16T", "1/32.", "1/32", "1/32T",
];
pub fn note_beats(i: usize) -> f32 {
    const BASE: [f32; 7] = [8.0, 4.0, 2.0, 1.0, 0.5, 0.25, 0.125];
    let b = BASE[(i / 3).min(6)];
    match i % 3 {
        0 => b * 1.5,
        1 => b,
        _ => b * 2.0 / 3.0,
    }
}
const OFF_ON: &[&str] = &["Off", "On"];
const SPACINGS: &[&str] = &["Machine", "Even", "Triplet", "Golden", "Silver"];
const KEYS: &[&str] = &["Internal", "External"];
const HEADS: &[&str] = &["Authentic", "1", "2", "3", "4", "5", "6"];
const HEADS_MACHINE: &[&str] = &["None", "Tape Loop", "Drum", "Cartridge"];

/// Machine constants (mm, mm/s): loop length, erase gap, head positions from the record head.
pub struct Machine {
    pub loop_mm: f32,
    pub gap_mm: f32,
    pub heads_mm: &'static [f32],
    /// Self-oscillation onset on the Intensity scale.
    pub onset: f32,
    /// Generations value where the loss per pass is "authentic".
    pub detent: f32,
    /// Playback head gap loss (um) used for the per-pass treble loss.
    pub spacing_um: f32,
    /// Speed (mm/s) at the default Motor setting.
    pub default_speed: f32,
}
pub const TAPE_LOOP: Machine = Machine { loop_mm: 1600.0, gap_mm: 18.0, heads_mm: &[27.6, 52.4, 75.6], onset: 7.4, detent: 0.45, spacing_um: 3.0, default_speed: 260.0 };
pub const DRUM: Machine = Machine { loop_mm: 388.28, gap_mm: 40.0, heads_mm: &[34.0, 68.0, 102.0, 136.0], onset: 8.1, detent: 0.30, spacing_um: 1.5, default_speed: 459.46 };
pub const CARTRIDGE: Machine = Machine { loop_mm: 2400.0, gap_mm: 24.0, heads_mm: &[162.56], onset: 7.8, detent: 0.55, spacing_um: 5.0, default_speed: 203.2 };
pub const MACHINE_SPECS: [&Machine; 3] = [&TAPE_LOOP, &DRUM, &CARTRIDGE];
/// Longest echo the engine's tape holds (s), engine/src/devices/echo.cpp kMaxSeconds.
pub const MAX_SECONDS: f32 = 14.0;
pub const CARTRIDGE_SPEED: f32 = 203.2;
pub const CARTRIDGE_TRAVEL: f32 = 146.3;
pub const DRUM_SPEED: f32 = 459.46;

/// Tape speed (mm/s) of a machine at a Motor value (Tape Loop, Drum).
pub fn speed(machine: u32, motor: f32, mains_60: bool) -> f32 {
    match machine {
        0 => 120.0 + motor.clamp(0.0, 1.0) * 280.0,
        1 => (DRUM_SPEED * if mains_60 { 1.2 } else { 1.0 } * (0.5 + motor.clamp(0.0, 1.0))).max(1.0),
        _ => CARTRIDGE_SPEED,
    }
}

/// Cartridge delay (ms) at a Motor value: the record head slides towards the playback head.
pub fn cartridge_ms(motor: f32) -> f32 {
    (CARTRIDGE.heads_mm[0] - motor.clamp(0.0, 1.0) * CARTRIDGE_TRAVEL) / CARTRIDGE_SPEED * 1000.0
}

/// One playback head as the tail picture sees it (the engine's head model, engine/src/devices/echo.cpp).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PictureHead {
    /// Distance from the record head (mm) and the resulting delay (ms, sync applied).
    pub mm: f32,
    pub ms: f32,
    /// -1 left .. 1 right.
    pub pan: f32,
    /// Playback level (linear).
    pub gain: f32,
    pub on: bool,
    pub fb: bool,
}

/// Predicted echo of the current settings: heads and the level change per pass.
#[derive(Debug, Clone, PartialEq)]
pub struct Picture {
    pub machine: u32,
    pub speed: f32,
    pub heads: Vec<PictureHead>,
    /// Linear gain from one repeat to the next (loop gain times the per-pass treble loss).
    pub pass_gain: f32,
}

/// The xorshift generator the engine seeds its pan pattern with (dsp::Rng).
struct Rng(u32);

impl Rng {
    fn bi(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        2.0 * (self.0 >> 8) as f32 * (1.0 / 16_777_216.0) - 1.0
    }
}

/// Generations 0 .. detent .. 1 -> wear factor 0.35 .. 1 .. 6.
fn wear(g: f32, detent: f32) -> f32 {
    if g <= detent { 0.35 + 0.65 * g / detent.max(1e-3) } else { 1.0 + 5.0 * (g - detent) / (1.0 - detent).max(1e-3) }
}

/// Head mask of the current mode (bit 0 = head 1) and the heads that feed back.
fn masks(machine: u32, v: &dyn Fn(&str) -> f32) -> (u8, u8) {
    match machine {
        0 => {
            let m = LOOP_MASKS[(v("mode_loop").round() as usize).min(11)];
            (m, m)
        }
        1 => {
            let m = DRUM_MASKS[(v("combo_drum").round() as usize).min(11)];
            (m, match v("mode_drum").round() as u32 { 0 => 0, 1 => m, _ => 0x0f })
        }
        _ => (1, 1),
    }
}

/// Whether the h1..h6 blocks (written by head drags) drive the current machine.
pub fn authored(machine: u32, v: &dyn Fn(&str) -> f32) -> bool {
    v("heads").round() >= 1.0 && v("heads_machine").round() as u32 == machine + 1
}

/// Head parameter id `h{n}_{field}` (n from 1) as the definition's static id.
pub fn head_param(n: usize, field: &str) -> Option<&'static str> {
    ECHO.param(&format!("h{n}_{field}")).map(|p| p.id)
}

/// The tail picture of a device's current values at `bpm`.
pub fn picture(def: &DeviceDef, d: &Device, bpm: f64) -> Picture {
    let v = |id: &str| def.param(id).map_or(0.0, |p| def.value(d, p));
    let machine = (v("machine").round() as u32).min(2);
    let ms = MACHINE_SPECS[machine as usize];
    let motor = v("motor").clamp(0.0, 1.0);
    let mut speed = speed(machine, motor, v("drum_mains_60") >= 0.5);

    let humanize = v("width_humanize").clamp(0.0, 100.0) * 0.01;
    let mut rng = Rng(0x7061_6e2e ^ (v("seed").clamp(0.0, 65535.0) as u32).wrapping_mul(2_654_435_761));
    if rng.0 == 0 {
        rng.0 = 0x9e37_79b9;
    }
    let mut pattern = [0.0f32; 6];
    for i in 0..6 {
        let side = if i & 1 == 1 { 1.0 } else { -1.0 };
        let mut p = (side * (1.0 - humanize) + rng.bi() * humanize).clamp(-1.0, 1.0);
        if i > 0 && (p - pattern[i - 1]).abs() < 0.15 {
            p = (p + if p >= pattern[i - 1] { 0.15 } else { -0.15 }).clamp(-1.0, 1.0);
        }
        pattern[i] = p;
    }

    let mut heads = Vec::new();
    if authored(machine, &v) {
        let span = (ms.loop_mm - ms.gap_mm - 1.1).max(1.0);
        for n in 1..=(v("heads").round() as usize).clamp(1, 6) {
            let h = |f: &str| head_param(n, f).map_or(0.0, |id| v(id));
            if h("present") < 0.5 {
                continue;
            }
            let lv = h("level").clamp(0.0, 1.0);
            let gain = if lv <= 0.01 { 0.0 } else { 10f32.powf((-60.0 + 66.0 * (lv - 0.01) / 0.99) / 20.0) };
            let pan = if h("pan_pinned") >= 0.5 { h("pan").clamp(-1.0, 1.0) } else if h("right") >= 0.5 { pattern[n - 1].abs() } else { -pattern[n - 1].abs() };
            heads.push(PictureHead { mm: 0.1 + h("pos").clamp(0.0, 1.0) * span, ms: 0.0, pan, gain, on: h("on") >= 0.5 && gain > 0.0, fb: h("fb") >= 0.5 });
        }
    } else {
        let (mask, fb_mask) = masks(machine, &v);
        let spacing = v("spacing").round().clamp(0.0, 4.0) as u32;
        let morph = v("spacing_morph").clamp(0.0, 1.0);
        let count = ms.heads_mm.len();
        for i in 0..count {
            let mut mm = if machine == 2 { ms.heads_mm[0] - motor * CARTRIDGE_TRAVEL } else { ms.heads_mm[i] };
            if spacing > 0 && i > 0 {
                let k = i as f32;
                let f = match spacing { 1 => k + 1.0, 2 => 1.0 + 2.0 / 3.0 * k, 3 => 1.0 + 1.618 * k, _ => 1.0 + 2.414 * k };
                mm += morph * (ms.heads_mm[0] * f - mm);
            }
            heads.push(PictureHead {
                mm: mm.clamp(0.1, ms.loop_mm - ms.gap_mm - 1.0),
                ms: 0.0,
                pan: if count == 1 { 0.0 } else { pattern[i] },
                gain: 1.0,
                on: (mask >> i) & 1 == 1,
                fb: (fb_mask >> i) & 1 == 1,
            });
        }
    }

    // Sync: the first audible left / right heads land on their note values.
    let (mut scale_l, mut scale_r) = (1.0f32, 1.0f32);
    if v("sync") >= 0.5 {
        let beat = 60.0 / bpm.clamp(20.0, 999.0) as f32;
        let note_s = |id: &str| (note_beats(v(id).round().clamp(0.0, 20.0) as usize) * beat).min(MAX_SECONDS - 0.1);
        let (tl, tr) = (note_s("note_l"), note_s("note_r"));
        let first = |right: bool| heads.iter().filter(|h| h.on && (h.pan > 0.0) == right).map(|h| h.mm).fold(None, |a: Option<f32>, m| Some(a.map_or(m, |a| a.min(m))));
        let (fl, fr) = (first(false), first(true));
        if let Some((fl, fr)) = fl.or(fr).zip(fr.or(fl)) {
            if machine != 2 {
                speed = (fl / tl.max(1e-3)).clamp(5.0, 5000.0);
            }
            scale_l = tl / (fl / speed).max(1e-4);
            scale_r = tr / (fr / speed).max(1e-4);
        }
    }
    speed = speed.clamp(5.0, 5000.0);
    for h in &mut heads {
        h.ms = h.mm / speed * if h.pan > 0.0 { scale_r } else { scale_l } * 1000.0;
    }

    let wire = if machine == 1 { (-2.2 * (speed / ms.default_speed).ln().abs()).exp() } else { 1.0 };
    let loop_gain = 10f32.powf((v("intensity").clamp(0.0, 10.0) - ms.onset) * 3.0 / 20.0) * if machine == 1 { 0.5 + 0.5 * wire } else { 1.0 };
    let w = wear(v("generations").clamp(0.0, 1.0), ms.detent);
    let hf_loss = (-2.0 * std::f32::consts::PI * 2000.0 / ms.default_speed * ms.spacing_um * (w - 1.0) * 0.001).exp();
    let feeding = heads.iter().filter(|h| h.fb).count().max(1) as f32;
    Picture { machine, speed, heads, pass_gain: loop_gain * hf_loss / feeding }
}

/// Parameter writes that turn the current heads into authored (user-pinned) heads with
/// head `dragged` (index into `pic.heads`) moved to `ms` (the reference's head drag).
pub fn authored_patch(pic: &Picture, dragged: usize, ms: f32) -> Vec<(&'static str, f32)> {
    let spec = MACHINE_SPECS[pic.machine as usize];
    let span = (spec.loop_mm - spec.gap_mm - 1.1).max(1.0);
    let mut out = vec![("heads", pic.heads.len().clamp(1, 6) as f32), ("heads_machine", (pic.machine + 1) as f32)];
    for (i, h) in pic.heads.iter().enumerate().take(6) {
        let mm = if i == dragged { (ms / 1000.0 * pic.speed).clamp(0.1, 0.1 + span) } else { h.mm };
        let level = if h.gain <= 0.0 { 0.0 } else { (0.01 + 0.99 * (20.0 * h.gain.log10() + 60.0) / 66.0).clamp(0.0, 1.0) };
        let fields: [(&str, f32); 9] = [
            ("present", 1.0),
            ("on", h.on as u8 as f32),
            ("fb", h.fb as u8 as f32),
            ("right", (h.pan > 0.0) as u8 as f32),
            ("pos", ((mm - 0.1) / span).clamp(0.0, 1.0)),
            ("level", level),
            ("pan", h.pan),
            ("pan_pinned", 1.0),
            ("pos_pinned", (i == dragged) as u8 as f32),
        ];
        out.extend(fields.iter().filter_map(|(f, x)| head_param(i + 1, f).map(|id| (id, *x))));
    }
    out
}

fn motor_text(def: &DeviceDef, d: &Device, p: &ParamDef, v: f32) -> Option<String> {
    if p.id != "motor" {
        return None;
    }
    let machine = def.param("machine").map_or(0.0, |m| def.value(d, m)).round() as u32;
    let mains = def.param("drum_mains_60").is_some_and(|m| def.value(d, m) >= 0.5);
    Some(match machine {
        2 => {
            let ms = cartridge_ms(v);
            if ms < 100.0 { format!("{ms:.1} ms") } else { format!("{ms:.0} ms") }
        }
        m => format!("{:.1} cm/s", speed(m, v, mains) / 10.0),
    })
}

/// Motor from typed text in the unit `motor_text` shows: ms (Cartridge) or cm/s (Tape Loop, Drum).
fn motor_read(def: &DeviceDef, d: &Device, p: &ParamDef, text: &str) -> Option<f32> {
    if p.id != "motor" {
        return None;
    }
    let machine = def.param("machine").map_or(0.0, |m| def.value(d, m)).round() as u32;
    let mains = def.param("drum_mains_60").is_some_and(|m| def.value(d, m) >= 0.5);
    let x: f32 = text.trim_end_matches(if machine == 2 { "ms" } else { "cm/s" }).trim().parse().ok()?;
    Some(match machine {
        2 => (CARTRIDGE.heads_mm[0] - x / 1000.0 * CARTRIDGE_SPEED) / CARTRIDGE_TRAVEL,
        0 => (x * 10.0 - 120.0) / 280.0,
        _ => x * 10.0 / (DRUM_SPEED * if mains { 1.2 } else { 1.0 }) - 0.5,
    })
}

fn generation_text(v: f32) -> String {
    format!("{}%", (v * 100.0).round() as i32)
}

pub(super) static BEHAVIOR: Behavior = Behavior {
    text: |def, d, p, v| motor_text(def, d, p, v),
    read: |def, d, p, t| if p.id == "generations" { generation_read(t) } else { motor_read(def, d, p, t) },
    ..Behavior::NONE
};

fn generation_read(text: &str) -> Option<f32> {
    text.trim_end_matches('%').trim().parse::<f32>().ok().map(|v| v / 100.0)
}

const HZ_SKEW: Taper = Taper::Linear { skew: 0.3 };
const MS_SKEW: Taper = Taper::Linear { skew: 0.4 };

pub static ECHO: DeviceDef = DeviceDef {
    id: "echo",
    name: "Echo",
    kind: PluginKind::Effect,
    category: "Delay",
    purpose: "A vintage multi-head echo: a tape loop, a magnetic drum or a cartridge, with worn-tape artefacts, a spring, ducking and a throw switch.",
    params: &[
        ParamDef::choice("machine", "Machine", MACHINES, 0).help("Which echo machine: tape loop, magnetic drum or cartridge."),
        ParamDef::choice("mode_loop", "Tape Loop Mode", LOOP_MODES, 3).label("Mode").help("Heads (and spring) the tape loop plays."),
        ParamDef::choice("mode_drum", "Drum Selector", DRUM_MODES, 1).help("Echo: single pass; Repeat: the selected heads feed back; Swell: every head feeds back."),
        ParamDef::choice("mode_cartridge", "Cartridge Mode", CARTRIDGE_MODES, 0).label("Mode").help("Echo, or Sound-on-Sound (layers keep building up)."),
        ParamDef::choice("combo_drum", "Drum Switch", DRUM_SWITCH, 11).label("Mode").help("Heads the drum plays."),
        ParamDef::new("motor", "Motor", 0.0, 1.0, 0.5, LINEAR, "", Format::Plain(2)).help("Tape speed (Tape Loop, Drum) or record-head position (Cartridge): sets the echo times."),
        ParamDef::choice("motor_glide", "Motor Glide", OFF_ON, 1).help("The motor speeds up and slows down with inertia (pitch glides)."),
        ParamDef::choice("sync", "Sync", OFF_ON, 0).help("Retune the motor so the first left and right heads land on note values."),
        ParamDef::choice("note_l", "Note L", NOTES, 10).help("Note value of the first left head while synced."),
        ParamDef::choice("note_r", "Note R", NOTES, 12).help("Note value of the first right head while synced."),
        ParamDef::new("intensity", "Intensity", 0.0, 10.0, 5.0, LINEAR, "", Format::Plain(1)).help("Feedback: above about 7.5 the echo runs away into self-oscillation."),
        ParamDef::new("mix", "Mix", 0.0, 100.0, 35.0, LINEAR, "%", Format::Percent).help("Level of the echoes against the dry signal."),
        ParamDef::choice("mix_lock", "Mix Lock", OFF_ON, 0).locks(&["mix"]).help("Keep Mix when loading presets."),
        ParamDef::choice("kill_dry", "Kill Dry", OFF_ON, 0).help("Remove the dry signal (for sends)."),
        ParamDef::new("trim", "Trim", -24.0, 12.0, 0.0, LINEAR, "dB", Format::Db).help("Output level of the echoes."),
        ParamDef::new("input_drive", "Input Drive", 0.0, 10.0, 5.0, LINEAR, "", Format::Plain(1)).help("Input amplifier drive."),
        ParamDef::new("rec_level", "Rec Level", 0.0, 10.0, 5.0, LINEAR, "", Format::Plain(1)).help("Recording level onto the tape (saturation)."),
        ParamDef::choice("spacing", "Spacing", SPACINGS, 0).help("Head spacing pattern."),
        ParamDef::new("spacing_morph", "Spacing Morph", 0.0, 1.0, 1.0, LINEAR, "", Format::Plain(2)).help("Blend from the machine's head spacing to the chosen pattern."),
        ParamDef::new("tone_bass", "Bass", -10.0, 10.0, 0.0, LINEAR, "dB", Format::Db).help("Bass of the echo path."),
        ParamDef::new("tone_treble", "Treble", -10.0, 10.0, 0.0, LINEAR, "dB", Format::Db).help("Treble of the echo path."),
        ParamDef::new("width", "Width", 0.0, 200.0, 100.0, LINEAR, "%", Format::Percent).help("Stereo spread of the heads."),
        ParamDef::new("width_humanize", "Width Humanize", 0.0, 100.0, 45.0, LINEAR, "%", Format::Percent).help("Irregularity of the head pan pattern."),
        ParamDef::new("generations", "Generations", 0.0, 1.0, 0.45, LINEAR, "", Format::Custom(TextFn(generation_text)))
            .label("Generation")
            .help("How much each pass wears the sound: studio-fresh to dying tape."),
        ParamDef::choice("spring_on", "Spring On", OFF_ON, 0).help("Spring reverb on the echoes."),
        ParamDef::new("spring", "Spring", 0.0, 100.0, 25.0, LINEAR, "%", Format::Percent).help("Spring reverb amount."),
        ParamDef::new("art_hiss", "Hiss", 0.0, 100.0, 100.0, LINEAR, "%", Format::Percent).help("Tape hiss."),
        ParamDef::new("art_wf", "Wow & Flutter", 0.0, 100.0, 100.0, LINEAR, "%", Format::Percent).help("Speed wobble of the transport."),
        ParamDef::new("art_splice", "Splice", 0.0, 100.0, 100.0, LINEAR, "%", Format::Percent).help("Dropout where the loop is spliced."),
        ParamDef::new("art_bias", "Bias", -100.0, 100.0, 0.0, LINEAR, "%", Format::Percent).help("Under-bias (gritty, bright) to over-bias (soft, dull)."),
        ParamDef::new("art_scrape", "Scrape", 0.0, 100.0, 100.0, LINEAR, "%", Format::Percent).help("Scrape flutter: fast rough modulation of the treble."),
        ParamDef::new("swell", "Smear", 0.0, 100.0, 0.0, LINEAR, "%", Format::Percent).help("Smears each repeat in time, more with every pass."),
        ParamDef::choice("tail_on", "Tail On", OFF_ON, 1).help("Tail filters on."),
        ParamDef::new("tail_hp_l", "Tail HP L", 20.0, 2000.0, 20.0, HZ_SKEW, "Hz", Format::Hz).help("High-pass of the left echoes."),
        ParamDef::new("tail_lp_l", "Tail LP L", 200.0, 20000.0, 6200.0, HZ_SKEW, "Hz", Format::Hz).help("Low-pass of the left echoes."),
        ParamDef::new("tail_hp_r", "Tail HP R", 20.0, 2000.0, 240.0, HZ_SKEW, "Hz", Format::Hz).help("High-pass of the right echoes."),
        ParamDef::new("tail_lp_r", "Tail LP R", 200.0, 20000.0, 20000.0, HZ_SKEW, "Hz", Format::Hz).help("Low-pass of the right echoes."),
        ParamDef::choice("duck_on", "Duck On", OFF_ON, 0).help("Lower the echoes while the input (or the sidechain) is loud."),
        ParamDef::new("duck_amount", "Duck Amount", 0.0, 100.0, 60.0, LINEAR, "%", Format::Percent).help("How far the echoes are lowered."),
        ParamDef::new("duck_thresh", "Duck Threshold", -60.0, 0.0, -24.0, LINEAR, "dB", Format::Db).help("Key level where ducking starts."),
        ParamDef::new("duck_attack", "Duck Attack", 0.5, 100.0, 5.0, MS_SKEW, "ms", Format::Time).help("How fast ducking sets in."),
        ParamDef::new("duck_release", "Duck Release", 20.0, 1000.0, 180.0, MS_SKEW, "ms", Format::Time).help("How fast the echoes come back."),
        ParamDef::choice("duck_key", "Duck Key", KEYS, 0).help("Duck from the input, or from the sidechain track."),
        ParamDef::choice("duck_fb_only", "Duck FB Only", OFF_ON, 0).help("Duck only the feedback, so the first echo stays."),
        ParamDef::choice("throw", "Throw", OFF_ON, 0).momentary().help("While held, sends the input into the echo at Throw Level."),
        ParamDef::new("throw_level", "Throw Level", -24.0, 12.0, 6.0, LINEAR, "dB", Format::Db).help("Send boost while Throw is held."),
        ParamDef::new("seed", "Seed", 0.0, 65535.0, 20673.0, LINEAR, "", Format::Plain(0)).step(1.0).internal(),
        ParamDef::choice("drum_mains_60", "Drum Mains 60Hz", OFF_ON, 0).internal(),
        ParamDef::choice("cartridge_late", "Cartridge Late", OFF_ON, 0).internal(),
        ParamDef::choice("heads", "Heads", HEADS, 0).internal(),
        ParamDef::choice("heads_machine", "Heads Machine", HEADS_MACHINE, 0).internal(),
        ParamDef::choice("h1_present", "H1 Present", OFF_ON, 0).internal(),
        ParamDef::choice("h1_on", "H1 On", OFF_ON, 0).internal(),
        ParamDef::choice("h1_fb", "H1 Feedback", OFF_ON, 1).internal(),
        ParamDef::choice("h1_right", "H1 Right", OFF_ON, 0).internal(),
        ParamDef::new("h1_pos", "H1 Position", 0.0, 1.0, 0.0, LINEAR, "", Format::Plain(3)).internal(),
        ParamDef::new("h1_level", "H1 Level", 0.0, 1.0, 0.0, LINEAR, "", Format::Plain(2)).internal(),
        ParamDef::new("h1_pan", "H1 Pan", -1.0, 1.0, 0.0, LINEAR, "", Format::Plain(2)).internal(),
        ParamDef::new("h1_tilt", "H1 Tilt", -1.0, 1.0, 0.0, LINEAR, "", Format::Plain(2)).internal(),
        ParamDef::choice("h1_pan_pinned", "H1 Pan Pinned", OFF_ON, 0).internal(),
        ParamDef::choice("h1_pos_pinned", "H1 Position Pinned", OFF_ON, 1).internal(),
        ParamDef::new("h1_level_before_mute", "H1 Level Before Mute", 0.0, 1.0, 0.0, LINEAR, "", Format::Plain(2)).internal(),
        ParamDef::choice("h2_present", "H2 Present", OFF_ON, 0).internal(),
        ParamDef::choice("h2_on", "H2 On", OFF_ON, 0).internal(),
        ParamDef::choice("h2_fb", "H2 Feedback", OFF_ON, 1).internal(),
        ParamDef::choice("h2_right", "H2 Right", OFF_ON, 0).internal(),
        ParamDef::new("h2_pos", "H2 Position", 0.0, 1.0, 0.0, LINEAR, "", Format::Plain(3)).internal(),
        ParamDef::new("h2_level", "H2 Level", 0.0, 1.0, 0.0, LINEAR, "", Format::Plain(2)).internal(),
        ParamDef::new("h2_pan", "H2 Pan", -1.0, 1.0, 0.0, LINEAR, "", Format::Plain(2)).internal(),
        ParamDef::new("h2_tilt", "H2 Tilt", -1.0, 1.0, 0.0, LINEAR, "", Format::Plain(2)).internal(),
        ParamDef::choice("h2_pan_pinned", "H2 Pan Pinned", OFF_ON, 0).internal(),
        ParamDef::choice("h2_pos_pinned", "H2 Position Pinned", OFF_ON, 1).internal(),
        ParamDef::new("h2_level_before_mute", "H2 Level Before Mute", 0.0, 1.0, 0.0, LINEAR, "", Format::Plain(2)).internal(),
        ParamDef::choice("h3_present", "H3 Present", OFF_ON, 0).internal(),
        ParamDef::choice("h3_on", "H3 On", OFF_ON, 0).internal(),
        ParamDef::choice("h3_fb", "H3 Feedback", OFF_ON, 1).internal(),
        ParamDef::choice("h3_right", "H3 Right", OFF_ON, 0).internal(),
        ParamDef::new("h3_pos", "H3 Position", 0.0, 1.0, 0.0, LINEAR, "", Format::Plain(3)).internal(),
        ParamDef::new("h3_level", "H3 Level", 0.0, 1.0, 0.0, LINEAR, "", Format::Plain(2)).internal(),
        ParamDef::new("h3_pan", "H3 Pan", -1.0, 1.0, 0.0, LINEAR, "", Format::Plain(2)).internal(),
        ParamDef::new("h3_tilt", "H3 Tilt", -1.0, 1.0, 0.0, LINEAR, "", Format::Plain(2)).internal(),
        ParamDef::choice("h3_pan_pinned", "H3 Pan Pinned", OFF_ON, 0).internal(),
        ParamDef::choice("h3_pos_pinned", "H3 Position Pinned", OFF_ON, 1).internal(),
        ParamDef::new("h3_level_before_mute", "H3 Level Before Mute", 0.0, 1.0, 0.0, LINEAR, "", Format::Plain(2)).internal(),
        ParamDef::choice("h4_present", "H4 Present", OFF_ON, 0).internal(),
        ParamDef::choice("h4_on", "H4 On", OFF_ON, 0).internal(),
        ParamDef::choice("h4_fb", "H4 Feedback", OFF_ON, 1).internal(),
        ParamDef::choice("h4_right", "H4 Right", OFF_ON, 0).internal(),
        ParamDef::new("h4_pos", "H4 Position", 0.0, 1.0, 0.0, LINEAR, "", Format::Plain(3)).internal(),
        ParamDef::new("h4_level", "H4 Level", 0.0, 1.0, 0.0, LINEAR, "", Format::Plain(2)).internal(),
        ParamDef::new("h4_pan", "H4 Pan", -1.0, 1.0, 0.0, LINEAR, "", Format::Plain(2)).internal(),
        ParamDef::new("h4_tilt", "H4 Tilt", -1.0, 1.0, 0.0, LINEAR, "", Format::Plain(2)).internal(),
        ParamDef::choice("h4_pan_pinned", "H4 Pan Pinned", OFF_ON, 0).internal(),
        ParamDef::choice("h4_pos_pinned", "H4 Position Pinned", OFF_ON, 1).internal(),
        ParamDef::new("h4_level_before_mute", "H4 Level Before Mute", 0.0, 1.0, 0.0, LINEAR, "", Format::Plain(2)).internal(),
        ParamDef::choice("h5_present", "H5 Present", OFF_ON, 0).internal(),
        ParamDef::choice("h5_on", "H5 On", OFF_ON, 0).internal(),
        ParamDef::choice("h5_fb", "H5 Feedback", OFF_ON, 1).internal(),
        ParamDef::choice("h5_right", "H5 Right", OFF_ON, 0).internal(),
        ParamDef::new("h5_pos", "H5 Position", 0.0, 1.0, 0.0, LINEAR, "", Format::Plain(3)).internal(),
        ParamDef::new("h5_level", "H5 Level", 0.0, 1.0, 0.0, LINEAR, "", Format::Plain(2)).internal(),
        ParamDef::new("h5_pan", "H5 Pan", -1.0, 1.0, 0.0, LINEAR, "", Format::Plain(2)).internal(),
        ParamDef::new("h5_tilt", "H5 Tilt", -1.0, 1.0, 0.0, LINEAR, "", Format::Plain(2)).internal(),
        ParamDef::choice("h5_pan_pinned", "H5 Pan Pinned", OFF_ON, 0).internal(),
        ParamDef::choice("h5_pos_pinned", "H5 Position Pinned", OFF_ON, 1).internal(),
        ParamDef::new("h5_level_before_mute", "H5 Level Before Mute", 0.0, 1.0, 0.0, LINEAR, "", Format::Plain(2)).internal(),
        ParamDef::choice("h6_present", "H6 Present", OFF_ON, 0).internal(),
        ParamDef::choice("h6_on", "H6 On", OFF_ON, 0).internal(),
        ParamDef::choice("h6_fb", "H6 Feedback", OFF_ON, 1).internal(),
        ParamDef::choice("h6_right", "H6 Right", OFF_ON, 0).internal(),
        ParamDef::new("h6_pos", "H6 Position", 0.0, 1.0, 0.0, LINEAR, "", Format::Plain(3)).internal(),
        ParamDef::new("h6_level", "H6 Level", 0.0, 1.0, 0.0, LINEAR, "", Format::Plain(2)).internal(),
        ParamDef::new("h6_pan", "H6 Pan", -1.0, 1.0, 0.0, LINEAR, "", Format::Plain(2)).internal(),
        ParamDef::new("h6_tilt", "H6 Tilt", -1.0, 1.0, 0.0, LINEAR, "", Format::Plain(2)).internal(),
        ParamDef::choice("h6_pan_pinned", "H6 Pan Pinned", OFF_ON, 0).internal(),
        ParamDef::choice("h6_pos_pinned", "H6 Position Pinned", OFF_ON, 1).internal(),
        ParamDef::new("h6_level_before_mute", "H6 Level Before Mute", 0.0, 1.0, 0.0, LINEAR, "", Format::Plain(2)).internal(),
        ParamDef::new("patch", "Patch", 0.0, 1.0, 0.0, LINEAR, "", Format::Plain(2)).internal(),
    ],
    layout: Layout {
        picker: &[Control::Dropdown("machine"), Control::Dropdown("mode_loop"), Control::Dropdown("combo_drum"), Control::Dropdown("mode_cartridge"), Control::Sidechain],
        view: Some(View::Custom("echo")),
        clusters: &[
            &[Control::Knob("motor"), Control::Knob("intensity"), Control::Knob("generations"), Control::Knob("swell"), Control::Toggle("throw")],
            &[Control::Knob("width"), Control::Knob("mix")],
        ],
        min_height: 220.0,
        shown_when: &[("mode_loop", "machine", &[0]), ("combo_drum", "machine", &[1]), ("mode_cartridge", "machine", &[2])],
        spread_dials: true,
        ..Layout::EMPTY
    },
    factory: &[
        FactoryPreset { name: "Vox Slap Cart", values: &[("machine", 2.0), ("motor", 0.78), ("intensity", 1.8), ("mix", 18.0), ("duck_on", 1.0), ("duck_amount", 50.0)] },
        FactoryPreset { name: "Vox Quarter Loop", values: &[("machine", 0.0), ("sync", 1.0), ("note_l", 10.0), ("note_r", 12.0), ("intensity", 4.6), ("mix", 22.0), ("duck_on", 1.0), ("tail_lp_l", 4500.0), ("tail_lp_r", 4500.0)] },
        FactoryPreset { name: "Ducked Lead Wash", values: &[("machine", 0.0), ("intensity", 5.8), ("mix", 28.0), ("duck_on", 1.0), ("duck_amount", 72.0), ("duck_release", 300.0), ("tail_lp_l", 3800.0), ("tail_lp_r", 3800.0)] },
        FactoryPreset { name: "Drum Room Two Heads", values: &[("machine", 1.0), ("mode_drum", 0.0), ("combo_drum", 4.0), ("intensity", 3.4), ("mix", 20.0), ("tail_hp_l", 220.0), ("tail_hp_r", 220.0)] },
        FactoryPreset { name: "Swelling Drum", values: &[("machine", 1.0), ("mode_drum", 2.0), ("intensity", 6.2), ("swell", 55.0), ("mix", 38.0)] },
        FactoryPreset { name: "Wire Spring Drum", values: &[("machine", 1.0), ("mode_drum", 0.0), ("combo_drum", 0.0), ("intensity", 2.5), ("spring_on", 1.0), ("spring", 45.0), ("mix", 20.0)] },
        FactoryPreset { name: "Four-Head Gallop", values: &[("machine", 1.0), ("combo_drum", 9.0), ("intensity", 4.4), ("mix", 26.0), ("width", 150.0), ("sync", 1.0), ("note_l", 13.0), ("note_r", 13.0)] },
        FactoryPreset { name: "Bias Burn Drum", values: &[("machine", 1.0), ("mode_drum", 0.0), ("art_bias", -60.0), ("intensity", 7.0), ("mix", 34.0)] },
        FactoryPreset { name: "Loop Single Head Short", values: &[("machine", 0.0), ("mode_loop", 0.0), ("motor", 0.95), ("intensity", 1.6), ("mix", 24.0)] },
        FactoryPreset { name: "Loop Far Head", values: &[("machine", 0.0), ("mode_loop", 2.0), ("intensity", 4.0), ("mix", 28.0)] },
        FactoryPreset { name: "Loop Spring Bath", values: &[("machine", 0.0), ("mode_loop", 11.0), ("spring_on", 1.0), ("spring", 65.0), ("mix", 40.0), ("trim", 2.0)] },
        FactoryPreset { name: "Siren Feedback", values: &[("machine", 0.0), ("mode_loop", 10.0), ("spring_on", 1.0), ("intensity", 7.8), ("tone_bass", 3.0), ("mix", 38.0)] },
        FactoryPreset { name: "Slow Motor Bloom", values: &[("machine", 0.0), ("mode_loop", 2.0), ("motor", 0.15), ("generations", 0.62), ("intensity", 6.2), ("mix", 40.0), ("trim", 1.5)] },
        FactoryPreset { name: "Sound-on-Sound Layers", values: &[("machine", 2.0), ("mode_cartridge", 1.0), ("generations", 0.65), ("intensity", 7.0), ("mix", 42.0), ("trim", 2.0)] },
        FactoryPreset { name: "Cart Long Slide", values: &[("machine", 2.0), ("motor", 0.05), ("intensity", 4.0), ("mix", 30.0)] },
        FactoryPreset { name: "Cart Warble", values: &[("machine", 2.0), ("motor", 0.4), ("intensity", 5.2), ("art_wf", 100.0), ("width", 140.0), ("mix", 34.0)] },
        FactoryPreset { name: "Fresh Studio Tape", values: &[("machine", 1.0), ("mode_drum", 0.0), ("combo_drum", 5.0), ("generations", 0.0), ("art_hiss", 15.0), ("art_wf", 20.0), ("intensity", 4.2), ("mix", 28.0)] },
        FactoryPreset { name: "Dying Tape", values: &[("machine", 2.0), ("generations", 1.0), ("motor", 0.35), ("intensity", 6.0), ("mix", 40.0), ("trim", 2.0)] },
        FactoryPreset { name: "Throw Catcher", values: &[("machine", 0.0), ("intensity", 6.4), ("mix", 36.0), ("duck_on", 1.0), ("duck_fb_only", 1.0), ("throw_level", 4.0)] },
        FactoryPreset { name: "Filtered Ping", values: &[("machine", 0.0), ("sync", 1.0), ("note_r", 13.0), ("intensity", 4.8), ("width", 170.0), ("tail_hp_l", 160.0), ("tail_hp_r", 300.0), ("tail_lp_l", 3000.0), ("tail_lp_r", 3600.0), ("mix", 30.0)] },
        FactoryPreset { name: "Golden Field", values: &[("machine", 0.0), ("mode_loop", 10.0), ("spacing", 3.0), ("intensity", 3.2), ("width", 135.0), ("mix", 36.0)] },
        FactoryPreset { name: "Silver Steps", values: &[("machine", 0.0), ("mode_loop", 10.0), ("spacing", 4.0), ("spacing_morph", 0.7), ("generations", 0.5), ("intensity", 3.0), ("mix", 38.0)] },
        FactoryPreset { name: "Triplet Cascade", values: &[("machine", 1.0), ("mode_drum", 0.0), ("combo_drum", 9.0), ("spacing", 2.0), ("intensity", 3.5), ("width_humanize", 70.0), ("mix", 34.0)] },
        FactoryPreset { name: "Send: Tape Loop", values: &[("machine", 0.0), ("intensity", 5.6), ("mix", 100.0), ("kill_dry", 1.0), ("tail_lp_l", 5000.0), ("tail_lp_r", 5000.0)] },
    ],
    sidechain: true,
    samples: 0,
};
