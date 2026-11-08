//! Saine Studio desktop application (milestone 1).
//!
//! Slint shell (FemtoVG on wgpu 30) + our own wgpu timeline renderer
//! (`gpu/`), wired to the real `saine_core::Document` and the C++ engine.
//!
//! ```text
//! saine-studio [--open <dir>] [--import <wav>...] [--save-to <dir>] [--no-audio]
//!              [--demo] [--state reference-default|reference-mixer|reference-assistant]
//!              [--assistant-fixture <turns.json>  (TEST provider: canned replies, no network)]
//!              [--screenshot <out.png>] [--exit-after-frames N] [--frame-stats] [--self-test]
//! ```

mod assistant_panel;
mod assistant_provider;
mod controller;
mod gpu;
mod highlight;
mod mixer;
mod peaks;
mod placeholder;
mod selftest;
mod timeline;
mod wavgen;

use controller::{Controller, ImportTarget, gain_from_slider};
use saine_core::keymap::{Chord, Context, Keymap};
use saine_core::{Project, TrackId, db_to_gain};
use saine_engine::{Config, Engine};
use slint::ComponentHandle;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

slint::include_modules!();

#[derive(Default, Debug)]
struct Args {
    open: Option<PathBuf>,
    import: Vec<PathBuf>,
    save_to: Option<PathBuf>,
    audio: bool,
    demo: bool,
    reference_default: bool,
    /// Extra panels for the reference states.
    open_mixer: bool,
    open_assistant: bool,
    assistant_fixture: Option<PathBuf>,
    screenshot: Option<PathBuf>,
    exit_after_frames: Option<u64>,
    frame_stats: bool,
    self_test: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut a = Args { audio: true, ..Default::default() };
    let v: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    let value = |i: &mut usize| -> Result<String, String> {
        *i += 1;
        v.get(*i).cloned().ok_or_else(|| format!("missing value for {}", v[*i - 1]))
    };
    while i < v.len() {
        match v[i].as_str() {
            "--open" => a.open = Some(value(&mut i)?.into()),
            "--import" => {
                a.import.push(value(&mut i)?.into());
                while v.get(i + 1).is_some_and(|s| !s.starts_with("--")) {
                    i += 1;
                    a.import.push(v[i].clone().into());
                }
            }
            "--save-to" => a.save_to = Some(value(&mut i)?.into()),
            "--no-audio" => a.audio = false,
            "--demo" => a.demo = true,
            "--state" => match value(&mut i)?.as_str() {
                "reference-default" => a.reference_default = true,
                "reference-mixer" => {
                    a.reference_default = true;
                    a.open_mixer = true;
                }
                "reference-assistant" => {
                    a.reference_default = true;
                    a.open_assistant = true;
                }
                s => return Err(format!("unknown state {s} (supported: reference-default, reference-mixer, reference-assistant)")),
            },
            "--assistant-fixture" => a.assistant_fixture = Some(value(&mut i)?.into()),
            "--screenshot" => a.screenshot = Some(value(&mut i)?.into()),
            "--exit-after-frames" => a.exit_after_frames = Some(value(&mut i)?.parse().map_err(|e| format!("--exit-after-frames: {e}"))?),
            "--frame-stats" => a.frame_stats = true,
            "--self-test" | "--self-test-audio" => {
                a.self_test = true;
                a.demo = true;
                // --self-test-audio also plays ~1 s through the device at -40 dB master.
                a.audio = v[i] == "--self-test-audio";
            }
            "-h" | "--help" => {
                println!("{}", include_str!("main.rs").lines().skip(6).take(5).map(|l| l.trim_start_matches("//! ")).collect::<Vec<_>>().join("\n"));
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument {other}")),
        }
        i += 1;
    }
    // Screenshot / benchmark runs are offline: no audio output while verifying.
    if a.screenshot.is_some() || a.exit_after_frames.is_some() {
        a.audio = false;
    }
    if a.screenshot.is_some() && a.exit_after_frames.is_none() {
        a.exit_after_frames = Some(30);
    }
    Ok(a)
}

#[derive(Default)]
struct Series(Vec<f64>);
impl Series {
    fn summary(&self) -> String {
        if self.0.is_empty() {
            return "null".into();
        }
        let mut s = self.0.clone();
        s.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let pct = |p: f64| s[((s.len() - 1) as f64 * p).round() as usize];
        let avg = s.iter().sum::<f64>() / s.len() as f64;
        format!("{{\"avg\":{avg:.3},\"p50\":{:.3},\"p95\":{:.3},\"p99\":{:.3},\"max\":{:.3}}}", pct(0.5), pct(0.95), pct(0.99), s[s.len() - 1])
    }
}

/// Render-loop state (wgpu renderer + timing), main thread only.
struct Frame {
    renderer: Option<gpu::Renderer>,
    dl: gpu::DrawList,
    adapter: String,
    interval: Series,
    ours: Series,
    slint: Series,
    last_before: Option<Instant>,
    slint_start: Option<Instant>,
    ready_frames: u64,
    setup_done: bool,
    shot_taken: bool,
    saved: bool,
    tex: (u32, u32),
}

/// Slint key text -> keymap key name.
fn key_name(text: &str) -> Option<String> {
    let k = match text {
        " " => "Space",
        "\n" | "\r" => "Enter",
        "\u{7f}" => "Delete",
        "\u{8}" => "Backspace",
        "\u{1b}" => "Escape",
        "\t" => "Tab",
        "\u{F700}" => "Up",
        "\u{F701}" => "Down",
        "\u{F702}" => "Left",
        "\u{F703}" => "Right",
        "," => "Comma",
        "." => "Period",
        t if t.chars().count() == 1 && t.chars().all(|c| c.is_ascii_graphic()) => return Some(t.to_ascii_uppercase()),
        _ => return None,
    };
    Some(k.into())
}

/// The core default preset plus app-level view bindings that `saine_core::keymap`
/// does not define (the reference's shortcuts for these are unverified; ours).
pub fn app_keymap() -> Keymap {
    use saine_core::keymap::{Binding, Condition};
    let mut k = Keymap::default_preset();
    for (action, chord) in [("view.toggle_mixer", "primary+shift+x"), ("view.toggle_assistant", "primary+shift+a")] {
        k.bindings.push(Binding { action: action.into(), chords: vec![Chord::parse(chord).expect("valid chord")], when: Condition::Always });
    }
    k
}

fn app_text_focused(aw: &slint::Weak<AppWindow>) -> bool {
    aw.upgrade().map(|a| a.global::<Studio>().get_chat_focused()).unwrap_or(false)
}

fn pick_import() -> Vec<PathBuf> {
    rfd::FileDialog::new().set_title("Import audio").add_filter("WAV audio", &["wav", "WAV"]).pick_files().unwrap_or_default()
}

fn pick_project_dir(save: bool) -> Option<PathBuf> {
    if save {
        // A project is a directory; ask for its name and create it.
        rfd::FileDialog::new().set_title("Save project").set_file_name(format!("Untitled.{}", saine_core::persist::EXTENSION)).save_file()
    } else {
        rfd::FileDialog::new().set_title("Open project folder (.saineproj)").pick_folder()
    }
}

type Ctl = Rc<RefCell<Controller>>;

/// Run a keymap action. File dialogs run with the controller NOT borrowed,
/// since the modal loop keeps rendering (and the renderer borrows it).
fn run_action(ctl: &Ctl, action: &str) -> bool {
    match action {
        "transport.play_pause" => ctl.borrow_mut().play_toggle(),
        "transport.stop_return" => ctl.borrow_mut().stop_return(),
        "transport.loop_toggle" => ctl.borrow_mut().toggle_loop(),
        "edit.undo" => ctl.borrow_mut().undo(),
        "edit.redo" => ctl.borrow_mut().redo(),
        "clip.delete" | "track.delete" => ctl.borrow_mut().delete_selection(),
        "track.add_audio" => {
            ctl.borrow_mut().add_audio_track();
        }
        "track.mute" => {
            let mut c = ctl.borrow_mut();
            if let Some(t) = c.selected_track {
                c.toggle_mute(t)
            }
        }
        "track.solo" => {
            let mut c = ctl.borrow_mut();
            if let Some(t) = c.selected_track {
                c.toggle_solo(t)
            }
        }
        "view.zoom_in" => ctl.borrow_mut().view.zoom_at(1.25, 0.0),
        "view.toggle_mixer" => ctl.borrow_mut().toggle_mixer(),
        "view.toggle_assistant" => ctl.borrow_mut().toggle_assistant(),
        "file.import_audio" => {
            let files = pick_import();
            if !files.is_empty() {
                ctl.borrow_mut().import_files(files, ImportTarget::Selected, None);
            }
        }
        "file.save" | "file.save_as" => {
            let existing = ctl.borrow().project_dir.clone().filter(|_| action == "file.save");
            if let Some(dir) = existing.or_else(|| pick_project_dir(true)) {
                ctl.borrow_mut().save_to(&dir);
            }
        }
        "file.open" => {
            if let Some(dir) = pick_project_dir(false) {
                ctl.borrow_mut().open(&dir);
            }
        }
        _ => return false,
    }
    true
}

fn main() {
    if let Err(e) = run() {
        eprintln!("saine-studio: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args = parse_args()?;

    // ---- engine (48 kHz) ----
    let engine = Arc::new(Engine::new(Config { sample_rate: 48_000.0, ..Config::default() })?);
    let mut device_error = None;
    if args.audio {
        if let Err(e) = engine.start_device() {
            device_error = Some(e.to_string());
        }
    }
    let device_ok = engine.device_running();
    eprintln!("engine: abi {} @ {} Hz, device {}", Engine::abi_version(), engine.sample_rate(), if device_ok { "running" } else { "offline" });

    // ---- Slint backend: our wgpu device + transparent macOS titlebar ----
    let (cfg, adapter) = gpu::make_wgpu_config()?;
    let selector = slint::BackendSelector::new().backend_name("winit".into()).renderer_name("femtovg-wgpu".into()).require_wgpu_30(cfg);
    #[cfg(target_os = "macos")]
    let selector = selector.with_winit_window_attributes_hook(|attrs| {
        use slint::winit_030::winit::platform::macos::WindowAttributesExtMacOS;
        attrs.with_titlebar_transparent(true).with_fullsize_content_view(true).with_title_hidden(true)
    });
    selector.select()?;

    let app = AppWindow::new()?;
    let studio = app.global::<Studio>();
    studio.set_tree(slint::ModelRc::new(slint::VecModel::from(placeholder::device_tree())));

    let ctl: Ctl = Rc::new(RefCell::new(Controller::new(engine.clone(), Project::default())));
    ctl.borrow().bind_models(&app);
    {
        // Provider: TEST fixture (flag, or the built-in one for --self-test), else the API key env var.
        let provider = match (&args.assistant_fixture, args.self_test) {
            (Some(path), _) => {
                let text = std::fs::read_to_string(path).map_err(|e| format!("--assistant-fixture {}: {e}", path.display()))?;
                assistant_provider::Provider::fixture_from_json(&text, &path.display().to_string())?
            }
            (None, true) => assistant_provider::Provider::fixture_from_json(include_str!("../fixtures/assistant-selftest.json"), "built-in self-test fixture")?,
            (None, false) => assistant_provider::Provider::from_env(),
        };
        eprintln!("assistant: provider {provider:?}{}", if provider.is_fixture() { " (TEST provider, no network)" } else { "" });
        let mut c = ctl.borrow_mut();
        c.provider = Arc::new(provider);
        c.mixer_open = args.open_mixer;
        c.assistant_open = args.open_assistant;
        c.ui_dirty = true;
    }
    if args.reference_default || args.screenshot.is_some() {
        ctl.borrow_mut().show_status = !args.reference_default;
    }
    if let Some(e) = &device_error {
        ctl.borrow_mut().set_status(format!("Audio device unavailable ({e}); running offline"), 8);
    }

    // ---- callbacks: every edit becomes a Command or a transport call ----
    macro_rules! on {
        ($setter:ident, |$c:ident $(, $a:ident : $t:ty)*| $body:expr) => {{
            let ctl = ctl.clone();
            let aw = app.as_weak();
            studio.$setter(move |$($a: $t),*| {
                {
                    let mut guard = ctl.borrow_mut();
                    let $c = &mut *guard;
                    $body;
                }
                if let Some(a) = aw.upgrade() { a.window().request_redraw(); }
            });
        }};
    }
    on!(on_play_toggle, |c| c.play_toggle());
    on!(on_stop_return, |c| c.stop_return());
    on!(on_toggle_loop, |c| c.toggle_loop());
    on!(on_toggle_metronome, |c| c.toggle_metronome());
    on!(on_record, |c| c.set_status("Recording is not implemented yet", 3));
    on!(on_add_track, |c| c.add_audio_track());
    on!(on_select_track, |c, id: i32| c.select_track(TrackId(id as u64)));
    on!(on_track_mute, |c, id: i32| c.toggle_mute(TrackId(id as u64)));
    on!(on_track_solo, |c, id: i32| c.toggle_solo(TrackId(id as u64)));
    on!(on_track_volume, |c, id: i32, p: f32| c.set_track_gain(TrackId(id as u64), gain_from_slider(p)));
    on!(on_track_pan, |c, id: i32, p: f32| c.set_track_pan(TrackId(id as u64), p));
    on!(on_master_volume, |c, p: f32| c.set_master_gain(gain_from_slider(p)));
    on!(on_sel_fader_moved, |c, f: f32| {
        if let Some(t) = c.selected_track {
            c.set_track_gain(t, db_to_gain(controller::db_from_fader(f)))
        }
    });
    on!(on_gesture_end, |c| c.ui_dirty = true);
    on!(on_toggle_mixer, |c| c.toggle_mixer());
    on!(on_toggle_assistant, |c| c.toggle_assistant());
    on!(on_mixer_fader, |c, id: i32, f: f32| c.set_mixer_fader(TrackId(id as u64), f));
    on!(on_master_fader_moved, |c, f: f32| c.set_master_fader(f));
    on!(on_reset_peaks, |c| c.reset_peaks());
    on!(on_chat_send, |c, text: slint::SharedString| {
        c.assistant_send(text.as_str());
    });
    on!(on_chat_new, |c| c.assistant_new_chat());
    on!(on_chat_set_mode, |c, teach: bool| c.assistant_set_mode(teach));
    on!(on_chat_dismiss_notice, |c| c.assistant_dismiss_notice());
    on!(on_chat_toggle_entry, |c, i: i32| c.assistant_toggle_entry(i.max(0) as usize));
    on!(on_chat_remove_context, |c| c.assistant_remove_context());
    on!(on_chat_placeholder, |c, what: slint::SharedString| c.set_status(format!("{what}: not implemented yet"), 3));
    on!(on_lane_pointer, |c, kind: i32, x: f32, y: f32, _shift: bool, _cmd: bool| c.pointer(kind, x, y));
    on!(on_lane_scroll, |c, dx: f32, dy: f32, x: f32, zoom: bool| c.scroll(dx, dy, x, zoom));
    // TODO(feedback): wire "Report a bug" / "Share your feedback" to a real channel. No-ops for now.
    on!(on_report_bug, |c| c.set_status("Report a bug: not connected yet (TODO)", 3));
    on!(on_feedback, |c| c.set_status("Feedback: not connected yet (TODO)", 3));
    {
        let ctl = ctl.clone();
        studio.on_lane_hover(move |x, y| {
            if let Ok(c) = ctl.try_borrow() {
                let cur = c.hover(x, y);
                drop(c);
                // Studio global is reachable through any component handle; avoid holding a strong ref.
                LANE_CURSOR.with(|lc| lc.set(cur));
            }
        });
    }
    {
        let ctl = ctl.clone();
        let aw = app.as_weak();
        let keymap = app_keymap();
        studio.on_key(move |text, primary, shift, alt, ctrl| {
            let Some(key) = key_name(text.as_str()) else { return false };
            let chord = Chord { key, primary, shift, alt, ctrl };
            let cx = {
                let c = ctl.borrow();
                Context {
                    mixer_open: c.mixer_open,
                    text_input_focused: app_text_focused(&aw),
                    track_selected: c.selected_track.is_some(),
                    track_header_selected: c.selected_track.is_some() && c.selected_clip.is_none(),
                    clips_selected: c.selected_clip.is_some() as usize,
                    ..Default::default()
                }
            };
            let Some(action) = keymap.resolve(&chord, &cx).map(str::to_owned) else { return false };
            let handled = run_action(&ctl, &action);
            if let Some(a) = aw.upgrade() {
                a.window().request_redraw();
            }
            handled
        });
    }

    // ---- startup work ----
    if let Some(dir) = &args.open {
        ctl.borrow_mut().open(dir);
    }
    if args.demo {
        let dir = std::env::temp_dir().join(format!("saine-demo-{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        let sine = dir.join("sine-bursts.wav");
        let noise = dir.join("noise-stereo.wav");
        wavgen::write_wav(&sine, 44_100, 1, &wavgen::sine_bursts(44_100))?;
        wavgen::write_wav(&noise, 44_100, 2, &wavgen::noise(44_100))?;
        eprintln!("demo: wrote {} and {}", sine.display(), noise.display());
        let mut c = ctl.borrow_mut();
        c.import_files(vec![sine], ImportTarget::Selected, Some(0));
        // Second clip at bar 3 beat 3; leave the playhead there too.
        c.import_files(vec![noise], ImportTarget::NewTrack, Some(10 * saine_core::PPQ));
        c.seek_beats(10.0);
    }
    if !args.import.is_empty() {
        ctl.borrow_mut().import_files(args.import.clone(), ImportTarget::Selected, None);
    }

    // ---- job polling (imports finish on worker threads) ----
    let poll_timer = slint::Timer::default();
    {
        let ctl = ctl.clone();
        let aw = app.as_weak();
        poll_timer.start(slint::TimerMode::Repeated, Duration::from_millis(30), move || {
            let Ok(mut c) = ctl.try_borrow_mut() else { return };
            if c.pending_jobs > 0 {
                c.poll_jobs();
                if let Some(a) = aw.upgrade() {
                    a.window().request_redraw();
                }
            }
            drop(c);
            if let Some(a) = aw.upgrade() {
                let cur = LANE_CURSOR.with(|lc| lc.get());
                let s = a.global::<Studio>();
                if s.get_lane_cursor() != cur {
                    s.set_lane_cursor(cur);
                }
            }
        });
    }

    // ---- rendering notifier: the wgpu integration point (ADR 0001) ----
    let frame = Rc::new(RefCell::new(Frame {
        renderer: None,
        dl: gpu::DrawList::default(),
        adapter,
        interval: Series::default(),
        ours: Series::default(),
        slint: Series::default(),
        last_before: None,
        slint_start: None,
        ready_frames: 0,
        setup_done: false,
        shot_taken: false,
        saved: false,
        tex: (0, 0),
    }));
    let measure = args.demo || args.frame_stats || args.exit_after_frames.is_some();
    let screenshot = args.screenshot.clone();
    let exit_after = args.exit_after_frames;
    let save_to = args.save_to.clone();
    {
        let aw = app.as_weak();
        let ctl = ctl.clone();
        let fr = frame.clone();
        app.window().set_rendering_notifier(move |state, api| {
            let Some(app) = aw.upgrade() else { return };
            match state {
                slint::RenderingState::RenderingSetup => {
                    let r = gpu::with_wgpu(api, gpu::Renderer::new);
                    if r.is_none() {
                        eprintln!("RenderingSetup: unexpected graphics API (expected wgpu 30)");
                    }
                    fr.borrow_mut().renderer = r;
                }
                slint::RenderingState::BeforeRendering => {
                    let t0 = Instant::now();
                    let Ok(mut c) = ctl.try_borrow_mut() else { return };
                    let mut f = fr.borrow_mut();
                    if let Some(last) = f.last_before
                        && f.ready_frames > 2
                    {
                        f.interval.0.push((t0 - last).as_secs_f64() * 1e3);
                    }
                    f.last_before = Some(t0);
                    c.poll_jobs();
                    c.lanes_size = (app.get_lanes_width(), app.get_lanes_height());
                    c.update_ui(&app);
                    c.rebuild_vis();
                    let scale = app.window().scale_factor();
                    let size = ((c.lanes_size.0 * scale).round() as u32, (c.lanes_size.1 * scale).round() as u32);
                    let f = &mut *f;
                    if let Some(r) = f.renderer.as_mut() {
                        if let Some(tex) = r.ensure_target(size) {
                            match gpu::to_slint_image(tex) {
                                Ok(img) => app.global::<Studio>().set_timeline_image(img),
                                Err(e) => eprintln!("timeline texture import failed: {e}"),
                            }
                        }
                        f.tex = r.target_size();
                        r.sync_peaks(c.peaks.generation, &c.peaks.data);
                        timeline::build(&mut f.dl, &c.scene(scale));
                        r.render(&f.dl);
                    }
                    let t1 = Instant::now();
                    if f.ready_frames > 2 {
                        f.ours.0.push((t1 - t0).as_secs_f64() * 1e3);
                    }
                    f.slint_start = Some(t1);
                }
                slint::RenderingState::AfterRendering => {
                    let now = Instant::now();
                    let mut f = fr.borrow_mut();
                    if let Some(t) = f.slint_start.take()
                        && f.ready_frames > 2
                    {
                        f.slint.0.push((now - t).as_secs_f64() * 1e3);
                    }
                    let (animating, ready) = match ctl.try_borrow() {
                        Ok(c) => (c.pending_jobs > 0 || c.engine.transport().playing || !c.assistant.highlights.is_empty(), c.pending_jobs == 0),
                        Err(_) => (true, false),
                    };
                    if ready {
                        f.setup_done = true;
                    }
                    if f.setup_done {
                        f.ready_frames += 1;
                    }
                    if f.setup_done
                        && !f.saved
                        && let Some(dir) = &save_to
                    {
                        f.saved = true;
                        if let Ok(mut c) = ctl.try_borrow_mut() {
                            c.save_to(dir);
                        }
                    }
                    if let Some(n) = exit_after
                        && f.ready_frames >= n
                        && !f.shot_taken
                    {
                        f.shot_taken = true;
                        let aw2 = app.as_weak();
                        let shot = screenshot.clone();
                        let report = stats_json(&f, &app);
                        // take_snapshot is not re-entrant inside the notifier: defer to the event loop.
                        slint::Timer::single_shot(Duration::from_millis(0), move || {
                            if let (Some(a), Some(path)) = (aw2.upgrade(), shot) {
                                save_snapshot(&a, &path);
                            }
                            println!("{report}");
                            let _ = slint::quit_event_loop();
                        });
                    }
                    drop(f);
                    if animating || measure {
                        app.window().request_redraw();
                    }
                }
                slint::RenderingState::RenderingTeardown => fr.borrow_mut().renderer = None,
                _ => {}
            }
        })?;
    }

    if args.self_test {
        selftest::start(&app, ctl.clone());
    }
    // Fallback for --screenshot when the window never presents frames (occluded window
    // or locked screen: macOS stops redraws). `take_snapshot()` renders offscreen and
    // runs the rendering notifier, so the wgpu timeline is included. A few warm-up
    // renders let models/text layout settle before the saved one.
    if let Some(path) = args.screenshot.clone() {
        let aw = app.as_weak();
        let fr = frame.clone();
        let ctl2 = ctl.clone();
        slint::Timer::single_shot(Duration::from_secs(4), move || {
            if fr.borrow().shot_taken {
                return;
            }
            let Some(a) = aw.upgrade() else { return };
            if ctl2.try_borrow().map(|c| c.pending_jobs > 0).unwrap_or(true) {
                eprintln!("screenshot: jobs still pending at the offscreen fallback");
            }
            eprintln!("screenshot: no frames presented (occluded/locked?), rendering offscreen");
            fr.borrow_mut().shot_taken = true;
            for _ in 0..3 {
                let _ = a.window().take_snapshot();
            }
            save_snapshot(&a, &path);
            let _ = slint::quit_event_loop();
        });
    }
    app.window().set_size(slint::LogicalSize::new(1440.0, 820.0));
    app.window().set_position(slint::LogicalPosition::new(0.0, 33.0));
    app.run()?;
    if args.frame_stats && exit_after.is_none() {
        println!("{}", stats_json(&frame.borrow(), &app));
    }
    let _ = engine.stop_device();
    Ok(())
}

thread_local! {
    static LANE_CURSOR: std::cell::Cell<i32> = const { std::cell::Cell::new(0) };
}

fn stats_json(f: &Frame, app: &AppWindow) -> String {
    let w = app.window();
    let sz = w.size();
    let rc = f.renderer.as_ref().map(|r| r.last_counts).unwrap_or((0, 0, 0));
    format!(
        "{{\"adapter\":\"{}\",\"scale_factor\":{},\"window_physical\":[{},{}],\"timeline_tex\":[{},{}],\"instances\":{{\"base_rects\":{},\"waves\":{},\"over_rects\":{}}},\"frames\":{},\"frame_interval_ms\":{},\"our_cpu_ms\":{},\"slint_render_cpu_ms\":{}}}",
        f.adapter,
        w.scale_factor(),
        sz.width,
        sz.height,
        f.tex.0,
        f.tex.1,
        rc.0,
        rc.1,
        rc.2,
        f.interval.0.len(),
        f.interval.summary(),
        f.ours.summary(),
        f.slint.summary()
    )
}

fn save_snapshot(app: &AppWindow, path: &std::path::Path) {
    match app.window().take_snapshot() {
        Ok(buf) => {
            let res = (|| -> Result<(), Box<dyn std::error::Error>> {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                let f = std::fs::File::create(path)?;
                let mut enc = png::Encoder::new(std::io::BufWriter::new(f), buf.width(), buf.height());
                enc.set_color(png::ColorType::Rgba);
                enc.set_depth(png::BitDepth::Eight);
                enc.write_header()?.write_image_data(buf.as_bytes())?;
                Ok(())
            })();
            match res {
                Ok(()) => eprintln!("screenshot: wrote {} ({}x{})", path.display(), buf.width(), buf.height()),
                Err(e) => eprintln!("screenshot: write failed: {e}"),
            }
        }
        Err(e) => eprintln!("screenshot: take_snapshot failed: {e}"),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn app_keymap_adds_panel_toggles_without_conflicts() {
        let k = super::app_keymap();
        assert!(k.conflicts().is_empty(), "{:?}", k.conflicts());
        assert_eq!(k.chords_for("view.toggle_mixer").len(), 1);
        assert_eq!(k.chords_for("view.toggle_assistant").len(), 1);
    }
}
