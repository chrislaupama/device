//! Custom bodies of built-in device cards (View::Custom). Each arm turns a
//! device's current values into a PanelCanvas (path layers, texts, grip pills,
//! drag zones) and, for grid bodies, freely placed knobs and buttons. The
//! drawings follow the reference cards' field painters (device specs in the
//! Captain drive); the curves come from the drawn models in saine_core.
//!
//!   "wave"          shaper wave field: Distortion, Tape Saturation, Soft Clipper
//!   "crush"         sample-and-hold picture: Bit Crusher
//!   "stereo-field"  stereo image + width-over-frequency plot: Stereo Widener
//!   "utility"       Input | Output grid: Utility
//!
//! Lengths are reference 100%-zoom values; everything handed to Slint is
//! multiplied by Z.

use crate::controller::Controller;
use crate::device_panel::{CAPTION_H, Z};
use crate::{PanelButton, PanelCanvas, PanelGrip, PanelPath, PanelText, PanelZone};
use saine_core::builtin::{self, DeviceDef};
use saine_core::Device;
use slint::{Color, ModelRc, SharedString, VecModel};
use std::f32::consts::TAU;
use std::fmt::Write;
use std::rc::Rc;

const MINUS: char = '\u{2212}';
/// Sample rate the drawn responses assume.
const DRAW_RATE: f32 = 48_000.0;

/// What a custom body adds to the generic card.
#[derive(Default)]
pub(crate) struct CustomBody {
    pub canvas: PanelCanvas,
    pub buttons: Vec<PanelButton>,
    /// Knob placement (param, x, y; card relative, 100%) replacing the dial row.
    pub knobs: Vec<(&'static str, f32, f32)>,
    /// Knobs drawn dimmed (no effect in the current state).
    pub dim: Vec<&'static str>,
    /// Knob cell width (pt, 0 = the 62 pt default).
    pub cell: f32,
}

fn model<T: Clone + 'static>(v: Vec<T>) -> ModelRc<T> {
    ModelRc::from(Rc::new(VecModel::from(v)))
}

fn ink(alpha: f32) -> Color {
    Color::from_argb_u8((alpha.clamp(0.0, 1.0) * 255.0).round() as u8, 255, 255, 255)
}

fn path(commands: String, stroke: Color, width: f32, fill: Color) -> PanelPath {
    PanelPath { commands: commands.into(), stroke, width: width * Z, fill, fill2: fill }
}

fn stroke(commands: String, color: Color, width: f32) -> PanelPath {
    path(commands, color, width, Color::from_argb_u8(0, 0, 0, 0))
}

fn fill(commands: String, color: Color) -> PanelPath {
    path(commands, Color::from_argb_u8(0, 0, 0, 0), 0.0, color)
}

fn text(x: f32, y: f32, w: f32, t: impl Into<SharedString>, color: Color, size: f32, weight: i32, align: i32) -> PanelText {
    PanelText { x, y: y - 7.0 * Z, w, h: 14.0 * Z, text: t.into(), color, size: size * Z, weight, align }
}

/// "M x y L x y ..." (coordinates already scaled).
pub(crate) fn poly(pts: &[(f32, f32)]) -> String {
    let mut out = String::with_capacity(pts.len() * 16);
    for (i, (x, y)) in pts.iter().enumerate() {
        let _ = write!(out, "{}{x:.2} {y:.2}", if i == 0 { "M" } else { " L" });
    }
    out
}

/// A polyline drawn as dashes of \`dash\` / \`gap\` length along its arc length.
pub(crate) fn dashed(pts: &[(f32, f32)], dash: f32, gap: f32) -> String {
    let mut out = String::new();
    let mut on = true;
    let mut left = dash;
    for w in pts.windows(2) {
        let (a, b) = (w[0], w[1]);
        let len = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
        let mut t0 = 0.0;
        while t0 < len {
            let step = left.min(len - t0);
            let p0 = (a.0 + (b.0 - a.0) * t0 / len, a.1 + (b.1 - a.1) * t0 / len);
            let t1 = t0 + step;
            let p1 = (a.0 + (b.0 - a.0) * t1 / len, a.1 + (b.1 - a.1) * t1 / len);
            if on {
                let _ = write!(out, "M{:.2} {:.2} L{:.2} {:.2} ", p0.0, p0.1, p1.0, p1.1);
            }
            left -= step;
            t0 = t1;
            if left <= 1e-4 {
                on = !on;
                left = if on { dash } else { gap };
            }
        }
    }
    out
}

fn ellipse(cx: f32, cy: f32, rx: f32, ry: f32) -> String {
    format!("M{:.2} {cy:.2} A{rx:.2} {ry:.2} 0 1 0 {:.2} {cy:.2} A{rx:.2} {ry:.2} 0 1 0 {:.2} {cy:.2} Z", cx - rx, cx + rx, cx - rx)
}

fn circle_pts(cx: f32, cy: f32, r: f32) -> Vec<(f32, f32)> {
    (0..=96).map(|i| {
        let a = TAU * i as f32 / 96.0;
        (cx + r * a.cos(), cy + r * a.sin())
    }).collect()
}

/// Harmonic distortion of a transfer curve (the reference's "Harmonics"
/// readout): one cycle of a full-scale sine, 256 points, harmonics 2..24
/// against the fundamental, as a percentage.
pub(crate) fn harmonics_percent(f: &dyn Fn(f32) -> f32) -> f32 {
    const N: usize = 256;
    let y: Vec<f32> = (0..N).map(|i| f((TAU * i as f32 / N as f32).sin())).collect();
    let power = |k: usize| {
        let (mut re, mut im) = (0.0f32, 0.0f32);
        for (i, v) in y.iter().enumerate() {
            let ph = TAU * ((k * i) % N) as f32 / N as f32;
            re += v * ph.cos();
            im -= v * ph.sin();
        }
        re * re + im * im
    };
    let fund = power(1);
    if fund <= 1e-12 {
        return 0.0;
    }
    100.0 * ((2..=24).map(power).sum::<f32>() / fund).sqrt()
}

pub(crate) fn fmt_percent_readout(p: f32) -> String {
    if p < 9.95 { format!("{p:.1}%") } else { format!("{p:.0}%") }
}

/// "Clipping" readout: "0.0 dB" under 0.05 dB, else "−N.N dB".
pub(crate) fn fmt_clipping(db: f32) -> String {
    let a = (db.abs() * 10.0).round() / 10.0;
    if a < 0.05 { "0.0 dB".into() } else if db < 0.0 { format!("{MINUS}{a:.1} dB") } else { format!("{a:.1} dB") }
}

impl Controller {
    /// Body of a View::Custom card. \`rect\` = the view area (x, y, w, h; card
    /// relative, 100%): left of the slider column, from the view row top to
    /// the bottom of the well.
    pub(crate) fn custom_body(&self, d: &Device, def: &'static DeviceDef, name: &str, rect: (f32, f32, f32, f32)) -> CustomBody {
        let value = |id: &str| def.param(id).map_or(0.0, |p| self.panel_value(d, def, p));
        let accent = self
            .project()
            .find_device(d.id)
            .map_or(Color::from_rgb_u8(0x43, 0x9b, 0x94), |(ti, _)| crate::controller::slint_color(crate::controller::track_color(self.project().tracks[ti].color)));
        let idx = |id: &str| def.index_of(id).map_or(-1, |i| i as i32);
        match name {
            "wave" => wave_body(def, &value, idx(def.params[0].id), accent, rect),
            "crush" => crush_body(&value, idx("downsample"), idx("bitdepth"), accent, rect),
            "stereo-field" => stereo_body(def, &value, idx("width2"), idx("sidegain"), idx("lowcut"), accent, rect),
            "utility" => utility_body(def, &value, &idx),
            "pitch-trace" | "tuner" | "spectrum" => self.analyzer_body(d, def, name, accent),
            "gain-scope" => scope_body(&self.panel_meter_frames(d.id), crate::device_panel::HISTORY, accent, rect),
            "declipper-wave" | "deesser-graph" | "dehummer-graph" | "denoiser-graph" => CustomBody { canvas: self.graph_canvas(d, def, accent), ..Default::default() },
            _ => CustomBody::default(),
        }
    }
}

/// Field frame: caption row then the well; returns the field and the well size (scaled).
fn frame(rect: (f32, f32, f32, f32), caption: &str, readout: String) -> (PanelCanvas, f32, f32) {
    let (x, y, w, h) = rect;
    let field = PanelCanvas {
        visible: true,
        well: true,
        x: x * Z,
        y: y * Z,
        w: w * Z,
        h: h * Z,
        well_y: CAPTION_H * Z,
        caption: caption.into(),
        readout: readout.into(),
        ..Default::default()
    };
    (field, w * Z, (h - CAPTION_H) * Z)
}

fn wave_body(def: &DeviceDef, value: &dyn Fn(&str) -> f32, drag_param: i32, accent: Color, rect: (f32, f32, f32, f32)) -> CustomBody {
    let v = value;
    let (curve, reference, caption, fill_alpha, hiss): (Box<dyn Fn(f32) -> f32>, Box<dyn Fn(f32) -> f32>, &str, f32, f32) = match def.id {
        "tapesaturation" => {
            let (dr, cp, mx, out, mode) = (v("drive"), v("compression"), v("mix"), v("output_gain"), v("mode").round() as u32);
            let warmth = v("warmth") / 100.0;
            let hiss = (v("noise") + 60.0) / 60.0;
            (Box::new(move |x| builtin::tape_transfer(x, dr, cp, mx, out, mode)), Box::new(|x| x), "Harmonics", 0.18 + 0.2 * warmth, hiss)
        }
        "soft-clipper" => {
            let (inp, ceil, soft, shape, gm, mx, out) = (v("input"), v("ceiling"), v("softness"), v("shape").round() as u32, v("gain_match") >= 0.5, v("mix"), v("output_gain"));
            (
                Box::new(move |x| builtin::clipper_transfer(x, inp, ceil, soft, shape, gm, mx, out)),
                Box::new(move |x| builtin::clipper_open(x, inp, gm, mx, out)),
                "Clipping",
                0.18,
                0.0,
            )
        }
        _ => {
            let (dr, bias, gain, mx, out, mode) = (v("drive"), v("bias"), v("output"), v("mix"), v("output_gain"), v("mode").round() as u32);
            (Box::new(move |x| builtin::distortion_transfer(x, dr, bias, gain, mx, out, mode)), Box::new(|x| x), "Harmonics", 0.18, 0.0)
        }
    };
    let readout = if caption == "Clipping" {
        let n = 256;
        let (mut pc, mut po) = (0.0f32, 0.0f32);
        for i in 0..n {
            let x = (TAU * i as f32 / n as f32).sin();
            pc = pc.max(curve(x).abs());
            po = po.max(reference(x).abs());
        }
        fmt_clipping(if po > 1e-9 && pc > 1e-9 { 20.0 * (pc / po).log10() } else { 0.0 })
    } else {
        fmt_percent_readout(harmonics_percent(&*curve))
    };
    let (mut field, w, h) = frame(rect, caption, readout);
    let inset = 8.0 * Z;
    let (x0, x1, y0, y1) = (inset, w - inset, inset, h - inset);
    let mid = 0.5 * (y0 + y1);
    let scale = 0.5 * (y1 - y0) / 1.25;
    let y_of = |s: f32| mid - s * scale;
    let n = ((x1 - x0).round() as usize).max(64);
    let xs: Vec<f32> = (0..=n).map(|i| x0 + (x1 - x0) * i as f32 / n as f32).collect();
    let input: Vec<f32> = (0..=n).map(|i| (TAU * 2.0 * i as f32 / n as f32).sin()).collect();
    let out: Vec<(f32, f32)> = xs.iter().zip(&input).map(|(&x, &s)| (x, y_of(curve(s).clamp(-1.25, 1.25)))).collect();
    let refs: Vec<(f32, f32)> = xs.iter().zip(&input).map(|(&x, &s)| (x, y_of(reference(s).clamp(-1.25, 1.25)))).collect();
    let mut paths = vec![
        stroke(format!("M{x0:.2} {mid:.2} L{x1:.2} {mid:.2}"), ink(0.13), 1.0),
        stroke(format!("M{x0:.2} {a:.2} L{x1:.2} {a:.2} M{x0:.2} {b:.2} L{x1:.2} {b:.2}", a = y_of(1.0), b = y_of(-1.0)), ink(0.05), 1.0),
        stroke(dashed(&refs, 3.0 * Z, 3.0 * Z), ink(0.25), 1.0),
        fill(format!("{} L{x1:.2} {mid:.2} L{x0:.2} {mid:.2} Z", poly(&out)), accent.with_alpha(fill_alpha.min(1.0))),
        stroke(poly(&out), accent, 1.5),
    ];
    if hiss > 0.0 {
        // Hiss dots around the output trace (deterministic xorshift, seed 7).
        let mut seed: u32 = 7;
        let mut dots = String::new();
        let amp = 0.12 * hiss * 0.5 * (y1 - y0);
        let mut x = x0;
        while x <= x1 {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            let r = seed as f32 / u32::MAX as f32 * 2.0 - 1.0;
            let t = (x - x0) / (x1 - x0);
            let y = y_of(curve((TAU * 2.0 * t).sin()).clamp(-1.25, 1.25)) + r * amp;
            let _ = write!(dots, "M{x:.2} {y:.2} h{z:.2} v{z:.2} h-{z:.2} Z ", z = Z);
            x += 2.0 * Z;
        }
        paths.push(fill(dots, accent.with_alpha(0.35)));
    }
    field.paths = model(paths);
    field.zones = model(vec![PanelZone { x: x0, y: y0, w: x1 - x0, h: y1 - y0, param_x: -1, param_y: drag_param, rate_x: 0.0, rate_y: 1.0 / (y1 - y0 - 16.0 * Z).max(8.0) }]);
    CustomBody { canvas: field, ..Default::default() }
}

/// Transient Shaper gain scope: the output as mirrored bars (square-root
/// amplitude law), what the clipper removed above them, a dashed 0 dB line and
/// the applied-gain trace (+-12 dB, flat at 0 dB at rest). `frames` are the
/// polled readouts (output peak, pre-clip peak, gain dB), oldest first; `cap`
/// frames span the well.
fn scope_body(frames: &[(f32, f32, f32)], cap: usize, accent: Color, rect: (f32, f32, f32, f32)) -> CustomBody {
    let (mut field, w, h) = frame(rect, "Output", "Applied Gain".into());
    let (mid, x0, n) = (h / 2.0, 6.0 * Z, frames.len());
    let x_of = |i: usize| w - (n - 1 - i) as f32 * (w - x0) / (cap.max(2) - 1) as f32;
    let r_of = |peak: f32| (peak.abs().min(1.41) / 1.41).sqrt() * mid;
    let out_r = |i: usize| r_of(frames[i].0);
    let pre_r = |i: usize| r_of(frames[i].1).max(out_r(i));
    let band = |outer: &dyn Fn(usize) -> f32, inner: &dyn Fn(usize) -> f32, sign: f32| {
        let mut pts: Vec<(f32, f32)> = (0..n).map(|i| (x_of(i), mid - sign * outer(i))).collect();
        pts.extend((0..n).rev().map(|i| (x_of(i), mid - sign * inner(i))));
        format!("{} Z", poly(&pts))
    };
    let y_of = |db: f32| 4.0 * Z + (12.0 - db.clamp(-12.0, 12.0)) / 24.0 * (h - 8.0 * Z);
    let mut paths = vec![stroke(dashed(&[(x0, mid), (w, mid)], 3.0 * Z, 4.0 * Z), ink(0.14), 1.0)];
    let gain = if n > 1 {
        paths.push(fill(format!("{} {}", band(&out_r, &|_| 0.0, 1.0), band(&out_r, &|_| 0.0, -1.0)), ink(0.3)));
        if frames.iter().any(|f| f.1 > f.0 * 1.005) {
            paths.push(fill(format!("{} {}", band(&pre_r, &out_r, 1.0), band(&pre_r, &out_r, -1.0)), ink(0.11)));
        }
        poly(&frames.iter().enumerate().map(|(i, f)| (x_of(i), y_of(f.2))).collect::<Vec<_>>())
    } else {
        poly(&[(x0, mid), (w, mid)])
    };
    paths.push(stroke(gain, accent, 2.0));
    field.paths = model(paths);
    CustomBody { canvas: field, ..Default::default() }
}

fn crush_body(value: &dyn Fn(&str) -> f32, p_down: i32, p_bits: i32, accent: Color, rect: (f32, f32, f32, f32)) -> CustomBody {
    let bits = value("bitdepth");
    let pic = builtin::crush_picture(bits, value("downsample"), value("mix"), value("dither"), value("jitter"));
    let (mut field, w, h) = frame(rect, "Waveform", String::new());
    let inset = 8.0 * Z;
    let (x0, x1, y0, y1) = (inset, w - inset, inset, h - inset);
    let mid = 0.5 * h;
    let scale = (0.5 * h - 8.0 * Z) / 1.15;
    let y_of = |s: f32| mid - s * scale;
    let mut guides = String::new();
    let levels: Vec<f32> = if bits.round() <= 4.0 {
        let q = 2f32.powf(bits.round() - 1.0);
        (1..=q as i32).flat_map(|k| [k as f32 / q, -(k as f32) / q]).collect()
    } else {
        vec![1.0, -1.0]
    };
    for s in levels {
        let y = y_of(s);
        let _ = write!(guides, "M{x0:.2} {y:.2} L{x1:.2} {y:.2} ");
    }
    let n = pic.len();
    let x_of = |i: usize| x0 + (x1 - x0) * i as f32 / n as f32;
    let dry: Vec<(f32, f32)> = (0..=n).map(|i| (x_of(i), y_of(pic[i.min(n - 1)].0))).collect();
    let mut steps = Vec::with_capacity(2 * n);
    for (i, (_, c)) in pic.iter().enumerate() {
        let y = y_of(c.clamp(-1.15, 1.15));
        steps.push((x_of(i), y));
        steps.push((x_of(i + 1), y));
    }
    field.paths = model(vec![
        stroke(format!("M{x0:.2} {mid:.2} L{x1:.2} {mid:.2}"), ink(0.13), 1.0),
        stroke(guides, ink(0.05), 1.0),
        stroke(dashed(&dry, 3.0 * Z, 3.0 * Z), ink(0.25), 1.0),
        fill(format!("M{x0:.2} {mid:.2} {} L{x1:.2} {mid:.2} Z", poly(&steps).replacen('M', "L", 1)), accent.with_alpha(0.3)),
        stroke(poly(&steps), accent, 1.5),
    ]);
    // Two-axis drag: right lowers Downsample (inverted), up raises Bit Depth.
    let top = y0 - 6.0 * Z;
    field.zones = model(vec![PanelZone {
        x: x0,
        y: top,
        w: x1 - x0,
        h: y1 - top,
        param_x: p_down,
        param_y: p_bits,
        rate_x: -1.0 / (x1 - x0 - 16.0 * Z).max(8.0),
        rate_y: 1.0 / (y1 - y0 - 16.0 * Z).max(8.0),
    }]);
    CustomBody { canvas: field, ..Default::default() }
}

#[allow(clippy::too_many_arguments)]
fn stereo_body(def: &DeviceDef, value: &dyn Fn(&str) -> f32, p_width: i32, p_side: i32, p_low: i32, accent: Color, rect: (f32, f32, f32, f32)) -> CustomBody {
    let (width, mid_db, side_db, low, mix) = (value("width2"), value("midgain"), value("sidegain"), value("lowcut"), value("mix"));
    let resp = |f: f32| builtin::widener_response(f, width, mid_db, side_db, low, mix, DRAW_RATE);
    let fmt = |id: &str, v: f32| def.param(id).map(|p| p.format(v)).unwrap_or_default();
    let (mut field, w, h) = frame(rect, "Width", format!("{:.0}%", resp(2000.0)));
    let m = 8.0 * Z;
    let mut paths = Vec::new();
    let mut texts = Vec::new();
    let mut grips = Vec::new();
    let axis = Color::from_rgb_u8(0x8f, 0x8f, 0x93);

    // Left: stereo image in a square.
    let size = (h - 16.0 * Z).min(0.32 * w).max(24.0 * Z);
    let (sx, sy) = (m, 0.5 * (h - size));
    let (cx, cy) = (sx + 0.5 * size, sy + 0.5 * size);
    let unity = 0.5 * size / 4.0;
    let x = mix / 100.0;
    let side_f = 1.0 - x + x * width / 100.0 * 10f32.powf(side_db / 20.0);
    let mid_f = 1.0 - x + x * 10f32.powf(mid_db / 20.0);
    let (rx, ry) = ((unity * side_f).min(0.5 * size), (unity * mid_f).min(0.5 * size));
    paths.push(stroke(format!("M{sx:.2} {cy:.2} L{:.2} {cy:.2} M{cx:.2} {sy:.2} L{cx:.2} {:.2}", sx + size, sy + size), ink(0.08), 1.0));
    paths.push(stroke(dashed(&circle_pts(cx, cy, unity), 2.0 * Z, 2.0 * Z), ink(0.3), 1.0));
    if rx > 0.3 && ry > 0.3 {
        paths.push(path(ellipse(cx, cy, rx, ry), accent, 1.5, accent.with_alpha(0.18)));
    } else {
        paths.push(stroke(format!("M{:.2} {cy:.2} L{:.2} {cy:.2}", cx - rx.max(0.3), cx + rx.max(0.3)), accent, 1.5));
    }
    paths.push(fill(ellipse(cx + rx, cy, 2.5 * Z, 2.5 * Z), accent));
    texts.push(text(sx + 6.0 * Z, sy + 8.0 * Z, 12.0 * Z, "L", axis, 10.0, 600, 0));
    texts.push(text(sx + size - 18.0 * Z, sy + 8.0 * Z, 12.0 * Z, "R", axis, 10.0, 600, 2));
    grips.push(PanelGrip { x: cx + rx + 6.0 * Z, y: cy, label: "Width".into(), value: fmt("width2", width).into(), hot: false });

    // Right: effective width over 20 Hz .. 2 kHz (log), 0 .. 400 %.
    let gutter = 30.0 * Z;
    let px0 = sx + size + 14.0 * Z + gutter;
    let px1 = w - m;
    let (py0, py1) = (m, h - m - 12.0 * Z);
    let fx = |f: f32| px0 + (px1 - px0) * (f / 20.0).ln() / 100f32.ln();
    let fy = |pct: f32| py1 - (py1 - py0) * (pct / 400.0).clamp(0.0, 1.0);
    paths.push(stroke(format!("M{px0:.2} {a:.2} L{px1:.2} {a:.2} M{px0:.2} {b:.2} L{px1:.2} {b:.2}", a = fy(0.0), b = fy(400.0)), ink(0.06), 1.0));
    paths.push(stroke(dashed(&[(px0, fy(100.0)), (px1, fy(100.0))], 3.0 * Z, 3.0 * Z), ink(0.22), 1.0));
    paths.push(stroke(format!("M{a:.2} {py0:.2} L{a:.2} {py1:.2} M{b:.2} {py0:.2} L{b:.2} {py1:.2}", a = fx(100.0), b = fx(1000.0)), ink(0.06), 1.0));
    for (pct, label) in [(400.0, "400%"), (100.0, "100%"), (0.0, "0%")] {
        texts.push(text(px0 - gutter - 4.0 * Z, fy(pct), gutter, label, axis, 9.5, 600, 2));
    }
    for (f, label) in [(20.0, "Hz"), (100.0, "100"), (1000.0, "1k")] {
        let tx = if f == 20.0 { fx(f) } else { fx(f) - 10.0 * Z };
        texts.push(text(tx, py1 + 8.0 * Z, 20.0 * Z, label, axis, 9.5, 600, if f == 20.0 { 0 } else { 1 }));
    }
    let n = ((px1 - px0).round() as usize).max(48);
    let curve: Vec<(f32, f32)> = (0..=n).map(|i| {
        let f = 20.0 * 100f32.powf(i as f32 / n as f32);
        (fx(f), fy(resp(f)))
    }).collect();
    paths.push(stroke(poly(&curve), accent, 1.5));
    let lx = fx(low.clamp(20.0, 2000.0));
    paths.push(stroke(dashed(&[(lx, py0), (lx, py1)], 3.0 * Z, 3.0 * Z), ink(0.3), 1.0));
    paths.push(fill(ellipse(lx, fy(resp(low)), 2.5 * Z, 2.5 * Z), accent));
    grips.push(PanelGrip { x: lx + 4.0 * Z, y: (fy(resp(low)) - 18.0 * Z).max(py0 + 8.0 * Z), label: "Low Cut".into(), value: fmt("lowcut", low).into(), hot: false });

    field.paths = model(paths);
    field.texts = model(texts);
    field.grips = model(grips);
    // Square: horizontal = Width, vertical = Side Gain, one square per full range.
    // Plot: horizontal = Low Cut on the plot's log axis (tied to 20 Hz .. 2 kHz).
    let half_gutter = 0.5 * (14.0 * Z + gutter);
    let low_travel = (px1 - px0) * (500f32 / 20.0).ln() / 100f32.ln();
    field.zones = model(vec![
        PanelZone { x: sx, y: sy, w: size + half_gutter, h: size, param_x: p_width, param_y: p_side, rate_x: 1.0 / size, rate_y: 1.0 / size },
        PanelZone { x: sx + size + half_gutter, y: 0.0, w: px1 - (sx + size + half_gutter) + m, h, param_x: p_low, param_y: -1, rate_x: 1.0 / low_travel.max(8.0), rate_y: 0.0 },
    ]);
    CustomBody { canvas: field, ..Default::default() }
}

fn utility_body(def: &DeviceDef, value: &dyn Fn(&str) -> f32, idx: &dyn Fn(&str) -> i32) -> CustomBody {
    let on = |id: &str| value(id) >= 0.5;
    let label = |id: &str| def.param(id).map_or("", |p| p.label);
    let header = Color::from_rgb_u8(0xa6, 0xa8, 0xad);
    // Two 91 pt columns around a 1 pt rule (centre 110.75 of the 221.5 pt card).
    let (left, right) = (56.2, 164.2);
    let latch = |id: &str, x: f32, w: f32| PanelButton { kind: 0, param: idx(id), x: x * Z, y: 0.0, w: w * Z, label: label(id).into(), on: on(id) };
    let at = |mut b: PanelButton, y: f32| {
        b.y = (y - 10.0) * Z;
        b
    };
    let mode = value("channel_mode").round() as usize;
    let mode_name = def.param("channel_mode").and_then(|p| p.choices.get(mode)).copied().unwrap_or("Stereo");
    let buttons = vec![
        at(latch("phase_l", left - 36.0, 34.0), 113.4),
        at(latch("phase_r", left + 2.0, 34.0), 113.4),
        at(PanelButton { kind: 1, param: idx("channel_mode"), x: (left - 36.0) * Z, y: 0.0, w: 72.0 * Z, label: mode_name.into(), on: false }, 137.9),
        at(latch("mono", left - 36.0, 72.0), 263.2),
        at(latch("mute", right - 36.0, 34.0), 263.2),
        at(latch("dc_filter", right + 2.0, 34.0), 263.2),
    ];
    let field = PanelCanvas {
        visible: true,
        well: false,
        x: 0.0,
        y: 0.0,
        w: 221.5 * Z,
        h: 300.0 * Z,
        well_y: 0.0,
        paths: model(vec![stroke(format!("M{x:.2} {:.2} L{x:.2} {:.2}", 66.6 * Z, 274.0 * Z, x = 110.75 * Z), ink(0.08), 1.0)]),
        texts: model(vec![
            text((left - 45.5) * Z, 74.5 * Z, 91.0 * Z, "Input", header, 12.0, 600, 1),
            text((right - 45.5) * Z, 74.5 * Z, 91.0 * Z, "Output", header, 12.0, 600, 1),
        ]),
        ..Default::default()
    };
    // Width has no effect in mono or with a single channel on both sides.
    let width_dead = on("mono") || ((mode == 1 || mode == 2) && on("phase_l") == on("phase_r"));
    CustomBody {
        canvas: field,
        buttons,
        knobs: vec![("gain", right - 31.0, 93.8), ("width", left - 31.0, 178.0), ("balance", right - 31.0, 178.0)],
        dim: if width_dead { vec!["width"] } else { vec![] },
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn harmonics_readout_matches_the_reference_defaults() {
        // Reference captures at defaults: Distortion "Harmonics 30%", Tape "27%".
        let dist = |x: f32| builtin::distortion_transfer(x, 30.0, 0.0, 0.0, 100.0, 0.0, 0);
        assert_eq!(fmt_percent_readout(harmonics_percent(&dist)), "30%");
        let tape = |x: f32| builtin::tape_transfer(x, 30.0, 30.0, 100.0, 0.0, 0);
        assert_eq!(fmt_percent_readout(harmonics_percent(&tape)), "27%");
        assert_eq!(fmt_percent_readout(harmonics_percent(&|x: f32| x)), "0.0%");
        assert_eq!(fmt_percent_readout(4.26), "4.3%");
    }

    #[test]
    fn clipping_readout_matches_the_reference_default() {
        // Soft Clipper at defaults: "Clipping −1.0 dB".
        let c = builtin::clipper_transfer(1.0, 0.0, 0.0, 50.0, 0, false, 100.0, 0.0);
        assert_eq!(fmt_clipping(20.0 * c.log10()), "\u{2212}1.0 dB");
        assert_eq!(fmt_clipping(-0.02), "0.0 dB");
    }

    #[test]
    fn widener_readout_matches_the_reference_default() {
        // Stereo Widener at defaults: "Width 198%".
        let r = builtin::widener_response(2000.0, 200.0, 0.0, 0.0, 120.0, 100.0, DRAW_RATE);
        assert_eq!(format!("{r:.0}%"), "198%");
    }

    #[test]
    fn dashes_follow_the_path() {
        let d = dashed(&[(0.0, 0.0), (10.0, 0.0)], 3.0, 3.0);
        assert_eq!(d.matches('M').count(), 2);
        assert!(d.starts_with("M0.00 0.00 L3.00 0.00"));
    }
}
