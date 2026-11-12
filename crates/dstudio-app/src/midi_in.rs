//! MIDI input in the app (spec 003): which ports are open (Settings › MIDI)
//! and the MIDI side of a recording pass.
//!
//! Playing happens entirely in the engine (routing on the audio thread from
//! the snapshot's arm / monitor flags: only armed or monitoring instrument
//! tracks hear the keyboard; the app sets no focus track). The app only opens
//! ports and, while recording, drains
//! the engine's capture ring and turns it into clips with
//! `dstudio_core::midi_record` when the pass stops.

use crate::MidiInputRow;
use crate::controller::Controller;
use dstudio_core::midi_record::{self, CapturedEvent, Pass};
use dstudio_core::{Command, TrackId};
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
    /// The engine reported a loop wrap during the pass (a seek back never counts).
    pub wrapped: bool,
    /// The loop range (frames) when the pass first wrapped: kept for the rest of the
    /// pass, so turning the loop off after a wrap still folds notes held across it.
    pub wrap_loop: Option<(i64, i64)>,
    /// The latest loop range seen during the pass. Kept after the loop is turned off,
    /// so a wrap the engine reports after that (its capture can trail the UI by a
    /// poll or more) is still read in the loop it happened in.
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
    /// Called every UI tick: port rescans and the recording pass.
    pub fn midi_tick(&mut self) {
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
        self.midi_in.pass = Some(MidiPass { start, wrapped: false, wrap_loop: None, last_loop, events: Vec::new(), tracks });
    }

    /// Drain the capture ring and notice loop wraps (the engine reports real wraps, so
    /// a seek back while recording is never read as one).
    fn midi_poll_pass(&mut self) {
        if self.midi_in.pass.is_none() {
            return;
        }
        let now_loop = self.loop_frames();
        let got = self.engine.read_midi_capture();
        // Read after the events: a wrap before any of them is already reported.
        let wrapped = self.engine.midi_capture_wrapped();
        let Some(pass) = self.midi_in.pass.as_mut() else { return };
        let events = got.into_iter().map(|n| CapturedEvent { frame: n.frame, on: n.on, pitch: n.pitch, velocity: n.velocity });
        poll_update(pass, events, now_loop, wrapped);
    }

    /// The loop range in frames while the loop is on.
    fn loop_frames(&self) -> Option<(i64, i64)> {
        let p = self.project();
        p.transport
            .loop_enabled
            .then(|| (p.ticks_to_frames(p.transport.loop_start, self.rate), p.ticks_to_frames(p.transport.loop_end, self.rate)))
    }

    /// The pass's frame range so far (or at stop), as `midi_record` takes it.
    pub(crate) fn midi_pass_bounds(&self, pass: &MidiPass, stop: i64) -> Pass {
        pass_bounds(pass, stop, self.loop_frames().or(pass.last_loop))
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
        let rows: Vec<MidiInputRow> =
            self.midi_rows().into_iter().map(|(name, on, lit)| MidiInputRow { name: name.into(), on, lit }).collect();
        crate::controller::sync_model(&self.midi_inputs_model, rows);
    }
}

/// The pass's range at `stop`. A pass that wrapped keeps the loop range it wrapped
/// in (even if the loop was turned off since); one that never wrapped has none, also
/// when a seek moved the playhead back before its start.
fn pass_bounds(pass: &MidiPass, stop: i64, current_loop: Option<(i64, i64)>) -> Pass {
    let wrapped_loop = if pass.wrapped { pass.wrap_loop.or(current_loop) } else { None };
    Pass { start: pass.start, stop, wrapped_loop }
}

/// Whether to ask the engine to open `want`: it differs from what is open, and it
/// is not a set that just failed to open (those are retried after RETRY).
fn should_open(opened: Option<&[String]>, retry_at: Option<&(Vec<String>, Instant)>, want: &[String], now: Instant) -> bool {
    let waiting = retry_at.is_some_and(|(w, at)| w.as_slice() == want && now < *at);
    opened != Some(want) && !waiting
}

/// One poll of a pass: append the captured events, remember the loop in force (kept
/// after the loop is turned off, for a wrap the engine reports a poll late) and take
/// the first wrap the engine reported with that loop.
fn poll_update(pass: &mut MidiPass, events: impl Iterator<Item = CapturedEvent>, now_loop: Option<(i64, i64)>, engine_wrapped: bool) {
    if now_loop.is_some() {
        pass.last_loop = now_loop;
    }
    pass.events.extend(events);
    if engine_wrapped && !pass.wrapped {
        pass.wrapped = true;
        pass.wrap_loop = pass.last_loop;
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
        poll_update(&mut p, std::iter::empty(), l, true);
        assert_eq!(pass_bounds(&p, 1200, l).wrapped_loop, l, "wrapped");
        assert_eq!(pass_bounds(&p, 6000, None).wrapped_loop, l, "loop turned off after the wrap");
    }
    #[test]
    fn a_wrap_just_before_the_loop_is_turned_off_still_counts() {
        let ev = |frame, on| CapturedEvent { frame, on, pitch: 60, velocity: if on { 100 } else { 0 } };
        let mut p = MidiPass { start: 1000, ..Default::default() };
        poll_update(&mut p, [ev(4800, true)].into_iter(), Some((1000, 5000)), false);
        // The loop is off by the next poll, which brings the wrap and the note-off after it.
        poll_update(&mut p, [ev(1200, false)].into_iter(), None, true);
        assert!(p.wrapped);
        assert_eq!(p.wrap_loop, Some((1000, 5000)));
        // Notes held across that wrap end at the loop end.
        let b = pass_bounds(&p, 1300, None);
        let notes = midi_record::pair_notes(&p.events, &b);
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].end, 5000);
    }
    #[test]
    fn a_seek_back_is_not_a_wrap() {
        let ev = |frame, on| CapturedEvent { frame, on, pitch: 60, velocity: if on { 100 } else { 0 } };
        let l = Some((1000, 5000));
        // The loop was on, then turned off; the user seeks back while recording.
        let mut p = MidiPass { start: 2000, ..Default::default() };
        poll_update(&mut p, [ev(4000, true)].into_iter(), l, false);
        poll_update(&mut p, [ev(1500, false), ev(1600, true), ev(1700, false)].into_iter(), None, false);
        assert!(!p.wrapped);
        let b = pass_bounds(&p, 1800, p.last_loop);
        assert_eq!(b.wrapped_loop, None, "the clip does not grow to the old loop");
        // The note held across the seek is dropped, not stretched to the old loop end.
        let notes = midi_record::pair_notes(&p.events, &b);
        assert_eq!(notes.len(), 1);
        assert_eq!((notes[0].start, notes[0].end), (1600, 1700));
        // A seek back with the loop still on is no wrap either.
        let mut q = MidiPass { start: 2000, ..Default::default() };
        poll_update(&mut q, [ev(4000, true), ev(1500, false)].into_iter(), l, false);
        assert_eq!(pass_bounds(&q, 1500, l).wrapped_loop, None);
    }
}
