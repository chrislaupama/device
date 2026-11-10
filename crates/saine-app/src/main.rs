//! Saine Studio desktop application (milestone 1).
//!
//! Slint shell (FemtoVG on wgpu 30) + our own wgpu timeline renderer
//! (`gpu/`), wired to the real `saine_core::Document` and the C++ engine.
//!
//! ```text
//! saine-studio [--open <dir>] [--import <wav>...] [--save-to <dir>] [--no-audio]
//!              [--demo] [--state reference-default|reference-mixer|reference-assistant|reference-midi-clip|reference-pianoroll|reference-synth|reference-input-menu|reference-clip-menu|review-clip-menu|reference-settings-audio|reference-devices-effects|reference-automation|review-automation|review-automation-picker|review-recording|review-device-chain|review-settings-plugins]
//!              [--synth-page <0..8>  (with --state reference-synth: category page to show)]
//!              [--assistant-fixture <turns.json>  (TEST provider: canned replies, no network)]
//!              [--plugin-fixture <plugins.json>  (TEST seam: fixed plugin catalogue instead of scanning)]
//!              [--load-sample <file.wav>  (into the selected track's sampler, as a file drop)]
//!              [--sampler-tab <0..4>  (with a sampler state: Sample, Pitch, Filter, Amp or Mod tab)]
//!              [--hold-note <0..127>  (with a synth state: hold the note and render 0.75 s offline first)]
//!              [--screenshot <out.png>] [--offscreen] [--exit-after-frames N] [--frame-stats] [--render-bench N] [--self-test]
//!              [--window-width <pt>  (OS window width, default 1440)] [--assistant-width <px>  (saved docked-assistant width, 297..600)]
//!              [--timeline-zoom <percent>  (horizontal timeline zoom, 100 = default; 100 is 12.76 pt per beat)]
//!              [--zoom <percent>  (interface zoom, default 100 or the saved level; 85 = our design units)]
//!              [--clip-menu <0|1|3|4|5>  (open a clip editor dropdown after --state: Speed, Warp, Grid, Type, Genre; 2, Mode, is disabled)]
//!              [--card-accent <RRGGBB>  (device cards in that colour, as on the reference's Master track)]
//! ```

static CLIP_MENU_AUDIO: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

mod analyzer_views;
mod assistant_panel;
mod assistant_provider;
mod automation;
mod clipedit;
mod composer;
mod controller;
mod device_canvas;
mod device_mod;
mod device_fields;
mod device_panel;
mod device_editors;
mod panel_graph;
mod device_views;
mod gain_scope;
mod devices;
mod devminimap;
mod dialogs;
mod files;
mod panel_multiband;
mod gpu;
mod highlight;
mod live_set;
mod minimap;
mod mixer;
mod panel_fields;
mod peaks;
mod pianoroll;
mod placeholder;
mod pointer;
mod prefs;
mod recording;
mod sampler_panel;
mod selftest;
mod settings;
mod stepseq;
mod sections;
mod splitters;
mod synth;
mod text_measure;
mod timeline;
mod warpedit;
mod wavgen;
mod zoom;

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
    /// reference-assistant / review-assistant-<name>: canned transcript (assistant_panel::assistant_demo).
    assistant_demo: Option<String>,
    /// 1 = reference-midi-clip (33), 2 = reference-pianoroll (34).
    midi_state: u8,
    audio_clip_state: bool,
    /// review-warp: the audio clip state with drum hits, warped (Drums, 1.5x, one marker).
    warp_state: bool,
    /// review-reverse: review-warp with the clip reversed.
    reverse_state: bool,
    /// review-warp-stereo: review-warp from a stereo file (hits on the left only).
    stereo_state: bool,
    /// reference-synth (32): Instrument track selected, Devices tab, test-state synth preset.
    synth_state: bool,
    /// --synth-page N: open that category page of the synth device (review screenshots).
    synth_page: Option<usize>,
    /// reference-input-menu (50): Instrument + Audio tracks, Audio selected, "In" popup open.
    input_menu_state: bool,
    /// reference-clip-menu: Instrument track with a 1-bar MIDI clip at bar 6, its context menu open.
    clip_menu_state: bool,
    /// review-clip-menu: the first audio clip's menu open with the playhead inside it.
    clip_menu_audio: bool,
    /// Step sequencer review state: lane tab shown (0 Steps .. 4 Tie), or None.
    seq_state: Option<i32>,
    /// Clip sidebar pattern list review: 0 empty MIDI clip, 1 chords made, 2 drum patterns made.
    patterns_state: Option<i32>,
    /// reference-settings-audio (14): Settings open on the Audio page.
    settings_state: bool,
    /// 1 export dialog, 2 unsaved-changes prompt, 3 export dialog on Stems, 4 with its Tracks menu open, 5 on Project.
    dialog_state: u8,
    settings_general_state: bool,
    /// review-recording: a take in progress (injected input, offline engine) for reviewing
    /// the live recording clip. Not a reference capture (the reference recording UI was not captured).
    recording_review: bool,
    /// reference-automation (60): Instrument track 1 (MIDI clip at bar 6), Audio track 2 selected,
    /// playhead at bar 6, automation lanes shown (Track Volume at +0.0).
    automation_state: bool,
    /// reference-live (renders/reference/ui/z100-main-default.png, 100 % zoom): the default project
    /// with the assistant panel open and the automation lanes shown.
    live_state: bool,
    /// review-automation / review-automation-picker: lanes with points (active, disabled, curved,
    /// an expanded extra lane, master) for review; the second also opens the picker with the
    /// Synth submenu. No reference capture.
    automation_review: u8,
    /// review-track-heights / review-bottom-collapsed: the demo tracks at 0.25x, 0.5x and 2x
    /// height; the second instead collapses the bottom panel to its tab row (compare
    /// z100-arr-resize-timeline-taller.png).
    splitters_review: u8,
    /// review-files-projects / review-files-folders / review-files-project: Browser › Files with
    /// an empty projects folder (compare z100-browser-files-projects.png), with a generated
    /// "audio" folder of two WAVs open (compare z100-files-audio-folder-expanded.png), and with a
    /// saved project open to its tracks, audio files and MIDI clips (no reference).
    /// review-files-preview: the audio folder with sine-a440-pulsed.wav playing in the preview
    /// bar (compare z100-files-row-preview-playing.png).
    /// review-files-sort: the audio folder with the sort menu open.
    files_review: u8,
    /// review-section-added / review-sections-two: one new 8-bar section (compare
    /// z100-marker-after-plus-click.png), or two with the second selected.
    sections_review: u8,
    /// reference-devices-effects (05): browser Devices › Effects sub-tab.
    devices_effects_state: bool,
    /// review-device-chain: Instrument track with the Synth and one AU effect (no reference).
    device_chain_review: bool,
    /// review-devices-minimap: Instrument track with the Synth and a long chain of built-in effects (the Devices minimap shows).
    devices_minimap_review: bool,
    /// reference-<built-in device id>: that device alone in an audio track's chain (an instrument on an instrument track).
    device_state: Option<&'static str>,
    /// Files loaded as if dropped on the window (after the state is set up).
    load_sample: Vec<PathBuf>,
    /// --sampler-tab N: with a sampler state, show that tab (review screenshots).
    sampler_tab: Option<i32>,
    /// Test hook: hold this note on the selected synth and render 0.75 s offline before the
    /// first frame, so the pictures show a voice's live modulation.
    hold_note: Option<u8>,
    /// Never show the window (renders for --screenshot fall back to offscreen snapshots).
    offscreen: bool,
    /// review-analyzers: Pitch Corrector, Tuner and Spectrum fed 2 s of a sung-like line (no reference).
    analyzers_review: bool,
    /// reference-lfo / reference-chain: 1 an LFO aimed at a Compressor, 2 a chain head
    /// (macro panel open) holding a Compressor and the CLAP test plugin's card.
    modulation_state: u8,
    /// review-settings-plugins: Settings open on the Plugins page (no reference comparison).
    plugins_page_review: bool,
    assistant_fixture: Option<PathBuf>,
    plugin_fixture: Option<PathBuf>,
    screenshot: Option<PathBuf>,
    exit_after_frames: Option<u64>,
    zoom: Option<u32>,
    /// --window-width <pt>: OS window width in points (default 1440), for narrow-window reviews.
    window_width: Option<f32>,
    /// --assistant-width <px>: saved docked-assistant width in the reference's px (297..600), for reviews.
    assistant_width: Option<f32>,
    clip_menu: Option<i32>,
    /// --ac-edit N: with review-warp, open the typed-value box (0 Original BPM, 1 Transpose).
    ac_edit: Option<i32>,
    frame_stats: bool,
    /// --timeline-zoom <percent>: horizontal timeline zoom at start (100 = the default).
    timeline_zoom: Option<f64>,
    /// --render-bench N: offscreen, render the state N times and print the mean / median /
    /// p95 milliseconds per full window render (Slint + wgpu surfaces + readback), then exit.
    render_bench: u32,
    /// --drag-bench track|bottom: with --render-bench, move a track-height or splitter drag
    /// one step (controller update + view-model sync) before each render.
    drag_bench: Option<String>,
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
                    a.assistant_demo = Some("turn".into());
                }
                "review-assistant-narrow" => {
                    a.reference_default = true;
                    a.open_assistant = true;
                    a.assistant_demo = Some("turn".into());
                    a.assistant_width = Some(600.0);
                    a.window_width.get_or_insert(1470.0);
                }
                s if s.starts_with("review-assistant-") => {
                    a.reference_default = true;
                    a.open_assistant = true;
                    a.assistant_demo = s.strip_prefix("review-assistant-").map(String::from);
                }
                "reference-midi-clip" => {
                    a.reference_default = true;
                    a.midi_state = 1;
                }
                "reference-pianoroll" => {
                    a.reference_default = true;
                    a.midi_state = 2;
                }
                "reference-audio-clip" => {
                    a.reference_default = true;
                    a.audio_clip_state = true;
                }
                s @ ("review-warp" | "review-reverse" | "review-warp-stereo") => {
                    a.reference_default = true;
                    a.audio_clip_state = true;
                    a.warp_state = true;
                    a.reverse_state = s == "review-reverse";
                    a.stereo_state = s == "review-warp-stereo";
                }
                "reference-synth" => {
                    a.reference_default = true;
                    a.synth_state = true;
                }
                "reference-input-menu" => {
                    a.reference_default = true;
                    a.input_menu_state = true;
                }
                s if s.starts_with("review-patterns-") => {
                    a.reference_default = true;
                    a.patterns_state = s.rsplit('-').next().and_then(|l| l.parse().ok());
                }
                s if s.starts_with("review-step-sequencer") => {
                    a.reference_default = true;
                    a.seq_state = Some(s.rsplit('-').next().and_then(|l| l.parse().ok()).unwrap_or(0));
                }
                "reference-export" => {
                    a.reference_default = true;
                    a.dialog_state = 1;
                }
                "reference-unsaved" => {
                    a.reference_default = true;
                    a.dialog_state = 2;
                }
                "review-export-stems" => {
                    a.reference_default = true;
                    a.dialog_state = 3;
                }
                "review-export-stems-menu" => {
                    a.reference_default = true;
                    a.dialog_state = 4;
                }
                "review-export-project" => {
                    a.reference_default = true;
                    a.dialog_state = 5;
                }
                "reference-settings-general" => {
                    a.reference_default = true;
                    a.settings_state = true;
                    a.settings_general_state = true;
                }
                "reference-clip-menu" => {
                    a.reference_default = true;
                    a.clip_menu_state = true;
                }
                "review-clip-menu" => {
                    a.reference_default = true;
                    a.clip_menu_audio = true;
                }
                "reference-settings-audio" => {
                    a.reference_default = true;
                    a.settings_state = true;
                }
                "reference-live" => {
                    a.reference_default = true;
                    a.open_assistant = true;
                    a.live_state = true;
                }
                "reference-automation" => {
                    a.reference_default = true;
                    a.automation_state = true;
                }
                "review-track-heights" => {
                    a.reference_default = true;
                    a.splitters_review = 1;
                }
                "review-section-added" => {
                    a.reference_default = true;
                    a.sections_review = 1;
                }
                "review-sections-two" => {
                    a.reference_default = true;
                    a.sections_review = 2;
                }
                "review-bottom-collapsed" => {
                    a.reference_default = true;
                    a.splitters_review = 2;
                }
                s if s.starts_with("review-files-") => {
                    a.reference_default = true;
                    a.files_review = match s {
                        "review-files-projects" => 1,
                        "review-files-folders" => 2,
                        "review-files-project" => 3,
                        "review-files-preview" => 4,
                        "review-files-sort" => 5,
                        _ => return Err(format!("unknown state {s} (supported: review-files-projects, review-files-folders, review-files-project, review-files-preview, review-files-sort)")),
                    };
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
                "review-devices-minimap" => {
                    a.reference_default = true;
                    a.devices_minimap_review = true;
                }
                "reference-lfo" => {
                    a.reference_default = true;
                    a.modulation_state = 1;
                }
                "reference-chain" => {
                    a.reference_default = true;
                    a.modulation_state = 2;
                }
                s if s.strip_prefix("reference-").and_then(saine_core::builtin::find).is_some() => {
                    a.reference_default = true;
                    a.device_state = s.strip_prefix("reference-").and_then(saine_core::builtin::find).map(|d| d.id);
                }
                "review-settings-plugins" => {
                    a.reference_default = true;
                    a.plugins_page_review = true;
                }
                "review-analyzers" => {
                    a.reference_default = true;
                    a.analyzers_review = true;
                    a.audio = false;
                }
                "review-recording" => {
                    a.reference_default = true;
                    a.recording_review = true;
                    a.audio = false;
                }
                s => {
                    return Err(format!(
                        "unknown state {s} (supported: reference-default, reference-mixer, reference-assistant, reference-midi-clip, reference-pianoroll, reference-synth, reference-input-menu, reference-settings-audio, reference-devices-effects, reference-automation, reference-live, review-automation, review-automation-picker, review-recording, review-device-chain, review-devices-minimap, reference-<built-in device id> e.g. reference-compressor, reference-lfo, reference-chain, review-settings-plugins, review-analyzers, review-assistant-<empty|turn|point|draft|steps|error|busy|highlight|narrow>)"
                    ));
                }
            },
            "--synth-page" => a.synth_page = Some(value(&mut i)?.parse().map_err(|e| format!("--synth-page: {e}"))?),
            "--assistant-fixture" => a.assistant_fixture = Some(value(&mut i)?.into()),
            "--plugin-fixture" => a.plugin_fixture = Some(value(&mut i)?.into()),
            "--screenshot" => a.screenshot = Some(value(&mut i)?.into()),
            "--offscreen" => a.offscreen = true,
            "--card-accent" => {
                let hex = value(&mut i)?;
                let hex = hex.trim_start_matches('#');
                if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
                    return Err(format!("--card-accent: want six hex digits (RRGGBB), got {hex:?}"));
                }
                let c = u32::from_str_radix(hex, 16).map_err(|e| format!("--card-accent: {e}"))?;
                let _ = controller::CARD_ACCENT.set(timeline::rgb(c));
            }
            "--load-sample" => a.load_sample.push(value(&mut i)?.into()),
            "--hold-note" => a.hold_note = Some(value(&mut i)?.parse().map_err(|e| format!("--hold-note: {e}"))?),
            "--sampler-tab" => a.sampler_tab = Some(value(&mut i)?.parse().map_err(|e| format!("--sampler-tab: {e}"))?),
            "--exit-after-frames" => a.exit_after_frames = Some(value(&mut i)?.parse().map_err(|e| format!("--exit-after-frames: {e}"))?),
            "--clip-menu" => {
                let m: i32 = value(&mut i)?.parse().map_err(|e| format!("--clip-menu: {e}"))?;
                if ![0, 1, 3, 4, 5].contains(&m) {
                    return Err(format!("--clip-menu: want 0, 1, 3, 4 or 5 (2, Mode, is disabled), got {m}"));
                }
                a.clip_menu = Some(m);
            }
            "--window-width" => {
                let w: f32 = value(&mut i)?.parse().map_err(|e| format!("--window-width: {e}"))?;
                if !(600.0..=4000.0).contains(&w) {
                    return Err(format!("--window-width: want 600..4000 points, got {w}"));
                }
                a.window_width = Some(w);
            }
            "--assistant-width" => {
                let w: f32 = value(&mut i)?.parse().map_err(|e| format!("--assistant-width: {e}"))?;
                if !w.is_finite() {
                    return Err(format!("--assistant-width: {w} is not a width"));
                }
                a.assistant_width = Some(w.clamp(297.0, 600.0));
            }
            "--ac-edit" => a.ac_edit = Some(value(&mut i)?.parse().map_err(|e| format!("--ac-edit: {e}"))?),
            "--zoom" => a.zoom = Some(value(&mut i)?.trim_end_matches('%').parse().map_err(|e| format!("--zoom: {e}"))?),
            "--frame-stats" => a.frame_stats = true,
            "--timeline-zoom" => {
                let z: f64 = value(&mut i)?.trim_end_matches('%').parse().map_err(|e| format!("--timeline-zoom: {e}"))?;
                if z.is_nan() {
                    return Err(format!("--timeline-zoom: invalid percentage {z}"));
                }
                a.timeline_zoom = Some(z);
            }
            "--drag-bench" => {
                let v = value(&mut i)?;
                if v != "track" && v != "bottom" {
                    return Err(format!("--drag-bench: want track or bottom, got {v:?}"));
                }
                a.drag_bench = Some(v.to_string());
            }
            "--render-bench" => {
                a.render_bench = value(&mut i)?.parse().map_err(|e| format!("--render-bench: {e}"))?;
                if a.render_bench == 0 {
                    return Err("--render-bench: the count must be at least 1".into());
                }
                a.offscreen = true;
                a.audio = false;
            }
            "--self-test" | "--self-test-audio" => {
                a.self_test = true;
                a.demo = true;
                // --self-test-audio also plays ~1 s through the device at -40 dB master.
                a.audio = v[i] == "--self-test-audio";
            }
            "-h" | "--help" => {
                println!("{}", include_str!("main.rs").lines().take_while(|l| l.starts_with("//!")).skip(6).filter(|l| !l.contains("```")).map(|l| l.trim_start_matches("//! ")).collect::<Vec<_>>().join("\n"));
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
    rfd::FileDialog::new().set_title("Import audio").add_filter("Audio", &crate::files::dialog_exts()).pick_files().unwrap_or_default()
}

fn pick_project_dir(save: bool) -> Option<PathBuf> {
    if save {
        // A project is a directory; ask for its name and create it.
        // Start in the projects folder Browser › Files › Projects lists.
        let dialog = rfd::FileDialog::new().set_title("Save project").set_file_name(format!("Untitled.{}", saine_core::persist::EXTENSION));
        let dialog = match crate::files::FilesUi::default().projects_dir.filter(|d| std::fs::create_dir_all(d).is_ok()) {
            Some(d) => dialog.set_directory(d),
            None => dialog,
        };
        dialog.save_file()
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
        "clip.split_at_marker" => ctl.borrow_mut().split_clip(),
        "clip.disable" => ctl.borrow_mut().toggle_clip_disabled(),
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
            if c.devmm_cancel() || c.editor_cancel() || c.panel_cancel_drag() || c.panel_cancel_field() || c.panel_cancel_canvas() || c.mod_disarm() || (c.synth.menu.is_none() && c.synth_cancel()) {
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
            c.rename_section = None;
            c.pr.menu = -1;
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
        "file.new" => ctl.borrow_mut().new_project_request(),
        "file.export_audio" => ctl.borrow_mut().export_open(),
        _ => return false,
    }
    true
}

/// Synth test state read off the reference captures (osc 2 FM, LFO 1 slow, 2 routes, one macro
/// mapping, Tape off / Chorus / Delay / Reverb); not a factory sound.
fn synth_test_state(device: saine_core::DeviceId) -> saine_core::Command {
    let params = [
        ("osc1_wave", 1.0), ("osc1_shape", 0.1), ("osc1_level", 0.8), ("osc2_wave", 5.0), ("osc2_shape", 0.25), ("osc2_octave", 3.0), ("osc2_fine", 7.0), ("osc2_level", 0.3),
        ("sub_level", 0.0), ("cutoff", 2400.0), ("resonance", 0.1), ("filter_drive", 0.2), ("filter_env", 0.25), ("filter_velocity", 0.35), ("env2_attack", 0.002),
        ("amp_attack", 0.004), ("amp_decay", 0.9 * 1.0), ("amp_sustain", 0.35), ("amp_release", 1.1), ("volume", -0.74), ("env2_decay", 0.9), ("env2_sustain", 0.1),
        ("lfo1_rate", 0.18), ("lfo2_rate", 5.0), ("unison_voices", 2.0), ("unison_detune", 0.18), ("unison_spread", 0.8),
        ("mod1_source", 3.0), ("mod1_target", 5.0), ("mod1_amount", 0.2), ("mod2_source", 7.0), ("mod2_target", 11.0), ("mod2_amount", 0.35), ("mod3_source", 0.0), ("mod3_target", 0.0), ("mod3_amount", 0.0),
        ("macro1_name", 12.0), ("macro2_name", 18.0), ("macro3_name", 7.0), ("macro4_name", 6.0), ("map1_macro", 1.0), ("map1_param", 22.0), ("map1_from", 0.693), ("map1_to", 0.943),
        ("fx1_on", 0.0), ("fx3_type", 7.0), ("fx3_on", 1.0), ("fx4_type", 8.0), ("fx4_on", 1.0),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect();
    saine_core::Command::SetDeviceParams { device, params, preset: Some("Test: reference-synth".into()) }
}

fn main() {
    if let Err(e) = run() {
        eprintln!("saine-studio: {e}");
        std::process::exit(1);
    }
}

/// AppKit adds Start Dictation, Emoji & Symbols and Enter Full Screen to any Edit and View
/// menus; the reference's menus have none of them. Registered (not saved) defaults.
#[cfg(target_os = "macos")]
fn suppress_appkit_menu_items() {
    use objc2_foundation::{NSDictionary, NSNumber, NSUserDefaults, ns_string};
    let yes = NSNumber::new_bool(true);
    let no = NSNumber::new_bool(false);
    let keys = [ns_string!("NSDisabledDictationMenuItem"), ns_string!("NSDisabledCharacterPaletteMenuItem"), ns_string!("NSFullScreenMenuItemEverywhere")];
    let values: [&objc2::runtime::AnyObject; 3] = [yes.as_ref(), yes.as_ref(), no.as_ref()];
    let defaults = NSDictionary::from_slices(&keys, &values);
    unsafe { NSUserDefaults::standardUserDefaults().registerDefaults(&defaults) };
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args = parse_args()?;
    #[cfg(target_os = "macos")]
    suppress_appkit_menu_items();

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
    // Interface zoom: --zoom, else the saved level, else 100 %. Self-tests keep our
    // design units (85 %) so their scripted geometry is unchanged.
    let zoom_percent = args.zoom.unwrap_or(if args.self_test { 85 } else if saved_prefs.interface_zoom > 0 { saved_prefs.interface_zoom } else { zoom::DEFAULT_PERCENT });
    let zoom = Rc::new(RefCell::new(zoom::Zoom::new(zoom_percent)));
    ctl.borrow().bind_models(&app);
    {
        // Provider: TEST fixture (flag, or the built-in one for --self-test), else the API key env var.
        let provider = match (&args.assistant_fixture, args.self_test) {
            (Some(path), _) => {
                let text = std::fs::read_to_string(path).map_err(|e| format!("--assistant-fixture {}: {e}", path.display()))?;
                assistant_provider::Provider::fixture_from_json(&text, &path.display().to_string())?
            }
            (None, true) => assistant_provider::Provider::fixture_from_json(include_str!("../fixtures/assistant-selftest.json"), "built-in self-test fixture")?,
            // Review renders: a stand-in provider so the composer shows its enabled state (never sends).
            (None, false) if args.assistant_demo.is_some() => assistant_provider::Provider::fixture_from_json("{\"actions\": []}", "assistant demo")?,
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
            pointer::simulate(true);
        }
        c.mixer_open = args.open_mixer;
        c.assistant_open = args.open_assistant;
        if let Some(px) = args.assistant_width {
            c.settings.prefs.assistant_width = px * zoom::DESIGN_LEVEL;
        }
        if let Some(d) = &args.assistant_demo {
            c.assistant_demo(d)?;
            if d == "draft" {
                app.global::<Studio>().set_chat_draft("Make the lead brighter".into());
            }
        }
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
            let device = c.project().track(t).and_then(|t| t.instrument_plugin.as_ref()).map(|d| d.id);
            if let Some(device) = device {
                c.execute(synth_test_state(device));
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
    CLIP_MENU_AUDIO.store(args.clip_menu_audio, std::sync::atomic::Ordering::Relaxed);
    if args.clip_menu_state {
        let mut c = ctl.borrow_mut();
        if let Some(inst) = c.project().tracks.iter().find(|t| t.kind == saine_core::TrackKind::Instrument).map(|t| t.id) {
            c.select_track(inst);
            c.seek_beats(20.0);
            if let Some(id) = c.insert_midi_clip() {
                c.open_clip_menu(id, 700.0, 215.0, (10_000.0, 10_000.0));
            }
        }
    }
    if let Some(state) = args.patterns_state {
        let mut c = ctl.borrow_mut();
        if let Some(inst) = c.project().tracks.iter().find(|t| t.kind == saine_core::TrackKind::Instrument).map(|t| t.id) {
            c.select_track(inst);
            if state == 2 {
                c.execute(saine_core::Command::SetInstrumentPlugin { track: inst, device: Some(saine_core::builtin::drum_rack::DRUM_RACK.instantiate()) });
            }
            c.seek_beats(20.0);
            if let Some(id) = c.insert_midi_clip() {
                c.select_clip(Some(id));
                if state >= 1 {
                    c.composer_create();
                }
            }
        }
    }
    if let Some(lane) = args.seq_state {
        // Drum Rack track with a one-bar drum pattern (reference z100-clip-pattern-sequencer-*:
        // Pad 1 on steps 1, 5, 9, 13, Pad 2 on 5, Pad 3 on 9), plus lane values to show.
        use saine_core::steps::StepLane;
        let mut c = ctl.borrow_mut();
        if let Some(inst) = c.project().tracks.iter().find(|t| t.kind == saine_core::TrackKind::Instrument).map(|t| t.id) {
            c.select_track(inst);
            c.execute(saine_core::Command::SetInstrumentPlugin { track: inst, device: Some(saine_core::builtin::drum_rack::DRUM_RACK.instantiate()) });
            c.seek_beats(16.0);
            if let Some(id) = c.insert_midi_clip() {
                let mut p = c.project().midi_clip(id).and_then(|m| m.pattern.clone()).unwrap_or_default();
                for (key, step) in [(36, 0), (36, 4), (36, 8), (36, 12), (37, 4), (38, 8)] {
                    p = p.toggled(key, step);
                }
                if lane == 5 {
                    // An 80-step pattern (a 20-beat clip converted past the 64-step Length choices),
                    // playing on step 69: Follow playhead shows page 3 with the ring on step 69.
                    let mut long = saine_core::steps::StepPattern { steps: 80, ..p.clone() };
                    for bar in 1..5 {
                        for c in &p.cells {
                            long = long.toggled(c.key, c.step + bar * 16);
                        }
                    }
                    p = long.toggled(42, 68).toggled(42, 70);
                    if let Some(m) = c.project().midi_clip(id).cloned() {
                        c.execute(saine_core::Command::ResizeMidiClip { clip: id, start: m.start, length: p.length() });
                    }
                }
                if (1..5).contains(&lane) || lane == 6 {
                    p = p.toggled(42, 2).toggled(42, 6).toggled(42, 10).toggled(42, 14).with_row_flag(39, false);
                    p = p.with_lane(42, 2, StepLane::Velocity, 60).with_lane(42, 10, StepLane::Velocity, 30);
                    p = p.with_lane(42, 6, StepLane::Offset, 25).with_lane(42, 14, StepLane::Offset, -20);
                    p = p.with_lane(37, 4, StepLane::Repeat, 3).with_lane(38, 8, StepLane::Tie, 2);
                }
                c.execute(saine_core::Command::SetStepPattern { clip: id, pattern: Some(p) });
                c.select_clip(Some(id));
                c.seq_lane(match lane {
                    5 => 0,
                    6 => 1,
                    l => l,
                });
                if lane == 5 {
                    c.seek_beats(16.0 + 17.0);
                    let _ = c.engine.play();
                }
                if lane == 6 {
                    // The Velocity tab's tooltip, as after a second of hover.
                    let s = app.global::<Studio>();
                    s.set_seq_tip(StepLane::Velocity.spec().tip.into());
                    s.set_seq_tip_ax(208.0 - 13.35);
                    s.set_seq_tip_ay(24.0);
                    s.set_seq_tip_on(true);
                }
            }
        }
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
    if args.live_state {
        let mut c = ctl.borrow_mut();
        let first = c.project().tracks[0].id;
        c.select_track(first);
        c.auto_toggle_shown();
    }
    if args.splitters_review == 1 {
        let mut c = ctl.borrow_mut();
        let ids: Vec<_> = c.project().tracks.iter().map(|t| t.id).collect();
        for (track, height) in ids.into_iter().zip([0.25, 0.5, 2.0]) {
            let _ = c.doc.set_track_height(track, height);
        }
    }
    if args.sections_review > 0 {
        let mut c = ctl.borrow_mut();
        for _ in 0..args.sections_review {
            c.insert_section(None);
        }
        c.ui_dirty = true;
    }
    if args.splitters_review == 2 {
        ctl.borrow_mut().doc.set_bottom_panel_height("devices", Some(0.0));
    }
    if args.files_review > 0 {
        // Generated media and projects in a temporary folder, never the user's own files.
        let root = std::env::temp_dir().join(format!("saine-review-files-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let projects = root.join("Projects");
        let _ = std::fs::create_dir_all(&projects);
        let audio = root.join("audio");
        let _ = std::fs::create_dir_all(&audio);
        let rate = 48_000;
        let steady: Vec<f32> = (0..rate * 2).map(|i| 0.5 * (std::f32::consts::TAU * 220.0 * i as f32 / rate as f32).sin()).collect();
        let _ = wavgen::write_wav(&audio.join("sine-a220-steady.wav"), rate as u32, 1, &steady);
        let _ = wavgen::write_wav(&audio.join("sine-a440-pulsed.wav"), rate as u32, 1, &wavgen::sine_bursts(rate as u32));
        let mut c = ctl.borrow_mut();
        c.files.projects_dir = Some(projects.clone());
        if args.files_review == 3 {
            let mut p = c.project().clone();
            p.assets.push(saine_core::Asset { id: saine_core::AssetId(p.next_id), relative_path: None, absolute_path: audio.join("sine-a220-steady.wav").to_string_lossy().into_owned(), sample_rate: rate as f64, channels: 1, frames: rate as u64 * 2 });
            p.next_id += 1;
            p.tracks[1].midi_clips.push(saine_core::MidiClip::empty(0, 4 * saine_core::PPQ));
            let song = projects.join(format!("Song.{}", saine_core::persist::EXTENSION));
            let _ = saine_core::persist::save(&p, &song);
            c.browser_section(files::SECTION_FILES);
            c.files_tab(files::TAB_PROJECTS);
            c.files_expand(files::FileItem::Project(song));
        } else if args.files_review >= 2 {
            c.files_add_folder(&audio);
            c.files_expand(files::FileItem::Folder(audio.clone()));
            c.files.sort_open = args.files_review == 5;
            if args.files_review == 4 {
                // Preview of the pulsed file, shown 30 % in (no audio device: the voice never advances).
                let path = audio.join("sine-a440-pulsed.wav");
                let loaded = c.engine.load_file(&path).map_err(|e| e.to_string());
                c.files.selected = Some(path.to_string_lossy().into_owned());
                c.files.audition.loading = Some(path.clone());
                c.finish_audition(path, loaded);
                if let Some((id, frames)) = c.files.audition.asset {
                    c.audition_play(id, frames * 3 / 10);
                }
            }
        } else {
            c.browser_section(files::SECTION_FILES);
            c.files_tab(files::TAB_PROJECTS);
        }
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
    // The dropdown opens once its editor is on screen (the audio clip state imports first).
    let clip_menu_timer = slint::Timer::default();
    if let Some(m) = args.clip_menu {
        let ctl = ctl.clone();
        // Opens after its editor has been on screen for 10 ticks, so the editor is drawn first.
        let shown_ticks = std::cell::Cell::new(0u32);
        let started = Instant::now();
        clip_menu_timer.start(slint::TimerMode::Repeated, Duration::from_millis(20), move || {
            if shown_ticks.get() < 10 && started.elapsed() > Duration::from_secs(3) {
                eprintln!("saine-studio: --clip-menu {m}: its editor never appeared; use a --state that shows it (0, 1: an audio clip; 3, 4, 5: a MIDI clip)");
                std::process::exit(2);
            }
            let Ok(mut c) = ctl.try_borrow_mut() else { return };
            if shown_ticks.get() > 10 || !c.clip_menu_shown(m) {
                shown_ticks.set(if c.clip_menu_shown(m) { shown_ticks.get() } else { 0 });
                return;
            }
            shown_ticks.set(shown_ticks.get() + 1);
            if shown_ticks.get() == 10 {
                c.clip_menu_open(m);
            }
        });
    }
    if args.settings_state {
        let mut c = ctl.borrow_mut();
        c.open_settings();
        if args.settings_general_state {
            c.settings_page(0);
        }
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
    if args.devices_minimap_review {
        // Review and benchmark: the Instrument track's Synth followed by ten built-in effects, so the chain is
        // several panels wide.
        let mut c = ctl.borrow_mut();
        if let Some(t) = c.project().tracks.iter().find(|t| t.kind == saine_core::TrackKind::Instrument).map(|t| t.id) {
            c.select_track(t);
            for id in ["compressor", "channeleq", "delay", "reverb", "chorus", "auto-filter", "limiter", "utility", "widener", "phaser"] {
                if let Some(def) = saine_core::builtin::find(id) {
                    c.add_item_to_track(&devices::Item::Builtin(def), t, None);
                }
            }
            c.devices.chain_scroll = 700.0;
        }
        c.set_bottom_tab(BottomTab::Devices);
        c.seek_beats(20.0);
    }
    if let Some(id) = args.device_state {
        // Compare with renders/reference/<id>.png (Captain drive): the built-in device at its
        // defaults, alone in an audio track's chain (an instrument on an instrument track),
        // transport stopped.
        let mut c = ctl.borrow_mut();
        let def = saine_core::builtin::find(id).expect("built-in is registered");
        let kind = if def.kind == saine_core::PluginKind::Instrument { saine_core::TrackKind::Instrument } else { saine_core::TrackKind::Audio };
        if let Some(t) = c.project().tracks.iter().find(|t| t.kind == kind).map(|t| t.id) {
            c.select_track(t);
            c.add_item_to_track(&devices::Item::Builtin(def), t, None);
            let added = c.project().track(t).and_then(|t| t.instrument_plugin.as_ref().filter(|_| id == "synth-v2")).map(|d| d.id);
            if let Some(device) = added {
                c.execute(synth_test_state(device));
            }
        }
        c.set_bottom_tab(BottomTab::Devices);
        c.seek_beats(8.0);
    }
    if args.analyzers_review {
        // Offline engine + injected input monitored through the audio track: a harmonic line
        // (A3 +35 ct with vibrato, then C4 -30 ct) through Pitch Corrector (C major, fast),
        // Tuner and Spectrum, rendered in 30 ms steps with the views polled after each step.
        let mut c = ctl.borrow_mut();
        c.rec.policy = recording::InputPolicy::InjectedOnly;
        if let Some(t) = c.project().tracks.iter().find(|t| t.kind == saine_core::TrackKind::Audio).map(|t| t.id) {
            c.select_track(t);
            for id in ["pitch-corrector", "tuner", "spectrum"] {
                let def = saine_core::builtin::find(id).expect("built-in is registered");
                c.add_item_to_track(&devices::Item::Builtin(def), t, None);
            }
            let pc = c.project().track(t).and_then(|tr| tr.devices.first()).map(|d| d.id);
            if let Some(d) = pc {
                for (param, value) in [("key_follow", 0.0), ("scale", 1.0), ("speed", 100.0), ("retune_speed_ms", 20.0), ("humanize", 15.0)] {
                    c.execute(saine_core::Command::SetDeviceParam { device: d, param: param.into(), value });
                }
            }
            c.execute(saine_core::Command::SetTrackMonitor { track: t, monitor: true });
            c.set_track_input(t, Some(saine_core::InputSource { channel: 0, stereo: false }));
        }
        c.set_bottom_tab(BottomTab::Devices);
        let rate = 48_000.0f64;
        let mut phase = 0.0f64;
        let mut at = std::time::Instant::now() - std::time::Duration::from_secs(2);
        let step = 1440usize;
        for k in 0..(2 * 48_000 / step) {
            let input: Vec<f32> = (0..step)
                .map(|i| {
                    let t = (k * step + i) as f64 / rate;
                    let midi = if t < 1.0 { 57.35 + 0.25 * (std::f64::consts::TAU * 5.0 * t).sin() } else { 59.7 };
                    phase += std::f64::consts::TAU * 440.0 * 2f64.powf((midi - 69.0) / 12.0) / rate;
                    ((1..=8).map(|h| (phase * h as f64).sin() / h as f64).sum::<f64>() * 0.2) as f32
                })
                .collect();
            let mut out = vec![0f32; 2 * step];
            if let Err(e) = c.engine.render_offline_with_input(&input, 1, &mut out) {
                c.set_status(format!("review-analyzers: {e}"), 10);
                break;
            }
            at += std::time::Duration::from_millis(30);
            c.panel_poll_views_at(at);
        }
    }
    if args.modulation_state > 0 {
        // No the reference app capture of an LFO card exists; compare the chain with
        // renders/reference/chain-head.png (Captain drive).
        let mut c = ctl.borrow_mut();
        if let Some(t) = c.project().tracks.iter().find(|t| t.kind == saine_core::TrackKind::Audio).map(|t| t.id) {
            c.select_track(t);
            let comp = saine_core::builtin::find("compressor").expect("compressor is registered");
            if args.modulation_state == 1 {
                c.add_item_to_track(&devices::Item::Builtin(comp), t, None);
                let id = c.project().track(t).and_then(|t| t.devices.last()).map(|d| d.id);
                if let Some(id) = id {
                    c.mod_new_lfo(id, saine_core::ModParam::Builtin("threshold".into()));
                }
            } else {
                c.add_item_to_track(&devices::Item::Builtin(&saine_core::builtin::rack::RACK), t, None);
                let head = c.project().track(t).and_then(|t| t.devices.last()).map(|d| d.id);
                if let Some(head) = head {
                    c.add_to_chain(head, devices::Item::Builtin(comp));
                    let gain = saine_core::PluginRef {
                        format: saine_core::PluginFormat::Clap,
                        id: "studio.saine.test.gain".into(),
                        path: Some(saine_engine::test_plugin_dir().join("SaineTestGain.clap").to_string_lossy().into_owned()),
                        name: "Saine Test Gain".into(),
                        vendor: "Saine".into(),
                        version: "1.0.0".into(),
                        kind: saine_core::PluginKind::Effect,
                    };
                    c.add_to_chain(head, devices::Item::Plugin(gain));
                    let ids: Vec<saine_core::DeviceId> = c.project().track(t).map(|t| t.devices.iter().map(|d| d.id).collect()).unwrap_or_default();
                    if let [_, comp, plug] = ids[..] {
                        if let Some(p) = c.engine.plugin_params(plug).first().map(|p| p.id) {
                            c.card_toggle_param(plug, p);
                        }
                        c.mod_link(head, 0, comp, saine_core::ModParam::Builtin("threshold".into()));
                        c.macro_select(head, 0);
                    }
                }
            }
        }
        c.set_bottom_tab(BottomTab::Devices);
        c.seek_beats(8.0);
    }
    if !args.load_sample.is_empty() {
        ctl.borrow_mut().drop_files(args.load_sample.clone());
    }
    if let Some(tab) = args.sampler_tab {
        let mut c = ctl.borrow_mut();
        if let Some(d) = c.selected_sampler() {
            c.sampler_tab(d, tab);
        }
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
    on!(on_export_close, |c| c.dialogs_close());
    on!(on_export_run, |c| c.export_run());
    on!(on_export_bits, |c, b: bool| c.export_set_bits(b));
    on!(on_export_pick_recipe, |c, i: i32| c.export_pick_recipe(i));
    on!(on_export_toggle_tracks, |c| c.export_toggle_tracks_menu());
    on!(on_export_pick_tracks, |c, i: i32| c.export_pick_tracks(i));
    on!(on_export_name_edited, |c, t: slint::SharedString| c.export_set_name(t.as_str()));
    on!(on_unsaved_cancel, |c| c.dialogs_close());
    on!(on_unsaved_dont_save, |c| c.new_project());
    // Folder picker and Save run their native dialogs with the controller NOT borrowed.
    {
        let ctl = ctl.clone();
        let aw = app.as_weak();
        studio.on_export_choose_dir(move || {
            let start = ctl.borrow().export.dir.clone();
            if let Some(dir) = rfd::FileDialog::new().set_title("Export to").set_directory(&start).pick_folder() {
                ctl.borrow_mut().export_set_dir(dir);
            }
            if let Some(a) = aw.upgrade() {
                ctl.borrow_mut().update_ui(&a);
            }
        });
    }
    {
        let ctl = ctl.clone();
        let aw = app.as_weak();
        studio.on_unsaved_save(move || {
            let existing = ctl.borrow().project_dir.clone();
            if let Some(dir) = existing.or_else(|| pick_project_dir(true)) {
                let mut c = ctl.borrow_mut();
                c.save_to(&dir);
                if !c.doc.is_dirty() {
                    c.new_project();
                }
            }
            if let Some(a) = aw.upgrade() {
                ctl.borrow_mut().update_ui(&a);
            }
        });
    }
    on!(on_menu_action, |c, id: slint::SharedString| c.menu_action(&id));
    {
        // A menu item with a shortcut takes that keystroke away from the window (macOS key
        // equivalents), so replay it: a focused text field, the modal gating and the keymap
        // then handle it exactly as if the key had been pressed. Deferred, so the replay
        // runs outside the menu's own event.
        let aw = app.as_weak();
        studio.on_menu_key(move |text, primary, shift| {
            let aw = aw.clone();
            slint::Timer::single_shot(std::time::Duration::ZERO, move || {
                use slint::platform::{Key, WindowEvent};
                let Some(a) = aw.upgrade() else { return };
                let s = a.global::<Studio>();
                s.set_menu_replay(true);
                let w = a.window();
                let mods: Vec<slint::SharedString> = [(primary, Key::Control), (shift, Key::Shift)].into_iter().filter(|m| m.0).map(|m| m.1.into()).collect();
                for m in &mods {
                    w.dispatch_event(WindowEvent::KeyPressed { text: m.clone() });
                }
                w.dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
                w.dispatch_event(WindowEvent::KeyReleased { text: text.clone() });
                for m in mods.iter().rev() {
                    w.dispatch_event(WindowEvent::KeyReleased { text: m.clone() });
                }
                s.set_menu_replay(false);
            });
        });
    }
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
    on!(on_track_pan_text, |c, id: i32, t: slint::SharedString| {
        if let Some(p) = controller::parse_pan_text(&t) {
            c.set_track_pan(TrackId(id as u64), p);
        }
    });
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
    on!(on_chat_resize, |c, phase: i32, x: f32| c.chat_resize(phase, x));
    on!(on_chat_resize_reset, |c| c.chat_resize_reset());
    on!(on_bottom_resize, |c, phase: i32, y: f32| c.bottom_resize(phase, y));
    on!(on_track_resize, |c, id: i32, phase: i32, y: f32| c.track_resize_drag(id, phase, y));
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
    on!(on_lane_double, |c, x: f32, y: f32| {
        c.section_double(x, y);
    });
    on!(on_lane_pointer, |c, kind: i32, x: f32, y: f32, shift: bool, _cmd: bool, alt: bool| c.pointer_mods(kind, x, y, shift, alt));
    on!(on_minimap_pointer, |c, kind: i32, x: f32, y: f32, w: f32, ox: f32, oy: f32| c.minimap_pointer(kind, x, y, w, ox, oy));
    on!(on_devmm_pointer, |c, kind: i32, x: f32, y: f32, ox: f32, oy: f32| c.devmm_pointer(kind, x, y, ox, oy));
    on!(on_devmm_scroll, |c, dx: f32, dy: f32| c.devmm_wheel(dx, dy));
    on!(on_minimap_double, |c| c.minimap_fit());
    on!(on_minimap_scroll, |c, dx: f32, dy: f32, x: f32, w: f32, zoom: bool| c.minimap_scroll(dx, dy, x, w, zoom));
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
    on!(on_pr_pointer, |c, kind: i32, x: f32, y: f32, shift: bool, alt: bool, command: bool| c.pr_pointer(kind, x, y, shift, alt, command));
    on!(on_pr_scroll, |c, dy: f32| c.pr_scroll(dy));
    on!(on_pr_action, |c, a: slint::SharedString| c.pr_action(a.as_str()));
    on!(on_clip_menu_open, |c, m: i32| c.clip_menu_open(m));
    on!(on_clip_menu_pick, |c, i: i32| c.clip_menu_pick(i));
    on!(on_ac_bpm_drag, |c, phase: i32, dx: f32, dy: f32| c.ac_bpm_drag(phase, dx, dy));
    on!(on_ac_reverse_toggle, |c| c.ac_reverse_toggle());
    on!(on_ac_transpose_drag, |c, phase: i32, dx: f32, dy: f32| c.ac_transpose_drag(phase, dx, dy));
    on!(on_ac_edit_start, |c, f: i32| c.ac_edit_start(if f == 0 { warpedit::AcEdit::OriginalBpm } else { warpedit::AcEdit::Transpose }));
    on!(on_ac_edit_commit, |c, text: slint::SharedString| c.ac_edit_commit(&text));
    on!(on_ac_edit_cancel, |c| c.ac_edit_cancel());
    on!(on_pr_value, |c, f: i32, phase: i32, dy: f32| c.pr_value(f, phase, dy));
    on!(on_seq_tab, |c, piano: bool| c.seq_tab(piano));
    on!(on_seq_lane_pick, |c, i: i32| c.seq_lane(i));
    on!(on_seq_action, |c, a: slint::SharedString| c.seq_action(a.as_str()));
    on!(on_seq_pointer, |c, kind: i32, x: f32, y: f32| c.seq_pointer(kind, x, y));
    on!(on_seq_scroll, |c, dy: f32| c.seq_scroll(dy));
    on!(on_seq_swing, |c, phase: i32, dy: f32| c.seq_swing(phase, dy));
    on!(on_seq_page_pick, |c, i: i32| c.seq_page(i));
    on!(on_seq_follow_toggle, |c| c.seq_follow());
    on!(on_composer_create, |c| c.composer_create());
    on!(on_composer_load, |c, i: i32| c.composer_load(i.max(0) as usize));
    on!(on_composer_original_pick, |c| c.composer_original());
    on!(on_menu_activate, |c, i: i32| c.menu_activate(i.max(0) as usize));
    on!(on_sub_activate, |c, i: i32| c.submenu_activate(i.max(0) as usize));
    on!(on_menu_dismiss, |c| c.menu_close());
    on!(on_rename_commit, |c, t: slint::SharedString| {
        if let Some(id) = c.rename_section {
            c.rename_section(id, t.as_str());
        } else if let Some(id) = c.rename {
            c.rename_clip(id, t.as_str());
        }
    });
    on!(on_rename_cancel, |c| {
        c.rename = None;
        c.rename_section = None;
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
    // ---- browser sections, Back / Forward, Files (src/files.rs) ----
    on!(on_browser_section_select, |c, i: i32| c.browser_section(i.max(0) as usize));
    on!(on_browser_back, |c| c.browser_back());
    on!(on_browser_forward, |c| c.browser_forward());
    on!(on_files_tab_select, |c, i: i32| c.files_tab(i.max(0) as usize));
    on!(on_files_sort_open_menu, |c, open: bool| c.files_sort_open_menu(open));
    on!(on_files_sort_pick, |c, i: i32| c.files_sort_pick(i.max(0) as usize));
    on!(on_files_click, |c, i: i32, chevron: bool| c.files_click(i.max(0) as usize, chevron));
    on!(on_files_activate, |c, i: i32| c.files_activate(i.max(0) as usize));
    on!(on_files_drag, |c, i: i32, phase: i32, x: f32, y: f32| c.files_drag(i.max(0) as usize, phase, x, y));
    on!(on_files_scroll_by, |c, dy: f32| c.files_scroll_by(dy));
    on!(on_preview_toggle, |c| c.audition_toggle());
    on!(on_preview_step, |c, dir: i32| c.audition_step(dir));
    on!(on_preview_volume_moved, |c, p: f32| c.audition_volume(p));
    on!(on_preview_volume_done, |c| c.save_prefs());
    on!(on_preview_solo_toggle, |c| c.audition_solo());
    on!(on_preview_reveal, |c| c.audition_reveal());
    {
        // The folder dialog runs with the controller NOT borrowed (its modal loop keeps rendering).
        let ctl = ctl.clone();
        studio.on_files_add_folder(move || {
            if let Some(dir) = rfd::FileDialog::new().set_title("Add folder").pick_folder() {
                ctl.borrow_mut().files_add_folder(&dir);
            }
        });
    }
    on!(on_device_power, |c, id: i32| c.device_power(saine_core::DeviceId(id as u64)));
    on!(on_device_edit, |c, id: i32| c.device_edit(saine_core::DeviceId(id as u64)));
    on!(on_device_remove, |c, id: i32| c.device_remove(saine_core::DeviceId(id as u64)));
    on!(on_device_use_synth, |c| c.device_use_synth());
    on!(on_device_drag, |c, id: i32, phase: i32, x: f32| c.device_drag(saine_core::DeviceId(id as u64), phase, x));
    on!(on_panel_drag, |c, id: i32, p: i32, phase: i32, dy: f32, rate: f32, fine: bool| c.panel_drag(saine_core::DeviceId(id as u64), p, phase, dy, rate, fine));
    on!(on_panel_drag_pair, |c, id: i32, p: i32, q: i32, phase: i32, dy: f32, rate: f32, fine: bool| c.panel_drag_pair(saine_core::DeviceId(id as u64), p, q, phase, dy, rate, fine));
    on!(on_panel_reset, |c, id: i32, p: i32| c.panel_reset(saine_core::DeviceId(id as u64), p));
    on!(on_panel_reset_xy, |c, id: i32, px: i32, py: i32| c.panel_reset_xy(saine_core::DeviceId(id as u64), [px, py]));
    on!(on_panel_step, |c, id: i32, p: i32, delta: i32, fine: bool| c.panel_step(saine_core::DeviceId(id as u64), p, delta, fine));
    on!(on_panel_drag_xy, |c, id: i32, px: i32, py: i32, phase: i32, dx: f32, dy: f32, rx: f32, ry: f32, fine: bool| c.panel_drag_xy(saine_core::DeviceId(id as u64), [px, py], phase, [dx, dy], [rx, ry], fine));
    on!(on_panel_choice, |c, id: i32, p: i32, i: i32| c.panel_choice(saine_core::DeviceId(id as u64), p, i));
    on!(on_panel_text, |c, id: i32, p: i32, text: slint::SharedString| c.panel_text(saine_core::DeviceId(id as u64), p, &text));
    on!(on_panel_field, |c, id: i32, phase: i32, dx: f32, dy: f32, fine: bool| c.panel_field_drag(saine_core::DeviceId(id as u64), phase, dx, dy, fine));
    on!(on_panel_menu, |c, id: i32, which: i32, x: f32, y: f32| c.panel_menu(saine_core::DeviceId(id as u64), which, x, y));
    on!(on_panel_preset_step, |c, id: i32, dir: i32| c.panel_preset_step(saine_core::DeviceId(id as u64), dir));
    on!(on_panel_latch, |c, id: i32, p: i32, phase: i32| c.panel_latch(saine_core::DeviceId(id as u64), p, phase));
    on!(on_panel_editor, |c, id: i32, kind: i32, x: f32, y: f32, wx: f32, wy: f32, shift: bool, alt: bool, wheel: f32| c.panel_editor(
        saine_core::DeviceId(id as u64),
        crate::device_editors::CanvasEvent { kind, x, y, wx, wy, shift, alt, wheel }
    ));
    on!(on_panel_view, |c, id: i32, action: i32, v: f32| c.panel_view(saine_core::DeviceId(id as u64), action, v));
    {
        use saine_core::DeviceId as Id;
        let id = |i: i32| Id(i as u64);
        on!(on_device_fold, |c, d: i32| c.device_fold(id(d)));
        on!(on_device_ask, |c, d: i32| c.device_ask(id(d)));
        on!(on_device_window, |c, d: i32| c.device_window(id(d)));
        on!(on_device_window_moved, |c, d: i32, x: f32, y: f32| c.device_window_moved(id(d), x, y));
        on!(on_device_collapse, |c, d: i32| c.device_collapse(id(d)));
        on!(on_device_swap_menu, |c, d: i32, x: f32, y: f32| c.device_swap_menu(id(d), x, y));
        on!(on_device_rename, |c, d: i32, t: slint::SharedString| c.device_rename(id(d), &t));
        on!(on_mod_menu, |c, d: i32, p: i32, x: f32, y: f32| c.mod_menu(id(d), p, x, y));
        on!(on_mod_map, |c, d: i32, slot: i32| c.mod_map(id(d), slot));
        on!(on_mod_add_menu, |c, d: i32, slot: i32, x: f32, y: f32| c.mod_add_menu(id(d), slot, x, y));
        on!(on_mod_amount_drag, |c, d: i32, i: i32, phase: i32, dy: f32, fine: bool| c.mod_amount_drag(id(d), i.max(0) as usize, phase, dy, fine));
        on!(on_mod_amount_text, |c, d: i32, i: i32, t: slint::SharedString| c.mod_amount_text(id(d), i.max(0) as usize, &t));
        on!(on_mod_amount_step, |c, d: i32, i: i32, delta: i32, fine: bool| c.mod_amount_step(id(d), i.max(0) as usize, delta, fine));
        on!(on_mod_remove, |c, d: i32, i: i32| c.mod_remove(id(d), i.max(0) as usize));
        on!(on_macro_select, |c, d: i32, i: i32| c.macro_select(id(d), i));
        on!(on_macro_rename, |c, d: i32, i: i32, t: slint::SharedString| c.macro_rename(id(d), i, &t));
        on!(on_chain_add_effect, |c, d: i32, x: f32, y: f32| c.chain_add_effect(id(d), x, y));
        on!(on_card_add_menu, |c, d: i32, x: f32, y: f32| c.card_add_menu(id(d), x, y));
        on!(on_card_page, |c, d: i32, dir: i32| c.card_page(id(d), dir));
    }
    on!(on_sampler_pad, |c, id: i32, which: i32, i: i32, down: bool| c.sampler_pad(saine_core::DeviceId(id as u64), which, i, down));
    on!(on_sampler_layer, |c, id: i32, dir: i32| c.sampler_layer(saine_core::DeviceId(id as u64), dir));
    on!(on_sampler_play, |c, id: i32, down: bool| c.sampler_play(saine_core::DeviceId(id as u64), down));
    on!(on_sampler_action, |c, id: i32, a: i32| c.sampler_action(saine_core::DeviceId(id as u64), a));
    on!(on_sampler_remove, |c, id: i32| c.sampler_remove(saine_core::DeviceId(id as u64)));
    on!(on_sampler_well, |c, id: i32, phase: i32, x: f32, tol: f32, alt: bool| c.sampler_well(saine_core::DeviceId(id as u64), phase, x, tol, alt));
    on!(on_sampler_handle, |c, id: i32, i: i32, phase: i32, x: f32, y: f32| c.sampler_handle(saine_core::DeviceId(id as u64), i, phase, x, y));
    {
        // The file dialog runs with the controller NOT borrowed (its modal loop keeps rendering).
        let ctl = ctl.clone();
        studio.on_sampler_load(move |id| {
            let target = ctl.borrow().sampler_load_target(saine_core::DeviceId(id as u64));
            let Some(target) = target else { return };
            if let Some(file) = rfd::FileDialog::new().set_title("Choose audio file").add_filter("Audio", &crate::files::dialog_exts()).pick_file() {
                ctl.borrow_mut().import_files(vec![file], target, None);
            }
        });
    }
    {
        // Files dropped from Finder: onto the selected track's sampler, else imported as clips.
        use slint::winit_030::{winit::event::WindowEvent, EventResult, WinitWindowAccessor};
        let ctl = ctl.clone();
        app.window().on_winit_window_event(move |_, ev| {
            if let WindowEvent::DroppedFile(path) = ev
                && let Ok(mut c) = ctl.try_borrow_mut()
            {
                c.drop_files(vec![path.clone()]);
            }
            EventResult::Propagate
        });
    }
    on!(on_panel_graph, |c, id: i32, phase: i32, x: f32, y: f32, fine: bool, alt: bool| {
        let id = saine_core::DeviceId(id as u64);
        if c.is_canvas_device(id) { c.panel_canvas(id, phase, x, y, alt, fine) } else { c.panel_graph(id, phase, x, y, fine, alt) }
    });
    on!(on_panel_graph_menu, |c, id: i32, x: f32, y: f32, wx: f32, wy: f32| {
        let id = saine_core::DeviceId(id as u64);
        if !c.is_canvas_device(id) {
            c.panel_graph_menu(id, x, y, wx, wy);
        }
    });
    on!(on_panel_eq, |c, id: i32, action: i32, value: i32| c.panel_eq(saine_core::DeviceId(id as u64), action, value));
    {
        // Canvas wheel / keys (EQ, Auto Filter) answer whether they used the event, so they cannot go through on!.
        let (ctl1, aw1) = (ctl.clone(), app.as_weak());
        studio.on_panel_graph_wheel(move |id, x, y, dy| {
            let used = ctl1.borrow_mut().panel_canvas_wheel(saine_core::DeviceId(id as u64), x, y, dy);
            if let Some(a) = aw1.upgrade() {
                if let Ok(mut c) = ctl1.try_borrow_mut() { c.update_ui(&a); }
                a.window().request_redraw();
            }
            used
        });
        let (ctl2, aw2) = (ctl.clone(), app.as_weak());
        studio.on_panel_graph_key(move |id, key, shift, alt| {
            let used = ctl2.borrow_mut().panel_canvas_key(saine_core::DeviceId(id as u64), key.as_str(), shift, alt);
            if let Some(a) = aw2.upgrade() {
                if let Ok(mut c) = ctl2.try_borrow_mut() { c.update_ui(&a); }
                a.window().request_redraw();
            }
            used
        });
    }
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
        let zm = zoom.clone();
        studio.on_key(move |text, primary, shift, alt, ctrl| {
            let Some(key) = key_name(text.as_str()) else { return false };
            let chord = Chord { key, primary, shift, alt, ctrl };
            // View › Zoom In (⌘= / ⌘+), Zoom Out (⌘-), Reset Zoom (⌘0), as in the reference.
            if primary && !alt && !ctrl {
                let step = match chord.key.as_str() {
                    "=" | "+" => Some(1),
                    "-" | "_" => Some(-1),
                    "0" if !shift => Some(0),
                    _ => None,
                };
                if let Some(step) = step {
                    let percent = {
                        let mut z = zm.borrow_mut();
                        if step == 0 { z.reset() } else { z.step(step) }
                        z.percent
                    };
                    if let Some(a) = aw.upgrade() {
                        zm.borrow_mut().sync(a.window());
                        a.window().request_redraw();
                    }
                    if let Ok(mut c) = ctl.try_borrow_mut() {
                        c.set_interface_zoom(percent);
                    }
                    return true;
                }
            }
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
            // The export dialog and the unsaved-changes prompt are modal: Escape closes them (Cancel).
            {
                let mut c = ctl.borrow_mut();
                if c.export.open || c.unsaved_open {
                    if action == "view.escape" && c.export.tracks_menu {
                        c.export_toggle_tracks_menu();
                    } else if action == "view.escape" {
                        c.dialogs_close();
                    }
                    drop(c);
                    if let Some(a) = aw.upgrade()
                        && let Ok(mut c) = ctl.try_borrow_mut()
                    {
                        c.update_ui(&a);
                    }
                    return true;
                }
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
    // After --open, which resets the view.
    if let Some(pct) = args.timeline_zoom {
        let mut c = ctl.borrow_mut();
        c.view.px_per_beat = (timeline::DEFAULT_PX_PER_BEAT * pct / 100.0).clamp(timeline::MIN_PX_PER_BEAT, timeline::MAX_PX_PER_BEAT);
        c.ui_dirty = true;
    }
    // After --open, so the held voice sounds on the project's own Synth.
    if let Some(pitch) = args.hold_note {
        let c = ctl.borrow();
        if let Some(d) = c.synth_device() {
            let mut out = vec![0f32; 2 * 36_000];
            if let Err(e) = c.engine.builtin_note(d.0, pitch, 100).and_then(|_| c.engine.render_offline(&mut out)) {
                eprintln!("--hold-note: {e}");
            }
        }
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
    // Reference z100-modal-export-audio-mixdown / -save-changes-on-new-project: an 8 s sine clip
    // at bar 3 of the Audio track (the export covers bars 1–6), then the dialog.
    let dialog_timer = slint::Timer::default();
    if args.dialog_state > 0 {
        let dir = std::env::temp_dir().join(format!("saine-ref-{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        let path = dir.join("sine-a220-steady.wav");
        let samples: Vec<f32> = (0..8 * 44_100).map(|i| (std::f32::consts::TAU * 220.0 * i as f32 / 44_100.0).sin() * 0.5).collect();
        wavgen::write_wav(&path, 44_100, 1, &samples)?;
        ctl.borrow_mut().import_files(vec![path], ImportTarget::Selected, Some(8 * saine_core::PPQ));
        let ctl = ctl.clone();
        let state = args.dialog_state;
        dialog_timer.start(slint::TimerMode::Repeated, Duration::from_millis(20), move || {
            let Ok(mut c) = ctl.try_borrow_mut() else { return };
            let imported = c.project().tracks.iter().any(|t| !t.clips.is_empty());
            if imported && !c.export.open && !c.unsaved_open {
                if state != 2 {
                    c.export_open();
                    if state >= 3 {
                        c.export_pick_recipe(if state == 5 { 2 } else { 1 });
                    }
                    if state == 4 {
                        c.export_toggle_tracks_menu();
                    }
                } else {
                    c.new_project_request();
                }
            }
        });
    }
    // Reference z100-inspector-audio-clip-tab-warp-off: a 4-bar 220 Hz sine clip at bar 4 on the
    // Audio track, opened in the Clip tab once the import lands; playhead at bar 6.
    let audio_clip_timer = slint::Timer::default();
    if args.audio_clip_state {
        let dir = std::env::temp_dir().join(format!("saine-ref-{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        let path = dir.join("sine-a220-steady.wav");
        let hit = |i: usize| if args.warp_state { (-((i % 22_050) as f32) / 900.0).exp() * (std::f32::consts::TAU * 90.0 * (i % 22_050) as f32 / 44_100.0).sin() * 0.6 } else { 0.0 };
        let samples: Vec<f32> = (0..8 * 44_100).map(|i| (std::f32::consts::TAU * 220.0 * i as f32 / 44_100.0).sin() * if args.warp_state { 0.15 } else { 0.5 } + hit(i)).collect();
        if args.stereo_state {
            let lr: Vec<f32> = samples.iter().enumerate().flat_map(|(i, &s)| [s, s - hit(i)]).collect();
            wavgen::write_wav(&path, 44_100, 2, &lr)?;
        } else {
            wavgen::write_wav(&path, 44_100, 1, &samples)?;
        }
        let mut c = ctl.borrow_mut();
        c.import_files(vec![path], ImportTarget::Selected, Some(12 * saine_core::PPQ));
        c.seek_beats(20.0);
        drop(c);
        let ctl = ctl.clone();
        let warp = args.warp_state;
        let reverse = args.reverse_state;
        let mut menu = args.clip_menu.filter(|_| warp);
        let mut edit = args.ac_edit.filter(|_| warp);
        // Selects the clip once, after the import lands; later tab changes are the user's.
        let done = std::cell::Cell::new(false);
        audio_clip_timer.start(slint::TimerMode::Repeated, Duration::from_millis(20), move || {
            let Ok(mut c) = ctl.try_borrow_mut() else { return };
            let clip = c.project().tracks.iter().flat_map(|t| t.clips.iter()).next().cloned();
            if clip.is_some() && !done.get() {
                c.select_clip(clip.as_ref().map(|cl| cl.id));
                c.set_bottom_tab(BottomTab::Clip);
                done.set(true);
            }
            if let Some(cl) = clip.filter(|cl| warp && !cl.warp.enabled) {
                let mut w = saine_core::ClipWarp { enabled: true, mode: saine_core::WarpMode::Drums, speed: 1.5, original_bpm: 120.0, reverse, transpose: 3, ..Default::default() };
                w.set_marker(0, 0.0);
                w.set_marker(4 * 44_100, 9.0);
                c.execute(saine_core::Command::SetClipWarp { clip: cl.id, warp: w, label: "Set clip speed".into() });
            }
            // review-warp + --clip-menu: open the dropdown once the warped clip is shown.
            if c.ac_clip().is_some_and(|cl| cl.warp.enabled)
                && let Some(m) = menu.take()
            {
                c.clip_menu_open(m);
            }
            if c.ac_clip().is_some_and(|cl| cl.warp.enabled)
                && let Some(f) = edit.take()
            {
                c.ac_edit_start(if f == 0 { warpedit::AcEdit::OriginalBpm } else { warpedit::AcEdit::Transpose });
            }
        });
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
    let want_menu = args.clip_menu;
    let save_to = args.save_to.clone();
    {
        let aw = app.as_weak();
        let ctl = ctl.clone();
        let fr = frame.clone();
        let zm = zoom.clone();
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
                    // Interface zoom: re-apply when the window is new or changed display.
                    if zm.borrow().needs_sync(app.window().scale_factor()) {
                        zm.borrow_mut().pending = true;
                        let (aw, zm) = (app.as_weak(), zm.clone());
                        slint::Timer::single_shot(Duration::ZERO, move || {
                            if let Some(a) = aw.upgrade() {
                                zm.borrow_mut().sync(a.window());
                                a.window().request_redraw();
                            }
                        });
                    }
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
                    if c.pending_jobs == 0 && CLIP_MENU_AUDIO.load(std::sync::atomic::Ordering::Relaxed) {
                        let found = c.project().tracks.iter().find_map(|t| t.clips.first().map(|k| (k.id, k.start, c.project().clip_length_ticks(k))));
                        if let Some((id, start, len)) = found {
                            CLIP_MENU_AUDIO.store(false, std::sync::atomic::Ordering::Relaxed);
                            c.seek_beats((start + len / 2) as f64 / saine_core::PPQ as f64);
                            c.open_clip_menu(id, 700.0, 215.0, (10_000.0, 10_000.0));
                        }
                    }
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
                    // Clip editor texture (only while a MIDI or audio clip is open in the Clip tab).
                    let pr_vis = c.clip_editor_visible();
                    if pr_vis {
                        // The step sequencer draws in Slint; the texture keeps its size for the piano roll tab.
                        let win = app.window().size();
                        let size = ((win.width as f32 / scale) - 10.0 - pianoroll::PANEL_ORIGIN.0 - 0.5, pianoroll::CLIP_PANEL_H - pianoroll::PANEL_ORIGIN.1 - pianoroll::FOOTER_H);
                        if c.pr.size != size {
                            c.pr.size = size;
                            c.ui_dirty = true;
                        }
                        let notes = c.pr_notes();
                        if let (false, Some(r), Some(sc)) = (c.seq_visible(), f.pr_renderer.as_mut(), c.pr_scene(scale, &notes)) {
                            let phys = ((size.0 * scale).round() as u32, (size.1 * scale).round() as u32);
                            if let Some(tex) = r.ensure_target(phys) {
                                match gpu::to_slint_image(tex) {
                                    Ok(img) => app.global::<Studio>().set_pr_image(img),
                                    Err(e) => eprintln!("piano roll texture import failed: {e}"),
                                }
                            }
                            r.sync_peaks(c.peaks.generation, &c.peaks.data);
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
                        Ok(c) => (c.pending_jobs > 0 || c.engine.transport().playing || c.files.audition.animating || !c.assistant.highlights.is_empty() || c.rec.recording, c.pending_jobs == 0 && want_menu.is_none_or(|m| c.pr.menu == m)),
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
        let zm = zoom.clone();
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
            // No frame ran, so the zoom was never applied: apply it before the snapshot.
            zm.borrow_mut().sync(a.window());
            for _ in 0..3 {
                let _ = a.window().take_snapshot();
            }
            save_snapshot(&a, &path);
            let _ = slint::quit_event_loop();
        });
    }
    if args.render_bench > 0 {
        let aw = app.as_weak();
        let zm = zoom.clone();
        let n = args.render_bench as usize;
        let drag = args.drag_bench.clone();
        let dc = ctl.clone();
        slint::Timer::single_shot(Duration::from_secs(4), move || {
            let Some(a) = aw.upgrade() else { return };
            zm.borrow_mut().sync(a.window());
            let mut ms = Vec::with_capacity(n);
            let track = dc.borrow().project().tracks[0].id.0 as i32;
            match drag.as_deref() {
                Some("track") => dc.borrow_mut().track_resize_drag(track, 0, 200.0),
                Some("bottom") => dc.borrow_mut().bottom_resize(0, 500.0),
                _ => {}
            }
            for i in 0..n + 5 {
                let t = Instant::now();
                let dy = (i % 40) as f32 * 2.0;
                match drag.as_deref() {
                    Some("track") => dc.borrow_mut().track_resize_drag(track, 1, 200.0 + dy),
                    Some("bottom") => dc.borrow_mut().bottom_resize(1, 500.0 - dy),
                    _ => {}
                }
                if drag.is_some() {
                    dc.borrow_mut().update_ui(&a);
                }
                if let Err(e) = a.window().take_snapshot() {
                    eprintln!("--render-bench: snapshot failed: {e}");
                    std::process::exit(1);
                }
                if i >= 5 {
                    ms.push(t.elapsed().as_secs_f64() * 1e3);
                }
            }
            ms.sort_by(f64::total_cmp);
            let mean = ms.iter().sum::<f64>() / n as f64;
            let median = if n % 2 == 0 { (ms[n / 2 - 1] + ms[n / 2]) / 2.0 } else { ms[n / 2] };
            println!("{{\"renders\":{n},\"mean_ms\":{mean:.3},\"median_ms\":{median:.3},\"p95_ms\":{:.3}}}", ms[(n * 95 / 100).min(n - 1)]);
            let _ = slint::quit_event_loop();
        });
    }
    // The OS window is 1440×820 pt like the reference's; interface zoom then sets how much
    // of our (85 %-unit) layout fits in it, see `zoom.rs`.
    app.window().set_size(slint::LogicalSize::new(args.window_width.unwrap_or(1440.0), 820.0));
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
