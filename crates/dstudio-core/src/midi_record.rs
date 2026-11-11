//! MIDI recording: turn the note events captured during one recording pass
//! into MIDI clips (spec 003, "Recording").
//!
//! The engine reports what it delivered to armed instrument tracks while the
//! transport played, as note-on / note-off events at timeline frames (output
//! latency compensated, loop wraps already folded back into the loop). This
//! module pairs them into notes and builds one clip per armed track. It is
//! pure: the app wraps the result in the pass's "Record" batch, so the whole
//! pass is one undo step.

use crate::midi::{MidiClip, Note};
use crate::model::{Project, TrackId, TrackKind};
use crate::Command;

/// One captured note event at a timeline frame (engine rate).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapturedEvent {
    pub frame: i64,
    pub on: bool,
    pub pitch: u8,
    pub velocity: u8,
}

/// What the transport did during the pass, in frames at the engine rate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pass {
    /// Playhead when recording started.
    pub start: i64,
    /// Playhead when recording stopped.
    pub stop: i64,
    /// The loop range, when playback wrapped at least once during the pass.
    pub wrapped_loop: Option<(i64, i64)>,
}

/// A recorded note in frames: `[start, end)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameNote {
    pub start: i64,
    pub end: i64,
    pub pitch: u8,
    pub velocity: u8,
}

/// Pair note-ons with note-offs (events in capture order). A note-on for a
/// pitch that is already held ends the held note first. Notes still held at
/// the end end at the stop position. In a looped pass a note held across the
/// wrap (its note-off folded to before its note-on) ends at the loop end.
/// A note whose on and off arrived in the same audio callback (same frame)
/// is kept as the shortest note, one frame long.
pub fn pair_notes(events: &[CapturedEvent], pass: &Pass) -> Vec<FrameNote> {
    let mut held: [Option<(i64, u8)>; 128] = [None; 128];
    let mut out = Vec::new();
    let close = |out: &mut Vec<FrameNote>, pitch: u8, (start, velocity): (i64, u8), end: i64| {
        let end = match pass.wrapped_loop {
            Some((_, loop_end)) if end < start => loop_end,
            _ if end == start => start + 1,
            _ => end,
        };
        if end > start {
            out.push(FrameNote { start, end, pitch, velocity });
        }
    };
    for e in events {
        let p = (e.pitch & 0x7F) as usize;
        if let Some(h) = held[p].take() {
            close(&mut out, p as u8, h, e.frame);
        }
        if e.on && e.velocity > 0 {
            held[p] = Some((e.frame, e.velocity.min(127)));
        }
    }
    for (p, h) in held.iter().enumerate() {
        if let Some(h) = *h {
            close(&mut out, p as u8, h, pass.stop);
        }
    }
    out.sort_by_key(|n| (n.start, n.pitch));
    out
}

/// The clip a pass records on one track, in ticks: from the bar where
/// recording started (or the loop start, if earlier and playback wrapped) to
/// the bar after the last note end or the stop position, whichever is later,
/// and at least to the loop end when playback wrapped. Notes are relative to
/// the clip start. `None` when no note was played.
pub fn recorded_clip(project: &Project, rate: f64, name: &str, notes: &[FrameNote], pass: &Pass) -> Option<MidiClip> {
    if notes.is_empty() {
        return None;
    }
    let bar = project.ticks_per_bar().max(1);
    let t = |f: i64| project.frames_to_ticks(f.max(0), rate);
    let mut from = t(pass.start);
    let mut to = t(pass.stop);
    if let Some((ls, le)) = pass.wrapped_loop {
        from = from.min(t(ls));
        to = to.max(t(le));
    }
    for n in notes {
        from = from.min(t(n.start));
        to = to.max(t(n.end));
    }
    let start = from.div_euclid(bar) * bar;
    let mut end = (to.div_euclid(bar) + 1) * bar;
    // A stop exactly on a bar line needs no extra empty bar, unless a note ends there.
    if to % bar == 0 && to > start {
        end = to;
    }
    if let Some((_, le)) = pass.wrapped_loop {
        end = end.max(t(le));
    }
    let mut clip = MidiClip::empty(start, (end - start).max(bar));
    clip.name = name.to_string();
    clip.notes = notes
        .iter()
        .enumerate()
        .map(|(i, n)| {
            let s = t(n.start);
            Note { id: i as u32 + 1, pitch: n.pitch, start: s - start, length: (t(n.end) - s).max(1), velocity: n.velocity.clamp(1, 127) }
        })
        .collect();
    Some(clip)
}

/// One `AddMidiClip` per recording track (in track order), each holding every
/// note of the pass. `tracks` are the instrument tracks that were armed when the
/// pass started (the ones that received its notes); tracks deleted or no longer
/// instrument tracks since are skipped. Empty when nothing was played. Clip ids
/// are left at 0 so the batch assigns them.
pub fn record_commands(project: &Project, rate: f64, tracks: &[TrackId], events: &[CapturedEvent], pass: &Pass) -> Vec<Command> {
    let notes = pair_notes(events, pass);
    project
        .tracks
        .iter()
        .filter(|t| t.kind == TrackKind::Instrument && tracks.contains(&t.id))
        .filter_map(|t| recorded_clip(project, rate, &t.name, &notes, pass).map(|clip| Command::AddMidiClip { track: t.id, clip }))
        .collect()
}

/// The `AddMidiClip` commands whose track still exists as an instrument track
/// (a track can be deleted while a pass's audio takes load).
pub fn still_valid(project: &Project, commands: Vec<Command>) -> Vec<Command> {
    commands
        .into_iter()
        .filter(|c| match c {
            Command::AddMidiClip { track, .. } => project.track(*track).is_some_and(|t| t.kind == TrackKind::Instrument),
            _ => true,
        })
        .collect()
}

/// Armed instrument tracks (the tracks a recording pass records MIDI on).
pub fn armed_instrument_tracks(project: &Project) -> Vec<TrackId> {
    project.tracks.iter().filter(|t| t.kind == TrackKind::Instrument && t.arm).map(|t| t.id).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::PPQ;

    const RATE: f64 = 48_000.0;
    // 120 BPM: one beat = 24 000 frames, one 4/4 bar = 96 000 frames.
    const BEAT: i64 = 24_000;
    const BAR: i64 = 4 * BEAT;

    fn on(frame: i64, pitch: u8, velocity: u8) -> CapturedEvent {
        CapturedEvent { frame, on: true, pitch, velocity }
    }
    fn off(frame: i64, pitch: u8) -> CapturedEvent {
        CapturedEvent { frame, on: false, pitch, velocity: 0 }
    }
    fn pass(start: i64, stop: i64) -> Pass {
        Pass { start, stop, wrapped_loop: None }
    }

    #[test]
    fn pairs_notes_and_closes_held_ones_at_stop() {
        let ev = [on(100, 60, 90), on(200, 64, 80), off(300, 60), on(400, 60, 70), off(500, 99)];
        let n = pair_notes(&ev, &pass(0, 1000));
        assert_eq!(
            n,
            vec![
                FrameNote { start: 100, end: 300, pitch: 60, velocity: 90 },
                FrameNote { start: 200, end: 1000, pitch: 64, velocity: 80 },
                FrameNote { start: 400, end: 1000, pitch: 60, velocity: 70 },
            ]
        );
        // A repeated note-on ends the held note first; velocity 0 is a note-off.
        let n = pair_notes(&[on(0, 60, 100), on(50, 60, 90), on(80, 60, 0)], &pass(0, 1000));
        assert_eq!(n.iter().map(|n| (n.start, n.end)).collect::<Vec<_>>(), [(0, 50), (50, 80)]);
    }

    #[test]
    fn a_note_on_and_off_in_one_callback_is_the_shortest_note() {
        // Same frame: kept, one frame long, looped pass or not (not stretched to the loop end).
        let ev = [on(BAR + 10, 60, 100), off(BAR + 10, 60)];
        assert_eq!(pair_notes(&ev, &pass(0, 2 * BAR)), vec![FrameNote { start: BAR + 10, end: BAR + 11, pitch: 60, velocity: 100 }]);
        let looped = Pass { start: BAR, stop: BAR + BEAT, wrapped_loop: Some((BAR, 3 * BAR)) };
        assert_eq!(pair_notes(&ev, &looped)[0].end, BAR + 11);
    }

    #[test]
    fn clip_spans_from_start_bar_to_bar_after_last_note() {
        let p = Project::default();
        // Record from beat 2 of bar 1 to beat 2 of bar 2; one note in bar 2 held past stop.
        let notes = pair_notes(&[on(BAR + BEAT, 69, 100)], &pass(BEAT, BAR + 2 * BEAT));
        let c = recorded_clip(&p, RATE, "Keys", &notes, &pass(BEAT, BAR + 2 * BEAT)).unwrap();
        assert_eq!((c.start, c.length, c.name.as_str()), (0, 8 * PPQ, "Keys"));
        assert_eq!((c.notes[0].start, c.notes[0].length, c.notes[0].pitch), (5 * PPQ, PPQ, 69));
        // Stopping exactly on a bar line adds no empty bar.
        let pa = pass(BAR, 3 * BAR);
        let notes = pair_notes(&[on(BAR + 10, 60, 100), off(BAR + BEAT, 60)], &pa);
        let c = recorded_clip(&p, RATE, "Keys", &notes, &pa).unwrap();
        assert_eq!((c.start, c.length), (4 * PPQ, 8 * PPQ));
        // Nothing played: no clip.
        assert!(recorded_clip(&p, RATE, "Keys", &[], &pa).is_none());
    }

    #[test]
    fn looped_pass_covers_the_loop_and_overdubs() {
        let p = Project::default();
        // Loop bars 2..4; two passes through it, a note in each, one held across the wrap.
        let pa = Pass { start: BAR, stop: BAR + BEAT, wrapped_loop: Some((BAR, 3 * BAR)) };
        let ev = [on(BAR + BEAT, 60, 100), off(BAR + 2 * BEAT, 60), on(3 * BAR - 1000, 62, 90), off(BAR + 500, 62), on(2 * BAR, 64, 80), off(2 * BAR + BEAT, 64)];
        let notes = pair_notes(&ev, &pa);
        assert_eq!(notes.iter().find(|n| n.pitch == 62).unwrap().end, 3 * BAR, "held across the wrap ends at the loop end");
        let c = recorded_clip(&p, RATE, "Keys", &notes, &pa).unwrap();
        assert_eq!((c.start, c.length), (4 * PPQ, 8 * PPQ));
        assert_eq!(c.notes.len(), 3);
    }

    #[test]
    fn one_clip_per_armed_instrument_track_in_one_undo_step() {
        use crate::{Document, Origin};
        let mut doc = Document::new(Project::default());
        let inst = doc.project().tracks.iter().find(|t| t.kind == TrackKind::Instrument).unwrap().id;
        let audio = doc.project().tracks.iter().find(|t| t.kind == TrackKind::Audio).unwrap().id;
        doc.execute(Command::SetTrackArm { track: inst, arm: true }, Origin::User).unwrap();
        doc.execute(Command::SetTrackArm { track: audio, arm: true }, Origin::User).unwrap();
        assert_eq!(armed_instrument_tracks(doc.project()), vec![inst]);
        let pa = pass(0, 2 * BAR);
        let armed = armed_instrument_tracks(doc.project());
        let cmds = record_commands(doc.project(), RATE, &armed, &[on(BEAT, 60, 100), off(2 * BEAT, 60)], &pa);
        assert_eq!(cmds.len(), 1);
        assert!(record_commands(doc.project(), RATE, &armed, &[], &pa).is_empty());
        // The tracks armed at the start record, even if disarmed since; audio tracks never do.
        assert_eq!(record_commands(doc.project(), RATE, &[inst, audio], &[on(BEAT, 60, 100)], &pa).len(), 1);
        assert!(record_commands(doc.project(), RATE, &[audio], &[on(BEAT, 60, 100)], &pa).is_empty());
        // A track deleted before the commands run is dropped from them.
        assert_eq!(still_valid(doc.project(), cmds.clone()).len(), 1);
        let gone = still_valid(&Project { tracks: vec![], ..doc.project().clone() }, cmds.clone());
        assert!(gone.is_empty());
        let before = doc.project().track(inst).unwrap().midi_clips.len();
        doc.execute(Command::Batch { label: "Record".into(), commands: cmds }, Origin::User).unwrap();
        let t = doc.project().track(inst).unwrap();
        assert_eq!(t.midi_clips.len(), before + 1);
        let c = t.midi_clips.iter().find(|c| c.notes.len() == 1 && c.notes[0].start == PPQ).unwrap();
        assert_eq!(c.length, 8 * PPQ);
        assert_eq!(doc.undo_label().as_deref(), Some("Record"));
        assert!(doc.undo().unwrap());
        assert_eq!(doc.project().track(inst).unwrap().midi_clips.len(), before);
    }
}
