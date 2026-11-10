//! Devices minimap (reference/findings/minimap.md, S68-S70): the overview at the right end of the
//! Clip / Devices strip. It shows while the selected track's device chain is wider than the panel.
//! Each device is a small tile, a chain's tiles sit in a frame, and a window marks the part the
//! panel shows. Drag to scroll, press outside the window to jump, wheel to scroll, Esc goes back.
//!
//! The metrics here are pure and work in the reference's own px (design units / 0.85); the
//! Controller methods at the bottom convert and keep the drag, the pointer and the Slint model.

use crate::controller::{BottomTab, Controller, card_color, sync_model};
use crate::devices::{CHAIN_AREA_X, CHAIN_X0};
use crate::minimap::CSS;
use crate::{AppWindow, DevTile, DeviceCard, Studio};
use saine_core::{Device, DeviceId, builtin, modulation};
use slint::{ComponentHandle, VecModel};
use std::rc::Rc;

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
/// Glyph: 24 x 14 box, at most 240 / 14 px wide, 1.5 px clear of the tile edge, hidden under 6 px.
const GLYPH_MAX: f32 = 240.0 / 14.0;
const GLYPH_PAD: f32 = 1.5;
const GLYPH_MIN: f32 = 6.0;
/// Where the strip sits in the bottom panel: its left edge is 252.45 design units (297 - 8 ref px) from the
/// panel's right edge, it is 20.4 units high and 3.4 units below the top.
pub const X_FROM_RIGHT: f32 = 252.45;
pub const Y: f32 = 3.4;
/// The bar needs room for the tabs: the strip hides in a panel narrower than this (design units).
pub const MIN_PANEL_W: f32 = X_FROM_RIGHT + 330.0;
/// Tile fill and the glyph's mix: 60 % of the track colour over the card colour.
const CARD: [u8; 3] = [0x21, 0x22, 0x27];
const GLYPH_MIX: f32 = 0.6;
/// Glyph colour of a bypassed device: ink at 30 % over the tile.
const BYPASS_INK: [u8; 4] = [0xff, 0xff, 0xff, 0x4d];

/// What a tile's glyph depicts (the device's family).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Glyph {
    Device,
    Synth,
    Sampler,
    Pads,
    Lfo,
    Macros,
    Eq,
    Dynamics,
    Reverb,
    Delay,
    Modulation,
    Distortion,
    Filter,
    Imaging,
    Pitch,
    Restoration,
    Utility,
    Analyzer,
}

impl Glyph {
    /// Stroke path in a 24 x 14 box (our own drawings).
    pub fn path(self) -> &'static str {
        match self {
            Glyph::Device => "M5 3.5 H19 V10.5 H5 Z M8 7 H16",
            Glyph::Synth => "M2 11 L7 3 V11 L12 3 V11 L17 3 V11 L22 3",
            Glyph::Sampler => "M3 7 V7.2 M6 4 V10 M9 2 V12 M12 5 V9 M15 3 V11 M18 5 V9 M21 6.5 V7.5",
            Glyph::Pads => "M4.5 3 H8.5 V6 H4.5 Z M10 3 H14 V6 H10 Z M15.5 3 H19.5 V6 H15.5 Z M4.5 8 H8.5 V11 H4.5 Z M10 8 H14 V11 H10 Z M15.5 8 H19.5 V11 H15.5 Z",
            Glyph::Lfo => "M2 7 C5 1 8 1 11 7 C14 13 17 13 20 7 H22",
            Glyph::Macros => "M2.4 7 a2.6 2.6 0 1 0 5.2 0 a2.6 2.6 0 1 0 -5.2 0 M9.4 7 a2.6 2.6 0 1 0 5.2 0 a2.6 2.6 0 1 0 -5.2 0 M16.4 7 a2.6 2.6 0 1 0 5.2 0 a2.6 2.6 0 1 0 -5.2 0",
            Glyph::Eq => "M2 9 H6 C8 9 9 3 12 3 C15 3 16 9 18 9 H22",
            Glyph::Dynamics => "M2 12 H8 C10 12 11 6 14 6 H22",
            Glyph::Reverb => "M2 2.5 C5 9 10 11 22 11.5",
            Glyph::Delay => "M4 3 V11 M10 4.5 V9.5 M16 6 V8 M21 6.8 V7.2",
            Glyph::Modulation => "M2 7 C5 2 8 2 12 7 S19 12 22 7",
            Glyph::Distortion => "M2 7 L5 3 H9 L15 11 H19 L22 7",
            Glyph::Filter => "M2 4 H11 C16 4 17 10 22 10",
            Glyph::Imaging => "M3 4 H8 M3 10 H8 M16 4 H21 M16 10 H21 M12 2 V12",
            Glyph::Pitch => "M12 12 V3 M8 7 L12 3 L16 7",
            Glyph::Restoration => "M4 7 H20 M12 3 V11",
            Glyph::Utility => "M7 7 a5 5 0 1 0 10 0 a5 5 0 1 0 -10 0 M12 7 L14.5 4.5",
            Glyph::Analyzer => "M3 12 V8 M7 12 V4 M11 12 V6 M15 12 V2 M19 12 V7",
        }
    }

    /// The glyph of a device: its own kind first, then its browser category.
    pub fn of(d: &Device) -> Glyph {
        if modulation::is_lfo(d) {
            return Glyph::Lfo;
        }
        if modulation::is_rack(d) {
            return Glyph::Macros;
        }
        let Some(def) = builtin::def_of(d) else { return Glyph::Device };
        match def.id {
            "drum-rack" => return Glyph::Pads,
            "sampler-steam" | "pad-sampler" => return Glyph::Sampler,
            "synth-v2" => return Glyph::Synth,
            _ => {}
        }
        match def.category {
            "EQ" => Glyph::Eq,
            "Dynamics" => Glyph::Dynamics,
            "Reverb" => Glyph::Reverb,
            "Delay" => Glyph::Delay,
            "Modulation" => Glyph::Modulation,
            "Distortion" => Glyph::Distortion,
            "Filter" => Glyph::Filter,
            "Imaging" => Glyph::Imaging,
            "Pitch" => Glyph::Pitch,
            "Restoration" => Glyph::Restoration,
            "Utility" => Glyph::Utility,
            "Analyzer" => Glyph::Analyzer,
            _ => Glyph::Device,
        }
    }
}

/// A device card (or a chain's frame with its cards as children) in chain coordinates: px from the
/// left of the chain's scroll area.
#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub left: f32,
    pub width: f32,
    pub bypassed: bool,
    pub glyph: Glyph,
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
}

impl Tile {
    /// The glyph's width: as wide as the tile allows (1.5 px clear each side), at most 17.1 px, none under 6 px.
    pub fn glyph_w(&self) -> Option<f32> {
        if self.role == Role::Frame {
            return None;
        }
        let w = GLYPH_MAX.min(self.w - 2.0 * GLYPH_PAD);
        (w >= GLYPH_MIN).then_some(w)
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
        Tile { x: s + l, y, w: (c - 2.0 * l).max(2.0 * snap), h: HEIGHT - 2.0 * y, role, block, child }
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

/// A drag in progress (positions in reference px).
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

    /// Where the pointer goes on release (reference px in the strip): the pressed spot moved by how far
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
    /// One device pixel in reference px (1 / (scale x 0.85)).
    pub snap: f32,
}

impl State {
    pub fn new() -> State {
        State { chain: None, drag: None, model: Rc::new(VecModel::default()), snap: 1.0 / CSS }
    }
}

fn mix(accent: [u8; 3]) -> slint::Color {
    let c = |a: u8, b: u8| (a as f32 * GLYPH_MIX + b as f32 * (1.0 - GLYPH_MIX)).round() as u8;
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
            blocks.push(Block { left: left(CHAIN_X0), width: r(w), bypassed: !synth.enabled, glyph: Glyph::Synth, name: "Synth".into(), children: Vec::new() });
        }
        let device = |c: &DeviceCard| t.devices.iter().position(|d| d.id == DeviceId(c.id as u64));
        let leaf = |x: f32, c: &DeviceCard| {
            let at = device(c);
            let d = at.map(|i| &t.devices[i]).or(t.instrument_plugin.as_ref().filter(|d| d.id == DeviceId(c.id as u64)));
            Block {
                left: left(x),
                width: r(c.width),
                bypassed: at.map_or(d.is_some_and(|d| !d.enabled), |i| !modulation::effectively_enabled(&t.devices, i)),
                glyph: d.map_or(Glyph::Device, Glyph::of),
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
            children.extend(cards.iter().filter(|(_, m)| m.member && device(m).is_some_and(|i| t.devices[i].chain == Some(DeviceId(c.id as u64)))).map(|(mx, m)| leaf(*mx, m)));
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
        self.bottom_tab == BottomTab::Devices && !self.mixer_open && self.devices.win.0 - 10.0 >= MIN_PANEL_W && self.dm.chain.as_ref().is_some_and(Chain::scrollable)
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
            return;
        };
        self.dm.snap = 1.0 / (app.window().scale_factor() * CSS);
        let accent = self.selected_track.and_then(|t| self.project().track(t)).map(|t| card_color(t.color)).unwrap_or_default();
        let ink = mix([accent.red(), accent.green(), accent.blue()]);
        let rows: Vec<DevTile> = tiles(chain, WIDTH, self.dm.snap)
            .iter()
            .map(|t| {
                let block = &chain.blocks[t.block];
                let b = t.child.map_or(block, |j| &block.children[j]);
                let glyph_w = t.glyph_w();
                DevTile {
                    x: t.x * CSS,
                    y: t.y * CSS,
                    w: t.w * CSS,
                    h: t.h * CSS,
                    frame: t.role == Role::Frame,
                    glyph: b.glyph.path().into(),
                    glyph_w: glyph_w.unwrap_or(0.0) * CSS,
                    ink: if b.bypassed { slint_color_rgba(BYPASS_INK) } else { ink },
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

    pub fn devmm_chain_for_test(&self) -> Option<&Chain> {
        self.dm.chain.as_ref()
    }
}

fn slint_color_rgba(c: [u8; 4]) -> slint::Color {
    slint::Color::from_argb_u8(c[3], c[0], c[1], c[2])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(left: f32, width: f32) -> Block {
        Block { left, width, bypassed: false, glyph: Glyph::Device, name: String::new(), children: Vec::new() }
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
        assert!(t[0].glyph_w().is_none());
    }

    #[test]
    fn glyph_is_sized_by_its_tile() {
        let t = |w| Tile { x: 0.0, y: 3.0, w, h: 18.0, role: Role::Block, block: 0, child: None };
        assert_eq!(t(40.0).glyph_w(), Some(240.0 / 14.0));
        assert_eq!(t(12.0).glyph_w(), Some(9.0));
        assert_eq!(t(8.9).glyph_w(), None);
        assert_eq!(t(9.0).glyph_w(), Some(6.0));
    }

    #[test]
    fn every_glyph_has_a_path() {
        use Glyph::*;
        for g in [Device, Synth, Sampler, Pads, Lfo, Macros, Eq, Dynamics, Reverb, Delay, Modulation, Distortion, Filter, Imaging, Pitch, Restoration, Utility, Analyzer] {
            assert!(g.path().starts_with('M'), "{g:?}");
        }
    }

    #[test]
    fn glyphs_follow_the_device_family() {
        use saine_core::builtin;
        let of = |id: &str| Glyph::of(&builtin::find(id).unwrap().instantiate());
        assert_eq!(of("compressor"), Glyph::Dynamics);
        assert_eq!(of("channeleq"), Glyph::Eq);
        assert_eq!(of("reverb"), Glyph::Reverb);
        assert_eq!(of("delay"), Glyph::Delay);
        assert_eq!(of("chorus"), Glyph::Modulation);
        assert_eq!(of("drum-rack"), Glyph::Pads);
        assert_eq!(of("sampler-steam"), Glyph::Sampler);
        assert_eq!(of("synth-v2"), Glyph::Synth);
        assert_eq!(of("lfo"), Glyph::Lfo);
        assert_eq!(of("device-rack"), Glyph::Macros);
        assert_eq!(of("utility"), Glyph::Utility);
    }
}
