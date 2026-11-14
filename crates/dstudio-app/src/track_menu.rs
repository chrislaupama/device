//! Track header menu (right-click a track header): Rename, Duplicate track, Duplicate track with
//! clips, Color and Delete track; and dragging a track header up or down to reorder the tracks.

use crate::controller::{Controller, MenuAction, slint_color, track_color, track_palette_len};
use crate::{MenuItem, Studio};
use dstudio_core::{ColorIndex, Command, TrackId};

/// Pointer travel (px) before a press on a track header becomes a reorder drag.
const DRAG_START: f32 = 4.0;
/// Window y of the top of the track header list (the timeline panel at 78.2, rows from 66.5; ui/app.slint).
const LIST_TOP: f32 = 78.2 + 66.5;

/// Track colour names, in palette order (`controller::TRACK_PALETTE`).
const COLOR_NAMES: [&str; 13] =
    ["Sky", "Periwinkle", "Violet", "Purple", "Pink", "Orange", "Teal", "Coral", "Amber", "Lime", "Green", "Mint", "Aqua"];

/// A press on a track header that may become a reorder drag.
#[derive(Debug, Clone, Copy)]
pub struct TrackDrag {
    pub track: TrackId,
    start_y: f32,
    /// Past [DRAG_START]: the drop line shows at `gap`.
    pub active: bool,
    /// Track index the dragged track would be inserted before (its group's end for the last place).
    pub gap: Option<usize>,
}

/// Index a track moves to when dropped before `gap`, from index `from`: the gap counts the
/// track itself, so a gap below it is one less once it is taken out.
pub fn move_target(from: usize, gap: usize) -> usize {
    if gap > from { gap - 1 } else { gap }
}

impl Controller {
    /// Right press on a track header at window (wx, wy).
    pub fn open_track_menu(&mut self, id: TrackId, wx: f32, wy: f32) {
        let Some(t) = self.project().track(id) else { return };
        let is_return = t.is_return();
        let can_copy = !is_return || self.project().returns().count() < dstudio_core::MAX_RETURNS;
        self.select_track(id);
        use MenuAction::*;
        let mut items = vec![(TrackRename, true), (TrackDuplicate, can_copy)];
        if !is_return {
            items.push((TrackDuplicateClips, true));
        }
        items.push((TrackColor, true));
        items.push((TrackDelete, true));
        let win = self.devices.win;
        self.open_menu(items, wx, wy, win);
        self.menu.track = Some(id);
    }

    pub fn track_menu_activate(&mut self, id: TrackId, action: MenuAction) {
        if action == MenuAction::TrackColor {
            self.menu.sub_open = !(self.menu.sub_open && self.menu.sub_for == Some(action));
            self.menu.sub_for = Some(action);
            self.ui_dirty = true;
            return;
        }
        self.menu_close();
        match action {
            MenuAction::TrackRename => {
                self.selected_track = Some(id);
                self.open_rename_field();
            }
            MenuAction::TrackDuplicate => self.duplicate_track(id, false),
            MenuAction::TrackDuplicateClips => self.duplicate_track(id, true),
            MenuAction::TrackDelete => {
                self.selected_clip = None;
                self.selected_section = None;
                self.selected_track = Some(id);
                self.delete_selection();
            }
            _ => {}
        }
    }

    /// Copy track `id` (devices, automation, sends and mix settings; its clips too with
    /// `with_clips`) into a new track right below it, and select the copy. One undo step.
    pub fn duplicate_track(&mut self, id: TrackId, with_clips: bool) {
        let p = self.project();
        let (Some(i), Some(copy)) = (p.track_index(id), p.track_copy(id, with_clips)) else { return };
        let new = copy.id;
        let label = if with_clips { "Duplicate Track with Clips" } else { "Duplicate Track" };
        let cmd = Command::Batch { label: label.into(), commands: vec![Command::AddTrack { index: Some(i + 1), track: copy }] };
        if self.execute(cmd) {
            self.select_track(new);
            self.ui_dirty = true;
        }
    }

    /// Colour submenu rows: a swatch per palette colour, the track's own ticked.
    pub fn track_color_rows(&self) -> (Vec<MenuItem>, f32) {
        let current = self.menu.track.and_then(|t| self.project().track(t)).map(|t| t.color.0 as usize % track_palette_len());
        let (mut rows, h) = crate::clipedit::layout_items(
            COLOR_NAMES
                .iter()
                .enumerate()
                .map(|(i, n)| (n.to_string(), if current == Some(i) { "✓".into() } else { String::new() }, true, false, false)),
        );
        for (i, r) in rows.iter_mut().enumerate() {
            r.swatch = slint_color(track_color(ColorIndex(i as u8)));
        }
        (rows, h)
    }

    pub fn track_color_pick(&mut self, id: TrackId, index: usize) {
        self.menu_close();
        if index >= COLOR_NAMES.len() {
            return;
        }
        let color = ColorIndex(index as u8);
        if self.project().track(id).is_some_and(|t| t.color.0 as usize % track_palette_len() != index) {
            self.execute(Command::SetTrackColor { track: id, color });
        }
    }

    /// Track header press (window y): phase 0 press, 1 move, 2 release, 3 cancel. A press
    /// that travels [DRAG_START] becomes a drag that shows a drop line between the rows of the
    /// track's group (ordinary tracks, or the returns); release moves the track there.
    pub fn track_drag(&mut self, id: TrackId, phase: i32, y: f32) {
        match phase {
            0 => {
                // Pinned return rows don't scroll with the list; they keep their order.
                let pinned = self.project().track_index(id).is_none_or(|i| self.view.is_pinned(i));
                self.track_drag = (!pinned).then_some(TrackDrag { track: id, start_y: y, active: false, gap: None });
            }
            1 => {
                let Some(mut d) = self.track_drag else { return };
                if !d.active && (y - d.start_y).abs() < DRAG_START {
                    return;
                }
                d.active = true;
                d.gap = self.track_gap(d.track, y);
                self.track_drag = Some(d);
                self.ui_dirty = true;
            }
            2 => {
                let Some(d) = self.track_drag.take() else { return };
                self.ui_dirty |= d.active;
                let (Some(gap), Some(from)) = (d.gap.filter(|_| d.active), self.project().track_index(d.track)) else { return };
                let to = move_target(from, gap);
                if to != from {
                    self.execute(Command::MoveTrack { track: d.track, index: to });
                }
            }
            _ => {
                self.ui_dirty |= self.track_drag.take().is_some_and(|d| d.active);
            }
        }
    }

    /// The gap (insert-before index) under window y for dragging `track`: within its group, the
    /// first row whose middle is below y, else the group's end.
    fn track_gap(&self, track: TrackId, y: f32) -> Option<usize> {
        let p = self.project();
        let i = p.track_index(track)?;
        let ordinary = p.ordinary_track_count();
        let (lo, hi) = if p.tracks[i].is_return() { (ordinary, p.tracks.len()) } else { (0, ordinary) };
        let hi = hi.min(self.view.pinned_from);
        let content_y = y - LIST_TOP + self.view.scroll_y as f32;
        Some((lo..hi).find(|&k| self.view.track_offset(k) + self.view.track_span(k) / 2.0 > content_y).unwrap_or(hi))
    }

    /// Drop line for the header list (content y), while a reorder drag is on.
    pub fn push_track_drag(&self, s: &Studio) {
        let line = self.track_drag.filter(|d| d.active).and_then(|d| d.gap);
        s.set_track_drop_on(line.is_some());
        if let Some(g) = line {
            s.set_track_drop_y(self.view.track_offset(g));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drop_gaps_map_to_move_indices() {
        // Track 1 of 0..4: gaps 0 and 1 leave it at 0 and 1 (1 = where it is), gap 3 puts it at 2.
        assert_eq!(move_target(1, 0), 0);
        assert_eq!(move_target(1, 1), 1);
        assert_eq!(move_target(1, 2), 1);
        assert_eq!(move_target(1, 3), 2);
        assert_eq!(move_target(1, 4), 3);
    }

    #[test]
    fn every_palette_colour_has_a_name() {
        assert_eq!(COLOR_NAMES.len(), track_palette_len());
    }
}
