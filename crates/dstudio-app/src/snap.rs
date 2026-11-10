//! Timeline snap: the toolbar readout and its menu (Off, Auto
//! and the grid sizes), and the beat a clip drag snaps to. Auto follows the ruler's step for the
//! zoom; a fixed size overrides it (finer sizes only, triplets always). Holding Cmd as a drag
//! starts moves clips off the grid.

use crate::controller::{Controller, sync_model};
use crate::timeline::GridSize;
use crate::{ChoiceItem, Studio};
use slint::VecModel;
use std::rc::Rc;

/// Menu metrics in design units (22 px rows and 4 px padding x 0.85).
const ROW_H: f32 = 18.7;
const PAD: f32 = 3.4;
const RULE_GAP: f32 = 6.8;
pub const MENU_W: f32 = 108.8;

pub struct State {
    /// Snap to grid on (the readout shows "Off" otherwise).
    pub on: bool,
    /// `None` is Auto.
    pub grid: Option<GridSize>,
    /// Cmd was held when the current drag started: this drag ignores the grid.
    pub bypass: bool,
    pub menu: Option<(f32, f32)>,
    pub model: Rc<VecModel<ChoiceItem>>,
}

impl State {
    pub fn new() -> State {
        State { on: true, grid: None, bypass: false, menu: None, model: Rc::new(VecModel::default()) }
    }

    /// The readout text: "Off", "Auto" or the grid size.
    pub fn text(&self) -> &'static str {
        if !self.on { "Off" } else { self.grid.map_or("Auto", GridSize::label) }
    }

    /// Menu rows in order: Off, Auto, then the grid sizes.
    pub fn row_count() -> usize {
        2 + GridSize::ALL.len()
    }

    /// What picking row `i` does.
    pub fn pick(&mut self, i: usize) {
        match i {
            0 => self.on = false,
            1 => (self.on, self.grid) = (true, None),
            _ => {
                if let Some(g) = GridSize::ALL.get(i - 2) {
                    (self.on, self.grid) = (true, Some(*g));
                }
            }
        }
    }

    /// Menu rows with their y offsets and the two rules' y.
    pub fn rows(&self) -> (Vec<ChoiceItem>, f32, f32, f32) {
        let mut rows = Vec::new();
        let mut y = PAD;
        let mut rules = [0.0f32; 2];
        for i in 0..Self::row_count() {
            if i == 1 || i == 2 + GridSize::STRAIGHT {
                rules[(i > 1) as usize] = y + RULE_GAP / 2.0;
                y += RULE_GAP;
            }
            let (label, checked) = match i {
                0 => ("Off", !self.on),
                1 => ("Auto", self.on && self.grid.is_none()),
                _ => {
                    let g = GridSize::ALL[i - 2];
                    (g.label(), self.on && self.grid == Some(g))
                }
            };
            rows.push(ChoiceItem { label: label.into(), checked, hint: false, y, h: ROW_H });
            y += ROW_H;
        }
        (rows, y + PAD, rules[0], rules[1])
    }
}

impl Controller {
    pub fn snap_toggle(&mut self) {
        self.snap.on = !self.snap.on;
        self.ui_dirty = true;
    }

    pub fn snap_menu_toggle(&mut self, x: f32, y: f32) {
        self.snap.menu = if self.snap.menu.is_some() { None } else { Some((x, y)) };
        self.ui_dirty = true;
    }

    pub fn snap_menu_pick(&mut self, i: usize) {
        self.snap.pick(i);
        self.snap.menu = None;
        self.ui_dirty = true;
    }

    /// Whether a drag now snaps: snapping is on and Cmd was not held at the press.
    pub fn snap_active(&self) -> bool {
        self.snap.on && !self.snap.bypass
    }

    pub(crate) fn push_snap(&mut self, s: &Studio) {
        if s.get_snap_text() != self.snap.text() {
            s.set_snap_text(self.snap.text().into());
        }
        let Some((x, y)) = self.snap.menu else {
            s.set_snap_menu_open(false);
            return;
        };
        let (rows, h, r1, r2) = self.snap.rows();
        sync_model(&self.snap.model, rows);
        s.set_snap_menu_h(h);
        s.set_snap_menu_w(MENU_W);
        s.set_snap_menu_x(x);
        s.set_snap_menu_y(y);
        s.set_snap_menu_rule1(r1);
        s.set_snap_menu_rule2(r2);
        s.set_snap_menu_open(true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readout_and_picks() {
        let mut s = State::new();
        assert_eq!(s.text(), "Auto");
        s.pick(5);
        assert_eq!((s.on, s.grid, s.text()), (true, Some(GridSize::Eighth), "1/8"));
        s.pick(0);
        assert_eq!((s.on, s.text()), (false, "Off"));
        s.pick(11);
        assert_eq!((s.on, s.text()), (true, "1/32T"));
        s.pick(1);
        assert_eq!((s.grid, s.text()), (None, "Auto"));
    }

    #[test]
    fn menu_checks_one_row_and_rules_sit_between_sections() {
        let mut s = State::new();
        let (rows, h, r1, r2) = s.rows();
        assert_eq!(rows.len(), 12);
        assert_eq!(rows.iter().filter(|r| r.checked).count(), 1);
        assert!(rows[1].checked);
        assert!(r1 > rows[0].y + rows[0].h && r1 < rows[1].y);
        assert!(r2 > rows[7].y + rows[7].h && r2 < rows[8].y);
        assert!(h > rows[11].y + rows[11].h);
        s.pick(0);
        assert!(s.rows().0[0].checked && !s.rows().0[1].checked);
    }
}
