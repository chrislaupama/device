//! Step sequencer view of a pattern clip (bottom panel, Clip tab, "Sequencer").
//!
//! Rows are Drum Rack pads (notes 36..51, Pad 1 at the top) or, on other
//! instrument tracks, pitches (Saine's extension; the reference app only sequences Drum
//! Rack clips). Every gesture previews on a local copy of the pattern and
//! commits ONE `SetStepPattern` on release, as the piano roll does with notes.
//!
//! Geometry is editor-local (the editor sits at bottom panel (255, 27), like the
//! piano roll), measured from `z100-clip-pattern-sequencer-*.png`.

use crate::Studio;
use crate::controller::Controller;
use saine_core::builtin::{self, drum_rack, pad_sampler};
use saine_core::midi::{self, MidiClip};
use saine_core::steps::{STEP_COUNTS, StepLane, StepPattern, StepRate, StepRows};
use saine_core::{ClipId, Command, PPQ};

/// Left edge of step 1 and the widest column pitch (cell + gap).
pub const COL_X0: f32 = 136.5;
pub const COL_PITCH: f32 = 25.5;
const COL_GAP: f32 = 2.5;
/// Paging: the reference fits 16 or 32 step columns of at least 24 px after
/// 182 px of row labels; in editor units (1 unit = 1.173 px there) that is
/// 20.5 and 155. Steps past the pattern end on the last page stay dim.
const PAGE_MIN_PITCH: f32 = 20.5;
const PAGE_LABELS_W: f32 = 155.0;
/// Length of the hit flash on the playing step (reference: 230 ms).
const FLASH: std::time::Duration = std::time::Duration::from_millis(230);
/// First row top and row pitch.
pub const ROW_Y0: f32 = 55.0;
pub const ROW_PITCH: f32 = 22.0;
/// Row label buttons (M, S): left edges, size.
const MUTE_X: f32 = 95.0;
const SOLO_X: f32 = 112.0;
const BTN_W: f32 = 12.0;
/// Editor height while the Clip tab is open, and the footer below the grid.
const EDITOR_H: f32 = crate::pianoroll::CLIP_PANEL_H - 27.0;
const FOOTER_H: f32 = crate::pianoroll::FOOTER_H;
/// Pitch rows open with C5 (72) on the top row.
const PITCH_TOP: u8 = 72;
/// Vertical drag distance (pt) for a lane's full range.
const LANE_DRAG: f32 = 150.0;

#[derive(Debug, Clone)]
enum SeqDrag {
    /// Steps lane: cells under the pointer are set on or off.
    Paint { on: bool },
    /// A value lane: vertical drag (Tie: horizontal) from the value at press.
    Value { key: u8, step: u16, lane: StepLane, from: i32, x0: f32, y0: f32 },
    /// Pressed a row's M or S button.
    Flag,
}

/// View state of the step sequencer (not part of the document).
#[derive(Debug, Clone, Default)]
pub struct SeqState {
    /// The piano roll tab is shown for the pattern clip instead.
    pub piano: bool,
    /// Lane tab: None for Steps.
    pub lane: Option<StepLane>,
    pub scroll: f32,
    drag: Option<SeqDrag>,
    /// The pattern as shown during a gesture.
    preview: Option<StepPattern>,
    /// Opened for a pattern clip (a conversion either way reopens the view).
    pub pattern: bool,
    /// Steps page shown (0-based; clamped to the pattern's pages when drawn).
    pub page: u16,
    /// Follow playhead switched off (the reference has it on by default).
    pub follow_off: bool,
    /// Step the playhead is on, and when it got there (the hit flash).
    play: Option<(u16, std::time::Instant)>,
}

impl SeqState {
    /// State for a newly opened clip: Sequencer tab, Steps lane, first page, scrolled to the pattern.
    pub fn opened(p: Option<&StepPattern>) -> Self {
        SeqState { scroll: p.map_or(0.0, initial_scroll), pattern: p.is_some(), ..Default::default() }
    }
}

/// Step columns per page for an editor this wide: 16 or 32.
pub fn steps_per_page(width: f32) -> u16 {
    let fit = ((width - PAGE_LABELS_W) / PAGE_MIN_PITCH).floor().max(0.0) as u16;
    (fit / 16 * 16).clamp(16, 32)
}

/// Pages a pattern of `steps` takes.
pub fn page_count(width: f32, steps: u16) -> u16 {
    steps.div_ceil(steps_per_page(width)).max(1)
}

/// Visible geometry of the grid.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SeqGeom {
    /// Columns of one page, the page shown, its first step, and the page count.
    pub cols: u16,
    pub page: u16,
    pub first: u16,
    pub pages: u16,
    pub pitch: f32,
    pub cell_w: f32,
    pub rows: u16,
    pub scroll: f32,
    pub view_h: f32,
}

impl SeqGeom {
    pub fn new(width: f32, steps: u16, rows: StepRows, scroll: f32, page: u16) -> Self {
        let cols = steps_per_page(width);
        let pages = page_count(width, steps);
        let page = page.min(pages - 1);
        let pitch = ((width - COL_X0 - 8.0) / cols as f32).min(COL_PITCH).max(6.0);
        let rows = match rows {
            StepRows::Pads => drum_rack::PADS as u16,
            StepRows::Pitches => 128,
        };
        let view_h = EDITOR_H - FOOTER_H - ROW_Y0;
        let max = (rows as f32 * ROW_PITCH - view_h + 4.0).max(0.0);
        SeqGeom { cols, page, first: page * cols, pages, pitch, cell_w: (pitch - COL_GAP).max(3.0), rows, scroll: scroll.clamp(0.0, max), view_h }
    }

    /// Left edge of a column of the page.
    pub fn col_x(&self, col: u16) -> f32 {
        COL_X0 + col as f32 * self.pitch
    }

    /// Column of a step, when the step is on the page shown.
    pub fn col_of(&self, step: u16) -> Option<u16> {
        step.checked_sub(self.first).filter(|&c| c < self.cols)
    }

    pub fn row_y(&self, row: u16) -> f32 {
        ROW_Y0 + row as f32 * ROW_PITCH - self.scroll
    }

    /// Rows at least partly inside the grid viewport.
    pub fn visible_rows(&self) -> std::ops::Range<u16> {
        let first = (self.scroll / ROW_PITCH).floor() as u16;
        let last = (((self.scroll + self.view_h) / ROW_PITCH).ceil() as u16).min(self.rows);
        first.min(last)..last
    }

    fn row_at(&self, y: f32) -> Option<u16> {
        let r = ((y - ROW_Y0 + self.scroll) / ROW_PITCH).floor();
        (y >= ROW_Y0 && y < ROW_Y0 + self.view_h && r >= 0.0 && (r as u16) < self.rows).then_some(r as u16)
    }

    /// Step under an editor x (on the page shown).
    fn step_at(&self, x: f32) -> Option<u16> {
        let s = ((x - COL_X0) / self.pitch).floor();
        (x >= COL_X0 && s >= 0.0 && (s as u16) < self.cols).then_some(self.first + s as u16)
    }
}

pub fn row_key(rows: StepRows, row: u16) -> u8 {
    match rows {
        StepRows::Pads => drum_rack::FIRST_NOTE + row as u8,
        StepRows::Pitches => 127 - row.min(127) as u8,
    }
}

/// What the grid shows for one cell. state: 0 past the pattern end, 1 off, 2 on.
/// lane: 0 Steps, 1 Velocity, 2 Offset, 3 Repeat, 4 Tie. bar: the lane value
/// as 0..1 (Velocity fill height, Offset line position); marks: Repeat hits.
#[derive(Debug, Clone, PartialEq)]
pub struct CellView {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub state: i32,
    pub lane: i32,
    pub bar: f32,
    pub marks: i32,
    pub label: String,
    pub dim: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RowView {
    pub y: f32,
    pub key: u8,
    pub number: String,
    pub name: String,
    pub muted: bool,
    pub soloed: bool,
    pub audible: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NumView {
    pub x: f32,
    pub text: String,
    pub beat: bool,
    pub active: bool,
}

pub struct SeqView {
    pub geom: SeqGeom,
    pub rows: Vec<RowView>,
    pub cells: Vec<CellView>,
    pub nums: Vec<NumView>,
}

fn lane_index(lane: Option<StepLane>) -> i32 {
    lane.map_or(0, |l| StepLane::ALL.iter().position(|&a| a == l).unwrap_or(0) as i32 + 1)
}

/// The grid's view model for one page: visible rows only, background cells
/// first, then the active cells on top (a tied cell spans the steps it holds,
/// cut at the page edges).
pub fn build_view(p: &StepPattern, width: f32, scroll: f32, page: u16, lane: Option<StepLane>, names: &dyn Fn(u8) -> String, dragging: Option<(u8, u16)>) -> SeqView {
    let g = SeqGeom::new(width, p.steps, p.rows, scroll, page);
    let last = g.first + g.cols - 1;
    let mut rows = Vec::new();
    let mut cells = Vec::with_capacity(g.visible_rows().len() * (g.cols as usize + 4));
    let li = lane_index(lane);
    for r in g.visible_rows() {
        let key = row_key(p.rows, r);
        let y = g.row_y(r);
        let audible = p.audible(key);
        rows.push(RowView {
            y,
            key,
            number: format!("{}", if p.rows == StepRows::Pads { r as u32 + 1 } else { key as u32 }),
            name: names(key),
            muted: p.muted.contains(&key),
            soloed: p.soloed.contains(&key),
            audible,
        });
        for col in 0..g.cols {
            let state = if g.first + col < p.steps { 1 } else { 0 };
            cells.push(CellView { x: g.col_x(col), y, w: g.cell_w, state, lane: li, bar: 0.0, marks: 0, label: String::new(), dim: false });
        }
        for (c, tie) in p.sounding(key) {
            let (a, b) = (c.step.max(g.first), (c.step + tie).min(last));
            if a > b {
                continue;
            }
            let (bar, marks) = match lane {
                Some(StepLane::Velocity) => (c.velocity as f32 / 127.0, 0),
                Some(StepLane::Offset) => ((p.effective_offset(&c) as f32 + 49.0) / 98.0, 0),
                Some(StepLane::Repeat) => (0.0, c.repeat as i32),
                _ => (0.0, 0),
            };
            let label = match (lane, dragging) {
                (Some(l), Some(at)) if at == (key, c.step) => l.format(c.get(l)),
                _ => String::new(),
            };
            cells.push(CellView { x: g.col_x(a - g.first), y, w: g.cell_w + (b - a) as f32 * g.pitch, state: 2, lane: li, bar, marks, label, dim: !audible });
        }
    }
    let steps_per_beat = p.rate.steps_per_beat();
    let every = if g.pitch >= 18.0 { 1 } else { steps_per_beat.round().max(1.0) as u16 };
    let nums = (0..g.cols)
        .filter(|col| (g.first + col) % every == 0)
        .map(|col| {
            let s = g.first + col;
            NumView { x: g.col_x(col), text: format!("{}", s + 1), beat: is_beat(p.rate, s), active: s < p.steps }
        })
        .collect();
    SeqView { geom: g, rows, cells, nums }
}

/// The step starts on a beat.
fn is_beat(rate: StepRate, step: u16) -> bool {
    (step as i64 * rate.ticks()) % PPQ == 0
}

impl Controller {
    /// The open clip's pattern (the gesture preview while dragging).
    pub fn seq_pattern(&self) -> Option<StepPattern> {
        let c = self.pr_clip()?;
        let p = c.pattern.as_ref()?;
        Some(self.pr.seq.preview.clone().unwrap_or_else(|| p.clone()))
    }

    /// The step sequencer (not the piano roll) is shown for the open clip.
    pub fn seq_visible(&self) -> bool {
        self.pr_clip().is_some_and(|c| c.pattern.is_some()) && !self.pr.seq.piano
    }

    /// Row label for a key: the Drum Rack pad's sample, else "Pad n"; pitch rows show the note.
    pub fn seq_row_name(&self, rows: StepRows, key: u8) -> String {
        if rows == StepRows::Pitches {
            return midi::note_name(key);
        }
        let pad = (key - drum_rack::FIRST_NOTE) as u32;
        let p = self.project();
        let rack = self.pr_clip().and_then(|c| p.find_midi_clip(c.id)).and_then(|(t, _)| p.tracks[t].instrument_plugin.as_ref()).filter(|d| is_drum_rack(d));
        rack.and_then(|d| d.sample(pad * pad_sampler::LAYERS)).map_or_else(|| format!("Pad {}", pad + 1), |s| s.name.clone())
    }

    pub fn seq_view(&self) -> Option<SeqView> {
        let p = self.seq_pattern()?;
        let dragging = match &self.pr.seq.drag {
            Some(SeqDrag::Value { key, step, .. }) => Some((*key, *step)),
            _ => None,
        };
        Some(build_view(&p, self.pr.size.0, self.pr.seq.scroll, self.pr.seq.page, self.pr.seq.lane, &|k| self.seq_row_name(p.rows, k), dragging))
    }

    fn seq_clip(&self) -> Option<(ClipId, StepPattern)> {
        let c = self.pr_clip()?;
        Some((c.id, c.pattern.clone()?))
    }

    fn seq_commit(&mut self, id: ClipId, old: &StepPattern, new: StepPattern) {
        if &new != old {
            self.execute(Command::SetStepPattern { clip: id, pattern: Some(new) });
        }
        self.ui_dirty = true;
    }

    /// Sequencer / Piano roll tabs of a pattern clip.
    pub fn seq_tab(&mut self, piano: bool) {
        self.pr.seq.piano = piano;
        self.pr.seq.drag = None;
        self.pr.seq.preview = None;
        self.ui_dirty = true;
    }

    /// Steps (0) or a value lane (1 Velocity .. 4 Tie).
    pub fn seq_lane(&mut self, index: i32) {
        self.pr.seq.lane = usize::try_from(index - 1).ok().and_then(|i| StepLane::ALL.get(i).copied());
        self.ui_dirty = true;
    }

    /// Toolbar Double / Clear.
    pub fn seq_action(&mut self, action: &str) {
        let Some((id, p)) = self.seq_clip() else { return };
        match action {
            "double" => match p.doubled() {
                Some(d) => self.seq_commit(id, &p, d),
                None => self.set_status("Pattern already at max length", 3),
            },
            "clear" => self.seq_commit(id, &p, p.cleared()),
            _ => {}
        }
    }

    /// Footer Length / Rate menus (index into STEP_COUNTS / StepRate::ALL).
    pub fn seq_set_length(&mut self, index: usize) {
        let (Some((id, p)), Some(&n)) = (self.seq_clip(), STEP_COUNTS.get(index)) else { return };
        self.seq_commit(id, &p, p.resized(n));
    }

    pub fn seq_set_rate(&mut self, index: usize) {
        let (Some((id, p)), Some(&rate)) = (self.seq_clip(), StepRate::ALL.get(index)) else { return };
        self.seq_commit(id, &p, StepPattern { rate, ..p.clone() });
    }

    /// Pager tab: show steps page `index` (0-based).
    pub fn seq_page(&mut self, index: i32) {
        let Some(p) = self.seq_pattern() else { return };
        if let Ok(i) = u16::try_from(index)
            && i < page_count(self.pr.size.0, p.steps)
        {
            self.pr.seq.page = i;
            self.ui_dirty = true;
        }
    }

    /// Pager's Follow playhead button.
    pub fn seq_follow(&mut self) {
        self.pr.seq.follow_off = !self.pr.seq.follow_off;
        self.ui_dirty = true;
    }

    /// Pattern step under the playhead while the transport plays inside the open clip.
    pub fn seq_play_step(&self) -> Option<u16> {
        let c = self.pr_clip()?;
        let p = c.pattern.as_ref()?;
        let at = self.playhead_ticks() - c.start;
        if !self.engine.transport().playing || at < 0 || at >= c.length {
            return None;
        }
        Some(((at % p.length()) / p.rate.ticks()) as u16)
    }

    /// Every frame: the playing step's ring column and hit flash, and Follow
    /// playhead turning the page when the playhead moves to another one.
    pub fn seq_tick(&mut self, s: &Studio) {
        let step = if self.seq_visible() { self.seq_play_step() } else { None };
        if step != self.pr.seq.play.map(|(st, _)| st) {
            self.pr.seq.play = step.map(|st| (st, std::time::Instant::now()));
            if let Some(st) = step.filter(|_| !self.pr.seq.follow_off) {
                let page = st / steps_per_page(self.pr.size.0);
                if page != self.pr.seq.page {
                    self.pr.seq.page = page;
                    self.ui_dirty = true;
                }
            }
        }
        let shown = self.pr.seq.play.zip(self.seq_pattern()).and_then(|((st, t0), p)| {
            let g = SeqGeom::new(self.pr.size.0, p.steps, p.rows, self.pr.seq.scroll, self.pr.seq.page);
            let col = g.col_of(st)?;
            let hits: Vec<f32> = g.visible_rows().filter(|&r| p.cell(row_key(p.rows, r), st).is_some()).map(|r| g.row_y(r)).collect();
            Some((col as i32, hits, (t0.elapsed().as_secs_f32() / FLASH.as_secs_f32()).min(1.0)))
        });
        let (col, hits, flash) = shown.unwrap_or((-1, Vec::new(), 1.0));
        if s.get_seq_play_col() != col {
            s.set_seq_play_col(col);
        }
        if s.get_seq_flash() != flash {
            s.set_seq_flash(flash);
        }
        crate::controller::sync_model(&self.seq_flash_model, hits);
    }

    /// Footer Swing: drag up to raise (phase 0 press, 1 move, 2 release; dy since press).
    pub fn seq_swing(&mut self, phase: i32, dy: f32) {
        let Some((id, p)) = self.seq_clip() else { return };
        let swing = (p.swing - dy / LANE_DRAG).clamp(0.0, 1.0);
        let swing = (swing * 100.0).round() / 100.0;
        let new = StepPattern { swing, ..p.clone() };
        if phase == 2 {
            self.pr.seq.preview = None;
            self.seq_commit(id, &p, new);
        } else {
            self.pr.seq.preview = Some(new);
            self.ui_dirty = true;
        }
    }

    pub fn seq_scroll(&mut self, dy: f32) {
        let Some(p) = self.seq_pattern() else { return };
        self.pr.seq.scroll = SeqGeom::new(self.pr.size.0, p.steps, p.rows, self.pr.seq.scroll - dy, self.pr.seq.page).scroll;
        self.ui_dirty = true;
    }

    /// Grid pointer (editor-local): kind 0 press, 1 drag, 2 release.
    pub fn seq_pointer(&mut self, kind: i32, x: f32, y: f32) {
        let Some((id, base)) = self.seq_clip() else { return };
        let shown = self.pr.seq.preview.clone().unwrap_or_else(|| base.clone());
        let g = SeqGeom::new(self.pr.size.0, shown.steps, shown.rows, self.pr.seq.scroll, self.pr.seq.page);
        let hit = g.row_at(y).map(|r| (row_key(shown.rows, r), g.step_at(x).filter(|&s| s < shown.steps)));
        match kind {
            0 => {
                self.pr.focused = true;
                let Some((key, step)) = hit else { return };
                if x >= MUTE_X && x < SOLO_X + BTN_W && step.is_none() {
                    let solo = x >= SOLO_X;
                    if x < MUTE_X + BTN_W || solo {
                        self.pr.seq.drag = Some(SeqDrag::Flag);
                        self.seq_commit(id, &base, base.with_row_flag(key, solo));
                    }
                    return;
                }
                let Some(step) = step else { return };
                let owner = base.sounding(key).into_iter().find(|(c, d)| c.step <= step && step <= c.step + d).map(|(c, _)| c);
                match (self.pr.seq.lane, owner) {
                    (Some(lane), Some(c)) => {
                        let (step, from) = (c.step, c.get(lane));
                        self.pr.seq.drag = Some(SeqDrag::Value { key, step, lane, from, x0: x, y0: y });
                        self.pr.seq.preview = Some(base.clone());
                    }
                    _ => {
                        let p = if owner.is_some_and(|c| c.step != step) { base.clone() } else { base.toggled(key, step) };
                        self.pr.seq.drag = Some(SeqDrag::Paint { on: p.cell(key, step).is_some() });
                        self.pr.seq.preview = Some(p);
                    }
                }
                self.ui_dirty = true;
            }
            1 => match self.pr.seq.drag.clone() {
                Some(SeqDrag::Paint { on }) => {
                    if let Some((key, Some(step))) = hit
                        && shown.cell(key, step).is_some() != on
                        && !(on && shown.is_covered(key, step))
                    {
                        self.pr.seq.preview = Some(shown.toggled(key, step));
                        self.ui_dirty = true;
                    }
                }
                Some(SeqDrag::Value { key, step, lane, from, x0, y0 }) => {
                    let s = lane.spec();
                    let v = if lane == StepLane::Tie {
                        from + ((x - x0) / g.pitch).round() as i32
                    } else {
                        from + ((y0 - y) / LANE_DRAG * (s.max - s.min) as f32).round() as i32
                    };
                    self.pr.seq.preview = Some(base.with_lane(key, step, lane, v.clamp(s.min, s.max)));
                    self.ui_dirty = true;
                }
                _ => {}
            },
            _ => {
                let drag = self.pr.seq.drag.take();
                if let Some(p) = self.pr.seq.preview.take()
                    && !matches!(drag, Some(SeqDrag::Flag) | None)
                {
                    self.seq_commit(id, &base, p);
                }
                self.ui_dirty = true;
            }
        }
    }

    /// Text of the footer's Sequence values: Length, Rate, Swing.
    pub fn seq_footer(&self) -> Option<(String, String, String)> {
        let p = self.seq_pattern()?;
        Some((p.steps.to_string(), p.rate.label().to_string(), format!("{}%", (p.swing * 100.0).round())))
    }

    /// Active entries of the Length and Rate menus.
    pub fn seq_menu_active(&self, rate: bool) -> i32 {
        let Some(p) = self.seq_pattern() else { return -1 };
        let i = if rate { StepRate::ALL.iter().position(|&r| r == p.rate) } else { STEP_COUNTS.iter().position(|&n| n == p.steps) };
        i.map_or(-1, |i| i as i32)
    }

    /// Rows for a new pattern clip on this track: pads on a Drum Rack track.
    pub fn seq_rows_for(&self, clip: &MidiClip) -> StepRows {
        let p = self.project();
        let rack = p.find_midi_clip(clip.id).and_then(|(t, _)| p.tracks[t].instrument_plugin.as_ref()).is_some_and(is_drum_rack);
        if rack { StepRows::Pads } else { StepRows::Pitches }
    }
}

pub fn is_drum_rack(d: &saine_core::Device) -> bool {
    builtin::def_of(d).is_some_and(|def| def.id == "drum-rack")
}

/// First pitch row on screen when a pitch pattern opens: the highest key in use, else C5.
pub fn initial_scroll(p: &StepPattern) -> f32 {
    match p.rows {
        StepRows::Pads => 0.0,
        StepRows::Pitches => {
            let top = p.cells.iter().map(|c| c.key.saturating_add(2)).max().unwrap_or(PITCH_TOP).min(127);
            (127 - top) as f32 * ROW_PITCH
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use saine_core::steps::StepLane;

    fn beat() -> StepPattern {
        let mut p = StepPattern::default();
        for s in [0, 4, 8, 12] {
            p = p.toggled(36, s);
        }
        p.toggled(37, 4).with_lane(37, 4, StepLane::Tie, 2).with_row_flag(38, false)
    }

    #[test]
    fn geometry_matches_the_reference_and_pages_long_patterns() {
        let g = SeqGeom::new(1174.5, 16, StepRows::Pads, 0.0, 0);
        assert_eq!((g.cols, g.pitch, g.rows, g.pages), (32, COL_PITCH, 16, 1));
        assert_eq!(g.visible_rows(), 0..12);
        let long = SeqGeom::new(1174.5, 100, StepRows::Pads, 0.0, 9);
        assert_eq!((long.cols, long.pages, long.page, long.first), (32, 4, 3, 96), "the page clamps to the last one");
        assert_eq!((long.col_of(97), long.col_of(95), long.step_at(COL_X0 + 1.0)), (Some(1), None, Some(96)));
        assert_eq!((steps_per_page(1174.5), steps_per_page(700.0), steps_per_page(100.0), steps_per_page(4000.0)), (32, 16, 16, 32));
        let pitches = SeqGeom::new(1174.5, 16, StepRows::Pitches, 1e9, 0);
        assert_eq!(pitches.visible_rows().end, 128, "scroll clamps to the last row");
        assert_eq!(row_key(StepRows::Pitches, 0), 127);
        assert_eq!(row_key(StepRows::Pads, 15), 51);
    }

    #[test]
    fn view_draws_ties_lanes_and_muted_rows() {
        let p = beat();
        let v = build_view(&p, 1174.5, 0.0, 0, Some(StepLane::Velocity), &|k| format!("k{k}"), Some((36, 4)));
        assert_eq!(v.rows.len(), 12);
        assert_eq!(v.rows[0].name, "k36");
        assert!(!v.rows[2].audible && v.rows[2].muted);
        let on: Vec<&CellView> = v.cells.iter().filter(|c| c.state == 2).collect();
        assert_eq!(on.len(), 5);
        let tied = on.iter().find(|c| c.y == v.rows[1].y).unwrap();
        assert_eq!(tied.w, v.geom.cell_w + 2.0 * v.geom.pitch);
        assert!((on[0].bar - 100.0 / 127.0).abs() < 1e-6);
        assert_eq!(on[1].label, "100", "the dragged cell shows its value");
        assert_eq!(v.cells.iter().filter(|c| c.state == 0).count(), 12 * 16);
        let beats: Vec<&str> = v.nums.iter().filter(|n| n.beat && n.active).map(|n| n.text.as_str()).collect();
        assert_eq!(beats, ["1", "5", "9", "13"]);
    }

    #[test]
    fn a_later_page_shows_its_steps_and_cuts_ties_at_the_edges() {
        let p = StepPattern { steps: 80, ..StepPattern::default() }.toggled(36, 30).with_lane(36, 30, StepLane::Tie, 4).toggled(37, 70);
        let v = build_view(&p, 1174.5, 0.0, 1, None, &|k| format!("k{k}"), None);
        assert_eq!((v.nums[0].text.as_str(), v.nums.len()), ("33", 32));
        let on: Vec<&CellView> = v.cells.iter().filter(|c| c.state == 2).collect();
        assert_eq!(on.len(), 1, "step 71 is on page 3");
        assert_eq!((on[0].x, on[0].w), (COL_X0, v.geom.cell_w + 2.0 * v.geom.pitch), "the tie from step 31 shows its last three steps");
        let last = build_view(&p, 1174.5, 0.0, 2, None, &|k| format!("k{k}"), None);
        assert_eq!(last.cells.iter().filter(|c| c.y == last.rows[0].y && c.state == 1).count(), 16, "steps 65..80 in the pattern, the rest past its end");
    }

    /// Cost of the grid's view model, built on every UI update while the sequencer is open.
    /// cargo test -p saine-app --release stepseq_view_bench -- --ignored --nocapture
    #[test]
    #[ignore]
    fn stepseq_view_bench() {
        let mut full = StepPattern { steps: 64, rate: StepRate::ThirtySecond, ..StepPattern::default() };
        for k in 36..52 {
            for s in 0..64 {
                full = full.toggled(k, s);
            }
        }
        for (name, p, lane) in [("16 steps, beat", beat(), None), ("64 steps, every cell on", full.clone(), None), ("64 steps, every cell on, Velocity", full, Some(StepLane::Velocity))] {
            let mut t: Vec<f64> = (0..2000)
                .map(|_| {
                    let s = std::time::Instant::now();
                    std::hint::black_box(build_view(&p, 1174.5, 0.0, 0, lane, &|k| format!("Pad {}", k - 35), None));
                    s.elapsed().as_secs_f64() * 1e6
                })
                .collect();
            t.sort_by(f64::total_cmp);
            println!("{name:<36} median {:>7.1} us  p99 {:>7.1} us", t[1000], t[1980]);
        }
    }
}

