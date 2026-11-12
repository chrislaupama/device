//! Return tracks, sends and Out routing in the app (spec 002): the Timeline's Returns pill, the
//! Add track / add send / Out menus, send edits (level, pre-fader, remove), and the view models
//! for the mixer Sends sections and the channel strip's Sends tab.
//!
//! Every edit is a core command: `SetSends` (wrapped in a labelled `Batch` for Add Send, Remove
//! Send and Send Pre-Fader / Post-Fader; level drags coalesce as "Send Level Change"),
//! `SetTrackOutput` and `AddTrack`. The pill and the expanded Sends sections are view state.

use crate::controller::{Controller, gain_from_slider, next_track_color, slider_from_gain, slint_color, sync_model, track_color};
use crate::{ChoiceItem, SendRow, Studio};
use dstudio_core::{Command, MAX_RETURNS, Track, TrackId, TrackKind, TrackSend, gain_to_db, return_letter};
use slint::VecModel;
use std::rc::Rc;

/// Menu row height and padding (the "In" selector's metrics).
const ROW: f32 = 20.0;
const PAD: f32 = 4.0;

/// What a routing menu row does.
#[derive(Debug, Clone, PartialEq)]
pub enum RouteItem {
    /// Add a send from the menu's track to this return.
    Send(TrackId),
    /// Create a return track and a send to it, in one undo step.
    NewReturn,
    /// Route the menu's track to the master (None) or a return.
    Out(Option<TrackId>),
    /// Add a track of this kind.
    AddTrack(TrackKind),
}

#[derive(Debug, Clone)]
pub struct RouteMenu {
    /// The track the menu edits (None for Add track).
    pub track: Option<TrackId>,
    pub items: Vec<(String, RouteItem, bool)>,
    /// Anchor in window coordinates (design units); the popup keeps itself on screen.
    pub x: f32,
    pub y: f32,
}

#[derive(Default)]
pub struct RoutingUi {
    /// Mixer: the Sends section of every strip is expanded.
    pub sends_open: bool,
    pub menu: Option<RouteMenu>,
    pub menu_model: Rc<VecModel<ChoiceItem>>,
    /// Mixer send rows, all strips (each row names its sender).
    pub strip_sends_model: Rc<VecModel<SendRow>>,
    /// Channel strip Sends tab rows for the selected track.
    pub sel_sends_model: Rc<VecModel<SendRow>>,
}

/// Send level readout: "0.0 dB", "−12.5 dB", "−∞ dB".
pub fn fmt_send_db(gain: f32) -> String {
    let db = gain_to_db(gain);
    if !db.is_finite() || db <= -60.0 { "−∞ dB".into() } else { format!("{db:.1} dB").replace('-', "−") }
}

/// Mixer send row readout (no unit): "0.0", "−12.5", "−∞".
fn fmt_send_short(gain: f32) -> String {
    let db = gain_to_db(gain);
    if !db.is_finite() || db <= -60.0 { "−∞".into() } else { format!("{db:.1}").replace('-', "−") }
}

impl Controller {
    /// The Returns pill is lit: return rows show on the timeline.
    pub fn returns_shown(&self) -> bool {
        !self.settings.prefs.hide_returns
    }

    /// Tracks drawn on the timeline: every track, or the ordinary ones while the return rows are
    /// hidden (returns always come last, so the rows keep their project indices).
    pub fn lane_tracks(&self) -> usize {
        let p = self.project();
        if self.returns_shown() { p.tracks.len() } else { p.ordinary_track_count() }
    }

    pub fn toggle_returns(&mut self) {
        self.settings.prefs.hide_returns = !self.settings.prefs.hide_returns;
        self.save_prefs();
        self.ui_dirty = true;
    }

    pub fn toggle_sends_open(&mut self) {
        self.routing.sends_open = !self.routing.sends_open;
        self.ui_dirty = true;
    }

    /// Add a return track (Add track › Return track). Returns its id.
    pub fn add_return_track(&mut self) -> Option<TrackId> {
        let p = self.project();
        if p.returns().count() >= MAX_RETURNS {
            self.set_status(format!("A project has at most {MAX_RETURNS} return tracks"), 3);
            return None;
        }
        let id = TrackId(p.next_id);
        let track = Track::new(id, TrackKind::Return, &p.next_return_name(), next_track_color(p));
        if self.execute(Command::AddTrack { index: None, track }) {
            self.select_track(id);
            return Some(id);
        }
        None
    }

    fn set_sends(&mut self, label: &str, track: TrackId, sends: Vec<TrackSend>) -> bool {
        self.doc.seal_coalescing();
        let ok = self.execute(Command::Batch { label: label.into(), commands: vec![Command::SetSends { track, sends }] });
        self.doc.seal_coalescing();
        ok
    }

    /// Add a send at 0 dB, post-fader, from `track` to the return `target` ("Add Send").
    pub fn add_send(&mut self, track: TrackId, target: TrackId) -> bool {
        let Some(t) = self.project().track(track) else { return false };
        if t.sends.iter().any(|s| s.target == target) {
            return false;
        }
        let mut sends = t.sends.clone();
        sends.push(TrackSend::new(target));
        self.set_sends("Add Send", track, sends)
    }

    /// Create a return track and a send to it from `track`, in one undo step ("Add Send").
    pub fn new_return_with_send(&mut self, track: TrackId) -> Option<TrackId> {
        let p = self.project();
        let t = p.track(track).filter(|t| !t.is_return())?;
        if p.returns().count() >= MAX_RETURNS {
            self.set_status(format!("A project has at most {MAX_RETURNS} return tracks"), 3);
            return None;
        }
        let id = TrackId(p.next_id);
        let ret = Track::new(id, TrackKind::Return, &p.next_return_name(), next_track_color(p));
        let mut sends = t.sends.clone();
        sends.push(TrackSend::new(id));
        self.doc.seal_coalescing();
        let ok = self.execute(Command::Batch {
            label: "Add Send".into(),
            commands: vec![Command::AddTrack { index: None, track: ret }, Command::SetSends { track, sends }],
        });
        self.doc.seal_coalescing();
        ok.then_some(id)
    }

    /// Remove the send from `track` to `target` ("Remove Send").
    pub fn remove_send(&mut self, track: TrackId, target: TrackId) -> bool {
        let Some(t) = self.project().track(track) else { return false };
        let sends: Vec<TrackSend> = t.sends.iter().filter(|s| s.target != target).cloned().collect();
        if sends.len() == t.sends.len() {
            return false;
        }
        self.set_sends("Remove Send", track, sends)
    }

    /// Flip a send between post- and pre-fader ("Send Pre-Fader" / "Send Post-Fader").
    pub fn toggle_send_pre(&mut self, track: TrackId, target: TrackId) -> bool {
        let Some(t) = self.project().track(track) else { return false };
        let mut sends = t.sends.clone();
        let Some(s) = sends.iter_mut().find(|s| s.target == target) else { return false };
        s.pre_fader = !s.pre_fader;
        let label = if s.pre_fader { "Send Pre-Fader" } else { "Send Post-Fader" };
        self.set_sends(label, track, sends)
    }

    /// Set a send's level (linear gain, clamped to −∞..+6 dB). Successive calls in one drag
    /// coalesce into one "Send Level Change" however long the knob is held still;
    /// `send_gesture_end` closes the drag.
    pub fn set_send_level(&mut self, track: TrackId, target: TrackId, gain: f32) -> bool {
        let Some(t) = self.project().track(track) else { return false };
        let gain = gain.clamp(0.0, TrackSend::MAX_LEVEL);
        let mut sends = t.sends.clone();
        let Some(s) = sends.iter_mut().find(|s| s.target == target) else { return false };
        if (s.level - gain).abs() < 1e-7 {
            return false;
        }
        s.level = gain;
        // One drag is one undo step, pauses while the knob is held included.
        let ok = self.execute(Command::SetSends { track, sends });
        if ok {
            self.doc.hold_coalescing();
        }
        ok
    }

    /// Send level from a slider / knob position (the track volume slider's taper).
    pub fn set_send_pos(&mut self, track: TrackId, target: TrackId, pos: f32) -> bool {
        self.set_send_level(track, target, gain_from_slider(pos))
    }

    pub fn send_gesture_end(&mut self) {
        self.doc.seal_coalescing();
    }

    /// Route `track`'s Out to the master (None) or a return ("Output Change").
    pub fn set_track_output(&mut self, track: TrackId, output: Option<TrackId>) -> bool {
        if self.project().track(track).is_none_or(|t| t.is_return() || t.output == output) {
            return false;
        }
        self.execute(Command::SetTrackOutput { track, output })
    }

    // ---------------------------------------------------------------- menus

    fn return_label(&self, id: TrackId) -> String {
        self.project().track(id).map(|t| t.name.clone()).unwrap_or_default()
    }

    /// Sends "+" (mixer strip or Sends tab): the returns `track` does not send to yet, then
    /// "New return track" (while fewer than [MAX_RETURNS] exist).
    pub fn open_send_menu(&mut self, track: TrackId, x: f32, y: f32) {
        let p = self.project();
        let Some(t) = p.track(track).filter(|t| !t.is_return()) else { return };
        let mut items: Vec<(String, RouteItem, bool)> = p
            .returns()
            .filter(|r| !t.sends.iter().any(|s| s.target == r.id))
            .map(|r| (r.name.clone(), RouteItem::Send(r.id), false))
            .collect();
        if p.returns().count() < MAX_RETURNS {
            items.push(("New return track".into(), RouteItem::NewReturn, false));
        }
        self.routing.menu = Some(RouteMenu { track: Some(track), items, x, y });
        self.ui_dirty = true;
    }

    /// Channel strip Out selector: Master, then every return (the current one checked).
    pub fn open_out_menu(&mut self, x: f32, y: f32) {
        let p = self.project();
        let Some(t) = self.selected_track.and_then(|id| p.track(id)).filter(|t| !t.is_return()) else { return };
        let mut items = vec![("Master".to_string(), RouteItem::Out(None), t.output.is_none())];
        items.extend(p.returns().map(|r| (r.name.clone(), RouteItem::Out(Some(r.id)), t.output == Some(r.id))));
        self.routing.menu = Some(RouteMenu { track: Some(t.id), items, x, y });
        self.ui_dirty = true;
    }

    /// Add track (timeline and mixer): Audio, Instrument or Return track.
    pub fn open_add_track_menu(&mut self, x: f32, y: f32) {
        let items = vec![
            ("Audio track".to_string(), RouteItem::AddTrack(TrackKind::Audio), false),
            ("Instrument track".to_string(), RouteItem::AddTrack(TrackKind::Instrument), false),
            ("Return track".to_string(), RouteItem::AddTrack(TrackKind::Return), false),
        ];
        self.routing.menu = Some(RouteMenu { track: None, items, x, y });
        self.ui_dirty = true;
    }

    pub fn route_menu_close(&mut self) {
        if self.routing.menu.take().is_some() {
            self.ui_dirty = true;
        }
    }

    pub fn route_menu_pick(&mut self, i: usize) {
        let Some(menu) = self.routing.menu.take() else { return };
        self.ui_dirty = true;
        let Some((_, item, _)) = menu.items.get(i).cloned() else { return };
        match (item, menu.track) {
            (RouteItem::Send(target), Some(track)) => {
                self.add_send(track, target);
            }
            (RouteItem::NewReturn, Some(track)) => {
                self.new_return_with_send(track);
            }
            (RouteItem::Out(out), Some(track)) => {
                self.set_track_output(track, out);
            }
            (RouteItem::AddTrack(TrackKind::Audio), _) => {
                self.add_audio_track();
            }
            (RouteItem::AddTrack(TrackKind::Instrument), _) => {
                self.add_instrument_track();
            }
            (RouteItem::AddTrack(TrackKind::Return), _) => {
                self.add_return_track();
            }
            _ => {}
        }
    }

    /// Labels of the open routing menu (self-test).
    pub fn route_menu_labels(&self) -> Vec<String> {
        self.routing.menu.as_ref().map(|m| m.items.iter().map(|(l, _, _)| l.clone()).collect()).unwrap_or_default()
    }

    // ---------------------------------------------------------------- view models

    fn send_rows(&self, track: &Track) -> Vec<SendRow> {
        let p = self.project();
        track
            .sends
            .iter()
            .enumerate()
            .filter_map(|(k, s)| {
                let (ri, r) = p.returns().enumerate().find(|(_, r)| r.id == s.target)?;
                Some(SendRow {
                    track: track.id.0 as i32,
                    row: k as i32,
                    target: r.id.0 as i32,
                    letter: return_letter(ri).to_string().into(),
                    name: r.name.clone().into(),
                    color: slint_color(track_color(r.color)),
                    pos: slider_from_gain(s.level),
                    db_text: fmt_send_db(s.level).into(),
                    short_text: fmt_send_short(s.level).into(),
                    pre: s.pre_fader,
                })
            })
            .collect()
    }

    /// Pill, Sends sections, the channel strip routing fields and the routing menu → Slint.
    pub(crate) fn push_routing(&self, s: &Studio) {
        let p = self.project();
        // The pill is lit while return rows actually show (not in a project without returns).
        s.set_returns_shown(self.returns_shown() && p.returns().next().is_some());
        s.set_sends_open(self.routing.sends_open);
        let rows: Vec<SendRow> = p.tracks.iter().filter(|t| !t.is_return()).flat_map(|t| self.send_rows(t)).collect();
        let most = p.tracks.iter().map(|t| t.sends.len()).max().unwrap_or(0);
        s.set_strip_sends_max(most as i32);
        sync_model(&self.routing.strip_sends_model, rows);
        let sel = self.selected_track.and_then(|id| p.track(id));
        s.set_sel_return(sel.is_some_and(|t| t.is_return()));
        let letter = sel.and_then(|t| p.return_index(t.id)).map(|i| return_letter(i).to_string()).unwrap_or_default();
        s.set_sel_letter(letter.into());
        let out = match sel.and_then(|t| t.output) {
            Some(r) => self.return_label(r),
            None => "Master".into(),
        };
        s.set_sel_out_text(out.into());
        sync_model(&self.routing.sel_sends_model, sel.map(|t| self.send_rows(t)).unwrap_or_default());
        s.set_sel_can_send(sel.is_some_and(|t| !t.is_return()));
        match &self.routing.menu {
            Some(m) => {
                let mut y = PAD;
                let rows: Vec<ChoiceItem> = m
                    .items
                    .iter()
                    .map(|(label, _, checked)| {
                        let r = ChoiceItem { label: label.clone().into(), checked: *checked, hint: false, y, h: ROW };
                        y += ROW;
                        r
                    })
                    .collect();
                let w = m.items.iter().map(|(l, _, _)| crate::text_measure::value_width(l)).fold(0.0f32, f32::max) + 44.0;
                s.set_route_menu_x(m.x);
                s.set_route_menu_y(m.y);
                s.set_route_menu_w(w.max(120.0));
                s.set_route_menu_h(y + PAD);
                sync_model(&self.routing.menu_model, rows);
                s.set_route_menu_open(true);
            }
            None => s.set_route_menu_open(false),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn send_readouts() {
        assert_eq!(fmt_send_db(1.0), "0.0 dB");
        assert_eq!(fmt_send_db(0.0), "−∞ dB");
        assert_eq!(fmt_send_short(0.5), "−6.0");
        assert_eq!(fmt_send_short(TrackSend::MAX_LEVEL), "6.0");
    }
}
