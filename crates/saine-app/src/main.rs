//! Saine Studio desktop application (milestone 1).
//!
//! Slint shell (FemtoVG on wgpu 30) + our own wgpu timeline renderer
//! (`gpu/`), wired to the real `saine_core::Document` and the C++ engine.
//!
//! ```text
//! saine-studio [--open <dir>] [--import <wav>...] [--save-to <dir>] [--no-audio]
//!              [--demo] [--state reference-default|reference-mixer|reference-assistant|reference-midi-clip|reference-pianoroll|reference-synth|reference-input-menu|reference-settings-audio|reference-devices-effects|reference-automation|review-automation|review-automation-picker|review-recording|review-device-chain|review-settings-plugins]
//!              [--synth-page <0..8>  (with --state reference-synth: category page to show)]
//!              [--assistant-fixture <turns.json>  (TEST provider: canned replies, no network)]
//!              [--plugin-fixture <plugins.json>  (TEST seam: fixed plugin catalogue instead of scanning)]
//!              [--screenshot <out.png>] [--offscreen] [--exit-after-frames N] [--frame-stats] [--self-test]
//! ```

mod assistant_panel;
mod assistant_provider;
mod automation;
mod clipedit;
mod controller;
mod device_fields;
mod device_panel;
mod panel_graph;
mod device_views;
mod devices;
mod gpu;
mod highlight;
mod mixer;
mod peaks;
mod pianoroll;
mod placeholder;
mod prefs;
mod recording;
mod selftest;
mod settings;
mod synth;
mod timeline;
mod wavgen;

use controller::{BottomTab, Controller, ImportTarget, gain_from_slider};
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
    /// 1 = reference-midi-clip (33), 2 = reference-pianoroll (34).
    midi_state: u8,
    /// reference-synth (32): Instrument track selected, Devices tab, test-state synth preset.
    synth_state: bool,
    /// --synth-page N: open that category page of the synth device (review screenshots).
    synth_page: Option<usize>,
    /// reference-input-menu (50): Instrument + Audio tracks, Audio selected, "In" popup open.
    input_menu_state: bool,
    /// reference-settings-audio (14): Settings open on the Audio page.
    settings_state: bool,
    /// review-recording: a take in progress (injected input, offline engine) for reviewing
    /// the live recording clip. Not a reference capture (the reference recording UI was not captured).
    recording_review: bool,
    /// reference-automation (60): Instrument track 1 (MIDI clip at bar 6), Audio track 2 selected,
    /// playhead at bar 6, automation lanes shown (Track Volume at +0.0).
    automation_state: bool,
    /// review-automation / review-automation-picker: lanes with points (active, disabled, curved,
    /// an expanded extra lane, master) for review; the second also opens the picker with the
    /// Synth submenu. No reference capture.
    automation_review: u8,
    /// reference-devices-effects (05): browser Devices › Effects sub-tab.
    devices_effects_state: bool,
    /// review-device-chain: Instrument track with the Synth and one AU effect (no reference).
    device_chain_review: bool,
    /// Never show the window (renders for --screenshot fall back to offscreen snapshots).
    offscreen: bool,
    /// reference-<built-in device id>: that device alone in an audio track's chain.
    device_state: Option<&'static str>,
    /// review-settings-plugins: Settings open on the Plugins page (no reference comparison).
    plugins_page_review: bool,
    assistant_fixture: Option<PathBuf>,
    plugin_fixture: Option<PathBuf>,
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
                "reference-midi-clip" => {
                    a.reference_default = true;
                    a.midi_state = 1;
                }
                "reference-pianoroll" => {
                    a.reference_default = true;
                    a.midi_state = 2;
                }
                "reference-synth" => {
                    a.reference_default = true;
                    a.synth_state = true;
                }
                "reference-input-menu" => {
                    a.reference_default = true;
                    a.input_menu_state = true;
                }
                "reference-settings-audio" => {
                    a.reference_default = true;
                    a.settings_state = true;
                }
                "reference-automation" => {
                    a.reference_default = true;
                    a.automation_state = true;
                }
                "review-automation" => {
                    a.reference_default = true;
                    a.automation_review = 1;
                }
                "review-automation-picker" => {
                    a.reference_default = true;
                    a.automation_review = 2;
                }
                "reference-devices-effects" => {
                    a.reference_default = true;
                    a.devices_effects_state = true;
                }
                "review-device-chain" => {
                    a.reference_default = true;
                    a.device_chain_review = true;
                }
                s if s.strip_prefix("reference-").and_then(saine_core::builtin::find).is_some() => {
                    a.reference_default = true;
                    a.device_state = s.strip_prefix("reference-").and_then(saine_core::builtin::find).map(|d| d.id);
                }
                "review-settings-plugins" => {
                    a.reference_default = true;
                    a.plugins_page_review = true;
                }
                "review-recording" => {
                    a.reference_default = true;
                    a.recording_review = true;
                    a.audio = false;
                }
                s => {
                    return Err(format!(
                        "unknown state {s} (supported: reference-default, reference-mixer, reference-assistant, reference-midi-clip, reference-pianoroll, reference-synth, reference-input-menu, reference-settings-audio, reference-devices-effects, reference-automation, review-automation, review-automation-picker, review-recording, review-device-chain, reference-<built-in device id> e.g. reference-compressor, review-settings-plugins)"
                    ));
                }
            },
            "--synth-page" => a.synth_page = Some(value(&mut i)?.parse().map_err(|e| format!("--synth-page: {e}"))?),
            "--assistant-fixture" => a.assistant_fixture = Some(value(&mut i)?.into()),
            "--plugin-fixture" => a.plugin_fixture = Some(value(&mut i)?.into()),
            "--screenshot" => a.screenshot = Some(value(&mut i)?.into()),
            "--offscreen" => a.offscreen = true,
            "--exit-after-frames" => a.exit_after_frames = Some(value(&mut i)?.parse().map_err(|e| format!("--exit-after-frames: {e}"))?),
            "--frame-stats" => a.frame_stats = true,
            "--self-test" | "--self-test-audio" => {
                a.self_test = true;
                a.demo = true;
                // --self-test-audio also plays ~1 s through the device at -40 dB master.
                a.audio = v[i] == "--self-test-audio";
            }
            "-h" | "--help" => {
                println!("{}", include_str!("main.rs").lines().skip(6).take(7).map(|l| l.trim_start_matches("//! ")).collect::<Vec<_>>().join("\n"));
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
    /// Piano roll: a second offscreen texture with its own renderer instance.
    pr_renderer: Option<gpu::Renderer>,
    pr_dl: gpu::DrawList,
    pr_visible: bool,
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
    for (action, chord) in [("view.toggle_mixer", "primary+shift+x"), ("view.toggle_assistant", "primary+shift+a"), ("view.escape", "escape")] {
        k.bindings.push(Binding { action: action.into(), chords: vec![Chord::parse(chord).expect("valid chord")], when: Condition::Always });
    }
    // Arm / input monitoring on the selected track (the core preset has no bindings for them;
    // the reference's chords are unverified, these are ours). Added only if the core lacks them.
    for (action, chord) in [("track.arm", "shift+r"), ("track.monitor", "shift+i")] {
        if k.chords_for(action).is_empty() {
            k.bindings.push(Binding { action: action.into(), chords: vec![Chord::parse(chord).expect("valid chord")], when: Condition::TrackSelected });
        }
    }
    // Piano roll (our bindings; the reference documents ↑/↓ transpose, Delete and B/E only).
    for (action, chords) in [
        ("midi.delete_notes", &["delete", "backspace"][..]),
        ("midi.transpose_up", &["up"][..]),
        ("midi.transpose_down", &["down"][..]),
        ("midi.octave_up", &["shift+up"][..]),
        ("midi.octave_down", &["shift+down"][..]),
        ("midi.select_all", &["primary+a"][..]),
    ] {
        let chords = chords.iter().map(|c| Chord::parse(c).expect("valid chord")).collect();
        k.bindings.push(Binding { action: action.into(), chords, when: Condition::MidiEditorFocused });
    }
    k
}

fn app_text_focused(aw: &slint::Weak<AppWindow>) -> bool {
    aw.upgrade()
        .map(|a| {
            let s = a.global::<Studio>();
            s.get_chat_focused() || s.get_rename_open() || s.get_search_focused() || s.get_picker_focused()
        })
        .unwrap_or(false)
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
        "clip.delete" | "track.delete" => {
            let mut c = ctl.borrow_mut();
            if !c.auto_delete_selected() {
                c.delete_selection();
            }
        }
        "draw_mode.toggle" => ctl.borrow_mut().auto_toggle_draw(),
        "clip.insert_midi" => {
            ctl.borrow_mut().insert_midi_clip();
        }
        "clip.duplicate" => ctl.borrow_mut().duplicate_clip(),
        "clip.cut" => ctl.borrow_mut().cut_clip(),
        "edit.copy" => {
            ctl.borrow_mut().copy_clip();
        }
        "edit.paste" => ctl.borrow_mut().paste_clip(),
        "midi.paint_pen" => ctl.borrow_mut().pr_action("pen"),
        "midi.delete_notes" => {
            let mut c = ctl.borrow_mut();
            if !c.pr_delete_notes() {
                c.delete_selection();
            }
        }
        "midi.transpose_up" => ctl.borrow_mut().pr_transpose(1),
        "midi.transpose_down" => ctl.borrow_mut().pr_transpose(-1),
        "midi.octave_up" => ctl.borrow_mut().pr_transpose(12),
        "midi.octave_down" => ctl.borrow_mut().pr_transpose(-12),
        "midi.select_all" => ctl.borrow_mut().pr_select_all(),
        "view.escape" => {
            let mut c = ctl.borrow_mut();
            if c.panel_cancel_drag() || (c.synth.menu.is_none() && c.synth_cancel()) {
                return true;
            }
            if c.settings.menu.is_some() {
                c.settings_menu_dismiss();
            } else if c.settings.open {
                c.close_settings();
            }
            c.input_menu_close();
            c.menu_close();
            c.synth_menu_dismiss();
            c.devmenu_dismiss();
            c.auto_picker_close();
            c.rename = None;
            c.ui_dirty = true;
        }
        a if a.starts_with("clip.") || a == "assistant.ask" => ctl.borrow_mut().set_status(format!("{a}: not implemented yet"), 3),
        "track.add_audio" => {
            ctl.borrow_mut().add_audio_track();
        }
        "track.mute" => {
            let mut c = ctl.borrow_mut();
            if let Some(t) = c.selected_track {
                c.toggle_mute(t)
            }
        }
        "transport.record" => ctl.borrow_mut().record_toggle(),
        "app.settings" => ctl.borrow_mut().toggle_settings(),
        "track.arm" => {
            let mut c = ctl.borrow_mut();
            if let Some(t) = c.selected_track {
                c.toggle_arm(t)
            }
        }
        "track.monitor" => {
            let mut c = ctl.borrow_mut();
            if let Some(t) = c.selected_track {
                c.toggle_monitor(t)
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
    // App preferences (Settings › Audio): never read or written by self-tests and
    // reference/screenshot runs, so those can never open a real input device.
    let use_prefs = !args.self_test && !args.reference_default && args.screenshot.is_none() && args.exit_after_frames.is_none();
    let prefs_path = if use_prefs { prefs::default_path() } else { None };
    let mut saved_prefs = prefs::Preferences::default();
    let mut prefs_error = None;
    if let Some(path) = &prefs_path {
        match prefs::load(path) {
            Ok(Some(p)) => saved_prefs = p,
            Ok(None) => {}
            Err(e) => prefs_error = Some(format!("Preferences ignored: {e}")),
        }
    }
    if args.audio {
        if let Err(e) = settings::apply_startup_prefs(&engine, &saved_prefs) {
            prefs_error = Some(e);
        }
        if !engine.device_running()
            && let Err(e) = engine.start_device()
        {
            device_error = Some(e.to_string());
        }
    }
    let device_ok = engine.device_running();
    eprintln!("engine: abi {} @ {} Hz, device {}", Engine::abi_version(), engine.sample_rate(), if device_ok { "running" } else { "offline" });

    // ---- Slint backend: our wgpu device + transparent macOS titlebar ----
    let (cfg, adapter) = gpu::make_wgpu_config()?;
    let selector = slint::BackendSelector::new().backend_name("winit".into()).renderer_name("femtovg-wgpu".into()).require_wgpu_30(cfg);
    let offscreen = args.offscreen;
    #[cfg(target_os = "macos")]
    let selector = selector.with_winit_window_attributes_hook(move |attrs| {
        use slint::winit_030::winit::platform::macos::WindowAttributesExtMacOS;
        let attrs = attrs.with_titlebar_transparent(true).with_fullsize_content_view(true).with_title_hidden(true);
        if offscreen { attrs.with_visible(false).with_active(false) } else { attrs }
    });
    #[cfg(not(target_os = "macos"))]
    let selector = selector.with_winit_window_attributes_hook(move |attrs| if offscreen { attrs.with_visible(false).with_active(false) } else { attrs });
    selector.select()?;

    let app = AppWindow::new()?;
    let studio = app.global::<Studio>();

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
        c.settings.prefs = saved_prefs;
        c.settings.prefs_path = prefs_path;
        if args.self_test {
            // Recording in the self-test uses injected input only (render_offline_with_input)
            // into a scratch directory; it never opens or requires a real input device.
            c.rec.policy = recording::InputPolicy::InjectedOnly;
            c.rec.dir_override = Some(std::env::temp_dir().join(format!("saine-selftest-rec-{}", std::process::id())));
        }
        c.mixer_open = args.open_mixer;
        c.assistant_open = args.open_assistant;
        // Plugin catalogue: the scan cache in the app support directory (startup reads the cache;
        // Settings › Plugins can force a rescan). The self-test uses its own cache file and never
        // touches the user's. --plugin-fixture (TEST seam) replaces the scan entirely.
        c.devices.catalog.cache_path = if args.self_test {
            Some(std::env::temp_dir().join("saine-selftest-plugin-cache.json"))
        } else {
            prefs::plugin_cache_path()
        };
        if let Some(path) = &args.plugin_fixture {
            c.devices.catalog.fixture = Some(devices::load_fixture(path)?);
            eprintln!("plugins: catalogue from {} (TEST fixture, no scan)", path.display());
        }
        c.devices.catalog.screenshot_apple_only = args.reference_default;
        c.start_plugin_scan(false);
        c.ui_dirty = true;
    }
    if args.midi_state > 0 {
        // Reference 33/34: default project, Instrument track selected, 1-bar MIDI clip at bar 6.
        let mut c = ctl.borrow_mut();
        let inst = c.project().tracks.iter().find(|t| t.kind == saine_core::TrackKind::Instrument).map(|t| t.id);
        if let Some(t) = inst {
            c.select_track(t);
        }
        c.seek_beats(20.0);
        let clip = c.insert_midi_clip();
        if args.midi_state == 2 {
            c.select_clip(clip);
            c.seek_beats(22.0); // 00:11.000, as captured
        }
    }
    if args.synth_state {
        // Reference 32: default project, Instrument track selected (playhead at bar 6), Devices tab,
        // Oscillators page. The oscillator values come from a TEST-STATE preset built here (not a
        // factory preset) so the displays are comparable with the reference capture.
        let mut c = ctl.borrow_mut();
        let inst = c.project().tracks.iter().find(|t| t.kind == saine_core::TrackKind::Instrument).map(|t| t.id);
        if let Some(t) = inst {
            c.select_track(t);
            // Values read off the reference captures (osc 2 FM, LFO 1 slow, 2 routes,
            // one macro mapping, Tape off / Chorus / Delay / Reverb); not a factory sound.
            let device = c.project().track(t).and_then(|t| t.instrument_plugin.as_ref()).map(|d| d.id);
            if let Some(device) = device {
                let params: std::collections::BTreeMap<String, f32> = [
                    ("osc1_wave", 1.0), ("osc1_shape", 0.1), ("osc1_level", 0.8), ("osc2_wave", 5.0), ("osc2_shape", 0.0), ("osc2_octave", 3.0), ("osc2_fine", 7.0), ("osc2_level", 0.3),
                    ("sub_level", 0.0), ("cutoff", 2400.0), ("resonance", 0.1), ("filter_drive", 0.2), ("filter_env", 0.25), ("filter_velocity", 0.35), ("env2_attack", 0.002),
                    ("amp_attack", 0.004), ("amp_decay", 0.9 * 1.0), ("amp_sustain", 0.35), ("amp_release", 1.1), ("volume", -0.7), ("env2_decay", 0.9), ("env2_sustain", 0.1),
                    ("lfo1_rate", 0.18), ("lfo2_rate", 5.0), ("unison_voices", 2.0), ("unison_detune", 0.18), ("unison_spread", 0.8),
                    ("mod1_source", 3.0), ("mod1_target", 5.0), ("mod1_amount", 0.2), ("mod2_source", 7.0), ("mod2_target", 11.0), ("mod2_amount", 0.35), ("mod3_source", 0.0), ("mod3_target", 0.0), ("mod3_amount", 0.0),
                    ("macro1_name", 12.0), ("macro2_name", 18.0), ("macro3_name", 7.0), ("macro4_name", 6.0), ("map1_macro", 1.0), ("map1_param", 22.0), ("map1_from", 0.693), ("map1_to", 0.943),
                    ("fx1_on", 0.0), ("fx3_type", 7.0), ("fx3_on", 1.0), ("fx4_type", 8.0), ("fx4_on", 1.0),
                ]
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect();
                c.execute(saine_core::Command::SetDeviceParams { device, params, preset: Some("Test: reference-synth".into()) });
            }
        }
        c.set_bottom_tab(BottomTab::Devices);
        c.synth_page(args.synth_page.unwrap_or(0));
        c.seek_beats(20.0);
    }
    if args.input_menu_state || args.settings_state {
        // SCREENSHOT-ONLY fake device listing (recording::screenshot_fake_devices), never
        // configured on the engine: "Built-in Speakers" / "Built-in Microphone", 48 kHz, 1024.
        let mut c = ctl.borrow_mut();
        c.settings.fake_devices = true;
        let a = &mut c.settings.prefs.audio;
        (a.output, a.output_name) = (Some("fake:speakers".into()), Some("Built-in Speakers".into()));
        (a.input, a.input_name) = (Some("fake:microphone".into()), Some("Built-in Microphone".into()));
        (a.sample_rate, a.buffer_size) = (48_000.0, 1024);
    }
    if args.input_menu_state {
        // Reference 50: Instrument track 1 (1-bar MIDI clip at bar 6), Audio track 2 selected,
        // playhead 00:11.000, Devices tab, "In" popup open ("None ✓", "Input 1 (Mono)").
        let mut p = Project::default();
        p.tracks.swap(0, 1);
        let mut c = ctl.borrow_mut();
        c.replace_project(p);
        let inst = c.project().tracks[0].id;
        let audio = c.project().tracks[1].id;
        c.select_track(inst);
        c.seek_beats(20.0);
        c.insert_midi_clip();
        c.select_track(audio);
        c.set_bottom_tab(BottomTab::Devices);
        c.seek_beats(22.0);
        c.open_input_menu();
    }
    if args.automation_state {
        // Reference 60: same project layout as 50 (Instrument 1 with a MIDI clip at bar 6, Audio 2
        // selected, playhead 00:11.000, Devices tab) with the automation lanes shown.
        let mut p = Project::default();
        p.tracks.swap(0, 1);
        let mut c = ctl.borrow_mut();
        c.replace_project(p);
        let inst = c.project().tracks[0].id;
        let audio = c.project().tracks[1].id;
        c.select_track(inst);
        c.seek_beats(20.0);
        c.insert_midi_clip();
        c.select_track(audio);
        c.set_bottom_tab(BottomTab::Devices);
        c.seek_beats(22.0);
        c.auto_toggle_shown();
    }
    if args.automation_review > 0 {
        use saine_core::{AutoPoint, AutomationLane, AutomationTarget, SynthParam};
        let pt = |beat: i64, value: f32, curve: f32| AutoPoint { time: beat * saine_core::PPQ, value, curve };
        let mut c = ctl.borrow_mut();
        let (audio, inst) = (c.project().tracks[0].id, c.project().tracks[1].id);
        c.select_track(inst);
        c.seek_beats(20.0);
        c.insert_midi_clip();
        let lanes = [
            (Some(audio), AutomationLane { target: AutomationTarget::Volume, enabled: true, points: vec![pt(0, -24.0, 0.0), pt(8, 0.0, -0.6), pt(16, -12.0, 0.0), pt(24, -6.0, 0.5), pt(32, -60.0, 0.0)] }),
            (Some(audio), AutomationLane { target: AutomationTarget::Pan, enabled: false, points: vec![pt(4, -0.5, 0.0), pt(20, 0.5, 0.0)] }),
            (Some(inst), AutomationLane { target: AutomationTarget::Synth { param: SynthParam::FilterCutoff }, enabled: true, points: vec![pt(0, 200.0, 0.0), pt(16, 8000.0, 0.0), pt(20, 800.0, 0.0)] }),
            (None, AutomationLane { target: AutomationTarget::Volume, enabled: true, points: vec![pt(0, 0.0, 0.0), pt(28, 0.0, 0.0), pt(36, -60.0, 0.4)] }),
        ];
        for (key, lane) in lanes {
            c.auto_commit(key, lane);
        }
        c.auto.targets.insert(Some(inst), AutomationTarget::Synth { param: SynthParam::FilterCutoff });
        c.auto.expanded.insert(audio);
        c.auto_toggle_shown();
        c.auto.sel = Some((Some(audio), AutomationTarget::Volume, vec![1]));
        c.set_bottom_tab(BottomTab::Devices);
        c.seek_beats(22.0);
        if args.automation_review == 2 {
            let y = 78.0 + 66.5 + c.view.track_offset(1) + 48.5 + 17.0;
            c.auto_open_picker(Some(inst), 262.0 + 42.0, y);
            c.auto_picker_pick(1);
        }
    }
    if args.settings_state {
        ctl.borrow_mut().open_settings();
    }
    if args.plugins_page_review {
        let mut c = ctl.borrow_mut();
        c.open_settings();
        c.settings_page(3);
    }
    if args.devices_effects_state {
        // Reference 05: default project, browser Devices › Effects (the catalogue as scanned).
        ctl.borrow_mut().browser_tab(1);
    }
    if args.device_chain_review {
        // Review only (no reference capture): Instrument track selected, Devices tab, the built-in
        // Synth followed by one Audio Unit effect (Apple's AULowpass, named by its AU identity).
        let mut c = ctl.borrow_mut();
        if let Some(t) = c.project().tracks.iter().find(|t| t.kind == saine_core::TrackKind::Instrument).map(|t| t.id) {
            c.select_track(t);
            let plugin = saine_core::PluginRef {
                format: saine_core::PluginFormat::AudioUnit,
                id: "aufx lpas appl".into(),
                path: None,
                name: "AULowpass".into(),
                vendor: "Apple".into(),
                version: String::new(),
                kind: saine_core::PluginKind::Effect,
            };
            c.add_item_to_track(&devices::Item::Plugin(plugin), t, None);
        }
        c.set_bottom_tab(BottomTab::Devices);
        c.seek_beats(20.0);
    }
    if let Some(id) = args.device_state {
        // Compare with renders/reference/<id>.png (Captain drive): the built-in device
        // at its defaults, alone in an audio track's chain, transport stopped.
        let mut c = ctl.borrow_mut();
        if let Some(t) = c.project().tracks.iter().find(|t| t.kind == saine_core::TrackKind::Audio).map(|t| t.id) {
            c.select_track(t);
            let def = saine_core::builtin::find(id).expect("device is registered");
            c.add_item_to_track(&devices::Item::Builtin(def), t, None);
        }
        c.set_bottom_tab(BottomTab::Devices);
        c.seek_beats(8.0);
    }
    if args.recording_review {
        // Offline engine + injected input (no input device): track 1 armed on Input 1,
        // recording from bar 2 with 3 s of a decaying 220 Hz tone rendered so far.
        let mut c = ctl.borrow_mut();
        c.rec.policy = recording::InputPolicy::InjectedOnly;
        c.rec.dir_override = Some(std::env::temp_dir().join(format!("saine-review-rec-{}", std::process::id())));
        let t = c.project().tracks[0].id;
        c.execute(saine_core::Command::SetTrackArm { track: t, arm: true });
        c.set_track_input(t, Some(saine_core::InputSource { channel: 0, stereo: false }));
        c.seek_beats(4.0);
        c.start_recording();
        if c.rec.recording {
            let n = 3 * 48_000;
            let input: Vec<f32> = (0..n).map(|i| {
                let t = i as f32 / 48_000.0;
                0.8 * (0.25 + 0.75 * (-(t * 2.0).fract() * 4.0).exp()) * (std::f32::consts::TAU * 220.0 * t).sin()
            }).collect();
            let mut out = vec![0f32; 2 * n];
            if let Err(e) = c.engine.render_offline_with_input(&input, 1, &mut out) {
                c.set_status(format!("review-recording: {e}"), 10);
            }
        }
    }
    // Push view models once before the first frame, so conditional panels are built
    // for the initial state (setting them from inside the first BeforeRendering is
    // too late for that frame's item tree).
    ctl.borrow_mut().update_ui(&app);
    if args.reference_default || args.screenshot.is_some() {
        ctl.borrow_mut().show_status = !args.reference_default;
    }
    if let Some(e) = &prefs_error {
        ctl.borrow_mut().set_status(e.clone(), 8);
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
                // Push view models now (outside the rendering notifier), so conditional
                // panels (`if` in Slint) are rebuilt for this state before the next frame.
                if let Some(a) = aw.upgrade() {
                    if let Ok(mut c) = ctl.try_borrow_mut() { c.update_ui(&a); }
                    a.window().request_redraw();
                }
            });
        }};
    }
    on!(on_play_toggle, |c| c.play_toggle());
    on!(on_stop_return, |c| c.stop_return());
    on!(on_toggle_loop, |c| c.toggle_loop());
    on!(on_toggle_metronome, |c| c.toggle_metronome());
    on!(on_record, |c| c.record_toggle());
    on!(on_track_arm, |c, id: i32| c.toggle_arm(TrackId(id as u64)));
    on!(on_track_monitor, |c, id: i32| c.toggle_monitor(TrackId(id as u64)));
    on!(on_open_input_menu, |c| c.open_input_menu());
    on!(on_input_menu_pick, |c, i: i32| c.input_menu_pick(i.max(0) as usize));
    on!(on_input_menu_dismiss, |c| c.input_menu_close());
    on!(on_toggle_settings, |c| c.toggle_settings());
    on!(on_close_settings, |c| c.close_settings());
    on!(on_settings_page_select, |c, i: i32| c.settings_page(i.max(0) as usize));
    on!(on_settings_dropdown, |c, f: i32| c.settings_dropdown(f));
    on!(on_settings_menu_pick, |c, i: i32| c.settings_menu_pick(i.max(0) as usize));
    on!(on_settings_menu_dismiss, |c| c.settings_menu_dismiss());
    on!(on_settings_toggle_mute, |c| c.settings_toggle_mute());
    on!(on_settings_toggle_pdc, |c| c.settings_toggle_pdc());
    on!(on_settings_test, |c| c.settings_test());
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
    on!(on_gesture_end, |c| {
        // Track / master control gesture end: finish write passes of those controls.
        c.auto_write_end(|w| matches!(w.target, saine_core::AutomationTarget::Volume | saine_core::AutomationTarget::Pan));
        c.ui_dirty = true
    });
    on!(on_auto_toggle_shown, |c| c.auto_toggle_shown());
    on!(on_auto_toggle_draw, |c| c.auto_toggle_draw());
    on!(on_auto_toggle_arm, |c| c.auto_toggle_arm());
    on!(on_auto_power, |c, id: i32, extra: i32| c.auto_power((id >= 0).then_some(TrackId(id as u64)), (extra >= 0).then_some(extra as usize)));
    on!(on_auto_expand, |c, id: i32| c.auto_toggle_expanded(TrackId(id as u64)));
    on!(on_auto_show_extra, |c, id: i32, i: i32| c.auto_show_extra(TrackId(id as u64), i.max(0) as usize));
    on!(on_auto_open_picker, |c, id: i32, x: f32, y: f32| c.auto_open_picker((id >= 0).then_some(TrackId(id as u64)), x, y));
    on!(on_picker_edited, |c, t: slint::SharedString| c.auto_picker_query(t.as_str()));
    on!(on_picker_pick, |c, i: i32| c.auto_picker_pick(i.max(0) as usize));
    on!(on_picker_sub_pick, |c, i: i32| c.auto_picker_sub_pick(i.max(0) as usize));
    on!(on_picker_dismiss, |c| c.auto_picker_close());
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
    on!(on_lane_pointer, |c, kind: i32, x: f32, y: f32, shift: bool, _cmd: bool, alt: bool| c.pointer_mods(kind, x, y, shift, alt));
    on!(on_lane_scroll, |c, dx: f32, dy: f32, x: f32, zoom: bool| c.scroll(dx, dy, x, zoom));
    on!(on_set_bottom_tab, |c, t: i32| c.set_bottom_tab(if t == 1 { BottomTab::Clip } else { BottomTab::Devices }));
    on!(on_synth_page_select, |c, i: i32| c.synth_page(i.max(0) as usize));
    on!(on_synth_param, |c, id: i32, norm: f32| c.synth_param(id.max(0) as usize, norm));
    on!(on_synth_param_end, |c, id: i32, moved: bool| c.synth_param_end(id.max(0) as usize, moved));
    on!(on_synth_param_reset, |c, id: i32| c.synth_param_reset(id.max(0) as usize));
    on!(on_synth_set, |c, id: i32, v: f32| c.synth_set(id.max(0) as usize, v));
    on!(on_synth_toggle, |c, id: i32| c.synth_toggle(id.max(0) as usize));
    on!(on_synth_pill, |c, action: i32, arg: i32, x: f32, y: f32, w: f32| c.synth_pill(action, arg.max(0) as usize, x, y, w));
    on!(on_synth_remove, |c, kind: i32, slot: i32| c.synth_remove(kind, slot.max(0) as usize));
    on!(on_synth_knob_menu, |c, id: i32, x: f32, y: f32| c.synth_knob_menu(id.max(0) as usize, x, y));
    on!(on_synth_rail, |c, src: i32, phase: i32, x: f32, y: f32| c.synth_rail(src.max(0) as usize, phase, x, y));
    on!(on_synth_scope_drag, |c, slot: i32, phase: i32, x: f32, y: f32| c.synth_scope_drag(slot.max(0) as usize, phase, x, y));
    on!(on_synth_macro_select, |c, m: i32| c.synth_macro_select(m.max(0) as usize));
    on!(on_synth_bypass, |c| c.synth_bypass());
    on!(on_synth_menu_pick, |c, i: i32| c.synth_menu_pick(i.max(0) as usize));
    on!(on_synth_menu_dismiss, |c| c.synth_menu_dismiss());
    on!(on_synth_preset_step, |c, dir: i32| c.synth_preset_step(dir));
    on!(on_synth_placeholder, |c, what: slint::SharedString| c.set_status(format!("{what}: not implemented yet"), 3));
    on!(on_toggle_clip_devices, |c| {
        let tab = if c.bottom_tab == BottomTab::Clip { BottomTab::Devices } else { BottomTab::Clip };
        c.set_bottom_tab(tab)
    });
    on!(on_pr_pointer, |c, kind: i32, x: f32, y: f32, shift: bool, alt: bool| c.pr_pointer(kind, x, y, shift, alt));
    on!(on_pr_scroll, |c, dy: f32| c.pr_scroll(dy));
    on!(on_pr_action, |c, a: slint::SharedString| c.pr_action(a.as_str()));
    on!(on_pr_value, |c, f: i32, phase: i32, dy: f32| c.pr_value(f, phase, dy));
    on!(on_menu_activate, |c, i: i32| c.menu_activate(i.max(0) as usize));
    on!(on_sub_activate, |c, i: i32| c.submenu_activate(i.max(0) as usize));
    on!(on_menu_dismiss, |c| c.menu_close());
    on!(on_rename_commit, |c, t: slint::SharedString| {
        if let Some(id) = c.rename {
            c.rename_clip(id, t.as_str());
        }
    });
    on!(on_rename_cancel, |c| {
        c.rename = None;
        c.ui_dirty = true;
    });
    {
        let ctl = ctl.clone();
        let aw = app.as_weak();
        studio.on_lane_context(move |x, y, wx, wy| {
            let Some(a) = aw.upgrade() else { return };
            let scale = a.window().scale_factor();
            let sz = a.window().size();
            ctl.borrow_mut().lane_context(x, y, wx, wy, (sz.width as f32 / scale, sz.height as f32 / scale));
            ctl.borrow_mut().update_ui(&a);
            a.window().request_redraw();
        });
    }
    // ---- plugin devices (src/devices.rs) ----
    on!(on_browser_tab_select, |c, i: i32| c.browser_tab(i.max(0) as usize));
    on!(on_browser_search_edited, |c, t: slint::SharedString| c.browser_search(t.as_str()));
    on!(on_tree_click, |c, i: i32| c.tree_click(i.max(0) as usize));
    on!(on_tree_activate, |c, i: i32| c.tree_activate(i.max(0) as usize));
    on!(on_tree_drag, |c, i: i32, phase: i32, x: f32, y: f32| c.tree_drag(i.max(0) as usize, phase, x, y));
    on!(on_tree_scroll_by, |c, dy: f32| c.tree_scroll_by(dy));
    on!(on_device_power, |c, id: i32| c.device_power(saine_core::DeviceId(id as u64)));
    on!(on_device_edit, |c, id: i32| c.device_edit(saine_core::DeviceId(id as u64)));
    on!(on_device_remove, |c, id: i32| c.device_remove(saine_core::DeviceId(id as u64)));
    on!(on_device_use_synth, |c| c.device_use_synth());
    on!(on_device_drag, |c, id: i32, phase: i32, x: f32| c.device_drag(saine_core::DeviceId(id as u64), phase, x));
    on!(on_panel_drag, |c, id: i32, p: i32, phase: i32, dy: f32, rate: f32, fine: bool| c.panel_drag(saine_core::DeviceId(id as u64), p, phase, dy, rate, fine));
    on!(on_panel_reset, |c, id: i32, p: i32| c.panel_reset(saine_core::DeviceId(id as u64), p));
    on!(on_panel_reset_xy, |c, id: i32, px: i32, py: i32| c.panel_reset_xy(saine_core::DeviceId(id as u64), [px, py]));
    on!(on_panel_step, |c, id: i32, p: i32, delta: i32, fine: bool| c.panel_step(saine_core::DeviceId(id as u64), p, delta, fine));
    on!(on_panel_drag_xy, |c, id: i32, px: i32, py: i32, phase: i32, dx: f32, dy: f32, rx: f32, ry: f32, fine: bool| c.panel_drag_xy(saine_core::DeviceId(id as u64), [px, py], phase, [dx, dy], [rx, ry], fine));
    on!(on_panel_choice, |c, id: i32, p: i32, i: i32| c.panel_choice(saine_core::DeviceId(id as u64), p, i));
    on!(on_panel_text, |c, id: i32, p: i32, text: slint::SharedString| c.panel_text(saine_core::DeviceId(id as u64), p, &text));
    on!(on_panel_field, |c, id: i32, phase: i32, dx: f32, dy: f32, fine: bool| c.panel_field_drag(saine_core::DeviceId(id as u64), phase, dx, dy, fine));
    on!(on_panel_menu, |c, id: i32, which: i32, x: f32, y: f32| c.panel_menu(saine_core::DeviceId(id as u64), which, x, y));
    on!(on_panel_preset_step, |c, id: i32, dir: i32| c.panel_preset_step(saine_core::DeviceId(id as u64), dir));
    on!(on_panel_graph, |c, id: i32, phase: i32, x: f32, y: f32, fine: bool, alt: bool| c.panel_graph(saine_core::DeviceId(id as u64), phase, x, y, fine, alt));
    on!(on_chain_scroll_by, |c, d: f32| c.chain_scroll_by(d));
    on!(on_add_category, |c, i: i32, x: f32, y: f32| c.add_category(i.max(0) as usize, x, y));
    on!(on_add_search_edited, |c, t: slint::SharedString| c.add_search(t.as_str()));
    on!(on_add_result_pick, |c, i: i32| c.add_result_pick(i.max(0) as usize));
    on!(on_open_inst_menu, |c, x: f32, y: f32| c.open_inst_menu(x, y));
    on!(on_devmenu_pick, |c, i: i32| c.devmenu_pick(i.max(0) as usize));
    on!(on_devmenu_dismiss, |c| c.devmenu_dismiss());
    on!(on_plugins_rescan, |c, rescan: bool| c.start_plugin_scan(rescan));
    on!(on_placeholder, |c, what: slint::SharedString| c.set_status(format!("{what}: not implemented yet"), 3));
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
                    track_header_selected: c.selected_track.is_some() && c.selected_clip.is_none() && !c.pr.focused,
                    clips_selected: c.selected_clip.is_some() as usize,
                    midi_clip_selected: c.pr_visible(),
                    midi_editor_focused: c.pr_visible() && c.pr.focused,
                    ..Default::default()
                }
            };
            let Some(action) = keymap.resolve(&chord, &cx).map(str::to_owned) else { return false };
            // Settings is modal: only Escape and ⌘, reach the app while it is open.
            if ctl.borrow().settings.open && action != "view.escape" && action != "app.settings" {
                return true;
            }
            let handled = run_action(&ctl, &action);
            if let Some(a) = aw.upgrade() {
                if let Ok(mut c) = ctl.try_borrow_mut() {
                    c.update_ui(&a);
                }
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
            c.poll_test_tone();
            c.device_poll();
            c.auto_poll();
            if c.pending_jobs > 0 {
                c.poll_jobs();
                if let Some(a) = aw.upgrade() {
                    a.window().request_redraw();
                }
            }
            // Keep view models current outside the rendering notifier (see the `on!` macro).
            if c.ui_dirty
                && let Some(a) = aw.upgrade()
            {
                c.update_ui(&a);
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
        pr_renderer: None,
        pr_dl: gpu::DrawList::default(),
        pr_visible: false,
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
                    let mut f = fr.borrow_mut();
                    f.pr_renderer = gpu::with_wgpu(api, gpu::Renderer::new);
                    f.renderer = r;
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
                        let auto = c.auto_scene(scale);
                        let mut sc = c.scene(scale);
                        sc.auto = auto.as_ref();
                        timeline::build(&mut f.dl, &sc);
                        r.render(&f.dl);
                    }
                    // Piano roll texture (only while a MIDI clip is open in the Clip tab).
                    let pr_vis = c.pr_visible();
                    if pr_vis {
                        let win = app.window().size();
                        let size = ((win.width as f32 / scale) - 10.0 - pianoroll::PANEL_ORIGIN.0 - 0.5, pianoroll::CLIP_PANEL_H - pianoroll::PANEL_ORIGIN.1 - pianoroll::FOOTER_H);
                        if c.pr.size != size {
                            c.pr.size = size;
                            c.ui_dirty = true;
                        }
                        let notes = c.pr_notes();
                        if let (Some(r), Some(sc)) = (f.pr_renderer.as_mut(), c.pr_scene(scale, &notes)) {
                            let phys = ((size.0 * scale).round() as u32, (size.1 * scale).round() as u32);
                            if let Some(tex) = r.ensure_target(phys) {
                                match gpu::to_slint_image(tex) {
                                    Ok(img) => app.global::<Studio>().set_pr_image(img),
                                    Err(e) => eprintln!("piano roll texture import failed: {e}"),
                                }
                            }
                            pianoroll::build(&mut f.pr_dl, &sc);
                            r.render(&f.pr_dl);
                        }
                    }
                    f.pr_visible = pr_vis;
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
                        Ok(c) => (c.pending_jobs > 0 || c.engine.transport().playing || !c.assistant.highlights.is_empty() || c.rec.recording, c.pending_jobs == 0),
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
                slint::RenderingState::RenderingTeardown => {
                    let mut f = fr.borrow_mut();
                    f.renderer = None;
                    f.pr_renderer = None;
                }
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
        assert_eq!(k.chords_for("track.arm").len(), 1);
        assert_eq!(k.chords_for("track.monitor").len(), 1);
        assert_eq!(k.chords_for("transport.record").len(), 1);
        assert_eq!(k.chords_for("app.settings").len(), 1);
    }
}
