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
    /// Like `Do`, but computes coordinates from the controller first (the closure must
    /// release its borrow before dispatching events).
    DoC(&'static str, Box<dyn Fn(&AppWindow, &Ctl)>),
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

/// Piano-roll texture origin in the window (bottom panel 390 pt at y 425, x 5 + 255).
const PR_ORIGIN: (f32, f32) = (260.0, 820.0 - 5.0 - 390.0 + 54.5);

/// Window point at the centre of a piano-roll cell (beat is clip-relative).
fn pr_xy(ctl: &Ctl, beat: f64, pitch: i32) -> (f32, f32) {
    let v = ctl.borrow().pr_view();
    (PR_ORIGIN.0 + v.x_of(beat), PR_ORIGIN.1 + v.y_of(pitch) + crate::pianoroll::ROW_H / 2.0)
}

fn instrument_clip(c: &Controller) -> Option<&saine_core::MidiClip> {
    c.project().tracks.iter().find(|t| t.kind == TrackKind::Instrument)?.midi_clips.first()
}

fn notes_of(c: &Controller) -> Vec<saine_core::Note> {
    instrument_clip(c).map(|m| m.notes.clone()).unwrap_or_default()
}

/// MIDI clip + piano roll steps (appended to the main script).
fn midi_steps() -> Vec<Step> {
    use Step::*;
    let hist = Rc::new(Cell::new(0usize));
    let saved: Rc<RefCell<(Vec<saine_core::Note>, i64)>> = Rc::new(RefCell::new((Vec::new(), 0)));
    let h = |n: usize| -> Vec<Rc<Cell<usize>>> { (0..n).map(|_| hist.clone()).collect() };
    let hs = h(16);
    let (sv1, sv2) = (saved.clone(), saved.clone());
    let lane_xy = |ctl: &Ctl, beat: f64, track: usize| -> (f32, f32) {
        let c = ctl.borrow();
        (LANE_X + c.view.x_of(beat), TRACK0_Y + track as f32 * PITCH + 40.0 - c.view.scroll_y as f32)
    };
    let record = |hc: Rc<Cell<usize>>| -> Box<dyn Fn(&Controller, &AppWindow) -> Result<(), String>> {
        Box::new(move |c, _| {
            hc.set(c.doc.history_labels().len());
            Ok(())
        })
    };
    // One new undo step since the last record, with this label; re-records.
    let one_step = |hc: Rc<Cell<usize>>, label: &'static str| {
        move |c: &Controller| -> Result<(), String> {
            let n = c.doc.history_labels().len();
            let ok = n == hc.get() + 1 && c.doc.undo_label() == Some(label);
            hc.set(n);
            ok.then_some(()).ok_or(format!("history {} -> {n}, last {:?} (want {label})", hc.get(), c.doc.undo_label()))
        }
    };
    let [h0, h1, h2, h3, h4, h5, h6, h7, h8, h9, h10, h11, h12, h13, h14, h15] = <[Rc<Cell<usize>>; 16]>::try_from(hs).ok().unwrap();
    vec![
        Do("Enter (playhead to 0)", Box::new(|a| key(a, Key::Return, false, false))),
        DoC("wheel back to the start of the timeline", Box::new(|a, _| {
            let w = a.window();
            w.dispatch_event(WindowEvent::PointerMoved { position: LogicalPosition::new(900.0, 300.0) });
            w.dispatch_event(WindowEvent::PointerScrolled { position: LogicalPosition::new(900.0, 300.0), delta_x: 4000.0, delta_y: 0.0 });
        })),
        DoC("click the Instrument track header", Box::new(|a, ctl| {
            let sy = ctl.borrow().view.scroll_y as f32;
            click(a, 300.0, TRACK0_Y + PITCH + 10.0 - sy);
        })),
        Check("record history", record(h0.clone())),
        Do("Cmd+Shift+M (clip.insert_midi)", Box::new(|a| key(a, "M", true, true))),
        Check("1-bar MIDI clip at the playhead, Clip tab open, nothing selected, one undo step", Box::new(move |c, a| {
            let m = instrument_clip(c).ok_or("no MIDI clip")?;
            let s = a.global::<Studio>();
            one_step(h0.clone(), "Insert MIDI Clip")(c)?;
            (m.start == 0 && m.length == 4 * PPQ && m.name == "MIDI" && s.get_bottom_tab() == 1 && !s.get_pr_visible() && c.selected_clip.is_none())
                .then_some(())
                .ok_or(format!("start {} len {} tab {} pr {}", m.start, m.length, s.get_bottom_tab(), s.get_pr_visible()))
        })),
        DoC("click the MIDI clip in the lane", Box::new(move |a, ctl| {
            let (x, y) = lane_xy(ctl, 2.0, 1);
            click(a, x, y);
        })),
        Check("clip selected, piano roll open", Box::new(|c, a| {
            let id = instrument_clip(c).map(|m| m.id);
            (c.selected_clip == id && a.global::<Studio>().get_pr_visible() && c.pr_visible()).then_some(()).ok_or(format!("sel {:?} vs {id:?}", c.selected_clip))
        })),
        DoC("click empty grid (focus) and press B (pen)", Box::new(|a, ctl| {
            let (x, y) = pr_xy(ctl, 3.5, 50);
            click(a, x, y);
            key(a, "b", false, false);
        })),
        Check("pen tool on (midi.paint_pen), record history", Box::new(move |c, a| {
            h1.set(c.doc.history_labels().len());
            (c.pr.pen && c.pr.focused && a.global::<Studio>().get_pr_pen()).then_some(()).ok_or("pen off".into())
        })),
        DoC("draw 3 notes with the pen (E3 at 1.1, G3 at 1.2, B3 at 1.3)", Box::new(|a, ctl| {
            for (beat, pitch) in [(0.1, 52), (1.1, 55), (2.1, 59)] {
                let (x, y) = pr_xy(ctl, beat, pitch);
                click(a, x, y);
            }
        })),
        Check("3 notes: pitches 52/55/59, starts 0/960/1920, length 1/16; 3 undo steps", Box::new(move |c, _| {
            let n = notes_of(c);
            let got: Vec<(u8, i64, i64)> = n.iter().map(|n| (n.pitch, n.start, n.length)).collect();
            let steps = c.doc.history_labels().len() - h2.get();
            h2.set(c.doc.history_labels().len());
            (got == vec![(52, 0, 240), (55, 960, 240), (59, 1920, 240)] && steps == 3).then_some(()).ok_or(format!("{got:?}, {steps} steps"))
        })),
        Check("drawn notes sound: instrument track meter > 0 in an offline render from the clip start", Box::new(|c, _| {
            if c.engine.device_running() {
                return Ok(()); // offline render is refused while the device owns the audio path
            }
            let p = c.project();
            let t = p.tracks.iter().find(|t| !t.midi_clips.is_empty()).ok_or("no MIDI clip")?;
            let start = p.ticks_to_frames(t.midi_clips[0].start, c.engine.sample_rate());
            let e = &c.engine;
            e.seek(start).map_err(|e| e.to_string())?;
            e.play().map_err(|e| e.to_string())?;
            let mut buf = vec![0f32; 2 * 4_800]; // first 100 ms: inside the first note (E3, 1/16 = 125 ms)
            let r = e.render_offline(&mut buf);
            let mut meters = Vec::new();
            e.peek_meters(&mut meters, 64);
            let _ = e.stop();
            r.map_err(|e| e.to_string())?;
            let m = meters.iter().find(|m| m.track_id == t.id.0).map(|m| m.peak_l.max(m.peak_r)).unwrap_or(0.0);
            (m > 0.01).then_some(()).ok_or(format!("instrument meter {m}"))
        })),
        DoC("drag the G3 note up 2 semitones", Box::new(|a, ctl| {
            let (x, y) = pr_xy(ctl, 1.1, 55);
            drag(a, x, y, x, y - 2.0 * crate::pianoroll::ROW_H);
        })),
        Check("note moved to A3 (57), start kept, one undo step", Box::new(move |c, _| {
            let n = notes_of(c);
            one_step(h3.clone(), "Edit Notes")(c)?;
            n.iter().any(|n| n.pitch == 57 && n.start == 960).then_some(()).ok_or(format!("{n:?}"))
        })),
        DoC("drag the B3 note's right edge to beat 3", Box::new(|a, ctl| {
            let (x0, y) = pr_xy(ctl, 2.25, 59);
            let (x1, _) = pr_xy(ctl, 3.0, 59);
            drag(a, x0 - 1.5, y, x1 - 1.5, y);
        })),
        Check("B3 note resized to one beat (960 ticks), one undo step", Box::new(move |c, _| {
            let n = notes_of(c);
            one_step(h4.clone(), "Edit Notes")(c)?;
            n.iter().any(|n| n.pitch == 59 && n.start == 1920 && n.length == 960).then_some(()).ok_or(format!("{n:?}"))
        })),
        DoC("Grid 1/32, draw an off-grid note at beat 3.375, Grid back to 1/16", Box::new(|a, ctl| {
            let (gx, gy) = (260.0 + 387.0 + 8.0, 452.0 + (390.0 - 27.0) - 54.5 + 30.0 + 7.0);
            click(a, gx, gy); // 1/16 -> 1/32
            let (x, y) = pr_xy(ctl, 3.375 + 0.03, 52);
            click(a, x, y);
            for _ in 0..3 {
                click(a, gx, gy); // 1/32 -> 1/4 -> 1/8 -> 1/16
            }
        })),
        Check("off-grid note drawn at 3240 ticks, grid back at 1/16", Box::new(move |c, a| {
            let n = notes_of(c);
            h13.set(c.doc.history_labels().len());
            (n.iter().any(|n| n.start == 3240 && n.pitch == 52) && c.pr.grid == 16 && a.global::<Studio>().get_pr_grid_text() == "1/16")
                .then_some(())
                .ok_or(format!("grid {} {n:?}", c.pr.grid))
        })),
        Do("click Quantize (selection = the new note)", Box::new(|a| click(a, 267.0 + 15.0, 466.0))),
        Check("Quantize snapped it to the 1/16 grid (3360), one undo step", Box::new(move |c, _| {
            let n = notes_of(c);
            one_step(h5.clone(), "Quantize Notes")(c)?;
            (n.iter().all(|n| n.start % 240 == 0) && n.iter().any(|n| n.start == 3360 && n.pitch == 52)).then_some(()).ok_or(format!("{n:?}"))
        })),
        DoC("click the E3 note at beat 0, press Up", Box::new(|a, ctl| {
            let (x, y) = pr_xy(ctl, 0.1, 52);
            click(a, x, y);
            key(a, Key::UpArrow, false, false);
        })),
        Check("↑ transposed only the selected note (52 -> 53), one undo step", Box::new(move |c, _| {
            let n = notes_of(c);
            one_step(h6.clone(), "Transpose Notes")(c)?;
            let first = n.iter().find(|n| n.start == 0).ok_or("no note at 0")?;
            (first.pitch == 53 && n.iter().filter(|n| n.pitch == 52).count() == 1).then_some(()).ok_or(format!("{n:?}"))
        })),
        DoC("Shift+Up (octave), then B (pen off) and click empty grid (deselect)", Box::new(|a, ctl| {
            key(a, Key::UpArrow, false, true);
            key(a, "b", false, false);
            let (x, y) = pr_xy(ctl, 2.9, 50);
            click(a, x, y);
        })),
        Check("⇧↑ moved it an octave (65), pen off, nothing selected; record notes", Box::new(move |c, _| {
            let n = notes_of(c);
            *sv1.borrow_mut() = (n.clone(), instrument_clip(c).map(|m| m.length).unwrap_or(0));
            h14.set(c.doc.history_labels().len());
            (n.iter().any(|n| n.start == 0 && n.pitch == 65) && !c.pr.pen && c.pr.selected.is_empty()).then_some(()).ok_or(format!("{n:?}"))
        })),
        Do("click Double", Box::new(|a| click(a, 260.0 + 270.5 + 10.0, 466.0))),
        Check("Double: 2x notes and 2x clip length in ONE undo step", Box::new(move |c, _| {
            let (before, len) = sv2.borrow().clone();
            let m = instrument_clip(c).ok_or("clip gone")?;
            one_step(h7.clone(), "Double")(c)?;
            (m.notes.len() == 2 * before.len() && m.length == 2 * len).then_some(()).ok_or(format!("{} notes, len {}", m.notes.len(), m.length))
        })),
        Do("Cmd+Z", Box::new(|a| key(a, "z", true, false))),
        Check("undo restored the notes and the clip length exactly", Box::new(move |c, _| {
            let (before, len) = saved.borrow().clone();
            let m = instrument_clip(c).ok_or("clip gone")?;
            h15.set(c.doc.history_labels().len());
            (m.notes == before && m.length == len).then_some(()).ok_or(format!("{} notes len {}", m.notes.len(), m.length))
        })),
        DoC("select the clip in the lane, Cmd+D", Box::new(move |a, ctl| {
            let (x, y) = lane_xy(ctl, 2.0, 1);
            click(a, x, y);
            key(a, "d", true, false);
        })),
        Check("⌘D duplicated the clip right after itself (same notes), copy selected", Box::new(move |c, _| {
            let t = c.project().tracks.iter().find(|t| t.kind == TrackKind::Instrument).ok_or("no inst")?;
            one_step(h8.clone(), "Duplicate Clip")(c)?;
            let ok = t.midi_clips.len() == 2 && t.midi_clips[1].start == t.midi_clips[0].end() && t.midi_clips[1].notes == t.midi_clips[0].notes && c.selected_clip == Some(t.midi_clips[1].id);
            ok.then_some(()).ok_or(format!("{} clips", t.midi_clips.len()))
        })),
        Do("Cmd+Z", Box::new(|a| key(a, "z", true, false))),
        DoC("right-click the clip", Box::new(move |a, ctl| {
            let (x, y) = lane_xy(ctl, 2.0, 1);
            let w = a.window();
            let p = LogicalPosition::new(x, y);
            w.dispatch_event(WindowEvent::PointerMoved { position: p });
            w.dispatch_event(WindowEvent::PointerPressed { position: p, button: PointerEventButton::Right });
            w.dispatch_event(WindowEvent::PointerReleased { position: p, button: PointerEventButton::Right });
        })),
        Check("context menu open with Cut/Copy/Duplicate enabled and Split disabled", Box::new(|c, a| {
            let s = a.global::<Studio>();
            let items = s.get_menu_items();
            let get = |i: usize| slint::Model::row_data(&items, i);
            let ok = s.get_menu_open()
                && get(0).is_some_and(|m| m.label == "Cut" && m.enabled)
                && get(2).is_some_and(|m| m.label == "Duplicate" && m.enabled)
                && get(3).is_some_and(|m| m.label == "Split" && !m.enabled)
                && instrument_clip(c).map(|m| m.id) == c.selected_clip;
            ok.then_some(()).ok_or(format!("open {} items {}", s.get_menu_open(), slint::Model::row_count(&items)))
        })),
        Do("Escape closes the menu, Delete", Box::new(|a| {
            key(a, Key::Escape, false, false);
            key(a, Key::Delete, false, false);
        })),
        Check("menu closed, clip deleted (RemoveMidiClip), piano roll closed", Box::new(move |c, a| {
            let s = a.global::<Studio>();
            (!s.get_menu_open() && instrument_clip(c).is_none() && !s.get_pr_visible()).then_some(()).ok_or(format!("menu {} clip {:?}", s.get_menu_open(), instrument_clip(c).map(|m| m.id)))
        })),
        Do("Cmd+Z", Box::new(|a| key(a, "z", true, false))),
        Check("undo restored the clip with its notes", Box::new(move |c, _| {
            let _ = (&h9, &h10, &h11, &h12);
            let n = notes_of(c);
            (instrument_clip(c).is_some() && n.len() == 4).then_some(()).ok_or(format!("{} notes", n.len()))
        })),
    ]
}

/// Synth device card origin in the window (bottom panel 306 pt at y 509; card at panel 263.5, 34).
const SYNTH: (f32, f32) = (5.0 + 263.5, 509.0 + 34.0);

fn synth_xy(cx: f32, cy: f32) -> (f32, f32) {
    (SYNTH.0 + cx, SYNTH.1 + cy)
}

/// Centre of category row `i` in the device's left list.
fn synth_category(i: usize) -> (f32, f32) {
    synth_xy(60.0, 31.85 + i as f32 * 25.3 + 12.65)
}

fn double_click(app: &AppWindow, x: f32, y: f32) {
    click(app, x, y);
    click(app, x, y);
}

/// Centre of row `i` of the open synth popup list (same clamping as ui/app.slint).
fn synth_menu_row(app: &AppWindow, i: usize) -> (f32, f32) {
    let s = app.global::<Studio>();
    let (w, h) = (s.get_synth_menu_w(), s.get_synth_menu_h());
    let x = s.get_synth_menu_x().min(1440.0 - 5.0 - w);
    let y = s.get_synth_menu_y().min(820.0 - 5.0 - h).max(5.0);
    (x + 30.0, y + crate::synth::MENU_PAD + i as f32 * crate::synth::MENU_ROW + crate::synth::MENU_ROW / 2.0)
}

fn inst_synth(c: &Controller) -> Option<saine_core::SynthParams> {
    c.project().tracks.iter().find(|t| t.kind == TrackKind::Instrument).map(|t| t.synth.clone())
}

/// Synth device (Devices tab on the Instrument track). Leaves the synth back at its
/// starting parameters (everything undone).
fn synth_steps() -> Vec<Step> {
    let bright_low = Rc::new(Cell::new(-1f32));
    let bright_low2 = bright_low.clone();
    use Step::*;
    use saine_core::instrument::{Waveform, factory_presets};
    #[derive(Default)]
    struct Rec {
        hist: usize,
        start: Option<saine_core::SynthParams>,
        before: Option<saine_core::SynthParams>,
        path: String,
    }
    let rec = Rc::new(RefCell::new(Rec::default()));
    let r = |n: usize| -> Vec<Rc<RefCell<Rec>>> { (0..n).map(|_| rec.clone()).collect() };
    let rs = r(14);
    let [r0, r1, r2, r3, r4, r5, r6, r7, r8, r9, r10, r11, r12, r13] = <[Rc<RefCell<Rec>>; 14]>::try_from(rs).ok().unwrap();
    // One new undo step labelled "Synth Parameter Change" since the last record; re-records.
    fn one_step(rc: &Rc<RefCell<Rec>>, c: &Controller) -> Result<(), String> {
        let n = c.doc.history_labels().len();
        let was = rc.borrow().hist;
        rc.borrow_mut().hist = n;
        (n == was + 1 && c.doc.undo_label() == Some("Synth Parameter Change"))
            .then_some(())
            .ok_or(format!("history {was} -> {n}, last {:?}", c.doc.undo_label()))
    }
    vec![
        DoC("click the Instrument track header", Box::new(|a, ctl| {
            let sy = ctl.borrow().view.scroll_y as f32;
            click(a, 300.0, TRACK0_Y + PITCH + 10.0 - sy);
        })),
        Check("synth: Instrument track selected -> Devices tab shows the Synth (Oscillators page)", Box::new(move |c, a| {
            let s = a.global::<Studio>();
            let mut r = r0.borrow_mut();
            r.hist = c.doc.history_labels().len();
            r.start = inst_synth(c);
            let ok = s.get_synth_visible() && s.get_bottom_tab() == 0 && s.get_synth_page() == 0 && c.synth_track().is_some() && s.get_synth_preset() == r.start.as_ref().map(|p| p.preset.clone()).unwrap_or_default();
            ok.then_some(()).ok_or(format!("visible {} tab {} page {}", s.get_synth_visible(), s.get_bottom_tab(), s.get_synth_page()))
        })),
        Do("click preset ›", Box::new(|a| {
            let (x, y) = synth_xy(726.5, 13.0);
            click(a, x, y);
        })),
        Check("synth: preset › loaded factory_presets()[1] in one undo step", Box::new(move |c, a| {
            one_step(&r1, c)?;
            let p = inst_synth(c).ok_or("no instrument")?;
            (p == factory_presets()[1] && a.global::<Studio>().get_synth_preset() == p.preset.as_str()).then_some(()).ok_or(format!("preset {}", p.preset))
        })),
        Do("click the preset name, pick the 4th preset in the list", Box::new(|a| {
            let (x, y) = synth_xy(690.0, 13.0);
            click(a, x, y);
            let (x, y) = synth_menu_row(a, 3);
            click(a, x, y);
        })),
        Check("synth: preset list loaded factory_presets()[3] in one undo step, list closed", Box::new(move |c, a| {
            one_step(&r2, c)?;
            let p = inst_synth(c).ok_or("no instrument")?;
            (p == factory_presets()[3] && !a.global::<Studio>().get_synth_menu_open()).then_some(()).ok_or(format!("preset {}", p.preset))
        })),
        Do("Cmd+Z, then the Filter page", Box::new(|a| {
            key(a, "z", true, false);
            let (x, y) = synth_category(2);
            click(a, x, y);
        })),
        Check("synth: undo back to preset [1]; Filter page shown", Box::new(move |c, a| {
            let mut r = r3.borrow_mut();
            r.hist = c.doc.history_labels().len();
            r.before = inst_synth(c);
            (r.before.as_ref() == Some(&factory_presets()[1]) && a.global::<Studio>().get_synth_page() == 2).then_some(()).ok_or(format!("page {}", a.global::<Studio>().get_synth_page()))
        })),
        Do("drag the Cutoff knob down 60 pt", Box::new(|a| {
            let (x, y) = synth_xy(152.5, 212.0);
            drag(a, x, y, x, y + 60.0);
        })),
        Check("synth: cutoff decreased (log taper) with exactly one undo step; readout in Hz", Box::new(move |c, a| {
            one_step(&r4, c)?;
            let before = r4.borrow().before.clone().ok_or("no record")?;
            let p = inst_synth(c).ok_or("no instrument")?;
            let want = crate::synth::from_norm(crate::synth::p::CUTOFF, crate::synth::to_norm(crate::synth::p::CUTOFF, before.filter.cutoff_hz) - 0.4);
            let text = slint::Model::row_data(&a.global::<Studio>().get_synth_params(), crate::synth::p::CUTOFF).map(|p| p.text.to_string()).unwrap_or_default();
            let mut rest = p.clone();
            rest.filter.cutoff_hz = before.filter.cutoff_hz;
            (p.filter.cutoff_hz < before.filter.cutoff_hz && (p.filter.cutoff_hz - want).abs() < want * 0.03 && rest == before && text.ends_with("Hz"))
                .then_some(())
                .ok_or(format!("cutoff {} -> {} (want ≈{want}), readout {text:?}", before.filter.cutoff_hz, p.filter.cutoff_hz))
        })),
        Check("audio: rendered output with the lowered cutoff (measured for the next check)", Box::new(move |c, _| {
            if let Some(b) = rendered_brightness(c)? {
                bright_low.set(b);
            }
            Ok(())
        })),
        Do("double-click the Cutoff knob", Box::new(|a| {
            let (x, y) = synth_xy(152.5, 212.0);
            double_click(a, x, y);
        })),
        Check("synth: double-click reset cutoff to the default (2093 Hz) in one undo step", Box::new(move |c, _| {
            one_step(&r5, c)?;
            let p = inst_synth(c).ok_or("no instrument")?;
            (p.filter.cutoff_hz == saine_core::SynthParams::default().filter.cutoff_hz).then_some(()).ok_or(format!("cutoff {}", p.filter.cutoff_hz))
        })),
        Check("audio: panel cutoff reaches the engine (brighter after reset than with the lowered cutoff)", Box::new(move |c, _| {
            let Some(reset) = rendered_brightness(c)? else { return Ok(()) };
            let low = bright_low2.get();
            (low >= 0.0 && reset > low * 1.15).then_some(()).ok_or(format!("brightness low {low} vs reset {reset}"))
        })),
        Do("Oscillators page, open the Osc 2 wave dropdown", Box::new(|a| {
            let (x, y) = synth_category(0);
            click(a, x, y);
            let (x, y) = synth_xy(117.5 + 318.0 + 45.0, 42.0);
            click(a, x, y);
        })),
        Check("synth: wave list open with Saw/Square/Triangle/Sine; record the Osc 2 display", Box::new(move |c, a| {
            let s = a.global::<Studio>();
            let items: Vec<String> = (0..slint::Model::row_count(&s.get_synth_menu_items())).filter_map(|i| slint::Model::row_data(&s.get_synth_menu_items(), i)).map(|m| m.label.to_string()).collect();
            r6.borrow_mut().path = slint::Model::row_data(&s.get_synth_displays(), 1).map(|d| d.line.to_string()).unwrap_or_default();
            r6.borrow_mut().hist = c.doc.history_labels().len();
            (s.get_synth_page() == 0 && s.get_synth_menu_open() && items == ["Saw", "Square", "Triangle", "Sine"]).then_some(()).ok_or(format!("page {} open {} {items:?}", s.get_synth_page(), s.get_synth_menu_open()))
        })),
        Do("pick Square", Box::new(|a| {
            let (x, y) = synth_menu_row(a, 1);
            click(a, x, y);
        })),
        Check("synth: Osc 2 wave = Square (one undo step), dropdown text and display path updated", Box::new(move |c, a| {
            one_step(&r7, c)?;
            let s = a.global::<Studio>();
            let p = inst_synth(c).ok_or("no instrument")?;
            let line = slint::Model::row_data(&s.get_synth_displays(), 1).map(|d| d.line.to_string()).unwrap_or_default();
            let text = slint::Model::row_data(&s.get_synth_choices(), crate::synth::ch::OSC2_WAVE).unwrap_or_default();
            let changed = line != r7.borrow().path;
            r7.borrow_mut().path = line;
            (p.osc[1].wave == Waveform::Square && text == "Square" && changed && !s.get_synth_menu_open()).then_some(()).ok_or(format!("wave {:?} text {text} path changed {changed}", p.osc[1].wave))
        })),
        Do("Osc 2 wave dropdown -> Saw", Box::new(|a| {
            let (x, y) = synth_xy(117.5 + 318.0 + 45.0, 42.0);
            click(a, x, y);
            let (x, y) = synth_menu_row(a, 0);
            click(a, x, y);
        })),
        Check("synth: Osc 2 wave = Saw; Saw and Square display paths differ", Box::new(move |c, a| {
            one_step(&r8, c)?;
            let p = inst_synth(c).ok_or("no instrument")?;
            let line = slint::Model::row_data(&a.global::<Studio>().get_synth_displays(), 1).map(|d| d.line.to_string()).unwrap_or_default();
            (p.osc[1].wave == Waveform::Saw && line != r8.borrow().path && !line.is_empty()).then_some(()).ok_or(format!("wave {:?}", p.osc[1].wave))
        })),
        Do("drag the Osc 1 Level fader up 20 pt", Box::new(|a| {
            let (x, y) = synth_xy(117.5 + 254.75 + 20.0, 61.5 + 45.0);
            drag(a, x, y, x, y - 20.0);
        })),
        Check("synth: Osc 1 level +20/91 in one undo step, readout in %", Box::new(move |c, a| {
            let before = factory_presets()[1].osc[0].level; // the preset loaded by ›
            one_step(&r9, c)?;
            let p = inst_synth(c).ok_or("no instrument")?;
            let text = slint::Model::row_data(&a.global::<Studio>().get_synth_params(), crate::synth::p::OSC1_LEVEL).map(|p| p.text.to_string()).unwrap_or_default();
            (approx(p.osc[0].level, before + 20.0 / 91.0, 0.01) && text == format!("{:.0}%", p.osc[0].level * 100.0)).then_some(()).ok_or(format!("level {} text {text}", p.osc[0].level))
        })),
        Do("Envelopes page", Box::new(|a| {
            let (x, y) = synth_category(3);
            click(a, x, y);
        })),
        Check("synth: Envelopes page; record params", Box::new(move |c, a| {
            r10.borrow_mut().before = inst_synth(c);
            r10.borrow_mut().hist = c.doc.history_labels().len();
            (a.global::<Studio>().get_synth_page() == 3).then_some(()).ok_or("page".into())
        })),
        Do("drag the Amp Attack knob up 45 pt", Box::new(|a| {
            let (x, y) = synth_xy(152.5, 212.0);
            drag(a, x, y, x, y - 45.0);
        })),
        Check("synth: amp attack increased (one undo step), filter envelope unchanged, readout in ms/s", Box::new(move |c, a| {
            one_step(&r11, c)?;
            let before = r11.borrow().before.clone().ok_or("no record")?;
            let p = inst_synth(c).ok_or("no instrument")?;
            let text = slint::Model::row_data(&a.global::<Studio>().get_synth_params(), crate::synth::p::AMP_A).map(|p| p.text.to_string()).unwrap_or_default();
            (p.amp_env.attack > before.amp_env.attack * 2.0 && p.filter_env == before.filter_env && (text.ends_with(" ms") || text.ends_with(" s")))
                .then_some(())
                .ok_or(format!("attack {} -> {} {text}", before.amp_env.attack, p.amp_env.attack))
        })),
        Do("Cmd+Z", Box::new(|a| key(a, "z", true, false))),
        Check("synth: ⌘Z restored the previous parameters exactly", Box::new(move |c, _| {
            let before = r12.borrow().before.clone().ok_or("no record")?;
            (inst_synth(c).as_ref() == Some(&before)).then_some(()).ok_or("params differ after undo".into())
        })),
        Do("Cmd+Z x6 (level, Saw, Square, reset, cutoff drag, preset ›)", Box::new(|a| {
            for _ in 0..6 {
                key(a, "z", true, false);
            }
        })),
        Check("synth: every synth edit undone -> starting parameters exactly", Box::new(move |c, _| {
            let start = r13.borrow().start.clone().ok_or("no record")?;
            (inst_synth(c).as_ref() == Some(&start)).then_some(()).ok_or(format!("preset {:?}", inst_synth(c).map(|p| p.preset)))
        })),
    ]
}

/// Render ~100 ms offline of a test note (C3) on the selected instrument
/// track, using the controller's current session (so the synth parameters are
/// exactly what the Devices panel set), then restore the real session.
/// Returns a brightness measure: RMS of the first difference over RMS of the
/// signal (higher = more high-frequency energy).
fn rendered_brightness(c: &Controller) -> Result<Option<f32>, String> {
    if c.engine.device_running() {
        return Ok(None); // offline render is refused while the device owns the audio path
    }
    let id = c.selected_track.ok_or("no selected track")?;
    let mut snap = c.current_snapshot();
    for t in &mut snap.tracks {
        if t.track_id == id.0 {
            t.notes = vec![saine_core::snapshot::NoteSnapshot { start: 0, end: 48_000, pitch: 48, velocity: 100 }];
        } else {
            t.mute = true; // isolate the instrument under test
        }
    }
    let e = &c.engine;
    e.publish(&snap).map_err(|e| e.to_string())?;
    e.seek(0).map_err(|e| e.to_string())?;
    e.play().map_err(|e| e.to_string())?;
    let mut buf = vec![0f32; 2 * 4_800];
    let r = e.render_offline(&mut buf);
    let _ = e.stop();
    e.publish(&c.current_snapshot()).map_err(|e| e.to_string())?;
    r.map_err(|e| e.to_string())?;
    let l: Vec<f32> = buf.iter().step_by(2).copied().collect();
    let rms = |v: &mut dyn Iterator<Item = f32>| {
        let (s, n) = v.fold((0f64, 0usize), |(s, n), x| (s + (x as f64).powi(2), n + 1));
        (s / n.max(1) as f64).sqrt() as f32
    };
    let level = rms(&mut l.iter().copied());
    if level < 1e-4 {
        return Err(format!("render is silent (rms {level})"));
    }
    Ok(Some(rms(&mut l.windows(2).map(|w| w[1] - w[0])) / level))
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

/// Window centre of a track header's R button (track index `i`, current scroll).
fn header_r(ctl: &Ctl, i: usize) -> (f32, f32) {
    let sy = ctl.borrow().view.scroll_y as f32;
    (262.0 + 60.5, TRACK0_Y + i as f32 * PITCH + 47.0 - sy)
}

/// Inspector (bottom panel 306 pt at y 509 with the Devices tab): R, I and the In field.
const INSP_R: (f32, f32) = (5.0 + 86.0 + 10.0, 509.0 + 275.0 + 9.0);
const INSP_I: (f32, f32) = (5.0 + 108.0 + 10.0, 509.0 + 275.0 + 9.0);
const INSP_IN: (f32, f32) = (5.0 + 80.0, 509.0 + 37.0 + 9.5);
/// "In" popup (window x 16, y 568): centre of row 0.
const IN_ROW0: (f32, f32) = (60.0, 568.0 + crate::recording::IN_PAD + crate::recording::IN_ROW / 2.0);
const RECORD_BTN: (f32, f32) = (423.0 + 14.0, 38.0 + 14.0);
/// Settings panel at (255, 7): sidebar row centres.
fn settings_row(i: usize) -> (f32, f32) {
    (255.0 + 10.0 + 83.0, 7.0 + 94.75 + i as f32 * 27.25 + 12.75)
}

/// Injected test input: 1 s of a 440 Hz sine at -6 dBFS, mono.
const INJ_FREQ: f32 = 440.0;
const INJ_AMP: f32 = 0.5;

/// Arm / monitor / input / record transport / settings (appended last). Never opens a
/// real input device: the In popup lists only the configured input device (none in the
/// self-test), and recording uses `Engine::render_offline_with_input` (offline run only).
fn recording_steps() -> Vec<Step> {
    use Step::*;
    let hist = Rc::new(Cell::new(0usize));
    let rec_n = Rc::new(Cell::new(0usize)); // audio track 1 clip count before recording
    let h = || hist.clone();
    // One new undo step with this label since the last record; re-records.
    fn one(hc: &Rc<Cell<usize>>, c: &Controller, label: &str) -> Result<(), String> {
        let n = c.doc.history_labels().len();
        let was = hc.get();
        hc.set(n);
        (n == was + 1 && c.doc.undo_label() == Some(label)).then_some(()).ok_or(format!("history {was} -> {n}, last {:?} (want {label})", c.doc.undo_label()))
    }
    let (h1, h2, h3, h4, h5, h6, h7, h8, h9, h10, h11) = (h(), h(), h(), h(), h(), h(), h(), h(), h(), h(), h());
    let (h12, h13, h14) = (h(), h(), h());
    let (rn1, rn2, rn3) = (rec_n.clone(), rec_n.clone(), rec_n.clone());
    let pre_hist = Rc::new(Cell::new(0usize)); // history length before the recording pass
    let (ph1, ph2) = (pre_hist.clone(), pre_hist.clone());
    fn status(a: &AppWindow) -> String {
        a.global::<Studio>().get_status_text().to_string()
    }
    fn offline(c: &Controller) -> bool {
        !c.engine.device_running()
    }
    vec![
        Do("Enter; Escape (close menus)", Box::new(|a| {
            key(a, Key::Return, false, false);
            key(a, Key::Escape, false, false);
        })),
        DoC("rec: select audio track 1 (header), Devices tab", Box::new(|a, ctl| {
            let sy = ctl.borrow().view.scroll_y as f32;
            click(a, 300.0, TRACK0_Y + 10.0 - sy);
            if ctl.borrow().bottom_tab == crate::controller::BottomTab::Clip {
                click(a, 5.0 + 286.0 + 15.0, 820.0 - 5.0 - 390.0 + 14.0);
            }
        })),
        Check("rec: nothing armed with an input; record history", Box::new(move |c, a| {
            h1.set(c.doc.history_labels().len());
            let t = c.project().tracks.first().ok_or("no tracks")?;
            (c.armed_with_input().is_empty() && c.selected_track == Some(t.id) && t.kind == TrackKind::Audio && a.global::<Studio>().get_bottom_tab() == 0)
                .then_some(())
                .ok_or(format!("armed {:?} sel {:?}", c.armed_with_input(), c.selected_track))
        })),
        Do("rec: click the header record button with nothing armed", Box::new(|a| click(a, RECORD_BTN.0, RECORD_BTN.1))),
        Check("rec: record refused with a clear status, not recording, project unchanged", Box::new(move |c, a| {
            let st = status(a);
            (!c.rec.recording && !a.global::<Studio>().get_recording() && st.starts_with("Nothing to record") && c.doc.history_labels().len() == h2.get())
                .then_some(())
                .ok_or(format!("recording {} status {st:?}", c.rec.recording))
        })),
        DoC("rec: click R on track 1's header", Box::new(|a, ctl| {
            let (x, y) = header_r(ctl, 0);
            click(a, x, y);
        })),
        Check("rec: track 1 armed (SetTrackArm), one undo step, header + inspector R on", Box::new(move |c, a| {
            one(&h3, c, "Arm Track")?;
            let s = a.global::<Studio>();
            let row = slint::Model::row_data(&s.get_tracks(), 0).ok_or("no row")?;
            (c.project().tracks[0].arm && row.arm && s.get_sel_arm()).then_some(()).ok_or("not armed".into())
        })),
        DoC("rec: click R on track 1's header again", Box::new(|a, ctl| {
            let (x, y) = header_r(ctl, 0);
            click(a, x, y);
        })),
        Check("rec: track 1 disarmed, one undo step", Box::new(move |c, _| {
            one(&h4, c, "Disarm Track")?;
            (!c.project().tracks[0].arm).then_some(()).ok_or("still armed".into())
        })),
        Do("rec: Shift+R (track.arm)", Box::new(|a| key(a, "R", false, true))),
        Check("rec: ⇧R armed the selected track, one undo step", Box::new(move |c, _| {
            one(&h5, c, "Arm Track")?;
            c.project().tracks[0].arm.then_some(()).ok_or("not armed".into())
        })),
        Do("rec: click the inspector R", Box::new(|a| click(a, INSP_R.0, INSP_R.1))),
        Check("rec: inspector R disarmed, one undo step", Box::new(move |c, a| {
            one(&h6, c, "Disarm Track")?;
            (!c.project().tracks[0].arm && !a.global::<Studio>().get_sel_arm()).then_some(()).ok_or("still armed".into())
        })),
        Do("rec: click the inspector I", Box::new(|a| click(a, INSP_I.0, INSP_I.1))),
        Check("rec: I turned input monitoring on (SetTrackMonitor), one undo step, I drawn on", Box::new(move |c, a| {
            one(&h7, c, "Input Monitoring Change")?;
            (c.project().tracks[0].monitor && a.global::<Studio>().get_sel_monitor()).then_some(()).ok_or("monitor off".into())
        })),
        Do("rec: Shift+I (track.monitor)", Box::new(|a| key(a, "I", false, true))),
        Check("rec: ⇧I turned monitoring off, one undo step", Box::new(move |c, _| {
            one(&h8, c, "Input Monitoring Change")?;
            (!c.project().tracks[0].monitor).then_some(()).ok_or("monitor on".into())
        })),
        DoC("rec: click R on the Instrument track's header (placeholder)", Box::new(|a, ctl| {
            let i = ctl.borrow().project().tracks.iter().position(|t| t.kind == TrackKind::Instrument).unwrap_or(1);
            let (x, y) = header_r(ctl, i);
            click(a, x, y);
        })),
        Check("rec: instrument R reports MIDI recording not implemented, no undo step", Box::new(move |c, a| {
            let st = status(a);
            (st.starts_with("MIDI recording is not implemented") && c.doc.history_labels().len() == h9.get()).then_some(()).ok_or(format!("status {st:?}"))
        })),
        DoC("rec: re-select track 1, click the inspector In field", Box::new(|a, ctl| {
            let sy = ctl.borrow().view.scroll_y as f32;
            click(a, 300.0, TRACK0_Y + 10.0 - sy);
            click(a, INSP_IN.0, INSP_IN.1);
        })),
        Check("rec: In popup open, \"None\" first and checked, device inputs listed (or the no-device hint)", Box::new(|c, a| {
            let s = a.global::<Studio>();
            let items = s.get_input_menu_items();
            let rows: Vec<_> = (0..slint::Model::row_count(&items)).filter_map(|i| slint::Model::row_data(&items, i)).collect();
            let first = rows.first().ok_or("empty popup")?;
            let want = match c.current_input_device() {
                Ok(Some(d)) => crate::recording::input_choices(Some(&d)).len(),
                _ => 1,
            };
            let entries = rows.iter().filter(|r| !r.hint).count();
            let hint_ok = entries > 1 || rows.iter().any(|r| r.hint);
            (s.get_input_menu_open() && first.label == "None" && first.checked && !first.hint && entries == want && hint_ok)
                .then_some(())
                .ok_or(format!("open {} rows {:?}", s.get_input_menu_open(), rows.iter().map(|r| r.label.to_string()).collect::<Vec<_>>()))
        })),
        Do("rec: click \"None\" in the popup", Box::new(|a| click(a, IN_ROW0.0, IN_ROW0.1))),
        Check("rec: popup closed; choosing the current entry made no undo step", Box::new(move |c, a| {
            (!a.global::<Studio>().get_input_menu_open() && c.doc.history_labels().len() == h10.get() && c.project().tracks[0].input.is_none())
                .then_some(())
                .ok_or("popup open or project changed".into())
        })),
        DoC("rec: pick a device input in the popup, or (COMMAND PATH: no device inputs are listed in the self-test) SetTrackInput Input 1 (Mono)", Box::new(|a, ctl| {
            click(a, INSP_IN.0, INSP_IN.1);
            let real = ctl.borrow().rec.input_menu.as_ref().is_some_and(|m| m.len() > 1);
            if real {
                click(a, IN_ROW0.0, IN_ROW0.1 + crate::recording::IN_ROW);
            } else {
                key(a, Key::Escape, false, false);
                let mut c = ctl.borrow_mut();
                let t = c.project().tracks[0].id;
                c.set_track_input(t, Some(saine_core::InputSource { channel: 0, stereo: false }));
            }
        })),
        Check("rec: track 1 input = Input 1 (Mono) (SetTrackInput), one undo step, In field shows it", Box::new(move |c, a| {
            one(&h11, c, "Input Change")?;
            let s = a.global::<Studio>();
            (c.project().tracks[0].input == Some(saine_core::InputSource { channel: 0, stereo: false }) && s.get_sel_input_text() == "Input 1 (Mono)" && !s.get_input_menu_open())
                .then_some(())
                .ok_or(format!("input {:?} text {}", c.project().tracks[0].input, s.get_sel_input_text()))
        })),
        // ---------------- settings ----------------
        Do("settings: Cmd+, (app.settings)", Box::new(|a| key(a, ",", true, false))),
        Check("settings: open on Audio with output/input/mute/sample-rate/buffer controls; 9 sidebar pages", Box::new(|c, a| {
            let s = a.global::<Studio>();
            let pages: Vec<String> = (0..slint::Model::row_count(&s.get_settings_pages())).filter_map(|i| slint::Model::row_data(&s.get_settings_pages(), i)).map(|p| p.to_string()).collect();
            let ok = c.settings.open
                && s.get_settings_open()
                && s.get_settings_page() == 1
                && pages == crate::settings::PAGES
                && !s.get_set_output_text().is_empty()
                && s.get_set_input_text() == "None"
                && s.get_set_rate_text().ends_with("kHz")
                && !s.get_set_buffer_text().is_empty()
                && !s.get_set_input_muted();
            ok.then_some(()).ok_or(format!("open {} page {} out {:?} in {:?} rate {:?}", s.get_settings_open(), s.get_settings_page(), s.get_set_output_text(), s.get_set_input_text(), s.get_set_rate_text()))
        })),
        Do("settings: click the Buffer size dropdown", Box::new(|a| click(a, 255.0 + 187.0 + 300.0, 7.0 + 312.0 + 14.0))),
        Check("settings: buffer list 64..2048 with ms readouts", Box::new(|_, a| {
            let s = a.global::<Studio>();
            let items = s.get_settings_menu_items();
            let labels: Vec<String> = (0..slint::Model::row_count(&items)).filter_map(|i| slint::Model::row_data(&items, i)).map(|r| r.label.to_string()).collect();
            (s.get_settings_menu_open() && labels.len() == 6 && labels[0].starts_with("64 samples (") && labels[5].starts_with("2048 samples (") && labels.iter().all(|l| l.ends_with(" ms)")))
                .then_some(())
                .ok_or(format!("{labels:?}"))
        })),
        Do("settings: Escape (closes the list), press R (blocked while Settings is open), click General", Box::new(|a| {
            key(a, Key::Escape, false, false);
            key(a, "r", false, false);
            let (x, y) = settings_row(0);
            click(a, x, y);
        })),
        Check("settings: list closed, R did nothing, General is a placeholder page", Box::new(move |c, a| {
            let s = a.global::<Studio>();
            (s.get_settings_open() && !s.get_settings_menu_open() && s.get_settings_page() == 0 && !c.rec.recording && c.doc.history_labels().len() == h12.get())
                .then_some(())
                .ok_or(format!("open {} menu {} page {}", s.get_settings_open(), s.get_settings_menu_open(), s.get_settings_page()))
        })),
        Do("settings: click Audio, then Escape", Box::new(|a| {
            let (x, y) = settings_row(1);
            click(a, x, y);
            key(a, Key::Escape, false, false);
        })),
        Check("settings: Escape closed the window", Box::new(|c, a| {
            (!c.settings.open && !a.global::<Studio>().get_settings_open()).then_some(()).ok_or("still open".into())
        })),
        // ---------------- recording with injected input (offline only) ----------------
        DoC("rec: Shift+R (arm track 1), click the ruler at bar 2", Box::new(|a, ctl| {
            key(a, "R", false, true);
            let x = LANE_X + ctl.borrow().view.x_of(4.0);
            click(a, x, 112.0);
        })),
        Check("rec: track 1 armed with Input 1, playhead at beat 4; record history", Box::new(move |c, _| {
            rn1.set(c.project().tracks[0].clips.len());
            h13.set(c.doc.history_labels().len());
            ph1.set(c.doc.history_labels().len());
            let at = c.playhead_ticks();
            (c.armed_with_input() == vec![c.project().tracks[0].id] && (at - 4 * PPQ).abs() <= 1).then_some(()).ok_or(format!("armed {:?} playhead {at}", c.armed_with_input()))
        })),
        Do("rec: press R (transport.record)", Box::new(|a| key(a, "r", false, false))),
        Check("rec: recording started (engine.start_recording), record button red [needs the engine recording API]", Box::new(move |c, a| {
            if !offline(c) {
                return Ok(()); // --self-test-audio: injected recording runs offline only
            }
            let st = c.engine.recording_state();
            (c.rec.recording && a.global::<Studio>().get_recording() && st.active).then_some(()).ok_or(format!("not recording: {:?} (engine active {})", status(a), st.active))
        })),
        Check("rec: rendered 1 s of injected 440 Hz input (render_offline_with_input) [needs the engine recording API]", Box::new(|c, _| {
            if !offline(c) || !c.rec.recording {
                return if offline(c) { Err("not recording".into()) } else { Ok(()) };
            }
            let n = 48_000usize;
            let input: Vec<f32> = (0..n).map(|i| INJ_AMP * (std::f32::consts::TAU * INJ_FREQ * i as f32 / 48_000.0).sin()).collect();
            let mut out = vec![0f32; 2 * n];
            c.engine.render_offline_with_input(&input, 1, &mut out).map_err(|e| e.to_string())
        })),
        DoC("rec: rebuild the live timeline clips", Box::new(|_, ctl| ctl.borrow_mut().rebuild_vis())),
        Check("rec: live clip on track 1 from recording_state(), ~1 s long, live peaks drawn [needs the engine recording API]", Box::new(|c, _| {
            if !offline(c) {
                return Ok(());
            }
            let v = c.rec.live_vis.first().ok_or("no live clip")?;
            let secs = v.len_beats * 60.0 / c.project().transport.bpm;
            let peak = v.live.as_ref().map(|(p, _)| p.iter().fold(0f32, |m, x| m.max(x[1]))).unwrap_or(0.0);
            (c.rec.live_vis.len() == 1 && v.track_index == 0 && (secs - 1.0).abs() < 0.05 && (peak - INJ_AMP).abs() < 0.05)
                .then_some(())
                .ok_or(format!("{} live clips, {secs:.3} s, peak {peak}", c.rec.live_vis.len()))
        })),
        Do("rec: press R (stop; the take loads on a worker)", Box::new(|a| key(a, "r", false, false))),
        Check("rec: one \"Record\" undo step added a clip at beat 4 on track 1 [needs the engine recording API]", Box::new(move |c, _| {
            if !offline(c) {
                return Ok(());
            }
            one(&h14, c, "Record")?;
            let t = &c.project().tracks[0];
            let clip = t.clips.iter().max_by_key(|cl| cl.id.0).ok_or("no clip")?;
            // Offline the engine reports no device latency, so the take starts at the playhead (±1 ms).
            (t.clips.len() == rn2.get() + 1 && (clip.start - 4 * PPQ).abs() <= 2)
                .then_some(())
                .ok_or(format!("{} clips (was {}), newest at {} ticks", t.clips.len(), rn2.get(), clip.start))
        })),
        Check("rec: the take decodes to the injected signal (440 Hz, -6 dBFS, 1 s) [needs the engine recording API]", Box::new(|c, _| {
            if !offline(c) {
                return Ok(());
            }
            let dir = std::fs::canonicalize(c.recording_dir()).unwrap_or_else(|_| c.recording_dir());
            let p = c.project();
            let (clip, asset) = p.tracks[0]
                .clips
                .iter()
                .filter_map(|cl| p.asset(cl.asset).map(|a| (cl, a)))
                .find(|(_, a)| std::path::Path::new(&a.absolute_path).starts_with(&dir))
                .ok_or(format!("no recorded take on track 1 (nothing under {})", dir.display()))?;
            let (rate, ch, s) = crate::wavgen::read_wav(std::path::Path::new(&asset.absolute_path))?;
            let mono: Vec<f32> = s.chunks(ch as usize).map(|f| f[0]).skip(clip.source_offset as usize).collect();
            let secs = mono.len() as f32 / rate as f32;
            let rms = (mono.iter().map(|x| x * x).sum::<f32>() / mono.len().max(1) as f32).sqrt();
            let crossings = mono.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count() as f32 / secs.max(1e-6);
            let peak = c.engine.peaks(c.asset_map.get(&asset.id).copied().ok_or("asset not loaded")?, 0, 4800, 10).map_err(|e| e.to_string())?.iter().fold(0f32, |m, p| m.max(p[1]));
            ((secs - 1.0).abs() < 0.02 && (rms - INJ_AMP / 2f32.sqrt()).abs() < 0.02 && (crossings - INJ_FREQ).abs() < 3.0 && (peak - INJ_AMP).abs() < 0.02)
                .then_some(())
                .ok_or(format!("{secs:.3} s, rms {rms:.4}, {crossings:.1} Hz, engine peak {peak:.3}"))
        })),
        Do("rec: Cmd+Z, Enter", Box::new(|a| {
            key(a, "z", true, false);
            key(a, Key::Return, false, false);
        })),
        Check("rec: ⌘Z removed the recorded clip and its asset [needs the engine recording API]", Box::new(move |c, _| {
            if !offline(c) {
                return Ok(());
            }
            let t = &c.project().tracks[0];
            (t.clips.len() == rn3.get() && c.doc.history_labels().len() == ph2.get() && c.doc.undo_label() != Some("Record"))
                .then_some(())
                .ok_or(format!("{} clips, undo {:?}", t.clips.len(), c.doc.undo_label()))
        })),
    ]
}

/// Run the scripted steps, one per timer tick. Prints a summary and quits.
pub fn start(app: &AppWindow, ctl: Ctl) {
    let mut all = steps();
    all.extend(synth_steps());
    all.extend(midi_steps());
    all.extend(recording_steps());
    let steps = Rc::new(all);
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
            Step::DoC(name, f) => {
                eprintln!("self-test: > {name}");
                if let Ok(mut c) = ctl.try_borrow_mut() {
                    c.update_ui(&app);
                }
                f(&app, &ctl);
            }
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
