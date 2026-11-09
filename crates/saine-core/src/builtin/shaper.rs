//! Shaper (reference: the reference app's built-in "Shaper"): seven tempo-synced curve
//! lanes (Volume, Time, Width, Pan, Filter, Crush, Pitch), each a drawn shape
//! of 2 to 16 points that modulates one effect once per cycle. A lane can work
//! on the whole signal or only below / above a split frequency. The cycles
//! follow the transport (Trigger = Sync) or restart when the input, or a
//! sidechain, crosses a threshold (Trigger = Audio).
//!
//! Parameter ids, ranges, defaults, tapers and the panel arrangement are taken
//! from the reference as facts. The factory presets and the Shape menu's
//! library are our own (names and points).
//!
//! Point storage per lane: `<lane>_points` points are in use; point n has
//! `<lane>_p<n>_x` (0..1, non-decreasing, first 0, last 1), `<lane>_p<n>_y`
//! (0..100 %, Pitch -12..12 st) and `<lane>_p<n>_bend` (-100..100 %, the
//! curvature of the segment that starts at the point).

use super::*;
use crate::device::PluginKind;

/// Cycle lengths, slowest first (plain value = index; reference value table LFO_DIVISION).
pub const DIVISIONS: &[&str] = &[
    "8/1", "4/1", "2/1", "1/1", "1/2.", "1/2", "1/2T", "1/4.", "1/4", "1/4T", "1/8.", "1/8", "1/8T", "1/16.", "1/16", "1/16T", "1/32", "1/64.", "1/64", "1/64T",
];
/// Quarter notes per cycle for each entry of [`DIVISIONS`].
pub const DIVISION_QUARTERS: [f64; 20] = [
    32.0, 16.0, 8.0, 4.0, 3.0, 2.0, 4.0 / 3.0, 1.5, 1.0, 2.0 / 3.0, 0.75, 0.5, 1.0 / 3.0, 0.375, 0.25, 1.0 / 6.0, 0.125, 0.09375, 0.0625, 1.0 / 24.0,
];
const OFF_ON: &[&str] = &["Off", "On"];
const TRIGGERS: &[&str] = &["Sync", "Audio"];
const BANDS: &[&str] = &["Full", "Lows", "Highs"];
const FILTER_TYPES: &[&str] = &["Low-pass", "Band-pass", "High-pass"];

/// Lanes in parameter order: (id prefix, name).
pub const LANES: [(&str, &str); 7] = [("vol", "Volume"), ("time", "Time"), ("width", "Width"), ("pan", "Pan"), ("filter", "Filter"), ("crush", "Crush"), ("pitch", "Pitch")];
pub const LANE_COUNT: usize = 7;
pub const MAX_POINTS: usize = 16;
/// Parameters before the first lane, and per lane (on, rate, depth, band, split, points, 16 x (x, y, bend)).
pub const GLOBALS: usize = 11;
pub const LANE_STRIDE: usize = 54;
/// Curve value (0..1) each lane rests at: the effect is neutral there.
pub const REST: [f32; 7] = [1.0, 0.0, 0.5, 0.5, 1.0, 0.0, 0.5];
/// Longest repeat the Time lane can reach (the engine's buffer, seconds):
/// deeper points of long cycles at slow tempos read this far back.
pub const TIME_MAX_SECONDS: f64 = 20.0;

macro_rules! shaper_params {
    ($( $lane:ident $name:literal on $on:literal rate $rate:literal points $pts:literal y ($ymin:expr, $ymax:expr, $yunit:literal, $yfmt:expr)
        [ $( ($n:literal, $x:expr, $y:expr, $b:expr) ),* ] );* $(;)?) => {
        [
            ParamDef::choice("trigger", "Trigger", TRIGGERS, 0).help("Sync runs the cycles with the transport; Audio restarts them when the level crosses the threshold."),
            ParamDef::new("threshold", "Threshold", -60.0, 0.0, -24.0, LINEAR, "dB", Format::Db).help("Level that restarts the cycles when Trigger is Audio."),
            ParamDef::new("mix", "Mix", 0.0, 100.0, 100.0, LINEAR, "%", Format::Percent).help("Blend of shaped and dry signal."),
            ParamDef::new("output", "Output", -12.0, 12.0, 0.0, LINEAR, "dB", Format::Db).help("Output level."),
            ParamDef::choice("sidechain", "Sidechain", OFF_ON, 0).not_automatable().help("Set while the trigger listens to another track."),
            ParamDef::choice("filter_type", "Filter Type", FILTER_TYPES, 0).label("Type").help("Response of the Filter lane's filter."),
            ParamDef::new("filter_resonance", "Filter Resonance", 0.5, 12.0, 1.2973, Taper::Log, "Q", Format::Plain(1)).label("Resonance").help("Resonance of the Filter lane's filter."),
            ParamDef::new("filter_low", "Filter Low", 20.0, 20000.0, 150.0, Taper::Log, "Hz", Format::Hz).label("Low").help("Cutoff at the bottom of the Filter lane."),
            ParamDef::new("filter_high", "Filter High", 20.0, 20000.0, 12000.0, Taper::Log, "Hz", Format::Hz).label("High").help("Cutoff at the top of the Filter lane."),
            ParamDef::new("crush_bits", "Crush Bits", 1.0, 16.0, 6.0, LINEAR, "bits", Format::Plain(1)).label("Bits").help("Bit depth at the top of the Crush lane."),
            ParamDef::new("crush_decimate", "Crush Decimate", 1.0, 32.0, 8.0, Taper::Log, "x", Format::Plain(1)).label("Decimate").help("Sample-rate division at the top of the Crush lane."),
            $(
                ParamDef::choice(concat!(stringify!($lane), "_on"), concat!($name, " On"), OFF_ON, $on).label($name).help("Switches the lane on."),
                ParamDef::choice(concat!(stringify!($lane), "_rate"), concat!($name, " Rate"), DIVISIONS, $rate).label("Rate").help("Length of one cycle of the lane's shape."),
                ParamDef::new(concat!(stringify!($lane), "_depth"), concat!($name, " Depth"), 0.0, 100.0, 100.0, LINEAR, "%", Format::Percent).label("Depth").help("How far the shape moves the effect from its resting value."),
                ParamDef::choice(concat!(stringify!($lane), "_band"), concat!($name, " Band"), BANDS, 0).label("Band").help("Shape the whole signal, or only the lows or highs of the split."),
                ParamDef::new(concat!(stringify!($lane), "_split"), concat!($name, " Split"), 40.0, 8000.0, 150.0, Taper::Log, "Hz", Format::Hz).label("Split").help("Crossover frequency for Lows and Highs."),
                ParamDef::new(concat!(stringify!($lane), "_points"), concat!($name, " Points"), 2.0, 16.0, $pts, LINEAR, "", Format::Plain(0)).label("Points").step(1.0).not_automatable().help("Number of points in the shape."),
                $(
                    ParamDef::new(concat!(stringify!($lane), "_p", $n, "_x"), concat!($name, " P", $n, " Position"), 0.0, 1.0, $x, LINEAR, "", Format::Plain(2)).label("Position"),
                    ParamDef::new(concat!(stringify!($lane), "_p", $n, "_y"), concat!($name, " P", $n, " Height"), $ymin, $ymax, $y, LINEAR, $yunit, $yfmt).label("Height"),
                    ParamDef::new(concat!(stringify!($lane), "_p", $n, "_bend"), concat!($name, " P", $n, " Bend"), -100.0, 100.0, $b, LINEAR, "%", Format::Percent).label("Bend"),
                )*
            )*
        ]
    };
}

/// The Pitch lane switch sets the reported latency (PDC), which follows the
/// stored value only, so it is not automatable.
const fn pitch_switch_fixed(mut p: [ParamDef; 389]) -> [ParamDef; 389] {
    p[lane_base(6) + ON].automatable = false;
    p
}

static PARAMS: [ParamDef; 389] = pitch_switch_fixed(shaper_params![
    vol "Volume" on 1 rate 8 points 3.0 y (0.0, 100.0, "%", Format::Percent) [
        (1, 0.0, 0.0, 55.0),
        (2, 0.5, 100.0, 0.0),
        (3, 1.0, 100.0, 0.0),
        (4, 1.0, 100.0, 0.0),
        (5, 1.0, 100.0, 0.0),
        (6, 1.0, 100.0, 0.0),
        (7, 1.0, 100.0, 0.0),
        (8, 1.0, 100.0, 0.0),
        (9, 1.0, 100.0, 0.0),
        (10, 1.0, 100.0, 0.0),
        (11, 1.0, 100.0, 0.0),
        (12, 1.0, 100.0, 0.0),
        (13, 1.0, 100.0, 0.0),
        (14, 1.0, 100.0, 0.0),
        (15, 1.0, 100.0, 0.0),
        (16, 1.0, 100.0, 0.0)
    ];
    time "Time" on 0 rate 8 points 4.0 y (0.0, 100.0, "%", Format::Percent) [
        (1, 0.0, 0.0, 0.0),
        (2, 0.5, 0.0, 0.0),
        (3, 0.5, 50.0, 0.0),
        (4, 1.0, 50.0, 0.0),
        (5, 1.0, 50.0, 0.0),
        (6, 1.0, 50.0, 0.0),
        (7, 1.0, 50.0, 0.0),
        (8, 1.0, 50.0, 0.0),
        (9, 1.0, 50.0, 0.0),
        (10, 1.0, 50.0, 0.0),
        (11, 1.0, 50.0, 0.0),
        (12, 1.0, 50.0, 0.0),
        (13, 1.0, 50.0, 0.0),
        (14, 1.0, 50.0, 0.0),
        (15, 1.0, 50.0, 0.0),
        (16, 1.0, 50.0, 0.0)
    ];
    width "Width" on 0 rate 5 points 3.0 y (0.0, 100.0, "%", Format::Percent) [
        (1, 0.0, 50.0, 45.0),
        (2, 0.5, 100.0, 45.0),
        (3, 1.0, 50.0, 0.0),
        (4, 1.0, 50.0, 0.0),
        (5, 1.0, 50.0, 0.0),
        (6, 1.0, 50.0, 0.0),
        (7, 1.0, 50.0, 0.0),
        (8, 1.0, 50.0, 0.0),
        (9, 1.0, 50.0, 0.0),
        (10, 1.0, 50.0, 0.0),
        (11, 1.0, 50.0, 0.0),
        (12, 1.0, 50.0, 0.0),
        (13, 1.0, 50.0, 0.0),
        (14, 1.0, 50.0, 0.0),
        (15, 1.0, 50.0, 0.0),
        (16, 1.0, 50.0, 0.0)
    ];
    pan "Pan" on 0 rate 5 points 5.0 y (0.0, 100.0, "%", Format::Percent) [
        (1, 0.0, 50.0, 50.0),
        (2, 0.25, 100.0, 50.0),
        (3, 0.5, 50.0, -50.0),
        (4, 0.75, 0.0, -50.0),
        (5, 1.0, 50.0, 0.0),
        (6, 1.0, 50.0, 0.0),
        (7, 1.0, 50.0, 0.0),
        (8, 1.0, 50.0, 0.0),
        (9, 1.0, 50.0, 0.0),
        (10, 1.0, 50.0, 0.0),
        (11, 1.0, 50.0, 0.0),
        (12, 1.0, 50.0, 0.0),
        (13, 1.0, 50.0, 0.0),
        (14, 1.0, 50.0, 0.0),
        (15, 1.0, 50.0, 0.0),
        (16, 1.0, 50.0, 0.0)
    ];
    filter "Filter" on 0 rate 3 points 2.0 y (0.0, 100.0, "%", Format::Percent) [
        (1, 0.0, 100.0, 40.0),
        (2, 1.0, 15.0, 0.0),
        (3, 1.0, 15.0, 0.0),
        (4, 1.0, 15.0, 0.0),
        (5, 1.0, 15.0, 0.0),
        (6, 1.0, 15.0, 0.0),
        (7, 1.0, 15.0, 0.0),
        (8, 1.0, 15.0, 0.0),
        (9, 1.0, 15.0, 0.0),
        (10, 1.0, 15.0, 0.0),
        (11, 1.0, 15.0, 0.0),
        (12, 1.0, 15.0, 0.0),
        (13, 1.0, 15.0, 0.0),
        (14, 1.0, 15.0, 0.0),
        (15, 1.0, 15.0, 0.0),
        (16, 1.0, 15.0, 0.0)
    ];
    crush "Crush" on 0 rate 8 points 2.0 y (0.0, 100.0, "%", Format::Percent) [
        (1, 0.0, 100.0, -50.0),
        (2, 1.0, 0.0, 0.0),
        (3, 1.0, 0.0, 0.0),
        (4, 1.0, 0.0, 0.0),
        (5, 1.0, 0.0, 0.0),
        (6, 1.0, 0.0, 0.0),
        (7, 1.0, 0.0, 0.0),
        (8, 1.0, 0.0, 0.0),
        (9, 1.0, 0.0, 0.0),
        (10, 1.0, 0.0, 0.0),
        (11, 1.0, 0.0, 0.0),
        (12, 1.0, 0.0, 0.0),
        (13, 1.0, 0.0, 0.0),
        (14, 1.0, 0.0, 0.0),
        (15, 1.0, 0.0, 0.0),
        (16, 1.0, 0.0, 0.0)
    ];
    pitch "Pitch" on 0 rate 8 points 2.0 y (-12.0, 12.0, "st", Format::Plain(1)) [
        (1, 0.0, 0.0, 50.0),
        (2, 1.0, -12.0, 0.0),
        (3, 1.0, -12.0, 0.0),
        (4, 1.0, -12.0, 0.0),
        (5, 1.0, -12.0, 0.0),
        (6, 1.0, -12.0, 0.0),
        (7, 1.0, -12.0, 0.0),
        (8, 1.0, -12.0, 0.0),
        (9, 1.0, -12.0, 0.0),
        (10, 1.0, -12.0, 0.0),
        (11, 1.0, -12.0, 0.0),
        (12, 1.0, -12.0, 0.0),
        (13, 1.0, -12.0, 0.0),
        (14, 1.0, -12.0, 0.0),
        (15, 1.0, -12.0, 0.0),
        (16, 1.0, -12.0, 0.0)
    ];
]);

/// Index of a lane's first parameter (`<lane>_on`).
pub const fn lane_base(lane: usize) -> usize {
    GLOBALS + LANE_STRIDE * lane
}

/// Lane parameter offsets from [`lane_base`].
pub const ON: usize = 0;
pub const RATE: usize = 1;
pub const DEPTH: usize = 2;
pub const BAND: usize = 3;
pub const SPLIT: usize = 4;
pub const POINTS: usize = 5;
/// Offset of point k's (0-based) x; y and bend follow.
pub const fn point(k: usize) -> usize {
    6 + 3 * k
}

/// A curve point: x and y in 0..1, tension in -1..1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
    pub tension: f32,
}

/// Shape of the segment from a to b at t in 0..1 (exponential bend, the
/// reference curve law): k = 12 tension, signed so positive tension bows the
/// segment towards its end value (fast start).
pub fn segment(a: Point, b: Point, t: f32) -> f32 {
    let k = a.tension * 12.0 * if b.y >= a.y { 1.0 } else { -1.0 };
    let f = if k.abs() < 1e-4 { t } else { (1.0 - (-k * t).exp()) / (1.0 - (-k).exp()) };
    a.y + (b.y - a.y) * f
}

/// Curve value at x (0..1) of a point list (x non-decreasing, at least one point).
pub fn eval(points: &[Point], x: f32) -> f32 {
    let Some(first) = points.first() else { return 0.0 };
    if x <= first.x {
        return first.y;
    }
    for w in points.windows(2) {
        let (a, b) = (w[0], w[1]);
        if x <= b.x {
            let span = b.x - a.x;
            return if span <= 1e-6 { b.y } else { segment(a, b, (x - a.x) / span) };
        }
    }
    points[points.len() - 1].y
}

/// Tension of a segment that passes through height `y` at its middle (the
/// inverse of [`segment`] at t = 0.5, used by the bend handles).
pub fn tension_through(a: Point, b: Point, y: f32) -> f32 {
    if (b.y - a.y).abs() < 1e-6 {
        return 0.0;
    }
    let r = ((y - a.y) / (b.y - a.y)).clamp(0.0025, 0.9975);
    let sign = if b.y >= a.y { 1.0 } else { -1.0 };
    (2.0 * (r / (1.0 - r)).ln() / (12.0 * sign)).clamp(-1.0, 1.0)
}

/// Height parameter (plain) -> curve y (0..1) for a lane.
pub fn y_of(lane: usize, plain: f32) -> f32 {
    (if lane == 6 { (plain + 12.0) / 24.0 } else { plain / 100.0 }).clamp(0.0, 1.0)
}

/// Curve y (0..1) -> height parameter (plain) for a lane.
pub fn plain_of(lane: usize, y: f32) -> f32 {
    let y = y.clamp(0.0, 1.0);
    if lane == 6 { y * 24.0 - 12.0 } else { y * 100.0 }
}

/// A lane's points from plain parameter values in definition order.
pub fn lane_points(values: &[f32], lane: usize) -> Vec<Point> {
    let base = lane_base(lane);
    let n = (values.get(base + POINTS).copied().unwrap_or(2.0).round() as usize).clamp(2, MAX_POINTS);
    let mut prev = 0.0f32;
    (0..n)
        .map(|k| {
            let i = base + point(k);
            let x = if k == 0 { 0.0 } else if k == n - 1 { 1.0 } else { values.get(i).copied().unwrap_or(1.0).clamp(prev, 1.0) };
            prev = x;
            Point { x, y: y_of(lane, values.get(i + 1).copied().unwrap_or(0.0)), tension: (values.get(i + 2).copied().unwrap_or(0.0) / 100.0).clamp(-1.0, 1.0) }
        })
        .collect()
}

/// Parameter writes (id, plain) that store `points` in a lane: the count, the
/// used points, and the unused points parked at x = 1 on the rest height.
pub fn lane_writes(lane: usize, points: &[Point]) -> Vec<(&'static str, f32)> {
    let base = lane_base(lane);
    let n = points.len().clamp(2, MAX_POINTS);
    let mut out = vec![(PARAMS[base + POINTS].id, n as f32)];
    for k in 0..MAX_POINTS {
        let i = base + point(k);
        let (x, y, t) = match points.get(k) {
            Some(p) if k < n => (if k == 0 { 0.0 } else if k == n - 1 { 1.0 } else { p.x }, p.y, p.tension),
            _ => (1.0, REST[lane], 0.0),
        };
        let q = |idx: usize, v: f32| PARAMS[idx].snap(v);
        out.push((PARAMS[i].id, q(i, x)));
        out.push((PARAMS[i + 1].id, q(i + 1, plain_of(lane, y))));
        out.push((PARAMS[i + 2].id, q(i + 2, t * 100.0)));
    }
    out
}

/// A named shape of the Shape menu: points as (x, y, tension).
pub struct Shape {
    pub name: &'static str,
    pub points: &'static [(f32, f32, f32)],
}

const fn sh(name: &'static str, points: &'static [(f32, f32, f32)]) -> Shape {
    Shape { name, points }
}

/// Shape library per lane (our own); the first entry of each lane is the lane's default shape.
pub static SHAPES: [&[Shape]; 7] = [
    &[
        sh("Sidechain", &[(0.0, 0.0, 0.55), (0.5, 1.0, 0.0), (1.0, 1.0, 0.0)]),
        sh("Hard Duck", &[(0.0, 0.0, 0.85), (0.22, 1.0, 0.0), (1.0, 1.0, 0.0)]),
        sh("Soft Duck", &[(0.0, 0.4, 0.3), (0.8, 1.0, 0.0), (1.0, 1.0, 0.0)]),
        sh("Upbeat Duck", &[(0.0, 1.0, 0.0), (0.45, 1.0, 0.0), (0.5, 0.0, 0.6), (1.0, 1.0, 0.0)]),
        sh("Half Gate", &[(0.0, 1.0, 0.0), (0.45, 1.0, 0.0), (0.45, 0.0, 0.0), (1.0, 0.0, 0.0)]),
        sh("Chop", &[(0.0, 1.0, 0.0), (0.15, 1.0, 0.0), (0.15, 0.0, 0.0), (0.25, 0.0, 0.0), (0.25, 1.0, 0.0), (0.4, 1.0, 0.0), (0.4, 0.0, 0.0), (0.5, 0.0, 0.0), (0.5, 1.0, 0.0), (0.65, 1.0, 0.0), (0.65, 0.0, 0.0), (0.75, 0.0, 0.0), (0.75, 1.0, 0.0), (0.9, 1.0, 0.0), (0.9, 0.0, 0.0), (1.0, 0.0, 0.0)]),
        sh("Rise", &[(0.0, 0.0, -0.45), (1.0, 1.0, 0.0)]),
        sh("Fall", &[(0.0, 1.0, -0.35), (1.0, 0.0, 0.0)]),
        sh("Flat", &[(0.0, 1.0, 0.0), (1.0, 1.0, 0.0)]),
    ],
    &[
        sh("Echo Half", &[(0.0, 0.0, 0.0), (0.5, 0.0, 0.0), (0.5, 0.5, 0.0), (1.0, 0.5, 0.0)]),
        sh("Quad Repeat", &[(0.0, 0.0, 0.0), (0.25, 0.0, 0.0), (0.25, 0.25, 0.0), (0.5, 0.25, 0.0), (0.5, 0.5, 0.0), (0.75, 0.5, 0.0), (0.75, 0.75, 0.0), (1.0, 0.75, 0.0)]),
        sh("Brake", &[(0.0, 0.0, -0.3), (1.0, 0.45, 0.0)]),
        sh("Drag", &[(0.0, 0.0, 0.0), (1.0, 0.4, 0.0)]),
        sh("Backspin", &[(0.0, 0.0, 0.0), (0.4, 0.0, 0.0), (0.7, 0.6, 0.0), (0.7, 0.0, 0.0), (1.0, 0.0, 0.0)]),
        sh("Wobble", &[(0.0, 0.0, 0.4), (0.25, 0.12, -0.4), (0.5, 0.0, 0.4), (0.75, 0.12, -0.4), (1.0, 0.0, 0.0)]),
        sh("Flat", &[(0.0, 0.0, 0.0), (1.0, 0.0, 0.0)]),
    ],
    &[
        sh("Swell", &[(0.0, 0.5, 0.45), (0.5, 1.0, 0.45), (1.0, 0.5, 0.0)]),
        sh("Narrow Hit", &[(0.0, 0.0, -0.4), (1.0, 0.5, 0.0)]),
        sh("Wide Hit", &[(0.0, 1.0, -0.4), (1.0, 0.5, 0.0)]),
        sh("Flip", &[(0.0, 0.8, 0.0), (0.5, 0.8, 0.0), (0.5, 0.2, 0.0), (1.0, 0.2, 0.0)]),
        sh("Flat", &[(0.0, 0.5, 0.0), (1.0, 0.5, 0.0)]),
    ],
    &[
        sh("Circle", &[(0.0, 0.5, 0.5), (0.25, 1.0, 0.5), (0.5, 0.5, -0.5), (0.75, 0.0, -0.5), (1.0, 0.5, 0.0)]),
        sh("Bounce", &[(0.0, 0.15, 0.0), (0.5, 0.15, 0.0), (0.5, 0.85, 0.0), (1.0, 0.85, 0.0)]),
        sh("Drift Right", &[(0.0, 0.1, 0.2), (1.0, 0.9, 0.0)]),
        sh("Drift Left", &[(0.0, 0.9, 0.2), (1.0, 0.1, 0.0)]),
        sh("Flat", &[(0.0, 0.5, 0.0), (1.0, 0.5, 0.0)]),
    ],
    &[
        sh("Close", &[(0.0, 1.0, 0.4), (1.0, 0.15, 0.0)]),
        sh("Open", &[(0.0, 0.2, -0.35), (1.0, 1.0, 0.0)]),
        sh("Wah", &[(0.0, 0.15, -0.45), (0.5, 0.85, 0.45), (1.0, 0.15, 0.0)]),
        sh("Snap", &[(0.0, 1.0, -0.7), (0.3, 0.12, 0.0), (1.0, 0.12, 0.0)]),
        sh("Stairs", &[(0.0, 0.25, 0.0), (0.25, 0.25, 0.0), (0.25, 0.5, 0.0), (0.5, 0.5, 0.0), (0.5, 0.75, 0.0), (0.75, 0.75, 0.0), (0.75, 1.0, 0.0), (1.0, 1.0, 0.0)]),
        sh("Flat", &[(0.0, 1.0, 0.0), (1.0, 1.0, 0.0)]),
    ],
    &[
        sh("Fade", &[(0.0, 1.0, -0.5), (1.0, 0.0, 0.0)]),
        sh("Grow", &[(0.0, 0.0, -0.4), (1.0, 1.0, 0.0)]),
        sh("Burst", &[(0.0, 1.0, 0.0), (0.2, 1.0, 0.0), (0.2, 0.0, 0.0), (1.0, 0.0, 0.0)]),
        sh("Backbeat", &[(0.0, 0.0, 0.0), (0.45, 0.0, 0.0), (0.45, 0.9, 0.0), (1.0, 0.9, 0.0)]),
        sh("Flat", &[(0.0, 0.0, 0.0), (1.0, 0.0, 0.0)]),
    ],
    &[
        sh("Drop", &[(0.0, 0.5, 0.5), (1.0, 0.0, 0.0)]),
        sh("Climb", &[(0.0, 0.5, -0.45), (1.0, 1.0, 0.0)]),
        sh("Laser", &[(0.0, 1.0, -0.75), (0.2, 0.5, 0.0), (1.0, 0.5, 0.0)]),
        sh("Jump Up", &[(0.0, 0.5, 0.0), (0.6, 0.5, 0.0), (0.6, 1.0, 0.0), (1.0, 1.0, 0.0)]),
        sh("Jump Down", &[(0.0, 0.5, 0.0), (0.6, 0.5, 0.0), (0.6, 0.0, 0.0), (1.0, 0.0, 0.0)]),
        sh("Arpeggio", &[(0.0, 0.5, 0.0), (0.25, 0.5, 0.0), (0.25, 0.6667, 0.0), (0.5, 0.6667, 0.0), (0.5, 0.7917, 0.0), (0.75, 0.7917, 0.0), (0.75, 1.0, 0.0), (1.0, 1.0, 0.0)]),
        sh("Vibrato", &[(0.0, 0.5, 0.5), (0.25, 0.53, -0.5), (0.5, 0.5, 0.5), (0.75, 0.47, -0.5), (1.0, 0.5, 0.0)]),
        sh("Flat", &[(0.0, 0.5, 0.0), (1.0, 0.5, 0.0)]),
    ],
];

/// Shapes of a lane as point lists.
pub fn shape_points(shape: &Shape) -> Vec<Point> {
    shape.points.iter().map(|&(x, y, tension)| Point { x, y, tension }).collect()
}

/// Name of the library shape a point list matches, if any.
pub fn matching_shape(lane: usize, points: &[Point]) -> Option<&'static str> {
    let close = |a: &Point, b: &Point| (a.x - b.x).abs() < 0.006 && (a.y - b.y).abs() < 0.006 && (a.tension - b.tension).abs() < 0.006;
    SHAPES[lane.min(LANE_COUNT - 1)].iter().find(|s| s.points.len() == points.len() && shape_points(s).iter().zip(points).all(|(a, b)| close(a, b))).map(|s| s.name)
}

macro_rules! preset_points {
    ($lane:literal: $( ($n:literal, $x:expr, $y:expr, $b:expr) ),*) => {
        &[$( (concat!($lane, "_p", $n, "_x"), $x), (concat!($lane, "_p", $n, "_y"), $y), (concat!($lane, "_p", $n, "_bend"), $b) ),*]
    };
}

const fn cat<const N: usize>(a: &'static [(&'static str, f32)], b: &'static [(&'static str, f32)]) -> [(&'static str, f32); N] {
    assert!(a.len() + b.len() == N, "preset value count");
    let mut out = [("", 0.0f32); N];
    let mut i = 0;
    while i < a.len() {
        out[i] = a[i];
        i += 1;
    }
    let mut j = 0;
    while j < b.len() {
        out[i + j] = b[j];
        j += 1;
    }
    out
}

static P_DEEP_DUCK: [(&str, f32); 8] = cat(&[("vol_depth", 100.0), ("output", 1.0)], preset_points!("vol": (1, 0.0, 0.0, 80.0), (2, 0.32, 100.0, 0.0)));
static P_LOW_DUCK: [(&str, f32); 9] = cat(&[("vol_band", 1.0), ("vol_split", 180.0), ("vol_depth", 95.0)], preset_points!("vol": (1, 0.0, 0.0, 65.0), (2, 0.4, 100.0, 0.0)));
static P_UPBEAT: [(&str, f32); 14] =
    cat(&[("vol_points", 4.0), ("vol_depth", 90.0)], preset_points!("vol": (1, 0.0, 100.0, 0.0), (2, 0.45, 100.0, 0.0), (3, 0.5, 0.0, 60.0), (4, 1.0, 100.0, 0.0)));
static P_HALF_BAR: [(&str, f32); 9] = cat(&[("vol_rate", 5.0), ("vol_depth", 70.0), ("output", 0.5)], preset_points!("vol": (1, 0.0, 20.0, 40.0), (2, 0.7, 100.0, 0.0)));
static P_EIGHTH_GATE: [(&str, f32); 15] = cat(
    &[("vol_rate", 11.0), ("vol_points", 4.0), ("output", 1.0)],
    preset_points!("vol": (1, 0.0, 100.0, 0.0), (2, 0.55, 100.0, 0.0), (3, 0.55, 0.0, 0.0), (4, 1.0, 0.0, 0.0)),
);
static P_FILTER_WAH: [(&str, f32); 17] = cat(
    &[("vol_on", 0.0), ("filter_on", 1.0), ("filter_rate", 11.0), ("filter_points", 3.0), ("filter_resonance", 3.5), ("filter_low", 180.0), ("filter_high", 5000.0), ("output", -2.0)],
    preset_points!("filter": (1, 0.0, 15.0, -45.0), (2, 0.5, 85.0, 45.0), (3, 1.0, 15.0, 0.0)),
);
static P_BAR_CLOSE: [(&str, f32); 10] = cat(
    &[("vol_on", 0.0), ("filter_on", 1.0), ("filter_rate", 3.0), ("filter_resonance", 1.8)],
    preset_points!("filter": (1, 0.0, 100.0, 35.0), (2, 1.0, 10.0, 0.0)),
);
static P_FILTER_PUMP: [(&str, f32); 14] = cat(
    &[("vol_depth", 70.0), ("filter_on", 1.0), ("filter_rate", 8.0), ("filter_low", 300.0), ("filter_resonance", 1.6)],
    preset_points!("filter": (1, 0.0, 20.0, -35.0), (2, 0.6, 100.0, 0.0), (3, 1.0, 100.0, 0.0)),
);
static P_PANNER: [(&str, f32); 17] = cat(
    &[("vol_on", 0.0), ("pan_on", 1.0), ("pan_rate", 8.0), ("pan_depth", 75.0), ("pan_points", 4.0)],
    preset_points!("pan": (1, 0.0, 15.0, 0.0), (2, 0.5, 15.0, 0.0), (3, 0.5, 85.0, 0.0), (4, 1.0, 85.0, 0.0)),
);
static P_WIDE_BREATH: [(&str, f32); 5] = [("vol_on", 0.0), ("width_on", 1.0), ("width_rate", 3.0), ("width_depth", 80.0), ("output", -1.0)];
static P_TAPE_BRAKE: [(&str, f32); 15] = cat(
    &[("vol_rate", 3.0), ("vol_points", 2.0), ("time_on", 1.0), ("time_rate", 3.0), ("time_points", 2.0)],
    &[("vol_p1_y", 100.0), ("vol_p1_bend", 45.0), ("vol_p2_x", 1.0), ("vol_p2_y", 0.0), ("vol_p2_bend", 0.0), ("time_p1_bend", -30.0), ("time_p2_x", 1.0), ("time_p2_y", 45.0), ("time_p2_bend", 0.0), ("time_p1_y", 0.0)],
);
static P_REPEATER: [(&str, f32); 28] = cat(
    &[("vol_on", 0.0), ("time_on", 1.0), ("time_rate", 3.0), ("time_points", 8.0)],
    preset_points!("time": (1, 0.0, 0.0, 0.0), (2, 0.25, 0.0, 0.0), (3, 0.25, 25.0, 0.0), (4, 0.5, 25.0, 0.0), (5, 0.5, 50.0, 0.0), (6, 0.75, 50.0, 0.0), (7, 0.75, 75.0, 0.0), (8, 1.0, 75.0, 0.0)),
);
static P_OCTAVE_CLIMB: [(&str, f32); 9] = cat(&[("vol_on", 0.0), ("pitch_on", 1.0), ("pitch_rate", 2.0)], preset_points!("pitch": (1, 0.0, 0.0, -45.0), (2, 1.0, 12.0, 0.0)));
static P_FIFTH_HOP: [(&str, f32); 16] = cat(
    &[("vol_on", 0.0), ("pitch_on", 1.0), ("pitch_rate", 5.0), ("pitch_points", 4.0)],
    preset_points!("pitch": (1, 0.0, 0.0, 0.0), (2, 0.5, 0.0, 0.0), (3, 0.5, 7.0, 0.0), (4, 1.0, 7.0, 0.0)),
);
static P_CRUSH_SWELL: [(&str, f32); 12] = cat(
    &[("vol_on", 0.0), ("crush_on", 1.0), ("crush_rate", 2.0), ("crush_bits", 4.0), ("crush_decimate", 20.0)],
    &[("crush_p1_y", 0.0), ("crush_p1_bend", -40.0), ("crush_p2_y", 100.0), ("crush_p2_x", 1.0), ("crush_p2_bend", 0.0), ("output", -1.5), ("mix", 90.0)],
);
static P_BEAT_GRIT: [(&str, f32); 15] = cat(
    &[("vol_depth", 60.0), ("crush_on", 1.0), ("crush_bits", 9.0), ("crush_decimate", 4.0)],
    &[("crush_points", 2.0), ("crush_p1_y", 85.0), ("crush_p1_bend", -60.0), ("crush_p2_x", 1.0), ("crush_p2_y", 0.0), ("crush_p2_bend", 0.0), ("mix", 85.0), ("vol_p1_y", 30.0), ("vol_p2_x", 0.6), ("vol_p1_bend", 45.0), ("output", 0.5)],
);

pub static SHAPER: DeviceDef = DeviceDef {
    id: "shaper",
    name: "Shaper",
    kind: PluginKind::Effect,
    category: "Modulation",
    purpose: "Draws rhythmic movement into a sound: tempo-synced curves for volume, time, width, pan, filter, bit crush and pitch.",
    params: &PARAMS,
    layout: Layout { picker: &[], view: Some(View::Custom("shaper")), sliders: &[], clusters: &[], faces: &[], min_height: 220.0, shown_when: &[], fixed_width: 501.0 },
    factory: &[
        FactoryPreset { name: "Deep Duck", values: &P_DEEP_DUCK },
        FactoryPreset { name: "Low-End Duck", values: &P_LOW_DUCK },
        FactoryPreset { name: "Upbeat Lift", values: &P_UPBEAT },
        FactoryPreset { name: "Half-Bar Breathe", values: &P_HALF_BAR },
        FactoryPreset { name: "Eighth Gate", values: &P_EIGHTH_GATE },
        FactoryPreset { name: "Filter Wah", values: &P_FILTER_WAH },
        FactoryPreset { name: "Bar Close", values: &P_BAR_CLOSE },
        FactoryPreset { name: "Filtered Pump", values: &P_FILTER_PUMP },
        FactoryPreset { name: "Quarter Panner", values: &P_PANNER },
        FactoryPreset { name: "Wide Breath", values: &P_WIDE_BREATH },
        FactoryPreset { name: "Tape Brake", values: &P_TAPE_BRAKE },
        FactoryPreset { name: "Quarter Repeater", values: &P_REPEATER },
        FactoryPreset { name: "Octave Climb", values: &P_OCTAVE_CLIMB },
        FactoryPreset { name: "Fifth Hop", values: &P_FIFTH_HOP },
        FactoryPreset { name: "Crush Swell", values: &P_CRUSH_SWELL },
        FactoryPreset { name: "Beat Grit", values: &P_BEAT_GRIT },
    ],
    sidechain: true,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parameter_block_layout() {
        assert!(!SHAPER.param("pitch_on").unwrap().automatable, "pitch_on sets PDC latency");
        assert!(SHAPER.param("vol_on").unwrap().automatable);
        let d = &SHAPER;
        assert_eq!(d.params.len(), 389);
        for (lane, (id, _)) in LANES.iter().enumerate() {
            let b = lane_base(lane);
            assert_eq!(d.params[b + ON].id, format!("{id}_on"));
            assert_eq!(d.params[b + RATE].id, format!("{id}_rate"));
            assert_eq!(d.params[b + POINTS].id, format!("{id}_points"));
            assert_eq!(d.params[b + point(15) + 2].id, format!("{id}_p16_bend"));
        }
        assert_eq!(lane_base(1), 65);
        assert_eq!(lane_base(6), 335);
        assert_eq!(d.param("vol_rate").unwrap().format(8.0), "1/4");
        assert_eq!(d.param("filter_rate").unwrap().format(3.0), "1/1");
        assert_eq!(d.param("crush_decimate").unwrap().format(8.0), "8.0x");
        assert_eq!(d.param("filter_resonance").unwrap().format(1.2973), "1.3 Q");
        assert_eq!(d.param("pitch_p2_y").unwrap().format(-12.0), "\u{2212}12.0 st");
        assert_eq!(DIVISIONS.len(), DIVISION_QUARTERS.len());
    }

    #[test]
    fn default_shapes_match_the_library() {
        let values: Vec<f32> = SHAPER.params.iter().map(|p| p.default).collect();
        for (lane, shapes) in SHAPES.iter().enumerate() {
            let pts = lane_points(&values, lane);
            assert_eq!(matching_shape(lane, &pts), Some(shapes[0].name), "lane {lane}");
        }
    }

    #[test]
    fn curve_law() {
        let a = Point { x: 0.0, y: 0.0, tension: 0.55 };
        let b = Point { x: 1.0, y: 1.0, tension: 0.0 };
        assert_eq!(segment(a, b, 0.0), 0.0);
        assert!((segment(a, b, 1.0) - 1.0).abs() < 1e-6);
        assert!(segment(a, b, 0.5) > 0.9, "positive tension rises early");
        let mid = segment(a, b, 0.5);
        assert!((tension_through(a, b, mid) - 0.55).abs() < 1e-3);
        let straight = Point { tension: 0.0, ..a };
        assert!((segment(straight, b, 0.25) - 0.25).abs() < 1e-6);
        let pts = [a, Point { x: 0.5, y: 1.0, tension: 0.0 }, Point { x: 1.0, y: 1.0, tension: 0.0 }];
        assert_eq!(eval(&pts, 0.75), 1.0);
        let jump = [Point { x: 0.0, y: 1.0, tension: 0.0 }, Point { x: 0.5, y: 1.0, tension: 0.0 }, Point { x: 0.5, y: 0.0, tension: 0.0 }, Point { x: 1.0, y: 0.0, tension: 0.0 }];
        assert_eq!(eval(&jump, 0.49), 1.0);
        assert_eq!(eval(&jump, 0.51), 0.0);
    }

    #[test]
    fn lane_writes_round_trip() {
        let mut values: Vec<f32> = SHAPER.params.iter().map(|p| p.default).collect();
        let pts = shape_points(&SHAPES[6][5]);
        for (id, v) in lane_writes(6, &pts) {
            values[SHAPER.index_of(id).unwrap()] = v;
        }
        let back = lane_points(&values, 6);
        assert_eq!(back.len(), pts.len());
        assert_eq!(matching_shape(6, &back), Some("Arpeggio"));
        assert_eq!(values[SHAPER.index_of("pitch_p16_x").unwrap()], 1.0);
    }
}
