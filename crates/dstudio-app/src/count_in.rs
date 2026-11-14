//! Count-in: the record button's menu sets one or two bars of metronome before a recording.
//! Recording starts that many bars early with the click on, and what is recorded before the
//! punch-in point (where the playhead was) is dropped: audio takes are trimmed to it and MIDI
//! notes played before it are left out. Sample-accurate, since the takes themselves are cut.

use crate::controller::{Controller, MenuAction};
use dstudio_core::midi_record::CapturedEvent;

/// Bars a count-in can last.
pub const MAX_BARS: u8 = 2;

/// The first timeline frame a take starting at `start` keeps: none before 0 (latency
/// compensation can start a take early) and none before the punch-in point.
pub fn first_kept_frame(start: i64, punch: Option<i64>) -> i64 {
    start.max(0).max(punch.unwrap_or(i64::MIN))
}

/// A poll's MIDI events without the notes started before the punch-in point. Note-offs are
/// kept (one without its note-on is ignored). Once the loop wraps, frames before the punch
/// point are real takes again, so the filter ends.
pub fn drop_count_in(punch: &mut Option<i64>, events: Vec<CapturedEvent>, wrapped: bool) -> Vec<CapturedEvent> {
    let Some(p) = *punch else { return events };
    if wrapped {
        *punch = None;
        return events;
    }
    events.into_iter().filter(|e| !(e.on && e.frame < p)).collect()
}

/// Where a count-in of `bars` bars starts for a punch-in at `punch_ticks` (ticks; may be
/// before 0, the engine plays negative positions as silence).
pub fn count_in_start(punch_ticks: i64, bars: u8, ticks_per_bar: i64) -> i64 {
    punch_ticks - bars as i64 * ticks_per_bar
}

impl Controller {
    /// Bars of count-in (0 = off).
    pub fn count_in_bars(&self) -> u8 {
        self.settings.prefs.count_in_bars.min(MAX_BARS)
    }

    pub fn set_count_in_bars(&mut self, bars: u8) {
        let bars = bars.min(MAX_BARS);
        if self.settings.prefs.count_in_bars != bars {
            self.settings.prefs.count_in_bars = bars;
            self.save_prefs();
        }
        self.set_status(
            match bars {
                0 => "Count-in off".to_string(),
                1 => "Count-in: 1 bar".to_string(),
                n => format!("Count-in: {n} bars"),
            },
            2,
        );
        self.ui_dirty = true;
    }

    /// Right-click on the record button (window x, y): the count-in menu.
    pub fn open_count_in_menu(&mut self, wx: f32, wy: f32) {
        use MenuAction::*;
        let win = self.devices.win;
        self.open_menu(vec![(CountInOff, true), (CountIn1, true), (CountIn2, true)], wx, wy, win);
        self.menu.count_in = true;
    }

    pub(crate) fn count_in_menu_activate(&mut self, action: MenuAction) {
        self.menu_close();
        match action {
            MenuAction::CountInOff => self.set_count_in_bars(0),
            MenuAction::CountIn1 => self.set_count_in_bars(1),
            MenuAction::CountIn2 => self.set_count_in_bars(2),
            _ => {}
        }
    }

    /// The menu's right column: a tick on the current count-in, shortcuts elsewhere.
    pub(crate) fn menu_shortcut(&self, a: MenuAction) -> String {
        let bars = match a {
            MenuAction::CountInOff => 0,
            MenuAction::CountIn1 => 1,
            MenuAction::CountIn2 => 2,
            _ => return a.shortcut().to_string(),
        };
        if bars == self.count_in_bars() { "✓".into() } else { String::new() }
    }

    /// Before a recording pass starts: with a count-in on and the transport stopped, move the
    /// playhead back by the count-in and turn the click on until the punch-in point, which
    /// this returns (engine frames).
    pub(crate) fn count_in_begin(&mut self) -> Option<i64> {
        let bars = self.count_in_bars();
        if bars == 0 || self.engine.transport().playing {
            return None;
        }
        let p = self.project();
        let punch_ticks = self.playhead_ticks();
        let punch = p.ticks_to_frames(punch_ticks, self.rate);
        let from = p.ticks_to_frames(count_in_start(punch_ticks, bars, p.ticks_per_bar()), self.rate);
        if let Err(e) = self.engine.seek(from) {
            self.set_status(format!("Count-in: {e}"), 4);
            return None;
        }
        self.rec.count_in = Some(punch);
        self.sync_loop_and_click();
        Some(punch)
    }

    /// Every UI tick: once the playhead reaches the punch-in point (or playback stops), the
    /// click goes back to the metronome setting.
    pub(crate) fn count_in_tick(&mut self) {
        let Some(punch) = self.rec.count_in else { return };
        let t = self.engine.transport();
        if !t.playing || t.position >= punch {
            self.count_in_cancel();
            if self.rec.recording {
                self.set_status("Recording…", 2);
            }
        }
    }

    /// Ends a count-in now (recording stopped, or it could not start).
    pub(crate) fn count_in_cancel(&mut self) {
        if self.rec.count_in.take().is_some() {
            self.sync_loop_and_click();
            self.ui_dirty = true;
        }
    }

    /// Counting in (the record button blinks, the bar shows "Count-in").
    pub fn counting_in(&self) -> bool {
        self.rec.count_in.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(frame: i64, on: bool) -> CapturedEvent {
        CapturedEvent { frame, on, pitch: 60, velocity: if on { 100 } else { 0 } }
    }

    #[test]
    fn takes_keep_nothing_before_zero_or_the_punch_point() {
        assert_eq!(first_kept_frame(-500, None), 0);
        assert_eq!(first_kept_frame(1_000, None), 1_000);
        // Count-in from 96 000 frames before the punch point at 192 000.
        assert_eq!(first_kept_frame(96_000, Some(192_000)), 192_000);
        // Punch at 0: the take started at -96 000 keeps frames from 0.
        assert_eq!(first_kept_frame(-96_000, Some(0)), 0);
    }

    #[test]
    fn notes_before_the_punch_point_are_dropped_until_a_wrap() {
        let mut punch = Some(1_000);
        let got = drop_count_in(&mut punch, vec![ev(500, true), ev(900, false), ev(1_000, true), ev(1_200, false)], false);
        assert_eq!(got.iter().map(|e| (e.frame, e.on)).collect::<Vec<_>>(), [(900, false), (1_000, true), (1_200, false)]);
        assert_eq!(punch, Some(1_000));
        let got = drop_count_in(&mut punch, vec![ev(200, true)], true);
        assert_eq!(got.len(), 1);
        assert_eq!(punch, None);
    }

    #[test]
    fn count_in_starts_whole_bars_early() {
        assert_eq!(count_in_start(4 * 3840, 1, 3840), 3 * 3840);
        assert_eq!(count_in_start(0, 2, 3840), -2 * 3840);
    }
}
