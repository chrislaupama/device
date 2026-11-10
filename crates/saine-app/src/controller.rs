//! Application controller: owns the `Document`, talks to the engine, and
//! translates UI gestures into `saine_core::Command`s.
//!
//! The UI never holds authoritative state. After every change we rebuild the
//! engine snapshot (`snapshot::build` + `engine.publish`) and push fresh view
//! models into the `Studio` Slint global. Transient gesture state (a clip being
//! dragged) lives here only until the gesture ends and becomes one Command.

use crate::peaks::{L0_FRAMES, PeakStore, Pyramid};
use crate::timeline::{self, ClipVis, Hit, TrackVis, View, Zone};
use crate::assistant_panel::AssistantState;
use crate::assistant_provider::Provider;
use crate::mixer::{self, Ballistics, MASTER_METER_ID};
use crate::pianoroll::{self, PrState};
use crate::synth::SynthUi;
use crate::{AppWindow, AutoExtra, AutoLabel, ChatEntry, ChoiceItem, ClipLabel, DeviceCard, HighlightRect, MenuItem, MixerStrip, PluginRow, RulerLabel, PatternRow, SeqCell, SeqNum, SeqRow, ShapeRow, StripMeter, Studio, SynthDisplay, SynthParam, TrackLevel, TrackRow, TreeRow, YLabel};
use saine_core::{
    Asset, AssetId, Clip, ClipId, ColorIndex, Command, Document, MidiClip, Origin, PPQ, Project, Ticks, Track, TrackId, TrackKind, db_to_gain, gain_to_db, persist, snapshot,
};
use saine_engine::{AssetInfo, Engine, Meter, MetronomeConfig};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

// ------------------------------------------------------------------ value mappings

/// Track/header volume slider position (0..1) for a linear gain. 0 dB sits at
/// 0.85 of the travel (measured handle position in the reference), +6 dB at 1.
/// Track-header and header volume sliders: (dB, fraction of travel), piecewise linear between
/// the points, as in the reference (reference/research-findings.md, "Track headers").
const SLIDER_SCALE: [(f32, f32); 6] = [(-60.0, 0.0), (-48.0, 0.045), (-36.0, 0.11), (-24.0, 0.22), (-12.0, 0.4), (6.0, 1.0)];
pub fn slider_from_gain(g: f32) -> f32 {
    if g <= 0.0 {
        return 0.0;
    }
    let db = gain_to_db(g);
    if db >= 6.0 {
        return 1.0;
    }
    if db <= -60.0 {
        return 0.0;
    }
    let k = SLIDER_SCALE.iter().position(|&(d, _)| db <= d).unwrap_or(SLIDER_SCALE.len() - 1).max(1);
    let ((d0, p0), (d1, p1)) = (SLIDER_SCALE[k - 1], SLIDER_SCALE[k]);
    p0 + (p1 - p0) * (db - d0) / (d1 - d0)
}
pub fn gain_from_slider(p: f32) -> f32 {
    let p = p.clamp(0.0, 1.0);
    if p <= 0.0 {
        return 0.0;
    }
    let k = SLIDER_SCALE.iter().position(|&(_, q)| p <= q).unwrap_or(SLIDER_SCALE.len() - 1).max(1);
    let ((d0, p0), (d1, p1)) = (SLIDER_SCALE[k - 1], SLIDER_SCALE[k]);
    db_to_gain(d0 + (d1 - d0) * (p - p0) / (p1 - p0))
}

/// Track-header meter length: linear in dB from -50 dB (empty) to 0 dB (full).
pub fn header_meter(g: f32) -> f32 {
    if g <= 0.0 { 0.0 } else { ((gain_to_db(g) + 50.0) / 50.0).clamp(0.0, 1.0) }
}

/// Fader/meter taper of the mixer strips and the track inspector: (dB, fraction of travel from
/// the top). Piecewise linear in dB between the reference's breakpoints (reference/
/// research-findings.md, "Channel strip fader"): +6 dB at the top, 0 dB at 20 %, -12 dB at
/// 60 %, then -24 / -36 / -48 dB at 78 / 89 / 95.5 %, and -60 dB (shown as −∞) at the bottom.
const FADER_SCALE: [(f32, f32); 6] = [(6.0, 0.0), (-12.0, 0.6), (-24.0, 0.78), (-36.0, 0.89), (-48.0, 0.955), (-60.0, 1.0)];
pub fn fader_from_db(db: f32) -> f32 {
    if !db.is_finite() || db <= -60.0 {
        return 1.0;
    }
    if db >= 6.0 {
        return 0.0;
    }
    for w in FADER_SCALE.windows(2) {
        let ((d0, f0), (d1, f1)) = (w[0], w[1]);
        if db <= d0 && db >= d1 {
            return f0 + (f1 - f0) * (d0 - db) / (d0 - d1);
        }
    }
    1.0
}
pub fn db_from_fader(f: f32) -> f32 {
    let f = f.clamp(0.0, 1.0);
    if f >= 0.999 {
        return f32::NEG_INFINITY;
    }
    for w in FADER_SCALE.windows(2) {
        let ((d0, f0), (d1, f1)) = (w[0], w[1]);
        if f >= f0 && f <= f1 {
            return d0 + (d1 - d0) * (f - f0) / (f1 - f0);
        }
    }
    f32::NEG_INFINITY
}

pub fn fmt_db(db: f32) -> String {
    if !db.is_finite() || db < -99.0 { "−∞".into() } else { format!("{db:.2}").replace('-', "−") }
}

/// Track-header pan entry: "C", "<n>L", "<n>R" (n 0..100) or a number -100..100.
pub fn parse_pan_text(s: &str) -> Option<f32> {
    let t = s.trim();
    if t.eq_ignore_ascii_case("c") {
        return Some(0.0);
    }
    let lower = t.to_ascii_lowercase();
    for (suffix, sign) in [("l", -1.0f32), ("r", 1.0)] {
        if let Some(n) = lower.strip_suffix(suffix).map(str::trim_end).filter(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())) {
            return n.parse::<f32>().ok().map(|v| sign * v.min(100.0) / 100.0);
        }
    }
    t.parse::<f32>().ok().filter(|v| v.is_finite()).map(|v| (v / 100.0).clamp(-1.0, 1.0))
}

pub(crate) fn fmt_pan(p: f32) -> String {
    let v = (p * 100.0).round() as i32;
    match v {
        0 => "C".into(),
        v if v < 0 => format!("{}L", -v),
        v => format!("{v}R"),
    }
}

/// The reference's 13 track colours as shown on screen: its palette after the track tint
/// (saturation ×1.64 about Rec. 709 luma, then 10 % toward white) in display-P3 values
/// (reference/research-findings.md, "Track colours"). Rotated so that index 0 and 1 are the
/// blue and purple of older projects; the order is the reference's cycle.
const TRACK_PALETTE: [u32; 13] =
    [0x72b5d7, 0x8092eb, 0x9b88f6, 0xc770f7, 0xec5c9f, 0xde7e36, 0x56b5ab, 0xec6e45, 0xe3aa4b, 0xc3cc47, 0xa9de4d, 0x78ea8e, 0x67c9c0];

pub fn track_color(c: ColorIndex) -> [f32; 3] {
    timeline::rgb(TRACK_PALETTE[c.0 as usize % TRACK_PALETTE.len()])
}

/// Colour for a new track: the first palette entry after the last track's colour that no track
/// uses yet, else the least-used one (the reference's rule). The first track is pink, as in the
/// reference's new projects.
pub fn next_track_color(p: &Project) -> ColorIndex {
    let n = TRACK_PALETTE.len();
    let Some(last) = p.tracks.last() else { return ColorIndex::FIRST };
    let start = last.color.0 as usize % n;
    let used = |c: usize| p.tracks.iter().filter(|t| t.color.0 as usize % n == c).count();
    (1..=n).map(|k| (start + k) % n).find(|&c| used(c) == 0).map_or_else(|| ColorIndex((0..n).min_by_key(|&c| used(c)).unwrap_or(0) as u8), |c| ColorIndex(c as u8))
}

/// Card accent for reference renders (`--card-accent RRGGBB`): the reference captures of the
/// effect cards were taken on the Master track, whose colour is white.
pub static CARD_ACCENT: std::sync::OnceLock<[f32; 3]> = std::sync::OnceLock::new();

/// Accent of a device card on a track of colour `c`.
pub fn card_color(c: ColorIndex) -> slint::Color {
    slint_color(CARD_ACCENT.get().copied().unwrap_or_else(|| track_color(c)))
}

/// Record-armed / recording red (ORIGINAL choice; the reference's armed state was not
/// captured, see ui-inventory.md §11). Mirrors `Tokens.record-red` (#e5484d).
pub const RECORD_RED: [f32; 3] = [0xe5 as f32 / 255.0, 0x48 as f32 / 255.0, 0x4d as f32 / 255.0];

pub(crate) fn slint_color(c: [f32; 3]) -> slint::Color {
    slint::Color::from_rgb_f32(c[0], c[1], c[2])
}

// ------------------------------------------------------------------ background jobs

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ImportTarget {
    /// Selected track if it is an audio track, else the first audio track, else a new one.
    Selected,
    NewTrack,
    /// A track resolved from `Selected` when the import was requested (an audio
    /// track, or a new one if it has since been removed). Imports finish on worker
    /// threads in any order, so `Selected` must not be resolved at completion: an
    /// earlier import's new track would be selected by then.
    Track(TrackId),
    /// A built-in sampler's sample slot (`next_free`: the first free layer
    /// of the pad that starts at `slot`).
    DeviceSample { device: saine_core::DeviceId, slot: u32, next_free: bool },
}

pub enum JobResult {
    /// A file import finished. `doc_epoch` is the project it was started in: a
    /// result from a project that has since been closed is dropped.
    Imported { path: PathBuf, target: ImportTarget, at: Ticks, doc_epoch: u64, result: Result<(AssetInfo, Pyramid), String> },
    /// Media of an opened project reloaded into the engine.
    Reloaded { asset: AssetId, doc_epoch: u64, result: Result<(AssetInfo, Pyramid), String> },
    /// Assistant provider reply (raw `AssistantTurn` JSON) for a request started at `started`.
    Assistant { started: Instant, mode: saine_core::assistant::Mode, result: Result<String, String> },
    /// Takes of one finished recording pass, loaded into the engine.
    Recorded { doc_epoch: u64, takes: Vec<(saine_engine::RecordedTake, Result<(AssetInfo, Pyramid), String>)> },
    /// Settings › Audio "Test" tone loaded into the engine.
    TestTone(Result<AssetInfo, String>),
    /// Plugin catalogue scan finished (src/devices.rs).
    PluginScan(Result<saine_engine::ScanReport, String>),
    /// Transients of an engine asset (warped clips, src/warpedit.rs).
    Onsets { engine_asset: u64, result: Result<Vec<saine_engine::Onset>, String>, tempo: Option<f64> },
    /// A file clicked in Browser › Files, decoded for the preview voice (src/files.rs).
    Auditioned { path: PathBuf, result: Result<AssetInfo, String> },
    /// A Consolidate, Bounce or Export render written and loaded (src/clip_render.rs).
    ClipRendered(Box<crate::clip_render::ClipRendered>),
}

/// Blocking decode + peak analysis. Runs on a worker thread.
pub fn load_and_analyse(engine: &Engine, path: &Path) -> Result<(AssetInfo, Pyramid), String> {
    let info = engine.load_file(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let pairs = (info.frames / L0_FRAMES as u64 + 1) as usize;
    let l0 = engine.peaks(info.asset_id, 0, L0_FRAMES, pairs).map_err(|e| e.to_string())?;
    Ok((info, Pyramid::from_level0(l0)))
}

// ------------------------------------------------------------------ gestures

#[derive(Debug, Clone)]
enum Drag {
    Scrub,
    Move { clip: ClipId, grab_beats: f64, orig_track: TrackId, orig_start: Ticks, track: TrackId, start: Ticks },
    TrimLeft { clip: ClipId, orig: (Ticks, u64, u64), cur: (Ticks, u64, u64) },
    TrimRight { clip: ClipId, orig: (Ticks, u64, u64), cur: (Ticks, u64, u64) },
    /// MIDI clip edges: (start, length) in ticks.
    MidiLeft { clip: ClipId, orig: (Ticks, Ticks), cur: (Ticks, Ticks) },
    MidiRight { clip: ClipId, orig: (Ticks, Ticks), cur: (Ticks, Ticks) },
}

/// Bottom panel tab (reference: "Clip" / "Devices").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BottomTab {
    Devices,
    Clip,
}

/// Internal clipboard (⌘C / ⌘X / ⌘V): one clip with its track kind.
#[derive(Debug, Clone)]
pub enum ClipboardItem {
    Audio { track: TrackId, clip: Clip },
    Midi { track: TrackId, clip: MidiClip },
}

/// Clip context menu entries (reference `35-clip-context-menu.png`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuAction {
    Cut,
    Copy,
    Duplicate,
    Split,
    Consolidate,
    Bounce,
    Delete,
    Disable,
    Enable,
    ShowInFinder,
    Rename,
    Export,
    Transpose,
    Quantize,
    Humanize,
    /// Saine extension: plain MIDI clip to a step pattern, and back.
    ToPattern,
    ToMidi,
    Ask,
    Paste,
    SectionRename,
    SectionColor,
    SectionInsert,
    SectionDelete,
    SectionClear,
}

#[derive(Debug, Default)]
pub struct MenuState {
    pub open: bool,
    pub pos: (f32, f32),
    pub clip: Option<ClipId>,
    pub section: Option<saine_core::SectionId>,
    pub items: Vec<(MenuAction, bool)>,
    pub sub_open: bool,
    /// The entry whose submenu `sub_open` shows (Transpose or Bounce).
    pub sub_for: Option<MenuAction>,
}

// ------------------------------------------------------------------ controller

pub struct Controller {
    pub doc: Document,
    pub engine: Arc<Engine>,
    pub rate: f64,
    pub(crate) asset_map: HashMap<AssetId, u64>,
    /// The engine's reversed copies of assets that reversed clips play (src/warpedit.rs).
    pub(crate) reversed_assets: HashMap<AssetId, u64>,
    /// Detected transients per engine asset (asset frames), and those being analysed.
    pub(crate) onsets: HashMap<u64, Vec<saine_core::Transient>>,
    pub(crate) onsets_pending: std::collections::HashSet<u64>,
    /// Tempo detected from each analysed engine asset's transients (`warp::estimate_bpm`).
    pub(crate) tempos: HashMap<u64, Option<f64>>,
    pub peaks: PeakStore,
    pub selected_track: Option<TrackId>,
    pub selected_clip: Option<ClipId>,
    pub view: View,
    pub lanes_size: (f32, f32),
    drag: Option<Drag>,
    /// Track-height drag preview (src/splitters.rs).
    pub(crate) track_resize: Option<crate::splitters::TrackResize>,
    /// Timeline / bottom-panel splitter drag (src/splitters.rs).
    pub(crate) bottom_drag: Option<crate::splitters::BottomDrag>,
    /// Docked-assistant width drag (src/splitters.rs).
    pub(crate) chat_drag: Option<crate::splitters::ChatDrag>,
    status: Option<(String, Instant, Duration)>,
    pub project_dir: Option<PathBuf>,
    /// Export dialog and the unsaved-changes prompt (src/dialogs.rs).
    pub export: crate::dialogs::ExportUi,
    pub clip_render: crate::clip_render::ClipRender,
    pub unsaved_open: bool,
    pub(crate) jobs_tx: Sender<JobResult>,
    jobs_rx: Receiver<JobResult>,
    pub pending_jobs: usize,
    pub(crate) doc_epoch: u64,
    published_rev: Option<u64>,
    ui_rev: Option<u64>,
    pub ui_dirty: bool,
    meters: Vec<Meter>,
    sel_peak_hold: f32,
    /// Displayed meter level: engine `peek_meters` (non-destructive ~50 ms
    /// window) with a visual release of 24 dB/s on top.
    meter_disp: (f32, f32),
    meter_t: Option<Instant>,
    // models (kept to update in place)
    pub(crate) tracks_model: Rc<VecModel<TrackRow>>,
    ruler_model: Rc<VecModel<RulerLabel>>,
    section_label_model: Rc<VecModel<crate::SectionLabel>>,
    clips_model: Rc<VecModel<ClipLabel>>,
    ruler_key: (i64, i64, i64, i64),
    pub(crate) mm: crate::minimap::State,
    /// Timeline snap toggle, grid size and menu (src/snap.rs).
    pub(crate) snap: crate::snap::State,
    /// Devices minimap (src/devminimap.rs) and the pointer hide / warp requests of minimap drags (src/pointer.rs).
    pub(crate) dm: crate::devminimap::State,
    pub(crate) ptr: crate::pointer::Queue,
    pub clip_vis: Vec<ClipVis>,
    pub track_vis: Vec<TrackVis>,
    pub(crate) clip_ids: Vec<ClipId>,
    /// Show status messages (disabled for pixel-comparison screenshots).
    pub show_status: bool,
    // ---- clip editing ----
    pub bottom_tab: BottomTab,
    pub pr: PrState,
    pub menu: MenuState,
    pub clipboard: Option<ClipboardItem>,
    /// Rename popup open for this clip.
    pub rename: Option<ClipId>,
    pub rename_section: Option<saine_core::SectionId>,
    pub selected_section: Option<saine_core::SectionId>,
    pub(crate) section_drag: Option<crate::sections::SectionDrag>,
    pub(crate) section_vis: Vec<crate::sections::SectionVis>,
    pub(crate) pr_bar_model: Rc<VecModel<RulerLabel>>,
    pub(crate) pr_key_model: Rc<VecModel<YLabel>>,
    pub(crate) seq_row_model: Rc<VecModel<SeqRow>>,
    pub(crate) seq_cell_model: Rc<VecModel<SeqCell>>,
    pub(crate) seq_num_model: Rc<VecModel<SeqNum>>,
    /// Row tops of the cells the playing step hits (the step sequencer's flash).
    pub(crate) seq_flash_model: Rc<VecModel<f32>>,
    pub(crate) composer_row_model: Rc<VecModel<PatternRow>>,
    /// The rows last pushed to `composer_row_model` (rebuilt only when they change).
    pub(crate) composer_rows_shown: std::cell::RefCell<Vec<crate::composer::RowView>>,
    pub(crate) composer_shape_model: Rc<VecModel<ShapeRow>>,
    pub(crate) composer_shape_shown: std::cell::RefCell<Vec<crate::composer::ShapeRowView>>,
    /// The clip sidebar tab picked (true: Shape); None until one is picked.
    pub(crate) composer_shape_tab: Option<bool>,
    pub(crate) menu_model: Rc<VecModel<MenuItem>>,
    pub(crate) sub_model: Rc<VecModel<MenuItem>>,
    /// Metronome on/off (app preference, not saved in the project).
    pub metronome_on: bool,
    // ---- panels (view state owned here, mirrored to Slint) ----
    pub mixer_open: bool,
    pub assistant_open: bool,
    pub assistant: AssistantState,
    pub provider: Arc<Provider>,
    ballistics: Ballistics,
    strips_model: Rc<VecModel<MixerStrip>>,
    strip_meters_model: Rc<VecModel<StripMeter>>,
    track_levels_model: Rc<VecModel<TrackLevel>>,
    chat_model: Rc<VecModel<ChatEntry>>,
    chat_rev: Option<u64>,
    highlights_model: Rc<VecModel<HighlightRect>>,
    // ---- synth device (src/synth.rs) ----
    pub synth: SynthUi,
    pub(crate) synth_params_model: Rc<VecModel<SynthParam>>,
    pub(crate) synth_displays_model: Rc<VecModel<SynthDisplay>>,
    pub(crate) synth_menu_model: Rc<VecModel<MenuItem>>,
    // ---- recording (src/recording.rs) + settings (src/settings.rs) ----
    pub rec: crate::recording::RecState,
    pub settings: crate::settings::SettingsState,
    pub(crate) input_menu_model: Rc<VecModel<ChoiceItem>>,
    pub(crate) export_tracks_model: Rc<VecModel<ChoiceItem>>,
    pub(crate) settings_menu_model: Rc<VecModel<ChoiceItem>>,
    // ---- plugin devices (src/devices.rs) ----
    pub devices: crate::devices::DevicesUi,
    pub(crate) tree_model: Rc<VecModel<TreeRow>>,
    // ---- browser Files section and Back / Forward (src/files.rs) ----
    pub files: crate::files::FilesUi,
    pub(crate) files_model: Rc<VecModel<crate::FileRow>>,
    pub(crate) files_guides_model: Rc<VecModel<crate::FileGuide>>,
    pub(crate) cards_model: Rc<VecModel<DeviceCard>>,
    pub(crate) window_cards_model: Rc<VecModel<DeviceCard>>,
    pub(crate) add_results_model: Rc<VecModel<MenuItem>>,
    pub(crate) devmenu_model: Rc<VecModel<MenuItem>>,
    pub(crate) scan_dirs_model: Rc<VecModel<PluginRow>>,
    pub(crate) plugin_rows_model: Rc<VecModel<PluginRow>>,
    pub(crate) plugin_failed_model: Rc<VecModel<PluginRow>>,
    // ---- automation (src/automation.rs) ----
    pub auto: crate::automation::AutoUi,
    pub(crate) auto_labels_model: Rc<VecModel<AutoLabel>>,
    pub(crate) auto_extras_model: Rc<VecModel<AutoExtra>>,
    pub(crate) picker_model: Rc<VecModel<MenuItem>>,
    pub(crate) picker_sub_model: Rc<VecModel<MenuItem>>,
}

impl Controller {
    pub fn new(engine: Arc<Engine>, project: Project) -> Self {
        let (jobs_tx, jobs_rx) = channel();
        let rate = engine.sample_rate();
        let mut c = Controller {
            doc: Document::new(project),
            engine,
            rate,
            asset_map: HashMap::new(),
            reversed_assets: HashMap::new(),
            onsets: HashMap::new(),
            onsets_pending: std::collections::HashSet::new(),
            tempos: HashMap::new(),
            peaks: PeakStore::default(),
            selected_track: None,
            selected_clip: None,
            view: View::default(),
            lanes_size: (983.0, 398.5),
            drag: None,
            track_resize: None,
            bottom_drag: None,
            chat_drag: None,
            status: None,
            project_dir: None,
            export: Default::default(),
            clip_render: Default::default(),
            unsaved_open: false,
            jobs_tx,
            jobs_rx,
            pending_jobs: 0,
            doc_epoch: 0,
            published_rev: None,
            ui_rev: None,
            ui_dirty: true,
            meters: Vec::new(),
            sel_peak_hold: 0.0,
            meter_disp: (0.0, 0.0),
            meter_t: None,
            tracks_model: Rc::new(VecModel::default()),
            ruler_model: Rc::new(VecModel::default()),
            mm: crate::minimap::State::new(),
            snap: crate::snap::State::new(),
            dm: crate::devminimap::State::new(),
            ptr: crate::pointer::Queue::default(),
            section_label_model: Rc::new(VecModel::default()),
            clips_model: Rc::new(VecModel::default()),
            ruler_key: (i64::MIN, 0, 0, 0),
            clip_vis: Vec::new(),
            track_vis: Vec::new(),
            clip_ids: Vec::new(),
            show_status: true,
            bottom_tab: BottomTab::Devices,
            pr: PrState::default(),
            menu: MenuState::default(),
            clipboard: None,
            rename: None,
            rename_section: None,
            selected_section: None,
            section_drag: None,
            section_vis: Vec::new(),
            pr_bar_model: Rc::new(VecModel::default()),
            pr_key_model: Rc::new(VecModel::default()),
            seq_row_model: Rc::new(VecModel::default()),
            seq_cell_model: Rc::new(VecModel::default()),
            seq_num_model: Rc::new(VecModel::default()),
            seq_flash_model: Rc::new(VecModel::default()),
            composer_row_model: Rc::new(VecModel::default()),
            composer_rows_shown: Default::default(),
            composer_shape_model: Rc::new(VecModel::default()),
            composer_shape_shown: Default::default(),
            composer_shape_tab: None,
            menu_model: Rc::new(VecModel::default()),
            sub_model: Rc::new(VecModel::default()),
            metronome_on: false,
            mixer_open: false,
            assistant_open: false,
            assistant: AssistantState::default(),
            provider: Arc::new(Provider::Unavailable),
            ballistics: Ballistics::default(),
            strips_model: Rc::new(VecModel::default()),
            strip_meters_model: Rc::new(VecModel::default()),
            track_levels_model: Rc::new(VecModel::default()),
            chat_model: Rc::new(VecModel::default()),
            chat_rev: None,
            highlights_model: Rc::new(VecModel::default()),
            synth: SynthUi::default(),
            synth_params_model: Rc::new(VecModel::default()),
            synth_displays_model: Rc::new(VecModel::default()),
            synth_menu_model: Rc::new(VecModel::default()),
            rec: Default::default(),
            settings: Default::default(),
            input_menu_model: Rc::new(VecModel::default()),
            export_tracks_model: Rc::new(VecModel::default()),
            settings_menu_model: Rc::new(VecModel::default()),
            devices: Default::default(),
            tree_model: Rc::new(VecModel::default()),
            files: Default::default(),
            files_model: Rc::new(VecModel::default()),
            files_guides_model: Rc::new(VecModel::default()),
            cards_model: Rc::new(VecModel::default()),
            window_cards_model: Rc::new(VecModel::default()),
            add_results_model: Rc::new(VecModel::default()),
            devmenu_model: Rc::new(VecModel::default()),
            scan_dirs_model: Rc::new(VecModel::default()),
            plugin_rows_model: Rc::new(VecModel::default()),
            plugin_failed_model: Rc::new(VecModel::default()),
            auto: Default::default(),
            auto_labels_model: Rc::new(VecModel::default()),
            auto_extras_model: Rc::new(VecModel::default()),
            picker_model: Rc::new(VecModel::default()),
            picker_sub_model: Rc::new(VecModel::default()),
        };
        c.selected_track = c.doc.project().tracks.first().map(|t| t.id);
        c.publish();
        c
    }

    pub fn bind_models(&self, app: &AppWindow) {
        let s = app.global::<Studio>();
        s.set_tracks(ModelRc::from(self.tracks_model.clone()));
        s.set_ruler_labels(ModelRc::from(self.ruler_model.clone()));
        s.set_minimap_bars(ModelRc::from(self.mm.model.clone()));
        s.set_devmm_tiles(ModelRc::from(self.dm.model.clone()));
        s.set_snap_menu_items(ModelRc::from(self.snap.model.clone()));
        s.set_section_labels(ModelRc::from(self.section_label_model.clone()));
        s.set_clip_labels(ModelRc::from(self.clip_ids_model()));
        s.set_strips(ModelRc::from(self.strips_model.clone()));
        s.set_strip_meters(ModelRc::from(self.strip_meters_model.clone()));
        s.set_track_levels(ModelRc::from(self.track_levels_model.clone()));
        s.set_chat(ModelRc::from(self.chat_model.clone()));
        s.set_highlights(ModelRc::from(self.highlights_model.clone()));
        s.set_pr_bar_labels(ModelRc::from(self.pr_bar_model.clone()));
        s.set_pr_key_labels(ModelRc::from(self.pr_key_model.clone()));
        s.set_seq_rows(ModelRc::from(self.seq_row_model.clone()));
        s.set_seq_cells(ModelRc::from(self.seq_cell_model.clone()));
        s.set_seq_nums(ModelRc::from(self.seq_num_model.clone()));
        s.set_seq_flash_rows(ModelRc::from(self.seq_flash_model.clone()));
        let tips: Vec<SharedString> = std::iter::once("Turn steps on and off").chain(saine_core::steps::StepLane::ALL.iter().map(|l| l.spec().tip)).map(Into::into).collect();
        s.set_seq_lane_tips(ModelRc::new(VecModel::from(tips)));
        s.set_composer_rows(ModelRc::from(self.composer_row_model.clone()));
        s.set_composer_shape_rows(ModelRc::from(self.composer_shape_model.clone()));
        s.set_menu_items(ModelRc::from(self.menu_model.clone()));
        s.set_sub_items(ModelRc::from(self.sub_model.clone()));
        s.set_synth_params(ModelRc::from(self.synth_params_model.clone()));
        s.set_synth_displays(ModelRc::from(self.synth_displays_model.clone()));
        s.set_synth_menu_items(ModelRc::from(self.synth_menu_model.clone()));
        s.set_input_menu_items(ModelRc::from(self.input_menu_model.clone()));
        s.set_export_tracks_items(ModelRc::from(self.export_tracks_model.clone()));
        s.set_settings_menu_items(ModelRc::from(self.settings_menu_model.clone()));
        s.set_tree(ModelRc::from(self.tree_model.clone()));
        s.set_files(ModelRc::from(self.files_model.clone()));
        s.set_files_guides(ModelRc::from(self.files_guides_model.clone()));
        s.set_device_cards(ModelRc::from(self.cards_model.clone()));
        s.set_device_window_cards(ModelRc::from(self.window_cards_model.clone()));
        s.set_add_results(ModelRc::from(self.add_results_model.clone()));
        s.set_devmenu_items(ModelRc::from(self.devmenu_model.clone()));
        s.set_scan_dirs(ModelRc::from(self.scan_dirs_model.clone()));
        s.set_plugin_rows(ModelRc::from(self.plugin_rows_model.clone()));
        s.set_plugin_failed(ModelRc::from(self.plugin_failed_model.clone()));
        s.set_auto_labels(ModelRc::from(self.auto_labels_model.clone()));
        s.set_auto_extras(ModelRc::from(self.auto_extras_model.clone()));
        s.set_picker_items(ModelRc::from(self.picker_model.clone()));
        s.set_picker_sub_items(ModelRc::from(self.picker_sub_model.clone()));
        let pages: Vec<SharedString> = crate::settings::PAGES.iter().map(|p| SharedString::from(*p)).collect();
        s.set_settings_pages(ModelRc::new(VecModel::from(pages)));
    }

    fn clip_ids_model(&self) -> Rc<VecModel<ClipLabel>> {
        self.clips_model.clone()
    }

    pub fn project(&self) -> &Project {
        self.doc.project()
    }

    pub fn status_text(&self) -> Option<&str> {
        self.status.as_ref().map(|s| s.0.as_str())
    }

    /// The message of the status line (self-test).
    pub(crate) fn status_message(&self) -> Option<&str> {
        self.status.as_ref().map(|s| s.0.as_str())
    }

    pub fn set_status(&mut self, msg: impl Into<String>, secs: u64) {
        let msg = msg.into();
        eprintln!("[status] {msg}");
        self.status = Some((msg, Instant::now(), Duration::from_secs(secs)));
        self.ui_dirty = true;
    }

    /// App menu bar items without a shortcut (`Studio.menu-action`). Settings and the
    /// dialogs are modal, so the menu does nothing while one is open.
    pub fn menu_action(&mut self, id: &str) {
        if self.settings.open || self.export.open || self.unsaved_open {
            return;
        }
        match id {
            "options.audio_midi" => self.open_settings(),
            "options.check_plugins" => self.start_plugin_scan(false),
            "options.rescan_plugins" => self.start_plugin_scan(true),
            "file.show_in_finder" => {
                let Some(dir) = &self.project_dir else { return };
                if let Err(e) = std::process::Command::new("open").arg("-R").arg(dir).spawn() {
                    self.set_status(format!("Show in Finder failed: {e}"), 3);
                }
            }
            other => self.set_status(format!("{other}: not implemented yet"), 3),
        }
    }

    // ---------------------------------------------------------------- core glue

    pub fn execute(&mut self, cmd: Command) -> bool {
        match self.doc.execute(cmd, Origin::User) {
            Ok(()) => {
                self.after_change();
                true
            }
            Err(e) => {
                self.set_status(format!("Edit rejected: {e}"), 4);
                false
            }
        }
    }

    /// Publish a new engine snapshot if the document changed.
    pub fn after_change(&mut self) {
        self.remember_device_names();
        let p = self.doc.project();
        if let Some(t) = self.selected_track
            && p.track(t).is_none()
        {
            self.selected_track = p.tracks.first().map(|t| t.id);
        }
        if let Some(c) = self.selected_clip
            && !p.clip_id_in_use(c)
        {
            self.selected_clip = None;
        }
        if self.selected_section.is_some_and(|s| p.section_index(s).is_none()) {
            self.selected_section = None;
        }
        if self.menu.clip.is_some_and(|c| !p.clip_id_in_use(c)) {
            self.menu_close();
        }
        self.pr_sync_clip();
        if self.published_rev != Some(self.doc.revision()) {
            self.publish();
        }
        self.devices_after_change();
        self.ui_dirty = true;
    }

    /// The session the engine should play right now: the document plus any
    /// automation lane being edited.
    pub fn current_snapshot(&self) -> snapshot::SessionSnapshot {
        let (mut snap, _skipped) = snapshot::build_full(self.doc.project(), self.rate, &self.asset_map, &self.reversed_assets, &self.transients_by_asset());
        // Automation drag / write preview: the pending lane replaces the model lane.
        if let Some((key, lane)) = &self.auto.pending {
            let p = self.doc.project();
            let ls = snapshot::LaneSnapshot { target: lane.target.clone(), points: lane.points.iter().map(|q| (p.ticks_to_frames(q.time, self.rate), q.value, q.curve)).collect() };
            let lanes = match key {
                None => Some(&mut snap.master_automation),
                Some(t) => snap.tracks.iter_mut().find(|ts| ts.track_id == t.0).map(|ts| &mut ts.automation),
            };
            if let Some(lanes) = lanes {
                lanes.retain(|l| l.target != lane.target);
                if lane.enabled && !lane.points.is_empty() {
                    lanes.push(ls);
                }
            }
        }
        snap
    }

    pub fn publish(&mut self) {
        self.request_onsets();
        self.ensure_reversed_assets();
        let snap = self.current_snapshot();
        if let Err(e) = self.engine.publish(&snap) {
            self.set_status(format!("Engine rejected the session: {e}"), 6);
        }
        self.published_rev = Some(self.doc.revision());
        self.restore_mod_bases();
        self.sync_loop_and_click();
    }

    /// Push loop region and metronome grid (both derived from the project
    /// transport) to the engine. Cheap; called after every publish.
    pub(crate) fn sync_loop_and_click(&mut self) {
        let p = self.doc.project();
        let tr = &p.transport;
        let (start, end) = (p.ticks_to_frames(tr.loop_start, self.rate), p.ticks_to_frames(tr.loop_end, self.rate));
        let beat_ticks = PPQ * 4 / tr.time_signature.denominator as i64;
        let frames_per_beat = p.ticks_to_seconds(beat_ticks) * self.rate;
        let cfg = MetronomeConfig { enabled: self.metronome_on, gain: 0.5, frames_per_beat, beats_per_bar: tr.time_signature.numerator as u32 };
        let r = self.engine.set_loop(tr.loop_enabled, start, end).and_then(|_| self.engine.set_metronome(cfg));
        if let Err(e) = r {
            self.set_status(format!("Engine: {e}"), 4);
        }
    }

    pub fn toggle_loop(&mut self) {
        let tr = self.doc.project().transport.clone();
        let cmd = Command::SetLoop { enabled: !tr.loop_enabled, start: tr.loop_start, end: tr.loop_end };
        match self.doc.execute(cmd, Origin::User) {
            Ok(()) => self.after_change(),
            Err(e) => self.set_status(format!("Loop: {e}"), 4),
        }
    }

    pub fn toggle_metronome(&mut self) {
        self.metronome_on = !self.metronome_on;
        self.sync_loop_and_click();
        self.ui_dirty = true;
    }

    pub fn undo(&mut self) {
        self.synth.drop_gesture();
        self.auto.sel = None;
        match self.doc.undo() {
            Ok(true) => self.after_change(),
            Ok(false) => {}
            Err(e) => self.set_status(format!("Undo failed: {e}"), 4),
        }
    }

    pub fn redo(&mut self) {
        self.synth.drop_gesture();
        self.auto.sel = None;
        match self.doc.redo() {
            Ok(true) => self.after_change(),
            Ok(false) => {}
            Err(e) => self.set_status(format!("Redo failed: {e}"), 4),
        }
    }

    // ---------------------------------------------------------------- transport

    pub fn play_toggle(&mut self) {
        if !self.engine.device_running() {
            self.set_status("No audio device: playback unavailable (offline mode)", 4);
            return;
        }
        let r = if self.engine.transport().playing { self.engine.pause() } else { self.engine.play() };
        if let Err(e) = r {
            self.set_status(format!("Transport: {e}"), 4);
        }
        self.ui_dirty = true;
    }

    pub fn stop_return(&mut self) {
        if self.rec.recording {
            self.stop_recording();
        }
        let _ = self.engine.stop();
        self.ui_dirty = true;
    }

    /// Position the user hears: transport position minus plugin delay
    /// compensation and output latency while playing (equal to the transport
    /// position when stopped). Used for the playhead and all readouts.
    pub fn audible_frames(&self) -> i64 {
        self.engine.audible_position().max(0)
    }

    pub fn playhead_ticks(&self) -> Ticks {
        self.project().frames_to_ticks(self.audible_frames(), self.rate)
    }

    pub fn seek_beats(&mut self, beats: f64) {
        let ticks = (beats.max(0.0) * PPQ as f64).round() as Ticks;
        let frame = self.project().ticks_to_frames(ticks, self.rate);
        // Applied immediately when no device runs (engine ABI), else at the next block.
        let _ = self.engine.seek(frame);
    }

    // ---------------------------------------------------------------- track edits

    pub fn select_track(&mut self, id: TrackId) {
        if self.selected_track != Some(id) {
            self.selected_section = None;
            self.selected_track = Some(id);
            if let Some(c) = self.selected_clip
                && self.clip_track(c) != Some(id)
            {
                self.selected_clip = None;
            }
            self.sel_peak_hold = 0.0;
            self.meter_disp = (0.0, 0.0);
            self.synth.drop_gesture();
            self.synth.menu = None;
            // Instrument tracks open their device (Devices tab) unless a clip is selected.
            if self.selected_clip.is_none() && self.project().track(id).is_some_and(|t| t.kind == TrackKind::Instrument) {
                self.bottom_tab = BottomTab::Devices;
                self.pr_sync_clip();
            }
            self.ui_dirty = true;
        }
    }

    pub fn add_audio_track(&mut self) -> Option<TrackId> {
        let track = Track::new(TrackId(0), TrackKind::Audio, "Audio", next_track_color(self.project()));
        if self.execute(Command::AddTrack { index: None, track }) {
            let id = self.project().tracks.last().map(|t| t.id);
            if let Some(id) = id {
                self.select_track(id);
            }
            return id;
        }
        None
    }

    pub fn delete_selection(&mut self) {
        if let Some(s) = self.selected_section.filter(|s| self.project().section_index(*s).is_some()) {
            self.delete_section(s);
        } else if let Some(c) = self.selected_clip.take() {
            let cmd = if self.project().find_midi_clip(c).is_some() { Command::RemoveMidiClip { clip: c } } else { Command::RemoveClip { clip: c } };
            self.execute(cmd);
        } else if let Some(t) = self.selected_track {
            let idx = self.project().track_index(t).unwrap_or(0);
            if self.execute(Command::RemoveTrack { track: t }) {
                let p = self.project();
                self.selected_track = p.tracks.get(idx.min(p.tracks.len().saturating_sub(1))).map(|t| t.id);
                self.ui_dirty = true;
            }
        }
    }

    pub fn toggle_mute(&mut self, id: TrackId) {
        if let Some(t) = self.project().track(id) {
            let mute = !t.mute;
            self.execute(Command::SetTrackMute { track: id, mute });
        }
    }

    pub fn toggle_solo(&mut self, id: TrackId) {
        if let Some(t) = self.project().track(id) {
            let solo = !t.solo;
            self.execute(Command::SetTrackSolo { track: id, solo });
        }
    }

    /// Continuous drags: the core coalesces consecutive SetTrackGain/Pan for one target.
    pub fn set_track_gain(&mut self, id: TrackId, gain: f32) {
        let gain = gain.clamp(0.0, saine_core::command::MAX_GAIN);
        if self.auto_intercept(Some(id), saine_core::AutomationTarget::Volume, gain_to_db(gain)) {
            return;
        }
        if self.project().track(id).map(|t| (t.gain - gain).abs() > 1e-7).unwrap_or(false) {
            self.execute(Command::SetTrackGain { track: id, gain });
        }
    }

    pub fn set_track_pan(&mut self, id: TrackId, pan: f32) {
        let pan = (pan.clamp(-1.0, 1.0) * 100.0).round() / 100.0;
        if self.auto_intercept(Some(id), saine_core::AutomationTarget::Pan, pan) {
            return;
        }
        if self.project().track(id).map(|t| (t.pan - pan).abs() > 1e-7).unwrap_or(false) {
            self.execute(Command::SetTrackPan { track: id, pan });
        }
    }

    pub fn set_master_gain(&mut self, gain: f32) {
        if self.auto_intercept(None, saine_core::AutomationTarget::Volume, gain_to_db(gain)) {
            return;
        }
        if (self.project().master_gain - gain).abs() > 1e-7 {
            self.execute(Command::SetMasterGain { gain });
        }
    }

    // ---------------------------------------------------------------- mixer

    pub fn toggle_mixer(&mut self) {
        self.mixer_open = !self.mixer_open;
        self.ballistics.reset_hold();
        self.ui_dirty = true;
    }

    /// Mixer strip fader (0 = +6 dB at the top .. 1 = −∞): same coalesced SetTrackGain as the headers.
    pub fn set_mixer_fader(&mut self, id: TrackId, f: f32) {
        self.set_track_gain(id, db_to_gain(db_from_fader(f)));
    }

    /// Master strip fader: SetMasterGain (coalesced).
    pub fn set_master_fader(&mut self, f: f32) {
        self.set_master_gain(db_to_gain(db_from_fader(f)).min(saine_core::command::MAX_GAIN));
    }

    pub fn reset_peaks(&mut self) {
        self.ballistics.reset_hold();
        self.sel_peak_hold = 0.0;
    }

    // ---------------------------------------------------------------- import / persistence

    /// Import on worker threads; clips land at `at` (default: the playhead).
    pub fn import_files(&mut self, paths: Vec<PathBuf>, target: ImportTarget, at: Option<Ticks>) {
        let at = at.unwrap_or_else(|| self.playhead_ticks());
        let doc_epoch = self.doc_epoch;
        let target = match target {
            ImportTarget::Selected => self.selected_audio_track().map_or(ImportTarget::NewTrack, ImportTarget::Track),
            t => t,
        };
        for (i, path) in paths.into_iter().enumerate() {
            let engine = self.engine.clone();
            let tx = self.jobs_tx.clone();
            // Additional files go to new tracks so they line up at the playhead.
            let target = if i == 0 || matches!(target, ImportTarget::DeviceSample { .. }) { target } else { ImportTarget::NewTrack };
            self.pending_jobs += 1;
            std::thread::Builder::new()
                .name("import".into())
                .spawn(move || {
                    let result = load_and_analyse(&engine, &path);
                    let _ = tx.send(JobResult::Imported { path, target, at, doc_epoch, result });
                })
                .expect("spawn import thread");
        }
        self.set_status("Importing…", 30);
    }

    /// The selected track if it is an audio track, else the first audio track.
    fn selected_audio_track(&self) -> Option<TrackId> {
        let p = self.project();
        self.selected_track
            .and_then(|t| p.track(t))
            .filter(|t| t.kind == TrackKind::Audio)
            .or_else(|| p.tracks.iter().find(|t| t.kind == TrackKind::Audio))
            .map(|t| t.id)
    }

    /// Drain finished background jobs (called from a UI timer and each frame).
    pub fn poll_jobs(&mut self) {
        self.clip_render_tick();
        while let Ok(job) = self.jobs_rx.try_recv() {
            self.pending_jobs = self.pending_jobs.saturating_sub(1);
            match job {
                JobResult::Imported { doc_epoch, result: Ok((info, _)), .. } if doc_epoch != self.doc_epoch => {
                    // Started in a project that has since been closed: its target
                    // track id could name a track in the new project.
                    let _ = self.engine.release_asset(info.asset_id);
                }
                JobResult::Imported { doc_epoch, .. } if doc_epoch != self.doc_epoch => {}
                JobResult::Imported { path, target, at, result, .. } => match result {
                    Ok((info, pyr)) => self.finish_import(&path, target, at, info, pyr),
                    Err(e) => self.set_status(format!("Import failed: {e}"), 6),
                },
                JobResult::Assistant { started, mode, result } => self.finish_assistant(started, mode, result),
                JobResult::Recorded { doc_epoch, takes } => self.finish_recorded(doc_epoch, takes),
                JobResult::TestTone(r) => self.finish_test_tone(r),
                JobResult::PluginScan(r) => self.finish_plugin_scan(r),
                JobResult::Onsets { engine_asset, result, tempo } => self.finish_onsets(engine_asset, result, tempo),
                JobResult::Auditioned { path, result } => self.finish_audition(path, result),
                JobResult::ClipRendered(r) => self.finish_clip_render(*r),
                JobResult::Reloaded { asset, doc_epoch, result } => {
                    if doc_epoch != self.doc_epoch {
                        if let Ok((info, _)) = result {
                            let _ = self.engine.release_asset(info.asset_id);
                        }
                        continue;
                    }
                    match result {
                        Ok((info, pyr)) => {
                            self.peaks.insert(info.asset_id, pyr);
                            self.asset_map.insert(asset, info.asset_id);
                            self.publish();
                            self.ui_dirty = true;
                        }
                        Err(e) => self.set_status(format!("Missing media: {e}"), 6),
                    }
                }
            }
            if self.pending_jobs == 0 && self.status.as_ref().is_some_and(|s| s.0.starts_with("Importing") || s.0.starts_with("Loading")) {
                self.status = None;
                self.ui_dirty = true;
            }
        }
    }

    fn finish_import(&mut self, path: &Path, target: ImportTarget, at: Ticks, info: AssetInfo, pyr: Pyramid) {
        let p = self.project();
        let mut next = p.next_id;
        let mut alloc = || {
            let id = next;
            next += 1;
            id
        };
        let asset_id = AssetId(alloc());
        let native_frames = (info.frames as f64 * info.source_sample_rate / self.rate).round().max(1.0) as u64;
        let asset = Asset {
            id: asset_id,
            relative_path: None,
            absolute_path: std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf()).to_string_lossy().into_owned(),
            sample_rate: info.source_sample_rate,
            channels: info.channels,
            frames: native_frames,
        };
        if let ImportTarget::DeviceSample { device, slot, next_free } = target {
            self.peaks.insert(info.asset_id, pyr);
            self.asset_map.insert(asset_id, info.asset_id);
            let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Sample".into());
            if !self.finish_device_sample(asset, name, device, slot, next_free) {
                self.asset_map.remove(&asset_id);
                let _ = self.engine.release_asset(info.asset_id);
            }
            self.ui_dirty = true;
            return;
        }
        let mut cmds = vec![Command::AddAsset { asset }];
        let existing = match target {
            ImportTarget::DeviceSample { .. } => None,
            ImportTarget::NewTrack => None,
            ImportTarget::Selected => self.selected_audio_track(),
            ImportTarget::Track(t) => p.track(t).filter(|t| t.kind == TrackKind::Audio).map(|t| t.id),
        };
        let track_id = existing.unwrap_or_else(|| {
            let id = TrackId(alloc());
            cmds.push(Command::AddTrack { index: None, track: Track::new(id, TrackKind::Audio, "Audio", next_track_color(p)) });
            id
        });
        let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Audio".into());
        let clip_id = ClipId(alloc());
        cmds.push(Command::AddClip {
            track: track_id,
            clip: Clip { id: clip_id, asset: asset_id, name, start: at.max(0), source_offset: 0, source_length: native_frames, gain: 1.0, warp: Default::default(), disabled: false },
        });
        // Engine asset must be known before the snapshot is published.
        self.peaks.insert(info.asset_id, pyr);
        self.asset_map.insert(asset_id, info.asset_id);
        if self.execute(Command::Batch { label: "Import Audio".into(), commands: cmds }) {
            self.selected_track = Some(track_id);
            self.selected_clip = Some(clip_id);
            self.ui_dirty = true;
        } else {
            self.asset_map.remove(&asset_id);
            let _ = self.engine.release_asset(info.asset_id);
        }
    }

    /// Start over with `project` (no undo history). Used by `--state` setups.
    pub fn replace_project(&mut self, project: Project) {
        self.close_all_editors();
        self.onsets.clear();
        self.tempos.clear();
        self.doc_epoch += 1;
        self.auto.reset_project_state();
        self.doc = Document::new(project);
        self.devices.modu.bases_pending = true;
        self.selected_track = self.project().tracks.first().map(|t| t.id);
        self.selected_clip = None;
        self.published_rev = None;
        self.after_change();
    }

    pub fn save_to(&mut self, dir: &Path) {
        // Plugin instances own their state: pull it into the model first (no undo step).
        let states = self.engine.plugin_states();
        self.doc.sync_device_states(&states);
        let bases = self.mod_bases_now();
        self.doc.sync_mod_bases(&bases);
        match persist::save(self.project(), dir) {
            Ok(()) => {
                self.doc.mark_saved();
                self.project_dir = Some(dir.to_path_buf());
                self.set_status(format!("Saved {}", dir.display()), 3);
                self.files_refresh();
            }
            Err(e) => self.set_status(format!("Save failed: {e}"), 6),
        }
    }

    pub fn open(&mut self, dir: &Path) {
        let loaded = match persist::load(dir) {
            Ok(l) => l,
            Err(e) => {
                self.set_status(format!("Open failed: {e}"), 6);
                return;
            }
        };
        self.stop_return();
        self.close_all_editors();
        for (_, ea) in self.asset_map.drain().chain(self.reversed_assets.drain()) {
            let _ = self.engine.release_asset(ea);
        }
        self.onsets.clear();
        self.tempos.clear();
        self.doc_epoch += 1;
        self.auto.reset_project_state();
        self.doc = Document::new(loaded.project);
        self.devices.modu.bases_pending = true;
        self.project_dir = Some(dir.to_path_buf());
        self.selected_track = self.project().tracks.first().map(|t| t.id);
        self.selected_clip = None;
        self.view = View::default();
        self.published_rev = None;
        self.after_change();
        self.devices_after_open();
        let missing: std::collections::HashSet<String> = loaded.missing_media.into_iter().collect();
        for a in self.project().assets.clone() {
            if !missing.contains(&a.absolute_path) {
                self.load_asset(&a);
            }
        }
        if !missing.is_empty() {
            self.set_status(format!("{} media file(s) missing", missing.len()), 6);
        } else if self.pending_jobs > 0 {
            self.set_status("Loading media…", 30);
        }
    }

    /// Load a project asset into the engine on a worker thread (finishes in `poll_jobs`).
    pub(crate) fn load_asset(&mut self, a: &Asset) {
        let engine = self.engine.clone();
        let tx = self.jobs_tx.clone();
        let epoch = self.doc_epoch;
        let (id, path) = (a.id, a.absolute_path.clone());
        self.pending_jobs += 1;
        std::thread::spawn(move || {
            let result = load_and_analyse(&engine, Path::new(&path));
            let _ = tx.send(JobResult::Reloaded { asset: id, doc_epoch: epoch, result });
        });
    }

    // ---------------------------------------------------------------- view

    pub fn scroll(&mut self, dx: f32, dy: f32, x: f32, zoom: bool) {
        if zoom {
            self.view.zoom_at((dy as f64 * 0.01).exp(), x);
        } else {
            self.view.start_beats -= dx as f64 / self.view.px_per_beat;
            self.view.scroll_y -= dy as f64;
            self.view.clamp();
        }
        let n = self.project().tracks.len();
        self.view.clamp_scroll(n, self.lanes_size.1);
    }

    // ---------------------------------------------------------------- lane gestures

    fn beats_per_bar(&self) -> f64 {
        self.project().ticks_per_bar() as f64 / PPQ as f64
    }

    fn ruler_steps(&self) -> timeline::RulerSteps {
        let beat_len = 4.0 / self.project().transport.time_signature.denominator as f64;
        timeline::RulerSteps::new(self.view.px_per_beat, self.beats_per_bar(), beat_len, self.snap.grid)
    }

    pub fn hover(&self, x: f32, y: f32) -> i32 {
        if self.auto.shown && y >= timeline::RULER_H {
            return 0;
        }
        match timeline::hit_test(&self.view, self.lanes_size, self.project().tracks.len(), &self.clip_vis, x, y) {
            Hit::Clip { zone: Zone::Left | Zone::Right, .. } => 1,
            Hit::Clip { zone: Zone::Body, .. } => 2,
            _ => 0,
        }
    }

    fn clip_region(&self, id: ClipId) -> Option<(TrackId, Ticks, u64, u64, f64, u64)> {
        let p = self.project();
        let (ti, ci) = p.find_clip(id)?;
        let c = &p.tracks[ti].clips[ci];
        let a = p.asset(c.asset)?;
        Some((p.tracks[ti].id, c.start, c.source_offset, c.source_length, a.sample_rate, a.frames))
    }

    /// Lane pointer with modifiers: automation rows take it while lanes are shown.
    pub fn pointer_mods(&mut self, kind: i32, x: f32, y: f32, shift: bool, alt: bool) {
        if self.drag.is_none() && self.section_pointer(kind, x, y) {
            return;
        }
        if kind == 0 && self.selected_section.take().is_some() {
            self.ui_dirty = true;
        }
        if self.drag.is_none() && self.auto_pointer(kind, x, y, shift, alt) {
            return;
        }
        let beat = self.view.beat_at(x).max(0.0);
        match kind {
            0 => {
                let hit = timeline::hit_test(&self.view, self.lanes_size, self.project().tracks.len(), &self.clip_vis, x, y);
                match hit {
                    Hit::Ruler => {
                        self.seek_beats(beat);
                        self.drag = Some(Drag::Scrub);
                    }
                    Hit::Clip { index, zone } => {
                        let id = self.clip_ids[index];
                        self.pr.focused = false;
                        if let Some((ti, ci)) = self.project().find_midi_clip(id) {
                            let (track, c) = (self.project().tracks[ti].id, &self.project().tracks[ti].midi_clips[ci]);
                            let (start, len) = (c.start, c.length);
                            self.select_track(track);
                            self.select_clip(Some(id));
                            self.drag = Some(match zone {
                                Zone::Body => Drag::Move { clip: id, grab_beats: beat - start as f64 / PPQ as f64, orig_track: track, orig_start: start, track, start },
                                Zone::Left => Drag::MidiLeft { clip: id, orig: (start, len), cur: (start, len) },
                                Zone::Right => Drag::MidiRight { clip: id, orig: (start, len), cur: (start, len) },
                            });
                            self.ui_dirty = true;
                            return;
                        }
                        let Some((track, start, off, len, _, _)) = self.clip_region(id) else { return };
                        self.select_track(track);
                        self.select_clip(Some(id));
                        self.drag = Some(match zone {
                            Zone::Body => Drag::Move { clip: id, grab_beats: beat - start as f64 / PPQ as f64, orig_track: track, orig_start: start, track, start },
                            Zone::Left => Drag::TrimLeft { clip: id, orig: (start, off, len), cur: (start, off, len) },
                            Zone::Right => Drag::TrimRight { clip: id, orig: (start, off, len), cur: (start, off, len) },
                        });
                    }
                    Hit::Track(i) => {
                        let id = self.project().tracks[i].id;
                        self.select_track(id);
                        self.select_clip(None);
                        self.pr.focused = false;
                        self.seek_beats(beat);
                    }
                    Hit::Empty => {
                        self.select_clip(None);
                        self.pr.focused = false;
                    }
                    Hit::Master => {}
                }
                self.ui_dirty = true;
            }
            1 => {
                let Some(drag) = self.drag.clone() else { return };
                let (steps, on) = (self.ruler_steps(), self.snap_active());
                let snap = |b: f64| (if on { steps.snap(b) } else { b.max(0.0) } * PPQ as f64).round() as Ticks;
                match drag {
                    Drag::Scrub => self.seek_beats(beat),
                    Drag::Move { clip, grab_beats, orig_track, orig_start, .. } => {
                        let want = beat - grab_beats;
                        let start = (if on { steps.snap_move(want, orig_start as f64 / PPQ as f64) } else { want.max(0.0) } * PPQ as f64).round() as Ticks;
                        let p = self.project();
                        let kind = p.track(orig_track).map(|t| t.kind);
                        let track = self
                            .view
                            .track_at(y)
                            .and_then(|i| p.tracks.get(i))
                            .filter(|t| Some(t.kind) == kind)
                            .map(|t| t.id)
                            .unwrap_or(orig_track);
                        self.drag = Some(Drag::Move { clip, grab_beats, orig_track, orig_start, track, start });
                    }
                    Drag::TrimLeft { clip, orig, .. } => {
                        let p = self.project();
                        let Some(c) = p.clip(clip) else { return };
                        let (s0, off0, len0) = orig;
                        if c.warp.reverse {
                            // Reversed: the left edge plays the window's end, so it trims (or extends) that.
                            let l0 = p.clip_ticks_at(c, (off0 + len0) as f64);
                            let frames = p.asset(c.asset).map_or(off0 + len0, |a| a.frames);
                            let min_start = s0 + (l0 - p.clip_ticks_at(c, frames as f64)).ceil() as Ticks;
                            let max_start = s0 + l0 as Ticks - PPQ / 4;
                            let start = snap(beat).clamp(min_start.max(0), max_start.max(s0));
                            let end = p.clip_source_at(c, l0 - (start - s0) as f64).round().clamp((off0 + 1) as f64, frames as f64) as u64;
                            self.drag = Some(Drag::TrimLeft { clip, orig, cur: (start, off0, end - off0) });
                            self.ui_dirty = true;
                            return;
                        }
                        let at = |t: Ticks| p.clip_source_at(c, (t - s0) as f64);
                        let mut start = snap(beat);
                        // Source material before the current offset (warped clips follow their map).
                        let min_start = s0 + p.clip_ticks_at(c, 0.0).ceil() as Ticks;
                        let max_start = s0 + p.clip_ticks_at(c, (off0 + len0) as f64) as Ticks - PPQ / 4;
                        start = start.clamp(min_start.max(0), max_start.max(s0));
                        let off = at(start).round().max(0.0) as u64;
                        let len = (len0 as i64 - (off as i64 - off0 as i64)).max(1) as u64;
                        self.drag = Some(Drag::TrimLeft { clip, orig, cur: (start, off, len) });
                    }
                    Drag::MidiLeft { clip, orig, .. } => {
                        let end = orig.0 + orig.1;
                        let start = snap(beat).clamp(0, end - PPQ);
                        self.drag = Some(Drag::MidiLeft { clip, orig, cur: (start, end - start) });
                    }
                    Drag::MidiRight { clip, orig, .. } => {
                        let end = snap(beat).max(orig.0 + PPQ);
                        self.drag = Some(Drag::MidiRight { clip, orig, cur: (orig.0, end - orig.0) });
                    }
                    Drag::TrimRight { clip, orig, .. } => {
                        let Some((_, _, _, _, _, frames)) = self.clip_region(clip) else { return };
                        let (s0, off0, len0) = orig;
                        let p = self.project();
                        let Some(c) = p.clip(clip) else { return };
                        let end = snap(beat).max(s0 + PPQ / 4);
                        if c.warp.reverse {
                            // Reversed: the right edge plays the window's start.
                            let l0 = p.clip_ticks_at(c, (off0 + len0) as f64);
                            let end = end.min((s0 + (l0 - p.clip_ticks_at(c, 0.0)).floor() as Ticks).max(s0 + PPQ / 4));
                            let off = p.clip_source_at(c, l0 - (end - s0) as f64).round().clamp(0.0, (off0 + len0 - 1) as f64) as u64;
                            self.drag = Some(Drag::TrimRight { clip, orig, cur: (s0, off, off0 + len0 - off) });
                            self.ui_dirty = true;
                            return;
                        }
                        let len = (p.clip_source_at(c, (end - s0) as f64) - off0 as f64).round().max(1.0) as u64;
                        let len = len.clamp(1, frames - off0);
                        self.drag = Some(Drag::TrimRight { clip, orig, cur: (s0, off0, len) });
                    }
                }
                self.ui_dirty = true;
            }
            _ => {
                match self.drag.take() {
                    Some(Drag::Move { clip, orig_track, orig_start, track, start, .. }) if (track, start) != (orig_track, orig_start) => {
                        let midi = self.project().find_midi_clip(clip).is_some();
                        self.execute(if midi { Command::MoveMidiClip { clip, track, start } } else { Command::MoveClip { clip, track, start } });
                        self.select_track(track);
                    }
                    Some(Drag::MidiRight { clip, orig, cur }) if orig != cur => {
                        self.execute(Command::ResizeMidiClip { clip, start: cur.0, length: cur.1 });
                    }
                    Some(Drag::MidiLeft { clip, orig, cur }) if orig != cur => {
                        if let Some(cmd) = self.project().midi_clip(clip).and_then(|c| pianoroll::midi_left_resize(c, cur.0)) {
                            self.execute(cmd);
                        }
                    }
                    Some(Drag::TrimLeft { clip, orig, cur } | Drag::TrimRight { clip, orig, cur }) if orig != cur => {
                        self.execute(Command::TrimClip { clip, start: cur.0, source_offset: cur.1, source_length: cur.2 });
                    }
                    _ => {}
                }
                self.ui_dirty = true;
            }
        }
    }

    // ---------------------------------------------------------------- per-frame

    /// Waveform of an audio clip: its pyramid slot (once the peaks exist) and its pieces.
    pub fn clip_wave(&self, clip: &Clip) -> Option<(crate::peaks::Slot, Vec<crate::peaks::WaveSeg>)> {
        let slot = self.clip_engine_asset(clip).and_then(|ea| self.peaks.slot(ea))?;
        Some((slot, crate::peaks::wave_segments(self.project(), clip, self.rate)))
    }

    /// The clip editor's waveform: a lane per channel once `ensure_lane_peaks`
    /// has run for a stereo asset, else the mono mix.
    pub fn clip_lanes(&self, clip: &Clip) -> Option<(Vec<crate::peaks::Slot>, Vec<crate::peaks::WaveSeg>)> {
        let ea = self.clip_engine_asset(clip)?;
        let slots = match self.peaks.channel_slots(ea) {
            Some(lr) => lr.to_vec(),
            None => vec![self.peaks.slot(ea)?],
        };
        Some((slots, crate::peaks::wave_segments(self.project(), clip, self.rate)))
    }

    /// Engine asset a clip plays (the reversed copy when it plays back to front).
    fn clip_engine_asset(&self, clip: &Clip) -> Option<u64> {
        let assets = if clip.warp.reverse { &self.reversed_assets } else { &self.asset_map };
        assets.get(&clip.asset).copied()
    }

    /// Per-channel peaks of the open stereo audio clip, for the editor's two
    /// lanes. Computed once per engine asset (about a ms per channel-minute).
    pub fn ensure_lane_peaks(&mut self) {
        let Some(c) = self.ac_clip() else { return };
        if self.project().asset(c.asset).is_none_or(|a| a.channels != 2) {
            return;
        }
        let Some(ea) = self.clip_engine_asset(c) else { return };
        if self.peaks.channel_slots(ea).is_some() {
            return;
        }
        let Ok(info) = self.engine.asset_info(ea) else { return };
        let pairs = (info.frames / crate::peaks::L0_FRAMES as u64 + 1) as usize;
        let lane = |ch: u32| self.engine.channel_peaks(ea, ch, 0, crate::peaks::L0_FRAMES, pairs).map(crate::peaks::Pyramid::from_level0);
        if let (Ok(l), Ok(r)) = (lane(0), lane(1)) {
            self.peaks.insert_channels(ea, [l, r]);
        }
    }

    /// Rebuild the clip/track visuals (positions include in-progress drag previews).
    pub fn rebuild_vis(&mut self) {
        self.auto_sync_view();
        let p = self.doc.project();
        self.track_vis = p.tracks.iter().map(|t| TrackVis { selected: Some(t.id) == self.selected_track }).collect();
        self.section_vis = self.build_section_vis();
        let p = self.doc.project();
        self.clip_vis.clear();
        self.clip_ids.clear();
        for (ti, t) in p.tracks.iter().enumerate() {
            for c in &t.clips {
                let (mut track_index, mut start, mut off, mut len) = (ti, c.start, c.source_offset, c.source_length);
                match &self.drag {
                    Some(Drag::Move { clip, track, start: s, .. }) if *clip == c.id => {
                        track_index = p.track_index(*track).unwrap_or(ti);
                        start = *s;
                    }
                    Some(Drag::TrimLeft { clip, cur, .. } | Drag::TrimRight { clip, cur, .. }) if *clip == c.id => {
                        (start, off, len) = *cur;
                    }
                    _ => {}
                }
                let shown = saine_core::Clip { source_offset: off, source_length: len, ..c.clone() };
                let len_beats = p.clip_length_ticks(&shown) as f64 / PPQ as f64;
                let wave = self.clip_wave(&shown);
                self.clip_vis.push(ClipVis {
                    track_index,
                    start_beats: start as f64 / PPQ as f64,
                    len_beats,
                    color: track_color(t.color),
                    selected: Some(c.id) == self.selected_clip,
                    muted: t.mute || c.disabled,
                    name: c.name.clone(),
                    wave,
                    midi: None,
                    live: None,
                });
                self.clip_ids.push(c.id);
            }
            for c in &t.midi_clips {
                let (mut track_index, mut start, mut len) = (ti, c.start, c.length);
                match &self.drag {
                    Some(Drag::Move { clip, track, start: s, .. }) if *clip == c.id => {
                        track_index = p.track_index(*track).unwrap_or(ti);
                        start = *s;
                    }
                    Some(Drag::MidiLeft { clip, cur, .. } | Drag::MidiRight { clip, cur, .. }) if *clip == c.id => (start, len) = *cur,
                    _ => {}
                }
                // Notes as displayed: piano-roll gesture preview for the open clip.
                let notes = if self.pr.clip == Some(c.id) { self.pr_notes() } else { c.notes.clone() };
                let shift = start - c.start; // left-resize preview keeps notes in place on the timeline
                self.clip_vis.push(ClipVis {
                    track_index,
                    start_beats: start as f64 / PPQ as f64,
                    len_beats: len as f64 / PPQ as f64,
                    color: track_color(t.color),
                    selected: Some(c.id) == self.selected_clip,
                    muted: t.mute || c.disabled,
                    name: c.name.clone(),
                    wave: None,
                    midi: Some(mini_notes(&notes, shift, len)),
                    live: None,
                });
                self.clip_ids.push(c.id);
            }
        }
        self.rebuild_live();
    }

    /// Track that holds a clip (audio or MIDI).
    pub fn clip_track(&self, id: ClipId) -> Option<TrackId> {
        let p = self.project();
        p.find_clip(id).or_else(|| p.find_midi_clip(id)).map(|(t, _)| p.tracks[t].id)
    }

    /// Select a clip (or none). Selecting a MIDI clip opens the Clip tab (piano roll).
    pub fn select_clip(&mut self, id: Option<ClipId>) {
        self.selected_clip = id;
        if id.is_some() {
            self.selected_section = None;
        }
        if id.is_some_and(|c| self.project().find_midi_clip(c).is_some()) {
            self.bottom_tab = BottomTab::Clip;
        }
        self.pr_sync_clip();
        self.ui_dirty = true;
    }

    pub fn set_bottom_tab(&mut self, tab: BottomTab) {
        self.bottom_tab = tab;
        self.pr_sync_clip();
        self.ui_dirty = true;
    }

    pub fn scene<'a>(&'a self, scale: f32) -> timeline::Scene<'a> {
        let p = self.project();
        let tr = &p.transport;
        timeline::Scene {
            view: self.view.clone(),
            size: self.lanes_size,
            scale,
            tracks: &self.track_vis,
            clips: &self.clip_vis,
            live: &self.rec.live_vis,
            playhead_beats: self.playhead_ticks() as f64 / PPQ as f64,
            loop_beats: (tr.loop_start as f64 / PPQ as f64, tr.loop_end as f64 / PPQ as f64),
            loop_on: tr.loop_enabled,
            beats_per_bar: self.beats_per_bar(),
            steps: self.ruler_steps(),
            auto: None,
            sections: &self.section_vis,
        }
    }

    /// Push transport readouts, meters and (when dirty) models to Slint.
    /// Returns true while something is animating (playing, meters decaying, jobs).
    pub fn update_ui(&mut self, app: &AppWindow) -> bool {
        let s = app.global::<Studio>();
        {
            let scale = app.window().scale_factor();
            let sz = app.window().size();
            let win = (sz.width as f32 / scale, sz.height as f32 / scale);
            let sounds = s.get_sounds_visible();
            if win != self.devices.win || sounds != self.devices.sounds {
                self.devices.win = win;
                self.devices.sounds = sounds;
                self.ui_dirty = true;
            }
        }
        let tp = self.engine.transport();
        let pos = self.audible_frames();
        let p = self.doc.project();
        // Transport readouts from the engine (audible position, see audible_frames).
        let ticks = p.frames_to_ticks(pos, self.rate).max(0);
        let bar = ticks / p.ticks_per_bar() + 1;
        let secs = pos as f64 / self.rate;
        let ms_total = (secs * 1000.0).floor() as i64;
        let set_if = |cur: SharedString, new: String, f: &dyn Fn(SharedString)| {
            if cur.as_str() != new {
                f(new.into());
            }
        };
        set_if(s.get_bar_text(), format!("{bar:03}"), &|v| s.set_bar_text(v));
        set_if(s.get_time_text(), format!("{:02}:{:02}.{:03}", ms_total / 60_000, (ms_total / 1000) % 60, ms_total % 1000), &|v| s.set_time_text(v));
        if s.get_playing() != tp.playing {
            s.set_playing(tp.playing);
        }
        // App preference (not a document revision), so synced every frame.
        if s.get_metronome_on() != self.metronome_on {
            s.set_metronome_on(self.metronome_on);
        }

        let previewing = self.push_audition(&s);
        self.files.audition.animating = previewing;

        // Meters (master first, then tracks).
        self.engine.peek_meters(&mut self.meters, 256);
        let sel_id = self.selected_track.map(|t| t.0).unwrap_or(u64::MAX);
        let (nl, nr) = self.meters.iter().find(|m| m.track_id == sel_id).map(|m| (m.peak_l, m.peak_r)).unwrap_or((0.0, 0.0));
        let now = Instant::now();
        let dt = self.meter_t.map(|t| (now - t).as_secs_f32()).unwrap_or(0.0).min(0.25);
        self.meter_t = Some(now);
        let fall = db_to_gain(-24.0 * dt);
        let ml = nl.max(self.meter_disp.0 * fall);
        let mr = nr.max(self.meter_disp.1 * fall);
        self.meter_disp = (if ml < 1e-5 { 0.0 } else { ml }, if mr < 1e-5 { 0.0 } else { mr });
        let (ml, mr) = self.meter_disp;
        let fl = fader_from_db(gain_to_db(ml));
        let fr = fader_from_db(gain_to_db(mr));
        if (s.get_sel_meter_l() - fl).abs() > 1e-4 || (s.get_sel_meter_r() - fr).abs() > 1e-4 {
            s.set_sel_meter_l(fl);
            s.set_sel_meter_r(fr);
        }
        // Peak hold readout (since selection change), like the reference "−∞" button.
        self.sel_peak_hold = self.sel_peak_hold.max(ml.max(mr));
        let peak_text = fmt_db(gain_to_db(self.sel_peak_hold));
        if s.get_sel_peak_text().as_str() != peak_text {
            s.set_sel_peak_text(peak_text.into());
        }
        let mut metering = ml > 1e-5 || mr > 1e-5;

        // Track-header meters (always shown) and the mixer meters share one ballistics state.
        metering |= self.ballistics.update(&self.meters);
        let p = self.doc.project();
        let level = |id: u64| {
            let (l, r, hold) = self.ballistics.get(id);
            TrackLevel { l: header_meter(l), r: header_meter(r), clip: hold >= 1.0 }
        };
        let levels: Vec<TrackLevel> = p.tracks.iter().map(|t| level(t.id.0)).collect();
        sync_model(&self.track_levels_model, levels);
        let ml = level(MASTER_METER_ID);
        if s.get_master_level() != ml {
            s.set_master_level(ml);
        }

        // Mixer meters (all strips + master), only while the mixer is visible.
        if self.mixer_open {
            let p = self.doc.project();
            let rows: Vec<StripMeter> = p
                .tracks
                .iter()
                .map(|t| {
                    let (l, r, hold) = self.ballistics.get(t.id.0);
                    StripMeter { l: fader_from_db(gain_to_db(l)), r: fader_from_db(gain_to_db(r)), peak: mixer::fmt_peak(hold).into() }
                })
                .collect();
            sync_model(&self.strip_meters_model, rows);
            let (l, r, hold) = self.ballistics.get(MASTER_METER_ID);
            let (fl, fr) = (fader_from_db(gain_to_db(l)), fader_from_db(gain_to_db(r)));
            if (s.get_master_meter_l() - fl).abs() > 1e-4 || (s.get_master_meter_r() - fr).abs() > 1e-4 {
                s.set_master_meter_l(fl);
                s.set_master_meter_r(fr);
            }
            let pt = mixer::fmt_peak(hold);
            if s.get_master_peak_text().as_str() != pt {
                s.set_master_peak_text(pt.into());
            }
        }

        // Assistant highlights (pulse for HIGHLIGHT_SECS, then expire).
        let highlighting = !self.assistant.highlights.is_empty();
        if highlighting || self.highlights_model.row_count() > 0 {
            let scale = app.window().scale_factor();
            let size = app.window().size();
            let layout = self.highlight_layout(size.width as f32 / scale, size.height as f32 / scale, s.get_sounds_visible());
            let rects = self.highlight_rects(&layout);
            sync_model(&self.highlights_model, rects);
        }

        // Status expiry.
        if let Some((_, t0, d)) = &self.status
            && t0.elapsed() > *d
        {
            self.status = None;
            self.ui_dirty = true;
        }

        self.seq_tick(&s);
        if self.ui_rev != Some(self.doc.revision()) {
            self.ui_dirty = true;
        }
        if self.ui_dirty {
            self.ui_dirty = false;
            self.ui_rev = Some(self.doc.revision());
            self.push_models(app);
        }
        self.auto_sync_view();
        self.view.clamp_scroll(self.project().tracks.len(), self.lanes_size.1);
        s.set_scroll_y(self.view.scroll_y as f32);
        self.update_labels();
        self.update_minimap(app);
        self.update_devmm(app);
        self.push_snap(&s);
        crate::pointer::apply(app, self.ptr.take());
        s.set_section_add_x(self.section_add_x());
        self.update_auto_labels();
        if self.auto_any_active() {
            self.push_auto_feedback(&s);
        }
        // The record button state follows the engine every frame while recording.
        if s.get_recording() != self.rec.recording {
            s.set_recording(self.rec.recording);
        }
        tp.playing || previewing || metering || highlighting || self.pending_jobs > 0 || self.drag.is_some() || self.pr_animating() || self.rec.recording || self.test_tone_active() || self.devmm_animating()
    }

    fn push_models(&mut self, app: &AppWindow) {
        self.rebuild_vis();
        let s = app.global::<Studio>();
        self.auto_sync_view();
        let rows = self.track_rows();
        let p = self.doc.project();
        if self.tracks_model.row_count() == rows.len() {
            for (i, r) in rows.into_iter().enumerate() {
                if self.tracks_model.row_data(i).as_ref() != Some(&r) {
                    self.tracks_model.set_row_data(i, r);
                }
            }
        } else {
            self.tracks_model.set_vec(rows);
        }
        s.set_project_name(p.name.clone().into());
        let sentence = saine_core::history::sentence_case;
        s.set_undo_text(self.doc.undo_label().map_or("Undo".into(), |l| format!("Undo {}", sentence(l))).into());
        s.set_redo_text(self.doc.redo_label().map_or("Redo".into(), |l| format!("Redo {}", sentence(l))).into());
        s.set_can_undo(self.doc.can_undo());
        s.set_can_redo(self.doc.can_redo());
        s.set_can_copy(self.selected_clip.is_some());
        s.set_project_on_disk(self.project_dir.is_some());
        s.set_bpm_text(format!("{}", p.transport.bpm.round() as i64).into());
        s.set_loop_enabled(p.transport.loop_enabled);
        s.set_sig_text(format!("{}/{}", p.transport.time_signature.numerator, p.transport.time_signature.denominator).into());
        s.set_master_pos(slider_from_gain(p.master_gain));
        let sel = self.selected_track.and_then(|t| p.track(t).map(|tr| (p.track_index(t).unwrap(), tr)));
        s.set_has_selection(sel.is_some());
        if let Some((i, t)) = sel {
            s.set_sel_id(t.id.0 as i32);
            s.set_sel_number(i as i32 + 1);
            s.set_sel_name(t.name.clone().into());
            s.set_sel_instrument(t.kind == TrackKind::Instrument);
            s.set_sel_color(slint_color(track_color(t.color)));
            s.set_sel_mute(t.mute);
            s.set_sel_solo(t.solo);
            s.set_sel_pan(t.pan);
            s.set_sel_pan_text(fmt_pan(t.pan).into());
            let db = gain_to_db(t.gain);
            s.set_sel_fader(fader_from_db(db));
            s.set_sel_db_text(fmt_db(db).into());
        }
        // ---- mixer strips ----
        s.set_mixer_open(self.mixer_open);
        let strips: Vec<MixerStrip> = p
            .tracks
            .iter()
            .map(|t| MixerStrip {
                id: t.id.0 as i32,
                name: t.name.clone().into(),
                instrument: t.kind == TrackKind::Instrument,
                color: slint_color(track_color(t.color)),
                selected: Some(t.id) == self.selected_track,
                mute: t.mute,
                solo: t.solo,
                pan: t.pan,
                pan_text: mixer::fmt_strip_pan(t.pan).into(),
                fader: fader_from_db(gain_to_db(t.gain)),
                gain_text: mixer::fmt_gain(gain_to_db(t.gain)).into(),
            })
            .collect();
        sync_model(&self.strips_model, strips);
        s.set_master_gain_text(mixer::fmt_gain(gain_to_db(p.master_gain)).into());
        if self.strip_meters_model.row_count() != p.tracks.len() {
            self.strip_meters_model.set_vec(vec![StripMeter { l: 1.0, r: 1.0, peak: "−∞".into() }; p.tracks.len()]);
        }
        s.set_master_fader(fader_from_db(gain_to_db(p.master_gain)));
        // ---- assistant ----
        s.set_assistant_open(self.assistant_open);
        s.set_assistant_pref_w(match self.chat_drag {
            Some(d) => d.w,
            None => self.chat_saved_w(),
        });
        s.set_chat_available(self.provider.available());
        s.set_chat_busy(self.assistant.busy.is_some());
        s.set_chat_notice(self.assistant.notice_visible);
        s.set_teach_mode(self.assistant.mode == saine_core::assistant::Mode::Teach);
        if self.chat_rev != Some(self.assistant.rev) {
            self.chat_rev = Some(self.assistant.rev);
            self.chat_model.set_vec(self.assistant.to_model());
            self.assistant.fresh_from = self.assistant.entries.len();
        }
        match self.assistant_context() {
            Some((name, scope, color, instrument)) => {
                s.set_chat_context_visible(true);
                s.set_chat_context_name(name.into());
                s.set_chat_context_scope(scope.into());
                s.set_chat_context_color(slint_color(track_color(color)));
                s.set_chat_context_instrument(instrument);
            }
            None => s.set_chat_context_visible(false),
        }
        self.push_clip_editor(&s);
        self.push_synth(&s);
        self.push_recording(&s);
        self.push_settings(&s);
        self.push_dialogs(&s);
        self.push_clip_render(&s);
        self.push_devices(&s);
        self.push_files(&s);
        self.push_automation(&s);
        s.set_bottom_h(self.bottom_h());
        let status = if self.show_status { self.status.as_ref().map(|s| s.0.clone()).unwrap_or_default() } else { String::new() };
        s.set_status_text(status.into());
        self.ruler_key.0 = i64::MIN; // force label refresh
    }

    fn update_labels(&mut self) {
        let v = self.view.clone();
        // Section names: 6 reference px in, hidden under 10 px of room (sections.rs).
        let w = self.lanes_size.0;
        let section_labels: Vec<crate::SectionLabel> = self
            .section_vis
            .iter()
            .filter_map(|s| {
                let x0 = v.x_of(s.start_beats);
                let x1 = v.x_of(s.start_beats + s.len_beats);
                let lx = (x0 + 5.1).max(3.4);
                let room = x1 - 5.1 - lx;
                (room >= 8.5 && x1 > 0.0 && x0 < w).then(|| {
                    let ink = crate::sections::label_ink(crate::sections::lift(s.color, s.state));
                    crate::SectionLabel { x: lx, w: room, text: s.name.clone().into(), ink: slint::Color::from_rgb_f32(ink[0], ink[1], ink[2]) }
                })
            })
            .collect();
        crate::controller::sync_model(&self.section_label_model, section_labels);
        let key = ((v.start_beats * 4096.0) as i64, (v.px_per_beat * 4096.0) as i64, self.lanes_size.0 as i64, (self.beats_per_bar() * 1024.0) as i64);
        if key != self.ruler_key {
            self.ruler_key = key;
            let labels = timeline::ruler_labels(&v, self.lanes_size.0, &self.ruler_steps());
            self.ruler_model.set_vec(labels.into_iter().map(|(x, t)| RulerLabel { x, text: t.into() }).collect::<Vec<_>>());
        }
        let w = self.lanes_size.0;
        let bottom = self.lanes_size.1 - v.master_h;
        let labels: Vec<ClipLabel> = self
            .clip_vis
            .iter()
            .chain(self.rec.live_vis.iter())
            .filter_map(|c| {
                let (x, y, cw, _) = c.rect(&v);
                if x > w || x + cw < 0.0 || y < timeline::RULER_H - 1.0 || y + timeline::CLIP_HEADER > bottom {
                    return None;
                }
                let x0 = x.max(0.0);
                Some(ClipLabel { x: x0, y, w: (x + cw).min(w) - x0, text: c.name.clone().into(), selected: c.selected })
            })
            .collect();
        let same = self.clips_model.row_count() == labels.len() && labels.iter().enumerate().all(|(i, l)| self.clips_model.row_data(i).as_ref() == Some(l));
        if !same {
            self.clips_model.set_vec(labels);
        }
    }
}

/// Mini note view of a MIDI clip: (rel start, rel length, rel pitch 0 top .. 1 bottom).
/// `shift` = ticks the clip start moved (left-resize preview).
fn mini_notes(notes: &[saine_core::Note], shift: Ticks, len: Ticks) -> Vec<[f32; 3]> {
    if notes.is_empty() || len <= 0 {
        return Vec::new();
    }
    let lo = notes.iter().map(|n| n.pitch).min().unwrap_or(60) as f32;
    let hi = notes.iter().map(|n| n.pitch).max().unwrap_or(60) as f32;
    let pad = ((12.0 - (hi - lo)) / 2.0).max(0.0);
    let (lo, hi) = (lo - pad, hi + pad);
    notes
        .iter()
        .filter_map(|n| {
            let s = n.start - shift;
            (s < len && s + n.length > 0).then(|| {
                let s0 = s.max(0);
                [s0 as f32 / len as f32, ((s + n.length).min(len) - s0) as f32 / len as f32, (hi - n.pitch as f32) / (hi - lo).max(1.0)]
            })
        })
        .collect()
}

/// Update a VecModel in place (row-wise) so Slint only re-renders changed rows.
pub(crate) fn sync_model<T: Clone + PartialEq + 'static>(m: &VecModel<T>, rows: Vec<T>) {
    if m.row_count() == rows.len() {
        for (i, r) in rows.into_iter().enumerate() {
            if m.row_data(i).as_ref() != Some(&r) {
                m.set_row_data(i, r);
            }
        }
    } else {
        m.set_vec(rows);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn slider_mapping_roundtrip() {
        for g in [0.0f32, 0.1, 0.5, 1.0, 1.5, 1.99] {
            let p = slider_from_gain(g);
            assert!((gain_from_slider(p) - g).abs() < 1e-3, "{g} -> {p}");
        }
        assert!((slider_from_gain(1.0) - 0.8).abs() < 1e-6);
    }
    #[test]
    fn pan_text_entry() {
        assert_eq!(parse_pan_text(" c "), Some(0.0));
        assert_eq!(parse_pan_text("30L"), Some(-0.3));
        assert_eq!(parse_pan_text("100 r"), Some(1.0));
        assert_eq!(parse_pan_text("250R"), Some(1.0));
        assert_eq!(parse_pan_text("-45"), Some(-0.45));
        assert_eq!(parse_pan_text("left"), None);
    }
    #[test]
    fn fader_mapping() {
        assert!((fader_from_db(0.0) - 0.2).abs() < 1e-4);
        assert!((fader_from_db(-24.0) - 0.78).abs() < 1e-4);
        assert!((db_from_fader(0.5) + 9.0).abs() < 1e-3);
        assert!((db_from_fader(fader_from_db(-30.0)) + 30.0).abs() < 1e-3);
        assert_eq!(fader_from_db(f32::NEG_INFINITY), 1.0);
        assert!(db_from_fader(1.0).is_infinite());
    }
    #[test]
    fn pan_text() {
        assert_eq!(fmt_pan(0.0), "C");
        assert_eq!(fmt_pan(-0.5), "50L");
        assert_eq!(fmt_pan(1.0), "100R");
    }
}
