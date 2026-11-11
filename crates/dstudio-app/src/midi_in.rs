//! MIDI input in the app (spec 003): which ports are open (Settings › MIDI),
//! which instrument track plays the keyboard when none is armed or monitoring
//! (the selected one), and the MIDI side of a recording pass.
//!
//! Playing happens entirely in the engine (routing on the audio thread from
//! the snapshot's arm / monitor flags plus the focus track set here). The app
//! only opens ports, keeps the focus up to date and, while recording, drains
//! the engine's capture ring and turns it into clips with
//! `dstudio_core::midi_record` when the pass stops.

use crate::controller::Controller;
use crate::MidiInputRow;
use dstudio_core::midi_record::{self, CapturedEvent, Pass};
use dstudio_core::{Command, TrackId, TrackKind};
use std::time::{Duration, Instant};

/// How often the port list is rescanned (new keyboards show up within this).
const RESCAN: Duration = Duration::from_secs(2);
/// Rescan rate while Settings › MIDI is showing (activity lights).
const RESCAN_PAGE: Duration = Duration::from_millis(100);
/// How long to wait before asking the engine again for ports that failed to open.
const RETRY: Duration = Duration::from_secs(5);
/// How long an activity light stays lit after a message.
const LIT: Duration = Duration::from_millis(180);
/// Settings page index of "MIDI".
pub const MIDI_PAGE: usize = 2;

/// One row of Settings › MIDI › MIDI inputs.
#[derive(Debug, Clone, PartialEq)]
pub struct PortRow {
    pub id: String,
    pub name: String,
    pub events: u64,
    pub lit_until: Option<Instant>,
}

/// The MIDI side of the recording pass in progress (frames at the engine rate).
#[derive(Debug, Clone, Default)]
pub struct MidiPass {
    pub start: i64,
    pub last_pos: i64,
    pub wrapped: bool,
    /// The loop range (frames) when the pass first wrapped: kept for the rest of the
    /// pass, so turning the loop off after a wrap still folds notes held across it.
    pub wrap_loop: Option<(i64, i64)>,
    /// The loop range at the previous poll: a wrap followed by the loop being turned
    /// off before the next poll is still read in the loop it happened in.
    pub last_loop: Option<(i64, i64)>,
    pub events: Vec<CapturedEvent>,
    /// The instrument tracks armed when the pass started: the ones it records on.
    pub tracks: Vec<TrackId>,
}

#[derive(Debug, Default)]
pub struct MidiInState {
    pub ports: Vec<PortRow>,
    /// Port ids the engine last opened (None = never, or the last open failed).
    opened: Option<Vec<String>>,
    /// After a failed open: no new attempt before this (unless the wanted set changes).
    retry_at: Option<(Vec<String>, Instant)>,
    last_scan: Option<Instant>,
    /// Focus last sent to the engine (None = never sent).
    focus: Option<Option<u64>>,
    pub pass: Option<MidiPass>,
    /// SCREENSHOT-ONLY: a fixed port list; the engine is never asked to list or open ports.
    pub fake_ports: bool,
    /// Last port error shown, so a failing port is reported once.
    last_error: Option<String>,
}

/// SCREENSHOT-ONLY port list for `--state review-settings-midi`.
pub fn screenshot_fake_ports() -> Vec<PortRow> {
    vec![
        PortRow { id: "fake:keys".into(), name: "USB Keyboard".into(), events: 0, lit_until: None },
        PortRow { id: "fake:pads".into(), name: "Pad Controller".into(), events: 0, lit_until: None },
    ]
}

/// Ports to open: every present port except the ones switched off.
pub fn wanted_ports(ports: &[PortRow], disabled: &[String]) -> Vec<String> {
    ports.iter().filter(|p| !disabled.contains(&p.id)).map(|p| p.id.clone()).collect()
}

impl Controller {
    /// Called every UI tick: focus, port rescans and the recording pass.
    pub fn midi_tick(&mut self) {
        // The selected instrument track plays the keyboard while no track is armed or monitoring.
        let focus = self.selected_track.and_then(|t| self.project().track(t)).filter(|t| t.kind == TrackKind::Instrument).map(|t| t.id.0);
        if self.midi_in.focus != Some(focus) {
            self.engine.set_midi_focus(focus);
            self.midi_in.focus = Some(focus);
        }
        self.midi_scan(false);
        self.midi_poll_pass();
    }

    /// Rescan the MIDI ports (every 2 s, or 10 times a second while Settings › MIDI shows,
    /// for the activity lights) and open the ones that are on. Self-test and screenshot
    /// runs never touch real ports.
    pub fn midi_scan(&mut self, force: bool) {
        if self.settings.headless || self.midi_in.fake_ports {
            return;
        }
        let page_open = self.settings.open && self.settings.page == MIDI_PAGE;
        let every = if page_open { RESCAN_PAGE } else { RESCAN };
        let now = Instant::now();
        if !force && self.midi_in.last_scan.is_some_and(|t| now.duration_since(t) < every) {
            return;
        }
        self.midi_in.last_scan = Some(now);
        let Ok(list) = self.engine.midi_inputs() else { return };
        let mut rows = Vec::with_capacity(list.len());
        let closed: Vec<String> = list.iter().filter(|p| !p.open).map(|p| p.id.clone()).collect();
        for p in list {
            let old = self.midi_in.ports.iter().find(|r| r.id == p.id);
            let mut lit_until = old.and_then(|r| r.lit_until).filter(|t| *t > now);
            if old.is_some_and(|r| p.events > r.events) {
                lit_until = Some(now + LIT);
            }
            rows.push(PortRow { id: p.id, name: p.name, events: p.events, lit_until });
        }
        if rows != self.midi_in.ports {
            self.midi_in.ports = rows;
            if page_open {
                self.ui_dirty = true;
            }
        }
        let want = wanted_ports(&self.midi_in.ports, &self.settings.prefs.midi.disabled_inputs);
        // A port that was missing when its set was opened (the engine skips unknown ids)
        // reads as closed once it is back: open the set again, at most every RETRY.
        let stale = self.midi_in.opened.as_ref() == Some(&want) && want.iter().any(|id| closed.contains(id));
        if stale {
            self.midi_in.opened = None;
        }
        if should_open(self.midi_in.opened.as_deref(), self.midi_in.retry_at.as_ref(), &want, now) {
            match self.engine.open_midi_inputs(&want) {
                Ok(()) => {
                    self.midi_in.last_error = None;
                    self.midi_in.retry_at = stale.then(|| (want.clone(), now + RETRY));
                    self.midi_in.opened = Some(want);
                }
                Err(e) => {
                    // Not remembered as opened: tried again after RETRY (a keyboard that was
                    // busy or still starting up comes back on its own).
                    let msg = format!("MIDI input: {e}");
                    if self.midi_in.last_error.as_ref() != Some(&msg) {
                        self.set_status(msg.clone(), 6);
                        self.midi_in.last_error = Some(msg);
                    }
                    self.midi_in.opened = None;
                    self.midi_in.retry_at = Some((want, now + RETRY));
                }
            }
        }
    }

    /// Settings › MIDI switch: turn an input on or off (an app preference).
    pub fn midi_toggle_input(&mut self, row: usize) {
        let Some(id) = self.midi_in.ports.get(row).map(|r| r.id.clone()) else { return };
        let off = &mut self.settings.prefs.midi.disabled_inputs;
        if let Some(i) = off.iter().position(|d| *d == id) {
            off.remove(i);
        } else {
            off.push(id);
        }
        self.save_prefs();
        self.midi_scan(true);
        self.ui_dirty = true;
    }

    /// Settings › MIDI rows: (name, on, lit).
    pub fn midi_rows(&self) -> Vec<(String, bool, bool)> {
        let now = Instant::now();
        let off = &self.settings.prefs.midi.disabled_inputs;
        self.midi_in.ports.iter().map(|p| (p.name.clone(), !off.contains(&p.id), p.lit_until.is_some_and(|t| t > now))).collect()
    }

    // ---------------------------------------------------------------- recording pass

    /// Armed instrument tracks: the tracks a recording pass records MIDI on.
    pub fn armed_midi(&self) -> Vec<TrackId> {
        midi_record::armed_instrument_tracks(self.project())
    }

    /// Start capturing what the armed instrument tracks hear (the transport is started
    /// by the caller).
    pub(crate) fn midi_pass_start(&mut self) {
        self.engine.set_midi_capture(true);
        let start = self.audible_frames();
        let tracks = self.armed_midi();
        let last_loop = self.loop_frames();
        self.midi_in.pass = Some(MidiPass { start, last_pos: start, wrapped: false, wrap_loop: None, last_loop, events: Vec::new(), tracks });
    }

    /// Drain the capture ring and notice loop wraps (the playhead jumping back).
    fn midi_poll_pass(&mut self) {
        if self.midi_in.pass.is_none() {
            return;
        }
        let pos = self.audible_frames();
        let now_loop = self.loop_frames();
        let got = self.engine.read_midi_capture();
        let Some(pass) = self.midi_in.pass.as_mut() else { return };
        // The loop in force since the last poll: the current one, or the one just turned off.
        let current = now_loop.or(pass.last_loop);
        pass.last_loop = now_loop;
        let looping = current.is_some();
        absorb(pass, got.into_iter().map(|n| CapturedEvent { frame: n.frame, on: n.on, pitch: n.pitch, velocity: n.velocity }), looping);
        if looping && pos < pass.last_pos {
            pass.wrapped = true;
        }
        if pass.wrapped && pass.wrap_loop.is_none() {
            pass.wrap_loop = current;
        }
        pass.last_pos = pos;
    }

    /// The loop range in frames while the loop is on.
    fn loop_frames(&self) -> Option<(i64, i64)> {
        let p = self.project();
        p.transport.loop_enabled.then(|| (p.ticks_to_frames(p.transport.loop_start, self.rate), p.ticks_to_frames(p.transport.loop_end, self.rate)))
    }

    /// The pass's frame range so far (or at stop), as `midi_record` takes it.
    pub(crate) fn midi_pass_bounds(&self, pass: &MidiPass, stop: i64) -> Pass {
        pass_bounds(pass, stop, self.loop_frames())
    }

    /// End the MIDI side of the pass: one `AddMidiClip` per armed instrument track that
    /// received notes (for the pass's "Record" batch).
    pub(crate) fn midi_pass_finish(&mut self) -> Vec<Command> {
        if self.midi_in.pass.is_none() {
            return Vec::new();
        }
        self.engine.set_midi_capture(false);
        self.midi_poll_pass();
        let Some(pass) = self.midi_in.pass.take() else { return Vec::new() };
        let stop = self.audible_frames();
        let bounds = self.midi_pass_bounds(&pass, stop);
        midi_record::record_commands(self.project(), self.rate, &pass.tracks, &pass.events, &bounds)
    }

    pub(crate) fn push_midi_settings(&self) {
        let rows: Vec<MidiInputRow> = self.midi_rows().into_iter().map(|(name, on, lit)| MidiInputRow { name: name.into(), on, lit }).collect();
        crate::controller::sync_model(&self.midi_inputs_model, rows);
    }
}

/// The pass's range at `stop`. A pass that wrapped keeps the loop range it wrapped
/// in (even if the loop was turned off since); otherwise the current loop counts when
/// the playhead is already back before the start.
fn pass_bounds(pass: &MidiPass, stop: i64, current_loop: Option<(i64, i64)>) -> Pass {
    let wrapped_loop = pass.wrap_loop.or_else(|| current_loop.filter(|_| pass.wrapped || stop < pass.start));
    Pass { start: pass.start, stop, wrapped_loop }
}

/// Whether to ask the engine to open `want`: it differs from what is open, and it
/// is not a set that just failed to open (those are retried after RETRY).
fn should_open(opened: Option<&[String]>, retry_at: Option<&(Vec<String>, Instant)>, want: &[String], now: Instant) -> bool {
    let waiting = retry_at.is_some_and(|(w, at)| w.as_slice() == want && now < *at);
    opened != Some(want) && !waiting
}

/// Append captured events; a frame earlier than the previous one in a looping pass
/// means playback wrapped.
fn absorb(pass: &mut MidiPass, events: impl Iterator<Item = CapturedEvent>, looping: bool) {
    for e in events {
        if looping && pass.events.last().is_some_and(|l| e.frame < l.frame) {
            pass.wrapped = true;
        }
        pass.events.push(e);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wanted_skips_disabled() {
        let ports = screenshot_fake_ports();
        assert_eq!(wanted_ports(&ports, &[]), ["fake:keys", "fake:pads"]);
        assert_eq!(wanted_ports(&ports, &["fake:keys".into()]), ["fake:pads"]);
        assert!(wanted_ports(&[], &[]).is_empty());
    }
    #[test]
    fn a_failed_open_is_retried_later() {
        let now = Instant::now();
        let want = vec!["keys".to_string()];
        assert!(should_open(None, None, &want, now), "never opened");
        assert!(!should_open(Some(&want), None, &want, now), "already open");
        let failed = (want.clone(), now + RETRY);
        assert!(!should_open(None, Some(&failed), &want, now), "waits after a failure");
        assert!(should_open(None, Some(&failed), &want, now + RETRY), "then tries again");
        assert!(should_open(None, Some(&failed), &["pads".to_string()], now), "a different set is tried at once");
    }
    #[test]
    fn a_wrapped_pass_keeps_its_loop_after_the_loop_is_turned_off() {
        let l = Some((1000, 5000));
        let mut p = MidiPass { start: 1000, ..Default::default() };
        assert_eq!(pass_bounds(&p, 3000, l).wrapped_loop, None, "not wrapped yet");
        assert_eq!(pass_bounds(&p, 1500, None).wrapped_loop, None, "no loop");
        assert_eq!(pass_bounds(&p, 500, l).wrapped_loop, l, "playhead back before the start: wrapped");
        p.wrapped = true;
        p.wrap_loop = l;
        assert_eq!(pass_bounds(&p, 6000, None).wrapped_loop, l, "loop turned off after the wrap");
    }
    #[test]
    fn a_wrap_just_before_the_loop_is_turned_off_still_counts() {
        // Events of the poll after the loop was turned off: the playhead had wrapped.
        let ev = |frame, on| CapturedEvent { frame, on, pitch: 60, velocity: if on { 100 } else { 0 } };
        let mut p = MidiPass { start: 1000, last_loop: Some((1000, 5000)), ..Default::default() };
        absorb(&mut p, [ev(4800, true)].into_iter(), true);
        let current = None.or(p.last_loop);
        absorb(&mut p, [ev(1200, false)].into_iter(), current.is_some());
        assert!(p.wrapped);
    }
    #[test]
    fn absorb_detects_wrap() {
        let ev = |frame| CapturedEvent { frame, on: true, pitch: 60, velocity: 100 };
        let mut p = MidiPass::default();
        absorb(&mut p, [ev(10), ev(20)].into_iter(), true);
        assert!(!p.wrapped);
        absorb(&mut p, [ev(5)].into_iter(), false);
        assert!(!p.wrapped, "no loop: a seek, not a wrap");
        absorb(&mut p, [ev(1)].into_iter(), true);
        assert!(p.wrapped);
        assert_eq!(p.events.len(), 4);
    }
}
