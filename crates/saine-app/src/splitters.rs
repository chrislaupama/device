//! Splitters: the timeline / bottom-panel splitter and per-track height handles.
//! Both preview while dragging and commit on release. Like the reference, both are view
//! changes: saved with the project, never undo steps.

use crate::controller::{BottomTab, Controller};
use crate::highlight::{DEFAULT_BOTTOM_H, MIXER_BOTTOM_H};
use crate::timeline::{MIN_TRACK_PITCH, TRACK_PITCH};
use saine_core::{Track, TrackId};

/// Bottom panel collapsed to its tab row (26.5 pt row plus its rule).
pub const BOTTOM_COLLAPSED_H: f32 = 27.0;
/// Below this the splitter snaps the panel shut.
const BOTTOM_SNAP_H: f32 = 80.0;
/// Room always left for the browser / timeline row.
const MIN_TOP_H: f32 = 150.0;
/// Window chrome above and below the two rows (app.slint `top-h`).
const CHROME_H: f32 = 88.4;

/// A track-height drag in progress.
#[derive(Debug, Clone, Copy)]
pub struct TrackResize {
    pub track: TrackId,
    start_y: f32,
    start_pitch: f32,
    pub scale: f32,
}

/// A splitter drag in progress: pointer y and panel height at the press.
#[derive(Debug, Clone, Copy)]
pub struct BottomDrag {
    start_y: f32,
    start_h: f32,
    pub h: f32,
}

impl Controller {
    fn bottom_kind(&self) -> &'static str {
        if self.mixer_open {
            "mixer"
        } else if self.bottom_tab == BottomTab::Clip {
            "clip"
        } else {
            "devices"
        }
    }

    /// Current bottom panel height (window points): the live drag, the project's saved
    /// height for this panel kind, else the default.
    pub fn bottom_h(&self) -> f32 {
        if let Some(d) = self.bottom_drag {
            return d.h;
        }
        let default = if self.mixer_open || self.bottom_tab == BottomTab::Clip { MIXER_BOTTOM_H } else { DEFAULT_BOTTOM_H };
        let saved = self.project().bottom_panel_heights.get(self.bottom_kind()).copied().unwrap_or(default);
        self.clamp_bottom_h(saved)
    }

    fn clamp_bottom_h(&self, h: f32) -> f32 {
        let max = (self.devices.win.1 - CHROME_H - MIN_TOP_H).max(BOTTOM_COLLAPSED_H);
        if h < BOTTOM_SNAP_H { BOTTOM_COLLAPSED_H } else { h.min(max) }
    }

    /// Splitter drag (window y): phase 0 press, 1 move, 2 release.
    pub fn bottom_resize(&mut self, phase: i32, y: f32) {
        match phase {
            0 => {
                let h = self.bottom_h();
                self.bottom_drag = Some(BottomDrag { start_y: y, start_h: h, h });
            }
            1 => {
                if let Some(d) = self.bottom_drag {
                    let h = self.clamp_bottom_h(d.start_h - (y - d.start_y));
                    self.bottom_drag = Some(BottomDrag { h, ..d });
                    self.ui_dirty = true;
                }
            }
            _ => {
                if let Some(d) = self.bottom_drag.take() {
                    let kind = self.bottom_kind();
                    self.doc.set_bottom_panel_height(kind, Some(d.h));
                    self.ui_dirty = true;
                }
            }
        }
    }

    /// Track bottom-edge handle drag (window y): phase 0 press, 1 move, 2 release.
    pub fn track_resize_drag(&mut self, id: i32, phase: i32, y: f32) {
        let track = TrackId(id as u64);
        match phase {
            0 => {
                let Some(i) = self.project().track_index(track) else { return };
                let scale = self.project().tracks[i].height;
                self.track_resize = Some(TrackResize { track, start_y: y, start_pitch: self.view.pitch(i), scale });
            }
            1 => {
                if let Some(r) = self.track_resize {
                    let (lo, hi) = Track::HEIGHT_RANGE;
                    let pitch = (r.start_pitch + y - r.start_y).max(MIN_TRACK_PITCH);
                    self.track_resize = Some(TrackResize { scale: (pitch / TRACK_PITCH).clamp(lo, hi), ..r });
                    self.ui_dirty = true;
                }
            }
            _ => {
                if let Some(r) = self.track_resize.take() {
                    let _ = self.doc.set_track_height(r.track, r.scale);
                    self.ui_dirty = true;
                }
            }
        }
    }
}
