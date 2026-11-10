//! `--self-test`: drives the real UI input path with synthetic Slint window
//! events (pointer + keyboard → TouchArea/FocusScope → callbacks → Commands)
//! and checks the resulting `Document` / engine state. Does not touch the
//! system cursor or keyboard. Coordinates are window points at 1440x820.

use crate::controller::{BottomTab, Controller};
use crate::{AppWindow, Studio};
use crate::assistant_panel::EntryKind;
use saine_core::assistant::Mode;
use saine_core::builtin::synth_v2::SYNTH_V2;
use saine_core::{PPQ, TrackKind, db_to_gain};
use std::cell::Cell;
use slint::platform::{Key, PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, Model, SharedString};
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
const LANE_X: f32 = 453.65;
const TRACK0_Y: f32 = 144.5;
const PITCH: f32 = crate::timeline::TRACK_PITCH;

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
            // Grid menu above the value: 8 rows of 20.4 with 3.4 padding and a 10.2 gap before the triplets.
            let item = |i: usize| (gx + 22.0, gy - 10.2 - (8.0 * 20.4 + 6.8 + 10.2) + 3.4 + i as f32 * 20.4 + 10.2);
            click(a, gx, gy);
            let (ix, iy) = item(3);
            click(a, ix, iy); // 1/32
            let (x, y) = pr_xy(ctl, 3.375 + 0.03, 52);
            click(a, x, y);
            click(a, gx, gy);
            let (ix, iy) = item(2);
            click(a, ix, iy); // back to 1/16
        })),
        Check("off-grid note drawn at 3240 ticks, grid back at 1/16", Box::new(move |c, a| {
            let n = notes_of(c);
            h13.set(c.doc.history_labels().len());
            (n.iter().any(|n| n.start == 3240 && n.pitch == 52) && c.pr.grid == 16 && a.global::<Studio>().get_pr_grid_text() == "1/16")
                .then_some(())
                .ok_or(format!("grid {} {n:?}", c.pr.grid))
        })),
        Do("click Quantize (selection = the new note)", Box::new(|a| click(a, 267.0 + 15.0, 466.0))),
        DoC("Grid menu: pick 1/8T", Box::new(|a, _| {
            let (gx, gy) = (260.0 + 387.0 + 8.0, 452.0 + (390.0 - 27.0) - 54.5 + 30.0 + 7.0);
            click(a, gx, gy);
            click(a, gx + 22.0, gy - 10.2 - (8.0 * 20.4 + 6.8 + 10.2) + 3.4 + 5.0 * 20.4 + 10.2 + 10.2);
        })),
        Check("Grid 1/8T (1/12 note), menu closed", Box::new(|c, a| {
            let s = a.global::<Studio>();
            (c.pr.grid == 12 && s.get_pr_grid_text() == "1/8T" && s.get_clip_menu() == -1)
                .then_some(())
                .ok_or(format!("grid {} {:?} menu {}", c.pr.grid, s.get_pr_grid_text(), s.get_clip_menu()))
        })),
        DoC("Grid back to 1/16 (menu pick)", Box::new(|a, ctl| {
            ctl.borrow_mut().clip_menu_open(3);
            a.global::<Studio>().invoke_clip_menu_pick(2);
        })),
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

/// The Instrument track's Synth: (values, preset name).
fn inst_synth(c: &Controller) -> Option<(Vec<f32>, Option<String>)> {
    let d = c.project().tracks.iter().find(|t| t.kind == TrackKind::Instrument)?.instrument_plugin.as_ref()?;
    Some((SYNTH_V2.values(d), d.preset.clone()))
}

fn sx(id: &str) -> usize {
    crate::synth::ix(id)
}

/// Factory sound n as the Synth's values.
fn factory_values(n: usize) -> Vec<f32> {
    let m = saine_core::builtin::DevicePreset::factory(&SYNTH_V2, &SYNTH_V2.factory[n]).load_values(&SYNTH_V2);
    SYNTH_V2.params.iter().map(|p| m.get(p.id).copied().unwrap_or(p.default)).collect()
}

/// Synth device (Devices tab on the Instrument track). Leaves the synth back at its
/// starting parameters (everything undone).
fn synth_steps() -> Vec<Step> {
    let bright_low = Rc::new(Cell::new(-1f32));
    let bright_low2 = bright_low.clone();
    let fold_hist = Rc::new(Cell::new(0usize));
    let fold_hist2 = fold_hist.clone();
    use Step::*;
    #[derive(Default)]
    struct Rec {
        hist: usize,
        start: Option<(Vec<f32>, Option<String>)>,
        before: Vec<f32>,
        path: String,
        routes: usize,
    }
    let rec = Rc::new(RefCell::new(Rec::default()));
    let r = |n: usize| -> Vec<Rc<RefCell<Rec>>> { (0..n).map(|_| rec.clone()).collect() };
    let rs = r(18);
    let [r0, r1, r2, r3, r4, r5, r6, r7, r8, r9, r10, r11, r12, r13, r14, r15, r16, r16b] = <[Rc<RefCell<Rec>>; 18]>::try_from(rs).ok().unwrap();
    // One new undo step with this label since the last record; re-records.
    fn one_step(rc: &Rc<RefCell<Rec>>, c: &Controller, label: &str) -> Result<(), String> {
        let n = c.doc.history_labels().len();
        let was = rc.borrow().hist;
        rc.borrow_mut().hist = n;
        (n == was + 1 && c.doc.undo_label().is_some_and(|l| l.starts_with(label)))
            .then_some(())
            .ok_or(format!("history {was} -> {n}, last {:?}", c.doc.undo_label()))
    }
    fn values(c: &Controller) -> Result<Vec<f32>, String> {
        inst_synth(c).map(|v| v.0).ok_or_else(|| "no Synth on the instrument track".to_string())
    }
    fn param_text(a: &AppWindow, i: usize) -> String {
        slint::Model::row_data(&a.global::<Studio>().get_synth_params(), i).map(|p| p.text.to_string()).unwrap_or_default()
    }
    fn others_equal(a: &[f32], b: &[f32], except: &[usize]) -> bool {
        a.iter().zip(b).enumerate().all(|(i, (x, y))| except.contains(&i) || x == y)
    }
    const P: &str = "Device Parameter Change";
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
            let ok = s.get_synth_visible() && s.get_bottom_tab() == 0 && s.get_synth_page() == 0 && c.synth_track().is_some() && slint::Model::row_count(&s.get_synth_knobs()) == 5;
            ok.then_some(()).ok_or(format!("visible {} tab {} page {}", s.get_synth_visible(), s.get_bottom_tab(), s.get_synth_page()))
        })),
        Do("click preset ›", Box::new(|a| {
            let (x, y) = synth_xy(crate::devices::SYNTH_W - 45.0, 16.0);
            click(a, x, y);
        })),
        Check("synth: preset › loaded the first factory sound in one undo step", Box::new(move |c, a| {
            one_step(&r1, c, "Load Preset")?;
            let (v, name) = inst_synth(c).ok_or("no instrument")?;
            let want = SYNTH_V2.factory[0].name;
            (v == factory_values(0) && name.as_deref() == Some(want) && a.global::<Studio>().get_synth_preset() == want).then_some(()).ok_or(format!("preset {name:?}"))
        })),
        Do("click the preset name, pick the 4th sound in the list", Box::new(|a| {
            let (x, y) = synth_xy(crate::devices::SYNTH_W - 74.0, 16.0);
            click(a, x, y);
            let (x, y) = synth_menu_row(a, 4);
            click(a, x, y);
        })),
        Check("synth: preset list loaded factory sound 4 in one undo step, list closed", Box::new(move |c, a| {
            one_step(&r2, c, "Load Preset")?;
            let (v, name) = inst_synth(c).ok_or("no instrument")?;
            (v == factory_values(3) && !a.global::<Studio>().get_synth_menu_open()).then_some(()).ok_or(format!("preset {name:?}"))
        })),
        Do("Cmd+Z, then the Filter page", Box::new(|a| {
            key(a, "z", true, false);
            let (x, y) = synth_category(2);
            click(a, x, y);
        })),
        Check("synth: undo back to the first sound; Filter page shown", Box::new(move |c, a| {
            let mut r = r3.borrow_mut();
            r.hist = c.doc.history_labels().len();
            r.before = values(c)?;
            (r.before == factory_values(0) && a.global::<Studio>().get_synth_page() == 2).then_some(()).ok_or(format!("page {}", a.global::<Studio>().get_synth_page()))
        })),
        Do("drag the Cutoff knob down 60 pt", Box::new(|a| {
            let (x, y) = synth_xy(152.5, 212.0);
            drag(a, x, y, x, y + 60.0);
        })),
        Check("synth: cutoff decreased (log taper) with exactly one undo step; readout in Hz", Box::new(move |c, a| {
            one_step(&r4, c, P)?;
            let before = r4.borrow().before.clone();
            let v = values(c)?;
            let (i, p) = (sx("cutoff"), &SYNTH_V2.params[sx("cutoff")]);
            let want = p.from_normalized(p.to_normalized(before[i]) - 0.4);
            let text = param_text(a, i);
            (v[i] < before[i] && (v[i] - want).abs() < want * 0.03 && others_equal(&v, &before, &[i]) && text.ends_with("Hz"))
                .then_some(())
                .ok_or(format!("cutoff {} -> {} (want ≈{want}), readout {text:?}", before[i], v[i]))
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
        Check("synth: double-click reset cutoff to its default in one undo step", Box::new(move |c, _| {
            one_step(&r5, c, P)?;
            let v = values(c)?;
            let i = sx("cutoff");
            (v[i] == SYNTH_V2.params[i].default).then_some(()).ok_or(format!("cutoff {}", v[i]))
        })),
        Check("audio: panel cutoff reaches the engine (brighter after reset than with the lowered cutoff)", Box::new(move |c, _| {
            let Some(reset) = rendered_brightness(c)? else { return Ok(()) };
            let low = bright_low2.get();
            (low >= 0.0 && reset > low * 1.15).then_some(()).ok_or(format!("brightness low {low} vs reset {reset}"))
        })),
        Do("Oscillators page, open the Osc 2 wave dropdown", Box::new(|a| {
            let (x, y) = synth_category(0);
            click(a, x, y);
            let (x, y) = synth_xy(433.0 + 45.0, 42.0);
            click(a, x, y);
        })),
        Check("synth: wave list open with the six waves; record the Osc 2 picture", Box::new(move |c, a| {
            let s = a.global::<Studio>();
            let items: Vec<String> = (0..slint::Model::row_count(&s.get_synth_menu_items())).filter_map(|i| slint::Model::row_data(&s.get_synth_menu_items(), i)).map(|m| m.label.to_string()).collect();
            r6.borrow_mut().path = slint::Model::row_data(&s.get_synth_displays(), 1).map(|d| d.line.to_string()).unwrap_or_default();
            r6.borrow_mut().hist = c.doc.history_labels().len();
            (s.get_synth_page() == 0 && s.get_synth_menu_open() && items == ["Sine", "Triangle", "Saw", "Square", "Sync", "FM"]).then_some(()).ok_or(format!("page {} open {} {items:?}", s.get_synth_page(), s.get_synth_menu_open()))
        })),
        Do("pick Square", Box::new(|a| {
            let (x, y) = synth_menu_row(a, 3);
            click(a, x, y);
        })),
        Check("synth: Osc 2 wave = Square (one undo step), pill text and picture updated", Box::new(move |c, a| {
            one_step(&r7, c, P)?;
            let s = a.global::<Studio>();
            let v = values(c)?;
            let i = sx("osc2_wave");
            let line = slint::Model::row_data(&s.get_synth_displays(), 1).map(|d| d.line.to_string()).unwrap_or_default();
            let pills = s.get_synth_pills();
            let text = (0..slint::Model::row_count(&pills)).filter_map(|k| slint::Model::row_data(&pills, k)).find(|p| p.action == 0 && p.arg == i as i32).map(|p| p.value.to_string()).unwrap_or_default();
            let changed = line != r7.borrow().path;
            r7.borrow_mut().path = line;
            (v[i] == 3.0 && text == "Square" && changed && !s.get_synth_menu_open()).then_some(()).ok_or(format!("wave {} text {text} path changed {changed}", v[i]))
        })),
        Do("Osc 2 wave dropdown -> Saw", Box::new(|a| {
            let (x, y) = synth_xy(433.0 + 45.0, 42.0);
            click(a, x, y);
            let (x, y) = synth_menu_row(a, 2);
            click(a, x, y);
        })),
        Check("synth: Osc 2 wave = Saw; Saw and Square pictures differ", Box::new(move |c, a| {
            one_step(&r8, c, P)?;
            let v = values(c)?;
            let line = slint::Model::row_data(&a.global::<Studio>().get_synth_displays(), 1).map(|d| d.line.to_string()).unwrap_or_default();
            (v[sx("osc2_wave")] == 2.0 && line != r8.borrow().path && !line.is_empty()).then_some(()).ok_or(format!("wave {}", v[sx("osc2_wave")]))
        })),
        Do("drag the Osc 1 Level fader up 10 pt", Box::new(|a| {
            let (x, y) = synth_xy(117.5 + 273.3, 61.5 + 45.0);
            drag(a, x, y, x, y - 10.0);
        })),
        Check("synth: Osc 1 level +10/91 in one undo step, readout in %", Box::new(move |c, a| {
            let i = sx("osc1_level");
            let before = factory_values(0)[i];
            one_step(&r9, c, P)?;
            let v = values(c)?;
            let text = param_text(a, i);
            (approx(v[i], before + 10.0 / 91.0, 0.01) && text == format!("{:.0}%", v[i] * 100.0)).then_some(()).ok_or(format!("level {} text {text}", v[i]))
        })),
        Do("Envelopes page", Box::new(|a| {
            let (x, y) = synth_category(3);
            click(a, x, y);
        })),
        Check("synth: Envelopes page; record params", Box::new(move |c, a| {
            r10.borrow_mut().before = values(c)?;
            r10.borrow_mut().hist = c.doc.history_labels().len();
            (a.global::<Studio>().get_synth_page() == 3).then_some(()).ok_or("page".into())
        })),
        Do("drag the Amp Attack knob up 45 pt", Box::new(|a| {
            let (x, y) = synth_xy(117.5 + 27.5, 212.0);
            drag(a, x, y, x, y - 45.0);
        })),
        Check("synth: amp attack increased (one undo step), Env 2 unchanged, readout in ms/s", Box::new(move |c, a| {
            one_step(&r11, c, P)?;
            let before = r11.borrow().before.clone();
            let v = values(c)?;
            let i = sx("amp_attack");
            let text = param_text(a, i);
            (v[i] > before[i] * 2.0 && others_equal(&v, &before, &[i]) && (text.ends_with(" ms") || text.ends_with(" s")))
                .then_some(())
                .ok_or(format!("attack {} -> {} {text}", before[i], v[i]))
        })),
        Do("Cmd+Z", Box::new(|a| key(a, "z", true, false))),
        Check("synth: ⌘Z restored the previous parameters exactly", Box::new(move |c, _| {
            let before = r12.borrow().before.clone();
            (values(c)? == before).then_some(()).ok_or("params differ after undo".into())
        })),
        Do("Modulation page", Box::new(|a| {
            let (x, y) = synth_category(5);
            click(a, x, y);
        })),
        Check("synth: Modulation page lists the routes", Box::new(move |c, a| {
            let s = a.global::<Studio>();
            let n = slint::Model::row_count(&s.get_synth_routes());
            let want = crate::synth::routes(&values(c)?).len();
            r13.borrow_mut().routes = n;
            r13.borrow_mut().hist = c.doc.history_labels().len();
            (s.get_synth_page() == 5 && n == want && s.get_synth_add_show()).then_some(()).ok_or(format!("{n} rows, {want} routes"))
        })),
        Do("+ Add modulation", Box::new(|a| {
            let y = a.global::<Studio>().get_synth_add_y();
            let (x, y) = synth_xy(112.0 + 40.0, 35.0 + y + 10.0);
            click(a, x, y);
        })),
        Check("synth: the source list is open", Box::new(|_, a| {
            let s = a.global::<Studio>();
            let n = slint::Model::row_count(&s.get_synth_menu_items());
            (s.get_synth_menu_open() && n == 9).then_some(()).ok_or(format!("open {} rows {n}", s.get_synth_menu_open()))
        })),
        Do("pick Velocity", Box::new(|a| {
            let (x, y) = synth_menu_row(a, 4);
            click(a, x, y);
        })),
        Check("synth: a Velocity route was added in one undo step", Box::new(move |c, a| {
            one_step(&r14, c, "Device Parameters Change")?;
            let v = values(c)?;
            let rs = crate::synth::routes(&v);
            let n = slint::Model::row_count(&a.global::<Studio>().get_synth_routes());
            (rs.len() == r14.borrow().routes + 1 && n == rs.len() && rs.iter().any(|r| r.1 == 5)).then_some(()).ok_or(format!("{} routes", rs.len()))
        })),
        Do("Effects page", Box::new(|a| {
            let (x, y) = synth_category(8);
            click(a, x, y);
        })),
        Check("synth: Effects page shows the four slot tabs, the mix fader and the dials", Box::new(move |_, a| {
            let s = a.global::<Studio>();
            let tabs = (0..slint::Model::row_count(&s.get_synth_pills())).filter_map(|k| slint::Model::row_data(&s.get_synth_pills(), k)).filter(|p| p.action == 9).count();
            let _ = &r15;
            (s.get_synth_page() == 8 && tabs == 4 && slint::Model::row_count(&s.get_synth_faders()) == 1 && slint::Model::row_count(&s.get_synth_knobs()) >= 4).then_some(()).ok_or(format!("{tabs} tabs"))
        })),
        Do("Cmd+Z x7 (route, level, Saw, Square, reset, cutoff drag, preset ›)", Box::new(|a| {
            for _ in 0..7 {
                key(a, "z", true, false);
            }
        })),
        Check("synth: every synth edit undone -> starting parameters exactly", Box::new(move |c, _| {
            let start = r16.borrow().start.clone().ok_or("no record")?;
            (inst_synth(c).as_ref() == Some(&start)).then_some(()).ok_or(format!("preset {:?}", inst_synth(c).map(|p| p.1)))
        })),
        DoC("click the Synth header's fold button", Box::new(move |a, ctl| {
            fold_hist.set(ctl.borrow().doc.history_labels().len());
            let (x, y) = synth_xy(crate::devices::SYNTH_W - 27.5, 16.0);
            click(a, x, y);
        })),
        Check("synth: folds to the 30 pt strip in one undo step", Box::new(move |c, a| {
            let card = a.global::<Studio>().get_synth_card();
            let n = c.doc.history_labels().len();
            (n == fold_hist2.get() + 1 && card.folded && card.width == crate::device_mod::FOLDED_W).then_some(()).ok_or(format!("history {} -> {n}, folded {} width {}", fold_hist2.get(), card.folded, card.width))
        })),
        Do("Cmd+Z", Box::new(|a| key(a, "z", true, false))),
        Check("synth: undo unfolds it to full width", Box::new(move |_, a| {
            let card = a.global::<Studio>().get_synth_card();
            (!card.folded && card.width == crate::devices::SYNTH_W).then_some(()).ok_or(format!("folded {} width {}", card.folded, card.width))
        })),
        Do("click the Synth header's remove button", Box::new(|a| {
            let (x, y) = synth_xy(crate::devices::SYNTH_W - 9.5, 16.0);
            click(a, x, y);
        })),
        Check("synth: removed, the slot is empty", Box::new(|c, a| {
            (c.synth_device().is_none() && !a.global::<Studio>().get_synth_visible()).then_some(()).ok_or("the Synth is still there".to_string())
        })),
        DoC("choose Synth for the empty slot", Box::new(|_, ctl| {
            let mut c = ctl.borrow_mut();
            let t = c.selected_track.expect("selected track");
            c.add_item_to_track(&crate::devices::Item::BuiltinSynth, t, None);
        })),
        Check("synth: choosing Synth restores it", Box::new(|c, a| {
            (c.synth_device().is_some() && a.global::<Studio>().get_synth_visible()).then_some(()).ok_or("no Synth".to_string())
        })),
        Do("Cmd+Z x2 (add, remove)", Box::new(|a| {
            key(a, "z", true, false);
            key(a, "z", true, false);
        })),
        Check("synth: undo brings back the original Synth", Box::new(move |c, _| {
            let start = r16b.borrow().start.clone().ok_or("no record")?;
            (inst_synth(c).as_ref() == Some(&start)).then_some(()).ok_or(format!("preset {:?}", inst_synth(c).map(|p| p.1)))
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
    // Card: 8 px inset, 143 px tall with the AI notice (CSS px x 0.85).
    const CARD_X: f32 = 1440.0 - 5.0 - 252.5 + 6.8;
    const CARD_Y: f32 = 78.0 + 426.0 - 6.8 - 121.55;
    const INPUT_Y: f32 = CARD_Y + 28.05 + 27.2 + 22.0;
    const MIXER_Y: f32 = 820.0 - 5.0 - 390.0;
    // Mixer strip fader travel (ui/channel_strip.slint): the 200 pt canvas sits 234 pt above the
    // strip bottom; its handle travels 168 pt starting 18 pt down (reference points * 0.85).
    const MIXER_TRAVEL_TOP: f32 = 390.0 - (234.0 - 18.0) * 0.85;
    const MIXER_TRAVEL: f32 = 168.0 * 0.85;
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
        Do("click M on track 1", Box::new(|a| click(a, 262.65 + 33.5, TRACK0_Y + 47.0))),
        Check("track 1 muted (SetTrackMute)", Box::new(|c, _| c.project().tracks[0].mute.then_some(()).ok_or("not muted".into()))),
        Check("Edit menu names the edit: Undo Mute Change enabled", Box::new(|_, a| {
            let s = a.global::<Studio>();
            (s.get_undo_text() == "Undo Mute Change" && s.get_can_undo()).then_some(()).ok_or(format!("{:?} {}", s.get_undo_text(), s.get_can_undo()))
        })),
        Do("Cmd+Z (Edit › Undo shortcut, replayed by the menu)", Box::new(|a| key(a, "z", true, false))),
        Check("undo restored mute=false", Box::new(|c, _| (!c.project().tracks[0].mute).then_some(()).ok_or("still muted".into()))),
        Check("Edit menu: Redo Mute Change enabled", Box::new(|_, a| {
            let s = a.global::<Studio>();
            (s.get_redo_text() == "Redo Mute Change" && s.get_can_redo()).then_some(()).ok_or(format!("{:?} {}", s.get_redo_text(), s.get_can_redo()))
        })),
        Do("Cmd+Shift+Z", Box::new(|a| key(a, "Z", true, true))),
        Check("redo muted again", Box::new(|c, _| c.project().tracks[0].mute.then_some(()).ok_or("redo failed".into()))),
        Do("⇧⌘E (file.export_audio)", Box::new(|a| key(a, "E", true, true))),
        Check("export dialog open with a range and a size estimate", Box::new(|_c, a| {
            let s = a.global::<Studio>();
            (s.get_export_open() && s.get_export_bars_text().starts_with("Bars 1–") && s.get_export_estimate().contains("MB"))
                .then_some(())
                .ok_or(format!("open {} bars {} est {}", s.get_export_open(), s.get_export_bars_text(), s.get_export_estimate()))
        })),
        DoC("export into a temp folder: click Export", Box::new(|a, ctl| {
            {
                let mut c = ctl.borrow_mut();
                c.export.dir = std::env::temp_dir().join(format!("saine-selftest-export-{}", std::process::id()));
                c.export.name = "mix".into();
            }
            // Export window centred in the 1440 x 820 window: 442 x 340.4, Export button 52.2 x 22.1.
            click(a, (1440.0 - 442.0) / 2.0 + 442.0 - 17.0 - 26.1, (820.0 - 340.4) / 2.0 + 340.4 - 57.8 + 16.15 + 11.05);
        })),
        Check("mix.wav written: stereo 24-bit at the engine rate, the full length, not silent; dialog closed", Box::new(|c, a| {
            let path = c.export.dir.join("mix.wav");
            let (rate, ch, s) = crate::wavgen::read_wav(&path)?;
            let peak = s.iter().fold(0f32, |m, v| m.max(v.abs()));
            let frames = s.len() / 2;
            (rate as f64 == c.engine.sample_rate() && ch == 2 && frames as u64 == c.export_frames() && peak > 0.01 && !a.global::<Studio>().get_export_open())
                .then_some(())
                .ok_or(format!("rate {rate} ch {ch} frames {frames} vs {} peak {peak}", c.export_frames()))
        })),
        Do("⇧⌘E again", Box::new(|a| key(a, "E", true, true))),
        Check("export dialog open again", Box::new(|_c, a| a.global::<Studio>().get_export_open().then_some(()).ok_or("not open".into()))),
        Do("click the Stems card", Box::new(|a| click(a, (1440.0 - 442.0) / 2.0 + 17.0 + 131.47 + 6.8 + 65.0, (820.0 - 340.4) / 2.0 + 44.2 + 22.0))),
        Check("Stems picked: Tracks row, one file per heard track with clips, Folder label", Box::new(|c, a| {
            let s = a.global::<Studio>();
            let n = crate::dialogs::stem_tracks(c.project(), crate::dialogs::StemsPick::AllTracks, None).len();
            (s.get_export_recipe() == 1 && n > 0 && s.get_export_stems_text() == format!("{n} WAV file{}", if n == 1 { "" } else { "s" }) && s.get_export_name_label() == "Folder" && s.get_export_tracks_label() == "All individual tracks")
                .then_some(())
                .ok_or(format!("recipe {} files {n} text {} label {}", s.get_export_recipe(), s.get_export_stems_text(), s.get_export_name_label()))
        })),
        Do("click the Tracks picker", Box::new(|a| click(a, (1440.0 - 442.0) / 2.0 + 71.4 + 30.0, (820.0 - 340.4) / 2.0 + 105.4 + 10.2))),
        Check("Tracks menu lists both choices, a gap and every track", Box::new(|c, a| {
            let s = a.global::<Studio>();
            let rows = s.get_export_tracks_items().row_count();
            (s.get_export_tracks_menu_open() && rows == 3 + c.project().tracks.len()).then_some(()).ok_or(format!("open {} rows {rows}", s.get_export_tracks_menu_open()))
        })),
        Do("Escape", Box::new(|a| key(a, Key::Escape, false, false))),
        Check("Escape closed the menu, not the dialog", Box::new(|_c, a| {
            let s = a.global::<Studio>();
            (s.get_export_open() && !s.get_export_tracks_menu_open()).then_some(()).ok_or("menu or dialog state".into())
        })),
        DoC("name the stems mix and click Export", Box::new(|a, ctl| {
            ctl.borrow_mut().export.name = "mix".into();
            click(a, (1440.0 - 442.0) / 2.0 + 442.0 - 17.0 - 26.1, (820.0 - 340.4) / 2.0 + 340.4 - 57.8 + 16.15 + 11.05);
        })),
        Check("'mix Stems' holds one 'NN Name.wav' per track, full length, summing to the mixdown", Box::new(|c, a| {
            let p = c.project();
            let tracks = crate::dialogs::stem_tracks(p, crate::dialogs::StemsPick::AllTracks, None);
            let folder = c.export.dir.join("mix Stems");
            let (_, _, mix) = crate::wavgen::read_wav(&c.export.dir.join("mix.wav"))?;
            let mut sum = vec![0f32; mix.len()];
            for (i, id) in tracks.iter().enumerate() {
                let name = crate::dialogs::stem_file_name(i, &p.track(*id).map_or(String::new(), |t| t.name.clone()));
                let (rate, ch, s) = crate::wavgen::read_wav(&folder.join(&name))?;
                if rate as f64 != c.engine.sample_rate() || ch != 2 || s.len() != mix.len() {
                    return Err(format!("{name}: rate {rate} ch {ch} len {} vs {}", s.len(), mix.len()));
                }
                sum.iter_mut().zip(&s).for_each(|(a, b)| *a += b);
            }
            let files = std::fs::read_dir(&folder).map_or(0, |d| d.count());
            let _ = std::fs::remove_dir_all(&c.export.dir);
            let err = sum.iter().zip(&mix).fold(0f32, |m, (a, b)| m.max((a - b).abs()));
            let peak = mix.iter().fold(0f32, |m, v| m.max(v.abs()));
            (files == tracks.len() && err < 1e-3 && peak > 0.01 && !a.global::<Studio>().get_export_open())
                .then_some(())
                .ok_or(format!("files {files} of {} max error {err} peak {peak}", tracks.len()))
        })),
        Do("⇧⌘E for the Project export", Box::new(|a| key(a, "E", true, true))),
        Check("export dialog open for Project", Box::new(|_c, a| a.global::<Studio>().get_export_open().then_some(()).ok_or("not open".into()))),
        DoC("click the Project card, name it set, click Export", Box::new(|a, ctl| {
            click(a, (1440.0 - 442.0) / 2.0 + 17.0 + 2.0 * (131.47 + 6.8) + 65.0, (820.0 - 340.4) / 2.0 + 44.2 + 22.0);
            let mut c = ctl.borrow_mut();
            c.export.dir = std::env::temp_dir().join(format!("saine-selftest-project-{}", std::process::id()));
            c.export.name = "set".into();
        })),
        Check("Project picked: recipe 2, Name label", Box::new(|_c, a| {
            let s = a.global::<Studio>();
            (s.get_export_recipe() == 2 && s.get_export_name_label() == "Name").then_some(()).ok_or(format!("recipe {}", s.get_export_recipe()))
        })),
        Do("click Export", Box::new(|a| click(a, (1440.0 - 442.0) / 2.0 + 442.0 - 17.0 - 26.1, (820.0 - 340.4) / 2.0 + 340.4 - 57.8 + 16.15 + 11.05))),
        Check("'set Project' holds set.als that passes the Live structure check, its renders and the notes", Box::new(|c, a| {
            let folder = c.export.dir.join("set Project");
            let gz = std::fs::read(folder.join("set.als")).map_err(|e| format!("set.als: {e}"))?;
            let mut xml = String::new();
            std::io::Read::read_to_string(&mut flate2::read::GzDecoder::new(&gz[..]), &mut xml).map_err(|e| e.to_string())?;
            let sum = crate::live_set::validate(&xml, Some(&folder))?;
            let p = c.project();
            let with_clips = p.tracks.iter().filter(|t| !t.clips.is_empty() || !t.midi_clips.is_empty()).count();
            let midi: usize = p.tracks.iter().map(|t| t.midi_clips.len()).sum();
            let notes = folder.join("Saine Export Notes.txt").is_file();
            let _ = std::fs::remove_dir_all(&c.export.dir);
            (sum.audio_clips == with_clips && sum.midi_clips == midi && notes && !a.global::<Studio>().get_export_open())
                .then_some(())
                .ok_or(format!("{sum:?} vs {with_clips} rendered, {midi} MIDI clips; notes {notes}"))
        })),
        Do("⌘N (file.new) with unsaved changes", Box::new(|a| key(a, "n", true, false))),
        Check("Save changes? prompt open, project untouched", Box::new(|c, a| {
            let n: usize = c.project().tracks.iter().map(|t| t.clips.len()).sum();
            (a.global::<Studio>().get_unsaved_open() && n == 2).then_some(()).ok_or(format!("prompt {} clips {n}", a.global::<Studio>().get_unsaved_open()))
        })),
        Do("Escape (Cancel)", Box::new(|a| key(a, Key::Escape, false, false))),
        Check("prompt closed, project kept", Box::new(|c, a| {
            let n: usize = c.project().tracks.iter().map(|t| t.clips.len()).sum();
            (!a.global::<Studio>().get_unsaved_open() && !c.unsaved_open && n == 2).then_some(()).ok_or(format!("clips {n}"))
        })),
        Do("drag track 1 volume slider left", Box::new(|a| drag(a, 262.65 + 115.7 + 5.95 + 0.8 * 56.1, TRACK0_Y + 47.6, 262.65 + 140.0, TRACK0_Y + 47.6))),
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
        DoC("select clip 1 and open the Clip tab", Box::new(move |a, ctl| {
            click(a, bx(11.0), TRACK0_Y + 40.0);
            ctl.borrow_mut().set_bottom_tab(BottomTab::Clip);
        })),
        Check("audio clip editor open: gain readout, no piano roll, clip-length ruler", Box::new(|c, a| {
            let s = a.global::<Studio>();
            let clip = c.ac_clip().ok_or("no audio clip open")?;
            let bars = (c.project().clip_length_ticks(clip) as f64 / c.project().ticks_per_bar() as f64).ceil() as usize;
            (s.get_ac_visible() && !s.get_pr_visible() && s.get_ac_gain_text() == "0.0 dB" && s.get_pr_bar_labels().row_count() >= bars && c.pr_view().gx == 0.0)
                .then_some(())
                .ok_or(format!("ac {} pr {} gain {} bars {}", s.get_ac_visible(), s.get_pr_visible(), s.get_ac_gain_text(), s.get_pr_bar_labels().row_count()))
        })),
        Check("audio clip editor draws the clip waveform across the clip", Box::new(|c, _| {
            let sc = c.pr_scene(1.0, &[]).ok_or("no clip editor scene")?;
            let mut dl = crate::gpu::DrawList::default();
            crate::pianoroll::build(&mut dl, &sc);
            let (x0, x1) = (sc.view.x_of(sc.range.0), sc.view.x_of(sc.range.1).min(sc.view.w));
            let span = dl.waves.iter().map(|w| w.rect[2]).sum::<f32>();
            (dl.waves.len() == 1 && (span - (x1 - x0)).abs() <= 1.0)
                .then_some(())
                .ok_or(format!("waves {} span {span} want {}", dl.waves.len(), x1 - x0))
        })),
        // ---- warp (SAIN-38): Speed / Warp / Mode menus, Original BPM drag, a warp marker ----
        DoC("warp: Warp menu -> On", Box::new(|a, ctl| {
            ctl.borrow_mut().clip_menu_open(1);
            a.global::<Studio>().invoke_clip_menu_pick(0);
        })),
        Check("warp on at the project tempo: same length, Mode enabled", Box::new(|c, a| {
            let s = a.global::<Studio>();
            let clip = c.ac_clip().ok_or("no audio clip open")?;
            let w = &clip.warp;
            let plain = c.project().seconds_to_ticks(clip.source_length as f64 / c.project().asset(clip.asset).map_or(48_000.0, |x| x.sample_rate));
            let len = c.project().clip_length_ticks(clip);
            (w.enabled && w.original_bpm == c.project().transport.bpm && (len - plain).abs() <= 1 && s.get_ac_warped() && s.get_ac_speed_text() == "1x" && c.doc.undo_label() == Some("Set clip warp mode"))
                .then_some(())
                .ok_or(format!("warp {w:?} len {len} vs {plain} speed {}", s.get_ac_speed_text()))
        })),
        DoC("warp: Speed menu -> 2x", Box::new(|a, ctl| {
            ctl.borrow_mut().clip_menu_open(0);
            a.global::<Studio>().invoke_clip_menu_pick(5);
        })),
        Check("double speed halves the clip", Box::new(|c, a| {
            let clip = c.ac_clip().ok_or("no audio clip open")?;
            let plain = c.project().seconds_to_ticks(clip.source_length as f64 / c.project().asset(clip.asset).map_or(48_000.0, |x| x.sample_rate));
            let len = c.project().clip_length_ticks(clip);
            let s = a.global::<Studio>();
            ((len - plain / 2).abs() <= 1 && s.get_ac_speed_text() == "2x" && c.doc.undo_label() == Some("Set clip speed"))
                .then_some(())
                .ok_or(format!("len {len} vs {plain} speed {}", s.get_ac_speed_text()))
        })),
        DoC("warp: Mode menu -> Drums", Box::new(|a, ctl| {
            ctl.borrow_mut().clip_menu_open(2);
            a.global::<Studio>().invoke_clip_menu_pick(1);
        })),
        Check("mode Drums", Box::new(|c, a| {
            let clip = c.ac_clip().ok_or("no audio clip open")?;
            (clip.warp.mode == saine_core::WarpMode::Drums && a.global::<Studio>().get_ac_mode_text() == "Drums" && c.pr.menu == -1)
                .then_some(())
                .ok_or(format!("mode {:?}", clip.warp.mode))
        })),
        Do("warp: drag Original BPM right 20 pt", Box::new(|a| {
            let s = a.global::<Studio>();
            s.invoke_ac_bpm_drag(0, 0.0, 0.0);
            s.invoke_ac_bpm_drag(1, 10.0, 0.0);
            s.invoke_ac_bpm_drag(1, 20.0, 0.0);
            s.invoke_ac_bpm_drag(2, 20.0, 0.0);
        })),
        Check("original BPM +20, one undo step", Box::new(|c, a| {
            let clip = c.ac_clip().ok_or("no audio clip open")?;
            let want = c.project().transport.bpm + 20.0;
            (clip.warp.original_bpm == want && a.global::<Studio>().get_ac_bpm_text() == format!("{}", want.round()) && c.doc.undo_label() == Some("Set original BPM"))
                .then_some(())
                .ok_or(format!("bpm {} label {:?}", clip.warp.original_bpm, c.doc.undo_label()))
        })),
        DoC("warp: click the handle band at beat 1 (add a warp marker)", Box::new(|_, ctl| {
            let x = ctl.borrow().pr_view().x_of(1.0);
            let mut c = ctl.borrow_mut();
            c.pr_pointer(0, x, 30.0, false, true, false);
            c.pr_pointer(2, x, 30.0, false, true, false);
        })),
        Check("warp marker added (with the clip-start anchor)", Box::new(|c, _| {
            let clip = c.ac_clip().ok_or("no audio clip open")?;
            let beats = c.ac_marker_beats();
            (clip.warp.markers.len() == 2 && beats.len() == 2 && beats[0].abs() < 1e-6 && (beats[1] - 1.0).abs() < 0.05 && c.doc.undo_label() == Some("Add warp marker"))
                .then_some(())
                .ok_or(format!("markers {:?} beats {beats:?}", clip.warp.markers))
        })),
        Check("the clip editor waveform bends at the warp marker", Box::new(|c, _| {
            let sc = c.pr_scene(1.0, &[]).ok_or("no clip editor scene")?;
            let segs = sc.wave.as_ref().map(|w| w.1.clone()).unwrap_or_default();
            let marker = c.ac_marker_beats().get(1).copied().unwrap_or(f64::NAN);
            let mut dl = crate::gpu::DrawList::default();
            crate::pianoroll::build(&mut dl, &sc);
            (segs.len() == 2 && dl.waves.len() == 2 && (segs[0].beats.1 - marker).abs() < 1e-6 && dl.waves[0].rect[0] + dl.waves[0].rect[2] == dl.waves[1].rect[0])
                .then_some(())
                .ok_or(format!("segs {segs:?} marker {marker} waves {}", dl.waves.len()))
        })),
        DoC("warp: Shift-drag the marker half a beat right (slide the audio)", Box::new(|_, ctl| {
            let (x0, x1) = { let c = ctl.borrow(); let v = c.pr_view(); (v.x_of(1.0), v.x_of(1.5)) };
            let mut c = ctl.borrow_mut();
            c.pr_pointer(0, x0, 30.0, true, false, false);
            c.pr_pointer(1, x1, 30.0, false, false, false);
            c.pr_pointer(2, x1, 30.0, true, false, false);
        })),
        Check("slid: the marker stays at beat 1 and pins audio half a beat later", Box::new(|c, _| {
            let clip = c.ac_clip().ok_or("no audio clip open")?;
            let beats = c.ac_marker_beats();
            let rate = c.project().asset(clip.asset).unwrap().sample_rate;
            let per_beat = rate * 60.0 / clip.warp.original_bpm * clip.warp.speed;
            let src = clip.warp.markers.get(1).map_or(0.0, |m| m.source as f64);
            let want = clip.source_offset as f64 + 1.5 * per_beat;
            ((beats.get(1).copied().unwrap_or(f64::NAN) - 1.0).abs() < 0.05 && (src - want).abs() < 0.02 * per_beat && c.doc.undo_label() == Some("Move warp marker"))
                .then_some(())
                .ok_or(format!("beats {beats:?} source {src} want {want} undo {:?}", c.doc.undo_label()))
        })),
        DoC("warp: three transients, Cmd-click the middle one in the handle band", Box::new(|_, ctl| {
            let mut c = ctl.borrow_mut();
            let Some(clip) = c.ac_clip().cloned() else { return };
            let rate = c.project().asset(clip.asset).unwrap().sample_rate;
            let per_beat = rate * 60.0 / clip.warp.original_bpm * clip.warp.speed;
            let at = |b: f64| (clip.source_offset as f64 + b * per_beat) as u64;
            let warp = saine_core::ClipWarp { transients_added: vec![at(2.25), at(2.5), at(2.75)], ..clip.warp.clone() };
            c.execute(saine_core::Command::SetClipWarp { clip: clip.id, warp, label: "Add transient".into() });
            drop(c);
            let x = ctl.borrow().ac_transient_beats().get(1).map(|&b| ctl.borrow().pr_view().x_of(b)).unwrap_or(0.0);
            let mut c = ctl.borrow_mut();
            c.pr_pointer(0, x, 30.0, false, false, true);
            c.pr_pointer(2, x, 30.0, false, false, true);
        })),
        Check("Add warp markers at transients: the transient and both neighbours, where they play", Box::new(|c, _| {
            let clip = c.ac_clip().ok_or("no audio clip open")?;
            let marks = c.ac_marker_beats();
            let tr = c.ac_transient_beats();
            let pinned = tr.iter().filter(|t| marks.iter().any(|m| (m - *t).abs() < 1e-6)).count();
            (clip.warp.markers.len() == 5 && pinned == 3 && c.doc.undo_label() == Some("Add warp markers at transients"))
                .then_some(())
                .ok_or(format!("markers {marks:?} transients {tr:?} undo {:?}", c.doc.undo_label()))
        })),
        DoC("warp: undo the markers and the transients", Box::new(|_, ctl| {
            let mut c = ctl.borrow_mut();
            c.undo();
            c.undo();
        })),
        DoC("warp: Warp menu -> Off", Box::new(|a, ctl| {
            ctl.borrow_mut().clip_menu_open(1);
            a.global::<Studio>().invoke_clip_menu_pick(1);
        })),
        Check("warp off resets the speed and plays 1:1; Mode disabled", Box::new(|c, a| {
            let clip = c.ac_clip().ok_or("no audio clip open")?;
            let plain = c.project().seconds_to_ticks(clip.source_length as f64 / c.project().asset(clip.asset).map_or(48_000.0, |x| x.sample_rate));
            let s = a.global::<Studio>();
            (!clip.warp.enabled && clip.warp.speed == 1.0 && c.project().clip_length_ticks(clip) == plain && !s.get_ac_warped() && c.doc.undo_label() == Some("Reset clip speed"))
                .then_some(())
                .ok_or(format!("warp {:?}", clip.warp))
        })),
        DoC("warp: Mode menu stays closed with warp off", Box::new(|_, ctl| ctl.borrow_mut().clip_menu_open(2))),
        Check("mode menu closed", Box::new(|c, _| (c.pr.menu == -1).then_some(()).ok_or(format!("menu {}", c.pr.menu)))),
        Do("Cmd+Z x7 (back to the unwarped clip)", Box::new(|a| {
            for _ in 0..7 {
                key(a, "z", true, false);
            }
        })),
        Check("warp undone", Box::new(|c, _| {
            let clip = c.ac_clip().ok_or("no audio clip open")?;
            clip.warp.is_default().then_some(()).ok_or(format!("warp {:?}", clip.warp))
        })),
        DoC("reverse: toolbar Reverse -> On", Box::new(|_, ctl| ctl.borrow_mut().ac_reverse_toggle())),
        Check("reversed: toolbar On, the engine's reversed copy exists and the editor draws it, one undo step", Box::new(|c, a| {
            let clip = c.ac_clip().ok_or("no audio clip open")?;
            let rev = c.reversed_assets.get(&clip.asset).copied().ok_or("no reversed copy")?;
            let slot = c.clip_wave(clip).map(|w| w.0.base);
            (clip.warp.reverse && a.global::<Studio>().get_ac_reversed() && slot == c.peaks.slot(rev).map(|s| s.base) && c.doc.undo_label() == Some("Toggle clip reverse"))
                .then_some(())
                .ok_or(format!("reverse {} ui {} slot {slot:?} undo {:?}", clip.warp.reverse, a.global::<Studio>().get_ac_reversed(), c.doc.undo_label()))
        })),
        Do("reversed: drag clip 1 left edge right by 1 beat", Box::new(move |a| drag(a, bx(9.0) + 1.5, TRACK0_Y + 40.0, bx(10.0) + 1.5, TRACK0_Y + 40.0))),
        Check("reversed left trim cuts the window's end: start 10, offset still 0.5 s, 4 beats", Box::new(|c, _| {
            let p = c.project();
            let cl = &p.tracks[0].clips[0];
            let a = p.asset(cl.asset).unwrap();
            let off_s = cl.source_offset as f64 / a.sample_rate;
            let beats = p.clip_length_ticks(cl) as f64 / PPQ as f64;
            ((cl.start as f64 / PPQ as f64 - 10.0).abs() < 1e-6 && (off_s - 0.5).abs() < 1e-3 && (beats - 4.0).abs() < 0.01)
                .then_some(())
                .ok_or(format!("start {} off {off_s}s len {beats}", cl.start))
        })),
        DoC("reverse: undo the trim and the toggle", Box::new(|_, ctl| {
            let mut c = ctl.borrow_mut();
            c.undo();
            c.undo();
        })),
        Check("reverse undone", Box::new(|c, a| {
            let cl = &c.project().tracks[0].clips[0];
            (!cl.warp.reverse && !a.global::<Studio>().get_ac_reversed() && cl.start == 9 * PPQ).then_some(()).ok_or(format!("reverse {} start {}", cl.warp.reverse, cl.start))
        })),
        DoC("transpose: drag the Transpose value 50 pt right", Box::new(|_, ctl| {
            let mut c = ctl.borrow_mut();
            c.ac_transpose_drag(0, 0.0, 0.0);
            c.ac_transpose_drag(1, 20.0, 0.0);
            c.ac_transpose_drag(1, 50.0, 0.0);
            c.ac_transpose_drag(2, 50.0, 0.0);
        })),
        Check("transpose +12 with warp off: an octave up, half as long, one undo step", Box::new(|c, a| {
            let s = a.global::<Studio>();
            let clip = c.ac_clip().ok_or("no audio clip open")?;
            let plain = saine_core::Clip { warp: saine_core::ClipWarp::default(), ..clip.clone() };
            let (len, full) = (c.project().clip_length_ticks(clip), c.project().clip_length_ticks(&plain));
            (clip.warp.transpose == 12 && s.get_ac_transpose_text() == "+12 st" && s.get_ac_transpose_enabled() && len * 2 == full && c.doc.undo_label() == Some("Adjust clip transpose"))
                .then_some(())
                .ok_or(format!("transpose {} text {} len {len} of {full} undo {:?}", clip.warp.transpose, s.get_ac_transpose_text(), c.doc.undo_label()))
        })),
        DoC("transpose: Re-pitch disables it", Box::new(|_, ctl| {
            let mut c = ctl.borrow_mut();
            let Some(clip) = c.ac_clip().cloned() else { return };
            let warp = saine_core::ClipWarp { enabled: true, mode: saine_core::WarpMode::Repitch, original_bpm: 60.0, ..clip.warp.clone() };
            c.execute(saine_core::Command::SetClipWarp { clip: clip.id, warp, label: "Set clip mode".into() });
        })),
        Check("Re-pitch: Transpose greyed, shows the tempo ratio (+12 st at 120 over 60 BPM)", Box::new(|_, a| {
            let s = a.global::<Studio>();
            (!s.get_ac_transpose_enabled() && s.get_ac_transpose_text() == "+12 st").then_some(()).ok_or(format!("enabled {} text {}", s.get_ac_transpose_enabled(), s.get_ac_transpose_text()))
        })),
        DoC("transpose: undo twice", Box::new(|_, ctl| {
            let mut c = ctl.borrow_mut();
            c.undo();
            c.undo();
        })),
        Check("transpose undone", Box::new(|c, a| {
            let clip = c.ac_clip().ok_or("no audio clip open")?;
            (clip.warp.is_default() && a.global::<Studio>().get_ac_transpose_text() == "0 st").then_some(()).ok_or(format!("warp {:?}", clip.warp))
        })),
        DoC("original BPM: double-click to type", Box::new(|_, ctl| ctl.borrow_mut().ac_edit_start(crate::warpedit::AcEdit::OriginalBpm))),
        Check("Original BPM edit box open with the shown value", Box::new(|_, a| {
            let s = a.global::<Studio>();
            (s.get_ac_edit_field() == 0 && s.get_ac_edit_text() == "120").then_some(()).ok_or(format!("field {} text {}", s.get_ac_edit_field(), s.get_ac_edit_text()))
        })),
        DoC("original BPM: type 96.5, Enter, then reopen it (shows 97) and click away", Box::new(|_, ctl| {
            let mut c = ctl.borrow_mut();
            c.ac_edit_commit("96.5");
            c.ac_edit_start(crate::warpedit::AcEdit::OriginalBpm);
            let shown = c.ac_clip().cloned().map(|clip| c.ac_edit_initial(&clip)).unwrap_or_default();
            c.ac_edit_commit(&shown);
        })),
        Check("typed BPM turns warp on at 96.5: the clip follows 120 BPM (longer), one undo step", Box::new(|c, a| {
            let clip = c.ac_clip().ok_or("no audio clip open")?;
            let p = c.project();
            let secs = clip.source_length as f64 / p.asset(clip.asset).unwrap().sample_rate;
            let beats = p.clip_length_ticks(clip) as f64 / PPQ as f64;
            (clip.warp.enabled && clip.warp.original_bpm == 96.5 && (beats - secs * 96.5 / 60.0).abs() < 0.01 && a.global::<Studio>().get_ac_edit_field() == -1 && c.doc.undo_label() == Some("Set original BPM"))
                .then_some(())
                .ok_or(format!("warp {:?} beats {beats} undo {:?}", clip.warp, c.doc.undo_label()))
        })),
        DoC("original BPM: clear the field, Enter", Box::new(|_, ctl| {
            let mut c = ctl.borrow_mut();
            c.ac_edit_start(crate::warpedit::AcEdit::OriginalBpm);
            c.ac_edit_commit("");
        })),
        Check("an empty BPM turns warp off (Clear original BPM)", Box::new(|c, _| {
            let clip = c.ac_clip().ok_or("no audio clip open")?;
            (!clip.warp.enabled && c.doc.undo_label() == Some("Clear original BPM")).then_some(()).ok_or(format!("warp {:?} undo {:?}", clip.warp, c.doc.undo_label()))
        })),
        DoC("transpose: type -7, then nonsense, then Escape, then Enter on an edit opened for another clip", Box::new(|_, ctl| {
            let mut c = ctl.borrow_mut();
            c.ac_edit_start(crate::warpedit::AcEdit::Transpose);
            c.ac_edit_commit("-7");
            c.ac_edit_start(crate::warpedit::AcEdit::Transpose);
            c.ac_edit_commit("abc");
            c.ac_edit_start(crate::warpedit::AcEdit::Transpose);
            c.ac_edit_cancel();
            c.ac_edit_start(crate::warpedit::AcEdit::Transpose);
            if let Some(e) = c.pr.ac_edit.as_mut() {
                e.1 = saine_core::ClipId(u64::MAX);
            }
            c.ac_edit_commit("5");
        })),
        Check("typed transpose -7 st; nonsense, Escape and another clip's edit change nothing", Box::new(|c, a| {
            let clip = c.ac_clip().ok_or("no audio clip open")?;
            let s = a.global::<Studio>();
            (clip.warp.transpose == -7 && s.get_ac_transpose_text() == "-7 st" && s.get_ac_edit_field() == -1 && c.doc.undo_label() == Some("Set clip pitch shift"))
                .then_some(())
                .ok_or(format!("transpose {} text {} field {} undo {:?}", clip.warp.transpose, s.get_ac_transpose_text(), s.get_ac_edit_field(), c.doc.undo_label()))
        })),
        DoC("typed values: undo three times", Box::new(|_, ctl| {
            let mut c = ctl.borrow_mut();
            for _ in 0..3 {
                c.undo();
            }
        })),
        Check("typed values undone", Box::new(|c, _| {
            let clip = c.ac_clip().ok_or("no audio clip open")?;
            clip.warp.is_default().then_some(()).ok_or(format!("warp {:?}", clip.warp))
        })),
        DoC("back to the Devices tab", Box::new(|_, ctl| ctl.borrow_mut().set_bottom_tab(BottomTab::Devices))),
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
        Do("(audio) unmute track 1 (muted by the redo step)", Box::new(|a| click(a, 262.65 + 33.5, TRACK0_Y + 47.0))),
        Do("(audio) click ruler at beat 9 (start of track 1 clip)", Box::new(move |a| click(a, bx(9.0), 112.0))),
        Do("(audio) master -40 dB, Space", Box::new(|a| {
            a.global::<Studio>().invoke_master_volume(crate::controller::slider_from_gain(0.01));
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
            (c.assistant_open && s.get_assistant_open() && s.get_chat_available() && (w - (1440.0 - 262.65 - 5.0 - 257.5 - 191.0)).abs() < 1.0)
                .then_some(())
                .ok_or(format!("open {} lanes width {w}", c.assistant_open))
        })),
        Check("recorded project state before the assistant turn", Box::new(move |c, _| {
            rt1.set(c.project().tracks.len());
            rg1.set(c.project().tracks[0].gain);
            Ok(())
        })),
        Do("click the input and type a prompt (Slint key events)", Box::new(|a| {
            click(a, CARD_X + 60.0, INPUT_Y);
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
        Do("click Point", Box::new(|a| click(a, CARD_X + 6.8 + 2.125 + 53.125 + 1.7 + 26.5, CARD_Y + 121.55 - 6.8 - 10.2))),
        Check("Point mode active", Box::new(move |c, a| {
            rr1.set(c.doc.revision());
            (c.assistant.mode == Mode::Teach && a.global::<Studio>().get_teach_mode()).then_some(()).ok_or("still Assist".into())
        })),
        Do("type + Enter (fixture turn with an edit)", Box::new(|a| {
            click(a, CARD_X + 60.0, INPUT_Y);
            type_text(a, "slow it down");
            key(a, Key::Return, false, false);
        })),
        Check("Point edit rejected: error entry, project unchanged", Box::new(move |c, _| {
            let e = c.assistant.last().ok_or("empty transcript")?;
            (e.kind == EntryKind::Error && e.text.contains("Point mode") && c.doc.revision() == rr2.get() && c.project().transport.bpm == 120.0)
                .then_some(())
                .ok_or(format!("{:?} {:?} rev {} vs {}", e.kind, e.text, c.doc.revision(), rr2.get()))
        })),
        Do("type + Enter (fixture turn highlighting transport.bpm)", Box::new(|a| {
            click(a, CARD_X + 60.0, INPUT_Y);
            type_text(a, "where is the tempo");
            key(a, Key::Return, false, false);
        })),
        Check("highlight active on transport.bpm, overlay rect mapped", Box::new(|c, a| {
            let hl = c.assistant.highlights.iter().any(|h| h.target == "transport.bpm");
            let rects = a.global::<Studio>().get_highlights();
            let r = slint::Model::row_data(&rects, 0);
            // Point ring around the tempo box (700, 42, 57 x 20): padded 6 px (CSS), radius 14
            let mapped = r.as_ref().is_some_and(|r| (r.x - 694.9).abs() < 0.01 && (r.y - 36.9).abs() < 0.01 && (r.r - 11.9).abs() < 0.01 && r.title == "Tempo (BPM)");
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
        Do("click the Instrument strip header", Box::new(|a| click(a, 5.0 + 108.0 * 0.85 + 40.0, MIXER_Y + 35.0))),
        Check("strip click selected track 2", Box::new(move |c, _| {
            rv1.set(c.doc.history_labels().iter().filter(|l| l.as_str() == "Track Volume Change").count());
            rm1.set(c.project().tracks[0].mute);
            (c.selected_track == Some(c.project().tracks[1].id)).then_some(()).ok_or("not selected".into())
        })),
        Do("drag the Audio strip fader from -24 dB to -36 dB", Box::new(|a| {
            let y = |f: f32| MIXER_Y + MIXER_TRAVEL_TOP + f * MIXER_TRAVEL;
            drag(a, 5.0 + 53.0 * 0.85, y(crate::controller::fader_from_db(-24.0)), 5.0 + 53.0 * 0.85, y(crate::controller::fader_from_db(-36.0)));
        })),
        Check("track 1 gain ≈ -36 dB, exactly one new undo step", Box::new(move |c, _| {
            let g = c.project().tracks[0].gain;
            let db = saine_core::gain_to_db(g);
            let n = c.doc.history_labels().iter().filter(|l| l.as_str() == "Track Volume Change").count();
            ((db + 36.0).abs() < 0.3 && n == rv2.get() + 1).then_some(()).ok_or(format!("{db} dB, volume steps {n} (was {})", rv2.get()))
        })),
        Do("click M on the Audio strip", Box::new(|a| click(a, 5.0 + 25.0 * 0.85, MIXER_Y + 390.0 - 17.0 * 0.85))),
        Check("track 1 mute toggled via the mixer (SetTrackMute)", Box::new(move |c, _| {
            (c.project().tracks[0].mute != rm2.get() && c.doc.undo_label() == Some("Mute Change")).then_some(()).ok_or(format!("mute {} undo {:?}", c.project().tracks[0].mute, c.doc.undo_label()))
        })),
        Do("drag the master strip fader to ~-24 dB", Box::new(|a| {
            let y = |f: f32| MIXER_Y + MIXER_TRAVEL_TOP + f * MIXER_TRAVEL;
            let x = 1440.0 - 5.0 - 108.0 * 0.85 + 53.0 * 0.85;
            drag(a, x, y(0.3), x, y(crate::controller::fader_from_db(-24.0)));
        })),
        Check("master gain ≈ -24 dB (SetMasterGain), mixer master fader follows", Box::new(|c, a| {
            let db = saine_core::gain_to_db(c.project().master_gain);
            let f = a.global::<Studio>().get_master_fader();
            ((db + 24.0).abs() < 0.3 && (f - 0.78).abs() < 0.01).then_some(()).ok_or(format!("{db} dB, fader {f}"))
        })),
        Do("click '+ Add track' in the mixer", Box::new(|a| click(a, 5.0 + 3.0 * 108.0 * 0.85 + 46.0, MIXER_Y + 210.0))),
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
    (262.65 + 60.5, TRACK0_Y + i as f32 * PITCH + 47.0 - sy)
}

/// Inspector (bottom panel 306 pt at y 509 with the Devices tab): R, I and the In field.
const INSP_R: (f32, f32) = (5.0 + 86.0 + 10.0, 509.0 + 275.0 + 9.0);
const INSP_I: (f32, f32) = (5.0 + 108.0 + 10.0, 509.0 + 275.0 + 9.0);
const INSP_IN: (f32, f32) = (5.0 + 80.0, 509.0 + 37.0 + 9.5);
/// "In" popup (window x 16, y 568): centre of row 0.
const IN_ROW0: (f32, f32) = (60.0, 568.0 + crate::recording::IN_PAD + crate::recording::IN_ROW / 2.0);
const RECORD_BTN: (f32, f32) = (423.0 + 14.0, 38.0 + 14.0);
/// Settings panel origin (the reference app's 960 x 820 pt window at x 255, y 6.5, in 85 % units) and width.
const SET_X: f32 = 216.75;
const SET_Y: f32 = 5.5;
const SET_W: f32 = 816.0;
/// Settings panel sidebar row centres.
fn settings_row(i: usize) -> (f32, f32) {
    (SET_X + 10.0 + 83.0, SET_Y + 94.75 + i as f32 * 27.25 + 12.75)
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
        Do("settings: click the Buffer size dropdown", Box::new(|a| click(a, SET_X + 187.0 + 300.0, SET_Y + 312.0 + 14.0))),
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
        Do("settings: Cmd+, again (Audio page)", Box::new(|a| key(a, ",", true, false))),
        Check("pdc: compensation on by default, latency readout shows the engine report", Box::new(|c, a| {
            let s = a.global::<Studio>();
            let txt = s.get_set_latency_text().to_string();
            let max = c.engine.latency_report().tracks.iter().map(|(_, l)| *l).max().unwrap_or(0);
            let ok_txt = if max == 0 { txt == "Plugin latency: none" } else { txt.contains(&format!("({max} samples)")) };
            (c.engine.delay_compensation_enabled() && s.get_set_pdc_on() && ok_txt).then_some(()).ok_or(format!("enabled {} ui {} text {txt:?} max {max}", c.engine.delay_compensation_enabled(), s.get_set_pdc_on()))
        })),
        Do("pdc: click the Plugin delay compensation switch", Box::new(|a| click(a, SET_X + SET_W - 41.0 + 10.25, SET_Y + 355.0 + 5.0))),
        Check("pdc: switched off in the engine, the UI and the (in-memory) preferences", Box::new(|c, a| {
            (!c.engine.delay_compensation_enabled() && !a.global::<Studio>().get_set_pdc_on() && c.settings.prefs.audio.pdc_off)
                .then_some(())
                .ok_or(format!("engine {} ui {} pref_off {}", c.engine.delay_compensation_enabled(), a.global::<Studio>().get_set_pdc_on(), c.settings.prefs.audio.pdc_off))
        })),
        Do("pdc: click the switch again, then Escape", Box::new(|a| {
            click(a, SET_X + SET_W - 41.0 + 10.25, SET_Y + 355.0 + 5.0);
            key(a, Key::Escape, false, false);
        })),
        Check("pdc: back on; Settings closed", Box::new(|c, a| {
            (c.engine.delay_compensation_enabled() && !c.settings.prefs.audio.pdc_off && !a.global::<Studio>().get_settings_open())
                .then_some(())
                .ok_or("pdc not restored or settings still open".into())
        })),
        DoC("pdc (offline): add the engine's test latency plugin (4800 frames) to track 1, play 1 s offline", Box::new(|_, ctl| {
            let mut c = ctl.borrow_mut();
            if c.engine.device_running() {
                return; // offline render is refused while the device owns the audio path
            }
            let track = c.project().tracks[0].id;
            let plugin = saine_core::PluginRef {
                format: saine_core::PluginFormat::Clap,
                id: "studio.saine.test.latency".into(),
                path: Some(saine_engine::test_plugin_dir().join("SaineTestLatency.clap").to_string_lossy().into_owned()),
                name: "Saine Test Latency".into(),
                vendor: "Saine".into(),
                version: "1.0.0".into(),
                kind: saine_core::PluginKind::Effect,
            };
            let mut dev = saine_core::Device::new(plugin);
            dev.state = Some(saine_core::device::b64::encode(&4800u32.to_le_bytes()));
            c.execute(saine_core::Command::AddDevice { track, index: None, device: dev });
            let e = &c.engine;
            let _ = e.seek(0);
            let _ = e.play();
            let mut buf = vec![0f32; 2 * 48_000];
            let _ = e.render_offline(&mut buf);
        })),
        Check("pdc (offline): L = 4800; audible position and Time readout are 100 ms behind the transport", Box::new(|c, a| {
            if c.engine.device_running() {
                return Ok(());
            }
            let l = c.engine.latency_report().total_frames;
            let pos = c.engine.transport().position;
            let aud = c.engine.audible_position();
            let t = a.global::<Studio>().get_time_text().to_string();
            (l == 4800 && pos == 48_000 && aud == 48_000 - 4800 && t == "00:00.900")
                .then_some(())
                .ok_or(format!("L {l} pos {pos} audible {aud} time {t:?} load errors {:?}", c.engine.plugin_load_errors()))
        })),
        DoC("pdc (offline): Enter (stop), Cmd+Z (remove the plugin)", Box::new(|a, ctl| {
            if ctl.borrow().engine.device_running() {
                return; // nothing was added in device mode; an undo here would revert an unrelated edit
            }
            key(a, Key::Return, false, false);
            key(a, "z", true, false);
        })),
        Check("pdc (offline): plugin removed by undo, no latency left", Box::new(|c, _| {
            if c.engine.device_running() {
                return Ok(());
            }
            let ok = c.project().tracks[0].devices.iter().all(|d| d.plugin.id != "studio.saine.test.latency");
            (ok && c.engine.latency_report().total_frames == 0).then_some(()).ok_or(format!("devices {:?} L {}", c.project().tracks[0].devices.len(), c.engine.latency_report().total_frames))
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

// ------------------------------------------------------------------ plugin devices

/// Index of the first track of `kind`.
fn track_of(c: &Controller, kind: TrackKind) -> Option<usize> {
    c.project().tracks.iter().position(|t| t.kind == kind)
}

/// Scroll the track list (wheel over the headers) so track `i` is fully visible, then click its header.
fn click_track_header(a: &AppWindow, ctl: &Ctl, i: usize) {
    let (sy, lanes_h) = {
        let c = ctl.borrow();
        (c.view.scroll_y as f32, c.lanes_size.1)
    };
    let y = TRACK0_Y + i as f32 * PITCH - sy;
    let bottom = 78.0 + 27.5 + lanes_h - 30.5;
    if y < TRACK0_Y || y + 74.0 > bottom {
        let target = i as f32 * PITCH;
        a.window().dispatch_event(WindowEvent::PointerMoved { position: LogicalPosition::new(300.0, 300.0) });
        a.window().dispatch_event(WindowEvent::PointerScrolled { position: LogicalPosition::new(300.0, 300.0), delta_x: 0.0, delta_y: sy - target });
        if let Ok(mut c) = ctl.try_borrow_mut() {
            c.update_ui(a);
        }
    }
    let sy = ctl.borrow().view.scroll_y as f32;
    click(a, 300.0, TRACK0_Y + i as f32 * PITCH + 10.0 - sy);
}

/// Window centre of browser tree row `i`.
fn tree_row_xy(a: &AppWindow, i: usize) -> (f32, f32) {
    let sc = a.global::<Studio>().get_tree_scroll();
    (5.0 + 110.0, crate::devices::TREE_Y + crate::devices::TREE_TOP + i as f32 * crate::devices::TREE_ROW + 10.5 - sc)
}

fn tree_labels(a: &AppWindow) -> Vec<(i32, String, String, bool)> {
    let m = a.global::<Studio>().get_tree();
    (0..slint::Model::row_count(&m)).filter_map(|i| slint::Model::row_data(&m, i)).map(|r| (r.depth, r.label.to_string(), r.count.to_string(), r.leaf)).collect()
}

fn tree_row_of(a: &AppWindow, label: &str) -> Option<usize> {
    tree_labels(a).iter().position(|r| r.1 == label && r.3)
}

fn cards(a: &AppWindow) -> Vec<crate::DeviceCard> {
    let m = a.global::<Studio>().get_device_cards();
    (0..slint::Model::row_count(&m)).filter_map(|i| slint::Model::row_data(&m, i)).collect()
}

fn card_named(a: &AppWindow, name: &str) -> Option<crate::DeviceCard> {
    cards(a).into_iter().find(|c| c.name == name)
}

/// Bottom panel (Devices tab, 306 pt) top in the window.
const BOTTOM_Y: f32 = 509.0;
/// Card-relative point -> window point.
fn card_xy(c: &crate::DeviceCard, x: f32, y: f32) -> (f32, f32) {
    (5.0 + c.x + x, BOTTOM_Y + 34.0 + y)
}

/// Centre of row `i` of the device popup list (same clamping as ui/app.slint).
fn devmenu_row(a: &AppWindow, i: usize) -> (f32, f32) {
    let s = a.global::<Studio>();
    let (w, h) = (s.get_devmenu_w(), s.get_devmenu_h());
    let x = s.get_devmenu_x().min(1440.0 - 5.0 - w).max(5.0);
    let y = s.get_devmenu_y().min(820.0 - 5.0 - h).max(5.0);
    (x + 40.0, y + crate::devices::MENU_PAD + i as f32 * crate::devices::MENU_ROW + 10.25)
}

fn devmenu_labels(a: &AppWindow) -> Vec<(String, String, bool)> {
    let m = a.global::<Studio>().get_devmenu_items();
    (0..slint::Model::row_count(&m)).filter_map(|i| slint::Model::row_data(&m, i)).map(|r| (r.label.to_string(), r.shortcut.to_string(), r.enabled)).collect()
}

fn track_devices(c: &Controller, kind: TrackKind) -> Vec<(u64, String, bool)> {
    track_of(c, kind).map(|i| c.project().tracks[i].devices.iter().map(|d| (d.id.0, d.plugin.name.clone(), d.enabled)).collect()).unwrap_or_default()
}

/// Replace the text of the focused-on-click search field (End, Backspace x n, type).
fn retype(a: &AppWindow, field: (f32, f32), text: &str) {
    click(a, field.0, field.1);
    key(a, Key::End, false, false);
    for _ in 0..24 {
        key(a, Key::Backspace, false, false);
    }
    type_text(a, text);
}

const BROWSER_SEARCH: (f32, f32) = (5.0 + 8.5 + 120.0, 78.0 + 34.0 + 10.0);
const EFFECTS_TAB: (f32, f32) = (5.0 + 86.0, 78.0 + 67.0);

/// Plugin devices (appended last). Checks marked [needs engine plugin …] depend on the
/// engine's plugin hosting; with the engine stub they FAIL (the catalogue is empty and no
/// instance exists), never pass by default.
fn device_steps() -> Vec<Step> {
    use Step::*;
    let hist = Rc::new(Cell::new(0usize));
    let h = || hist.clone();
    fn one(hc: &Rc<Cell<usize>>, c: &Controller, label: &str) -> Result<(), String> {
        let n = c.doc.history_labels().len();
        let was = hc.get();
        hc.set(n);
        (n == was + 1 && c.doc.undo_label() == Some(label)).then_some(()).ok_or(format!("history {was} -> {n}, last {:?} (want {label})", c.doc.undo_label()))
    }
    let removed = Rc::new(Cell::new(0u64));
    let (rm1, rm2) = (removed.clone(), removed.clone());
    // A lane of the removed AULowpass keeps its colour; its colour is never asked for before
    // the removal, so the name must have been recorded when the device was added.
    let rm3 = removed.clone();
    /// A plugin-parameter target on device `id` named "Cutoff" (colour key "AULowpass/Cutoff").
    fn lowpass_cutoff(id: u64) -> saine_core::AutomationTarget {
        saine_core::AutomationTarget::Plugin { device: saine_core::DeviceId(id), param: 0, name: "Cutoff".into() }
    }
    let scans = Rc::new(Cell::new(0usize));
    let (sc1, sc2) = (scans.clone(), scans.clone());
    // Editor step result: None = skipped (engine reports no editor), Some((opened, closed, error)).
    let editor: Rc<RefCell<Option<(bool, bool, String)>>> = Rc::new(RefCell::new(None));
    let (ed1, ed2) = (editor.clone(), editor.clone());
    let saved: Rc<RefCell<Vec<(u64, String, bool)>>> = Rc::new(RefCell::new(Vec::new()));
    let (sv1, sv2) = (saved.clone(), saved.clone());
    let save_dir = std::env::temp_dir().join(format!("saine-selftest-devices-{}.saineproj", std::process::id()));
    let (sd1, sd2, sd3) = (save_dir.clone(), save_dir.clone(), save_dir);
    let (h0, h1, h2, h3, h4, h5, h6, h7, h8, h9, h10, h11, h12) = (h(), h(), h(), h(), h(), h(), h(), h(), h(), h(), h(), h(), h());
    fn scan_state(c: &Controller) -> String {
        format!("{:?}", c.devices.catalog.state)
    }
    vec![
        Do("devices: Enter; Escape (close menus)", Box::new(|a| {
            key(a, Key::Return, false, false);
            key(a, Key::Escape, false, false);
        })),
        DoC("devices: select the first audio track, Devices tab, browser Effects sub-tab", Box::new(|a, ctl| {
            let i = track_of(&ctl.borrow(), TrackKind::Audio).unwrap_or(0);
            click_track_header(a, ctl, i);
            if ctl.borrow().bottom_tab == crate::controller::BottomTab::Clip {
                click(a, 5.0 + 286.0 + 15.0, 820.0 - 5.0 - 390.0 + 14.0);
            }
            click(a, EFFECTS_TAB.0, EFFECTS_TAB.1);
        })),
        Check("devices: Effects tab lists Audio Units with plugins (catalogue scanned) [needs engine plugin scanning]", Box::new(|c, a| {
            let rows = tree_labels(a);
            let au = rows.iter().find(|r| r.0 == 0 && r.1 == "Audio Units").map(|r| r.2.clone());
            let ok = a.global::<Studio>().get_browser_tab() == 1 && au.as_deref().and_then(|n| n.parse::<usize>().ok()).is_some_and(|n| n > 0);
            ok.then_some(()).ok_or(format!("tab {} Audio Units count {au:?}; scan {}", a.global::<Studio>().get_browser_tab(), scan_state(c)))
        })),
        Do("devices: type “AULowpass” into the browser search", Box::new(|a| retype(a, BROWSER_SEARCH, "AULowpass"))),
        Check("devices: search shows Audio Units › Apple › AULowpass; record history [needs engine plugin scanning]", Box::new(move |c, a| {
            h0.set(c.doc.history_labels().len());
            let rows = tree_labels(a);
            let names: Vec<&str> = rows.iter().map(|r| r.1.as_str()).collect();
            (names.first() == Some(&"Audio Units") && tree_row_of(a, "AULowpass").is_some() && a.global::<Studio>().get_browser_search() == "AULowpass")
                .then_some(())
                .ok_or(format!("rows {names:?}, search {:?}; scan {}", a.global::<Studio>().get_browser_search(), scan_state(c)))
        })),
        Do("devices: double-click AULowpass", Box::new(|a| {
            if let Some(i) = tree_row_of(a, "AULowpass") {
                let (x, y) = tree_row_xy(a, i);
                double_click(a, x, y);
            }
        })),
        Check("devices: AULowpass added to the audio track (AddDevice), one undo step, card shown [needs engine plugin scanning]", Box::new(move |c, a| {
            one(&h1, c, "Add AULowpass")?;
            let d = track_devices(c, TrackKind::Audio);
            (d.iter().map(|x| x.1.as_str()).collect::<Vec<_>>() == ["AULowpass"] && card_named(a, "AULowpass").is_some())
                .then_some(())
                .ok_or(format!("devices {d:?}"))
        })),
        Check("engine: AULowpass is instantiated (plugin_states lists it, no load error) [needs engine plugin hosting]", Box::new(|c, _| {
            let id = track_of(c, TrackKind::Audio).and_then(|i| c.project().tracks[i].devices.first().map(|d| d.id)).ok_or("no device")?;
            let states = c.engine.plugin_states();
            let errors = c.engine.plugin_load_errors();
            (states.iter().any(|(d, s)| *d == id && !s.is_empty()) && !errors.iter().any(|(d, _)| *d == id))
                .then_some(())
                .ok_or(format!("plugin_states {} entries (none for {}), load errors {errors:?}", states.len(), id.0))
        })),
        Do("devices: AULowpass power toggle off", Box::new(|a| {
            if let Some(cd) = card_named(a, "AULowpass") {
                let (x, y) = card_xy(&cd, 19.0, 13.25);
                click(a, x, y);
            }
        })),
        Check("devices: AULowpass bypassed (SetDeviceEnabled false), one undo step, toggle drawn off [needs engine plugin scanning]", Box::new(move |c, a| {
            one(&h2, c, "Bypass Device")?;
            let d = track_devices(c, TrackKind::Audio);
            (d.first().is_some_and(|x| !x.2) && card_named(a, "AULowpass").is_some_and(|cd| !cd.enabled)).then_some(()).ok_or(format!("devices {d:?}"))
        })),
        Do("devices: AULowpass power toggle on", Box::new(|a| {
            if let Some(cd) = card_named(a, "AULowpass") {
                let (x, y) = card_xy(&cd, 19.0, 13.25);
                click(a, x, y);
            }
        })),
        Check("devices: AULowpass enabled again, one undo step [needs engine plugin scanning]", Box::new(move |c, _| {
            one(&h3, c, "Enable Device")?;
            track_devices(c, TrackKind::Audio).first().is_some_and(|x| x.2).then_some(()).ok_or("still bypassed".into())
        })),
        DoC("devices: clear the search; Add device › Audio Units", Box::new(|a, ctl| {
            retype(a, BROWSER_SEARCH, "");
            key(a, Key::Escape, false, false);
            if let Ok(mut c) = ctl.try_borrow_mut() {
                c.update_ui(a);
            }
            let add_x = a.global::<Studio>().get_add_device_x();
            click(a, 5.0 + add_x + 60.0, BOTTOM_Y + 34.5 + 56.5 + 22.0 + 11.0);
        })),
        Check("devices: Audio Units list open with AUDelay [needs engine plugin scanning]", Box::new(|c, a| {
            let items = devmenu_labels(a);
            (a.global::<Studio>().get_devmenu_open() && items.iter().any(|r| r.0 == "AUDelay" && r.2))
                .then_some(())
                .ok_or(format!("open {} items {:?}; scan {}", a.global::<Studio>().get_devmenu_open(), items.iter().take(4).collect::<Vec<_>>(), scan_state(c)))
        })),
        Do("devices: pick AUDelay", Box::new(|a| {
            if let Some(i) = devmenu_labels(a).iter().position(|r| r.0 == "AUDelay") {
                let (x, y) = devmenu_row(a, i);
                click(a, x, y);
            } else {
                key(a, Key::Escape, false, false);
            }
        })),
        Check("devices: chain = AULowpass, AUDelay (AddDevice), one undo step, list closed [needs engine plugin scanning]", Box::new(move |c, a| {
            one(&h4, c, "Add AUDelay")?;
            let d = track_devices(c, TrackKind::Audio);
            (d.iter().map(|x| x.1.as_str()).collect::<Vec<_>>() == ["AULowpass", "AUDelay"] && !a.global::<Studio>().get_devmenu_open())
                .then_some(())
                .ok_or(format!("devices {d:?}"))
        })),
        Do("devices: drag the AUDelay card (header) before AULowpass", Box::new(|a| {
            if let (Some(lp), Some(dl)) = (card_named(a, "AULowpass"), card_named(a, "AUDelay")) {
                let (x0, y0) = card_xy(&dl, 140.0, 13.0);
                let (x1, _) = card_xy(&lp, 30.0, 13.0);
                drag(a, x0, y0, x1, y0);
            }
        })),
        Check("devices: reordered to AUDelay, AULowpass in one undo step (MoveDevice) [needs engine plugin scanning]", Box::new(move |c, _| {
            one(&h5, c, "Move Device")?;
            let d = track_devices(c, TrackKind::Audio);
            (d.iter().map(|x| x.1.as_str()).collect::<Vec<_>>() == ["AUDelay", "AULowpass"]).then_some(()).ok_or(format!("devices {d:?}"))
        })),
        DoC("devices: ✕ on the AULowpass card", Box::new(move |a, ctl| {
            rm1.set(track_devices(&ctl.borrow(), TrackKind::Audio).iter().find(|x| x.1 == "AULowpass").map(|x| x.0).unwrap_or(0));
            if let Some(cd) = card_named(a, "AULowpass") {
                let (x, y) = card_xy(&cd, crate::devices::CARD_W - 14.5, 13.5);
                click(a, x, y);
            }
        })),
        Check("devices: AULowpass removed (RemoveDevice), one undo step [needs engine plugin scanning]", Box::new(move |c, a| {
            one(&h6, c, "Remove Device")?;
            let d = track_devices(c, TrackKind::Audio);
            (d.iter().map(|x| x.1.as_str()).collect::<Vec<_>>() == ["AUDelay"] && card_named(a, "AULowpass").is_none()).then_some(()).ok_or(format!("devices {d:?}"))
        })),
        Check("automation: a removed plugin's lane keeps its colour (\"AULowpass/Cutoff\", not \"/Cutoff\") [needs engine plugin scanning]", Box::new(move |c, _| {
            let now = c.lane_hue(&lowpass_cutoff(rm3.get()));
            let want = crate::automation::Hue::key("AULowpass", "Cutoff");
            (rm3.get() != 0 && now == want).then_some(()).ok_or(format!("device {}: {now:?}, want {want:?}", rm3.get()))
        })),
        Do("devices: Cmd+Z", Box::new(|a| key(a, "z", true, false))),
        Check("devices: ⌘Z restored AULowpass with the same device id, after AUDelay [needs engine plugin scanning]", Box::new(move |c, _| {
            h7.set(c.doc.history_labels().len());
            let d = track_devices(c, TrackKind::Audio);
            (d.iter().map(|x| (x.0, x.1.as_str())).collect::<Vec<_>>().get(1) == Some(&(rm2.get(), "AULowpass")) && rm2.get() != 0)
                .then_some(())
                .ok_or(format!("devices {d:?}, removed id {}", rm2.get()))
        })),
        // Opens a real plugin window for an instant (it can take focus); runs only when the
        // engine reports the plugin has an editor.
        DoC("devices: open and close the AULowpass plugin window (only if the engine reports an editor)", Box::new(move |a, ctl| {
            let has_editor = {
                let c = ctl.borrow();
                c.devices.catalog.plugins.iter().any(|p| p.plugin.name == "AULowpass" && p.has_editor) && c.devices.catalog.fixture.is_none()
            };
            if !has_editor {
                *ed1.borrow_mut() = None;
                return;
            }
            let Some(cd) = card_named(a, "AULowpass") else { return };
            let id = saine_core::DeviceId(cd.id as u64);
            let (x, y) = card_xy(&cd, crate::devices::CARD_W - 53.0 * crate::device_panel::Z, 16.0 * crate::device_panel::Z);
            click(a, x, y);
            let (opened, err) = {
                let c = ctl.borrow();
                (c.engine.plugin_editor_is_open(id), a.global::<Studio>().get_status_text().to_string())
            };
            if let Ok(mut c) = ctl.try_borrow_mut() {
                c.update_ui(a);
            }
            if let Some(cd) = card_named(a, "AULowpass") {
                let (x, y) = card_xy(&cd, crate::devices::CARD_W - 53.0 * crate::device_panel::Z, 16.0 * crate::device_panel::Z);
                click(a, x, y);
            }
            let closed = !ctl.borrow().engine.plugin_editor_is_open(id);
            *ed1.borrow_mut() = Some((opened, closed, err));
        })),
        Check("devices: plugin window opened, then closed (plugin_editor_is_open toggled) [needs engine plugin editors]", Box::new(move |_, _| match &*ed2.borrow() {
            None => Err("SKIP: the engine reports no editor for AULowpass (no scan, fixture catalogue or has_editor false)".into()),
            Some((true, true, _)) => Ok(()),
            Some((o, cl, e)) => Err(format!("opened {o} closed {cl}; status {e:?}")),
        })),
        DoC("devices: select the Instrument track; open the Inst selector", Box::new(move |a, ctl| {
            let i = track_of(&ctl.borrow(), TrackKind::Instrument).unwrap_or(1);
            click_track_header(a, ctl, i);
            if let Ok(mut c) = ctl.try_borrow_mut() {
                c.update_ui(a);
            }
            click(a, INSP_IN.0, INSP_IN.1);
        })),
        Check("devices: Inst list shows Synth ✓ and DLSMusicDevice; record history [needs engine plugin scanning]", Box::new(move |c, a| {
            h8.set(c.doc.history_labels().len());
            let items = devmenu_labels(a);
            (items.first().is_some_and(|r| r.0 == "Synth" && r.1 == "✓") && items.iter().any(|r| r.0 == "DLSMusicDevice" && r.2))
                .then_some(())
                .ok_or(format!("items {:?}; scan {}", items.iter().take(5).collect::<Vec<_>>(), scan_state(c)))
        })),
        Do("devices: pick DLSMusicDevice", Box::new(|a| {
            if let Some(i) = devmenu_labels(a).iter().position(|r| r.0 == "DLSMusicDevice") {
                let (x, y) = devmenu_row(a, i);
                click(a, x, y);
            } else {
                key(a, Key::Escape, false, false);
            }
        })),
        Check("devices: DLSMusicDevice is the instrument (SetInstrumentPlugin), one undo step; its card replaces the Synth card [needs engine plugin scanning]", Box::new(move |c, a| {
            one(&h9, c, "Load DLSMusicDevice")?;
            let i = track_of(c, TrackKind::Instrument).ok_or("no instrument track")?;
            let ip = c.project().tracks[i].instrument_plugin.as_ref().map(|d| d.plugin.name.clone());
            let s = a.global::<Studio>();
            (ip.as_deref() == Some("DLSMusicDevice") && !s.get_synth_visible() && card_named(a, "DLSMusicDevice").is_some_and(|cd| cd.instrument) && s.get_sel_inst_text() == "DLSMusicDevice")
                .then_some(())
                .ok_or(format!("instrument {ip:?}, synth visible {}, inst text {}", s.get_synth_visible(), s.get_sel_inst_text()))
        })),
        Do("devices: Inst selector → Synth", Box::new(|a| {
            click(a, INSP_IN.0, INSP_IN.1);
            let (x, y) = devmenu_row(a, 0);
            click(a, x, y);
        })),
        Check("devices: back to the built-in Synth (a new Synth device), one undo step, Synth card shown [needs engine plugin scanning]", Box::new(move |c, a| {
            one(&h10, c, "Load Synth")?;
            let i = track_of(c, TrackKind::Instrument).ok_or("no instrument track")?;
            (c.project().tracks[i].instrument_plugin.as_ref().is_some_and(crate::devices::is_builtin_synth) && a.global::<Studio>().get_synth_visible()).then_some(()).ok_or("still a plugin instrument".into())
        })),
        DoC("devices: search “AUDelay”, drag it from the browser onto the Instrument track lane", Box::new(|a, ctl| {
            retype(a, BROWSER_SEARCH, "AUDelay");
            key(a, Key::Escape, false, false);
            if let Ok(mut c) = ctl.try_borrow_mut() {
                c.update_ui(a);
            }
            let Some(r) = tree_row_of(a, "AUDelay") else { return };
            let (x0, y0) = tree_row_xy(a, r);
            let (ti, sy) = {
                let c = ctl.borrow();
                (track_of(&c, TrackKind::Instrument).unwrap_or(1), c.view.scroll_y as f32)
            };
            drag(a, x0, y0, LANE_X + 300.0, TRACK0_Y + ti as f32 * PITCH + 37.0 - sy);
        })),
        Check("devices: AUDelay dropped on the Instrument track (AddDevice), one undo step [needs engine plugin scanning]", Box::new(move |c, a| {
            one(&h11, c, "Add AUDelay")?;
            let d = track_devices(c, TrackKind::Instrument);
            (d.iter().map(|x| x.1.as_str()).collect::<Vec<_>>() == ["AUDelay"] && !a.global::<Studio>().get_dnd_active()).then_some(()).ok_or(format!("instrument devices {d:?}"))
        })),
        DoC("devices: search “AULowpass”, drag it into the device chain before AUDelay", Box::new(|a, ctl| {
            retype(a, BROWSER_SEARCH, "AULowpass");
            key(a, Key::Escape, false, false);
            if let Ok(mut c) = ctl.try_borrow_mut() {
                c.update_ui(a);
            }
            let (Some(r), Some(dl)) = (tree_row_of(a, "AULowpass"), card_named(a, "AUDelay")) else { return };
            let (x0, y0) = tree_row_xy(a, r);
            let (x1, y1) = card_xy(&dl, 30.0, 120.0);
            drag(a, x0, y0, x1, y1);
        })),
        Check("devices: AULowpass dropped into the chain at index 0 (AddDevice), one undo step [needs engine plugin scanning]", Box::new(move |c, _| {
            one(&h12, c, "Add AULowpass")?;
            let d = track_devices(c, TrackKind::Instrument);
            (d.iter().map(|x| x.1.as_str()).collect::<Vec<_>>() == ["AULowpass", "AUDelay"]).then_some(()).ok_or(format!("instrument devices {d:?}"))
        })),
        DoC("devices: ⌘, then Settings › Plugins; record scans", Box::new(move |a, ctl| {
            sc1.set(ctl.borrow().devices.catalog.scans);
            key(a, ",", true, false);
            let (x, y) = settings_row(crate::settings::PLUGINS_PAGE);
            click(a, x, y);
        })),
        Check("settings: Plugins page shows scan directories, scan status and every catalogue entry", Box::new(|c, a| {
            let s = a.global::<Studio>();
            let rows = slint::Model::row_count(&s.get_plugin_rows());
            let dirs = slint::Model::row_count(&s.get_scan_dirs());
            (s.get_settings_open() && s.get_settings_page() == crate::settings::PLUGINS_PAGE as i32 && dirs >= 3 && rows == c.devices.catalog.plugins.len() && !s.get_plugins_status().is_empty())
                .then_some(())
                .ok_or(format!("page {} dirs {dirs} rows {rows} status {:?}", s.get_settings_page(), s.get_plugins_status()))
        })),
        Do("settings: click “Rescan all plugins”", Box::new(|a| {
            let dirs = slint::Model::row_count(&a.global::<Studio>().get_scan_dirs()).min(8) as f32; // list is at most 8 rows tall
            let content_w = SET_W - 187.0;
            let card_x = 20.5 + ((content_w - 41.0) / 2.0 + 4.5);
            let card_y = 91.0 + dirs * 30.5 + 86.0;
            click(a, SET_X + 187.0 + card_x + 10.5 + 30.0, SET_Y + card_y + 31.0 + 13.0);
        })),
        Check("settings: a forced rescan ran (scan_plugins with rescan = true) and finished", Box::new(move |c, _| {
            let st = &c.devices.catalog.state;
            (c.devices.catalog.scans == sc2.get() + 1 && c.devices.catalog.last_rescan && *st != crate::devices::ScanState::Scanning)
                .then_some(())
                .ok_or(format!("scans {} -> {}, rescan {}, state {st:?}", sc2.get(), c.devices.catalog.scans, c.devices.catalog.last_rescan))
        })),
        Do("settings: Escape", Box::new(|a| key(a, Key::Escape, false, false))),
        DoC("devices: ⌘S into a scratch project folder, then reopen it", Box::new(move |a, ctl| {
            {
                let mut c = ctl.borrow_mut();
                let mut all = track_devices(&c, TrackKind::Audio);
                all.extend(track_devices(&c, TrackKind::Instrument));
                *sv1.borrow_mut() = all;
                c.project_dir = Some(sd1.clone());
            }
            key(a, "s", true, false);
            ctl.borrow_mut().open(&sd1);
        })),
        Check("devices: reopened project has the same devices (ids, plugins, order, bypass) [needs engine plugin scanning]", Box::new(move |c, _| {
            let mut all = track_devices(c, TrackKind::Audio);
            all.extend(track_devices(c, TrackKind::Instrument));
            let want = sv2.borrow().clone();
            (!want.is_empty() && all == want && c.project_dir.as_deref() == Some(sd2.as_path())).then_some(()).ok_or(format!("reopened {all:?}, saved {want:?}"))
        })),
        Check("engine: the saved project carries each device's plugin state (plugin_states before save) [needs engine plugin hosting]", Box::new(move |_, _| {
            let l = saine_core::persist::load(&sd3).map_err(|e| e.to_string())?;
            let devs: Vec<(String, bool)> = l.project.tracks.iter().flat_map(|t| t.devices.iter()).map(|d| (d.plugin.name.clone(), d.state.as_ref().is_some_and(|s| !s.is_empty()))).collect();
            let _ = std::fs::remove_dir_all(&sd3);
            (!devs.is_empty() && devs.iter().all(|d| d.1)).then_some(()).ok_or(format!("devices with state: {devs:?}"))
        })),
    ]
}

// ------------------------------------------------------------------ automation

/// Lanes surface origin in the window (timeline x 262 + lane-x 191, y 78 + 27.5).
const LANES_ORIGIN: (f32, f32) = (453.65, 105.5);

fn auto_row(c: &Controller, track: usize, target: &saine_core::AutomationTarget) -> Option<crate::automation::Row> {
    let key = Some(c.project().tracks.get(track)?.id);
    c.auto_rows().into_iter().find(|r| r.key == key && r.target == *target)
}

/// Window point on an automation row at `beat` and lane value `v`.
fn auto_xy(c: &Controller, track: usize, target: &saine_core::AutomationTarget, beat: f64, v: f32) -> (f32, f32) {
    let Some(r) = auto_row(c, track, target) else { return (0.0, 0.0) };
    (LANES_ORIGIN.0 + c.view.x_of(beat), LANES_ORIGIN.1 + r.y_of(crate::automation::norm_of(target, v)))
}

fn lane_points(c: &Controller, track: usize, target: &saine_core::AutomationTarget) -> Vec<(i64, f32, f32)> {
    let Some(t) = c.project().tracks.get(track) else { return Vec::new() };
    c.model_lane(Some(t.id), target).map(|l| l.points.iter().map(|p| (p.time, p.value, p.curve)).collect()).unwrap_or_default()
}

/// Track header automation pill (track index `i`): window centre of the label and of the power icon.
fn pill_xy(c: &Controller, i: usize) -> ((f32, f32), (f32, f32)) {
    let y = TRACK0_Y + c.view.track_offset(i) - c.view.scroll_y as f32 + 49.0 + 8.25;
    ((262.65 + 42.0 + 70.0, y), (262.65 + 42.0 + 8.5, y))
}

fn with_shift(a: &AppWindow, f: impl FnOnce()) {
    a.window().dispatch_event(WindowEvent::KeyPressed { text: Key::Shift.into() });
    f();
    a.window().dispatch_event(WindowEvent::KeyReleased { text: Key::Shift.into() });
}

/// Advance the transport by `secs`: offline render (no device), else wall-clock time.
fn advance(ctl: &Ctl, secs: f32) {
    let c = ctl.borrow();
    if c.engine.device_running() {
        drop(c);
        std::thread::sleep(std::time::Duration::from_secs_f32(secs));
    } else {
        let mut buf = vec![0f32; 2 * (48_000.0 * secs) as usize];
        let _ = c.engine.render_offline(&mut buf);
    }
}

thread_local! {
    /// Undo-history length recorded by the automation checks.
    static AUTO_HIST: Cell<usize> = const { Cell::new(0) };
}
fn ah_get() -> usize {
    AUTO_HIST.with(|h| h.get())
}
fn ah_set(n: usize) {
    AUTO_HIST.with(|h| h.set(n))
}

fn automation_steps() -> Vec<Step> {
    use Step::*;
    use saine_core::AutomationTarget as AT;
    fn one(c: &Controller, label: &str) -> Result<(), String> {
        let n = c.doc.history_labels().len();
        let ok = n == ah_get() + 1 && c.doc.undo_label() == Some(label);
        let was = ah_get();
        ah_set(n);
        ok.then_some(()).ok_or(format!("history {was} -> {n}, last {:?} (want one {label:?})", c.doc.undo_label()))
    }
    let wav = std::env::temp_dir().join(format!("saine-selftest-auto-{}.wav", std::process::id()));
    let wav2 = wav.clone();
    vec![
        DoC("automation: fresh project, steady 440 Hz clip on Audio track 1 at 0", Box::new(move |a, ctl| {
            key(a, Key::Escape, false, false);
            let n = 3 * 48_000;
            let sine: Vec<f32> = (0..n).map(|i| 0.5 * (std::f32::consts::TAU * 440.0 * i as f32 / 48_000.0).sin()).collect();
            let _ = crate::wavgen::write_wav(&wav, 48_000, 1, &sine);
            let mut c = ctl.borrow_mut();
            let _ = c.engine.stop();
            c.mixer_open = false;
            c.assistant_open = false;
            c.set_bottom_tab(crate::controller::BottomTab::Devices);
            let mut p = saine_core::Project::default();
            if c.engine.device_running() {
                p.master_gain = db_to_gain(-40.0); // the write pass plays through the device
            }
            c.replace_project(p);
            c.view.start_beats = 0.0;
            c.view.scroll_y = 0.0;
            let t = c.project().tracks[0].id;
            c.select_track(t);
            c.import_files(vec![wav.clone()], crate::controller::ImportTarget::Selected, Some(0));
        })),
        Check("automation: clip imported, lanes hidden, timeline at full width", Box::new(|c, a| {
            let ok = c.project().tracks[0].clips.len() == 1 && !c.auto.shown && !a.global::<Studio>().get_auto_shown() && (a.get_lanes_width() - (1440.0 - 262.65 - 5.0 - 191.0)).abs() < 1.0;
            ok.then_some(()).ok_or(format!("clips {} shown {}", c.project().tracks[0].clips.len(), c.auto.shown))
        })),
        Do("automation: click “Show automation lanes” in the timeline header", Box::new(|a| click(a, 262.65 + (1440.0 - 262.65 - 5.0) - 296.9, 78.0 + 14.0))),
        Check("automation: lanes shown; each header has the Track Volume pill and the disclosure; master row 64.5 pt; value labels (tracks +0.0, master at its static level)", Box::new(move |c, a| {
            ah_set(c.doc.history_labels().len());
            let s = a.global::<Studio>();
            let rows = s.get_tracks();
            let r0 = slint::Model::row_data(&rows, 0).ok_or("no rows")?;
            let labels: Vec<String> = slint::Model::iter(&s.get_auto_labels()).map(|l| l.text.to_string()).collect();
            let ok = c.auto.shown && s.get_auto_shown() && r0.auto_cat == "Track" && r0.auto_param == "Volume" && r0.auto_count == 1 && (s.get_master_h() - (crate::timeline::MASTER_H_AUTO - 0.5)).abs() < 1e-3 && labels.len() == 3 && labels[..2] == ["+0.0", "+0.0"] && labels[2] == crate::automation::volume_label(saine_core::gain_to_db(c.project().master_gain));
            ok.then_some(()).ok_or(format!("shown {} pill {:?} {:?} count {} master {} labels {labels:?}", s.get_auto_shown(), r0.auto_cat, r0.auto_param, r0.auto_count, s.get_master_h()))
        })),
        DoC("automation: open the track 1 parameter picker, click “Track ›”, then “Pan”", Box::new(|a, ctl| {
            let (pill, _) = pill_xy(&ctl.borrow(), 0);
            click(a, pill.0, pill.1);
            let top = pill.1 + 8.25 + 1.5;
            click(a, 304.0 + 50.0, top + 30.5 + 10.5);
            click(a, 304.0 + 190.0 + 50.0, top + 30.5 - 4.0 + 4.0 + 21.0 + 10.25);
        })),
        Check("automation: picker closed, track 1 shows Track › Pan (UI state only, no undo step)", Box::new(move |c, a| {
            let s = a.global::<Studio>();
            let r0 = slint::Model::row_data(&s.get_tracks(), 0).ok_or("no rows")?;
            let ok = c.auto_target(Some(c.project().tracks[0].id)) == AT::Pan && !s.get_picker_open() && r0.auto_param == "Pan" && lane_points(c, 0, &AT::Pan).is_empty();
            ok.then_some(()).ok_or(format!("target {:?} picker {} pill {:?}", c.auto_target(Some(c.project().tracks[0].id)), s.get_picker_open(), r0.auto_param))
        })),
        Check("automation: record history", Box::new(move |c, _| {
            ah_set(c.doc.history_labels().len());
            Ok(())
        })),
        DoC("automation: click the Pan line at beat 4", Box::new(|a, ctl| {
            let (x, y) = auto_xy(&ctl.borrow(), 0, &AT::Pan, 4.0, 0.0);
            click(a, x, y);
        })),
        Check("automation: first point (beat 4, C) creates the lane, one undo step", Box::new({
            move |c, _| {
                one(c, "Edit Pan Automation")?;
                let p = lane_points(c, 0, &AT::Pan);
                (p == vec![(4 * PPQ, 0.0, 0.0)]).then_some(()).ok_or(format!("{p:?}"))
            }
        })),
        DoC("automation: click the Pan line at beat 8", Box::new(|a, ctl| {
            let (x, y) = auto_xy(&ctl.borrow(), 0, &AT::Pan, 8.0, 0.0);
            click(a, x, y);
        })),
        Check("automation: second point at beat 8, one undo step", Box::new({
            move |c, _| {
                one(c, "Edit Pan Automation")?;
                let p = lane_points(c, 0, &AT::Pan);
                (p == vec![(4 * PPQ, 0.0, 0.0), (8 * PPQ, 0.0, 0.0)]).then_some(()).ok_or(format!("{p:?}"))
            }
        })),
        DoC("automation: drag the beat-8 point to beat 10 and up 20 pt", Box::new(|a, ctl| {
            let (x, y) = auto_xy(&ctl.borrow(), 0, &AT::Pan, 8.0, 0.0);
            let x1 = LANES_ORIGIN.0 + ctl.borrow().view.x_of(10.0);
            drag(a, x, y, x1 + 3.0, y - 20.0);
        })),
        Check("automation: point moved to beat 10 (snapped) with a value right of centre, one undo step", Box::new({
            move |c, _| {
                one(c, "Edit Pan Automation")?;
                let p = lane_points(c, 0, &AT::Pan);
                (p.len() == 2 && p[1].0 == 10 * PPQ && p[1].1 > 0.4 && p[1].1 < 0.8).then_some(()).ok_or(format!("{p:?}"))
            }
        })),
        DoC("automation: Shift-drag the segment (beat 7) up 20 pt", Box::new(|a, ctl| {
            let (x, y) = {
                let c = ctl.borrow();
                let lane = c.model_lane(Some(c.project().tracks[0].id), &AT::Pan).cloned().unwrap();
                auto_xy(&c, 0, &AT::Pan, 7.0, lane.value_at(7 * PPQ).unwrap())
            };
            with_shift(a, || drag(a, x, y, x, y - 20.0));
        })),
        Check("automation: segment curve changed (point 1 curve < 0: faster rise), one undo step", Box::new({
            move |c, _| {
                one(c, "Edit Pan Automation")?;
                let p = lane_points(c, 0, &AT::Pan);
                (p.len() == 2 && p[0].2 < -0.2 && p[0].0 == 4 * PPQ).then_some(()).ok_or(format!("{p:?}"))
            }
        })),
        DoC("automation: click the beat-4 point (select) and press Delete", Box::new(|a, ctl| {
            let (x, y) = auto_xy(&ctl.borrow(), 0, &AT::Pan, 4.0, 0.0);
            click(a, x, y);
            key(a, "\u{7f}", false, false);
        })),
        Check("automation: point deleted (one step), track and clip untouched", Box::new({
            move |c, _| {
                one(c, "Edit Pan Automation")?;
                let p = lane_points(c, 0, &AT::Pan);
                (p.len() == 1 && p[0].0 == 10 * PPQ && c.project().tracks.len() == 2 && c.project().tracks[0].clips.len() == 1).then_some(()).ok_or(format!("{p:?}, {} tracks", c.project().tracks.len()))
            }
        })),
        Do("automation: Cmd+Z", Box::new(|a| key(a, "z", true, false))),
        Check("automation: undo restored the deleted point", Box::new(move |c, _| {
            ah_set(c.doc.history_labels().len());
            let p = lane_points(c, 0, &AT::Pan);
            (p.len() == 2 && p[0].0 == 4 * PPQ && p[0].2 < -0.2).then_some(()).ok_or(format!("{p:?}"))
        })),
        DoC("automation: click the pill's power icon", Box::new(|a, ctl| {
            let (_, power) = pill_xy(&ctl.borrow(), 0);
            click(a, power.0, power.1);
        })),
        Check("automation: lane disabled (enabled = false), points kept, pill state 2, one undo step", Box::new(move |c, a| {
            let t = c.project().tracks[0].id;
            let l = c.model_lane(Some(t), &AT::Pan).ok_or("no lane")?;
            let st = slint::Model::row_data(&a.global::<Studio>().get_tracks(), 0).map(|r| r.auto_state).unwrap_or(-1);
            let snap = c.current_snapshot();
            let in_snap = snap.tracks.iter().find(|s| s.track_id == t.0).map(|s| s.automation.len()).unwrap_or(9);
            one(c, "Edit Pan Automation")?;
            (!l.enabled && l.points.len() == 2 && st == 2 && in_snap == 0).then_some(()).ok_or(format!("enabled {} points {} state {st} snapshot lanes {in_snap}", l.enabled, l.points.len()))
        })),
        DoC("automation: pick Track › Volume for track 1 via the picker", Box::new(|a, ctl| {
            let (pill, _) = pill_xy(&ctl.borrow(), 0);
            click(a, pill.0, pill.1);
            let top = pill.1 + 8.25 + 1.5;
            click(a, 304.0 + 50.0, top + 30.5 + 10.5);
            click(a, 304.0 + 190.0 + 50.0, top + 30.5 - 4.0 + 4.0 + 10.25);
        })),
        Check("automation: track 1 shows Volume", Box::new(move |c, _| {
            ah_set(c.doc.history_labels().len());
            (c.auto_target(Some(c.project().tracks[0].id)) == AT::Volume).then_some(()).ok_or("not Volume".into())
        })),
        DoC("automation: Volume points at beats 4 and 12 (0 dB), then drag the second to the bottom (−∞)", Box::new(|a, ctl| {
            let p4 = auto_xy(&ctl.borrow(), 0, &AT::Volume, 4.0, 0.0);
            click(a, p4.0, p4.1);
            let p12 = auto_xy(&ctl.borrow(), 0, &AT::Volume, 12.0, 0.0);
            click(a, p12.0, p12.1);
            let bottom = auto_xy(&ctl.borrow(), 0, &AT::Volume, 12.0, -60.0);
            drag(a, p12.0, p12.1, p12.0, bottom.1 + 6.0);
        })),
        Check("automation: Volume value_at: 0 dB before beat 4, −30 dB at beat 8 (linear), −60 dB after beat 12; 3 steps", Box::new(move |c, _| {
            let t = c.project().tracks[0].id;
            let l = c.model_lane(Some(t), &AT::Volume).ok_or("no lane")?;
            let (a0, a8, a16) = (l.value_at(0).unwrap_or(9.0), l.value_at(8 * PPQ).unwrap_or(9.0), l.value_at(16 * PPQ).unwrap_or(9.0));
            let steps = c.doc.history_labels().len() - ah_get();
            ah_set(c.doc.history_labels().len());
            (a0 == 0.0 && (a8 + 30.0).abs() < 0.1 && a16 == -60.0 && steps == 3 && l.points.len() == 2).then_some(()).ok_or(format!("{a0} {a8} {a16}, {steps} steps, {:?}", l.points))
        })),
        Check("automation: header slider and inspector fader follow the automated value at the playhead (display only, gold)", Box::new(|c, a| {
            let s = a.global::<Studio>();
            let r0 = slint::Model::row_data(&s.get_tracks(), 0).ok_or("no rows")?;
            let at = c.project().frames_to_ticks(c.audible_frames(), c.rate);
            let want = c.model_lane(Some(c.project().tracks[0].id), &AT::Volume).and_then(|l| l.value_at(at)).ok_or("no value")?;
            let g = if want <= -60.0 { 0.0 } else { db_to_gain(want) };
            let pos = crate::controller::slider_from_gain(g);
            (r0.vol_auto && s.get_sel_vol_auto() && (r0.volume_pos - pos).abs() < 1e-3 && (c.project().tracks[0].gain - 1.0).abs() < 1e-6).then_some(()).ok_or(format!("auto {} pos {} want {pos}", r0.vol_auto, r0.volume_pos))
        })),
        DoC("automation: move the volume slider while not armed (automation is active)", Box::new(move |a, ctl| {
            ah_set(ctl.borrow().doc.history_labels().len());
            let y = TRACK0_Y + 44.0 - 13.0 + 4.0;
            drag(a, 262.65 + 115.6 + 10.0, y, 262.65 + 115.6 + 30.0, y);
        })),
        Check("automation: no edit, status hint “Volume is automated: arm automation to write, or disable the lane”", Box::new(move |c, a| {
            let st = a.global::<Studio>().get_status_text().to_string();
            let n = c.doc.history_labels().len();
            (n == ah_get() && (c.project().tracks[0].gain - 1.0).abs() < 1e-6 && st.contains("Volume is automated: arm automation to write, or disable the lane")).then_some(()).ok_or(format!("gain {} status {st:?}", c.project().tracks[0].gain))
        })),
        DoC("automation: double-click the beat-4 Volume point", Box::new(move |a, ctl| {
            ah_set(ctl.borrow().doc.history_labels().len());
            let p = auto_xy(&ctl.borrow(), 0, &AT::Volume, 4.0, 0.0);
            click(a, p.0, p.1);
            click(a, p.0, p.1);
        })),
        Check("automation: double-click removed the point (one step); Cmd+Z next restores it", Box::new(move |c, _| {
            let p = lane_points(c, 0, &AT::Volume);
            let n = c.doc.history_labels().len();
            (p.len() == 1 && n == ah_get() + 1 && c.doc.undo_label() == Some("Edit Volume Automation")).then_some(()).ok_or(format!("{p:?}"))
        })),
        Do("automation: Cmd+Z", Box::new(|a| key(a, "z", true, false))),
        Check("automation: point back", Box::new(|c, _| {
            let p = lane_points(c, 0, &AT::Volume);
            (p.len() == 2).then_some(()).ok_or(format!("{p:?}"))
        })),
        DoC("automation: Volume lane −60 → 0 dB over 1 s on track 1 (one SetAutomationLane)", Box::new(|_, ctl| {
            let mut c = ctl.borrow_mut();
            let t = c.project().tracks[0].id;
            let one_s = c.project().seconds_to_ticks(1.0);
            let lane = saine_core::AutomationLane {
                target: AT::Volume,
                enabled: true,
                points: vec![saine_core::AutoPoint { time: 0, value: -60.0, curve: 0.0 }, saine_core::AutoPoint { time: one_s, value: 0.0, curve: 0.0 }],
            };
            c.auto_commit(Some(t), lane);
        })),
        Check("automation: offline render of track 1 rises from silence to the clip level over 1 s [needs engine automation playback]", Box::new(|c, _| {
            if c.engine.device_running() {
                return Err("SKIP: offline render needs --no-audio".into());
            }
            let e = &c.engine;
            let t1 = c.project().tracks[0].id;
            let mut snap = c.current_snapshot();
            for t in &mut snap.tracks {
                t.mute = t.track_id != t1.0;
            }
            e.publish(&snap).map_err(|e| e.to_string())?;
            e.seek(0).map_err(|e| e.to_string())?;
            e.play().map_err(|e| e.to_string())?;
            let mut buf = vec![0f32; 2 * 48_000];
            let r = e.render_offline(&mut buf);
            let _ = e.stop();
            let _ = e.seek(0);
            e.publish(&c.current_snapshot()).map_err(|e| e.to_string())?;
            r.map_err(|e| e.to_string())?;
            let rms = |a: usize, b: usize| ((buf[2 * a..2 * b].iter().map(|x| (*x as f64).powi(2)).sum::<f64>()) / (2 * (b - a)) as f64).sqrt() as f32;
            let (early, mid, late) = (rms(0, 4_800), rms(21_600, 26_400), rms(43_200, 48_000));
            (late > 0.05 && early < late * 0.05 && mid > early && late > mid)
                .then_some(())
                .ok_or(format!("rms 0–0.1 s {early:.4}, 0.45–0.55 s {mid:.4}, 0.9–1.0 s {late:.4}: the level does not follow the lane; the engine does not apply automation yet (depends on the engine agent's automation playback)"))
        })),
        Do("automation: click Automation Arm in the transport", Box::new(|a| click(a, 496.0 + 10.5, 41.0 + 11.0))),
        Check("automation: armed (transport button on)", Box::new(move |c, a| {
            ah_set(c.doc.history_labels().len());
            (c.auto.armed && a.global::<Studio>().get_auto_armed()).then_some(()).ok_or("not armed".into())
        })),
        DoC("automation: play from 2 s; while playing, drag the track 1 volume slider (press, 4 moves 0.15 s apart, release)", Box::new(|a, ctl| {
            {
                let c = ctl.borrow();
                let f = c.project().ticks_to_frames(c.project().seconds_to_ticks(2.0), c.rate);
                let _ = c.engine.seek(f);
                let _ = c.engine.play();
            }
            advance(ctl, 0.1);
            let y = TRACK0_Y + 44.0 - 13.0 + 4.0;
            let x0 = 262.65 + 115.6 + 3.5;
            press(a, x0 + 50.0, y);
            for k in 1..=4 {
                advance(ctl, 0.15);
                a.window().dispatch_event(WindowEvent::PointerMoved { position: LogicalPosition::new(x0 + 50.0 - 9.0 * k as f32, y) });
            }
            advance(ctl, 0.1);
            release(a, x0 + 14.0, y);
            let _ = ctl.borrow().engine.stop();
        })),
        Check("automation: ONE “Write Automation” step; the Volume lane got the written points in the touched range, the curve outside kept", Box::new(move |c, _| {
            one(c, "Write Automation")?;
            let p = lane_points(c, 0, &AT::Volume);
            let tps = c.project().seconds_to_ticks(1.0);
            let inside: Vec<_> = p.iter().filter(|q| q.0 >= 2 * tps && q.0 <= 3 * tps).collect();
            let falling = inside.windows(2).any(|w| w[1].1 < w[0].1);
            (p.first().map(|q| (q.0, q.1)) == Some((0, -60.0)) && inside.len() >= 2 && falling && c.auto.writes.is_empty() && c.auto.pending.is_none())
                .then_some(())
                .ok_or(format!("points {p:?}"))
        })),
        Do("automation: click Automation Arm again (off), hide lanes", Box::new(|a| {
            click(a, 496.0 + 10.5, 41.0 + 11.0);
            click(a, 262.65 + (1440.0 - 262.65 - 5.0) - 296.9, 78.0 + 14.0);
        })),
        Check("automation: disarmed, lanes hidden, master row back to 30 pt", Box::new(move |c, a| {
            let _ = &wav2;
            let s = a.global::<Studio>();
            (!c.auto.armed && !c.auto.shown && (s.get_master_h() - (crate::timeline::MASTER_H - 0.5)).abs() < 1e-3).then_some(()).ok_or(format!("armed {} shown {} master {}", c.auto.armed, c.auto.shown, s.get_master_h()))
        })),
    ]
}

// ------------------------------------------------------------------ built-in devices

fn builtin_device(c: &Controller) -> Option<&saine_core::Device> {
    let t = track_of(c, TrackKind::Audio)?;
    c.project().tracks[t].devices.iter().find(|d| d.plugin.format == saine_core::PluginFormat::Builtin)
}

fn builtin_value(c: &Controller, param: &str) -> Option<f32> {
    let d = builtin_device(c)?;
    let def = saine_core::builtin::def_of(d)?;
    Some(def.value(d, def.param(param)?))
}

/// Window centre of knob `i` of a built-in card (ui/device_panel.slint KnobCell).
fn panel_knob_xy(cd: &crate::DeviceCard, i: usize) -> Option<(f32, f32)> {
    let k = slint::Model::row_data(&cd.knobs, i)?;
    let z = crate::device_panel::Z;
    Some(card_xy(cd, k.x + 31.0 * z, k.y + 16.0 * z))
}

fn builtin_of<'a>(c: &'a Controller, id: &str) -> Option<&'a saine_core::Device> {
    let t = track_of(c, TrackKind::Audio)?;
    c.project().tracks[t].devices.iter().find(|d| saine_core::builtin::def_of(d).is_some_and(|def| def.id == id))
}

/// Each other built-in: add it from the Built-in list, check its card, defaults and engine
/// state, drag its first knob (one undo step), then undo back to the empty chain.
fn builtin_each_steps() -> Vec<Step> {
    use Step::*;
    let mut steps: Vec<Step> = vec![Do("builtins: Escape (close menus)", Box::new(|a| key(a, Key::Escape, false, false)))];
    for def in saine_core::builtin::all().iter().filter(|d| ["distortion", "tapesaturation", "soft-clipper", "bitcrusher", "widener", "utility"].contains(&d.id)) {
        let (id, name) = (def.id, def.name);
        let base = Rc::new(Cell::new(0usize));
        let before = Rc::new(Cell::new(0.0f32));
        let (b0, b1, b2, b3) = (base.clone(), base.clone(), base.clone(), base);
        let (v0, v1) = (before.clone(), before);
        steps.push(DoC(format!("{name}: Add device › Built-in › {name}").leak(), Box::new(move |a, ctl| {
            b0.set(ctl.borrow().doc.history_labels().len());
            if let Ok(mut c) = ctl.try_borrow_mut() {
                c.update_ui(a);
            }
            let add_x = a.global::<Studio>().get_add_device_x();
            click(a, 5.0 + add_x + 60.0, BOTTOM_Y + 34.5 + 56.5 + 11.0);
            if let Ok(mut c) = ctl.try_borrow_mut() {
                c.update_ui(a);
            }
            if let Some(i) = devmenu_labels(a).iter().position(|r| r.0 == name) {
                let (x, y) = devmenu_row(a, i);
                click(a, x, y);
            }
        })));
        steps.push(Check(format!("{name}: added at its defaults in one undo step; card drawn; engine live").leak(), Box::new(move |c, a| {
            let n = c.doc.history_labels().len();
            let label = format!("Add {name}");
            if n != b1.get() + 1 || c.doc.undo_label() != Some(label.as_str()) {
                return Err(format!("history {} -> {n}, last {:?}", b1.get(), c.doc.undo_label()));
            }
            let d = builtin_of(c, id).ok_or("no device")?;
            let def = saine_core::builtin::def_of(d).ok_or("no def")?;
            if let Some(p) = def.params.iter().find(|p| def.value(d, p) != p.default) {
                return Err(format!("{} = {} (default {})", p.id, def.value(d, p), p.default));
            }
            let cd = card_named(a, name).ok_or("no card")?;
            if !cd.builtin || slint::Model::row_count(&cd.knobs) == 0 {
                return Err(format!("builtin {} knobs {}", cd.builtin, slint::Model::row_count(&cd.knobs)));
            }
            let errors = c.engine.plugin_load_errors();
            (!errors.iter().any(|(e, _)| *e == d.id)).then_some(()).ok_or(format!("load errors {errors:?}"))
        })));
        steps.push(DoC(format!("{name}: drag the first knob 40 px").leak(), Box::new(move |a, ctl| {
            let Some(cd) = card_named(a, name) else { return };
            let Some(k) = slint::Model::row_data(&cd.knobs, 0) else { return };
            {
                let c = ctl.borrow();
                let Some(d) = builtin_of(&c, id) else { return };
                let def = saine_core::builtin::def_of(d).unwrap();
                v0.set(def.value(d, &def.params[k.param as usize]));
            }
            if let Some((x, y)) = panel_knob_xy(&cd, 0) {
                drag(a, x, y, x, if k.norm > 0.5 { y + 40.0 } else { y - 40.0 });
            }
        })));
        steps.push(Check(format!("{name}: the drag changed the first knob's parameter in ONE undo step").leak(), Box::new(move |c, a| {
            let n = c.doc.history_labels().len();
            if n != b2.get() + 2 || c.doc.undo_label() != Some("Device Parameter Change") {
                return Err(format!("history {} -> {n}, last {:?}", b2.get() + 1, c.doc.undo_label()));
            }
            let cd = card_named(a, name).ok_or("no card")?;
            let k = slint::Model::row_data(&cd.knobs, 0).ok_or("no knob")?;
            let d = builtin_of(c, id).ok_or("no device")?;
            let def = saine_core::builtin::def_of(d).ok_or("no def")?;
            let v = def.value(d, &def.params[k.param as usize]);
            (v != v1.get()).then_some(()).ok_or(format!("value stayed {v}"))
        })));
        steps.push(Do(format!("{name}: ⌘Z twice").leak(), Box::new(|a| {
            key(a, "z", true, false);
            key(a, "z", true, false);
        })));
        steps.push(Check(format!("{name}: undo removes the device").leak(), Box::new(move |c, _| {
            let n = c.doc.history_labels().len();
            (builtin_of(c, id).is_none() && n == b3.get()).then_some(()).ok_or(format!("history {n}, want {}", b3.get()))
        })));
    }
    steps
}

/// The built-in Compressor driven through its generated panel (no plugins needed).
fn builtin_device_steps() -> Vec<Step> {
    use Step::*;
    let hist = Rc::new(Cell::new(0usize));
    let h = || hist.clone();
    fn one(hc: &Rc<Cell<usize>>, c: &Controller, label: &str) -> Result<(), String> {
        let n = c.doc.history_labels().len();
        let was = hc.get();
        hc.set(n);
        (n == was + 1 && c.doc.undo_label() == Some(label)).then_some(()).ok_or(format!("history {was} -> {n}, last {:?} (want {label})", c.doc.undo_label()))
    }
    let (h0, h1, h2, h3, h4, h5, h6, h7, h8) = (h(), h(), h(), h(), h(), h(), h(), h(), h());
    vec![
        Do("builtin: Escape (close menus)", Box::new(|a| key(a, Key::Escape, false, false))),
        DoC("builtin: select the first audio track, Devices tab, Add device › Built-in", Box::new(move |a, ctl| {
            let i = track_of(&ctl.borrow(), TrackKind::Audio).unwrap_or(0);
            click_track_header(a, ctl, i);
            if ctl.borrow().bottom_tab == crate::controller::BottomTab::Clip {
                click(a, 5.0 + 286.0 + 15.0, 820.0 - 5.0 - 390.0 + 14.0);
            }
            h0.set(ctl.borrow().doc.history_labels().len());
            if let Ok(mut c) = ctl.try_borrow_mut() {
                c.update_ui(a);
            }
            let add_x = a.global::<Studio>().get_add_device_x();
            click(a, 5.0 + add_x + 60.0, BOTTOM_Y + 34.5 + 56.5 + 11.0);
        })),
        Check("builtin: the Built-in list offers Compressor", Box::new(|_, a| {
            let items = devmenu_labels(a);
            (a.global::<Studio>().get_devmenu_open() && items.iter().any(|r| r.0 == "Compressor" && r.2)).then_some(()).ok_or(format!("items {items:?}"))
        })),
        Do("builtin: pick Compressor", Box::new(|a| {
            if let Some(i) = devmenu_labels(a).iter().position(|r| r.0 == "Compressor") {
                let (x, y) = devmenu_row(a, i);
                click(a, x, y);
            }
        })),
        Check("builtin: Compressor added (AddDevice) at its defaults, one undo step; generated card has 7 knobs, 1 slider, a strip", Box::new(move |c, a| {
            one(&h1, c, "Add Compressor")?;
            let cd = card_named(a, "Compressor").ok_or("no card")?;
            let n = |m: usize| m;
            let ok = cd.builtin
                && n(slint::Model::row_count(&cd.knobs)) == 7
                && slint::Model::row_count(&cd.sliders) == 1
                && cd.strip.visible
                && approx(cd.width, 521.0 * crate::device_panel::Z, 0.01)
                && builtin_value(c, "threshold") == Some(-24.0);
            ok.then_some(()).ok_or(format!("builtin {} knobs {} width {}", cd.builtin, slint::Model::row_count(&cd.knobs), cd.width))
        })),
        Check("engine: the Compressor is live (readouts available, no load error)", Box::new(|c, _| {
            let id = builtin_device(c).ok_or("no device")?.id;
            let mut r = Vec::new();
            c.engine.builtin_readouts(id.0, &mut r).map_err(|e| e.to_string())?;
            let errors = c.engine.plugin_load_errors();
            (r.len() == 3 && !errors.iter().any(|(d, _)| *d == id)).then_some(()).ok_or(format!("readouts {r:?} errors {errors:?}"))
        })),
        DoC("builtin: drag the Threshold knob up 40 px", Box::new(|a, _| {
            if let Some((x, y)) = card_named(a, "Compressor").and_then(|cd| panel_knob_xy(&cd, 0)) {
                drag(a, x, y, x, y - 40.0);
            }
        })),
        Check("builtin: Threshold −24 → −9 dB (40 px at 1/160 per px), ONE undo step for the drag", Box::new(move |c, a| {
            one(&h2, c, "Device Parameter Change")?;
            let v = builtin_value(c, "threshold").unwrap_or(f32::NAN);
            let shown = card_named(a, "Compressor").and_then(|cd| slint::Model::row_data(&cd.knobs, 0)).map(|k| k.value.to_string()).unwrap_or_default();
            (approx(v, -9.0, 0.05) && shown == "\u{2212}9.0 dB").then_some(()).ok_or(format!("threshold {v}, knob shows {shown:?}"))
        })),
        DoC("builtin: double-click the Threshold knob", Box::new(|a, _| {
            if let Some((x, y)) = card_named(a, "Compressor").and_then(|cd| panel_knob_xy(&cd, 0)) {
                double_click(a, x, y);
            }
        })),
        Check("builtin: double-click reset Threshold to −24 dB, one undo step", Box::new(move |c, _| {
            one(&h3, c, "Device Parameter Change")?;
            let v = builtin_value(c, "threshold").unwrap_or(f32::NAN);
            (v == -24.0).then_some(()).ok_or(format!("threshold {v}"))
        })),
        DoC("builtin: click › (next preset)", Box::new(|a, _| {
            if let Some(cd) = card_named(a, "Compressor") {
                let z = crate::device_panel::Z;
                // Header: next-preset arrow centre (fold, window, rule and nav from the right edge).
                let (x, y) = card_xy(&cd, cd.width - 71.25 * z, 16.0 * z);
                click(a, x, y);
            }
        })),
        Check("builtin: first factory preset loaded (SetDeviceParams), one undo step, header names it", Box::new(move |c, a| {
            let def = saine_core::builtin::find("compressor").unwrap();
            let first = def.factory[0];
            one(&h4, c, &format!("Load Preset {}", first.name))?;
            let want = def.factory_values(&first);
            let d = builtin_device(c).ok_or("no device")?;
            let shown = card_named(a, "Compressor").map(|cd| cd.preset.to_string()).unwrap_or_default();
            (def.params.iter().all(|p| def.value(d, p) == want[p.id]) && shown == first.name).then_some(()).ok_or(format!("preset {:?}, header {shown:?}", d.preset))
        })),
        DoC("builtin: Sidechain menu, pick the Instrument track", Box::new(|a, ctl| {
            let (id, name) = {
                let c = ctl.borrow();
                let Some(d) = builtin_device(&c) else { return };
                let Some(t) = track_of(&c, TrackKind::Instrument) else { return };
                (d.id.0 as i32, c.project().tracks[t].name.clone())
            };
            a.global::<Studio>().invoke_panel_menu(id, -1, 600.0, 600.0);
            if let Ok(mut c) = ctl.try_borrow_mut() {
                c.update_ui(a);
            }
            if let Some(i) = devmenu_labels(a).iter().position(|r| r.0 == name) {
                let (x, y) = devmenu_row(a, i);
                click(a, x, y);
            }
        })),
        Check("builtin: sidechain source set and enabled in one undo step; the Gain pill is live", Box::new(move |c, a| {
            one(&h5, c, "Sidechain Change")?;
            let d = builtin_device(c).ok_or("no device")?;
            let inst = track_of(c, TrackKind::Instrument).map(|t| c.project().tracks[t].id);
            let pill = card_named(a, "Compressor").and_then(|cd| (0..slint::Model::row_count(&cd.picks)).filter_map(|i| slint::Model::row_data(&cd.picks, i)).find(|p| p.kind == 3));
            (d.sidechain == inst && builtin_value(c, "sidechainEnable") == Some(1.0) && pill.is_some_and(|p| p.enabled))
                .then_some(())
                .ok_or(format!("sidechain {:?} enable {:?}", d.sidechain, builtin_value(c, "sidechainEnable")))
        })),
        Do("builtin: ⌘Z", Box::new(|a| key(a, "z", true, false))),
        Check("builtin: undo removes the sidechain (source and enable)", Box::new(move |c, _| {
            let d = builtin_device(c).ok_or("no device")?;
            h6.set(c.doc.history_labels().len());
            (d.sidechain.is_none() && builtin_value(c, "sidechainEnable") == Some(0.0)).then_some(()).ok_or(format!("sidechain {:?}", d.sidechain))
        })),
        DoC("builtin: pick the Instrument track as sidechain, then load the second factory preset", Box::new(|_, ctl| {
            let mut c = ctl.borrow_mut();
            let (Some(d), Some(t)) = (builtin_device(&c).map(|d| d.id), track_of(&c, TrackKind::Instrument)) else { return };
            let src = c.project().tracks[t].id;
            c.panel_set_sidechain(d, Some(src));
            c.panel_load_preset(d, 1);
        })),
        Check("builtin: a preset keeps the sidechain source and its enable switch, and is not marked edited", Box::new(|c, a| {
            let d = builtin_device(c).ok_or("no device")?;
            let shown = card_named(a, "Compressor").map(|cd| cd.preset.to_string()).unwrap_or_default();
            (d.sidechain.is_some() && builtin_value(c, "sidechainEnable") == Some(1.0) && !shown.is_empty() && !shown.ends_with('*'))
                .then_some(())
                .ok_or(format!("sidechain {:?} enable {:?} header {shown:?}", d.sidechain, builtin_value(c, "sidechainEnable")))
        })),
        DoC("builtin: reset Threshold, then drag it up 40 px as forty 1 px events", Box::new(|_, ctl| {
            let mut c = ctl.borrow_mut();
            let Some(d) = builtin_device(&c).map(|d| d.id) else { return };
            c.panel_reset(d, 0);
            c.panel_drag(d, 0, 0, 0.0, 1.0 / 160.0, false);
            for _ in 0..40 {
                c.panel_drag(d, 0, 1, 1.0, 1.0 / 160.0, false);
            }
            c.panel_drag(d, 0, 2, 0.0, 1.0 / 160.0, false);
        })),
        Check("builtin: small drag events add up like one large event (−24 → −9 dB)", Box::new(|c, _| {
            let v = builtin_value(c, "threshold").unwrap_or(f32::NAN);
            approx(v, -9.0, 0.05).then_some(()).ok_or(format!("threshold {v}"))
        })),
        DoC("builtin: drag Threshold 20 px, press Escape, release", Box::new(move |a, ctl| {
            h7.set(ctl.borrow().doc.history_labels().len());
            let Some(d) = builtin_device(&ctl.borrow()).map(|d| d.id) else { return };
            ctl.borrow_mut().panel_drag(d, 0, 0, 0.0, 1.0 / 160.0, false);
            ctl.borrow_mut().panel_drag(d, 0, 1, 20.0, 1.0 / 160.0, false);
            key(a, Key::Escape, false, false);
            ctl.borrow_mut().panel_drag(d, 0, 2, 0.0, 1.0 / 160.0, false);
        })),
        Check("builtin: Escape cancels the drag (Threshold stays −9 dB, no undo step)", Box::new(move |c, _| {
            let v = builtin_value(c, "threshold").unwrap_or(f32::NAN);
            let n = c.doc.history_labels().len();
            (approx(v, -9.0, 0.05) && n == h8.get()).then_some(()).ok_or(format!("threshold {v}, history {} -> {n}", h8.get()))
        })),
    ]
}

fn analyzer_device<'a>(c: &'a Controller, id: &str) -> Option<&'a saine_core::Device> {
    let t = track_of(c, TrackKind::Audio)?;
    c.project().tracks[t].devices.iter().find(|d| d.plugin.format == saine_core::PluginFormat::Builtin && d.plugin.id == id)
}

/// Pitch Corrector, Tuner and Spectrum: added from the Built-in list's
/// definitions, generated cards with their analyzer fields, live engine
/// instances (readouts and analysis frames), latency reported for PDC, and
/// Key Follow (picking a Key while following the project turns Follow off
/// in one undo step).
fn analyzer_device_steps() -> Vec<Step> {
    use Step::*;
    let hist = Rc::new(Cell::new(0usize));
    let (h0, h1) = (hist.clone(), hist.clone());
    vec![
        DoC("analyzers: select the audio track, clear its chain, add Pitch Corrector, Tuner and Spectrum", Box::new(move |a, ctl| {
            key(a, Key::Escape, false, false);
            let Some(i) = track_of(&ctl.borrow(), TrackKind::Audio) else { return };
            click_track_header(a, ctl, i);
            let mut c = ctl.borrow_mut();
            let t = c.project().tracks[i].id;
            let ids: Vec<saine_core::DeviceId> = c.project().tracks[i].devices.iter().map(|d| d.id).collect();
            for d in ids {
                c.execute(saine_core::Command::RemoveDevice { device: d });
            }
            for id in ["pitch-corrector", "tuner", "spectrum"] {
                if let Some(def) = saine_core::builtin::find(id) {
                    c.add_item_to_track(&crate::devices::Item::Builtin(def), t, None);
                }
            }
            c.update_ui(a);
        })),
        Check("analyzers: three cards with their canvases (Pitch trace, tuner, Spectrum) at the reference widths", Box::new(|_, a| {
            let z = crate::device_panel::Z;
            let want = [("Pitch Corrector", 375.0, "Pitch"), ("Tuner", 335.0, ""), ("Spectrum", 397.0, "Spectrum")];
            for (name, w, caption) in want {
                let cd = card_named(a, name).ok_or(format!("no {name} card"))?;
                if !(cd.builtin && cd.canvas.visible && cd.canvas.caption == caption && approx(cd.width, w * z, 0.01)) {
                    return Err(format!("{name}: builtin {} field {} caption {:?} width {}", cd.builtin, cd.canvas.visible, cd.canvas.caption, cd.width));
                }
            }
            Ok(())
        })),
        Check("engine: the three are live (readouts, analysis frames, no load error); Pitch Corrector reports 1024 frames latency", Box::new(|c, _| {
            let errors = c.engine.plugin_load_errors();
            for id in ["pitch-corrector", "tuner", "spectrum"] {
                let d = analyzer_device(c, id).ok_or(format!("no {id}"))?;
                let (mut r, mut f) = (Vec::new(), Vec::new());
                c.engine.builtin_readouts(d.id.0, &mut r).map_err(|e| format!("{id} readouts: {e}"))?;
                c.engine.builtin_analysis(d.id.0, &mut f).map_err(|e| format!("{id} analysis: {e}"))?;
                if errors.iter().any(|(e, _)| *e == d.id) {
                    return Err(format!("{id}: load error {errors:?}"));
                }
            }
            let lat = analyzer_device(c, "pitch-corrector").map_or(0, |d| c.engine.plugin_latency(d.id));
            (lat == 1024).then_some(()).ok_or(format!("Pitch Corrector latency {lat} (want 1024 at 48 kHz)"))
        })),
        DoC("analyzers: with no project key, pick Key E on the Pitch Corrector", Box::new(|a, ctl| {
            let mut c = ctl.borrow_mut();
            let Some(d) = analyzer_device(&c, "pitch-corrector").map(|d| d.id) else { return };
            let key = saine_core::builtin::find("pitch-corrector").and_then(|def| def.index_of("key")).unwrap_or(0);
            c.panel_choice(d, key as i32, 4);
            c.update_ui(a);
        })),
        Check("analyzers: nothing is followed without a project key: Key E, Follow stays Project", Box::new(|c, _| {
            let d = analyzer_device(c, "pitch-corrector").ok_or("no device")?;
            let def = saine_core::builtin::def_of(d).ok_or("no def")?;
            let v = |id: &str| def.param(id).map(|p| def.value(d, p));
            (v("key") == Some(4.0) && v("key_follow") == Some(1.0)).then_some(()).ok_or(format!("key {:?} follow {:?}", v("key"), v("key_follow")))
        })),
        DoC("analyzers: set the project key to A minor, then pick Key D on the following Pitch Corrector", Box::new(move |a, ctl| {
            let mut c = ctl.borrow_mut();
            c.execute(saine_core::Command::SetProjectKey { key: Some(saine_core::ProjectKey { root: 9, minor: true }) });
            h0.set(c.doc.history_labels().len());
            let Some(d) = analyzer_device(&c, "pitch-corrector").map(|d| d.id) else { return };
            let key = saine_core::builtin::find("pitch-corrector").and_then(|def| def.index_of("key")).unwrap_or(0);
            c.panel_choice(d, key as i32, 2);
            c.update_ui(a);
        })),
        Check("analyzers: Follow → Manual, Key D, Scale Minor (the shown project scale) in ONE undo step", Box::new(move |c, _| {
            let d = analyzer_device(c, "pitch-corrector").ok_or("no device")?;
            let def = saine_core::builtin::def_of(d).ok_or("no def")?;
            let v = |id: &str| def.param(id).map(|p| def.value(d, p));
            let n = c.doc.history_labels().len();
            (n == h1.get() + 1 && v("key_follow") == Some(0.0) && v("key") == Some(2.0) && v("scale") == Some(2.0))
                .then_some(())
                .ok_or(format!("history {} -> {n}, follow {:?} key {:?} scale {:?}", h1.get(), v("key_follow"), v("key"), v("scale")))
        })),
        DoC("analyzers: undo", Box::new(|a, ctl| {
            let mut c = ctl.borrow_mut();
            c.undo();
            c.update_ui(a);
        })),
        Check("analyzers: undo restores Follow = Project", Box::new(|c, _| {
            let d = analyzer_device(c, "pitch-corrector").ok_or("no device")?;
            let def = saine_core::builtin::def_of(d).ok_or("no def")?;
            let f = def.param("key_follow").map(|p| def.value(d, p));
            (f == Some(1.0)).then_some(()).ok_or(format!("follow {f:?}"))
        })),
        DoC("analyzers: remove the three devices and the project key", Box::new(|a, ctl| {
            let mut c = ctl.borrow_mut();
            c.execute(saine_core::Command::SetProjectKey { key: None });
            for id in ["pitch-corrector", "tuner", "spectrum"] {
                if let Some(d) = analyzer_device(&c, id).map(|d| d.id) {
                    c.execute(saine_core::Command::RemoveDevice { device: d });
                }
            }
            c.update_ui(a);
        })),
        Check("analyzers: the audio track's chain is empty again", Box::new(|c, _| {
            let t = track_of(c, TrackKind::Audio).ok_or("no audio track")?;
            let n = c.project().tracks[t].devices.len();
            (n == 0 && c.project().transport.key.is_none()).then_some(()).ok_or(format!("{n} devices left, key {:?}", c.project().transport.key))
        })),
    ]
}

/// Built-in device of definition `id` on the first audio track.
fn builtin_def_of<'a>(c: &'a Controller, id: &str) -> Option<(&'a saine_core::Device, &'static saine_core::builtin::DeviceDef)> {
    let t = track_of(c, TrackKind::Audio)?;
    c.project().tracks[t].devices.iter().find_map(|d| saine_core::builtin::def_of(d).filter(|def| def.id == id).map(|def| (d, def)))
}

fn leak(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

/// Character and Multiband Compressor: added from the Built-in list, live in the engine,
/// and their custom controls (Opto latch, band selector) drive the card.
fn dynamics_device_steps() -> Vec<Step> {
    use Step::*;
    let mut v = Vec::new();
    for (id, name, readouts) in [("character-compressor", "Character Compressor", 4usize), ("multiband-compressor", "Multiband Compressor", 8)] {
        let hist = Rc::new(Cell::new(0usize));
        let h0 = hist.clone();
        v.push(DoC(format!("dynamics: Add device › Built-in › {name}").leak(), Box::new(move |a, ctl| {
            h0.set(ctl.borrow().doc.history_labels().len());
            key(a, Key::Escape, false, false);
            let add_x = a.global::<Studio>().get_add_device_x();
            click(a, 5.0 + add_x + 60.0, BOTTOM_Y + 34.5 + 56.5 + 11.0);
            if let Ok(mut c) = ctl.try_borrow_mut() {
                c.update_ui(a);
            }
            if let Some(i) = devmenu_labels(a).iter().position(|r| r.0 == name) {
                let (x, y) = devmenu_row(a, i);
                click(a, x, y);
            }
        })));
        v.push(Check(format!("dynamics: {name} added in one undo step, card built, engine live ({readouts} readouts)").leak(), Box::new(move |c, a| {
            let n = c.doc.history_labels().len();
            if n != hist.get() + 1 || c.doc.undo_label() != Some(&format!("Add {name}")) {
                return Err(format!("history {} -> {n}, last {:?}", hist.get(), c.doc.undo_label()));
            }
            let d = builtin_of(c, id).ok_or("no device")?;
            let cd = card_named(a, name).ok_or("no card")?;
            let mut r = Vec::new();
            c.engine.builtin_readouts(d.id.0, &mut r).map_err(|e| e.to_string())?;
            let errors = c.engine.plugin_load_errors();
            let ok = cd.builtin && slint::Model::row_count(&cd.knobs) >= 6 && (cd.strip.visible || cd.bands.visible) && r.len() == readouts && !errors.iter().any(|(e, _)| *e == d.id);
            ok.then_some(()).ok_or(format!("knobs {} readouts {r:?} errors {errors:?}", slint::Model::row_count(&cd.knobs)))
        })));
    }
    v.push(DoC("dynamics: Character Compressor model › Opto", Box::new(|_, ctl| {
        let mut c = ctl.borrow_mut();
        let Some(d) = builtin_of(&c, "character-compressor").map(|d| d.id) else { return };
        let def = saine_core::builtin::find("character-compressor").unwrap();
        let i = def.params.iter().position(|p| p.id == "model").unwrap_or(0) as i32;
        let opto = def.param("model").and_then(|p| p.choices.iter().position(|c| *c == "Opto")).unwrap_or(0) as i32;
        c.panel_choice(d, i, opto);
    })));
    v.push(Check("dynamics: Opto shows the Comp/Limit latch in the dial row", Box::new(|_, a| {
        let cd = card_named(a, "Character Compressor").ok_or("no card")?;
        let latch = (0..slint::Model::row_count(&cd.knobs)).filter_map(|i| slint::Model::row_data(&cd.knobs, i)).find(|k| k.face == 2);
        latch.is_some_and(|k| k.value == "Comp" || k.value == "Limit").then_some(()).ok_or("no latch".into())
    })));
    v.push(DoC("dynamics: Multiband band selector › Mid", Box::new(|a, ctl| {
        let Some(d) = builtin_of(&ctl.borrow(), "multiband-compressor").map(|d| d.id.0 as i32) else { return };
        a.global::<Studio>().invoke_panel_view(d, 1, 1.0);
    })));
    v.push(Check("dynamics: the Threshold knob follows the Mid band (−20.0 dB)", Box::new(|_, a| {
        let cd = card_named(a, "Multiband Compressor").ok_or("no card")?;
        let k = slint::Model::row_data(&cd.knobs, 0).ok_or("no knob")?;
        (k.label == "Threshold" && k.value == "\u{2212}20.0 dB").then_some(()).ok_or(format!("{} {}", k.label, k.value))
    })));
    // Leave the chain as the later sections expect it (their clicks assume its width).
    v.push(DoC("dynamics: remove both devices again", Box::new(|a, ctl| {
        let mut c = ctl.borrow_mut();
        for id in ["character-compressor", "multiband-compressor"] {
            if let Some(d) = builtin_of(&c, id).map(|d| d.id) {
                c.execute(saine_core::Command::RemoveDevice { device: d });
            }
        }
        c.update_ui(a);
    })));
    v.push(Check("dynamics: both devices are gone", Box::new(|c, _| {
        (builtin_of(c, "character-compressor").is_none() && builtin_of(c, "multiband-compressor").is_none()).then_some(()).ok_or("still in the chain".into())
    })));
    v
}

/// The dynamics built-ins (limiter, gate, glue, transient shaper, mastering
/// limiter): each added from Add device › Built-in with its generated card and a
/// live engine instance, then the gestures the generic panel gained for them:
/// a style choice that writes its overlays, the strip's Alt+drag of two lines,
/// and latches (one undo step each).
fn limiter_family_steps() -> Vec<Step> {
    use Step::*;
    fn clear_audio_chain(a: &AppWindow, ctl: &Ctl) {
        let Some(t) = track_of(&ctl.borrow(), TrackKind::Audio) else { return };
        click_track_header(a, ctl, t);
        let mut c = ctl.borrow_mut();
        let ids: Vec<saine_core::DeviceId> = c.project().tracks[t].devices.iter().map(|d| d.id).collect();
        for d in ids {
            c.execute(saine_core::Command::RemoveDevice { device: d });
        }
        c.update_ui(a);
    }
    let mut out = vec![DoC("dynamics: select the audio track and clear its chain", Box::new(|a, ctl| clear_audio_chain(a, ctl)))];
    for id in ["limiter", "gate", "glue-compressor", "transient-shaper", "mastering-limiter"] {
        let def = saine_core::builtin::find(id).expect("registered");
        let name = def.name;
        out.push(DoC(leak(format!("{id}: Add device › Built-in, pick {name}")), Box::new(move |a, ctl| {
            key(a, Key::Escape, false, false);
            if let Ok(mut c) = ctl.try_borrow_mut() {
                c.update_ui(a);
            }
            let add_x = a.global::<Studio>().get_add_device_x();
            click(a, 5.0 + add_x + 60.0, BOTTOM_Y + 34.5 + 56.5 + 11.0);
            if let Some(i) = devmenu_labels(a).iter().position(|r| r.0 == name) {
                let (x, y) = devmenu_row(a, i);
                click(a, x, y);
            }
        })));
        out.push(Check(leak(format!("{id}: added at its defaults in one undo step; card has its knobs, sliders, view and width; engine instance live")), Box::new(move |c, a| {
            let (d, def) = builtin_def_of(c, id).ok_or("not added")?;
            let cd = card_named(a, name).ok_or("no card")?;
            let knobs: usize = def.layout.clusters.iter().map(|k| k.len()).sum();
            let mut r = Vec::new();
            c.engine.builtin_readouts(d.id.0, &mut r).map_err(|e| e.to_string())?;
            let errors = c.engine.plugin_load_errors();
            let ok = c.doc.undo_label() == Some(&format!("Add {name}"))
                && def.params.iter().all(|p| def.value(d, p) == p.default)
                && slint::Model::row_count(&cd.knobs) == knobs
                && slint::Model::row_count(&cd.sliders) == def.layout.sliders.len()
                && (cd.strip.visible || slint::Model::row_count(&cd.canvas.paths) > 0)
                && approx(cd.width, def.layout.width() * crate::device_panel::Z, 0.01)
                && r.len() == 3
                && !errors.iter().any(|(e, _)| *e == d.id);
            ok.then_some(()).ok_or(format!("undo {:?} knobs {} sliders {} strip {} width {} readouts {r:?}", c.doc.undo_label(), slint::Model::row_count(&cd.knobs), slint::Model::row_count(&cd.sliders), cd.strip.visible, cd.width))
        })));
    }
    let undo_len = Rc::new(Cell::new(0usize));
    let (u0, u1, u2, u3, u4, u5) = (undo_len.clone(), undo_len.clone(), undo_len.clone(), undo_len.clone(), undo_len.clone(), undo_len.clone());
    let param = |c: &Controller, dev: &str, p: &str| builtin_def_of(c, dev).and_then(|(d, def)| Some(def.value(d, def.param(p)?)));
    let invoke = |dev: &'static str, f: fn(&Studio, i32, &'static saine_core::builtin::DeviceDef)| -> Box<dyn Fn(&AppWindow, &Ctl)> {
        Box::new(move |a, ctl| {
            let Some((id, def)) = builtin_def_of(&ctl.borrow(), dev).map(|(d, def)| (d.id.0 as i32, def)) else { return };
            f(&a.global::<Studio>(), id, def);
        })
    };
    out.push(DoC("mastering-limiter: pick Style › Loud", Box::new(move |a, ctl| {
        u0.set(ctl.borrow().doc.history_labels().len());
        invoke("mastering-limiter", |s, id, def| s.invoke_panel_choice(id, def.index_of("style").map_or(-1, |i| i as i32), 1))(a, ctl);
    })));
    out.push(Check("mastering-limiter: the style and its overlay values (times, links, auto release) change in ONE undo step", Box::new(move |c, _| {
        let (d, def) = builtin_def_of(c, "mastering-limiter").ok_or("no device")?;
        let want = def.choice_values("style", 1.0);
        let n = c.doc.history_labels().len();
        (n == u1.get() + 1 && want.len() > 1 && want.iter().all(|(k, v)| def.value(d, def.param(k).unwrap()) == *v))
            .then_some(())
            .ok_or(format!("history {} -> {n}, want {want:?}", u1.get()))
    })));
    out.push(DoC("mastering-limiter: Alt+drag the strip lines down 20 px (threshold and ceiling together)", Box::new(move |a, ctl| {
        u2.set(ctl.borrow().doc.history_labels().len());
        let Some((id, rate)) = builtin_def_of(&ctl.borrow(), "mastering-limiter").and_then(|(d, _)| Some((d.id.0 as i32, card_named(a, "Mastering Limiter")?.strip.thresh_rate))) else { return };
        let s = a.global::<Studio>();
        let def = saine_core::builtin::find("mastering-limiter").unwrap();
        let (t, ce) = (def.index_of("threshold").unwrap() as i32, def.index_of("ceiling").unwrap() as i32);
        s.invoke_panel_drag_pair(id, t, ce, 0, 0.0, rate, false);
        for _ in 0..4 {
            s.invoke_panel_drag_pair(id, t, ce, 1, -5.0, rate, false);
        }
        s.invoke_panel_drag_pair(id, t, ce, 2, 0.0, rate, false);
    })));
    out.push(Check("mastering-limiter: both lines moved down by the same dB, ONE undo step", Box::new(move |c, _| {
        let (t, ce) = (param(c, "mastering-limiter", "threshold").unwrap_or(f32::NAN), param(c, "mastering-limiter", "ceiling").unwrap_or(f32::NAN));
        let n = c.doc.history_labels().len();
        (n == u3.get() + 1 && t < -1.0 && approx(t, ce + 1.0, 0.051)).then_some(()).ok_or(format!("history {} -> {n}, threshold {t} ceiling {ce}", u3.get()))
    })));
    out.push(Check("mastering-limiter: Auto Release is a right-aligned picker latch reading \"Auto Release On\"", Box::new(|_, a| {
        let cd = card_named(a, "Mastering Limiter").ok_or("no card")?;
        let picks: Vec<_> = (0..slint::Model::row_count(&cd.picks)).filter_map(|i| slint::Model::row_data(&cd.picks, i)).collect();
        let latch = picks.iter().position(|p| p.kind == 5).ok_or("no latch")?;
        let p = &picks[latch];
        let label = p.label.to_string();
        (picks[..latch].iter().any(|q| q.kind == 4) && p.selected == 1 && label == "Auto Release On").then_some(()).ok_or(format!("label {label:?}"))
    })));
    out.push(DoC("transient-shaper: switch the Clip latch on (panel-choice)", Box::new(move |a, ctl| {
        u4.set(ctl.borrow().doc.history_labels().len());
        invoke("transient-shaper", |s, id, def| s.invoke_panel_choice(id, def.index_of("clip").map_or(-1, |i| i as i32), 1))(a, ctl);
    })));
    out.push(Check("transient-shaper: Clip on in one undo step; its dial cell is a latch reading On", Box::new(move |c, a| {
        let n = c.doc.history_labels().len();
        let cd = card_named(a, "Transient Shaper").ok_or("no card")?;
        let k = (0..slint::Model::row_count(&cd.knobs)).filter_map(|i| slint::Model::row_data(&cd.knobs, i)).find(|k| k.face == 2).ok_or("no latch")?;
        (n == u5.get() + 1 && c.doc.undo_label() == Some("Device Parameter Change") && param(c, "transient-shaper", "clip") == Some(1.0) && k.norm > 0.5 && k.value == "On")
            .then_some(())
            .ok_or(format!("history {} -> {n}, clip {:?}", u5.get(), param(c, "transient-shaper", "clip")))
    })));
    out.push(DoC("dynamics: remove the section's devices", Box::new(|a, ctl| clear_audio_chain(a, ctl))));
    out.push(Check("dynamics: the audio track's chain is empty again", Box::new(|c, _| {
        let n = track_of(c, TrackKind::Audio).map_or(usize::MAX, |t| c.project().tracks[t].devices.len());
        (n == 0).then_some(()).ok_or(format!("{n} devices left"))
    })));
    out
}

// ------------------------------------------------------------------ modulation and containers

fn device_where(c: &Controller, f: impl Fn(&saine_core::Device) -> bool) -> Option<saine_core::Device> {
    let t = track_of(c, TrackKind::Audio)?;
    c.project().tracks[t].devices.iter().find(|d| f(d)).cloned()
}

fn lfo_device(c: &Controller) -> Option<saine_core::Device> {
    device_where(c, saine_core::modulation::is_lfo)
}

fn chain_head(c: &Controller) -> Option<saine_core::Device> {
    device_where(c, saine_core::modulation::is_rack)
}

/// LFO from a control's menu, chain head with a member and a macro, folding,
/// collapsing, the swap picker and the plugin card's "+ Add".
fn modulation_steps() -> Vec<Step> {
    use Step::*;
    let hist = Rc::new(Cell::new(0usize));
    let h = || hist.clone();
    fn one(hc: &Rc<Cell<usize>>, c: &Controller, label: &str) -> Result<(), String> {
        let n = c.doc.history_labels().len();
        let was = hc.get();
        hc.set(n);
        (n == was + 1 && c.doc.undo_label() == Some(label)).then_some(()).ok_or(format!("history {was} -> {n}, last {:?} (want {label})", c.doc.undo_label()))
    }
    let first_comp = |c: &Controller| device_where(c, |d| d.plugin.id == "compressor" && d.chain.is_none()).map(|d| d.id);
    let (h0, h1, h2, h3, h4, h5, h6, h7, h8, h9, h10, h11) = (h(), h(), h(), h(), h(), h(), h(), h(), h(), h(), h(), h());
    vec![
        DoC("modulation: right-click the Compressor's Threshold knob", Box::new(move |a, ctl| {
            key(a, Key::Escape, false, false);
            h0.set(ctl.borrow().doc.history_labels().len());
            let Some(id) = first_comp(&ctl.borrow()) else { return };
            a.global::<Studio>().invoke_mod_menu(id.0 as i32, 0, 600.0, 500.0);
        })),
        Check("modulation: the control menu offers Automate Threshold and Modulators › New LFO", Box::new(|_, a| {
            let items = devmenu_labels(a);
            (items.iter().any(|r| r.0 == "Automate Threshold" && r.2) && items.iter().any(|r| r.0 == "New LFO" && r.2)).then_some(()).ok_or(format!("items {items:?}"))
        })),
        Do("modulation: pick New LFO", Box::new(|a| {
            if let Some(i) = devmenu_labels(a).iter().position(|r| r.0 == "New LFO") {
                let (x, y) = devmenu_row(a, i);
                click(a, x, y);
            }
        })),
        Check("modulation: an LFO at the chain start aims at Threshold (±25%) in one undo step; its card lists the target; the knob shows the reach", Box::new(move |c, a| {
            one(&h1, c, "New LFO")?;
            let t = track_of(c, TrackKind::Audio).ok_or("no audio track")?;
            let lfo = c.project().tracks[t].devices.first().filter(|d| saine_core::modulation::is_lfo(d)).ok_or("no LFO first")?;
            let l = lfo.links.first().ok_or("no link")?;
            let cd = card_named(a, "LFO").ok_or("no LFO card")?;
            let comp = card_named(a, "Compressor").and_then(|cd| slint::Model::row_data(&cd.knobs, 0)).ok_or("no knob")?;
            let row = slint::Model::row_data(&cd.modv.targets, 0).ok_or("no target row")?;
            (l.param == saine_core::ModParam::Builtin("threshold".into()) && approx(l.max, 0.25, 1e-6) && approx(l.min, -0.25, 1e-6) && cd.modv.kind == 1 && row.amount == "25%" && comp.mod_on && comp.mod_hi > comp.mod_lo)
                .then_some(())
                .ok_or(format!("link {l:?} card kind {} row {:?} knob reach {}..{}", cd.modv.kind, row.amount, comp.mod_lo, comp.mod_hi))
        })),
        Check("engine: the LFO is live and reports its readouts (position, depth, cycle)", Box::new(|c, _| {
            let id = lfo_device(c).ok_or("no LFO")?.id;
            let mut r = Vec::new();
            c.engine.builtin_readouts(id.0, &mut r).map_err(|e| e.to_string())?;
            (r.len() == 3).then_some(()).ok_or(format!("readouts {r:?}"))
        })),
        DoC("modulation: drag the target amount up 30 px", Box::new(|_, ctl| {
            let Some(id) = lfo_device(&ctl.borrow()).map(|d| d.id) else { return };
            let mut c = ctl.borrow_mut();
            c.mod_amount_drag(id, 0, 0, 0.0, false);
            for _ in 0..30 {
                c.mod_amount_drag(id, 0, 1, 1.0, false);
            }
            c.mod_amount_drag(id, 0, 2, 0.0, false);
        })),
        Check("modulation: amount 25% → 45% (±), one undo step", Box::new(move |c, _| {
            one(&h2, c, "Modulation Amount Change")?;
            let l = lfo_device(c).and_then(|d| d.links.first().cloned()).ok_or("no link")?;
            (approx(l.max, 0.45, 1e-4) && approx(l.min, -l.max, 1e-6)).then_some(()).ok_or(format!("link {l:?}"))
        })),
        Do("modulation: fold the LFO (header −)", Box::new(|a| {
            if let Some(cd) = card_named(a, "LFO") {
                let z = crate::device_panel::Z;
                let (x, y) = card_xy(&cd, cd.width - 33.0 * z, 16.0 * z);
                click(a, x, y);
            }
        })),
        Check("modulation: the LFO folds to a 30 pt strip, one undo step", Box::new(move |c, a| {
            one(&h3, c, "Fold Device")?;
            let cd = card_named(a, "LFO").ok_or("no card")?;
            (cd.folded && approx(cd.width, crate::device_mod::FOLDED_W, 0.01)).then_some(()).ok_or(format!("folded {} width {}", cd.folded, cd.width))
        })),
        Do("modulation: click the folded strip", Box::new(|a| {
            if let Some(cd) = card_named(a, "LFO") {
                let (x, y) = card_xy(&cd, 15.0, 150.0);
                click(a, x, y);
            }
        })),
        Check("modulation: a click unfolds it, one undo step", Box::new(move |c, a| {
            one(&h4, c, "Unfold Device")?;
            card_named(a, "LFO").is_some_and(|cd| !cd.folded).then_some(()).ok_or("still folded".into())
        })),
        DoC("chain: Add device › Chains", Box::new(|a, ctl| {
            // The Add device card is scrolled out of view by now: use its row callback.
            a.global::<Studio>().invoke_add_category(4, 600.0, 300.0);
            if let Ok(mut c) = ctl.try_borrow_mut() {
                c.update_ui(a);
            }
        })),
        Do("chain: pick Empty chain", Box::new(|a| {
            if let Some(i) = devmenu_labels(a).iter().position(|r| r.0 == "Empty chain") {
                let (x, y) = devmenu_row(a, i);
                click(a, x, y);
            }
        })),
        Check("chain: a chain head with 8 macros, one undo step; its empty frame offers Add effect", Box::new(move |c, a| {
            one(&h5, c, "Add Chain")?;
            let cd = card_named(a, "Chain").ok_or("no Chain card")?;
            let frames = a.global::<Studio>().get_chain_frames();
            let f = slint::Model::row_data(&frames, 0).ok_or("no frame")?;
            (cd.modv.kind == 2 && slint::Model::row_count(&cd.knobs) == 8 && f.empty).then_some(()).ok_or(format!("kind {} macros {} empty {}", cd.modv.kind, slint::Model::row_count(&cd.knobs), f.empty))
        })),
        DoC("chain: Add effect › Compressor", Box::new(|a, ctl| {
            let Some(head) = chain_head(&ctl.borrow()).map(|d| d.id) else { return };
            a.global::<Studio>().invoke_chain_add_effect(head.0 as i32, 600.0, 300.0);
            if let Ok(mut c) = ctl.try_borrow_mut() {
                c.update_ui(a);
            }
            if let Some(i) = devmenu_labels(a).iter().position(|r| r.0 == "Compressor") {
                let (x, y) = devmenu_row(a, i);
                click(a, x, y);
            }
        })),
        Check("chain: the Compressor joins the chain right after the head, one undo step; the frame wraps it", Box::new(move |c, a| {
            one(&h6, c, "Add Compressor")?;
            let head = chain_head(c).ok_or("no head")?;
            let member = device_where(c, |d| d.chain == Some(head.id)).ok_or("no member")?;
            let f = slint::Model::row_data(&a.global::<Studio>().get_chain_frames(), 0).ok_or("no frame")?;
            (member.plugin.id == "compressor" && !f.empty).then_some(()).ok_or(format!("member {:?} empty {}", member.plugin.id, f.empty))
        })),
        DoC("chain: open Macro 1, press Map, click the member's Threshold knob", Box::new(|_, ctl| {
            let (head, member) = {
                let c = ctl.borrow();
                let Some(h) = chain_head(&c) else { return };
                let Some(m) = device_where(&c, |d| d.chain == Some(h.id)) else { return };
                (h.id, m.id)
            };
            // The member card may be scrolled out of the window: press its knob through the panel callback.
            let mut c = ctl.borrow_mut();
            c.macro_select(head, 0);
            c.mod_map(head, 0);
            c.panel_drag(member, 0, 0, 0.0, 1.0 / 160.0, false);
            c.panel_drag(member, 0, 2, 0.0, 1.0 / 160.0, false);
        })),
        Check("chain: Macro 1 targets the member's Threshold (one undo step) and is drawn mapped", Box::new(move |c, a| {
            one(&h7, c, "Add Modulation")?;
            let head = chain_head(c).ok_or("no head")?;
            let l = head.links.first().ok_or("no link")?;
            let cd = card_named(a, "Chain").ok_or("no card")?;
            let m1 = slint::Model::row_data(&cd.knobs, 0).ok_or("no macro")?;
            (l.slot == 0 && l.param == saine_core::ModParam::Builtin("threshold".into()) && m1.mapped && m1.selected && slint::Model::row_count(&cd.modv.targets) == 1)
                .then_some(())
                .ok_or(format!("link {l:?} mapped {} selected {}", m1.mapped, m1.selected))
        })),
        Do("chain: Escape", Box::new(|a| key(a, Key::Escape, false, false))),
        Check("chain: Escape stops mapping", Box::new(|c, _| c.devices.modu.map.is_none().then_some(()).ok_or(format!("map {:?}", c.devices.modu.map)))),
        DoC("chain: drag Macro 1 to the top", Box::new(|_, ctl| {
            let Some(head) = chain_head(&ctl.borrow()).map(|d| d.id) else { return };
            let mut c = ctl.borrow_mut();
            c.panel_drag(head, 0, 0, 0.0, 1.0 / 100.0, false);
            c.panel_drag(head, 0, 1, 120.0, 1.0 / 100.0, false);
            c.panel_drag(head, 0, 2, 0.0, 1.0 / 100.0, false);
        })),
        Check("chain: Macro 1 at 100% moves the member's Threshold to the top of its range (shown on the knob)", Box::new(move |c, a| {
            one(&h8, c, "Device Parameter Change")?;
            let head = chain_head(c).ok_or("no head")?;
            let member = device_where(c, |d| d.chain == Some(head.id)).ok_or("no member")?;
            let k = cards(a).into_iter().find(|cd| cd.id as u64 == member.id.0).and_then(|cd| slint::Model::row_data(&cd.knobs, 0)).ok_or("no knob")?;
            (k.mod_on && approx(k.mod_now, 1.0, 1e-4)).then_some(()).ok_or(format!("knob now {}", k.mod_now))
        })),
        DoC("chain: collapse the chain («)", Box::new(|a, ctl| {
            let Some(head) = chain_head(&ctl.borrow()).map(|d| d.id) else { return };
            a.global::<Studio>().invoke_device_collapse(head.0 as i32);
        })),
        Check("chain: collapsed, the member's card is hidden (one undo step)", Box::new(move |c, a| {
            one(&h9, c, "Unfold Device")?;
            let head = chain_head(c).ok_or("no head")?;
            let member = device_where(c, |d| d.chain == Some(head.id)).ok_or("no member")?;
            (head.collapsed && !cards(a).iter().any(|cd| cd.id as u64 == member.id.0)).then_some(()).ok_or("member still shown".into())
        })),
        DoC("chain: ✕ on the chain head", Box::new(|a, ctl| {
            let Some(head) = chain_head(&ctl.borrow()).map(|d| d.id) else { return };
            match cards(a).into_iter().find(|cd| cd.id as u64 == head.0) {
                // Click the ✕ when the card is on screen; otherwise (a smaller
                // window, or a longer device row) press it through its callback.
                Some(cd) if 5.0 + cd.x + cd.width <= 1440.0 - 5.0 => {
                    let (x, y) = card_xy(&cd, cd.width - 14.5, 13.5);
                    click(a, x, y);
                }
                _ => a.global::<Studio>().invoke_device_remove(head.0 as i32),
            }
        })),
        Check("chain: removing the head removes its member in one undo step", Box::new(move |c, _| {
            one(&h10, c, "Remove Chain")?;
            (chain_head(c).is_none() && device_where(c, |d| d.chain.is_some()).is_none()).then_some(()).ok_or("chain left behind".into())
        })),
        Do("chain: ⌘Z", Box::new(|a| key(a, "z", true, false))),
        Check("chain: undo brings back the head and its member", Box::new(move |c, _| {
            h11.set(c.doc.history_labels().len());
            let head = chain_head(c).ok_or(format!("no head; history {:?}", c.doc.history_labels().iter().rev().take(3).collect::<Vec<_>>()))?;
            device_where(c, |d| d.chain == Some(head.id)).map(|_| ()).ok_or("no member".into())
        })),
        DoC("swap: open the Compressor's name menu", Box::new(move |a, ctl| {
            let Some(id) = first_comp(&ctl.borrow()) else { return };
            a.global::<Studio>().invoke_device_swap_menu(id.0 as i32, 600.0, 300.0);
        })),
        Check("swap: the effect picker marks Compressor and offers a new LFO", Box::new(|_, a| {
            let items = devmenu_labels(a);
            (items.iter().any(|r| r.0 == "Compressor" && r.1 == "✓") && items.iter().any(|r| r.0 == "LFO" && r.2)).then_some(()).ok_or(format!("items {items:?}"))
        })),
        DoC("plugin card: add the CLAP test gain, open + Add", Box::new(|a, ctl| {
            key(a, Key::Escape, false, false);
            let gain = saine_core::PluginRef {
                format: saine_core::PluginFormat::Clap,
                id: "studio.saine.test.gain".into(),
                path: Some(saine_engine::test_plugin_dir().join("SaineTestGain.clap").to_string_lossy().into_owned()),
                name: "Saine Test Gain".into(),
                vendor: "Saine".into(),
                version: "1.0.0".into(),
                kind: saine_core::PluginKind::Effect,
            };
            let id = {
                let mut c = ctl.borrow_mut();
                let Some(t) = track_of(&c, TrackKind::Audio).map(|i| c.project().tracks[i].id) else { return };
                c.add_item_to_track(&crate::devices::Item::Plugin(gain), t, None);
                c.update_ui(a);
                c.project().track(t).and_then(|t| t.devices.last()).map(|d| d.id)
            };
            if let Some(id) = id {
                a.global::<Studio>().invoke_card_add_menu(id.0 as i32, 600.0, 300.0);
            }
        })),
        Check("plugin card: a new card shows only + Add; the list names the plugin's parameters", Box::new(|_, a| {
            let cd = card_named(a, "Saine Test Gain").ok_or("no card")?;
            let items = devmenu_labels(a);
            (!cd.builtin && slint::Model::row_count(&cd.knobs) == 0 && items.iter().any(|r| r.2)).then_some(()).ok_or(format!("knobs {} items {items:?}", slint::Model::row_count(&cd.knobs)))
        })),
        Do("plugin card: pick the first parameter", Box::new(|a| {
            if let Some(i) = devmenu_labels(a).iter().position(|r| r.2) {
                let (x, y) = devmenu_row(a, i);
                click(a, x, y);
            }
        })),
        Check("plugin card: the control is on the card (SetCardParams, one undo step)", Box::new(|c, a| {
            let cd = card_named(a, "Saine Test Gain").ok_or("no card")?;
            (c.doc.undo_label() == Some("Device Controls Change") && slint::Model::row_count(&cd.knobs) == 1).then_some(()).ok_or(format!("last {:?} knobs {}", c.doc.undo_label(), slint::Model::row_count(&cd.knobs)))
        })),
    ]
}

// ------------------------------------------------------------------ custom device views

fn device_of<'a>(c: &'a Controller, id: &str) -> Option<&'a saine_core::Device> {
    let t = track_of(c, TrackKind::Audio)?;
    c.project().tracks[t].devices.iter().find(|d| saine_core::builtin::def_of(d).is_some_and(|def| def.id == id))
}

fn value_of(c: &Controller, device: &str, param: &str) -> Option<f32> {
    let d = device_of(c, device)?;
    let def = saine_core::builtin::def_of(d)?;
    Some(def.value(d, def.param(param)?))
}

/// A pointer event on a custom view at a card-relative reference point (pt at 100%).
fn field_event(a: &AppWindow, ctl: &Ctl, device: &str, kind: i32, x: f32, y: f32) {
    let Some(id) = device_of(&ctl.borrow(), device).map(|d| d.id.0 as i32) else { return };
    let z = crate::device_panel::Z;
    a.global::<Studio>().invoke_panel_editor(id, kind, x * z, y * z - 27.0, 600.0, 400.0, false, false, 0.0);
}

/// Window click on the Trance Gate's ring hole (no action): gives its field keyboard focus.
fn focus_trance_gate(a: &AppWindow) {
    if let Some(cd) = card_named(a, "Trance Gate") {
        let z = crate::device_panel::Z;
        let (x, y) = card_xy(&cd, 98.0 * z, 157.5 * z + 30.0 * z);
        click(a, x, y);
    }
}

/// Trance Gate ring: reference point in the middle of step i of 16.
fn ring_xy(i: usize) -> (f32, f32) {
    let a = std::f32::consts::TAU * (i as f32 + 0.5) / 16.0;
    let r = 0.8 * 83.0;
    (98.0 + r * a.sin(), 157.5 - r * a.cos())
}

/// Trance Gate and Shaper: added from the Built-in list, then driven through their custom views.
fn custom_view_steps() -> Vec<Step> {
    use Step::*;
    let hist = Rc::new(Cell::new(0usize));
    let mask0 = Rc::new(Cell::new(0u32));
    fn grew(hc: &Rc<Cell<usize>>, c: &Controller, n: usize) -> Result<(), String> {
        let now = c.doc.history_labels().len();
        let was = hc.get();
        hc.set(now);
        (now == was + n).then_some(()).ok_or(format!("history {was} -> {now} (want +{n}), last {:?}", c.doc.undo_label()))
    }
    fn mask(c: &Controller) -> u32 {
        saine_core::builtin::trance_gate::mask(value_of(c, "trance-gate", "p1_lo").unwrap_or(0.0), value_of(c, "trance-gate", "p1_hi").unwrap_or(0.0))
    }
    fn act(ctl: &Ctl, a: &AppWindow, f: impl FnOnce(&AppWindow, &Controller) -> Vec<(i32, f32, f32)>, device: &str) {
        let events = f(a, &ctl.borrow());
        for (kind, x, y) in events {
            field_event(a, ctl, device, kind, x, y);
        }
        if let Ok(mut c) = ctl.try_borrow_mut() {
            c.update_ui(a);
        }
    }
    let (h0, h1, h2, h3, h4, h5, h6, h7) = (hist.clone(), hist.clone(), hist.clone(), hist.clone(), hist.clone(), hist.clone(), hist.clone(), hist.clone());
    let (m0, m1, m2, m3) = (mask0.clone(), mask0.clone(), mask0.clone(), mask0.clone());
    vec![
        Do("views: Escape (close menus)", Box::new(|a| key(a, Key::Escape, false, false))),
        DoC("views: select the first audio track, clear its chain, add Trance Gate and Shaper", Box::new(move |a, ctl| {
            let Some(i) = track_of(&ctl.borrow(), TrackKind::Audio) else { return };
            click_track_header(a, ctl, i);
            let mut c = ctl.borrow_mut();
            let ids: Vec<saine_core::DeviceId> = c.project().tracks[i].devices.iter().map(|d| d.id).collect();
            for d in ids {
                c.execute(saine_core::Command::RemoveDevice { device: d });
            }
            h0.set(c.doc.history_labels().len());
            let t = c.project().tracks[i].id;
            for id in ["trance-gate", "shaper"] {
                let def = saine_core::builtin::find(id).expect("registered");
                c.add_item_to_track(&crate::devices::Item::Builtin(def), t, None);
            }
            c.update_ui(a);
        })),
        Check("views: both added (two undo steps); cards are 528 / 501 pt with an interactive canvas, 6 / 4 knobs on the dial row; engine loaded them", Box::new(move |c, a| {
            grew(&h1, c, 2)?;
            let z = crate::device_panel::Z;
            let tg = card_named(a, "Trance Gate").ok_or("no Trance Gate card")?;
            let sh = card_named(a, "Shaper").ok_or("no Shaper card")?;
            let errors = c.engine.plugin_load_errors();
            let ok = tg.canvas.visible
                && sh.canvas.visible
                && approx(tg.width, 528.0 * z, 0.01)
                && approx(sh.width, 501.0 * z, 0.01)
                && slint::Model::row_count(&tg.knobs) == 6
                && slint::Model::row_count(&sh.knobs) == 4
                && slint::Model::row_count(&tg.canvas.paths) > 10
                && slint::Model::row_count(&sh.canvas.paths) > 10
                && tg.canvas.interactive
                && sh.canvas.interactive
                && [&tg.knobs, &sh.knobs].iter().all(|k| slint::Model::iter(*k).all(|k| approx(k.y, crate::device_panel::DIAL_Y * z, 0.01)))
                && ["trance-gate", "shaper"].iter().all(|id| device_of(c, id).is_some_and(|d| !errors.iter().any(|(e, _)| *e == d.id)));
            ok.then_some(()).ok_or(format!("tg {} {} sh {} {} errors {errors:?}", tg.width, tg.canvas.visible, sh.width, sh.canvas.visible))
        })),
        DoC("views: click step 1 on the Trance Gate ring", Box::new(move |a, ctl| {
            m0.set(mask(&ctl.borrow()));
            act(ctl, a, |_, _| {
                let (x, y) = ring_xy(0);
                vec![(0, x, y), (2, x, y)]
            }, "trance-gate");
        })),
        Check("views: step 1 toggled, one undo step", Box::new(move |c, _| {
            grew(&h2, c, 1)?;
            let (was, now) = (m1.get(), mask(c));
            (now == was ^ 1).then_some(()).ok_or(format!("mask {was:#x} -> {now:#x}"))
        })),
        DoC("views: paint steps 2-4 in one drag", Box::new(move |a, ctl| {
            m2.set(mask(&ctl.borrow()));
            act(ctl, a, |_, _| {
                let ((x1, y1), (x2, y2), (x3, y3)) = (ring_xy(1), ring_xy(2), ring_xy(3));
                vec![(0, x1, y1), (1, x2, y2), (1, x3, y3), (2, x3, y3)]
            }, "trance-gate");
        })),
        Check("views: steps 2-4 all take the first step's new state, ONE undo step", Box::new(move |c, _| {
            grew(&h3, c, 1)?;
            let (was, now) = (m3.get(), mask(c));
            let want = if was >> 1 & 1 == 1 { was & !0b1110 } else { was | 0b1110 };
            (now == want).then_some(()).ok_or(format!("mask {was:#x} -> {now:#x}, want {want:#x}"))
        })),
        DoC("views: paint steps 6-7, press Escape before release", Box::new(move |a, ctl| {
            focus_trance_gate(a);
            act(ctl, a, |_, _| {
                let ((x1, y1), (x2, y2)) = (ring_xy(5), ring_xy(6));
                vec![(0, x1, y1), (1, x2, y2)]
            }, "trance-gate");
            key(a, Key::Escape, false, false);
            act(ctl, a, |_, _| vec![(2, 0.0, 0.0)], "trance-gate");
        })),
        Check("views: Escape cancels the paint (no undo step)", Box::new(move |c, _| grew(&h4, c, 0))),
        DoC("views: drag the Shaper's middle point down 40 px", Box::new(|a, ctl| {
            act(ctl, a, |_, _| {
                let (x, y) = (48.0 + 0.5 * 435.0, 108.0);
                let d = 40.0 / crate::device_panel::Z;
                vec![(0, x, y), (1, x, y + d / 2.0), (1, x, y + d), (2, x, y + d)]
            }, "shaper");
        })),
        Check("views: the point follows (Volume 100% -> about 51%), one undo step", Box::new(move |c, _| {
            grew(&h5, c, 1)?;
            let v = value_of(c, "shaper", "vol_p2_y").unwrap_or(f32::NAN);
            approx(v, 51.0, 1.5).then_some(()).ok_or(format!("vol_p2_y {v}"))
        })),
        DoC("views: Shaper › Filter tab, switch the lane on, pick the second library shape", Box::new(|a, ctl| {
            act(ctl, a, |_, _| vec![(0, 291.5 + 30.0, 50.0), (2, 291.5 + 30.0, 50.0), (0, 291.5, 50.0), (2, 291.5, 50.0), (0, 60.0, 81.0)], "shaper");
            if let Some(i) = devmenu_labels(a).iter().position(|r| r.0 == saine_core::builtin::shaper::SHAPES[4][1].name) {
                let (x, y) = devmenu_row(a, i);
                click(a, x, y);
            }
        })),
        Check("views: Filter lane on and shaped (two undo steps); the strip names the shape", Box::new(move |c, _| {
            grew(&h6, c, 2)?;
            let d = device_of(c, "shaper").ok_or("no shaper")?;
            let def = saine_core::builtin::def_of(d).ok_or("no def")?;
            let pts = saine_core::builtin::shaper::lane_points(&def.values(d), 4);
            let name = saine_core::builtin::shaper::matching_shape(4, &pts);
            (value_of(c, "shaper", "filter_on") == Some(1.0) && name == Some(saine_core::builtin::shaper::SHAPES[4][1].name))
                .then_some(())
                .ok_or(format!("filter_on {:?} shape {name:?}", value_of(c, "shaper", "filter_on")))
        })),
        Do("views: click the Trance Gate's ring hole, ⌘Z", Box::new(|a| {
            focus_trance_gate(a);
            key(a, "z", true, false);
        })),
        Check("views: undo restores the Filter lane's default shape", Box::new(move |c, _| {
            h7.set(c.doc.history_labels().len());
            let d = device_of(c, "shaper").ok_or("no shaper")?;
            let def = saine_core::builtin::def_of(d).ok_or("no def")?;
            let pts = saine_core::builtin::shaper::lane_points(&def.values(d), 4);
            let name = saine_core::builtin::shaper::matching_shape(4, &pts);
            (name == Some(saine_core::builtin::shaper::SHAPES[4][0].name)).then_some(()).ok_or(format!("shape {name:?}"))
        })),
        DoC("views: remove Trance Gate and Shaper", Box::new(|a, ctl| {
            let mut c = ctl.borrow_mut();
            let ids: Vec<saine_core::DeviceId> = ["trance-gate", "shaper"].iter().filter_map(|id| device_of(&c, id).map(|d| d.id)).collect();
            for d in ids {
                c.execute(saine_core::Command::RemoveDevice { device: d });
            }
            c.update_ui(a);
        })),
        Check("views: the audio track's chain is empty again", Box::new(|c, _| {
            let t = track_of(c, TrackKind::Audio).ok_or("no audio track")?;
            let n = c.project().tracks[t].devices.len();
            (n == 0).then_some(()).ok_or(format!("{n} devices left"))
        })),
    ]
}

/// The restoration devices (De-Clipper, De-Esser, De-Hummer, De-Noiser): each one is added
/// through Add device › Built-in, checked against its definition and the engine, and its field
/// is dragged with real pointer events (one undo step), then undone.
fn restoration_device_steps() -> Vec<Step> {
    use Step::*;
    // (display name, id, readouts, field drag in px, params the drag must move)
    type Moves = &'static [&'static str];
    const DEVICES: [(&str, &str, usize, (f32, f32), Moves); 4] = [
        ("De-Clipper", "declipper", 0, (40.0, 0.0), &["threshold"]),
        ("De-Esser", "deesser", 49, (30.0, 20.0), &["frequency", "reduction"]),
        ("De-Hummer", "dehummer", 48, (-30.0, 20.0), &["frequency", "depth"]),
        ("De-Noiser", "denoiser", 49, (0.0, -20.0), &["threshold"]),
    ];
    fn device<'a>(c: &'a Controller, id: &str) -> Option<&'a saine_core::Device> {
        let t = track_of(c, TrackKind::Audio)?;
        c.project().tracks[t].devices.iter().find(|d| d.plugin.format == saine_core::PluginFormat::Builtin && d.plugin.id == id)
    }
    fn value(c: &Controller, id: &str, param: &str) -> Option<f32> {
        let d = device(c, id)?;
        let def = saine_core::builtin::def_of(d)?;
        Some(def.value(d, def.param(param)?))
    }
    let mut steps = Vec::new();
    for (name, id, readouts, (dx, dy), moves) in DEVICES {
        let hist = Rc::new(Cell::new(0usize));
        let (h0, h1, h2, h3) = (hist.clone(), hist.clone(), hist.clone(), hist.clone());
        steps.push(DoC(leak(format!("restoration: clear the audio track's chain, open Add device › Built-in for {name}")), Box::new(move |a, ctl| {
            key(a, Key::Escape, false, false);
            let i = track_of(&ctl.borrow(), TrackKind::Audio).unwrap_or(0);
            click_track_header(a, ctl, i);
            {
                let mut c = ctl.borrow_mut();
                let ids: Vec<saine_core::DeviceId> = c.project().tracks[i].devices.iter().map(|d| d.id).collect();
                for d in ids {
                    c.execute(saine_core::Command::RemoveDevice { device: d });
                }
                h0.set(c.doc.history_labels().len());
                c.update_ui(a);
            }
            let add_x = a.global::<Studio>().get_add_device_x();
            click(a, 5.0 + add_x + 60.0, BOTTOM_Y + 34.5 + 56.5 + 11.0);
        })));
        steps.push(Do(leak(format!("restoration: pick {name}")), Box::new(move |a| {
            if let Some(i) = devmenu_labels(a).iter().position(|r| r.0 == name && r.2) {
                let (x, y) = devmenu_row(a, i);
                click(a, x, y);
            }
        })));
        steps.push(Check(leak(format!("restoration: {name} added at its defaults (one undo step); card follows the definition (width, knobs, field)")), Box::new(move |c, a| {
            let n = c.doc.history_labels().len();
            let want = format!("Add {name}");
            if n != h1.get() + 1 || c.doc.undo_label() != Some(want.as_str()) {
                return Err(format!("history {} -> {n}, last {:?}", h1.get(), c.doc.undo_label()));
            }
            let def = saine_core::builtin::find(id).ok_or("not registered")?;
            let d = device(c, id).ok_or("not in the chain")?;
            let cd = card_named(a, name).ok_or("no card")?;
            let knobs: usize = def.layout.clusters.iter().map(|c| c.len()).sum();
            let ok = cd.builtin
                && cd.canvas.visible
                && slint::Model::row_count(&cd.knobs) == knobs
                && approx(cd.width, def.layout.width() * crate::device_panel::Z, 0.01)
                && def.params.iter().all(|p| def.value(d, p) == p.default);
            ok.then_some(()).ok_or(format!("knobs {} width {} field {}", slint::Model::row_count(&cd.knobs), cd.width, cd.canvas.visible))
        })));
        steps.push(Check(leak(format!("engine: {name} is live with {readouts} readouts (spectrum bands where the field draws one)")), Box::new(move |c, _| {
            let d = device(c, id).ok_or("no device")?.id;
            let mut r = Vec::new();
            c.engine.builtin_readouts(d.0, &mut r).map_err(|e| e.to_string())?;
            let errors = c.engine.plugin_load_errors();
            (r.len() == readouts && !errors.iter().any(|(e, _)| *e == d)).then_some(()).ok_or(format!("readouts {} errors {errors:?}", r.len()))
        })));
        steps.push(DoC(leak(format!("restoration: drag {name}'s field ({dx:+}, {dy:+}) px")), Box::new(move |a, ctl| {
            h2.set(ctl.borrow().doc.history_labels().len());
            let Some(cd) = card_named(a, name) else { return };
            let f = &cd.canvas;
            // The De-Esser grabs its band handle (the circle path "M{x - r} {y} a..."), the
            // De-Noiser its threshold line; the others drag the plane.
            let handle = || {
                let p = slint::Model::iter(&f.paths).find(|p| p.commands.contains(" a"))?;
                let mut n = p.commands.trim_start_matches('M').split_whitespace().map(|t| t.parse::<f32>().unwrap_or(0.0));
                Some((n.next()? + 5.0 * crate::device_panel::Z, n.next()?))
            };
            let (wx, wy) = match id {
                "deesser" => handle().unwrap_or((0.0, 0.0)),
                "denoiser" => (f.w * 0.75, slint::Model::row_data(&f.grips, 0).map_or(0.0, |p| p.y)),
                _ => (f.w / 2.0, (f.h - f.well_y) / 2.0),
            };
            let (x, y) = card_xy(&cd, f.x + wx, f.y + f.well_y + wy);
            drag(a, x, y, x + dx, y + dy);
        })));
        steps.push(Check(leak(format!("restoration: the {name} field gesture moved {moves:?} in ONE undo step")), Box::new(move |c, _| {
            let n = c.doc.history_labels().len();
            if n != h3.get() + 1 || c.doc.undo_label() != Some("Device Parameter Change") {
                return Err(format!("history {} -> {n}, last {:?}", h3.get(), c.doc.undo_label()));
            }
            let def = saine_core::builtin::find(id).ok_or("not registered")?;
            let still: Vec<&str> = moves.iter().copied().filter(|p| value(c, id, p) == def.param(p).map(|p| p.default)).collect();
            still.is_empty().then_some(()).ok_or(format!("unchanged: {still:?}"))
        })));
        steps.push(Do(leak(format!("restoration: ⌘Z after the {name} gesture")), Box::new(|a| key(a, "z", true, false))));
        steps.push(Check(leak(format!("restoration: undo restores every {name} parameter")), Box::new(move |c, _| {
            let def = saine_core::builtin::find(id).ok_or("not registered")?;
            let d = device(c, id).ok_or("no device")?;
            let off: Vec<&str> = def.params.iter().filter(|p| def.value(d, p) != p.default).map(|p| p.id).collect();
            let vals: Vec<(&str, f32)> = def.params.iter().map(|p| (p.id, def.value(d, p))).collect();
            off.is_empty().then_some(()).ok_or(format!("not at default: {off:?} values {vals:?} history {:?}", c.doc.history_labels().iter().rev().take(4).collect::<Vec<_>>()))
        })));
    }
    // The De-Esser's Listen latch in the dial row (the De-Esser is still in the chain from above).
    let hl = Rc::new(Cell::new(0usize));
    let hl2 = hl.clone();
    steps.push(DoC("restoration: click the De-Esser's Listen latch", Box::new(move |a, ctl| {
        {
            let mut c = ctl.borrow_mut();
            let Some(t) = track_of(&c, TrackKind::Audio) else { return };
            let ids: Vec<saine_core::DeviceId> = c.project().tracks[t].devices.iter().map(|d| d.id).collect();
            for d in ids {
                c.execute(saine_core::Command::RemoveDevice { device: d });
            }
            let tid = c.project().tracks[t].id;
            let def = saine_core::builtin::find("deesser").expect("registered");
            c.add_item_to_track(&crate::devices::Item::Builtin(def), tid, None);
            hl.set(c.doc.history_labels().len());
            c.update_ui(a);
        }
        let Some(cd) = card_named(a, "De-Esser") else { return };
        let z = crate::device_panel::Z;
        if let Some(k) = (0..slint::Model::row_count(&cd.knobs)).filter_map(|i| slint::Model::row_data(&cd.knobs, i)).find(|k| k.face == 2) {
            let (x, y) = card_xy(&cd, k.x + 31.0 * z, (229.0 + 24.0) * z);
            click(a, x, y);
        }
    })));
    steps.push(Check("restoration: the Listen latch turns Listen on (one undo step) and shows On", Box::new(move |c, a| {
        let n = c.doc.history_labels().len();
        let shown = card_named(a, "De-Esser").and_then(|cd| (0..slint::Model::row_count(&cd.knobs)).filter_map(|i| slint::Model::row_data(&cd.knobs, i)).find(|k| k.face == 2)).map(|k| k.value.to_string());
        (n == hl2.get() + 1 && value(c, "deesser", "listen") == Some(1.0) && shown.as_deref() == Some("On"))
            .then_some(())
            .ok_or(format!("history {} -> {n}, listen {:?}, latch {shown:?}", hl2.get(), value(c, "deesser", "listen")))
    })));
    steps
}

fn index_of(id: &str, param: &str) -> i32 {
    saine_core::builtin::find(id).and_then(|def| def.index_of(param)).map_or(-1, |i| i as i32)
}

/// Delay, Echo, Reverb, Pitch Shifter and Grains: added from the Built-in list, then
/// their latches, view switches and custom fields driven through the panel entry points.
fn time_fx_steps() -> Vec<Step> {
    use Step::*;
    const IDS: [&str; 5] = ["delay", "echo", "reverb", "pitch-shifter", "grains"];
    thread_local! {
        /// History length at the last mark (A) and a second mark inside one step (B).
        static A: Cell<usize> = const { Cell::new(0) };
        static B: Cell<usize> = const { Cell::new(0) };
    }
    fn mark(m: &'static std::thread::LocalKey<Cell<usize>>, c: &Controller) {
        m.with(|m| m.set(c.doc.history_labels().len()));
    }
    fn steps(m: &'static std::thread::LocalKey<Cell<usize>>, c: &Controller) -> usize {
        c.doc.history_labels().len() - m.with(|m| m.get())
    }
    fn id_of(c: &Controller, id: &str) -> Option<saine_core::DeviceId> {
        device_of(c, id).map(|d| d.id)
    }
    vec![
        Do("time fx: Escape (close menus)", Box::new(|a| key(a, Key::Escape, false, false))),
        DoC("time fx: select the audio track, clear its chain, Add device › Built-in", Box::new(|a, ctl| {
            let i = track_of(&ctl.borrow(), TrackKind::Audio).unwrap_or(0);
            click_track_header(a, ctl, i);
            if let Ok(mut c) = ctl.try_borrow_mut() {
                let ids: Vec<saine_core::DeviceId> = c.project().tracks[i].devices.iter().map(|d| d.id).collect();
                for d in ids {
                    c.execute(saine_core::Command::RemoveDevice { device: d });
                }
                c.update_ui(a);
            }
            let add_x = a.global::<Studio>().get_add_device_x();
            click(a, 5.0 + add_x + 60.0, BOTTOM_Y + 34.5 + 56.5 + 11.0);
        })),
        Check("time fx: the Built-in list offers Delay, Echo, Reverb, Pitch Shifter and Grains", Box::new(|_, a| {
            let items = devmenu_labels(a);
            let all = ["Delay", "Echo", "Reverb", "Pitch Shifter", "Grains"].iter().all(|n| items.iter().any(|r| r.0 == *n && r.2));
            all.then_some(()).ok_or(format!("items {items:?}"))
        })),
        DoC("time fx: pick Delay, add the other four", Box::new(|a, ctl| {
            mark(&A, &ctl.borrow());
            if let Some(i) = devmenu_labels(a).iter().position(|r| r.0 == "Delay") {
                let (x, y) = devmenu_row(a, i);
                click(a, x, y);
            }
            let mut c = ctl.borrow_mut();
            let Some(t) = track_of(&c, TrackKind::Audio).map(|i| c.project().tracks[i].id) else { return };
            for id in &IDS[1..] {
                c.add_item_to_track(&crate::devices::Item::Builtin(saine_core::builtin::find(id).unwrap()), t, None);
            }
        })),
        Check("time fx: five cards at their defaults, reference widths, each with its field, one undo step each", Box::new(|c, a| {
            let n = steps(&A, c);
            for (id, name, w) in [("delay", "Delay", 615.0), ("echo", "Echo", 521.0), ("reverb", "Reverb", 543.0), ("pitch-shifter", "Pitch Shifter", 332.0), ("grains", "Grains", 532.0)] {
                let cd = card_named(a, name).ok_or(format!("no {name} card"))?;
                if !cd.builtin || !cd.canvas.visible || slint::Model::row_count(&cd.canvas.paths) == 0 || !approx(cd.width, w * crate::device_panel::Z, 0.01) {
                    return Err(format!("{name}: builtin {} field {} width {}", cd.builtin, cd.canvas.visible, cd.width));
                }
                let d = device_of(c, id).ok_or(format!("no {id}"))?;
                let def = saine_core::builtin::def_of(d).unwrap();
                if def.params.iter().any(|p| def.value(d, p) != p.default) {
                    return Err(format!("{id} not at defaults"));
                }
            }
            (n == 5).then_some(()).ok_or(format!("history +{n}"))
        })),
        DoC("time fx: Delay Link latch off", Box::new(|_, ctl| {
            mark(&A, &ctl.borrow());
            let Some(d) = id_of(&ctl.borrow(), "delay") else { return };
            ctl.borrow_mut().panel_latch(d, index_of("delay", "link"), 0);
        })),
        Check("time fx: Link off, Note L / Note R named, Note R live; one undo step", Box::new(|c, a| {
            let cd = card_named(a, "Delay").ok_or("no card")?;
            let knobs: Vec<_> = (0..slint::Model::row_count(&cd.knobs)).filter_map(|i| slint::Model::row_data(&cd.knobs, i)).collect();
            let ok = value_of(c, "delay", "link") == Some(0.0)
                && knobs.iter().any(|k| k.label == "Note L")
                && knobs.iter().any(|k| k.label == "Note R" && !k.inert)
                && steps(&A, c) == 1;
            ok.then_some(()).ok_or(format!("link {:?} knobs {:?}", value_of(c, "delay", "link"), knobs.iter().map(|k| (k.label.to_string(), k.inert)).collect::<Vec<_>>()))
        })),
        DoC("time fx: drag the Delay echoes field 30 px down", Box::new(|_, ctl| {
            mark(&A, &ctl.borrow());
            let Some(d) = id_of(&ctl.borrow(), "delay") else { return };
            let mut c = ctl.borrow_mut();
            c.panel_field_pointer(d, 0, 60.0, 40.0, false, false);
            for i in 1..=30 {
                c.panel_field_pointer(d, 1, 60.0, 40.0 + i as f32, false, false);
            }
            c.panel_field_pointer(d, 2, 60.0, 70.0, false, false);
        })),
        Check("time fx: the echoes drag lowers Feedback, one undo step", Box::new(|c, _| {
            let fb = value_of(c, "delay", "feedback").unwrap_or(f32::NAN);
            (fb < 50.0 && fb > 0.0 && steps(&A, c) == 1).then_some(()).ok_or(format!("feedback {fb} history +{}", steps(&A, c)))
        })),
        DoC("time fx: Delay view EQ", Box::new(|_, ctl| {
            let Some(d) = id_of(&ctl.borrow(), "delay") else { return };
            ctl.borrow_mut().card_view_select(d, 1);
        })),
        Check("time fx: the EQ view is selected and drawn (UI state, no undo step)", Box::new(|c, a| {
            let cd = card_named(a, "Delay").ok_or("no card")?;
            let view = (0..slint::Model::row_count(&cd.picks)).filter_map(|i| slint::Model::row_data(&cd.picks, i)).find(|p| p.kind == 7).map(|p| p.selected);
            (view == Some(1) && slint::Model::row_count(&cd.canvas.grips) == 1 && steps(&A, c) == 1).then_some(()).ok_or(format!("view {view:?}"))
        })),
        DoC("time fx: hold the Echo Throw latch", Box::new(|_, ctl| {
            mark(&A, &ctl.borrow());
            let Some(d) = id_of(&ctl.borrow(), "echo") else { return };
            ctl.borrow_mut().panel_latch(d, index_of("echo", "throw"), 1);
        })),
        Check("time fx: Throw shows on while held, live only (model off, no undo step)", Box::new(|c, a| {
            let cd = card_named(a, "Echo").ok_or("no card")?;
            let on = (0..slint::Model::row_count(&cd.knobs)).filter_map(|i| slint::Model::row_data(&cd.knobs, i)).any(|k| k.label == "Throw" && k.latch && k.on);
            (on && value_of(c, "echo", "throw") == Some(0.0) && steps(&A, c) == 0).then_some(()).ok_or(format!("on {on}"))
        })),
        DoC("time fx: release Throw, then drag the first Echo head right", Box::new(|_, ctl| {
            let Some(d) = id_of(&ctl.borrow(), "echo") else { return };
            ctl.borrow_mut().panel_latch(d, index_of("echo", "throw"), 2);
            mark(&A, &ctl.borrow());
            // Defaults: heads 2+3 of the tape loop; the first repeat of head 2 sits at 0.2 s of a 3 s span.
            let (w, x) = {
                let c = ctl.borrow();
                let def = saine_core::builtin::find("echo").unwrap();
                let (_, _, w, _, _) = crate::panel_fields::field_rect(def, crate::device_panel::view_right(def));
                let pic = c.echo_picture(device_of(&c, "echo").unwrap(), def);
                let ms = pic.heads.iter().filter(|h| h.on).map(|h| h.ms).fold(f32::MAX, f32::min);
                let (left, right) = (40.0 * crate::device_panel::Z, w - 6.0 * crate::device_panel::Z);
                (w, left + ms / 1000.0 / 3.0 * (right - left))
            };
            let _ = w;
            let mut c = ctl.borrow_mut();
            c.panel_field_pointer(d, 0, x, 80.0, false, false);
            c.panel_field_pointer(d, 1, x + 20.0, 80.0, false, false);
            c.panel_field_pointer(d, 2, x + 20.0, 80.0, false, false);
        })),
        Check("time fx: Throw released; the head drag authors the heads (Heads, Heads Machine, pinned position) in one undo step", Box::new(|c, _| {
            let heads = value_of(c, "echo", "heads").unwrap_or(0.0);
            let machine = value_of(c, "echo", "heads_machine").unwrap_or(0.0);
            let ok = value_of(c, "echo", "throw") == Some(0.0) && heads >= 1.0 && machine == 1.0 && steps(&A, c) == 1;
            ok.then_some(()).ok_or(format!("heads {heads} machine {machine} history +{}", steps(&A, c)))
        })),
        DoC("time fx: Reverb Mode › Shimmer", Box::new(|_, ctl| {
            mark(&A, &ctl.borrow());
            let Some(d) = id_of(&ctl.borrow(), "reverb") else { return };
            ctl.borrow_mut().panel_choice(d, index_of("reverb", "mode2"), 4);
        })),
        Check("time fx: Shimmer dial row [Decay] [Pre-Delay, Mod Depth] [Shift 1, Shift 2, Shimmer] [Mix]", Box::new(|c, a| {
            let cd = card_named(a, "Reverb").ok_or("no card")?;
            let labels: Vec<String> = (0..slint::Model::row_count(&cd.knobs)).filter_map(|i| slint::Model::row_data(&cd.knobs, i)).map(|k| k.label.to_string()).collect();
            let want = ["Decay", "Pre-Delay", "Mod Depth", "Shift 1", "Shift 2", "Shimmer", "Mix"];
            (labels == want && steps(&A, c) == 1).then_some(()).ok_or(format!("{labels:?}"))
        })),
        DoC("time fx: drag the Reverb pre-delay line 20 px right", Box::new(|_, ctl| {
            mark(&A, &ctl.borrow());
            let Some(d) = id_of(&ctl.borrow(), "reverb") else { return };
            // Pre-delay 10 ms on a span of a few seconds: the line sits at the left edge of the plot.
            let x = 26.0 * crate::device_panel::Z + 1.0;
            let mut c = ctl.borrow_mut();
            c.panel_field_pointer(d, 0, x, 60.0, false, false);
            c.panel_field_pointer(d, 1, x + 20.0, 60.0, false, false);
            c.panel_field_pointer(d, 2, x + 20.0, 60.0, false, false);
        })),
        Check("time fx: Pre-Delay grows, one undo step", Box::new(|c, _| {
            let v = value_of(c, "reverb", "predelay").unwrap_or(f32::NAN);
            (v > 10.0 && steps(&A, c) == 1).then_some(()).ok_or(format!("pre-delay {v}"))
        })),
        DoC("time fx: press the Pitch Shifter field at +7 st", Box::new(|_, ctl| {
            mark(&A, &ctl.borrow());
            let Some(d) = id_of(&ctl.borrow(), "pitch-shifter") else { return };
            let def = saine_core::builtin::find("pitch-shifter").unwrap();
            let mut c = ctl.borrow_mut();
            let (_, _, w, _, _) = crate::panel_fields::field_rect(def, crate::device_panel::view_right(def));
            let x = (7.0 + 24.5) / 49.0 * w;
            c.panel_field_pointer(d, 0, x - 10.0, 40.0, false, false);
            c.panel_field_pointer(d, 1, x, 40.0, false, false);
            c.panel_field_pointer(d, 2, x, 40.0, false, false);
        })),
        Check("time fx: Semitones +7, the caption reads Fifth up, one undo step", Box::new(|c, a| {
            let cd = card_named(a, "Pitch Shifter").ok_or("no card")?;
            let st = value_of(c, "pitch-shifter", "semitones");
            (st == Some(7.0) && cd.canvas.readout == "Fifth up" && steps(&A, c) == 1).then_some(()).ok_or(format!("semitones {st:?} caption {}", cd.canvas.readout))
        })),
        DoC("time fx: Grains Freeze latch, then drag Position 30 px left and press Escape", Box::new(|a, ctl| {
            mark(&A, &ctl.borrow());
            let Some(d) = id_of(&ctl.borrow(), "grains") else { return };
            ctl.borrow_mut().panel_latch(d, index_of("grains", "freeze"), 0);
            mark(&B, &ctl.borrow());
            let def = saine_core::builtin::find("grains").unwrap();
            let x = {
                let (_, _, w, _, _) = crate::panel_fields::field_rect(def, crate::device_panel::view_right(def));
                let right = w - 5.0 * crate::device_panel::Z;
                // Frozen defaults: reach 0.25 s on a 0.5 s span.
                right - 0.5 * (right - 4.0 * crate::device_panel::Z)
            };
            ctl.borrow_mut().panel_field_pointer(d, 0, x, 50.0, false, false);
            ctl.borrow_mut().panel_field_pointer(d, 1, x - 30.0, 50.0, false, false);
            // The press focuses the field in the UI; here the pointer is synthetic, so run the Escape action.
            let _ = a;
            crate::run_action(ctl, "view.escape");
            ctl.borrow_mut().panel_field_pointer(d, 2, x - 30.0, 50.0, false, false);
        })),
        Check("time fx: Freeze on (one undo step); Escape cancels the Position drag (250 ms, no undo step)", Box::new(|c, _| {
            let pos = value_of(c, "grains", "position").unwrap_or(f32::NAN);
            let ok = value_of(c, "grains", "freeze") == Some(1.0) && pos == 250.0 && steps(&A, c) == 1 && steps(&B, c) == 0;
            ok.then_some(()).ok_or(format!("freeze {:?} position {pos}", value_of(c, "grains", "freeze")))
        })),
        DoC("time fx: remove the five devices", Box::new(|_, ctl| {
            let mut c = ctl.borrow_mut();
            let ids: Vec<saine_core::DeviceId> = IDS.iter().filter_map(|id| device_of(&c, id).map(|d| d.id)).collect();
            for d in ids {
                c.execute(saine_core::Command::RemoveDevice { device: d });
            }
        })),
        Check("time fx: the audio track's chain is empty again", Box::new(|c, _| {
            let n = track_of(c, TrackKind::Audio).map_or(usize::MAX, |t| c.project().tracks[t].devices.len());
            (n == 0).then_some(()).ok_or(format!("{n} devices left"))
        })),
    ]
}

/// The devices section drives Apple's Audio Units (AULowpass, AUDelay,
/// DLSMusicDevice). Elsewhere its actions are not performed and its checks
/// are reported as skipped (counted like the other guarded skips).
fn platform_device_steps() -> Vec<Step> {
    if cfg!(target_os = "macos") {
        return device_steps();
    }
    device_steps()
        .into_iter()
        .filter_map(|s| match s {
            Step::Check(name, _) => Some(Step::Check(name, Box::new(|_, _| Err("SKIP: Audio Units are macOS-only (this section uses Apple's AUs)".into())))),
            Step::Do(..) | Step::DoC(..) => None,
        })
        .collect()
}

// ------------------------------------------------------------------ built-in samplers

fn inst_device(c: &Controller) -> Option<saine_core::Device> {
    let t = track_of(c, TrackKind::Instrument)?;
    c.project().tracks[t].instrument_plugin.clone()
}

fn sampler_card(a: &AppWindow) -> Option<crate::DeviceCard> {
    cards(a).into_iter().find(|c| c.sampler.visible)
}

/// Drum Rack, Pad Sampler and Sampler in the instrument slot: added from the
/// Built-in list, files loaded as drops, pads pressed, slices and a picture handle.
fn sampler_steps() -> Vec<Step> {
    use Step::*;
    let wav = std::env::temp_dir().join(format!("saine-selftest-sampler-{}.wav", std::process::id()));
    let _ = crate::wavgen::write_wav(&wav, 48_000, 1, &crate::wavgen::sine_bursts(48_000));
    let hist = Rc::new(Cell::new(0usize));
    fn one(hc: &Rc<Cell<usize>>, c: &Controller, label: &str) -> Result<(), String> {
        let n = c.doc.history_labels().len();
        let was = hc.get();
        hc.set(n);
        (n == was + 1 && c.doc.undo_label() == Some(label)).then_some(()).ok_or(format!("history {was} -> {n}, last {:?} (want {label})", c.doc.undo_label()))
    }
    let (h0, h1, h2, h3, h4, h5, h6) = (hist.clone(), hist.clone(), hist.clone(), hist.clone(), hist.clone(), hist.clone(), hist.clone());
    let (w1, w2, w3, w4) = (wav.clone(), wav.clone(), wav.clone(), wav);
    let h4b = h4.clone();
    let (hp, hp2) = (hist.clone(), hist.clone());
    vec![
        Do("sampler: Escape (close menus)", Box::new(|a| key(a, Key::Escape, false, false))),
        DoC("sampler: select the instrument track, Devices tab, Add device › Built-in", Box::new(move |a, ctl| {
            {
                let mut c = ctl.borrow_mut();
                let t = track_of(&c, TrackKind::Instrument).map(|i| c.project().tracks[i].id);
                if let Some(t) = t {
                    c.select_track(t);
                }
                c.set_bottom_tab(crate::controller::BottomTab::Devices);
                c.devmenu_dismiss();
                h0.set(c.doc.history_labels().len());
                // The chain is long here (Synth + plugin inserts): open Add device › Built-in directly.
                c.add_category(0, 600.0, 520.0);
                c.update_ui(a);
            }
        })),
        Check("sampler: the Built-in list offers Sampler, Drum Rack and Pad Sampler", Box::new(|c, a| {
            let items = devmenu_labels(a);
            let has = |n: &str| items.iter().any(|r| r.0 == n && r.2);
            (a.global::<Studio>().get_devmenu_open() && has("Sampler") && has("Drum Rack") && has("Pad Sampler")).then_some(()).ok_or(format!("items {items:?} selected {:?}", c.selected_track))
        })),
        Do("sampler: pick Drum Rack", Box::new(|a| {
            if let Some(i) = devmenu_labels(a).iter().position(|r| r.0 == "Drum Rack") {
                let (x, y) = devmenu_row(a, i);
                click(a, x, y);
            }
        })),
        Check("sampler: Drum Rack is the track's instrument (one undo step); card has 16 pads, 5 knobs, an empty well; engine instance live (48 readouts)", Box::new(move |c, a| {
            one(&h1, c, "Load Drum Rack")?;
            let d = inst_device(c).ok_or("no instrument")?;
            let cd = sampler_card(a).ok_or("no sampler card")?;
            let s = &cd.sampler;
            let mut r = Vec::new();
            c.engine.builtin_readouts(d.id.0, &mut r).map_err(|e| e.to_string())?;
            let ok = d.plugin.id == "drum-rack" && s.kind == 2 && slint::Model::row_count(&s.pads) == 16 && slint::Model::row_count(&s.knobs) == 5 && !s.loaded && r.len() == 48;
            ok.then_some(()).ok_or(format!("id {} kind {} pads {} knobs {} loaded {} readouts {}", d.plugin.id, s.kind, slint::Model::row_count(&s.pads), slint::Model::row_count(&s.knobs), s.loaded, r.len()))
        })),
        DoC("sampler: drop a WAV on the window (pad 1)", Box::new(move |_, ctl| ctl.borrow_mut().drop_files(vec![w1.clone()]))),
        Check("sampler: the file is pad 1's first sound (asset + sample, one undo step); the pad and well show it", Box::new(move |c, a| {
            one(&h2, c, "Load Sample")?;
            let d = inst_device(c).ok_or("no instrument")?;
            let cd = sampler_card(a).ok_or("no card")?;
            let pad = slint::Model::row_data(&cd.sampler.pads, 0).ok_or("no pad")?;
            let ok = d.sample(0).is_some() && pad.filled && cd.sampler.loaded && !cd.sampler.wave.is_empty() && cd.sampler.layer_text == "1/1";
            ok.then_some(()).ok_or(format!("sample {:?} pad {} well {} layer {}", d.sample(0).map(|s| &s.name), pad.filled, cd.sampler.loaded, cd.sampler.layer_text))
        })),
        DoC("sampler: drop it again (second sound of pad 1)", Box::new(move |_, ctl| ctl.borrow_mut().drop_files(vec![w2.clone()]))),
        Check("sampler: pad 1 holds two alternating sounds (slots 0 and 1); the panel shows sound 2/2", Box::new(move |c, a| {
            one(&h3, c, "Load Sample")?;
            let d = inst_device(c).ok_or("no instrument")?;
            let cd = sampler_card(a).ok_or("no card")?;
            (d.sample(1).is_some() && cd.sampler.layer_text == "2/2").then_some(()).ok_or(format!("slot1 {} layer {}", d.sample(1).is_some(), cd.sampler.layer_text))
        })),
        Do("sampler: press the D1 pad", Box::new(|a| {
            if let Some(cd) = sampler_card(a)
                && let Some(p) = slint::Model::row_data(&cd.sampler.pads, 2)
            {
                let (x, y) = card_xy(&cd, p.x + p.w / 2.0, p.y + p.h / 2.0);
                click(a, x, y);
            }
        })),
        Check("sampler: D1 is selected (UI only, no undo step): the sub-card shows Pad 3, Mapped to D1, an empty well, and edits p3_ parameters", Box::new(|c, a| {
            let cd = sampler_card(a).ok_or("no card")?;
            let s = &cd.sampler;
            let def = saine_core::builtin::find("drum-rack").unwrap();
            let k = slint::Model::row_data(&s.knobs, 0).ok_or("no knob")?;
            let ok = s.layer_name == "Pad 3" && s.mapped == "Mapped to D1" && !s.loaded && def.params[k.param as usize].id == "p3_gain" && c.doc.undo_label() == Some("Load Sample");
            ok.then_some(()).ok_or(format!("{} / {} / loaded {} / knob {}", s.layer_name, s.mapped, s.loaded, def.params[k.param as usize].id))
        })),
        DoC("sampler: load the Pad Sampler as the instrument and drop the WAV on it", Box::new(move |_, ctl| {
            let mut c = ctl.borrow_mut();
            let t = track_of(&c, TrackKind::Instrument).map(|i| c.project().tracks[i].id);
            if let Some(t) = t {
                c.add_item_to_track(&crate::devices::Item::Builtin(saine_core::builtin::find("pad-sampler").unwrap()), t, None);
            }
            hp.set(c.doc.history_labels().len());
            c.drop_files(vec![w4.clone()]);
        })),
        Check("sampler: the Pad Sampler holds the file as sound 1/1, answers every key, and shows its 5 knobs", Box::new(move |c, a| {
            one(&hp2, c, "Load Sample")?;
            let d = inst_device(c).ok_or("no instrument")?;
            let cd = sampler_card(a).ok_or("no card")?;
            let s = &cd.sampler;
            let ok = d.plugin.id == "pad-sampler" && d.sample(0).is_some() && s.kind == 1 && s.loaded && s.layer_text == "1/1" && s.mapped == "Mapped to all keys" && slint::Model::row_count(&s.knobs) == 5;
            ok.then_some(()).ok_or(format!("id {} kind {} loaded {} layer {} mapped {}", d.plugin.id, s.kind, s.loaded, s.layer_text, s.mapped))
        })),
        DoC("sampler: load the Sampler as the instrument and drop the WAV on it", Box::new(move |_, ctl| {
            let mut c = ctl.borrow_mut();
            let t = track_of(&c, TrackKind::Instrument).map(|i| c.project().tracks[i].id);
            if let Some(t) = t {
                c.add_item_to_track(&crate::devices::Item::Builtin(saine_core::builtin::find("sampler-steam").unwrap()), t, None);
            }
            h4b.set(c.doc.history_labels().len());
            c.drop_files(vec![w3.clone()]);
        })),
        Check("sampler: the Sampler plays the file (One Shot); tabs Sample..Mod; start/end read in seconds (0.00 s / 4.00 s)", Box::new(move |c, a| {
            one(&h4, c, "Load Sample")?;
            let d = inst_device(c).ok_or("no instrument")?;
            let cd = sampler_card(a).ok_or("no card")?;
            let s = &cd.sampler;
            let v: Vec<String> = slint::Model::iter(&s.knobs).map(|k| k.value.to_string()).collect();
            let ok = d.plugin.id == "sampler-steam" && d.sample(0).is_some() && s.loaded && slint::Model::row_count(&s.tab_pick.choices) == 5 && v.first().map(String::as_str) == Some("0.00 s") && v.get(1).map(String::as_str) == Some("4.00 s");
            ok.then_some(()).ok_or(format!("id {} loaded {} knobs {v:?}", d.plugin.id, s.loaded))
        })),
        DoC("sampler: switch the mode to Slice", Box::new(|_, ctl| {
            let mut c = ctl.borrow_mut();
            if let Some(d) = inst_device(&c) {
                let i = saine_core::builtin::find("sampler-steam").unwrap().index_of("mode").unwrap();
                c.panel_choice(d.id, i as i32, 2);
            }
        })),
        Check("sampler: Slice mode chops the sample by beat (quarter notes at 120 BPM: 8 slices of 4 s) in the same undo step", Box::new(move |c, _| {
            one(&h5, c, "Device Parameter Change")?;
            let d = inst_device(c).ok_or("no instrument")?;
            let n = d.sample(0).map_or(0, |s| s.slices.len());
            (n == 8).then_some(()).ok_or(format!("{n} slices"))
        })),
        DoC("sampler: Amp tab, drag the attack handle to the right", Box::new(|a, ctl| {
            let id = inst_device(&ctl.borrow()).map(|d| d.id);
            let Some(id) = id else { return };
            ctl.borrow_mut().sampler_tab(id, 3);
            if let Ok(mut c) = ctl.try_borrow_mut() {
                c.update_ui(a);
            }
            let Some(h) = sampler_card(a).and_then(|cd| slint::Model::row_data(&cd.sampler.handles, 0)) else { return };
            let mut c = ctl.borrow_mut();
            c.sampler_handle(id, 0, 0, h.x, h.y);
            c.sampler_handle(id, 0, 1, h.x + 40.0, h.y);
            c.sampler_handle(id, 0, 2, h.x + 40.0, h.y);
        })),
        Check("sampler: the attack handle drag is one undo step (Amp Envelope) and lengthens the attack", Box::new(move |c, _| {
            one(&h6, c, "Amp Envelope")?;
            let d = inst_device(c).ok_or("no instrument")?;
            let def = saine_core::builtin::def_of(&d).ok_or("def")?;
            let v = def.value(&d, def.param("ampAttack").unwrap());
            (v > 2.0).then_some(()).ok_or(format!("attack {v} ms"))
        })),
    ]
}

fn named_builtin<'a>(c: &'a Controller, id: &str) -> Option<&'a saine_core::Device> {
    let t = track_of(c, TrackKind::Audio)?;
    c.project().tracks[t].devices.iter().find(|d| saine_core::builtin::def_of(d).is_some_and(|def| def.id == id))
}

fn named_value(c: &Controller, id: &str, param: &str) -> Option<f32> {
    let d = named_builtin(c, id)?;
    let def = saine_core::builtin::def_of(d)?;
    Some(def.value(d, def.param(param)?))
}

/// The modulation devices: each one added from Add device › Built-in, checked
/// live in the engine with its generated field card, then removed; plus the
/// Tremolo field drag (one undo step for both axes), the Sync swap of the Rate
/// knob, and Character Mod's circuit switch moving untouched artifacts.
fn modulation_device_steps() -> Vec<Step> {
    use Step::*;
    const DEVICES: [(&str, &str, usize); 7] =
        [("tremolo", "Tremolo", 4), ("autopanner", "Auto-Panner", 4), ("chorus", "Chorus", 5), ("flanger", "Flanger", 5), ("phaser", "Phaser", 5), ("ring-mod", "Ring Mod", 5), ("character-mod", "Character Mod", 5)];
    let hist = Rc::new(Cell::new(0usize));
    fn one(hc: &Rc<Cell<usize>>, c: &Controller, label: &str) -> Result<(), String> {
        let n = c.doc.history_labels().len();
        let was = hc.get();
        hc.set(n);
        (n == was + 1 && c.doc.undo_label() == Some(label)).then_some(()).ok_or(format!("history {was} -> {n}, last {:?} (want {label})", c.doc.undo_label()))
    }
    let id_of = |ctl: &Ctl, id: &str| named_builtin(&ctl.borrow(), id).map(|d| d.id.0 as i32);
    let refresh = |a: &AppWindow, ctl: &Ctl| {
        if let Ok(mut c) = ctl.try_borrow_mut() {
            c.update_ui(a);
        }
    };
    let mut steps = vec![DoC("modulation: remove the Compressor", Box::new(move |a, ctl| {
        if let Some(id) = id_of(ctl, "compressor") {
            a.global::<Studio>().invoke_device_remove(id);
        }
        refresh(a, ctl);
    }))];
    for (id, name, knobs) in DEVICES {
        let (h0, h1) = (hist.clone(), hist.clone());
        steps.push(DoC(leak(format!("modulation: Add device › Built-in › {name}")), Box::new(move |a, ctl| {
            h0.set(ctl.borrow().doc.history_labels().len());
            refresh(a, ctl);
            let add_x = a.global::<Studio>().get_add_device_x();
            click(a, 5.0 + add_x + 60.0, BOTTOM_Y + 34.5 + 56.5 + 11.0);
            refresh(a, ctl);
            if let Some(i) = devmenu_labels(a).iter().position(|r| r.0 == name) {
                let (x, y) = devmenu_row(a, i);
                click(a, x, y);
            }
        })));
        steps.push(Check(leak(format!("{name}: added at its defaults in one step; field card with {knobs} knobs; live in the engine (LFO readout)")), Box::new(move |c, a| {
            one(&h1, c, &format!("Add {name}"))?;
            let d = named_builtin(c, id).ok_or("not added")?;
            let def = saine_core::builtin::def_of(d).ok_or("no def")?;
            let cd = card_named(a, name).ok_or("no card")?;
            let mut r = Vec::new();
            c.engine.builtin_readouts(d.id.0, &mut r).map_err(|e| e.to_string())?;
            let errors = c.engine.plugin_load_errors();
            let ok = cd.field.visible
                && cd.field.main != ""
                && slint::Model::row_count(&cd.knobs) == knobs
                && approx(cd.width, def.layout.width() * crate::device_panel::Z, 0.01)
                && r.len() == 1
                && !errors.iter().any(|(e, _)| *e == d.id);
            ok.then_some(()).ok_or(format!("field {} knobs {} width {} readouts {r:?} errors {errors:?}", cd.field.visible, slint::Model::row_count(&cd.knobs), cd.width))
        })));
        match id {
            "tremolo" => {
                let (h2, h3) = (hist.clone(), hist.clone());
                steps.push(DoC("tremolo: drag the field 40 px right and 30 px up", Box::new(|a, _| {
                    if let Some(cd) = card_named(a, "Tremolo") {
                        let f = &cd.field;
                        let (x, y) = card_xy(&cd, f.x + f.plot_x + f.plot_w / 2.0, f.y + f.plot_y + f.plot_h / 2.0);
                        drag(a, x, y, x + 40.0, y - 30.0);
                    }
                })));
                steps.push(Check("tremolo: Rate and Depth both moved, ONE undo step for the drag", Box::new(move |c, _| {
                    one(&h2, c, "Device Parameter Change")?;
                    let (r, d) = (named_value(c, "tremolo", "rate").unwrap_or(0.0), named_value(c, "tremolo", "depth").unwrap_or(0.0));
                    (r > 5.5 && d > 75.0).then_some(()).ok_or(format!("rate {r} depth {d}"))
                })));
                // Through the controller: the Add device search field keeps keyboard focus after an add.
                steps.push(DoC("tremolo: undo", Box::new(move |a, ctl| {
                    ctl.borrow_mut().undo();
                    refresh(a, ctl);
                })));
                steps.push(Check("tremolo: one undo restores Rate 5 Hz and Depth 70%", Box::new(move |c, _| {
                    h3.set(c.doc.history_labels().len());
                    let (r, d) = (named_value(c, "tremolo", "rate").unwrap_or(0.0), named_value(c, "tremolo", "depth").unwrap_or(0.0));
                    (r == 5.0 && d == 70.0).then_some(()).ok_or(format!("rate {r} depth {d}"))
                })));
                let h4 = hist.clone();
                steps.push(DoC("tremolo: Timebase › Sync", Box::new(move |a, ctl| {
                    let Some(id) = id_of(ctl, "tremolo") else { return };
                    let p = saine_core::builtin::find("tremolo").and_then(|d| d.index_of("sync")).map_or(-1, |i| i as i32);
                    a.global::<Studio>().invoke_panel_choice(id, p, 1);
                    refresh(a, ctl);
                })));
                steps.push(Check("tremolo: Sync on swaps the Rate knob for Rate Sync (1/4)", Box::new(move |c, a| {
                    one(&h4, c, "Device Parameter Change")?;
                    let k = card_named(a, "Tremolo").and_then(|cd| slint::Model::row_data(&cd.knobs, 0)).ok_or("no knob")?;
                    (k.value == "1/4" && k.label == "Rate").then_some(()).ok_or(format!("knob {} {}", k.label, k.value))
                })));
            }
            "character-mod" => {
                let h2 = hist.clone();
                steps.push(DoC("character mod: Character dropdown › Phaser", Box::new(move |a, ctl| {
                    let Some(id) = id_of(ctl, "character-mod") else { return };
                    let p = saine_core::builtin::find("character-mod").and_then(|d| d.index_of("model")).map_or(-1, |i| i as i32);
                    a.global::<Studio>().invoke_panel_menu(id, p, 600.0, 600.0);
                    refresh(a, ctl);
                    if let Some(i) = devmenu_labels(a).iter().position(|r| r.0 == "Phaser") {
                        let (x, y) = devmenu_row(a, i);
                        click(a, x, y);
                    }
                })));
                steps.push(Check("character mod: Phaser chosen, untouched Hiss moved to 0 in the same undo step; Mode now lists Phaser modes", Box::new(move |c, a| {
                    one(&h2, c, "Character Change, with Hiss moved to its own defaults")?;
                    let (m, hiss) = (named_value(c, "character-mod", "model"), named_value(c, "character-mod", "art_hiss"));
                    let mode = card_named(a, "Character Mod")
                        .and_then(|cd| (0..slint::Model::row_count(&cd.picks)).filter_map(|i| slint::Model::row_data(&cd.picks, i)).find(|p| p.label == "Mode"))
                        .map(|p| p.value.to_string())
                        .unwrap_or_default();
                    (m == Some(7.0) && hiss == Some(0.0) && mode == "Four Plain").then_some(()).ok_or(format!("model {m:?} hiss {hiss:?} mode {mode:?}"))
                })));
            }
            _ => {}
        }
        let h5 = hist.clone();
        steps.push(DoC(leak(format!("modulation: remove {name}")), Box::new(move |a, ctl| {
            if let Some(i) = id_of(ctl, id) {
                a.global::<Studio>().invoke_device_remove(i);
            }
            refresh(a, ctl);
            h5.set(ctl.borrow().doc.history_labels().len());
        })));
    }
    steps
}

/// Window point of a canvas-local point of a card's canvas view.
fn canvas_xy(cd: &crate::DeviceCard, x: f32, y: f32) -> (f32, f32) {
    card_xy(cd, cd.canvas.x + x, cd.canvas.y + cd.canvas.well_y + y)
}

/// Opens Add device › Built-in on the first audio track and picks \`name\`.
fn add_builtin(a: &AppWindow, ctl: &Ctl, name: &str) {
    if let Ok(mut c) = ctl.try_borrow_mut() {
        c.update_ui(a);
    }
    let add_x = a.global::<Studio>().get_add_device_x();
    click(a, 5.0 + add_x + 60.0, BOTTOM_Y + 34.5 + 56.5 + 11.0);
    if let Ok(mut c) = ctl.try_borrow_mut() {
        c.update_ui(a);
    }
    if let Some(i) = devmenu_labels(a).iter().position(|r| r.0 == name) {
        let (x, y) = devmenu_row(a, i);
        click(a, x, y);
    }
}

/// Picks row \`label\` of the parameter menu of \`param\` (a device panel pill).
fn pick_param_menu(a: &AppWindow, ctl: &Ctl, device: &str, param: &str, label: &str) {
    let ids = {
        let c = ctl.borrow();
        named_builtin(&c, device).and_then(|d| saine_core::builtin::def_of(d).and_then(|def| def.index_of(param)).map(|i| (d.id.0 as i32, i as i32)))
    };
    let Some((id, p)) = ids else { return };
    a.global::<Studio>().invoke_panel_menu(id, p, 600.0, 600.0);
    if let Ok(mut c) = ctl.try_borrow_mut() {
        c.update_ui(a);
    }
    if let Some(i) = devmenu_labels(a).iter().position(|r| r.0 == label) {
        let (x, y) = devmenu_row(a, i);
        click(a, x, y);
    }
}

/// Channel EQ, Character EQ and Auto Filter driven through their canvas views.
fn field_device_steps() -> Vec<Step> {
    use Step::*;
    let hist = Rc::new(Cell::new(0usize));
    let h = || hist.clone();
    fn one(hc: &Rc<Cell<usize>>, c: &Controller, label: &str) -> Result<(), String> {
        let n = c.doc.history_labels().len();
        let was = hc.get();
        hc.set(n);
        (n == was + 1 && c.doc.undo_label() == Some(label)).then_some(()).ok_or(format!("history {was} -> {n}, last {:?} (want {label})", c.doc.undo_label()))
    }
    fn live(c: &Controller, id: &str) -> Result<(), String> {
        let d = named_builtin(c, id).ok_or("no device")?.id;
        let mut r = Vec::new();
        c.engine.builtin_readouts(d.0, &mut r).map_err(|e| e.to_string())?;
        let errors = c.engine.plugin_load_errors();
        (!errors.iter().any(|(x, _)| *x == d)).then_some(()).ok_or(format!("load errors {errors:?}"))
    }
    let (h0, h1, h2, h3, h4, h5, h6, h7, h8, h9, h10) = (h(), h(), h(), h(), h(), h(), h(), h(), h(), h(), h());
    let (h1b, h4b, h8b) = (h1.clone(), h4.clone(), h8.clone());
    let (h5b, h7b, h10b) = (h(), h(), h());
    vec![
        Do("eq: Escape (close menus)", Box::new(|a| key(a, Key::Escape, false, false))),
        DoC("eq: select the first audio track, clear its chain, add Channel EQ from Add device › Built-in", Box::new(move |a, ctl| {
            let i = track_of(&ctl.borrow(), TrackKind::Audio).unwrap_or(0);
            click_track_header(a, ctl, i);
            let ids: Vec<saine_core::DeviceId> = ctl.borrow().project().tracks[i].devices.iter().map(|d| d.id).collect();
            for id in ids {
                ctl.borrow_mut().device_remove(id);
            }
            h0.set(ctl.borrow().doc.history_labels().len());
            add_builtin(a, ctl, "Channel EQ");
        })),
        Check("eq: Channel EQ added in one undo step; card has the EQ field, 8 band tabs, 3 cells, the Output slider, 528 pt wide", Box::new(move |c, a| {
            one(&h1b, c, "Add Channel EQ")?;
            live(c, "channeleq")?;
            let cd = card_named(a, "Channel EQ").ok_or("no card")?;
            let ok = cd.canvas.visible && cd.eq.visible && slint::Model::row_count(&cd.eq.tabs) == 8 && slint::Model::row_count(&cd.eq.cells) == 3 && slint::Model::row_count(&cd.sliders) == 1 && approx(cd.width, 528.0 * crate::device_panel::Z, 0.01) && cd.eq.selected == 2;
            ok.then_some(()).ok_or(format!("field {} eq {} tabs {} width {}", cd.canvas.visible, cd.eq.visible, slint::Model::row_count(&cd.eq.tabs), cd.width))
        })),
        DoC("eq: drag the band 3 node up 40 px", Box::new(|a, _| {
            let Some(cd) = card_named(a, "Channel EQ") else { return };
            // The selected node is drawn as a ring dot (ring > 0) then the node dot.
            let Some(dot) = (0..slint::Model::row_count(&cd.canvas.dots)).filter_map(|i| slint::Model::row_data(&cd.canvas.dots, i)).find(|d| d.ring > 0.0) else { return };
            let (x, y) = canvas_xy(&cd, dot.x, dot.y);
            drag(a, x, y, x, y - 40.0);
        })),
        Check("eq: band 3 gain rises, frequency stays at 100 Hz, ONE undo step", Box::new(move |c, _| {
            one(&h2, c, "Device Parameter Change")?;
            let (g, f) = (named_value(c, "channeleq", "band2_gain2").unwrap_or(f32::NAN), named_value(c, "channeleq", "band2_freq").unwrap_or(f32::NAN));
            (g > 3.0 && approx(f, 100.0, 1.0)).then_some(()).ok_or(format!("gain {g} freq {f}"))
        })),
        DoC("eq: click band tab 5, then the Bell glyph's neighbour (Low Shelf) in the type picker", Box::new(|a, ctl| {
            let Some(id) = named_builtin(&ctl.borrow(), "channeleq").map(|d| d.id.0 as i32) else { return };
            a.global::<Studio>().invoke_panel_eq(id, 0, 4);
            if let Ok(mut c) = ctl.try_borrow_mut() {
                c.update_ui(a);
            }
            let Some(cd) = card_named(a, "Channel EQ") else { return };
            if let Some(g) = (0..slint::Model::row_count(&cd.eq.glyphs)).filter_map(|i| slint::Model::row_data(&cd.eq.glyphs, i)).find(|g| g.name == "Low Shelf") {
                a.global::<Studio>().invoke_panel_eq(id, 2, g.choice);
            }
        })),
        Check("eq: band 5 is selected and becomes a Low Shelf in one undo step", Box::new(move |c, a| {
            one(&h3, c, "Device Parameter Change")?;
            let cd = card_named(a, "Channel EQ").ok_or("no card")?;
            let t = named_value(c, "channeleq", "band4_type").unwrap_or(f32::NAN);
            (cd.eq.selected == 4 && t == 1.0).then_some(()).ok_or(format!("selected {} type {t}", cd.eq.selected))
        })),
        DoC("eq: double-click the band 3 node (parks it)", Box::new(move |a, ctl| {
            h4.set(ctl.borrow().doc.history_labels().len());
            let Some(id) = named_builtin(&ctl.borrow(), "channeleq").map(|d| d.id.0 as i32) else { return };
            a.global::<Studio>().invoke_panel_eq(id, 0, 2);
            if let Ok(mut c) = ctl.try_borrow_mut() {
                c.update_ui(a);
            }
            let Some(cd) = card_named(a, "Channel EQ") else { return };
            let Some(dot) = (0..slint::Model::row_count(&cd.canvas.dots)).filter_map(|i| slint::Model::row_data(&cd.canvas.dots, i)).find(|d| d.ring > 0.0) else { return };
            let (x, y) = canvas_xy(&cd, dot.x, dot.y);
            double_click(a, x, y);
        })),
        Check("eq: band 3 is off, its gain kept", Box::new(move |c, _| {
            let (on, g) = (named_value(c, "channeleq", "band2_enabled").unwrap_or(f32::NAN), named_value(c, "channeleq", "band2_gain2").unwrap_or(f32::NAN));
            let n = c.doc.history_labels().len();
            (on == 0.0 && g > 3.0 && n > h4b.get()).then_some(()).ok_or(format!("on {on} gain {g}"))
        })),
        DoC("eq: replace it with Character EQ (Add device › Built-in)", Box::new(move |a, ctl| {
            let old = named_builtin(&ctl.borrow(), "channeleq").map(|d| d.id);
            if let Some(id) = old {
                ctl.borrow_mut().device_remove(id);
            }
            h5.set(ctl.borrow().doc.history_labels().len());
            add_builtin(a, ctl, "Character EQ");
        })),
        Check("eq: Character EQ added; 12 band tabs, a Bank menu, no band enabled", Box::new(move |c, a| {
            one(&h5b, c, "Add Character EQ")?;
            live(c, "character-eq")?;
            let cd = card_named(a, "Character EQ").ok_or("no card")?;
            let ok = slint::Model::row_count(&cd.eq.tabs) == 12 && cd.eq.bank_param >= 0 && !cd.eq.on && approx(cd.width, 575.0 * crate::device_panel::Z, 0.01);
            ok.then_some(()).ok_or(format!("tabs {} bank {} on {}", slint::Model::row_count(&cd.eq.tabs), cd.eq.bank_param, cd.eq.on))
        })),
        DoC("eq: click empty field space at 2 kHz, +6 dB", Box::new(|a, _| {
            let Some(cd) = card_named(a, "Character EQ") else { return };
            let (w, h) = (cd.canvas.w, cd.canvas.h);
            let x = w * (100f32).log10() / 1000f32.log10();
            let y = (24.0 - 6.0) / 48.0 * h;
            let (x, y) = canvas_xy(&cd, x, y);
            click(a, x, y);
        })),
        Check("eq: band 1 created there (on, about 2 kHz, about +6 dB) in one undo step", Box::new(move |c, _| {
            one(&h6, c, "Device Parameter Change")?;
            let g = |p: &str| named_value(c, "character-eq", p).unwrap_or(f32::NAN);
            let (on, f, gain) = (g("band1_enabled"), g("band1_freq"), g("band1_gain"));
            (on == 1.0 && (f / 2000.0 - 1.0).abs() < 0.05 && approx(gain, 6.0, 0.5)).then_some(()).ok_or(format!("on {on} freq {f} gain {gain}"))
        })),
        DoC("eq: Bank menu › Passive Valve", Box::new(move |a, ctl| {
            h7.set(ctl.borrow().doc.history_labels().len());
            pick_param_menu(a, ctl, "character-eq", "bank", "Passive Valve");
        })),
        Check("eq: the bank and the band's re-snapped frequency change in one undo step", Box::new(move |c, _| {
            one(&h7b, c, "Device Parameter Change")?;
            let bank = named_value(c, "character-eq", "bank").unwrap_or(f32::NAN);
            let f = named_value(c, "character-eq", "band1_freq").unwrap_or(f32::NAN);
            let detents = saine_core::builtin::eq::bank(1.0).detents;
            (bank == 1.0 && (detents.is_empty() || detents.contains(&f))).then_some(()).ok_or(format!("bank {bank} freq {f}"))
        })),
        DoC("eq: replace it with Auto Filter (Add device › Built-in)", Box::new(move |a, ctl| {
            let old = named_builtin(&ctl.borrow(), "character-eq").map(|d| d.id);
            if let Some(id) = old {
                ctl.borrow_mut().device_remove(id);
            }
            h8.set(ctl.borrow().doc.history_labels().len());
            add_builtin(a, ctl, "Auto Filter");
        })),
        Check("eq: Auto Filter added; Response field, 4 picker pills, 7 knobs (Rate shows the free rate)", Box::new(move |c, a| {
            one(&h8b, c, "Add Auto Filter")?;
            live(c, "auto-filter")?;
            let cd = card_named(a, "Auto Filter").ok_or("no card")?;
            let knobs: Vec<String> = (0..slint::Model::row_count(&cd.knobs)).filter_map(|i| slint::Model::row_data(&cd.knobs, i)).map(|k| k.label.to_string()).collect();
            let pills = (0..slint::Model::row_count(&cd.picks)).filter_map(|i| slint::Model::row_data(&cd.picks, i)).filter(|p| p.kind != 4).count();
            let ok = cd.canvas.visible && cd.canvas.caption == "Response" && !cd.eq.visible && pills == 4 && knobs.len() == 7 && knobs[5] == "Rate";
            ok.then_some(()).ok_or(format!("canvas {} pills {pills} knobs {knobs:?}", cd.canvas.visible))
        })),
        DoC("eq: drag the filter corner left 120 px and up 30 px", Box::new(|a, _| {
            let Some(cd) = card_named(a, "Auto Filter") else { return };
            let Some(dot) = (0..slint::Model::row_count(&cd.canvas.dots)).filter_map(|i| slint::Model::row_data(&cd.canvas.dots, i)).last() else { return };
            let (x, y) = canvas_xy(&cd, dot.x, dot.y);
            drag(a, x, y, x - 120.0, y - 30.0);
        })),
        Check("eq: frequency falls and resonance rises, in ONE undo step", Box::new(move |c, _| {
            one(&h9, c, "Device Parameter Change")?;
            let (f, r) = (named_value(c, "auto-filter", "frequency").unwrap_or(f32::NAN), named_value(c, "auto-filter", "resonance").unwrap_or(f32::NAN));
            (f < 5000.0 && r > 0.1).then_some(()).ok_or(format!("freq {f} reso {r}"))
        })),
        DoC("eq: Filter menu › DJ, then Filter menu › HP", Box::new(|a, ctl| {
            pick_param_menu(a, ctl, "auto-filter", "filter_type", "DJ");
        })),
        Check("eq: DJ entry switches the mode on; the Filter pill reads DJ", Box::new(move |c, a| {
            let m = named_value(c, "auto-filter", "mode").unwrap_or(f32::NAN);
            let cd = card_named(a, "Auto Filter").ok_or("no card")?;
            let pill = slint::Model::row_data(&cd.picks, 0).map(|p| p.value.to_string()).unwrap_or_default();
            h10.set(c.doc.history_labels().len());
            (m == 1.0 && pill == "DJ").then_some(()).ok_or(format!("mode {m} pill {pill:?}"))
        })),
        DoC("eq: Filter menu › HP", Box::new(|a, ctl| {
            pick_param_menu(a, ctl, "auto-filter", "filter_type", "HP");
        })),
        Check("eq: HP sets the type and leaves DJ mode in one undo step", Box::new(move |c, _| {
            one(&h10b, c, "Device Parameter Change")?;
            let (t, m) = (named_value(c, "auto-filter", "filter_type").unwrap_or(f32::NAN), named_value(c, "auto-filter", "mode").unwrap_or(f32::NAN));
            (t == 1.0 && m == 0.0).then_some(()).ok_or(format!("type {t} mode {m}"))
        })),
        DoC("eq: remove this section's devices from the audio track", Box::new(|a, ctl| {
            key(a, Key::Escape, false, false);
            let i = track_of(&ctl.borrow(), TrackKind::Audio).unwrap_or(0);
            let ids: Vec<saine_core::DeviceId> = ctl.borrow().project().tracks[i].devices.iter().map(|d| d.id).collect();
            for id in ids {
                ctl.borrow_mut().device_remove(id);
            }
            if let Ok(mut c) = ctl.try_borrow_mut() {
                c.update_ui(a);
            }
        })),
        Check("eq: the audio track's chain is empty again", Box::new(|c, _| {
            let n = track_of(c, TrackKind::Audio).map_or(0, |i| c.project().tracks[i].devices.len());
            (n == 0).then_some(()).ok_or(format!("{n} devices left"))
        })),
    ]
}

/// Window point at the centre of step-sequencer cell (row, step): editor at window (260, 452).
fn seq_xy(a: &AppWindow, row: u16, step: u16) -> (f32, f32) {
    let s = a.global::<Studio>();
    let (pitch, w, scroll) = (s.get_seq_col_pitch(), s.get_seq_cell_w(), s.get_seq_scroll_y());
    (260.0 + crate::stepseq::COL_X0 + step as f32 * pitch + w / 2.0, 452.0 + crate::stepseq::ROW_Y0 + row as f32 * crate::stepseq::ROW_PITCH + 9.75 - scroll)
}

fn seq_clip(c: &Controller) -> Option<saine_core::MidiClip> {
    c.pr_clip().filter(|m| m.pattern.is_some()).cloned()
}

fn seq_cells(c: &Controller) -> Vec<(u8, u16)> {
    seq_clip(c).and_then(|m| m.pattern).map(|p| p.cells.iter().map(|x| (x.key, x.step)).collect()).unwrap_or_default()
}

/// Step sequencer: a Drum Rack track makes pattern clips; cells, lanes, M / S, footer menus,
/// Double, the Piano roll tab and conversion, each one undo step. Restores the track's
/// instrument and removes its clip at the end.
fn step_sequencer_steps() -> Vec<Step> {
    use Step::*;
    let hist = Rc::new(Cell::new(0usize));
    let prev: Rc<RefCell<Option<saine_core::Device>>> = Rc::new(RefCell::new(None));
    fn one(hc: &Rc<Cell<usize>>, c: &Controller, label: &str) -> Result<(), String> {
        let n = c.doc.history_labels().len();
        let was = hc.get();
        hc.set(n);
        (n == was + 1 && c.doc.undo_label() == Some(label)).then_some(()).ok_or(format!("history {was} -> {n}, last {:?} (want {label})", c.doc.undo_label()))
    }
    let h = |_: u8| hist.clone();
    let (h0, h1, h2, h3, h4, h5, h6, h7, h8, h9) = (h(0), h(1), h(2), h(3), h(4), h(5), h(6), h(7), h(8), h(9));
    let (p0, p1) = (prev.clone(), prev);
    vec![
        Do("seq: Escape (close menus)", Box::new(|a| key(a, Key::Escape, false, false))),
        DoC("seq: instrument track gets a Drum Rack; Cmd+Shift+M at bar 60 makes a pattern clip; select it", Box::new(move |a, ctl| {
            let mut c = ctl.borrow_mut();
            let Some(t) = track_of(&c, TrackKind::Instrument).map(|i| c.project().tracks[i].id) else { return };
            c.select_track(t);
            *p0.borrow_mut() = c.project().track(t).and_then(|t| t.instrument_plugin.clone());
            c.execute(saine_core::Command::SetInstrumentPlugin { track: t, device: Some(saine_core::builtin::drum_rack::DRUM_RACK.instantiate()) });
            c.seek_beats(236.0);
            h0.set(c.doc.history_labels().len());
            if let Some(id) = c.insert_midi_clip() {
                c.select_clip(Some(id));
            }
            c.update_ui(a);
        })),
        Check("seq: one-bar 'Pattern' clip of 16 1/16 steps on pads; Sequencer tab shown with 16 pad rows", Box::new(move |c, a| {
            one(&h1, c, "Insert MIDI Clip")?;
            let m = seq_clip(c).ok_or("no pattern clip open")?;
            let p = m.pattern.clone().unwrap_or_default();
            let s = a.global::<Studio>();
            let names: Vec<String> = slint::Model::iter(&s.get_seq_rows()).map(|r| r.name.to_string()).collect();
            let ok = m.name == "Pattern" && m.length == 4 * PPQ && p.steps == 16 && p.rows == saine_core::steps::StepRows::Pads && s.get_seq_visible() && s.get_seq_tabs() && names.first().map(String::as_str) == Some("Pad 1") && s.get_seq_row_count() == 16;
            ok.then_some(()).ok_or(format!("{} len {} steps {} rows {names:?} visible {}", m.name, m.length, p.steps, s.get_seq_visible()))
        })),
        Do("seq: click Pad 1 steps 1 and 5", Box::new(|a| {
            for s in [0, 4] {
                let (x, y) = seq_xy(a, 0, s);
                click(a, x, y);
            }
        })),
        Check("seq: two cells, two notes (C1 at 0 and 1 beat), two undo steps 'Edit Step Sequencer'", Box::new(move |c, _| {
            let n = c.doc.history_labels().len();
            let ok = seq_cells(c) == vec![(36, 0), (36, 4)] && n == h2.get() + 2 && c.doc.undo_label() == Some("Edit Step Sequencer");
            let notes: Vec<(u8, i64)> = seq_clip(c).map(|m| m.notes.iter().map(|n| (n.pitch, n.start)).collect()).unwrap_or_default();
            h2.set(n);
            (ok && notes == vec![(36, 0), (36, PPQ)]).then_some(()).ok_or(format!("cells {:?} notes {notes:?}", seq_cells(c)))
        })),
        Do("seq: drag across Pad 2 steps 1..4 (paint)", Box::new(|a| {
            let (x0, y) = seq_xy(a, 1, 0);
            let (x1, _) = seq_xy(a, 1, 3);
            drag(a, x0, y, x1, y);
        })),
        Check("seq: Pad 2 painted on steps 1..4 in one undo step", Box::new(move |c, _| {
            one(&h3, c, "Edit Step Sequencer")?;
            let pad2: Vec<u16> = seq_cells(c).into_iter().filter(|x| x.0 == 37).map(|x| x.1).collect();
            (pad2 == vec![0, 1, 2, 3]).then_some(()).ok_or(format!("pad 2 {pad2:?}"))
        })),
        Do("seq: Velocity tab, drag Pad 1 step 1 down 60 pt", Box::new(|a| {
            click(a, 260.0 + 220.0, 452.0 + 13.5);
            let (x, y) = seq_xy(a, 0, 0);
            drag(a, x, y, x, y + 60.0);
        })),
        Check("seq: Velocity lane shown; Pad 1 step 1 softer (one undo step), its note follows", Box::new(move |c, a| {
            one(&h4, c, "Edit Step Sequencer")?;
            let p = seq_clip(c).and_then(|m| m.pattern).ok_or("no pattern")?;
            let v = p.cell(36, 0).map(|x| x.velocity).unwrap_or(0);
            let nv = seq_clip(c).and_then(|m| m.notes.iter().find(|n| n.start == 0 && n.pitch == 36).map(|n| n.velocity)).unwrap_or(0);
            (a.global::<Studio>().get_seq_lane() == 1 && v < 60 && v >= 1 && nv == v).then_some(()).ok_or(format!("lane {} velocity {v} note {nv}", a.global::<Studio>().get_seq_lane()))
        })),
        Do("seq: Tie tab, drag Pad 1 step 5 two steps right", Box::new(|a| {
            click(a, 260.0 + 404.0, 452.0 + 13.5);
            let (x, y) = seq_xy(a, 0, 4);
            let (x2, _) = seq_xy(a, 0, 6);
            drag(a, x, y, x2, y);
        })),
        Check("seq: Pad 1 step 5 ties two steps (note three steps long, less the gate gap)", Box::new(move |c, _| {
            one(&h5, c, "Edit Step Sequencer")?;
            let p = seq_clip(c).and_then(|m| m.pattern).ok_or("no pattern")?;
            let len = seq_clip(c).and_then(|m| m.notes.iter().find(|n| n.start == PPQ && n.pitch == 36).map(|n| n.length)).unwrap_or(0);
            (p.cell(36, 4).map(|x| x.tie) == Some(2) && len > 2 * PPQ / 4 && len <= 3 * PPQ / 4).then_some(()).ok_or(format!("tie {:?} length {len}", p.cell(36, 4).map(|x| x.tie)))
        })),
        Do("seq: click Pad 2's M", Box::new(|a| {
            let s = a.global::<Studio>();
            click(a, 260.0 + 101.0, 452.0 + crate::stepseq::ROW_Y0 + crate::stepseq::ROW_PITCH + 9.75 - s.get_seq_scroll_y());
        })),
        Check("seq: Pad 2 muted (one undo step): its notes stay, playback leaves them out", Box::new(move |c, a| {
            one(&h6, c, "Edit Step Sequencer")?;
            let m = seq_clip(c).ok_or("no pattern")?;
            let muted = slint::Model::row_data(&a.global::<Studio>().get_seq_rows(), 1).is_some_and(|r| r.muted && !r.audible);
            let ok = muted && m.notes.iter().any(|n| n.pitch == 37) && m.playing_notes().iter().all(|n| n.pitch != 37);
            ok.then_some(()).ok_or(format!("muted {muted}"))
        })),
        DoC("seq: footer Length 32, Rate 1/8; Double", Box::new(|a, ctl| {
            {
                let mut c = ctl.borrow_mut();
                c.clip_menu_open(6);
                c.clip_menu_pick(4);
                c.clip_menu_open(7);
                c.clip_menu_pick(3);
                c.update_ui(a);
            }
            click(a, 260.0 + 20.0, 452.0 + 13.5);
        })),
        Check("seq: 64 steps of 1/8 (three undo steps); the footer reads 64 / 1/8; Double is now disabled", Box::new(move |c, a| {
            let n = c.doc.history_labels().len();
            let was = h7.get();
            h7.set(n);
            let p = seq_clip(c).and_then(|m| m.pattern).ok_or("no pattern")?;
            let s = a.global::<Studio>();
            let ok = n == was + 3 && p.steps == 64 && p.rate == saine_core::steps::StepRate::Eighth && s.get_seq_length_text() == "64" && s.get_seq_rate_text() == "1/8" && !s.get_seq_can_double();
            ok.then_some(()).ok_or(format!("history +{} steps {} rate {:?}", n - was, p.steps, p.rate))
        })),
        Do("seq: Piano roll tab", Box::new(|a| click(a, 5.0 + 420.0, 425.0 + 13.5))),
        Check("seq: piano roll shown for the pattern clip", Box::new(|c, a| {
            let s = a.global::<Studio>();
            (!s.get_seq_visible() && s.get_seq_tabs() && s.get_pr_visible() && !c.seq_visible()).then_some(()).ok_or("sequencer still shown".into())
        })),
        DoC("seq: convert the clip to plain MIDI", Box::new(|_, ctl| {
            let mut c = ctl.borrow_mut();
            if let Some(id) = c.pr_clip().map(|m| m.id) {
                c.convert_clip(id);
            }
        })),
        Check("seq: plain MIDI clip with the pattern's notes (one undo step); no sequencer tabs", Box::new(move |c, a| {
            one(&h8, c, "Convert to MIDI Clip")?;
            let m = c.pr_clip().ok_or("clip closed")?;
            (m.pattern.is_none() && !m.notes.is_empty() && !a.global::<Studio>().get_seq_tabs()).then_some(()).ok_or("still a pattern".into())
        })),
        DoC("seq: stretch the plain clip to 20 beats (notes in the first bar only), convert to a pattern", Box::new(|_, ctl| {
            let mut c = ctl.borrow_mut();
            let Some(m) = c.pr_clip().cloned() else { return };
            c.execute(saine_core::Command::ResizeMidiClip { clip: m.id, start: m.start, length: 20 * PPQ });
            c.convert_clip(m.id);
        })),
        Check("seq: conversion refused (the clip is longer than 64 steps would repeat), notes unchanged; undo the resize", Box::new(|c, _| {
            let m = c.pr_clip().ok_or("clip closed")?;
            let ok = m.pattern.is_none() && m.length == 20 * PPQ && !m.notes.is_empty() && c.status_text().is_some_and(|s| s.starts_with("Too long"));
            ok.then_some(()).ok_or(format!("pattern {} status {:?}", m.pattern.is_some(), c.status_text()))
        })),
        DoC("seq: undo the resize", Box::new(|a, ctl| {
            let mut c = ctl.borrow_mut();
            let _ = c.doc.undo();
            c.ui_dirty = true;
            c.update_ui(a);
        })),
        DoC("seq: Cmd+Z, then remove the clip and restore the instrument", Box::new(move |a, ctl| {
            {
                let mut c = ctl.borrow_mut();
                let _ = c.doc.undo();
                c.ui_dirty = true;
                c.update_ui(a);
                h9.set(seq_clip(&c).is_some() as usize);
                if let Some(id) = c.pr_clip().map(|m| m.id) {
                    c.execute(saine_core::Command::RemoveMidiClip { clip: id });
                }
                if let Some(t) = track_of(&c, TrackKind::Instrument).map(|i| c.project().tracks[i].id) {
                    let d = p1.borrow_mut().take();
                    c.execute(saine_core::Command::SetInstrumentPlugin { track: t, device: d });
                }
                c.select_clip(None);
                c.pr.focused = false;
                c.set_bottom_tab(crate::controller::BottomTab::Devices);
                c.seek_beats(0.0);
            }
        })),
        Check("seq: undo brought the pattern back; clip removed, instrument restored", Box::new(move |c, _| {
            let rack = track_of(c, TrackKind::Instrument).and_then(|i| c.project().tracks[i].instrument_plugin.as_ref()).is_some_and(crate::stepseq::is_drum_rack);
            let left = track_of(c, TrackKind::Instrument).map_or(0, |i| c.project().tracks[i].midi_clips.iter().filter(|m| m.start >= 236 * PPQ).count());
            (hist.get() == 1 && !rack && left == 0).then_some(()).ok_or(format!("undo restored {} rack {rack} clips left {left}", hist.get()))
        })),
    ]
}

/// Track height handle and timeline / bottom-panel splitter (appended last; restores both).
fn splitter_steps() -> Vec<Step> {
    use Step::*;
    fn edge_y(c: &Controller) -> f32 {
        TRACK0_Y + c.view.pitch(0) - 2.5 - c.view.scroll_y as f32
    }
    fn splitter_y(c: &Controller) -> f32 {
        820.0 - 5.0 - c.bottom_h() - 1.0
    }
    let base_h = Rc::new(Cell::new(0f32));
    let (b1, b2) = (base_h.clone(), base_h.clone());
    let undo_n = Rc::new(Cell::new(0usize));
    let (u1, u2) = (undo_n.clone(), undo_n.clone());
    vec![
        Check("record the undo depth", Box::new(move |c, _| {
            u1.set(c.doc.undo_depth());
            Ok(())
        })),
        DoC("drag track 1's bottom edge down half a row", Box::new(|a, ctl| {
            let y = edge_y(&ctl.borrow());
            drag(a, 300.0, y, 300.0, y + PITCH / 2.0);
        })),
        Check("track 1 is 1.5x tall in the model and the view, not an undo step", Box::new(move |c, _| {
            let h = c.project().tracks[0].height;
            let ok = approx(h, 1.5, 0.02) && approx(c.view.pitch(0), PITCH * h, 0.5) && c.doc.undo_depth() == u2.get();
            ok.then_some(()).ok_or(format!("height {h} pitch {} undo depth {} vs {}", c.view.pitch(0), c.doc.undo_depth(), u2.get()))
        })),
        DoC("reset track 1's height", Box::new(|_, ctl| {
            let mut c = ctl.borrow_mut();
            let t = c.project().tracks[0].id;
            let _ = c.doc.set_track_height(t, 1.0);
            c.ui_dirty = true;
        })),
        Check("track 1 back to its default height", Box::new(|c, _| {
            let h = c.project().tracks[0].height;
            (h == 1.0 && approx(c.view.pitch(0), PITCH, 0.01)).then_some(()).ok_or(format!("height {h}"))
        })),
        DoC("drag the bottom-panel splitter up 100 pt", Box::new(move |a, ctl| {
            let (y, h) = { let c = ctl.borrow(); (splitter_y(&c), c.bottom_h()) };
            b1.set(h);
            drag(a, 700.0, y, 700.0, y - 100.0);
        })),
        Check("bottom panel 100 pt taller and saved with the project", Box::new(move |c, a| {
            let h = c.bottom_h();
            let saved = c.project().bottom_panel_heights.values().copied().next();
            let ok = approx(h, b2.get() + 100.0, 0.5) && saved == Some(h) && approx(a.global::<Studio>().get_bottom_h(), h, 0.01);
            ok.then_some(()).ok_or(format!("h {h} base {} saved {saved:?}", b2.get()))
        })),
        DoC("drag the splitter to the bottom edge", Box::new(|a, ctl| {
            let y = splitter_y(&ctl.borrow());
            drag(a, 700.0, y, 700.0, 815.0);
        })),
        Check("bottom panel collapses to its tab row", Box::new(|c, _| {
            approx(c.bottom_h(), crate::splitters::BOTTOM_COLLAPSED_H, 0.01).then_some(()).ok_or(format!("h {}", c.bottom_h()))
        })),
        DoC("restore the default panel height", Box::new(|_, ctl| {
            let mut c = ctl.borrow_mut();
            let kinds: Vec<String> = c.project().bottom_panel_heights.keys().cloned().collect();
            for k in kinds {
                c.doc.set_bottom_panel_height(&k, None);
            }
            c.ui_dirty = true;
        })),
    ]
}

/// Docked assistant width splitter: drag, narrowing to the header's room, double-click reset
/// (appended last; restores the panel state and the saved width).
fn chat_dock_steps() -> Vec<Step> {
    use crate::splitters::{CHAT_GAP, CHAT_MAX_W, CHAT_MIN_W};
    use Step::*;
    fn split_x(c: &Controller) -> f32 {
        1440.0 - 5.0 - c.chat_w() - CHAT_GAP / 2.0
    }
    let was_open = Rc::new(Cell::new(false));
    let (w1, w2) = (was_open.clone(), was_open.clone());
    let undo_n = Rc::new(Cell::new(0usize));
    let (u1, u2) = (undo_n.clone(), undo_n.clone());
    vec![
        DoC("open the assistant at its default width", Box::new(move |_, ctl| {
            let mut c = ctl.borrow_mut();
            w1.set(c.assistant_open);
            u1.set(c.doc.undo_depth());
            c.assistant_open = true;
            c.settings.prefs.assistant_width = 0.0;
            c.ui_dirty = true;
        })),
        Check("default width 297 px, zoom sliders shown", Box::new(|c, a| {
            let ok = approx(c.chat_w(), CHAT_MIN_W, 0.01) && approx(a.global::<Studio>().get_assistant_pref_w(), CHAT_MIN_W, 0.01) && !a.get_header_zoom_hidden();
            ok.then_some(()).ok_or(format!("w {} hidden {}", c.chat_w(), a.get_header_zoom_hidden()))
        })),
        DoC("drag the splitter 60 pt left", Box::new(|a, ctl| {
            let x = split_x(&ctl.borrow());
            drag(a, x, 300.0, x - 60.0, 300.0);
        })),
        Check("chat 60 pt wider, saved as a preference, timeline narrower, not an undo step", Box::new(move |c, a| {
            let w = c.chat_w();
            let saved = c.settings.prefs.assistant_width;
            let lanes = a.get_lanes_width();
            let ok = approx(w, CHAT_MIN_W + 60.0, 0.6) && approx(saved, w, 0.01) && approx(lanes, 1440.0 - 262.65 - 5.0 - 5.0 - w - 191.0, 1.0) && c.doc.undo_depth() == u2.get();
            ok.then_some(()).ok_or(format!("w {w} saved {saved} lanes {lanes}"))
        })),
        DoC("drag the splitter far left: stops at the room that keeps the sliders", Box::new(|a, ctl| {
            let x = split_x(&ctl.borrow());
            drag(a, x, 300.0, 200.0, 300.0);
        })),
        Check("chat narrowed to the header's room (below 600 px), sliders still shown", Box::new(|c, a| {
            let room = crate::splitters::chat_room(1440.0, true);
            let w = c.chat_w();
            let ok = room < CHAT_MAX_W && approx(w, room, 0.01) && !a.get_header_zoom_hidden() && approx(a.get_lanes_width(), 1440.0 - 262.65 - 5.0 - 5.0 - room - 191.0, 1.0);
            ok.then_some(()).ok_or(format!("w {w} room {room} hidden {}", a.get_header_zoom_hidden()))
        })),
        DoC("drag the splitter right: stops at the minimum", Box::new(|a, ctl| {
            let x = split_x(&ctl.borrow());
            drag(a, x, 300.0, 1400.0, 300.0);
        })),
        Check("chat at 297 px", Box::new(|c, _| approx(c.chat_w(), CHAT_MIN_W, 0.01).then_some(()).ok_or(format!("w {}", c.chat_w())))),
        DoC("widen, then double-click the splitter", Box::new(|a, ctl| {
            let x = split_x(&ctl.borrow());
            drag(a, x, 300.0, x - 40.0, 300.0);
            let x = split_x(&ctl.borrow());
            double_click(a, x, 300.0);
        })),
        Check("double-click restored the default and cleared the preference", Box::new(|c, _| {
            let ok = approx(c.chat_w(), CHAT_MIN_W, 0.01) && c.settings.prefs.assistant_width == 0.0;
            ok.then_some(()).ok_or(format!("w {} pref {}", c.chat_w(), c.settings.prefs.assistant_width))
        })),
        DoC("restore the panel state", Box::new(move |_, ctl| {
            let mut c = ctl.borrow_mut();
            c.assistant_open = w2.get();
            c.ui_dirty = true;
        })),
    ]
}

/// Window centre of Browser › Files row `i` (rows as pushed to the UI).
fn files_row_xy(a: &AppWindow, i: usize) -> Option<(f32, f32)> {
    let s = a.global::<Studio>();
    let r = s.get_files().row_data(i)?;
    Some((120.0, 78.2 + s.get_files_list_y() + 0.5 + r.y - s.get_files_scroll() + r.h / 2.0))
}

fn files_labels(a: &AppWindow) -> Vec<String> {
    let m = a.global::<Studio>().get_files();
    (0..m.row_count()).filter_map(|i| m.row_data(i)).map(|r| r.label.to_string()).collect()
}

fn files_row_of(a: &AppWindow, label: &str) -> Option<usize> {
    let m = a.global::<Studio>().get_files();
    (0..m.row_count()).find(|&i| m.row_data(i).is_some_and(|r| r.kind == 0 && r.label == label))
}

/// Browser sections, Back / Forward, and Files › Folders and Projects on generated files.
fn files_steps() -> Vec<Step> {
    use crate::files::{SECTION_DEVICES, SECTION_FILES, TAB_FOLDERS, TAB_PROJECTS};
    use Step::*;
    let root = std::env::temp_dir().join(format!("saine-selftest-files-{}", std::process::id()));
    let kit = root.join("kit");
    let (r1, k1, k2) = (root.clone(), kit.clone(), kit.clone());
    let tracks = Rc::new(Cell::new(0usize));
    let (t1, t2, t3, t4) = (tracks.clone(), tracks.clone(), tracks.clone(), tracks.clone());
    vec![
        DoC("files: generated projects folder and sample folder; Devices section", Box::new(move |_, ctl| {
            let _ = std::fs::remove_dir_all(&r1);
            let _ = std::fs::create_dir_all(k1.join("loops"));
            let _ = std::fs::create_dir_all(r1.join("Projects"));
            let _ = crate::wavgen::write_wav(&k1.join("kick.wav"), 48_000, 1, &crate::wavgen::sine_bursts(48_000));
            let mut c = ctl.borrow_mut();
            c.files.projects_dir = Some(r1.join("Projects"));
            c.browser_section(SECTION_DEVICES);
            t1.set(c.project().tracks.len());
        })),
        Do("files: Escape (close menus)", Box::new(|a| key(a, Key::Escape, false, false))),
        Do("files: click the Files section tab", Box::new(|a| click(a, 150.0, 92.0))),
        Check("files: Files › Projects is empty, Back is enabled", Box::new(|c, a| {
            let s = a.global::<Studio>();
            let ok = c.files.section == SECTION_FILES && c.files.tab == TAB_PROJECTS && s.get_browser_can_back() && !s.get_browser_can_forward() && s.get_files_empty_title() == "No projects yet" && s.get_files_meta() == "";
            ok.then_some(()).ok_or(format!("section {} tab {} title {:?}", c.files.section, c.files.tab, s.get_files_empty_title()))
        })),
        Do("files: click the Folders sub-tab", Box::new(|a| click(a, 76.0, 144.7))),
        Check("files: Folders is empty with its hint", Box::new(|c, a| {
            let s = a.global::<Studio>();
            (c.files.tab == TAB_FOLDERS && s.get_files_empty_title() == "No folders yet").then_some(()).ok_or(format!("tab {} title {:?}", c.files.tab, s.get_files_empty_title()))
        })),
        DoC("files: add the sample folder", Box::new(move |_, ctl| ctl.borrow_mut().files_add_folder(&k2))),
        Check("files: one folder row and the count row", Box::new(|_, a| {
            let s = a.global::<Studio>();
            (files_labels(a) == ["kit"] && s.get_files_meta() == "1 folder" && s.get_files_sort() == "Recent").then_some(()).ok_or(format!("{:?} {:?}", files_labels(a), s.get_files_meta()))
        })),
        DoC("files: click the sort button", Box::new(|a, _| {
            let y = 78.2 + a.global::<Studio>().get_files_list_y() - 10.2;
            click(a, 247.25 - 12.0, y);
        })),
        Check("files: the sort menu opens on Recent", Box::new(|c, a| {
            let s = a.global::<Studio>();
            (c.files.sort_open && s.get_files_sort_open() && s.get_files_sort_index() == 0).then_some(()).ok_or(format!("open {} index {}", s.get_files_sort_open(), s.get_files_sort_index()))
        })),
        DoC("files: pick Name in the sort menu", Box::new(|a, _| {
            let y = 78.2 + a.global::<Studio>().get_files_list_y() + 3.4 + 3.4 + 18.7 * 1.5;
            click(a, 247.25 - 15.0, y);
        })),
        Check("files: sorted by Name, menu closed", Box::new(|c, a| {
            let s = a.global::<Studio>();
            (c.files.sort_name && !s.get_files_sort_open() && s.get_files_sort() == "Name").then_some(()).ok_or(format!("sort {:?} open {}", s.get_files_sort(), s.get_files_sort_open()))
        })),
        DoC("files: open the menu and click outside it", Box::new(|a, ctl| {
            ctl.borrow_mut().files_sort_open_menu(true);
            ctl.borrow_mut().update_ui(a);
            click(a, 600.0, 400.0);
        })),
        Check("files: a click outside closes the menu and keeps Name", Box::new(|c, _| {
            (!c.files.sort_open && c.files.sort_name).then_some(()).ok_or(format!("open {} name {}", c.files.sort_open, c.files.sort_name))
        })),
        DoC("files: back to Recent", Box::new(|_, ctl| ctl.borrow_mut().files_sort_pick(0))),
        DoC("files: click the folder row", Box::new(|a, _| {
            let Some((x, y)) = files_row_xy(a, 0) else { return };
            click(a, x, y);
        })),
        Check("files: the folder opens to its subfolder and WAV", Box::new(|_, a| {
            let m = a.global::<Studio>().get_files();
            let wav = (0..m.row_count()).filter_map(|i| m.row_data(i)).find(|r| r.label == "kick.wav");
            let ok = files_labels(a) == ["kit", "loops", "kick.wav"] && wav.is_some_and(|r| r.depth == 1 && r.detail == "wav") && a.global::<Studio>().get_files_guides().row_count() == 1;
            ok.then_some(()).ok_or(format!("{:?}", files_labels(a)))
        })),
        DoC("files: click kick.wav to preview it", Box::new(|a, _| {
            let Some(i) = files_row_of(a, "kick.wav") else { return };
            let Some((x, y)) = files_row_xy(a, i) else { return };
            click(a, x, y);
        })),
        Check("files: kick.wav loads into the preview bar and plays", Box::new(|c, a| {
            let s = a.global::<Studio>();
            let au = &c.files.audition;
            let m = s.get_files();
            let row = (0..m.row_count()).filter_map(|i| m.row_data(i)).find(|r| r.label == "kick.wav");
            let ok = au.path.as_ref().is_some_and(|p| p.ends_with("kick.wav")) && au.asset.is_some() && au.playing && s.get_preview_active() && s.get_preview_playing() && row.as_ref().is_some_and(|r| r.detail == "Playing");
            ok.then_some(()).ok_or(format!("{:?} asset {:?} playing {} detail {:?}", au.path, au.asset, au.playing, row.map(|r| r.detail)))
        })),
        DoC("files: pause the preview and solo it", Box::new(|_, ctl| {
            let mut c = ctl.borrow_mut();
            c.audition_toggle();
            c.audition_solo();
        })),
        Check("files: paused and soloed; the row shows its type again", Box::new(|c, a| {
            let s = a.global::<Studio>();
            let m = s.get_files();
            let row = (0..m.row_count()).filter_map(|i| m.row_data(i)).find(|r| r.label == "kick.wav");
            let ok = !c.files.audition.playing && !s.get_preview_playing() && s.get_preview_solo() && row.as_ref().is_some_and(|r| r.detail == "wav");
            ok.then_some(()).ok_or(format!("playing {} solo {} detail {:?}", c.files.audition.playing, s.get_preview_solo(), row.map(|r| r.detail)))
        })),
        DoC("files: solo off", Box::new(|_, ctl| ctl.borrow_mut().audition_solo())),
        Do("files: Back (chevron)", Box::new(|a| click(a, 229.5, 91.2))),
        Check("files: Back returns to Projects with Forward enabled", Box::new(|c, a| {
            (c.files.section == SECTION_FILES && c.files.tab == TAB_PROJECTS && a.global::<Studio>().get_browser_can_forward()).then_some(()).ok_or(format!("section {} tab {}", c.files.section, c.files.tab))
        })),
        Do("files: Back again", Box::new(|a| click(a, 229.5, 91.2))),
        Check("files: Back reaches Devices", Box::new(|c, a| {
            (c.files.section == SECTION_DEVICES && a.global::<Studio>().get_browser_section() == SECTION_DEVICES as i32).then_some(()).ok_or(format!("section {}", c.files.section))
        })),
        DoC("files: Forward, then the Devices › Effects sub-tab", Box::new(|_, ctl| {
            let mut c = ctl.borrow_mut();
            c.browser_forward();
            c.browser_tab(1);
        })),
        Check("files: a new location drops the Forward entries", Box::new(|c, a| {
            let s = a.global::<Studio>();
            (c.files.section == SECTION_DEVICES && c.devices.tab == 1 && !s.get_browser_can_forward() && s.get_browser_can_back()).then_some(()).ok_or(format!("section {} tab {}", c.files.section, c.devices.tab))
        })),
        DoC("files: Folders again, search \"KICK\"", Box::new(|_, ctl| {
            let mut c = ctl.borrow_mut();
            c.browser_tab(0);
            c.files_tab(TAB_FOLDERS);
            c.browser_search("KICK");
        })),
        Check("files: search keeps the folder and its match only", Box::new(|c, a| {
            let shown = a.global::<Studio>().get_browser_search();
            let ok = files_labels(a) == ["kit", "kick.wav"] && shown == "KICK" && c.files.search == "KICK";
            ok.then_some(()).ok_or(format!("{:?} field {shown:?} devices {:?}", files_labels(a), c.devices.search))
        })),
        DoC("files: clear the search; double-click kick.wav", Box::new(|a, ctl| {
            ctl.borrow_mut().browser_search("");
            ctl.borrow_mut().update_ui(a);
            let Some(i) = files_row_of(a, "kick.wav") else { return };
            let Some((x, y)) = files_row_xy(a, i) else { return };
            double_click(a, x, y);
        })),
        Check("files: the WAV lands on a new audio track", Box::new(move |c, _| {
            let p = c.project();
            let last = p.tracks.last();
            let ok = p.tracks.len() == t2.get() + 1 && last.is_some_and(|t| t.kind == TrackKind::Audio && t.clips.first().is_some_and(|k| k.name == "kick"));
            ok.then_some(()).ok_or(format!("{} tracks (was {})", p.tracks.len(), t2.get()))
        })),
        DoC("files: undo; save the project into the projects folder; open Projects", Box::new(move |_, ctl| {
            let mut c = ctl.borrow_mut();
            c.undo();
            let dir = c.files.projects_dir.clone().unwrap_or_default().join("Song.saineproj");
            c.save_to(&dir);
            c.files_tab(TAB_PROJECTS);
        })),
        Check("files: the saved project is listed", Box::new(|_, a| {
            (files_labels(a) == ["Song"] && a.global::<Studio>().get_files_meta() == "1 project").then_some(()).ok_or(format!("{:?}", files_labels(a)))
        })),
        DoC("files: open the project with its chevron", Box::new(|a, _| {
            let Some((_, y)) = files_row_xy(a, 0) else { return };
            click(a, 5.0 + 10.2 + 4.0, y);
        })),
        Check("files: the project lists its tracks and MIDI clips", Box::new(|_, a| {
            let l = files_labels(a);
            (l.first().map(String::as_str) == Some("Song") && l.contains(&"Tracks".to_string()) && files_row_of(a, "Instrument").is_some()).then_some(()).ok_or(format!("{l:?}"))
        })),
        DoC("files: double-click the project's Instrument track", Box::new(move |a, ctl| {
            t3.set(ctl.borrow().project().tracks.len());
            let Some(i) = files_row_of(a, "Instrument") else { return };
            let Some((x, y)) = files_row_xy(a, i) else { return };
            double_click(a, x, y);
        })),
        Check("files: the track is added as a new track (one undo step)", Box::new(move |c, _| {
            let p = c.project();
            let ok = p.tracks.len() == t4.get() + 1 && p.tracks.last().is_some_and(|t| t.kind == TrackKind::Instrument) && c.doc.undo_label() == Some("Add Track");
            ok.then_some(()).ok_or(format!("{} tracks, undo {:?}", p.tracks.len(), c.doc.undo_label()))
        })),
        DoC("files: undo; back to Devices; forget the folder", Box::new(move |_, ctl| {
            let mut c = ctl.borrow_mut();
            c.undo();
            let _ = c.engine.preview_stop();
            c.files.audition.playing = false;
            c.settings.prefs.browser_folders.clear();
            c.browser_section(SECTION_DEVICES);
            c.project_dir = None;
            let _ = std::fs::remove_dir_all(&root);
        })),
    ]
}

/// Section lane under the loop strip (appended last; clears its sections at the end).
fn section_steps() -> Vec<Step> {
    use Step::*;
    use saine_core::SectionId;
    // Lane-local (x, y) → window: the lanes start at LANE_X, their ruler at TRACK0_Y - RULER_H.
    const LANE_Y0: f32 = TRACK0_Y - crate::timeline::RULER_H;
    const ROW_Y: f32 = LANE_Y0 + 31.8;
    fn bar_x(c: &Controller, bar: f64) -> f32 {
        let bpb = c.project().ticks_per_bar() as f64 / PPQ as f64;
        LANE_X + c.view.x_of(bar * bpb)
    }
    fn names(c: &Controller) -> Vec<String> {
        c.project().sections.iter().map(|s| s.name.clone()).collect()
    }
    fn bars(c: &Controller) -> Vec<i64> {
        let bar = c.project().ticks_per_bar();
        c.project().sections.iter().map(|s| s.length / bar).collect()
    }
    fn right_click(a: &AppWindow, x: f32, y: f32) {
        let w = a.window();
        let p = LogicalPosition::new(x, y);
        w.dispatch_event(WindowEvent::PointerMoved { position: p });
        w.dispatch_event(WindowEvent::PointerPressed { position: p, button: PointerEventButton::Right });
        w.dispatch_event(WindowEvent::PointerReleased { position: p, button: PointerEventButton::Right });
    }
    fn menu_row(a: &AppWindow, label: &str) -> Option<(f32, f32)> {
        let s = a.global::<Studio>();
        let items = s.get_menu_items();
        (0..slint::Model::row_count(&items))
            .filter_map(|i| slint::Model::row_data(&items, i))
            .find(|m| m.label == label)
            .map(|m| (s.get_menu_x() + 40.0, s.get_menu_y() + m.y + crate::clipedit::MENU_ROW / 2.0))
    }
    let first = Rc::new(Cell::new(0u64));
    let (f1, f2) = (first.clone(), first.clone());
    vec![
        DoC("start the section lane at bar 1, click its +", Box::new(|a, ctl| {
            {
                let mut c = ctl.borrow_mut();
                c.view.start_beats = 0.0;
                c.ui_dirty = true;
            }
            let x = LANE_X + ctl.borrow().section_add_x();
            click(a, x, ROW_Y);
        })),
        Check("+ added \"Section 1\": 8 bars from bar 1, first colour, selected, one undo step", Box::new(move |c, _| {
            let p = c.project();
            let s = p.sections.first().ok_or("no section")?;
            f1.set(s.id.0);
            let ok = p.sections.len() == 1
                && s.name == "Section 1"
                && s.length == 8 * p.ticks_per_bar()
                && s.color == crate::sections::PALETTE[6]
                && c.selected_section == Some(s.id)
                && c.doc.undo_label() == Some("Add Section");
            ok.then_some(()).ok_or(format!("{:?} {:?}", names(c), c.doc.undo_label()))
        })),
        DoC("click the + after it", Box::new(|a, ctl| {
            let x = LANE_X + ctl.borrow().section_add_x();
            click(a, x, ROW_Y);
        })),
        Check("\"Section 2\" follows at bar 9 in the next colour", Box::new(|c, _| {
            let p = c.project();
            let ok = names(c) == ["Section 1", "Section 2"] && p.section_start(1) == 8 * p.ticks_per_bar() && p.sections[1].color == crate::sections::PALETTE[7];
            ok.then_some(()).ok_or(format!("{:?} {:?}", names(c), bars(c)))
        })),
        DoC("drag Section 1's right edge from bar 9 to bar 5", Box::new(|a, ctl| {
            let (x0, x1) = { let c = ctl.borrow(); (bar_x(&c, 8.0) - 2.0, bar_x(&c, 4.0)) };
            drag(a, x0, ROW_Y, x1, ROW_Y);
        })),
        Check("Section 1 is 4 bars and Section 2 moved with its end", Box::new(|c, _| {
            let p = c.project();
            (bars(c) == [4, 8] && p.section_start(1) == 4 * p.ticks_per_bar() && c.doc.undo_label() == Some("Resize Section")).then_some(()).ok_or(format!("{:?}", bars(c)))
        })),
        DoC("drag Section 2 to bar 1", Box::new(|a, ctl| {
            let (x0, x1) = { let c = ctl.borrow(); (bar_x(&c, 7.0), bar_x(&c, 0.2)) };
            drag(a, x0, ROW_Y, x1, ROW_Y);
        })),
        Check("Section 2 now comes first", Box::new(|c, _| {
            (names(c) == ["Section 2", "Section 1"] && c.doc.undo_label() == Some("Move Section")).then_some(()).ok_or(format!("{:?}", names(c)))
        })),
        DoC("double-click Section 1", Box::new(|a, ctl| {
            let x = bar_x(&ctl.borrow(), 10.0);
            double_click(a, x, ROW_Y);
        })),
        Check("rename popup open for the section", Box::new(move |c, a| {
            let s = a.global::<Studio>();
            (s.get_rename_open() && s.get_rename_title() == "Rename section" && c.rename_section == Some(SectionId(f2.get()))).then_some(()).ok_or(format!("open {} title {}", s.get_rename_open(), s.get_rename_title()))
        })),
        Do("type Chorus, Enter", Box::new(|a| {
            key(a, "a", true, false);
            type_text(a, "Chorus");
            key(a, Key::Return, false, false);
        })),
        Check("renamed to Chorus (one undo step)", Box::new(|c, _| {
            (names(c) == ["Section 2", "Chorus"] && c.doc.undo_label() == Some("Rename Section")).then_some(()).ok_or(format!("{:?} {:?}", names(c), c.doc.undo_label()))
        })),
        DoC("right-click Chorus", Box::new(|a, ctl| {
            let x = bar_x(&ctl.borrow(), 10.0);
            right_click(a, x, ROW_Y);
        })),
        Check("section menu: Duplicate, Rename, Delete section, Clear all sections", Box::new(|_, a| {
            let s = a.global::<Studio>();
            let ok = s.get_menu_open() && ["Duplicate", "Rename…", "Delete section", "Clear all sections"].iter().all(|l| menu_row(a, l).is_some());
            ok.then_some(()).ok_or(format!("open {}", s.get_menu_open()))
        })),
        Do("choose Delete section", Box::new(|a| {
            if let Some((x, y)) = menu_row(a, "Delete section") {
                click(a, x, y);
            }
        })),
        Check("Chorus deleted, menu closed", Box::new(|c, a| {
            (names(c) == ["Section 2"] && !a.global::<Studio>().get_menu_open()).then_some(()).ok_or(format!("{:?}", names(c)))
        })),
        Do("Cmd+Z", Box::new(|a| key(a, "z", true, false))),
        DoC("click Section 2, press Delete", Box::new(|a, ctl| {
            let x = bar_x(&ctl.borrow(), 2.0);
            click(a, x, ROW_Y);
            key(a, Key::Delete, false, false);
        })),
        Check("undo restored Chorus; Delete removed the selected section", Box::new(|c, _| {
            (names(c) == ["Chorus"]).then_some(()).ok_or(format!("{:?}", names(c)))
        })),
        DoC("right-click Chorus, Clear all sections", Box::new(|a, ctl| {
            let x = bar_x(&ctl.borrow(), 1.0);
            right_click(a, x, ROW_Y);
            if let Some((x, y)) = menu_row(a, "Clear all sections") {
                click(a, x, y);
            }
        })),
        Check("no sections left; the + is back at bar 1", Box::new(|c, _| {
            let x = c.section_add_x();
            (c.project().sections.is_empty() && approx(x, c.view.x_of(0.0) + 8.5, 0.01)).then_some(()).ok_or(format!("{:?} add x {x}", names(c)))
        })),
    ]
}


fn clip_menu_steps() -> Vec<Step> {
    use Step::*;
    use saine_core::{Asset, AssetId, Clip, ClipId, Command, MidiClip, Track, TrackId};
    const LANE_Y0: f32 = TRACK0_Y - crate::timeline::RULER_H;
    fn right_click(a: &AppWindow, x: f32, y: f32) {
        let w = a.window();
        let p = LogicalPosition::new(x, y);
        w.dispatch_event(WindowEvent::PointerMoved { position: p });
        w.dispatch_event(WindowEvent::PointerPressed { position: p, button: PointerEventButton::Right });
        w.dispatch_event(WindowEvent::PointerReleased { position: p, button: PointerEventButton::Right });
    }
    fn menu_entry(a: &AppWindow, label: &str) -> Option<(f32, f32, bool)> {
        let s = a.global::<Studio>();
        let items = s.get_menu_items();
        (0..slint::Model::row_count(&items))
            .filter_map(|i| slint::Model::row_data(&items, i))
            .find(|m| m.label == label)
            .map(|m| (s.get_menu_x() + 40.0, s.get_menu_y() + m.y + crate::clipedit::MENU_ROW / 2.0, m.enabled))
    }
    fn click_entry(a: &AppWindow, label: &str) {
        if let Some((x, y, _)) = menu_entry(a, label) {
            click(a, x, y);
        }
    }
    fn clip_xy(c: &Controller, id: ClipId) -> (f32, f32) {
        let i = c.clip_ids.iter().position(|&k| k == id).unwrap_or(0);
        let (x, y, w, h) = c.clip_vis[i].rect(&c.view);
        (LANE_X + x + w / 2.0, LANE_Y0 + y + h * 0.7)
    }
    fn audio_clips(c: &Controller) -> Vec<(u64, i64, u64, bool)> {
        let t = c.project().track(c.selected_track.unwrap_or(TrackId(0))).map(|t| t.clips.clone()).unwrap_or_default();
        t.iter().map(|k| (k.id.0, k.start, k.source_length, k.disabled)).collect()
    }
    let ids = Rc::new(Cell::new((0u64, 0u64, 0u64)));
    let (i1, i2, i3) = (ids.clone(), ids.clone(), ids.clone());
    let (i4, i5) = (ids.clone(), ids.clone());
    vec![
        DoC("clip menu: add its own audio and instrument tracks, an audio clip (beats 4-12) and a MIDI clip (beats 4-8)", Box::new(move |_, ctl| {
            let mut c = ctl.borrow_mut();
            let p = c.project();
            let (a_id, i_id, asset) = (p.next_id, p.next_id + 1, p.next_id + 2);
            c.execute(Command::Batch {
                label: "clip menu test".into(),
                commands: vec![
                    Command::AddTrack { index: Some(0), track: Track::new(TrackId(a_id), TrackKind::Audio, "Clip menu", saine_core::ColorIndex::FIRST) },
                    Command::AddTrack { index: Some(1), track: Track::new(TrackId(i_id), TrackKind::Instrument, "Clip menu MIDI", saine_core::ColorIndex::FIRST) },
                    Command::AddAsset { asset: Asset { id: AssetId(asset), relative_path: None, absolute_path: "/nonexistent/clip-menu-test.wav".into(), sample_rate: 48_000.0, channels: 1, frames: 48_000 * 4 } },
                    Command::AddClip { track: TrackId(a_id), clip: Clip { id: ClipId(asset + 1), asset: AssetId(asset), name: "menu".into(), start: 4 * PPQ, source_offset: 0, source_length: 48_000 * 4, gain: 1.0, warp: Default::default(), disabled: false } },
                    Command::AddMidiClip { track: TrackId(i_id), clip: MidiClip { id: ClipId(asset + 2), start: 4 * PPQ, ..MidiClip::empty(4 * PPQ, 4 * PPQ) } },
                ],
            });
            i1.set((a_id, asset + 1, asset + 2));
            c.select_track(TrackId(a_id));
            c.select_clip(None);
            c.seek_beats(8.0);
        })),
        DoC("right-click the audio clip", Box::new(move |a, ctl| {
            let (x, y) = clip_xy(&ctl.borrow(), ClipId(i2.get().1));
            right_click(a, x, y);
        })),
        Check("menu: Split, Disable and Show in Finder are enabled; Consolidate is not", Box::new(|_, a| {
            let on = |l: &str| menu_entry(a, l).is_some_and(|e| e.2);
            let ok = a.global::<Studio>().get_menu_open() && on("Split") && on("Disable") && on("Show in Finder") && menu_entry(a, "Consolidate").is_some_and(|e| !e.2);
            ok.then_some(()).ok_or("clip menu entries".into())
        })),
        Do("choose Split", Box::new(|a| click_entry(a, "Split"))),
        Check("audio clip cut at beat 8 into two 2 s halves; one undo step; right half selected", Box::new(move |c, _| {
            let v = audio_clips(c);
            let ok = v.len() == 2 && v[0].1 == 4 * PPQ && v[0].2 == 96_000 && v[1].1 == 8 * PPQ && v[1].2 == 96_000 && c.doc.undo_label() == Some("Split Clip") && c.selected_clip == Some(ClipId(v[1].0));
            ok.then_some(()).ok_or(format!("{v:?} {:?}", c.doc.undo_label()))
        })),
        Do("Cmd+Z", Box::new(|a| key(a, "z", true, false))),
        Check("undo joined the clip again", Box::new(|c, _| {
            let v = audio_clips(c);
            (v.len() == 1 && v[0].2 == 48_000 * 4).then_some(()).ok_or(format!("{v:?}"))
        })),
        DoC("select the clip, Cmd+E", Box::new(move |a, ctl| {
            ctl.borrow_mut().select_clip(Some(ClipId(i3.get().1)));
            key(a, "e", true, false);
        })),
        Check("Cmd+E split it at the playhead", Box::new(|c, _| {
            let v = audio_clips(c);
            (v.len() == 2 && v[1].1 == 8 * PPQ).then_some(()).ok_or(format!("{v:?}"))
        })),
        Do("press 0", Box::new(|a| key(a, "0", false, false))),
        Check("the right half is disabled and drawn grey", Box::new(|c, _| {
            let v = audio_clips(c);
            let i = c.clip_ids.iter().position(|k| Some(k.0) == v.get(1).map(|x| x.0)).ok_or("no vis")?;
            (v[1].3 && !v[0].3 && c.clip_vis[i].muted && c.doc.undo_label() == Some("Disable Clip")).then_some(()).ok_or(format!("{v:?}"))
        })),
        DoC("right-click the disabled half", Box::new(|a, ctl| {
            let id = ctl.borrow().selected_clip.unwrap_or(ClipId(0));
            let (x, y) = clip_xy(&ctl.borrow(), id);
            right_click(a, x, y);
        })),
        Check("menu offers Enable instead of Disable", Box::new(|_, a| {
            (menu_entry(a, "Enable").is_some() && menu_entry(a, "Disable").is_none()).then_some(()).ok_or("menu labels".into())
        })),
        Do("choose Enable", Box::new(|a| click_entry(a, "Enable"))),
        Check("both halves enabled again", Box::new(|c, _| {
            let v = audio_clips(c);
            (v.len() == 2 && !v[0].3 && !v[1].3).then_some(()).ok_or(format!("{v:?}"))
        })),
        DoC("right-click the MIDI clip and choose Split (playhead beat 8 → beat 6 is outside, so move it first)", Box::new(move |a, ctl| {
            let mid = {
                let mut c = ctl.borrow_mut();
                let t = c.project().tracks[1].id;
                c.select_track(t);
                c.seek_beats(6.0);
                ClipId(i4.get().2)
            };
            let (x, y) = clip_xy(&ctl.borrow(), mid);
            right_click(a, x, y);
            click_entry(a, "Split");
        })),
        Check("MIDI clip split at beat 6 into 2 + 2 beats", Box::new(|c, _| {
            let m = &c.project().tracks[1].midi_clips;
            (m.len() == 2 && m[0].length == 2 * PPQ && m[1].start == 6 * PPQ && m[1].length == 2 * PPQ).then_some(()).ok_or(format!("{} clips", m.len()))
        })),
        DoC("clip menu: remove the test tracks and clips", Box::new(move |_, ctl| {
            let mut c = ctl.borrow_mut();
            let (a_id, _, mid) = i5.get();
            let inst = c.project().tracks[1].id;
            c.execute(Command::Batch {
                label: "clip menu cleanup".into(),
                commands: vec![Command::RemoveTrack { track: TrackId(a_id) }, Command::RemoveTrack { track: inst }, Command::RemoveAsset { asset: AssetId(mid - 2) }],
            });
            c.select_clip(None);
            c.seek_beats(0.0);
        })),
        Check("clip menu: project back to its earlier tracks and assets", Box::new(|c, _| {
            let p = c.project();
            (!p.tracks.iter().any(|t| t.name.starts_with("Clip menu")) && !p.assets.iter().any(|a| a.absolute_path.contains("clip-menu-test"))).then_some(()).ok_or("leftovers".into())
        })),
    ]
}

/// Clip sidebar pattern list (Saine's local library): Create, load a row, Original,
/// Type and Genre, each one undo step. Removes its clip at the end.
fn pattern_library_steps() -> Vec<Step> {
    use Step::*;
    let hist = Rc::new(Cell::new(0usize));
    fn one(hc: &Rc<Cell<usize>>, c: &Controller, label: &str) -> Result<(), String> {
        let n = c.doc.history_labels().len();
        let was = hc.get();
        hc.set(n);
        (n == was + 1 && c.doc.undo_label() == Some(label)).then_some(()).ok_or(format!("history {was} -> {n}, last {:?} (want {label})", c.doc.undo_label()))
    }
    fn comp(c: &Controller) -> Option<saine_core::library::Composer> {
        c.pr_clip().and_then(|m| m.composer.clone())
    }
    let h = |_: u8| hist.clone();
    let (h0, h1, h2, h3, h4, h5) = (h(0), h(1), h(2), h(3), h(4), h(5));
    vec![
        Do("patterns: Escape (close menus)", Box::new(|a| key(a, Key::Escape, false, false))),
        DoC("patterns: Cmd+Shift+M on the instrument track at bar 61; select the clip", Box::new(move |a, ctl| {
            let mut c = ctl.borrow_mut();
            let Some(t) = track_of(&c, TrackKind::Instrument).map(|i| c.project().tracks[i].id) else { return };
            c.select_track(t);
            c.seek_beats(240.0);
            if let Some(id) = c.insert_midi_clip() {
                c.select_clip(Some(id));
            }
            h0.set(c.doc.history_labels().len());
            c.update_ui(a);
        })),
        Check("patterns: empty list, 'Create chords' pill", Box::new(|_, a| {
            let s = a.global::<Studio>();
            (slint::Model::row_count(&s.get_composer_rows()) == 0 && s.get_composer_create_text() == "Create chords").then_some(()).ok_or(format!("{:?}", s.get_composer_create_text()))
        })),
        Do("patterns: click Create chords", Box::new(|a| click(a, 5.0 + 60.0, 425.0 + 352.5 + 13.5))),
        Check("patterns: six chord patterns, the first plays in the clip (one undo step 'Create Patterns')", Box::new(move |c, a| {
            one(&h1, c, "Create Patterns")?;
            let k = comp(c).ok_or("no composer")?;
            let s = a.global::<Studio>();
            let notes = c.pr_clip().map_or(0, |m| m.notes.len());
            let ok = k.playing == Some(0) && slint::Model::row_count(&s.get_composer_rows()) == 6 && notes > 0 && s.get_composer_create_text() == "Create new chords" && s.get_composer_original();
            ok.then_some(()).ok_or(format!("playing {:?} notes {notes}", k.playing))
        })),
        Do("patterns: click the third row", Box::new(|a| click(a, 5.0 + 60.0, 425.0 + 104.0 + 25.0 + 2.0 * 35.0 + 16.0))),
        Check("patterns: row 3 plays (one undo step 'Load Pattern')", Box::new(move |c, _| {
            one(&h2, c, "Load Pattern")?;
            (comp(c).and_then(|k| k.playing) == Some(2)).then_some(()).ok_or("row 3 not playing".into())
        })),
        Do("patterns: click Original on the clip chip", Box::new(|a| click(a, 5.0 + 10.5 + 234.0 - 8.0 - 20.0, 425.0 + 38.0 + 15.25))),
        Check("patterns: the clip's own (empty) notes are back (one undo step 'Load Original')", Box::new(move |c, a| {
            one(&h3, c, "Load Original")?;
            let ok = comp(c).is_some_and(|k| k.playing.is_none()) && c.pr_clip().is_some_and(|m| m.notes.is_empty()) && !a.global::<Studio>().get_composer_original();
            ok.then_some(()).ok_or("original not restored".into())
        })),
        DoC("patterns: Type menu › Bass", Box::new(|a, ctl| {
            let mut c = ctl.borrow_mut();
            c.clip_menu_open(4);
            c.clip_menu_pick(1);
            c.update_ui(a);
        })),
        Check("patterns: Bass set made and playing (one undo step 'Pattern Type Change')", Box::new(move |c, a| {
            one(&h4, c, "Pattern Type Change")?;
            let k = comp(c).ok_or("no composer")?;
            let ok = k.part == saine_core::library::PartType::Bass && k.playing == Some(0) && a.global::<Studio>().get_composer_part_text() == "Bass";
            ok.then_some(()).ok_or(format!("{:?} playing {:?}", k.part, k.playing))
        })),
        DoC("patterns: Genre menu › Drill", Box::new(|a, ctl| {
            let mut c = ctl.borrow_mut();
            c.clip_menu_open(5);
            c.clip_menu_pick(3);
            c.update_ui(a);
        })),
        Check("patterns: Genre reads Drill (one undo step 'Pattern Genre Change')", Box::new(move |c, a| {
            one(&h5, c, "Pattern Genre Change")?;
            (a.global::<Studio>().get_composer_genre_text() == "Drill").then_some(()).ok_or("genre not set".into())
        })),
        DoC("patterns: remove the clip", Box::new(|_, ctl| {
            let mut c = ctl.borrow_mut();
            if let Some(id) = c.pr_clip().map(|m| m.id) {
                c.execute(saine_core::Command::RemoveMidiClip { clip: id });
            }
            c.select_clip(None);
            c.set_bottom_tab(crate::controller::BottomTab::Devices);
            c.seek_beats(0.0);
        })),
        Check("patterns: clip removed", Box::new(|c, _| {
            let left = track_of(c, TrackKind::Instrument).map_or(0, |i| c.project().tracks[i].midi_clips.iter().filter(|m| m.start >= 240 * PPQ).count());
            (left == 0).then_some(()).ok_or(format!("{left} clips left"))
        })),
    ]
}

/// Run the scripted steps, one per timer tick. Prints a summary and quits.
pub fn start(app: &AppWindow, ctl: Ctl) {
    let mut all = steps();
    all.extend(synth_steps());
    all.extend(midi_steps());
    all.extend(recording_steps());
    all.extend(platform_device_steps());
    all.extend(builtin_device_steps());
    all.extend(dynamics_device_steps());
    all.extend(modulation_steps());
    all.extend(builtin_each_steps());
    all.extend(modulation_device_steps());
    all.extend(custom_view_steps());
    all.extend(limiter_family_steps());
    all.extend(restoration_device_steps());
    all.extend(analyzer_device_steps());
    all.extend(sampler_steps());
    all.extend(time_fx_steps());
    all.extend(automation_steps());
    all.extend(field_device_steps());
    all.extend(step_sequencer_steps());
    all.extend(pattern_library_steps());
    all.extend(splitter_steps());
    all.extend(chat_dock_steps());
    all.extend(files_steps());
    all.extend(section_steps());
    all.extend(clip_menu_steps());
    let steps = Rc::new(all);
    let idx = Rc::new(std::cell::Cell::new(0usize));
    let fails = Rc::new(std::cell::Cell::new(0usize));
    let skips = Rc::new(std::cell::Cell::new(0usize));
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
            let n = steps.iter().filter(|s| matches!(s, Step::Check(..))).count() - skips.get();
            let skipped = if skips.get() > 0 { format!(" ({} skipped)", skips.get()) } else { String::new() };
            let fixture = ctl.try_borrow().map(|c| c.devices.catalog.fixture.is_some()).unwrap_or(false);
            let note = if fixture { " [plugin catalogue from a TEST fixture: the plugin checks did not exercise the engine scanner]" } else { "" };
            println!("self-test: {} / {n} checks passed{skipped}{note}", n - fails.get());
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
                    // A guarded check that did not run (e.g. needs a plugin window): not counted.
                    Err(e) if e.starts_with("SKIP: ") => {
                        skips.set(skips.get() + 1);
                        println!("SKIP {name}: {}", &e[6..]);
                    }
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
