//! Devices minimap: the overview at the right end of the
//! Clip / Devices strip. It shows while the selected track's device chain is wider than the panel.
//! Each device is a small tile with a picture of its panel, a chain's tiles sit in a frame, and a
//! window marks the part the panel shows. Tiles fade in when a device is added and glide when the
//! layout changes ([Motion]); the strip redraws only while one runs. Drag to scroll, press outside the window to jump, wheel to scroll, Esc goes back.
//!
//! The metrics here are pure and work in 100%-zoom px (design units / 0.85); the
//! Controller methods at the bottom convert and keep the drag, the pointer and the Slint model.

use crate::controller::{BottomTab, Controller, card_color, sync_model};
use crate::devices::{CHAIN_AREA_X, CHAIN_X0};
use crate::minimap::CSS;
use crate::{AppWindow, DevTile, DeviceCard, Studio};
use dstudio_core::builtin::Control;
use dstudio_core::{Device, DeviceId, builtin, modulation};
use slint::{ComponentHandle, VecModel};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Instant;

/// The strip is 24 px high and, in the Clip / Devices bar, 227 px wide (the bar's trailing group is
/// 289 px, less an 8 px gap and the two 24 px layout buttons with their 4 px gap and 2 px margin).
pub const HEIGHT: f32 = 24.0;
pub const WIDTH: f32 = 227.0;
/// A strip never gets narrower than this (flex minimum); the bar hides it when it cannot fit.
pub const MIN_WIDTH: f32 = 96.0;
const _: () = assert!(WIDTH >= MIN_WIDTH);
/// Tile rows: a plain device, a chain's frame, and a device inside a frame; tiles keep 1 px off
/// their neighbours when wide enough.
const BLOCK_Y: f32 = 3.0;
const FRAME_Y: f32 = 2.0;
const CHILD_Y: f32 = 4.0;
const INSET: f32 = 1.0;
/// The window is at least 8 px wide.
const MIN_WINDOW: f32 = 8.0;
/// A press must travel 3 px before it counts as a drag.
const DRAG_SLOP: f32 = 3.0;
/// A tile narrower than this shows no picture.
const ART_MIN: f32 = 6.0;
/// Where the strip sits in the bottom panel: its left edge is 252.45 design units (297 - 8 ref px) from the
/// panel's right edge, it is 20.4 units high and 3.4 units below the top.
pub const X_FROM_RIGHT: f32 = 252.45;
pub const Y: f32 = 3.4;
/// The bar needs room for the tabs: the strip hides in a panel narrower than this (design units).
pub const MIN_PANEL_W: f32 = X_FROM_RIGHT + 330.0;
/// Tile fill and the picture's accent: 60 % of the track colour over the card colour.
const CARD: [u8; 3] = [0x21, 0x22, 0x27];
const ACCENT_MIX: f32 = 0.6;
/// A bypassed device's tile is drawn at 40 % (its picture keeps the accent).
pub const BYPASS_OPACITY: f32 = 0.4;
/// Fade-in of a new tile: 300 ms on cubic-bezier(.22, 1, .36, 1).
pub const FADE_S: f32 = 0.3;
const EASE: [f32; 4] = [0.22, 1.0, 0.36, 1.0];
/// Glide of the tiles' left edge and width: a critically damped spring whose rate lets it settle
/// within the same 300 ms. A tile is at rest when it is within half a 100%-zoom px of its target and
/// slower than 20 px/s.
const GLIDE_OMEGA: f32 = 19.0;
const REST_POS: f32 = 0.5;
const REST_VEL: f32 = 20.0;
/// A frame's key in [Motion] (its head device's key with this bit set).
const FRAME_KEY: u64 = 1 << 62;

/// A device's picture: its panel in miniature, built from the device's own layout (a view well,
/// the slider column, the dial rows). Drawn as two filled paths in a 127 x 75 box (the tile's viewbox in
/// app.slint) that stretches over the tile: the neutral parts (title bar, wells, dial bodies) and the accent parts
/// (dial marks, slider thumbs) that take the track colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Art {
    pub view: bool,
    pub sliders: u8,
    pub knobs: u8,
}

const MAX_SLIDERS: usize = 5;
const MAX_KNOBS: usize = 12;
const PER_ROW: usize = 6;

impl Art {
    /// The instrument panel: a view well over eight dials.
    pub const SYNTH: Art = Art { view: true, sliders: 0, knobs: 8 };
    /// A device this app has no layout for (a hosted plug-in): one large well.
    pub const PLUGIN: Art = Art { view: true, sliders: 0, knobs: 0 };

    pub fn of(d: &Device) -> Art {
        if modulation::is_lfo(d) {
            return Art { view: true, sliders: 0, knobs: 3 };
        }
        if modulation::is_rack(d) {
            return Art { view: false, sliders: 0, knobs: 8 };
        }
        let Some(def) = builtin::def_of(d) else { return Art::PLUGIN };
        let l = &def.layout;
        let knobs = l
            .clusters
            .iter()
            .flat_map(|c| c.iter())
            .filter(|c| matches!(c, Control::Knob(_) | Control::Swap(_) | Control::Toggle(_)))
            .count();
        Art { view: l.view.is_some(), sliders: l.sliders.len().min(MAX_SLIDERS) as u8, knobs: knobs.min(MAX_KNOBS) as u8 }
    }

    /// The neutral and the accent path commands (filled shapes only, so the stretch keeps them crisp).
    pub fn paths(self) -> (String, String) {
        use std::fmt::Write;
        let (mut base, mut acc) = (String::new(), String::new());
        let rect = |s: &mut String, x: f32, y: f32, w: f32, h: f32| {
            let _ = write!(s, "M{x:.1} {y:.1}H{:.1}V{:.1}H{x:.1}Z", x + w, y + h);
        };
        let dot = |s: &mut String, cx: f32, cy: f32, r: f32| {
            let _ = write!(s, "M{:.1} {cy:.1}a{r:.1} {r:.1} 0 1 0 {:.1} 0a{r:.1} {r:.1} 0 1 0 -{:.1} 0Z", cx - r, 2.0 * r, 2.0 * r);
        };
        rect(&mut base, 6.0, 6.0, 46.0, 5.0);
        dot(&mut acc, 115.0, 8.5, 3.0);
        let (top, bottom) = (17.0, 69.0);
        let n = self.sliders as usize;
        let mut right = 121.0;
        for i in 0..n {
            let x = 121.0 - (n - i) as f32 * 10.0 + 3.0;
            rect(&mut base, x, top, 4.0, bottom - top);
            rect(&mut acc, x - 1.0, top + (i * 17 % 36) as f32 + 4.0, 6.0, 5.0);
            right = x - 5.0;
        }
        let knobs = self.knobs as usize;
        let well_bottom = if knobs > 0 { 43.0 } else { bottom };
        if self.view {
            rect(&mut base, 6.0, top, right - 6.0, well_bottom - top);
        }
        if knobs == 0 {
            return (base, acc);
        }
        let rows = knobs.div_ceil(PER_ROW);
        let y0 = if self.view { 49.0 } else { top };
        let row_h = (bottom - y0) / rows as f32;
        for row in 0..rows {
            let cols = (knobs - row * PER_ROW).min(PER_ROW);
            let col_w = (right - 6.0) / PER_ROW.min(knobs) as f32;
            let r = (row_h / 2.0 - 1.0).min(col_w / 2.0 - 1.0).min(5.5);
            for col in 0..cols {
                let k = row * PER_ROW + col;
                let (cx, cy) = (6.0 + col_w * (col as f32 + 0.5), y0 + row_h * (row as f32 + 0.5));
                dot(&mut base, cx, cy, r);
                let a = -2.35 + (k * 37 % 100) as f32 / 100.0 * 4.7;
                dot(&mut acc, cx + r * 0.55 * a.sin(), cy - r * 0.55 * a.cos(), r * 0.35);
            }
        }
        (base, acc)
    }
}

/// A device card (or a chain's frame with its cards as children) in chain coordinates: px from the
/// left of the chain's scroll area.
#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub left: f32,
    pub width: f32,
    pub bypassed: bool,
    pub art: Art,
    /// The device's id: tiles keep their motion across layout changes by it.
    pub key: u64,
    pub name: String,
    pub children: Vec<Block>,
}

/// The selected track's device chain: scrollable width, panel width and the blocks.
#[derive(Debug, Clone, PartialEq)]
pub struct Chain {
    pub content_w: f32,
    pub view_w: f32,
    pub blocks: Vec<Block>,
}

impl Chain {
    /// The strip shows while the chain is at least 1 px wider than the panel.
    pub fn scrollable(&self) -> bool {
        self.content_w - self.view_w >= 1.0
    }

    pub fn max_scroll(&self) -> f32 {
        (self.content_w - self.view_w).max(0.0)
    }

    pub fn clamp(&self, scroll: f32) -> f32 {
        scroll.clamp(0.0, self.max_scroll())
    }

    /// What the strip maps: the scrollable width, or less when the last tile ends earlier (the
    /// trailing room after it is cut by as much as the leading room before the first one).
    pub fn map_w(&self) -> f32 {
        match (self.blocks.first(), self.blocks.last()) {
            (Some(f), Some(l)) => self.content_w.min(l.left + l.width + f.left),
            _ => self.content_w,
        }
    }

    /// Strip px per chain px.
    pub fn ratio(&self, strip: f32) -> f32 {
        let m = self.map_w();
        if m > 0.0 { strip / m } else { 0.0 }
    }

    /// The window (left, width) for a scroll position: the panel's width scaled, at least 8 px.
    pub fn window(&self, scroll: f32, strip: f32) -> (f32, f32) {
        let r = self.ratio(strip);
        let w = strip.min((self.view_w * r).max(MIN_WINDOW));
        ((scroll * r).max(0.0).min(strip - w), w)
    }

    /// Scroll that centres the window on strip x.
    pub fn scroll_at(&self, x: f32, strip: f32) -> f32 {
        let r = self.ratio(strip);
        if r <= 0.0 { 0.0 } else { self.clamp(x / r - self.view_w / 2.0) }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Block,
    Frame,
}

/// A rectangle of the strip, in px.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tile {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub role: Role,
    /// Index into Chain::blocks, and into its children for a device inside a frame.
    pub block: usize,
    pub child: Option<usize>,
    /// The tile's motion key: the device's id, or a frame's head id with [FRAME_KEY] set.
    pub key: u64,
}

impl Tile {
    /// Whether the tile shows its device's picture: devices only, and not narrower than 6 px.
    pub fn has_art(&self) -> bool {
        self.role == Role::Block && self.w >= ART_MIN
    }
}

/// Tiles of a chain in a strip `strip` px wide, with edges snapped to `snap` px (one device pixel).
/// A frame comes before its children.
pub fn tiles(chain: &Chain, strip: f32, snap: f32) -> Vec<Tile> {
    let r = chain.ratio(strip);
    if r <= 0.0 {
        return Vec::new();
    }
    let round = |v: f32| (v / snap).round() * snap;
    let tile = |b: &Block, role: Role, y: f32, inset: f32, block: usize, child: Option<usize>| {
        let s = round(b.left * r);
        let c = round((b.left + b.width) * r) - s;
        let l = if c - 2.0 * inset >= 2.0 { inset } else { 0.0 };
        let key = if role == Role::Frame { b.key | FRAME_KEY } else { b.key };
        Tile { x: s + l, y, w: (c - 2.0 * l).max(2.0 * snap), h: HEIGHT - 2.0 * y, role, block, child, key }
    };
    let mut out = Vec::new();
    for (i, b) in chain.blocks.iter().enumerate() {
        if b.children.is_empty() {
            out.push(tile(b, Role::Block, BLOCK_Y, INSET, i, None));
            continue;
        }
        out.push(tile(b, Role::Frame, FRAME_Y, 0.0, i, None));
        for (j, c) in b.children.iter().enumerate() {
            out.push(tile(c, Role::Block, CHILD_Y, INSET, i, Some(j)));
        }
    }
    out
}

/// Wheel over the strip: scroll by the dominant delta, in strip px (Slint's positive delta means
/// content moves right, so the chain scrolls back).
pub fn wheel(chain: &Chain, scroll: f32, dx: f32, dy: f32, strip: f32) -> f32 {
    let r = chain.ratio(strip);
    if r <= 0.0 {
        return scroll;
    }
    let d = if dx.abs() > dy.abs() { dx } else { dy };
    chain.clamp(scroll - d / r)
}

/// The fade-in curve, cubic-bezier(.22, 1, .36, 1): progress for time fraction `u`.
fn ease(u: f32) -> f32 {
    let [x1, y1, x2, y2] = EASE;
    let at = |t: f32, a: f32, b: f32| 3.0 * (1.0 - t) * (1.0 - t) * t * a + 3.0 * (1.0 - t) * t * t * b + t * t * t;
    if u <= 0.0 || u >= 1.0 {
        return u.clamp(0.0, 1.0);
    }
    let (mut lo, mut hi) = (0.0f32, 1.0f32);
    for _ in 0..24 {
        let mid = (lo + hi) / 2.0;
        if at(mid, x1, x2) < u { lo = mid } else { hi = mid }
    }
    at((lo + hi) / 2.0, y1, y2)
}

/// One step of a critically damped spring (exact, so any `dt` is stable).
fn spring(x: &mut f32, v: &mut f32, target: f32, dt: f32) {
    let d = *x - target;
    let k = *v + GLIDE_OMEGA * d;
    let e = (-GLIDE_OMEGA * dt).exp();
    *x = target + (d + k * dt) * e;
    *v = (*v - GLIDE_OMEGA * k * dt) * e;
    if (*x - target).abs() < REST_POS && v.abs() < REST_VEL {
        (*x, *v) = (target, 0.0);
    }
}

#[derive(Debug, Clone, Copy)]
struct Mot {
    x: f32,
    vx: f32,
    w: f32,
    vw: f32,
    /// Seconds since the tile appeared (capped at [FADE_S]).
    age: f32,
}

/// Where a tile is drawn now: its left edge and width in strip px, and its opacity from 0 to 1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placed {
    pub x: f32,
    pub w: f32,
    pub appear: f32,
}

/// The tiles' fade and glide. A tile new to a layout fades in at its place; a tile whose place
/// or width changed glides there. UI side only: the audio thread never sees it, and `busy` tells the
/// frame loop when to keep redrawing.
#[derive(Debug, Default)]
pub struct Motion {
    items: HashMap<u64, Mot>,
    busy: bool,
}

impl Motion {
    /// Whether any tile is still fading or gliding after the last step.
    pub fn busy(&self) -> bool {
        self.busy
    }

    /// Advance `dt` seconds toward `targets` (key, left, width); `jump` places every tile at once
    /// (the first layout shown, or another track's chain). Tiles no longer in `targets` are dropped.
    pub fn step(&mut self, targets: &[(u64, f32, f32)], dt: f32, jump: bool) -> Vec<Placed> {
        let fresh = jump || self.items.is_empty();
        let mut next = HashMap::with_capacity(targets.len());
        let mut busy = false;
        let out = targets
            .iter()
            .map(|&(key, tx, tw)| {
                let mut m = self.items.get(&key).copied().filter(|_| !jump).unwrap_or(Mot {
                    x: tx,
                    vx: 0.0,
                    w: tw,
                    vw: 0.0,
                    age: if fresh { FADE_S } else { 0.0 },
                });
                spring(&mut m.x, &mut m.vx, tx, dt);
                spring(&mut m.w, &mut m.vw, tw, dt);
                m.age = (m.age + dt).min(FADE_S);
                busy |= m.x != tx || m.w != tw || m.age < FADE_S;
                next.insert(key, m);
                Placed { x: m.x, w: m.w, appear: ease(m.age / FADE_S) }
            })
            .collect();
        self.items = next;
        self.busy = busy;
        out
    }
}

/// A drag in progress (positions in 100%-zoom px).
#[derive(Debug, Clone)]
pub struct Drag {
    /// Scroll before the press (Esc goes back to it).
    pub start_scroll: f32,
    /// Scroll the travel counts from (after a jump).
    from: f32,
    press: (f32, f32),
    strip: f32,
    pub moved: bool,
    /// The strip's origin in the window, design units (where the pointer goes back to).
    pub origin: (f32, f32),
}

/// Press at (x, y): the drag, and the scroll to jump to when the press is outside the window.
pub fn press(chain: &Chain, scroll: f32, x: f32, y: f32, strip: f32) -> Option<(Drag, Option<f32>)> {
    if chain.ratio(strip) <= 0.0 || strip <= 0.0 {
        return None;
    }
    let (left, width) = chain.window(scroll, strip);
    let jump = (x < left || x > left + width).then(|| chain.scroll_at(x, strip));
    let from = jump.unwrap_or(scroll);
    Some((Drag { start_scroll: scroll, from, press: (x, y), strip, moved: false, origin: (0.0, 0.0) }, jump))
}

impl Drag {
    /// Pointer moved to strip x: the scroll once the travel has passed the slop.
    pub fn moved(&mut self, chain: &Chain, x: f32) -> Option<f32> {
        let travel = x - self.press.0;
        if !self.moved && travel.abs() < DRAG_SLOP {
            return None;
        }
        self.moved = true;
        let r = chain.ratio(self.strip);
        (r > 0.0).then(|| chain.clamp(self.from + travel / r))
    }

    /// Where the pointer goes on release (100%-zoom px in the strip): the pressed spot moved by how far
    /// the window moved, kept inside the strip, on the pressed row.
    pub fn landing(&self, chain: &Chain, scroll: f32) -> (f32, f32) {
        let a = (scroll - self.from) * chain.ratio(self.strip);
        ((self.press.0 + a).clamp(0.0, self.strip), self.press.1)
    }
}

/// Controller-side state of the strip.
pub struct State {
    pub chain: Option<Chain>,
    pub drag: Option<Drag>,
    pub model: Rc<VecModel<DevTile>>,
    pub motion: Motion,
    /// When the tiles were last stepped (only while they move).
    last: Option<Instant>,
    /// The track whose chain the tiles show (another track's chain is placed without a fade).
    track: Option<dstudio_core::TrackId>,
    /// The device the render bench added (--drag-bench devmm).
    bench: Option<DeviceId>,
    /// Pictures by shape: built once, cloned per frame.
    art: HashMap<Art, (slint::SharedString, slint::SharedString)>,
    /// One device pixel in 100%-zoom px (1 / (scale x 0.85)).
    pub snap: f32,
}

impl State {
    pub fn new() -> State {
        State {
            chain: None,
            drag: None,
            model: Rc::new(VecModel::default()),
            motion: Motion::default(),
            last: None,
            track: None,
            bench: None,
            art: HashMap::new(),
            snap: 1.0 / CSS,
        }
    }
}

/// A frame's fill (ink at 7 %) and the picture's neutral layer (white at 18 %).
const FRAME_BG: [u8; 4] = [0xff, 0xff, 0xff, 0x12];
const ART_FILL: [u8; 4] = [0xff, 0xff, 0xff, 0x2e];

/// A colour with its alpha scaled by `opacity`. The fade works on colours, not on a Slint opacity,
/// which would put every fading tile in its own layer.
fn fade(c: [u8; 4], opacity: f32) -> slint::Color {
    slint::Color::from_argb_u8((c[3] as f32 * opacity).round() as u8, c[0], c[1], c[2])
}

fn mix(accent: [u8; 3]) -> slint::Color {
    let c = |a: u8, b: u8| (a as f32 * ACCENT_MIX + b as f32 * (1.0 - ACCENT_MIX)).round() as u8;
    slint::Color::from_rgb_u8(c(accent[0], CARD[0]), c(accent[1], CARD[1]), c(accent[2], CARD[2]))
}

impl Controller {
    /// The chain of the selected track from the cards just laid out (called when the device view models are rebuilt).
    pub(crate) fn devmm_sync_chain(&mut self, cards: &[(f32, DeviceCard)], add_x: f32) {
        self.dm.chain = self.devmm_chain(cards, add_x);
    }

    fn devmm_chain(&self, cards: &[(f32, DeviceCard)], add_x: f32) -> Option<Chain> {
        let t = self.selected_track.and_then(|t| self.project().track(t))?;
        let r = |v: f32| v / CSS;
        let left = |x: f32| r(x - CHAIN_AREA_X);
        let mut blocks = Vec::new();
        if let Some(synth) = self.synth_device().and_then(|id| self.project().device(id)) {
            let w = if synth.folded { crate::device_mod::FOLDED_W } else { crate::devices::SYNTH_W };
            blocks.push(Block {
                left: left(CHAIN_X0),
                width: r(w),
                bypassed: !synth.enabled,
                art: Art::SYNTH,
                key: synth.id.0,
                name: "Synth".into(),
                children: Vec::new(),
            });
        }
        let device = |c: &DeviceCard| t.devices.iter().position(|d| d.id == DeviceId(c.id as u64));
        let leaf = |x: f32, c: &DeviceCard| {
            let at = device(c);
            let d = at.map(|i| &t.devices[i]).or(t.instrument_plugin.as_ref().filter(|d| d.id == DeviceId(c.id as u64)));
            Block {
                left: left(x),
                width: r(c.width),
                bypassed: at.map_or(d.is_some_and(|d| !d.enabled), |i| !modulation::effectively_enabled(&t.devices, i)),
                art: d.map_or(Art::PLUGIN, Art::of),
                key: c.id as u64,
                name: c.name.to_string(),
                children: Vec::new(),
            }
        };
        let frames = self.chain_frames(cards);
        for (x, c) in cards {
            if c.member {
                continue;
            }
            let Some(frame) = frames.iter().find(|f| f.head == c.id) else {
                blocks.push(leaf(*x, c));
                continue;
            };
            let mut children = vec![leaf(*x, c)];
            children.extend(
                cards
                    .iter()
                    .filter(|(_, m)| m.member && device(m).is_some_and(|i| t.devices[i].chain == Some(DeviceId(c.id as u64))))
                    .map(|(mx, m)| leaf(*mx, m)),
            );
            let mut b = leaf(frame.x, c);
            b.width = r(frame.w);
            b.left = left(frame.x);
            b.children = children;
            blocks.push(b);
        }
        Some(Chain { content_w: r(add_x + crate::devices::ADD_W + 8.0 - CHAIN_AREA_X), view_w: r(self.chain_view_w()), blocks })
    }

    fn devmm_scroll_ref(&self) -> f32 {
        self.devices.chain_scroll / CSS
    }

    fn devmm_set_scroll(&mut self, scroll: f32) {
        self.devices.chain_scroll = scroll * CSS;
        self.clamp_chain_scroll();
        self.ui_dirty = true;
    }

    /// Whether the strip is on screen: the Devices tab, a panel wide enough, and a chain that scrolls.
    fn devmm_shown(&self) -> bool {
        self.bottom_tab == BottomTab::Devices
            && !self.mixer_open
            && self.devices.win.0 - 10.0 >= MIN_PANEL_W
            && self.dm.chain.as_ref().is_some_and(Chain::scrollable)
    }

    /// Pointer on the strip: kind 0 press, 1 move, 2 release; x and y in strip coordinates and (ox, oy) the
    /// strip's origin in the window (design units).
    pub fn devmm_pointer(&mut self, kind: i32, x: f32, y: f32, ox: f32, oy: f32) {
        let Some(chain) = self.dm.chain.clone() else { return };
        match kind {
            0 => {
                if !self.devmm_shown() {
                    return;
                }
                let Some((mut drag, jump)) = press(&chain, self.devmm_scroll_ref(), x / CSS, y / CSS, WIDTH) else { return };
                drag.origin = (ox, oy);
                self.dm.drag = Some(drag);
                if let Some(s) = jump {
                    self.devmm_set_scroll(s);
                }
                self.ui_dirty = true;
            }
            1 => {
                let Some(mut drag) = self.dm.drag.take() else { return };
                let s = drag.moved(&chain, x / CSS);
                if drag.moved {
                    self.ptr.hide();
                }
                self.dm.drag = Some(drag);
                if let Some(s) = s {
                    self.devmm_set_scroll(s);
                }
            }
            _ => {
                let Some(d) = self.dm.drag.take() else { return };
                let (lx, ly) = d.landing(&chain, self.devmm_scroll_ref());
                self.ptr.release(Some((d.origin.0 + lx * CSS, d.origin.1 + ly * CSS)));
                self.ui_dirty = true;
            }
        }
    }

    /// Esc during a drag: the chain goes back to where it was at the press, and the pointer to the pressed spot.
    pub fn devmm_cancel(&mut self) -> bool {
        let Some(d) = self.dm.drag.take() else { return false };
        self.devmm_set_scroll(d.start_scroll);
        self.ptr.release(Some((d.origin.0 + d.press.0 * CSS, d.origin.1 + d.press.1 * CSS)));
        true
    }

    pub fn devmm_wheel(&mut self, dx: f32, dy: f32) {
        let Some(chain) = self.dm.chain.clone() else { return };
        if self.devmm_shown() {
            let s = wheel(&chain, self.devmm_scroll_ref(), dx / CSS, dy / CSS, WIDTH);
            self.devmm_set_scroll(s);
        }
    }

    /// Push the tiles and the window to Slint (every frame; rows only change with the chain).
    pub fn update_devmm(&mut self, app: &AppWindow) {
        let s = app.global::<Studio>();
        let shown = self.devmm_shown();
        if s.get_devmm_visible() != shown {
            s.set_devmm_visible(shown);
        }
        if s.get_pointer_hidden() != self.ptr.hidden() {
            s.set_pointer_hidden(self.ptr.hidden());
        }
        let dragging = self.dm.drag.is_some();
        if s.get_devmm_dragging() != dragging {
            s.set_devmm_dragging(dragging);
        }
        let Some(chain) = self.dm.chain.as_ref().filter(|_| shown) else {
            if self.dm.drag.take().is_some() {
                self.ptr.release(None);
            }
            sync_model(&self.dm.model, Vec::new());
            self.dm.motion = Motion::default();
            self.dm.last = None;
            self.dm.track = None;
            return;
        };
        self.dm.snap = 1.0 / (app.window().scale_factor() * CSS);
        let accent = self.selected_track.and_then(|t| self.project().track(t)).map(|t| card_color(t.color)).unwrap_or_default();
        let ink = mix([accent.red(), accent.green(), accent.blue()]);
        let laid = tiles(chain, WIDTH, self.dm.snap);
        let targets: Vec<(u64, f32, f32)> = laid.iter().map(|t| (t.key, t.x, t.w)).collect();
        let now = Instant::now();
        let dt = match self.dm.last {
            Some(t) if self.dm.motion.busy() => (now - t).as_secs_f32().min(0.1),
            _ => 0.0,
        };
        self.dm.last = Some(now);
        let jump = self.dm.track != self.selected_track;
        self.dm.track = self.selected_track;
        let placed = self.dm.motion.step(&targets, dt, jump);
        let rows: Vec<DevTile> = laid
            .iter()
            .zip(&placed)
            .map(|(t, p)| {
                let block = &chain.blocks[t.block];
                let b = t.child.map_or(block, |j| &block.children[j]);
                let opacity = p.appear * if b.bypassed && t.role == Role::Block { BYPASS_OPACITY } else { 1.0 };
                let (art, art_accent) = if t.has_art() {
                    self.dm
                        .art
                        .entry(b.art)
                        .or_insert_with(|| {
                            let (n, a) = b.art.paths();
                            (n.into(), a.into())
                        })
                        .clone()
                } else {
                    Default::default()
                };
                DevTile {
                    x: p.x * CSS,
                    y: t.y * CSS,
                    w: p.w * CSS,
                    h: t.h * CSS,
                    frame: t.role == Role::Frame,
                    art,
                    art_accent,
                    bg: fade(if t.role == Role::Frame { FRAME_BG } else { [CARD[0], CARD[1], CARD[2], 0xff] }, opacity),
                    art_fill: fade(ART_FILL, opacity),
                    ink: fade([ink.red(), ink.green(), ink.blue(), 0xff], opacity),
                    opacity,
                    name: b.name.clone().into(),
                }
            })
            .collect();
        sync_model(&self.dm.model, rows);
        let (wl, ww) = chain.window(self.devmm_scroll_ref(), WIDTH);
        let (wl, ww) = (wl * CSS, ww * CSS);
        if s.get_devmm_win_x() != wl {
            s.set_devmm_win_x(wl);
        }
        if s.get_devmm_win_w() != ww {
            s.set_devmm_win_w(ww);
        }
    }

    /// --drag-bench devmm: add a delay to the selected track, or take it out again (the tiles fade and
    /// glide, or with `still` are placed at once).
    pub fn devmm_bench_toggle(&mut self, still: bool) {
        let Some(t) = self.selected_track else { return };
        if still {
            self.dm.track = None;
        }
        if let Some(device) = self.dm.bench.take() {
            self.execute(dstudio_core::Command::RemoveDevice { device });
            return;
        }
        let Some(def) = builtin::find("delay") else { return };
        if self.add_item_to_track(&crate::devices::Item::Builtin(def), t, None) {
            self.dm.bench = self.project().track(t).and_then(|t| t.devices.last()).map(|d| d.id);
        }
    }

    /// Whether the tiles are still fading or gliding (the frame loop keeps redrawing while so).
    pub fn devmm_animating(&self) -> bool {
        self.dm.motion.busy()
    }

    pub fn devmm_chain_for_test(&self) -> Option<&Chain> {
        self.dm.chain.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(left: f32, width: f32) -> Block {
        Block { left, width, bypassed: false, art: Art::PLUGIN, key: left as u64, name: String::new(), children: Vec::new() }
    }

    /// Three cards 200 wide, 8 apart, starting 10 from the left; 700 wide to scroll, 300 shown.
    fn chain() -> Chain {
        Chain { content_w: 700.0, view_w: 300.0, blocks: vec![block(10.0, 200.0), block(218.0, 200.0), block(426.0, 200.0)] }
    }

    #[test]
    fn shows_only_when_the_chain_is_wider_than_the_panel() {
        let mut c = chain();
        assert!(c.scrollable());
        c.content_w = 300.5;
        assert!(!c.scrollable());
        c.content_w = 301.0;
        assert!(c.scrollable());
        assert_eq!(c.max_scroll(), 1.0);
    }

    #[test]
    fn map_ends_after_the_last_tile_with_the_leading_margin() {
        let c = chain();
        assert_eq!(c.map_w(), 636.0);
        let mut wide = chain();
        wide.content_w = 600.0;
        assert_eq!(wide.map_w(), 600.0);
        assert_eq!(Chain { content_w: 500.0, view_w: 100.0, blocks: Vec::new() }.map_w(), 500.0);
    }

    #[test]
    fn window_scales_the_panel_and_has_a_minimum() {
        let c = chain();
        let (l, w) = c.window(0.0, 227.0);
        assert!(l.abs() < 1e-4 && (w - 300.0 * 227.0 / 636.0).abs() < 1e-3);
        let (l, w2) = c.window(200.0, 227.0);
        assert!((l - 200.0 * 227.0 / 636.0).abs() < 1e-3 && w2 == w);
        let (l, _) = c.window(10_000.0, 227.0);
        assert!((l + w - 227.0).abs() < 1e-3, "kept inside the strip");
        let tiny = Chain { content_w: 100_000.0, view_w: 100.0, blocks: vec![block(0.0, 100_000.0)] };
        assert_eq!(tiny.window(0.0, 227.0).1, MIN_WINDOW);
        let all = Chain { content_w: 301.0, view_w: 300.0, blocks: vec![block(0.0, 301.0)] };
        assert!(all.window(0.0, 227.0).1 <= 227.0);
    }

    #[test]
    fn a_press_outside_the_window_centres_it() {
        let c = chain();
        let r = c.ratio(227.0);
        let (d, jump) = press(&c, 0.0, 150.0, 12.0, 227.0).unwrap();
        let s = jump.unwrap();
        assert!((s - (150.0 / r - 150.0)).abs() < 1e-3);
        assert_eq!(d.start_scroll, 0.0);
        let (_, none) = press(&c, 0.0, 5.0, 12.0, 227.0).unwrap();
        assert!(none.is_none(), "a press inside the window does not jump");
        assert_eq!(c.scroll_at(0.0, 227.0), 0.0);
        assert_eq!(c.scroll_at(227.0, 227.0), c.max_scroll());
    }

    #[test]
    fn drag_follows_the_pointer_one_to_one_after_the_slop() {
        let c = chain();
        let r = c.ratio(227.0);
        let (mut d, _) = press(&c, 0.0, 5.0, 12.0, 227.0).unwrap();
        assert!(d.moved(&c, 7.0).is_none());
        assert!(!d.moved);
        let s = d.moved(&c, 25.0).unwrap();
        assert!((s - 20.0 / r).abs() < 1e-3, "{s}");
        assert!(d.moved(&c, 5000.0).unwrap() <= c.max_scroll());
        assert_eq!(d.moved(&c, -500.0), Some(0.0));
    }

    #[test]
    fn drag_after_a_jump_counts_from_the_jumped_position() {
        let c = chain();
        let (mut d, jump) = press(&c, 0.0, 150.0, 12.0, 227.0).unwrap();
        let from = jump.unwrap();
        let s = d.moved(&c, 170.0).unwrap();
        assert!((s - (from + 20.0 / c.ratio(227.0))).abs() < 1e-3);
        assert_eq!(d.start_scroll, 0.0, "Esc goes back to the scroll before the jump");
    }

    #[test]
    fn the_pointer_lands_where_the_pressed_spot_ended_up() {
        let c = chain();
        let r = c.ratio(227.0);
        let (mut d, _) = press(&c, 0.0, 5.0, 12.0, 227.0).unwrap();
        let s = d.moved(&c, 45.0).unwrap();
        let (x, y) = d.landing(&c, s);
        assert!((x - 45.0).abs() < 1e-3 && y == 12.0);
        let s = d.moved(&c, 5000.0).unwrap();
        let (x, _) = d.landing(&c, s);
        assert!((x - (5.0 + c.max_scroll() * r)).abs() < 1e-3, "the window stops at the end, so does the pointer: {x}");
        assert!(d.landing(&c, 1e9).0 <= 227.0);
    }

    #[test]
    fn wheel_scrolls_by_the_dominant_delta() {
        let c = chain();
        let r = c.ratio(227.0);
        assert!((wheel(&c, 100.0, 0.0, -10.0, 227.0) - (100.0 + 10.0 / r)).abs() < 1e-3);
        assert!((wheel(&c, 100.0, 20.0, -5.0, 227.0) - (100.0 - 20.0 / r)).abs() < 1e-3);
        assert_eq!(wheel(&c, 10.0, 0.0, 1000.0, 227.0), 0.0);
        assert_eq!(wheel(&c, 10.0, 0.0, -1e6, 227.0), c.max_scroll());
    }

    #[test]
    fn tiles_are_inset_and_snapped() {
        let c = chain();
        let t = tiles(&c, 227.0, 0.5);
        assert_eq!(t.len(), 3);
        let r = c.ratio(227.0);
        assert!((t[0].x - ((10.0 * r / 0.5).round() * 0.5 + 1.0)).abs() < 1e-4);
        assert_eq!((t[0].y, t[0].h), (3.0, 18.0));
        assert!(t.iter().all(|t| (t.x / 0.5).fract() == 0.0 && (t.w / 0.5).fract() == 0.0));
        let gap = t[1].x - (t[0].x + t[0].w);
        assert!(gap > 0.0, "1 px off each neighbour: {gap}");
    }

    #[test]
    fn narrow_tiles_lose_their_inset_and_keep_two_device_pixels() {
        let c = Chain { content_w: 1000.0, view_w: 100.0, blocks: vec![block(0.0, 5.0), block(995.0, 0.1)] };
        let t = tiles(&c, 200.0, 0.5);
        assert_eq!(t[0].x, 0.0, "a 1 px tile is too narrow for an inset");
        assert!(t[1].w >= 1.0);
    }

    #[test]
    fn a_chain_is_framed_around_its_devices() {
        let mut head = block(0.0, 450.0);
        head.children = vec![block(5.0, 200.0), block(225.0, 200.0)];
        let c = Chain { content_w: 900.0, view_w: 300.0, blocks: vec![head, block(460.0, 200.0)] };
        let t = tiles(&c, 227.0, 0.5);
        assert_eq!(t.iter().map(|t| t.role).collect::<Vec<_>>(), vec![Role::Frame, Role::Block, Role::Block, Role::Block]);
        assert_eq!((t[0].y, t[0].h, t[1].y, t[1].h), (2.0, 20.0, 4.0, 16.0));
        assert_eq!(t[0].x, 0.0, "a frame has no inset");
        assert_eq!((t[1].block, t[1].child, t[3].child), (0, Some(0), None));
        assert!(!t[0].has_art());
    }

    #[test]
    fn a_picture_needs_a_device_tile_of_6_px() {
        let t = |w, role| Tile { x: 0.0, y: 3.0, w, h: 18.0, role, block: 0, child: None, key: 1 };
        assert!(t(6.0, Role::Block).has_art());
        assert!(!t(5.9, Role::Block).has_art());
        assert!(!t(40.0, Role::Frame).has_art());
    }

    #[test]
    fn every_picture_is_two_filled_paths_in_the_box() {
        for art in [
            Art::SYNTH,
            Art::PLUGIN,
            Art { view: false, sliders: 0, knobs: 12 },
            Art { view: true, sliders: 5, knobs: 7 },
            Art { view: false, sliders: 0, knobs: 0 },
        ] {
            let (base, acc) = art.paths();
            assert!(base.starts_with('M') && acc.starts_with('M'), "{art:?}");
            assert!(base.ends_with('Z') && acc.ends_with('Z'), "{art:?}");
            assert!(base.matches('M').count() >= 1);
        }
        let (_, none) = Art { view: true, sliders: 0, knobs: 0 }.paths();
        assert_eq!(none.matches('M').count(), 1, "only the title dot is accent without dials");
        let (_, acc) = Art { view: false, sliders: 2, knobs: 4 }.paths();
        assert_eq!(acc.matches('M').count(), 1 + 2 + 4, "a title dot, a thumb per slider, a mark per dial");
    }

    #[test]
    fn pictures_follow_the_device_layout() {
        use dstudio_core::builtin;
        let of = |id: &str| Art::of(&builtin::find(id).unwrap().instantiate());
        let comp = of("compressor");
        let def = builtin::find("compressor").unwrap();
        assert_eq!(comp.view, def.layout.view.is_some());
        assert_eq!(comp.sliders as usize, def.layout.sliders.len().min(MAX_SLIDERS));
        assert!(comp.knobs > 0);
        assert_ne!(of("compressor"), of("reverb"), "different layouts give different pictures");
        assert_eq!(of("lfo"), Art { view: true, sliders: 0, knobs: 3 });
        assert_eq!(of("device-rack").knobs, 8);
    }

    #[test]
    fn fade_curve_runs_from_0_to_1_and_leads_the_clock() {
        assert!(ease(0.0).abs() < 1e-3 && (ease(1.0) - 1.0).abs() < 1e-3);
        assert!(ease(0.5) > 0.5, "a fast start: {}", ease(0.5));
        assert!(ease(0.9) > 0.97 && ease(0.9) <= 1.0);
        let mut last = 0.0;
        for i in 0..=20 {
            let v = ease(i as f32 / 20.0);
            assert!(v >= last - 1e-4, "monotonic at {i}");
            last = v;
        }
    }

    #[test]
    fn the_first_layout_is_placed_at_rest_without_a_fade() {
        let mut m = Motion::default();
        let p = m.step(&[(1, 10.0, 40.0), (2, 60.0, 40.0)], 0.0, false);
        assert_eq!((p[0].x, p[0].w, p[0].appear), (10.0, 40.0, 1.0));
        assert!(!m.busy());
    }

    #[test]
    fn a_new_tile_fades_in_and_the_others_glide() {
        let mut m = Motion::default();
        m.step(&[(1, 10.0, 80.0), (2, 100.0, 80.0)], 0.0, false);
        let p = m.step(&[(1, 10.0, 50.0), (2, 70.0, 50.0), (3, 130.0, 50.0)], 0.0, false);
        assert_eq!(p[2].appear, 0.0, "a new tile starts transparent");
        assert_eq!((p[0].w, p[1].x), (80.0, 100.0), "the others have not moved yet");
        assert!(m.busy());
        let p = m.step(&[(1, 10.0, 50.0), (2, 70.0, 50.0), (3, 130.0, 50.0)], 0.1, false);
        assert!(p[0].w < 80.0 && p[0].w > 50.0 && p[1].x < 100.0 && p[1].x > 70.0);
        assert!(p[2].appear > 0.5 && p[2].appear < 1.0);
        let p = m.step(&[(1, 10.0, 50.0), (2, 70.0, 50.0), (3, 130.0, 50.0)], 0.3, false);
        assert!(p[2].appear > 0.999);
        for _ in 0..10 {
            m.step(&[(1, 10.0, 50.0), (2, 70.0, 50.0), (3, 130.0, 50.0)], 0.1, false);
        }
        assert!(!m.busy(), "everything settles");
        let p = m.step(&[(1, 10.0, 50.0), (2, 70.0, 50.0), (3, 130.0, 50.0)], 0.0, false);
        assert_eq!((p[0].w, p[1].x, p[2].appear), (50.0, 70.0, 1.0));
    }

    #[test]
    fn the_glide_is_smooth_settles_in_about_300_ms_and_never_overshoots() {
        let (mut x, mut v) = (0.0f32, 0.0f32);
        let mut at_300 = 0.0;
        for i in 1..=60 {
            spring(&mut x, &mut v, 100.0, 0.01);
            assert!(x <= 100.0 + 1e-3, "no overshoot at {i}");
            if i == 30 {
                at_300 = x;
            }
        }
        assert!(at_300 > 95.0, "within 5 % after 300 ms: {at_300}");
        assert_eq!((x, v), (100.0, 0.0), "at rest after 600 ms");
        let (mut a, mut va, mut b, mut vb) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
        spring(&mut a, &mut va, 50.0, 0.1);
        for _ in 0..10 {
            spring(&mut b, &mut vb, 50.0, 0.01);
        }
        assert!((a - b).abs() < 1e-2, "one 100 ms step equals ten 10 ms steps: {a} {b}");
    }

    #[test]
    fn another_chain_and_a_removed_tile_do_not_animate() {
        let mut m = Motion::default();
        m.step(&[(1, 0.0, 50.0), (2, 60.0, 50.0)], 0.0, false);
        let p = m.step(&[(7, 5.0, 40.0)], 0.0, true);
        assert_eq!((p[0].x, p[0].appear), (5.0, 1.0));
        assert!(!m.busy());
        m.step(&[(7, 5.0, 40.0), (8, 50.0, 40.0)], 0.0, false);
        assert!(m.busy());
        let p = m.step(&[(8, 5.0, 40.0)], 0.0, false);
        assert_eq!(p.len(), 1, "a removed tile is gone at once");
    }
}
