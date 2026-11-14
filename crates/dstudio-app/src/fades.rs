//! Audio clip fades in the timeline: the square handles at a clip's top corners (shown while
//! the clip is hovered or selected) drag its fade-in and fade-out lengths.

use crate::controller::Controller;
use crate::timeline;
use dstudio_core::{ClipFades, ClipId, Command, PPQ, Ticks};

/// A fade handle being dragged.
#[derive(Debug, Clone, Copy)]
pub struct FadeDrag {
    pub clip: ClipId,
    /// The fade-out handle (else the fade-in one).
    pub out: bool,
    /// Clip start and timeline length (ticks) when the drag began.
    start: Ticks,
    len: Ticks,
    orig: ClipFades,
    pub cur: ClipFades,
}

/// The fades while the `out` handle (or the fade-in one) is at tick `at` on the timeline of a
/// clip `start`..`start + len`: the dragged fade ends there, kept within what the other leaves.
pub fn dragged_fades(f: ClipFades, out: bool, start: Ticks, len: Ticks, at: Ticks) -> ClipFades {
    let f = f.clamped(len);
    if out {
        ClipFades { fade_out: (start + len - at).clamp(0, len - f.fade_in), ..f }
    } else {
        ClipFades { fade_in: (at - start).clamp(0, len - f.fade_out), ..f }
    }
}

impl Controller {
    /// The fades an audio clip shows: its own (clamped to its length), or the drag's.
    pub(crate) fn shown_fades(&self, id: ClipId, own: ClipFades, len: Ticks) -> (f64, f64) {
        let f = match self.fade_drag {
            Some(d) if d.clip == id => d.cur,
            _ => own.clamped(len),
        };
        (f.fade_in as f64 / PPQ as f64, f.fade_out as f64 / PPQ as f64)
    }

    /// Whether a clip shows its fade handles: hovered, selected or being faded.
    pub(crate) fn shows_fade_handles(&self, id: ClipId) -> bool {
        self.hover_clip == Some(id) || self.is_clip_selected(id) || self.fade_drag.is_some_and(|d| d.clip == id)
    }

    /// Lane hover: remembers the audio clip under the pointer so its handles show.
    pub fn clip_hover(&mut self, x: f32, y: f32) {
        let hit = match timeline::hit_test(&self.view, self.lanes_size, self.lane_tracks(), &self.clip_vis, x, y) {
            timeline::Hit::Clip { index, .. } if self.clip_vis[index].midi.is_none() => self.clip_ids.get(index).copied(),
            // Handles just outside a narrow clip still count as the clip.
            _ => timeline::fade_hit(&self.view, &self.clip_vis, x, y).and_then(|(i, _)| self.clip_ids.get(i).copied()),
        };
        if hit != self.hover_clip {
            self.hover_clip = hit;
            self.ui_dirty = true;
        }
    }

    /// Press at lane (x, y): takes a fade handle if one is there.
    pub(crate) fn fade_press(&mut self, x: f32, y: f32) -> bool {
        let Some((index, out)) = timeline::fade_hit(&self.view, &self.clip_vis, x, y) else { return false };
        let Some(&id) = self.clip_ids.get(index) else { return false };
        let p = self.project();
        let Some(c) = p.clip(id) else { return false };
        let (start, len, orig) = (c.start, p.clip_length_ticks(c), c.fades);
        if !self.is_clip_selected(id) {
            if let Some(track) = self.clip_track(id) {
                self.select_track(track);
            }
            self.select_clip(Some(id));
        }
        self.fade_drag = Some(FadeDrag { clip: id, out, start, len, orig, cur: orig.clamped(len) });
        self.ui_dirty = true;
        true
    }

    /// Drag a fade handle to lane x (follows the pointer; no snapping).
    pub(crate) fn fade_move(&mut self, x: f32) {
        let Some(mut d) = self.fade_drag else { return };
        let at = (self.view.beat_at(x) * PPQ as f64).round() as Ticks;
        d.cur = dragged_fades(d.cur, d.out, d.start, d.len, at);
        self.fade_drag = Some(d);
        self.ui_dirty = true;
    }

    /// Release: one undo step when the fades changed.
    pub(crate) fn fade_release(&mut self) {
        let Some(d) = self.fade_drag.take() else { return };
        self.ui_dirty = true;
        if d.cur != d.orig.clamped(d.len) {
            self.execute(Command::SetClipFades { clip: d.clip, fades: d.cur });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dragging_a_handle_sets_its_fade_within_the_clip() {
        let f = ClipFades { fade_in: PPQ, fade_out: PPQ };
        // Clip at beat 4, 8 beats long.
        let (s, l) = (4 * PPQ, 8 * PPQ);
        assert_eq!(dragged_fades(f, false, s, l, 7 * PPQ), ClipFades { fade_in: 3 * PPQ, fade_out: PPQ });
        assert_eq!(dragged_fades(f, true, s, l, 9 * PPQ), ClipFades { fade_in: PPQ, fade_out: 3 * PPQ });
        // Past the other fade: stops where it begins. Before the clip: no fade.
        assert_eq!(dragged_fades(f, false, s, l, 20 * PPQ), ClipFades { fade_in: 7 * PPQ, fade_out: PPQ });
        assert_eq!(dragged_fades(f, false, s, l, 0), ClipFades { fade_in: 0, fade_out: PPQ });
        assert_eq!(dragged_fades(f, true, s, l, 20 * PPQ), ClipFades { fade_in: PPQ, fade_out: 0 });
    }

    #[test]
    fn handles_sit_under_the_title_strip_inside_the_clip() {
        let v = timeline::View::default();
        let mut c = timeline::ClipVis {
            track_index: 0,
            start_beats: 4.0,
            len_beats: 8.0,
            color: [1.0; 3],
            selected: false,
            muted: false,
            name: String::new(),
            wave: None,
            midi: None,
            live: None,
            fades: (0.0, 0.0),
            fade_handles: false,
        };
        c.fade_handles = true;
        c.fades = (2.0, 0.0);
        let (x, y, w, _) = c.rect(&v);
        let [(ix, iy), (ox, _)] = c.fade_handle_centres(&v);
        assert!((ix - (x + 2.0 * v.px_per_beat as f32)).abs() < 1e-3);
        assert!(iy > y + timeline::CLIP_HEADER && iy < y + timeline::CLIP_HEADER + 2.0 * timeline::FADE_GRAB);
        assert!(ox < x + w && ox > x + w - timeline::FADE_GRAB);
        assert_eq!(timeline::fade_hit(&v, &[c.clone()], ix + 1.0, iy), Some((0, false)));
        assert_eq!(timeline::fade_hit(&v, &[c.clone()], ox, iy + 1.0), Some((0, true)));
        assert_eq!(timeline::fade_hit(&v, &[c.clone()], (ix + ox) / 2.0, iy), None);
        c.fade_handles = false;
        assert_eq!(timeline::fade_hit(&v, &[c], ix, iy), None);
    }
}
