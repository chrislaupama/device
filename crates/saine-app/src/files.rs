//! Browser › Files (Projects and Folders) and the browser's Back / Forward history.
//!
//! Projects lists the `.saineproj` folders in the projects folder; a project expands to the
//! tracks, audio files and MIDI clips it held when last saved. Folders lists folders the user
//! added (kept in the preferences) and the WAV files and subfolders inside them. Rows are laid
//! out here, in design units, so the UI only draws them (research-findings: findings/files.md).

use crate::controller::{Controller, ImportTarget, sync_model};
use crate::{FileGuide, FileRow, Studio};
use slint::Model;
use saine_core::{ClipId, Command, MidiClip, Project, Ticks, Track, TrackId, TrackKind, persist};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub const SECTION_DEVICES: usize = 2;
pub const SECTION_FILES: usize = 3;
pub const TAB_PROJECTS: usize = 0;
pub const TAB_FOLDERS: usize = 1;

/// Panel-relative y of the rule under the tabs (no count row) and under the count row.
pub const LIST_Y: f32 = 81.0;
pub const LIST_Y_META: f32 = 96.0;
/// Row metrics (the reference app px × 0.85): 24 px rows 2 px apart, 20 px group labels, 4 px list padding.
pub const ROW_H: f32 = 20.4;
pub const GAP: f32 = 1.7;
pub const LABEL_H: f32 = 17.0;
pub const PAD_TOP: f32 = 3.4;
pub const PAD_BOTTOM: f32 = 10.2;
/// Indent per depth and the inset of a row's content (chevron, then glyph and name).
pub const INDENT: f32 = 6.8;
pub const ROW_X: f32 = 10.2;

pub const AUDIO_EXT: &[&str] = &["wav"];

/// A browser location: section and that section's tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Loc {
    pub section: usize,
    pub tab: usize,
}

/// Back / Forward: visited locations. Visiting a new location drops the forward entries;
/// visiting the current one again adds nothing.
#[derive(Debug, Clone)]
pub struct Nav {
    history: Vec<Loc>,
    index: usize,
}

impl Nav {
    pub fn new(at: Loc) -> Self {
        Nav { history: vec![at], index: 0 }
    }
    pub fn current(&self) -> Loc {
        self.history[self.index]
    }
    pub fn push(&mut self, loc: Loc) {
        if self.current() == loc {
            return;
        }
        self.history.truncate(self.index + 1);
        self.history.push(loc);
        self.index = self.history.len() - 1;
    }
    pub fn back(&mut self) -> Option<Loc> {
        self.index = self.index.checked_sub(1)?;
        Some(self.current())
    }
    pub fn forward(&mut self) -> Option<Loc> {
        if self.index + 1 >= self.history.len() {
            return None;
        }
        self.index += 1;
        Some(self.current())
    }
    pub fn can_back(&self) -> bool {
        self.index > 0
    }
    pub fn can_forward(&self) -> bool {
        self.index + 1 < self.history.len()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum FileItem {
    Folder(PathBuf),
    Audio(PathBuf),
    Project(PathBuf),
    Track { project: PathBuf, track: TrackId },
    Midi { project: PathBuf, clip: ClipId },
}

impl FileItem {
    fn key(&self) -> String {
        match self {
            FileItem::Folder(p) | FileItem::Audio(p) | FileItem::Project(p) => p.to_string_lossy().into_owned(),
            FileItem::Track { project, track } => format!("{}#t{}", project.display(), track.0),
            FileItem::Midi { project, clip } => format!("{}#m{}", project.display(), clip.0),
        }
    }
    fn expandable(&self) -> bool {
        matches!(self, FileItem::Folder(_) | FileItem::Project(_))
    }
}

#[derive(Debug, Clone)]
pub struct FileDrag {
    pub item: FileItem,
    pub label: String,
    pub start: (f32, f32),
    pub pos: (f32, f32),
    pub active: bool,
}

#[derive(Debug)]
pub struct FilesUi {
    pub nav: Nav,
    pub section: usize,
    pub tab: usize,
    pub sort_name: bool,
    pub search: String,
    pub projects_dir: Option<PathBuf>,
    /// Projects found in `projects_dir` with their modification time.
    pub projects: Vec<(PathBuf, SystemTime)>,
    /// Expanded rows by key (`FileItem::key`).
    pub expanded: HashSet<String>,
    pub selected: Option<String>,
    pub scroll: f32,
    /// Items of the rows last pushed (None for labels and notes).
    pub items: Vec<Option<FileItem>>,
    pub content_h: f32,
    pub drag: Option<FileDrag>,
    listings: HashMap<PathBuf, Result<Vec<(String, bool)>, String>>,
    project_cache: HashMap<PathBuf, Result<Project, String>>,
}

impl Default for FilesUi {
    fn default() -> Self {
        let projects_dir = std::env::var_os("SAINE_PROJECTS_DIR")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Documents").join("Saine Studio")));
        FilesUi {
            nav: Nav::new(Loc { section: SECTION_DEVICES, tab: 0 }),
            section: SECTION_DEVICES,
            tab: TAB_PROJECTS,
            sort_name: false,
            search: String::new(),
            projects_dir,
            projects: Vec::new(),
            expanded: HashSet::new(),
            selected: None,
            scroll: 0.0,
            items: Vec::new(),
            content_h: 0.0,
            drag: None,
            listings: HashMap::new(),
            project_cache: HashMap::new(),
        }
    }
}

fn file_name(p: &Path) -> String {
    p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| p.display().to_string())
}

fn is_audio(p: &Path) -> bool {
    p.extension().and_then(|e| e.to_str()).is_some_and(|e| AUDIO_EXT.iter().any(|a| e.eq_ignore_ascii_case(a)))
}

/// Subfolders then WAV files of `dir`, by name; hidden entries skipped.
pub fn list_folder(dir: &Path) -> Result<Vec<(String, bool)>, String> {
    let mut out: Vec<(String, bool)> = std::fs::read_dir(dir)
        .map_err(|_| "That couldn’t be read.".to_string())?
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let dir = e.file_type().ok()?.is_dir() || e.path().is_dir();
            (!name.starts_with('.') && (dir || is_audio(&e.path()))).then_some((name, dir))
        })
        .collect();
    out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.to_lowercase().cmp(&b.0.to_lowercase())));
    Ok(out)
}

/// `.saineproj` folders in `dir` with their modification times.
pub fn scan_projects(dir: &Path) -> Vec<(PathBuf, SystemTime)> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    rd.filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_dir() && p.extension().is_some_and(|e| e == persist::EXTENSION))
        .map(|p| {
            let t = std::fs::metadata(p.join(persist::PROJECT_FILE)).or_else(|_| std::fs::metadata(&p)).and_then(|m| m.modified()).unwrap_or(SystemTime::UNIX_EPOCH);
            (p, t)
        })
        .collect()
}

/// One group under an expanded row: an optional label and its rows.
struct Group {
    label: Option<&'static str>,
    rows: Vec<(FileItem, String, String, i32)>,
}

enum Contents {
    Groups(Vec<Group>),
    Note(String),
}

/// Rows, guide lines and content height for the current state. Pure apart from the caches
/// it reads (folder listings and project files loaded on expand).
pub struct Built {
    pub rows: Vec<FileRow>,
    pub items: Vec<Option<FileItem>>,
    pub guides: Vec<FileGuide>,
    pub height: f32,
    /// Top-level rows shown.
    pub count: usize,
}

struct Builder<'a> {
    ui: &'a FilesUi,
    q: String,
    out: Built,
    y: f32,
}

impl Builder<'_> {
    fn push(&mut self, kind: i32, depth: i32, h: f32, label: String, detail: String, glyph: i32, expander: i32, item: Option<FileItem>) {
        let selected = item.as_ref().is_some_and(|i| self.ui.selected.as_deref() == Some(i.key().as_str()));
        self.out.rows.push(FileRow { kind, depth, y: self.y, h, label: label.into(), detail: detail.into(), glyph, expander, selected });
        self.out.items.push(item);
        self.y += h;
    }

    fn contents(&self, item: &FileItem) -> Option<Contents> {
        Some(match item {
            FileItem::Folder(dir) => match self.ui.listings.get(dir)? {
                Err(e) => Contents::Note(e.clone()),
                Ok(list) if list.is_empty() => Contents::Note("No audio files or folders here".into()),
                Ok(list) => Contents::Groups(vec![Group {
                    label: None,
                    rows: list
                        .iter()
                        .map(|(name, d)| {
                            let p = dir.join(name);
                            if *d {
                                (FileItem::Folder(p), name.clone(), String::new(), 0)
                            } else {
                                let ext = p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
                                (FileItem::Audio(p), name.clone(), ext, 1)
                            }
                        })
                        .collect(),
                }]),
            },
            FileItem::Project(dir) => match self.ui.project_cache.get(dir)? {
                Err(e) => Contents::Note(e.clone()),
                Ok(p) => {
                    let tracks = p.tracks.iter().map(|t| (FileItem::Track { project: dir.clone(), track: t.id }, t.name.clone(), String::new(), if t.kind == TrackKind::Instrument { 4 } else { 3 })).collect();
                    let audio = p.assets.iter().map(|a| {
                        let path = PathBuf::from(&a.absolute_path);
                        let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
                        (FileItem::Audio(path.clone()), file_name(&path), ext, 1)
                    });
                    let midi = p.tracks.iter().flat_map(|t| t.midi_clips.iter()).map(|c| (FileItem::Midi { project: dir.clone(), clip: c.id }, c.name.clone(), String::new(), 5)).collect();
                    let groups = vec![Group { label: Some("Tracks"), rows: tracks }, Group { label: Some("Audio files"), rows: audio.collect() }, Group { label: Some("MIDI clips"), rows: midi }];
                    if groups.iter().all(|g| g.rows.is_empty()) {
                        Contents::Note("Nothing in this project".into())
                    } else {
                        Contents::Groups(groups.into_iter().filter(|g| !g.rows.is_empty() || g.label == Some("Tracks")).collect())
                    }
                }
            },
            _ => return None,
        })
    }

    fn matches(&self, name: &str) -> bool {
        self.q.is_empty() || name.to_lowercase().contains(&self.q)
    }

    /// Whether `item` (named `name`) or anything loaded below it matches the search.
    fn visible(&self, item: &FileItem, name: &str, depth: usize) -> bool {
        if self.matches(name) {
            return true;
        }
        depth < 8
            && matches!(self.contents(item), Some(Contents::Groups(gs)) if gs.iter().any(|g| g.rows.iter().any(|(i, n, _, _)| self.visible(i, n, depth + 1))))
    }

    /// A row and, when open, its contents. `filter`: hide rows that neither match nor lead to a match.
    fn row_group(&mut self, item: FileItem, label: String, detail: String, glyph: i32, depth: i32, filter: bool) {
        let open = item.expandable() && (self.ui.expanded.contains(&item.key()) || (filter && !self.matches(&label)));
        let filter_below = filter && !self.matches(&label);
        let expander = if !item.expandable() { 0 } else if open { 2 } else { 1 };
        let contents = if open { self.contents(&item) } else { None };
        self.push(0, depth, ROW_H, label, detail, glyph, expander, Some(item));
        let x = ROW_X + depth as f32 * INDENT + 4.25;
        match contents {
            None => {}
            Some(Contents::Note(n)) => {
                self.y += GAP;
                self.push(2, depth, ROW_H, n, String::new(), 0, 0, None);
            }
            Some(Contents::Groups(groups)) => {
                self.y += GAP;
                let top = self.y;
                for (gi, g) in groups.into_iter().enumerate() {
                    let rows: Vec<_> = g.rows.into_iter().filter(|(i, n, _, _)| !filter_below || self.visible(i, n, 0)).collect();
                    if filter_below && rows.is_empty() {
                        continue;
                    }
                    if gi > 0 {
                        self.y += GAP + 3.4;
                    }
                    if let Some(l) = g.label {
                        self.push(1, depth, LABEL_H, l.into(), String::new(), 0, 0, None);
                        self.y += GAP;
                    }
                    if rows.is_empty() {
                        self.push(2, depth, ROW_H, "None".into(), String::new(), 0, 0, None);
                    }
                    for (ri, (i, n, d, gl)) in rows.into_iter().enumerate() {
                        if ri > 0 {
                            self.y += GAP;
                        }
                        self.row_group(i, n, d, gl, depth + 1, filter_below);
                    }
                }
                self.out.guides.push(FileGuide { x, y: top, h: self.y - top });
            }
        }
    }
}

/// Top-level items of the open tab in display order: Recent (projects by last save, folders
/// by last added) or Name.
fn top_items(ui: &FilesUi, folders: &[String]) -> Vec<(FileItem, String)> {
    let mut top: Vec<(FileItem, String)> = if ui.tab == TAB_PROJECTS {
        let mut ps = ui.projects.clone();
        ps.sort_by(|a, b| b.1.cmp(&a.1));
        ps.into_iter()
            .map(|(p, _)| {
                let name = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                (FileItem::Project(p), name)
            })
            .collect()
    } else {
        folders.iter().rev().map(|f| (FileItem::Folder(PathBuf::from(f)), file_name(Path::new(f)))).collect()
    };
    if ui.sort_name {
        top.sort_by_key(|(_, n)| n.to_lowercase());
    }
    top
}

/// Rows of the open tab. `folders`: the added folders, oldest first (preferences).
pub fn build(ui: &FilesUi, folders: &[String]) -> Built {
    let mut b = Builder { ui, q: ui.search.trim().to_lowercase(), out: Built { rows: Vec::new(), items: Vec::new(), guides: Vec::new(), height: 0.0, count: 0 }, y: PAD_TOP };
    let top = top_items(ui, folders);
    let filter = !b.q.is_empty();
    let mut first = true;
    for (item, name) in top {
        if filter && !b.visible(&item, &name, 0) {
            continue;
        }
        if !first {
            b.y += GAP;
        }
        first = false;
        b.out.count += 1;
        let glyph = if matches!(item, FileItem::Project(_)) { 2 } else { 0 };
        b.row_group(item, name, String::new(), glyph, 0, filter);
    }
    b.out.height = b.y + PAD_BOTTOM;
    b.out
}

const UNREADABLE: &str = "That couldn’t be read.";

impl Controller {
    fn browser_loc(&self, section: usize) -> Loc {
        Loc { section, tab: if section == SECTION_FILES { self.files.tab } else { self.devices.tab } }
    }

    /// Show `loc` without touching the history.
    fn browser_go(&mut self, loc: Loc) {
        self.files.section = loc.section;
        if loc.section == SECTION_FILES {
            self.files.tab = loc.tab.min(TAB_FOLDERS);
            self.files.scroll = 0.0;
            self.files_refresh();
        } else {
            self.devices.tab = loc.tab.min(2);
            self.devices.tree_scroll = 0.0;
            self.devices.selected = None;
        }
        self.ui_dirty = true;
    }

    pub(crate) fn browser_visit(&mut self, loc: Loc) {
        self.browser_go(loc);
        self.files.nav.push(loc);
    }

    /// Section tabs: Devices and Files (Search and Library have no content).
    pub fn browser_section(&mut self, section: usize) {
        if section == SECTION_DEVICES || section == SECTION_FILES {
            self.browser_visit(self.browser_loc(section));
        }
    }

    pub fn browser_back(&mut self) {
        if let Some(loc) = self.files.nav.back() {
            self.browser_go(loc);
        }
    }

    pub fn browser_forward(&mut self) {
        if let Some(loc) = self.files.nav.forward() {
            self.browser_go(loc);
        }
    }

    pub fn files_tab(&mut self, tab: usize) {
        self.browser_visit(Loc { section: SECTION_FILES, tab: tab.min(TAB_FOLDERS) });
    }

    /// Rescan the projects folder and reread expanded folders and projects.
    pub fn files_refresh(&mut self) {
        self.files.projects = self.files.projects_dir.as_deref().map(scan_projects).unwrap_or_default();
        let folders: Vec<PathBuf> = self.files.listings.drain().map(|(d, _)| d).collect();
        let projects: Vec<PathBuf> = self.files.project_cache.drain().map(|(p, _)| p).collect();
        for item in folders.into_iter().map(FileItem::Folder).chain(projects.into_iter().map(FileItem::Project)) {
            self.files_load(&item);
        }
    }

    fn files_load(&mut self, item: &FileItem) {
        match item {
            FileItem::Folder(d) => {
                self.files.listings.insert(d.clone(), list_folder(d));
            }
            FileItem::Project(p) => {
                self.files.project_cache.insert(p.clone(), persist::load(p).map(|l| l.project).map_err(|_| UNREADABLE.to_string()));
            }
            _ => {}
        }
    }

    pub fn files_sort_toggle(&mut self) {
        self.files.sort_name = !self.files.sort_name;
        self.ui_dirty = true;
    }

    /// Add a folder to Folders (most recent first) and show it.
    pub fn files_add_folder(&mut self, dir: &Path) {
        let s = dir.to_string_lossy().into_owned();
        self.settings.prefs.browser_folders.retain(|f| *f != s);
        self.settings.prefs.browser_folders.push(s);
        self.save_prefs();
        self.files.search.clear();
        self.browser_visit(Loc { section: SECTION_FILES, tab: TAB_FOLDERS });
    }

    /// Click: select; a folder (anywhere) or a project (its chevron) opens or closes.
    pub fn files_click(&mut self, i: usize, chevron: bool) {
        let Some(Some(item)) = self.files.items.get(i).cloned() else { return };
        let key = item.key();
        self.files.selected = Some(key.clone());
        if matches!(item, FileItem::Folder(_)) || (chevron && item.expandable()) {
            if !self.files.expanded.remove(&key) {
                self.files.expanded.insert(key);
                self.files_load(&item);
            }
        }
        self.ui_dirty = true;
    }

    /// Open `item` (a folder or project) without selecting it.
    pub fn files_expand(&mut self, item: FileItem) {
        self.files.expanded.insert(item.key());
        self.files_load(&item);
        self.ui_dirty = true;
    }

    /// Double-click: open a project, add a track, audio file or MIDI clip as a new track.
    pub fn files_activate(&mut self, i: usize) {
        let Some(Some(item)) = self.files.items.get(i).cloned() else { return };
        match item {
            FileItem::Folder(_) => {}
            FileItem::Project(p) => self.open(&p),
            FileItem::Audio(p) => self.import_files(vec![p], ImportTarget::NewTrack, None),
            FileItem::Track { project, track } => self.files_add_track(&project, track, None),
            FileItem::Midi { project, clip } => self.files_add_midi(&project, clip, None, None),
        }
    }

    /// Copy `track` of the (loaded) project at `project` into this one as a new track.
    pub fn files_add_track(&mut self, project: &Path, track: TrackId, index: Option<usize>) {
        let Some(Ok(src)) = self.files.project_cache.get(project) else { return };
        let mut next = self.project().next_id;
        let Some((t, assets)) = saine_core::import::import_track(src, track, &mut next) else { return };
        let id = t.id;
        let mut commands: Vec<Command> = assets.iter().cloned().map(|asset| Command::AddAsset { asset }).collect();
        commands.push(Command::AddTrack { index, track: t });
        if self.execute(Command::Batch { label: "Add Track".into(), commands }) {
            for a in &assets {
                self.load_asset(a);
            }
            self.select_track(id);
        }
    }

    /// Add MIDI clip `clip` of the (loaded) project at `project` to instrument track `to`
    /// (else a new instrument track) at `at` (default: the playhead).
    pub fn files_add_midi(&mut self, project: &Path, clip: ClipId, to: Option<TrackId>, at: Option<Ticks>) {
        let Some(Ok(src)) = self.files.project_cache.get(project) else { return };
        let Some(c) = src.tracks.iter().flat_map(|t| t.midi_clips.iter()).find(|c| c.id == clip).cloned() else { return };
        let at = at.unwrap_or_else(|| self.playhead_ticks()).max(0);
        let p = self.project();
        let mut next = p.next_id;
        let mut commands = Vec::new();
        let track = match to.and_then(|t| p.track(t)).filter(|t| t.kind == TrackKind::Instrument) {
            Some(t) => t.id,
            None => {
                let id = TrackId(next);
                next += 1;
                commands.push(Command::AddTrack { index: None, track: Track::new(id, TrackKind::Instrument, "Instrument", crate::controller::next_track_color(p)) });
                id
            }
        };
        commands.push(Command::AddMidiClip { track, clip: MidiClip { id: ClipId(next), start: at, ..c } });
        if self.execute(Command::Batch { label: "Add MIDI Clip".into(), commands }) {
            self.select_track(track);
        }
    }

    pub fn files_scroll_by(&mut self, dy: f32) {
        let view = self.layout().top_h() - self.files_list_y() - 36.0;
        let max = (self.files.content_h - view).max(0.0);
        self.files.scroll = (self.files.scroll - dy).clamp(0.0, max);
        self.ui_dirty = true;
    }

    /// Panel-relative y of the rule above the rows.
    pub fn files_list_y(&self) -> f32 {
        if top_items(&self.files, &self.settings.prefs.browser_folders).is_empty() { LIST_Y } else { LIST_Y_META }
    }

    /// Row drag: phase 0 press, 1 move, 2 release (window coordinates).
    pub fn files_drag(&mut self, i: usize, phase: i32, x: f32, y: f32) {
        match phase {
            0 => {
                self.files.drag = self.files.items.get(i).cloned().flatten().filter(|it| !it.expandable()).map(|item| {
                    let label = self.files_model.row_data(i).map(|r| r.label.to_string()).unwrap_or_default();
                    FileDrag { item, label, start: (x, y), pos: (x, y), active: false }
                });
            }
            1 => {
                if let Some(d) = self.files.drag.as_mut() {
                    d.pos = (x, y);
                    if !d.active && ((x - d.start.0).abs() > 4.0 || (y - d.start.1).abs() > 4.0) {
                        d.active = true;
                    }
                    if d.active {
                        self.ui_dirty = true;
                    }
                }
            }
            _ => {
                let Some(d) = self.files.drag.take() else { return };
                self.ui_dirty = true;
                if !d.active {
                    return;
                }
                let Some((track, at)) = self.files_drop_at(x, y) else { return };
                let p = self.project();
                let index = track.and_then(|t| p.track_index(t)).map(|i| i + 1);
                match d.item {
                    FileItem::Audio(path) => {
                        let target = track.filter(|t| p.track(*t).is_some_and(|t| t.kind == TrackKind::Audio)).map_or(ImportTarget::NewTrack, ImportTarget::Track);
                        self.import_files(vec![path], target, Some(at));
                    }
                    FileItem::Track { project, track } => self.files_add_track(&project, track, index),
                    FileItem::Midi { project, clip } => self.files_add_midi(&project, clip, track, Some(at)),
                    _ => {}
                }
            }
        }
    }

    /// Over the lanes: the track under (x, y) (None below the last track) and the beat there.
    pub fn files_drop_at(&self, x: f32, y: f32) -> Option<(Option<TrackId>, Ticks)> {
        let l = self.layout();
        let (tx, tw) = (l.timeline_x(), l.timeline_w());
        let (top, bottom) = (78.0 + 66.5, 78.0 + l.top_h() - crate::timeline::MASTER_H);
        if x < tx || x >= tx + tw || y < top || y >= bottom {
            return None;
        }
        let track = self.view.track_at(y - top + crate::timeline::RULER_H).and_then(|i| self.project().tracks.get(i)).map(|t| t.id);
        let beat = self.view.beat_at(x - tx).max(0.0).round();
        Some((track, (beat * saine_core::PPQ as f64) as Ticks))
    }

    pub(crate) fn push_files(&mut self, s: &Studio) {
        let built = build(&self.files, &self.settings.prefs.browser_folders);
        let list_y = self.files_list_y();
        self.files.items = built.items;
        self.files.content_h = built.height;
        let view = self.layout().top_h() - list_y - 36.0;
        self.files.scroll = self.files.scroll.clamp(0.0, (built.height - view).max(0.0));
        sync_model(&self.files_model, built.rows);
        sync_model(&self.files_guides_model, built.guides);
        s.set_browser_section(self.files.section as i32);
        s.set_browser_can_back(self.files.nav.can_back());
        s.set_browser_can_forward(self.files.nav.can_forward());
        s.set_files_tab(self.files.tab as i32);
        s.set_files_scroll(self.files.scroll);
        s.set_files_list_y(list_y);
        let total = top_items(&self.files, &self.settings.prefs.browser_folders).len();
        let (one, many) = if self.files.tab == TAB_PROJECTS { ("project", "projects") } else { ("folder", "folders") };
        s.set_files_meta(if total == 0 { String::new() } else { format!("{} {}", built.count, if built.count == 1 { one } else { many }) }.into());
        s.set_files_sort(if self.files.sort_name { "Name" } else { "Recent" }.into());
        let q = self.files.search.trim();
        let (title, hint) = if total == 0 && self.files.tab == TAB_PROJECTS {
            ("No projects yet".to_string(), "Projects you save show up here.")
        } else if total == 0 {
            ("No folders yet".to_string(), "Add a folder of your own samples to browse it here.")
        } else if built.count == 0 {
            (format!("No files match “{q}”"), "")
        } else {
            (String::new(), "")
        };
        s.set_files_empty_title(title.into());
        s.set_files_empty_hint(hint.into());
        if self.files.section == SECTION_FILES && s.get_browser_search().as_str() != self.files.search {
            s.set_browser_search(self.files.search.clone().into());
        }
        if let Some(d) = self.files.drag.as_ref().filter(|d| d.active) {
            let drop = self.files_drop_at(d.pos.0, d.pos.1);
            s.set_dnd_active(true);
            s.set_dnd_x(d.pos.0);
            s.set_dnd_y(d.pos.1);
            s.set_dnd_label(d.label.clone().into());
            s.set_dnd_ok(drop.is_some());
            let rect = drop.and_then(|(t, _)| t).and_then(|t| self.track_rect(t));
            s.set_dnd_target_visible(rect.is_some());
            if let Some((x, y, w, h)) = rect {
                s.set_dnd_tx(x);
                s.set_dnd_ty(y);
                s.set_dnd_tw(w);
                s.set_dnd_th(h);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loc(section: usize, tab: usize) -> Loc {
        Loc { section, tab }
    }

    #[test]
    fn nav_drops_forward_entries_and_skips_repeats() {
        let mut n = Nav::new(loc(2, 0));
        assert!(!n.can_back() && !n.can_forward());
        n.push(loc(2, 0));
        assert!(!n.can_back());
        n.push(loc(3, 0));
        n.push(loc(3, 1));
        assert_eq!(n.back(), Some(loc(3, 0)));
        assert_eq!(n.back(), Some(loc(2, 0)));
        assert_eq!(n.back(), None);
        assert_eq!(n.forward(), Some(loc(3, 0)));
        assert!(n.can_forward());
        n.push(loc(2, 1));
        assert!(!n.can_forward());
        assert_eq!(n.back(), Some(loc(3, 0)));
        assert_eq!(n.forward(), Some(loc(2, 1)));
        assert_eq!(n.forward(), None);
    }

    #[test]
    fn folders_list_wavs_and_subfolders_and_search_finds_nested_files() {
        let dir = std::env::temp_dir().join(format!("saine-files-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("kit/drums")).unwrap();
        for f in ["kit/b.wav", "kit/A.WAV", "kit/notes.txt", "kit/.hidden.wav", "kit/drums/kick.wav"] {
            std::fs::write(dir.join(f), b"").unwrap();
        }
        let kit = dir.join("kit");
        assert_eq!(list_folder(&kit).unwrap(), vec![("drums".to_string(), true), ("A.WAV".to_string(), false), ("b.wav".to_string(), false)]);
        let mut ui = FilesUi { tab: TAB_FOLDERS, projects_dir: None, ..Default::default() };
        let folders = vec![dir.join("other").to_string_lossy().into_owned(), kit.to_string_lossy().into_owned()];
        let b = build(&ui, &folders);
        assert_eq!(b.rows.iter().map(|r| r.label.to_string()).collect::<Vec<_>>(), ["kit", "other"]);
        ui.sort_name = true;
        ui.expanded.insert(kit.to_string_lossy().into_owned());
        ui.listings.insert(kit.clone(), list_folder(&kit));
        ui.listings.insert(dir.join("other"), list_folder(&dir.join("other")));
        let b = build(&ui, &folders);
        let labels: Vec<_> = b.rows.iter().map(|r| (r.kind, r.depth, r.label.to_string(), r.detail.to_string())).collect();
        assert_eq!(labels[0], (0, 0, "kit".into(), String::new()));
        assert_eq!(labels[1], (0, 1, "drums".into(), String::new()));
        assert_eq!(labels[2], (0, 1, "A.WAV".into(), "wav".into()));
        assert_eq!(labels[4].2, "other");
        assert!((b.rows[1].y - (PAD_TOP + ROW_H + GAP)).abs() < 1e-3);
        assert!((b.rows[2].y - b.rows[1].y - ROW_H - GAP).abs() < 1e-3);
        assert_eq!(b.guides.len(), 1);
        // Search: "kick" sits in the unopened "drums" once its listing is loaded.
        ui.listings.insert(kit.join("drums"), list_folder(&kit.join("drums")));
        ui.search = "KICK".into();
        let b = build(&ui, &folders);
        assert_eq!(b.rows.iter().map(|r| r.label.to_string()).collect::<Vec<_>>(), ["kit", "drums", "kick.wav"]);
        assert_eq!(b.count, 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn project_rows_group_tracks_audio_and_midi() {
        let dir = std::env::temp_dir().join(format!("saine-files-proj-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("Song.saineproj");
        let mut p = Project::default();
        p.tracks[1].midi_clips.push(MidiClip::empty(0, 4 * saine_core::PPQ));
        persist::save(&p, &path).unwrap();
        std::fs::create_dir_all(dir.join("not-a-project")).unwrap();
        let mut ui = FilesUi { projects_dir: Some(dir.clone()), ..Default::default() };
        ui.projects = scan_projects(&dir);
        assert_eq!(ui.projects.len(), 1);
        ui.expanded.insert(path.to_string_lossy().into_owned());
        ui.project_cache.insert(path.clone(), persist::load(&path).map(|l| l.project).map_err(|e| e.to_string()));
        let b = build(&ui, &[]);
        let labels: Vec<_> = b.rows.iter().map(|r| (r.kind, r.label.to_string())).collect();
        assert_eq!(labels, [(0, "Song".into()), (1, "Tracks".into()), (0, "Audio".into()), (0, "Instrument".into()), (1, "MIDI clips".into()), (0, "MIDI".into())]);
        assert!(matches!(b.items[2], Some(FileItem::Track { .. })));
        assert!(matches!(b.items[5], Some(FileItem::Midi { .. })));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
