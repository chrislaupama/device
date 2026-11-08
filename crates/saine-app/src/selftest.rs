//! `--self-test`: drives the real UI input path with synthetic Slint window
//! events (pointer + keyboard → TouchArea/FocusScope → callbacks → Commands)
//! and checks the resulting `Document` / engine state. Does not touch the
//! system cursor or keyboard. Coordinates are window points at 1440x820.

use crate::controller::Controller;
use crate::{AppWindow, Studio};
use crate::assistant_panel::EntryKind;
use saine_core::assistant::Mode;
use saine_core::{PPQ, TrackKind, db_to_gain};
use std::cell::Cell;
use slint::platform::{Key, PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, SharedString};
use std::cell::RefCell;
use std::rc::Rc;

type Ctl = Rc<RefCell<Controller>>;

enum Step {
    Do(&'static str, Box<dyn Fn(&AppWindow)>),
    Check(&'static str, Box<dyn Fn(&Controller, &AppWindow) -> Result<(), String>>),
}

fn press(app: &AppWindow, x: f32, y: f32) {
    let w = app.window();
    let p = LogicalPosition::new(x, y);
    w.dispatch_event(WindowEvent::PointerMoved { position: p });
    w.dispatch_event(WindowEvent::PointerPressed { position: p, button: PointerEventButton::Left });
}
fn release(app: &AppWindow, x: f32, y: f32) {
    app.window().dispatch_event(WindowEvent::PointerReleased { position: LogicalPosition::new(x, y), button: PointerEventButton::Left });
}
fn click(app: &AppWindow, x: f32, y: f32) {
    press(app, x, y);
    release(app, x, y);
}
fn drag(app: &AppWindow, x0: f32, y0: f32, x1: f32, y1: f32) {
    press(app, x0, y0);
    for i in 1..=12 {
        let t = i as f32 / 12.0;
        app.window().dispatch_event(WindowEvent::PointerMoved { position: LogicalPosition::new(x0 + (x1 - x0) * t, y0 + (y1 - y0) * t) });
    }
    release(app, x1, y1);
}
fn key(app: &AppWindow, text: impl Into<SharedString> + Clone, primary: bool, shift: bool) {
    let w = app.window();
    if primary {
        w.dispatch_event(WindowEvent::KeyPressed { text: Key::Control.into() });
    }
    if shift {
        w.dispatch_event(WindowEvent::KeyPressed { text: Key::Shift.into() });
    }
    w.dispatch_event(WindowEvent::KeyPressed { text: text.clone().into() });
    w.dispatch_event(WindowEvent::KeyReleased { text: text.into() });
    if shift {
        w.dispatch_event(WindowEvent::KeyReleased { text: Key::Shift.into() });
    }
    if primary {
        w.dispatch_event(WindowEvent::KeyReleased { text: Key::Control.into() });
    }
}

fn type_text(app: &AppWindow, s: &str) {
    let w = app.window();
    for ch in s.chars() {
        let t: SharedString = ch.to_string().into();
        w.dispatch_event(WindowEvent::KeyPressed { text: t.clone() });
        w.dispatch_event(WindowEvent::KeyReleased { text: t });
    }
}

fn approx(a: f32, b: f32, tol: f32) -> bool {
    (a - b).abs() <= tol
}

// Window geometry (reference layout): lanes origin and track rows.
const LANE_X: f32 = 453.0;
const TRACK0_Y: f32 = 144.5;
const PITCH: f32 = 74.5;

fn clip_start_beats(c: &Controller, track: usize) -> Option<f64> {
    c.project().tracks.get(track)?.clips.first().map(|cl| cl.start as f64 / PPQ as f64)
}

fn steps() -> Vec<Step> {
    use Step::*;
    // Values recorded by one check and compared by a later one.
    let rec_tracks = Rc::new(Cell::new(0usize));
    let rec_gain = Rc::new(Cell::new(0f32));
    let rec_rev = Rc::new(Cell::new(0u64));
    let rec_vol_steps = Rc::new(Cell::new(0usize));
    let rec_mute = Rc::new(Cell::new(false));
    let (rt1, rt2, rt3, rt4, rt5) = (rec_tracks.clone(), rec_tracks.clone(), rec_tracks.clone(), rec_tracks.clone(), rec_tracks.clone());
    let (rg1, rg2) = (rec_gain.clone(), rec_gain.clone());
    let (rr1, rr2) = (rec_rev.clone(), rec_rev.clone());
    let (rv1, rv2) = (rec_vol_steps.clone(), rec_vol_steps.clone());
    let (rm1, rm2) = (rec_mute.clone(), rec_mute.clone());
    // Assistant panel geometry at 1440x820 with the mixer closed (see ui/assistant.slint).
    const CARD_X: f32 = 1440.0 - 5.0 - 252.5 + 7.0;
    const CARD_Y: f32 = 78.0 + 426.0 - 8.0 - 119.5;
    const MIXER_Y: f32 = 820.0 - 5.0 - 390.0;
    let bx = |b: f64| LANE_X + (b * crate::timeline::DEFAULT_PX_PER_BEAT) as f32;
    vec![
        Check("demo imported two clips on two audio tracks", Box::new(|c, _| {
            let p = c.project();
            let n: usize = p.tracks.iter().map(|t| t.clips.len()).sum();
            (n == 2 && p.tracks.len() == 3 && p.assets.len() == 2).then_some(()).ok_or(format!("tracks {} clips {n}", p.tracks.len()))
        })),
        Check("second clip placed at the playhead (beat 10)", Box::new(|c, _| {
            let s = clip_start_beats(c, 2).ok_or("no clip on track 3")?;
            ((s - 10.0).abs() < 1e-6).then_some(()).ok_or(format!("start {s}"))
        })),
        Do("click track 2 header", Box::new(|a| click(a, 300.0, TRACK0_Y + PITCH + 10.0))),
        Check("track 2 selected, inspector follows", Box::new(|c, a| {
            let t = c.project().tracks[1].id;
            let s = a.global::<Studio>();
            (c.selected_track == Some(t) && s.get_sel_number() == 2 && s.get_sel_name() == "Instrument").then_some(()).ok_or("selection".into())
        })),
        Do("click M on track 1", Box::new(|a| click(a, 262.0 + 33.5, TRACK0_Y + 47.0))),
        Check("track 1 muted (SetTrackMute)", Box::new(|c, _| c.project().tracks[0].mute.then_some(()).ok_or("not muted".into()))),
        Do("Cmd+Z", Box::new(|a| key(a, "z", true, false))),
        Check("undo restored mute=false", Box::new(|c, _| (!c.project().tracks[0].mute).then_some(()).ok_or("still muted".into()))),
        Do("Cmd+Shift+Z", Box::new(|a| key(a, "Z", true, true))),
        Check("redo muted again", Box::new(|c, _| c.project().tracks[0].mute.then_some(()).ok_or("redo failed".into()))),
        Do("drag track 1 volume slider left", Box::new(|a| drag(a, 262.0 + 116.0 + 0.85 * 61.0 + 3.5, TRACK0_Y + 48.0, 262.0 + 140.0, TRACK0_Y + 48.0))),
        Check("track 1 gain lowered, one coalesced undo step", Box::new(|c, _| {
            let g = c.project().tracks[0].gain;
            let labels = c.doc.history_labels();
            let n = labels.iter().filter(|l| l.as_str() == "Track Volume Change").count();
            (g < 0.5 && n == 1).then_some(()).ok_or(format!("gain {g}, volume steps {n}"))
        })),
        Do("drag clip 1 body right by ~2 bars", Box::new(move |a| drag(a, bx(1.0), TRACK0_Y + 40.0, bx(1.0) + 102.3, TRACK0_Y + 40.0))),
        Check("clip moved to beat 8 (snapped)", Box::new(|c, _| {
            let s = clip_start_beats(c, 0).ok_or("clip gone")?;
            ((s - 8.0).abs() < 1e-6).then_some(()).ok_or(format!("start {s}"))
        })),
        Do("drag clip 1 right edge left by 2 beats", Box::new(move |a| drag(a, bx(16.0) - 2.0, TRACK0_Y + 40.0, bx(14.0) - 2.0, TRACK0_Y + 40.0))),
        Check("clip trimmed to 6 beats (TrimClip)", Box::new(|c, _| {
            let p = c.project();
            let cl = &p.tracks[0].clips[0];
            let beats = p.clip_length_ticks(cl) as f64 / PPQ as f64;
            ((beats - 6.0).abs() < 0.01).then_some(()).ok_or(format!("len {beats} beats"))
        })),
        Do("drag clip 1 left edge right by 1 beat", Box::new(move |a| drag(a, bx(8.0) + 1.5, TRACK0_Y + 40.0, bx(9.0) + 1.5, TRACK0_Y + 40.0))),
        Check("left trim: start 9, offset 0.5 s of source, length 5 beats", Box::new(|c, _| {
            let p = c.project();
            let cl = &p.tracks[0].clips[0];
            let a = p.asset(cl.asset).unwrap();
            let off_s = cl.source_offset as f64 / a.sample_rate;
            let beats = p.clip_length_ticks(cl) as f64 / PPQ as f64;
            ((cl.start as f64 / PPQ as f64 - 9.0).abs() < 1e-6 && (off_s - 0.5).abs() < 1e-3 && (beats - 5.0).abs() < 0.01)
                .then_some(())
                .ok_or(format!("start {} off {off_s}s len {beats}", cl.start))
        })),
        Do("click ruler at bar 5", Box::new(move |a| click(a, bx(16.0), 112.0))),
        Check("engine playhead at bar 5 (Bar readout 005, Time 00:08.000)", Box::new(|_, a| {
            let s = a.global::<Studio>();
            (s.get_bar_text() == "005" && s.get_time_text() == "00:08.000").then_some(()).ok_or(format!("{} {}", s.get_bar_text(), s.get_time_text()))
        })),
        Do("Enter (stop + return)", Box::new(|a| key(a, Key::Return, false, false))),
        Check("playhead back at 0", Box::new(|c, _| (c.engine.transport().position == 0).then_some(()).ok_or(format!("pos {}", c.engine.transport().position)))),
        Do("click loop button", Box::new(|a| click(a, 481.0, 52.0))),
        Check("loop enabled in project + UI, undoable (SetLoop)", Box::new(|c, a| {
            let on = c.project().transport.loop_enabled;
            let ui = a.global::<Studio>().get_loop_enabled();
            (on && ui && c.doc.undo_label() == Some("Loop Change")).then_some(()).ok_or(format!("model {on} ui {ui} undo {:?}", c.doc.undo_label()))
        })),
        Do("click loop button again", Box::new(|a| click(a, 481.0, 52.0))),
        Check("loop disabled", Box::new(|c, _| (!c.project().transport.loop_enabled).then_some(()).ok_or("still on".into()))),
        Do("click metronome button", Box::new(|a| click(a, 992.0, 52.0))),
        Check("metronome on in controller + UI", Box::new(|c, a| {
            (c.metronome_on && a.global::<Studio>().get_metronome_on()).then_some(()).ok_or("metronome off".into())
        })),
        Check("engine clicks land on the 120 BPM beat grid (offline render)", Box::new(|c, _| {
            if c.engine.device_running() {
                return Ok(()); // offline render is refused while the device owns the audio path
            }
            let e = &c.engine;
            e.seek(0).map_err(|e| e.to_string())?;
            e.play().map_err(|e| e.to_string())?;
            let mut buf = vec![0f32; 2 * 48_100];
            let r = e.render_offline(&mut buf);
            let _ = e.stop();
            r.map_err(|e| e.to_string())?;
            let at = |f: usize| buf[2 * f].abs() + buf[2 * f + 1].abs();
            let loud = |a: usize, b: usize| (a..b).any(|f| at(f) > 1e-3);
            // Clips start at beats 9/10, so the first 2 s hold only clicks:
            // beat 0 at frame 0, beat 1 at 24000, beat 2 at 48000; silence between tails.
            (loud(0, 50) && !loud(2_000, 23_990) && loud(24_000, 24_050) && !loud(26_000, 47_990) && loud(48_000, 48_050))
                .then_some(())
                .ok_or("click positions wrong".into())
        })),
        Do("click metronome button again", Box::new(|a| click(a, 992.0, 52.0))),
        Check("metronome off", Box::new(|c, _| (!c.metronome_on).then_some(()).ok_or("still on".into()))),
        Do("select clip 1 and press Delete", Box::new(move |a| {
            click(a, bx(11.0), TRACK0_Y + 40.0);
            key(a, Key::Delete, false, false)
        })),
        Check("clip deleted (RemoveClip)", Box::new(|c, _| c.project().tracks[0].clips.is_empty().then_some(()).ok_or("clip still there".into()))),
        Do("Cmd+Z", Box::new(|a| key(a, "z", true, false))),
        Check("undo restored the clip", Box::new(|c, _| (c.project().tracks[0].clips.len() == 1).then_some(()).ok_or("not restored".into()))),
        Do("click 'Add track'", Box::new(|a| click(a, 300.0, 124.0))),
        Check("4 tracks, new one is Audio and selected", Box::new(|c, _| {
            let p = c.project();
            let t = p.tracks.last().unwrap();
            (p.tracks.len() == 4 && t.kind == TrackKind::Audio && c.selected_track == Some(t.id)).then_some(()).ok_or(format!("{} tracks", p.tracks.len()))
        })),
        Do("Delete (track header selected)", Box::new(|a| key(a, Key::Backspace, false, false))),
        Check("track deleted (RemoveTrack)", Box::new(|c, _| (c.project().tracks.len() == 3).then_some(()).ok_or(format!("{} tracks", c.project().tracks.len())))),
        Do("click track 1 header", Box::new(|a| click(a, 300.0, TRACK0_Y + 10.0))),
        Do("drag inspector pan knob up 30 pt", Box::new(|a| drag(a, 5.0 + 47.0, 509.0 + 210.0, 5.0 + 47.0, 509.0 + 210.0 - 30.0))),
        Check("track 1 pan moved right, readout updated", Box::new(|c, a| {
            let pan = c.project().tracks[0].pan;
            let txt = a.global::<Studio>().get_sel_pan_text();
            (pan > 0.2 && txt.ends_with('R')).then_some(()).ok_or(format!("pan {pan} text {txt}"))
        })),
        Do("(audio) unmute track 1 (muted by the redo step)", Box::new(|a| click(a, 262.0 + 33.5, TRACK0_Y + 47.0))),
        Do("(audio) click ruler at beat 9 (start of track 1 clip)", Box::new(move |a| click(a, bx(9.0), 112.0))),
        Do("(audio) master -40 dB, Space", Box::new(|a| {
            a.global::<Studio>().invoke_master_volume(0.85 * 0.01f32.cbrt());
            key(a, " ", false, false)
        })),
        Do("wait", Box::new(|_| {})),
        Do("wait", Box::new(|_| {})),
        Do("wait", Box::new(|_| {})),
        Do("wait", Box::new(|_| {})),
        Do("wait", Box::new(|_| {})),
        Do("wait", Box::new(|_| {})),
        Check("(audio) transport running: Time readout advanced, selected-track meter > 0", Box::new(|c, a| {
            if !c.engine.device_running() {
                return Ok(()); // offline run: Space only shows a status message
            }
            let s = a.global::<Studio>();
            let t = s.get_time_text();
            let playing = c.engine.transport().playing;
            (playing && t.as_str() > "00:04.700" && s.get_sel_meter_l() < 0.999).then_some(()).ok_or(format!("playing {playing} time {t} meter {}", s.get_sel_meter_l()))
        })),
        Do("Space (pause)", Box::new(|a| key(a, " ", false, false))),
        Check("(audio) paused", Box::new(|c, _| (!c.engine.transport().playing).then_some(()).ok_or("still playing".into()))),
        Do("wheel: scroll timeline right", Box::new(|a| {
            let w = a.window();
            w.dispatch_event(WindowEvent::PointerMoved { position: LogicalPosition::new(900.0, 300.0) });
            w.dispatch_event(WindowEvent::PointerScrolled { position: LogicalPosition::new(900.0, 300.0), delta_x: -51.0, delta_y: 0.0 });
        })),
        Check("view scrolled by ~4 beats", Box::new(|c, _| ((c.view.start_beats - 4.0).abs() < 0.2).then_some(()).ok_or(format!("start {}", c.view.start_beats)))),
        Do("Cmd + wheel: zoom in around x=900", Box::new(|a| {
            let w = a.window();
            w.dispatch_event(WindowEvent::KeyPressed { text: Key::Control.into() });
            w.dispatch_event(WindowEvent::PointerScrolled { position: LogicalPosition::new(900.0, 300.0), delta_x: 0.0, delta_y: 50.0 });
            w.dispatch_event(WindowEvent::KeyReleased { text: Key::Control.into() });
        })),
        Check("zoom changed px/beat, beat under the cursor kept", Box::new(|c, _| {
            let ppb = c.view.px_per_beat;
            let anchor = c.view.beat_at(900.0 - LANE_X);
            let expected = 4.0 + (900.0 - LANE_X) as f64 / crate::timeline::DEFAULT_PX_PER_BEAT;
            ((ppb - crate::timeline::DEFAULT_PX_PER_BEAT).abs() > 1.0 && (anchor - expected).abs() < 0.05).then_some(()).ok_or(format!("ppb {ppb} anchor {anchor} vs {expected}"))
        })),
        // ---------------- assistant panel (TEST fixture provider, no network) ----------------
        Do("click the assistant toggle", Box::new(|a| click(a, 1440.0 - 147.0 + 12.5, 48.0))),
        Check("assistant panel open, timeline narrower, fixture provider available", Box::new(|c, a| {
            let w = a.get_lanes_width();
            let s = a.global::<Studio>();
            (c.assistant_open && s.get_assistant_open() && s.get_chat_available() && (w - (1440.0 - 262.0 - 5.0 - 257.5 - 191.0)).abs() < 1.0)
                .then_some(())
                .ok_or(format!("open {} lanes width {w}", c.assistant_open))
        })),
        Check("recorded project state before the assistant turn", Box::new(move |c, _| {
            rt1.set(c.project().tracks.len());
            rg1.set(c.project().tracks[0].gain);
            Ok(())
        })),
        Do("click the input and type a prompt (Slint key events)", Box::new(|a| {
            click(a, CARD_X + 60.0, CARD_Y + 27.0 + 46.0);
            type_text(a, "add drums, tempo 128, audio down 6 db");
        })),
        Check("typed text reached the composer", Box::new(|c, a| {
            let d = a.global::<Studio>().get_chat_draft();
            (d == "add drums, tempo 128, audio down 6 db" && c.assistant.mode == Mode::Assist).then_some(()).ok_or(format!("draft {d:?}"))
        })),
        Do("Enter (send in Assist mode)", Box::new(|a| key(a, Key::Return, false, false))),
        Check("Assist turn applied: track added, 128 BPM, Audio at -6 dB, one undo step", Box::new(move |c, _| {
            let p = c.project();
            let ok = p.tracks.len() == rt2.get() + 1
                && p.tracks.last().is_some_and(|t| t.name == "Drums")
                && p.transport.bpm == 128.0
                && approx(p.tracks[0].gain, db_to_gain(-6.0), 1e-4)
                && c.doc.undo_label() == Some("Assistant Edit");
            ok.then_some(()).ok_or(format!("tracks {} bpm {} gain {} undo {:?}", p.tracks.len(), p.transport.bpm, p.tracks[0].gain, c.doc.undo_label()))
        })),
        Check("transcript shows the prompt and the plan summary lines", Box::new(|c, a| {
            let e = c.assistant.last().ok_or("empty transcript")?;
            let want = ["Added an audio track, Drums.", "Changed tempo to 128 BPM.", "Set Audio volume to -6.0 dB."];
            let s = a.global::<Studio>();
            let chat = s.get_chat();
            let rows = slint::Model::row_count(&chat);
            let ui_lines = slint::Model::row_data(&chat, rows.saturating_sub(1)).map(|r| slint::Model::row_count(&r.lines)).unwrap_or(0);
            let user_ok = slint::Model::row_data(&chat, rows.saturating_sub(2)).is_some_and(|r| r.kind == 0 && r.text.starts_with("add drums"));
            (e.kind == EntryKind::Reply && e.lines == want && rows == 2 && ui_lines == 3 && user_ok && s.get_chat_draft().is_empty())
                .then_some(())
                .ok_or(format!("{:?} rows {rows} ui lines {ui_lines}", e.lines))
        })),
        Do("Cmd+Z", Box::new(|a| key(a, "z", true, false))),
        Check("one undo reverted the whole assistant turn", Box::new(move |c, _| {
            let p = c.project();
            (p.tracks.len() == rt3.get() && p.transport.bpm == 120.0 && approx(p.tracks[0].gain, rg2.get(), 1e-6))
                .then_some(())
                .ok_or(format!("tracks {} bpm {} gain {}", p.tracks.len(), p.transport.bpm, p.tracks[0].gain))
        })),
        Do("click Teach", Box::new(|a| click(a, CARD_X + 6.5 + 57.0 + 27.0, CARD_Y + 27.0 + 66.5 + 10.0))),
        Check("Teach mode active", Box::new(move |c, a| {
            rr1.set(c.doc.revision());
            (c.assistant.mode == Mode::Teach && a.global::<Studio>().get_teach_mode()).then_some(()).ok_or("still Assist".into())
        })),
        Do("type + Enter (fixture turn with an edit)", Box::new(|a| {
            click(a, CARD_X + 60.0, CARD_Y + 27.0 + 46.0);
            type_text(a, "slow it down");
            key(a, Key::Return, false, false);
        })),
        Check("Teach edit rejected: error entry, project unchanged", Box::new(move |c, _| {
            let e = c.assistant.last().ok_or("empty transcript")?;
            (e.kind == EntryKind::Error && e.text.contains("teach mode") && c.doc.revision() == rr2.get() && c.project().transport.bpm == 120.0)
                .then_some(())
                .ok_or(format!("{:?} {:?} rev {} vs {}", e.kind, e.text, c.doc.revision(), rr2.get()))
        })),
        Do("type + Enter (fixture turn highlighting transport.bpm)", Box::new(|a| {
            click(a, CARD_X + 60.0, CARD_Y + 27.0 + 46.0);
            type_text(a, "where is the tempo");
            key(a, Key::Return, false, false);
        })),
        Check("highlight active on transport.bpm, overlay rect mapped", Box::new(|c, a| {
            let hl = c.assistant.highlights.iter().any(|h| h.target == "transport.bpm");
            let rects = a.global::<Studio>().get_highlights();
            let r = slint::Model::row_data(&rects, 0);
            let mapped = r.as_ref().is_some_and(|r| r.x == 700.0 && r.y == 42.0);
            let note = c.assistant.last().is_some_and(|e| e.lines.iter().any(|l| l.starts_with("Tempo (BPM)")));
            (hl && mapped && note && slint::Model::row_count(&rects) == 1).then_some(()).ok_or(format!("hl {hl} rect {r:?} note {note}"))
        })),
        // ---------------- mixer ----------------
        Do("click the mixer toggle", Box::new(|a| click(a, 1440.0 - 119.0 + 12.5, 47.0))),
        Check("mixer open: bottom panel 390 pt, timeline lanes shorter", Box::new(move |c, a| {
            let h = a.get_lanes_height();
            rt4.set(c.project().tracks.len());
            (c.mixer_open && a.global::<Studio>().get_mixer_open() && (h - (342.0 - 27.5)).abs() < 1.0).then_some(()).ok_or(format!("open {} lanes h {h}", c.mixer_open))
        })),
        Do("click the Instrument strip header", Box::new(|a| click(a, 5.0 + 91.5 + 40.0, MIXER_Y + 35.0))),
        Check("strip click selected track 2", Box::new(move |c, _| {
            rv1.set(c.doc.history_labels().iter().filter(|l| l.as_str() == "Track Volume Change").count());
            rm1.set(c.project().tracks[0].mute);
            (c.selected_track == Some(c.project().tracks[1].id)).then_some(()).ok_or("not selected".into())
        })),
        Do("drag the Audio strip fader from -24 dB to -36 dB", Box::new(|a| {
            let y = |f: f32| MIXER_Y + 196.0 + 10.5 + f * 143.0;
            drag(a, 5.0 + 45.0, y(0.5), 5.0 + 45.0, y(crate::controller::fader_from_db(-36.0)));
        })),
        Check("track 1 gain ≈ -36 dB, exactly one new undo step", Box::new(move |c, _| {
            let g = c.project().tracks[0].gain;
            let db = saine_core::gain_to_db(g);
            let n = c.doc.history_labels().iter().filter(|l| l.as_str() == "Track Volume Change").count();
            ((db + 36.0).abs() < 0.3 && n == rv2.get() + 1).then_some(()).ok_or(format!("{db} dB, volume steps {n} (was {})", rv2.get()))
        })),
        Do("click M on the Audio strip", Box::new(|a| click(a, 5.0 + 14.0 + 7.5, MIXER_Y + 368.5 + 7.0))),
        Check("track 1 mute toggled via the mixer (SetTrackMute)", Box::new(move |c, _| {
            (c.project().tracks[0].mute != rm2.get() && c.doc.undo_label() == Some("Mute Change")).then_some(()).ok_or(format!("mute {} undo {:?}", c.project().tracks[0].mute, c.doc.undo_label()))
        })),
        Do("drag the master strip fader to ~-24 dB", Box::new(|a| {
            let y = |f: f32| MIXER_Y + 196.0 + 10.5 + f * 143.0;
            drag(a, 1440.0 - 5.0 - 92.0 + 45.0, y(0.3), 1440.0 - 5.0 - 92.0 + 45.0, y(0.5));
        })),
        Check("master gain ≈ -24 dB (SetMasterGain), mixer master fader follows", Box::new(|c, a| {
            let db = saine_core::gain_to_db(c.project().master_gain);
            let f = a.global::<Studio>().get_master_fader();
            ((db + 24.0).abs() < 0.3 && (f - 0.5).abs() < 0.01).then_some(()).ok_or(format!("{db} dB, fader {f}"))
        })),
        Do("click '+ Add track' in the mixer", Box::new(|a| click(a, 5.0 + 3.0 * 91.5 + 46.0, MIXER_Y + 190.0))),
        Check("mixer Add track added an audio track", Box::new(move |c, _| {
            let p = c.project();
            (p.tracks.len() == rt5.get() + 1 && p.tracks.last().unwrap().kind == TrackKind::Audio).then_some(()).ok_or(format!("{} tracks", p.tracks.len()))
        })),
        Do("Cmd+Shift+X (view.toggle_mixer)", Box::new(|a| key(a, "X", true, true))),
        Check("mixer closed by the shortcut, bottom panel back", Box::new(|c, a| {
            let h = a.get_lanes_height();
            (!c.mixer_open && (h - (426.0 - 27.5)).abs() < 1.0).then_some(()).ok_or(format!("open {} lanes h {h}", c.mixer_open))
        })),
    ]
}

/// Run the scripted steps, one per timer tick. Prints a summary and quits.
pub fn start(app: &AppWindow, ctl: Ctl) {
    let steps = Rc::new(steps());
    let idx = Rc::new(std::cell::Cell::new(0usize));
    let fails = Rc::new(std::cell::Cell::new(0usize));
    let timer = Rc::new(slint::Timer::default());
    let aw = app.as_weak();
    let t2 = timer.clone();
    timer.start(slint::TimerMode::Repeated, std::time::Duration::from_millis(120), move || {
        let Some(app) = aw.upgrade() else { return };
        {
            let Ok(mut c) = ctl.try_borrow_mut() else { return };
            if c.pending_jobs > 0 {
                c.poll_jobs();
                return;
            }
        }
        let i = idx.get();
        if i >= steps.len() {
            let n = steps.iter().filter(|s| matches!(s, Step::Check(..))).count();
            println!("self-test: {} / {n} checks passed", n - fails.get());
            t2.stop();
            let _ = slint::quit_event_loop();
            return;
        }
        match &steps[i] {
            Step::Do(name, f) => {
                eprintln!("self-test: > {name}");
                // Push models first, as a rendered frame would: when the window is not
                // presenting frames (occluded/locked screen) the UI would otherwise act
                // on stale view models.
                if let Ok(mut c) = ctl.try_borrow_mut() {
                    c.update_ui(&app);
                }
                f(&app);
            }
            Step::Check(name, f) => {
                // Let the frame update readouts first.
                if let Ok(mut c) = ctl.try_borrow_mut() {
                    c.update_ui(&app);
                }
                let r = f(&ctl.borrow(), &app);
                match r {
                    Ok(()) => println!("PASS {name}"),
                    Err(e) => {
                        fails.set(fails.get() + 1);
                        println!("FAIL {name}: {e}");
                    }
                }
            }
        }
        idx.set(i + 1);
    });
    // Keep the timer alive for the lifetime of the event loop.
    std::mem::forget(timer);
}
