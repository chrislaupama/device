//! Splitters: the timeline / bottom-panel splitter, per-track height handles and the docked
//! assistant's width splitter. All preview while dragging and commit on release.
//! They are view changes, never undo steps: the first two are saved with the project,
//! the chat width with the app preferences.

use crate::controller::{BottomTab, Controller};
use crate::highlight::{DEFAULT_BOTTOM_H, MIXER_BOTTOM_H};
use crate::timeline::{MIN_TRACK_PITCH, TRACK_PITCH};
use dstudio_core::{Track, TrackId};

/// Bottom panel collapsed to its tab row (26.5 pt row plus its rule).
pub const BOTTOM_COLLAPSED_H: f32 = 27.0;
/// Below this the splitter snaps the panel shut.
const BOTTOM_SNAP_H: f32 = 80.0;
/// Room always left for the browser / timeline row.
const MIN_TOP_H: f32 = 150.0;
/// Window chrome above and below the two rows (app.slint `top-h`).
const CHROME_H: f32 = 88.4;

/// Docked assistant width (297 px default and minimum, 600 px maximum at 100% zoom; design units).
pub const CHAT_MIN_W: f32 = 252.45;
pub const CHAT_DEFAULT_W: f32 = CHAT_MIN_W;
pub const CHAT_MAX_W: f32 = 510.0;
/// Gap between the timeline and the docked assistant, and the window margin on the right.
pub const CHAT_GAP: f32 = 5.0;
const WINDOW_MARGIN: f32 = 5.0;
/// Timeline panel width below which the header can no longer give its minimap the 140 px
/// minimum while the zoom sliders are shown (the width of the header's content, design units).
const TIMELINE_ZOOM_W: f32 = 711.5;
/// Pointer offset into the splitter strip that a drag keeps (3 px).
const CHAT_GRAB: f32 = 2.55;

fn timeline_x(sounds: bool) -> f32 {
    if sounds { 262.65 } else { 5.0 }
}

/// Widest the docked assistant can be while the header keeps its zoom sliders. The header
/// narrows the chat first; only when even the minimum chat does not fit do the sliders hide.
pub fn chat_room(win_w: f32, sounds: bool) -> f32 {
    win_w - timeline_x(sounds) - WINDOW_MARGIN - CHAT_GAP - TIMELINE_ZOOM_W
}

/// Chat width on screen: the saved width, narrowed to the room, never below the minimum.
/// (The saved width itself is untouched, so the chat grows back when the window does.)
pub fn chat_width(saved: f32, win_w: f32, sounds: bool) -> f32 {
    saved.clamp(CHAT_MIN_W, CHAT_MAX_W).min(chat_room(win_w, sounds)).max(CHAT_MIN_W)
}

/// Widest a splitter drag can make the chat right now.
pub fn chat_drag_max(win_w: f32, sounds: bool) -> f32 {
    chat_room(win_w, sounds).min(CHAT_MAX_W).max(CHAT_MIN_W)
}

/// Whether the header hides its zoom sliders (mirrors `zoom-hidden` in ui/app.slint).
pub fn zoom_hidden(win_w: f32, sounds: bool, chat_open: bool) -> bool {
    let room = chat_room(win_w, sounds);
    if chat_open { room < CHAT_MIN_W } else { room + CHAT_GAP < 0.0 }
}

/// A docked-assistant width drag in progress.
#[derive(Debug, Clone, Copy)]
pub struct ChatDrag {
    grab: f32,
    pub w: f32,
}

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
        // The clip editor and Devices tab share one height (the tab row at 454 pt in a
        // 820 pt window); only the mixer is taller.
        let default = if self.mixer_open { MIXER_BOTTOM_H } else { DEFAULT_BOTTOM_H };
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

    /// Saved docked-assistant width (app preference; the default when never set).
    pub fn chat_saved_w(&self) -> f32 {
        let w = self.settings.prefs.assistant_width;
        if w > 0.0 { w.clamp(CHAT_MIN_W, CHAT_MAX_W) } else { CHAT_DEFAULT_W }
    }

    /// The width the panel is drawn at: the live drag, else the saved width narrowed to the room.
    pub fn chat_w(&self) -> f32 {
        self.chat_w_at(self.devices.win.0, self.devices.sounds)
    }

    pub fn chat_w_at(&self, win_w: f32, sounds: bool) -> f32 {
        match self.chat_drag {
            Some(d) => d.w.min(chat_drag_max(win_w, sounds)),
            None => chat_width(self.chat_saved_w(), win_w, sounds),
        }
    }

    /// Splitter drag (window x): phase 0 press, 1 move, 2 release.
    pub fn chat_resize(&mut self, phase: i32, x: f32) {
        let (win_w, sounds) = (self.devices.win.0, self.devices.sounds);
        let right = win_w - WINDOW_MARGIN;
        match phase {
            0 => {
                let w = self.chat_w();
                self.chat_drag = Some(ChatDrag { grab: ((right - x) - w).max(CHAT_GRAB), w });
            }
            1 => {
                if let Some(d) = self.chat_drag {
                    let w = (right - x - d.grab).clamp(CHAT_MIN_W, chat_drag_max(win_w, sounds));
                    self.chat_drag = Some(ChatDrag { w, ..d });
                    self.ui_dirty = true;
                }
            }
            _ => {
                if let Some(d) = self.chat_drag.take() {
                    self.settings.prefs.assistant_width = d.w;
                    self.save_prefs();
                    self.ui_dirty = true;
                }
            }
        }
    }

    /// Double-click on the splitter: back to the default width.
    pub fn chat_resize_reset(&mut self) {
        self.chat_drag = None;
        self.settings.prefs.assistant_width = 0.0;
        self.save_prefs();
        self.ui_dirty = true;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_narrows_to_the_room_and_never_below_the_minimum() {
        // Wide window: the saved width is kept (clamped to 297..600 px).
        assert_eq!(chat_width(400.0, 1600.0, true), 400.0);
        assert_eq!(chat_width(900.0, 2400.0, true), CHAT_MAX_W);
        assert_eq!(chat_width(10.0, 2400.0, true), CHAT_MIN_W);
        // Narrower: the chat gives up room until the minimum.
        let room = chat_room(1300.0, true);
        assert!((room - 315.85).abs() < 1e-3, "{room}");
        assert_eq!(chat_width(510.0, 1300.0, true), room);
        assert_eq!(chat_width(510.0, 1100.0, true), CHAT_MIN_W);
        // The saved width is not changed by narrowing: back in a wide window it is restored.
        assert_eq!(chat_width(510.0, 2000.0, true), 510.0);
        // Without the browser the timeline starts further left, so there is more room.
        assert!(chat_room(1300.0, false) > chat_room(1300.0, true));
    }

    #[test]
    fn zoom_sliders_hide_only_after_the_chat_is_at_its_minimum() {
        // Chat open: sliders stay while the room fits the minimum chat, however wide the saved chat is.
        let w_edge = 262.65 + 5.0 + CHAT_GAP + TIMELINE_ZOOM_W + CHAT_MIN_W + 0.01;
        assert!(!zoom_hidden(w_edge, true, true));
        assert!(zoom_hidden(w_edge - 0.5, true, true));
        assert!((chat_width(510.0, w_edge, true) - CHAT_MIN_W).abs() < 0.02);
        // Chat closed: the sliders hide when the timeline itself is narrower than its need.
        let w_closed = 262.65 + 5.0 + TIMELINE_ZOOM_W + 0.01;
        assert!(!zoom_hidden(w_closed, true, false));
        assert!(zoom_hidden(w_closed - 0.5, true, false));
    }

    #[test]
    fn drag_limits() {
        assert_eq!(chat_drag_max(2400.0, true), CHAT_MAX_W);
        assert_eq!(chat_drag_max(1300.0, true), chat_room(1300.0, true));
        assert_eq!(chat_drag_max(900.0, true), CHAT_MIN_W);
    }
}
