//! Clip-level editing: insert MIDI clip (⌘⇧M), duplicate (⌘D), the internal
//! clipboard (⌘X / ⌘C / ⌘V), rename, the clip context menu, and pushing the
//! Clip tab / piano-roll view models to Slint. Every edit is one Command.

use crate::controller::{BottomTab, ClipboardItem, Controller, MenuAction, sync_model, track_color};
use crate::pianoroll::{self, GRID_Y, ROW_H};
use crate::{MenuItem, PatternRow, PreviewNote, RulerLabel, SeqCell, ShapeRow, SeqNum, SeqRow, Studio, YLabel};
use dstudio_core::midi::{self, MidiClip};
use dstudio_core::steps::StepPattern;
use dstudio_core::{ClipId, Command, PPQ, Ticks, TrackKind};

/// Context-menu geometry (window points).
pub const MENU_W: f32 = 209.0;
pub const MENU_PAD: f32 = 3.4;
pub const MENU_ROW: f32 = 20.4;
pub const MENU_SEP: f32 = 10.6;

/// Clip editor dropdowns, in `Studio.clip-menu` order: Speed, Warp, Mode (audio toolbar),
/// Grid (piano roll footer), Type, Genre (clip sidebar). Speed, Warp and Mode edit the clip's
/// warp (`dstudio_core::warp`, presets and modes in the same order); Type and Genre pick the
/// sidebar's library part (dstudio_core::library::GENRES has a feel for each).
/// Genre lists a fixed set of entries.
const CLIP_MENUS: [&[&str]; 10] = [
    &["0.5x", "0.75x", "1x", "1.25x", "1.5x", "2x"],
    &["On", "Off"],
    &["Auto", "Drums", "Melody", "Vocal", "Re-pitch"],
    &["1/4", "1/8", "1/16", "1/32", "1/4T", "1/8T", "1/16T", "1/32T"],
    &["Chords", "Bass", "Lead", "Arp"],
    &dstudio_core::library::GENRES,
    &["8", "12", "16", "24", "32", "36", "48", "64"],
    &["1/1", "1/2", "1/4", "1/8", "1/16", "1/32", "1/8T", "1/16T", "1/32T"],
    &dstudio_core::scale::ROOTS,
    &dstudio_core::scale::SCALES,
];
const GRID_DENOMS: [u32; 8] = [4, 8, 16, 32, 6, 12, 24, 48];

fn grid_label(denominator: u32) -> String {
    GRID_DENOMS.iter().position(|&d| d == denominator).map_or(format!("1/{denominator}"), |i| CLIP_MENUS[3][i].to_owned())
}

fn slint_color(c: [f32; 3]) -> slint::Color {
    slint::Color::from_rgb_f32(c[0], c[1], c[2])
}

impl MenuAction {
    fn label(self) -> &'static str {
        match self {
            MenuAction::Cut => "Cut",
            MenuAction::Copy => "Copy",
            MenuAction::Duplicate => "Duplicate",
            MenuAction::Split => "Split",
            MenuAction::Consolidate => "Consolidate",
            MenuAction::Bounce => "Bounce",
            MenuAction::Delete => "Delete",
            MenuAction::Disable => "Disable",
            MenuAction::Enable => "Enable",
            MenuAction::ShowInFinder => "Show in Finder",
            MenuAction::Rename => "Rename…",
            MenuAction::Export => "Export audio to Finder",
            MenuAction::Transpose => "Transpose",
            MenuAction::Quantize => "Quantize",
            MenuAction::Humanize => "Humanize",
            MenuAction::ToPattern => "Convert to Step Pattern",
            MenuAction::ToMidi => "Convert to MIDI Clip",
            MenuAction::Ask => "Ask",
            MenuAction::Paste => "Paste",
            MenuAction::SectionRename => "Rename…",
            MenuAction::SectionColor => "Color",
            MenuAction::SectionInsert => "Insert",
            MenuAction::SectionDelete => "Delete section",
            MenuAction::SectionClear => "Clear all sections",
        }
    }
    fn shortcut(self) -> &'static str {
        match self {
            MenuAction::Cut => "⌘ X",
            MenuAction::Copy => "⌘ C",
            MenuAction::Duplicate => "⌘ D",
            MenuAction::Split => "⌘ E",
            MenuAction::Consolidate => "⌘ J",
            MenuAction::Delete => "Del",
            MenuAction::Disable | MenuAction::Enable => "0",
            MenuAction::Ask => "⌘ K",
            MenuAction::Paste => "⌘ V",
            MenuAction::SectionDelete => "Del",
            _ => "",
        }
    }
    /// A separator is drawn above these entries.
    fn separated(self) -> bool {
        matches!(
            self,
            MenuAction::Delete | MenuAction::Disable | MenuAction::Enable | MenuAction::ShowInFinder | MenuAction::Transpose | MenuAction::Ask | MenuAction::SectionRename | MenuAction::SectionDelete | MenuAction::SectionClear
        )
    }
    fn submenu(self) -> bool {
        matches!(self, MenuAction::Bounce | MenuAction::Transpose | MenuAction::SectionColor | MenuAction::SectionInsert)
    }
}

/// Bounce submenu: (label, shortcut text, enabled). Two entries;
/// DStudio builds Bounce to new track. Bounce track in place needs a track that changes kind (MIDI to
/// audio, its instrument and devices removed), which the model cannot do yet, so it stays greyed.
const BOUNCE: [(&str, &str, bool); 2] = [("Bounce to new track", "⌘ B", true), ("Bounce track in place", "", false)];

/// Transpose submenu: (label, shortcut text, semitones).
const TRANSPOSE: [(&str, &str, i32); 4] = [("Octave up", "+12", 12), ("Semitone up", "+1", 1), ("Semitone down", "−1", -1), ("Octave down", "−12", -12)];

fn layout_items(items: impl Iterator<Item = (String, String, bool, bool, bool)>) -> (Vec<MenuItem>, f32) {
    let mut y = MENU_PAD;
    let mut out = Vec::new();
    for (i, (label, shortcut, enabled, separator, submenu)) in items.enumerate() {
        let separator = separator && i > 0;
        if separator {
            y += MENU_SEP;
        }
        out.push(MenuItem { label: label.into(), shortcut: shortcut.into(), enabled, separator, submenu, y });
        y += MENU_ROW;
    }
    (out, y + MENU_PAD)
}

impl Controller {
    // ---------------------------------------------------------------- clip commands

    /// ⌘⇧M: 1-bar empty MIDI clip at the playhead (snapped to the beat) on the
    /// selected instrument track. Opens the Clip tab (the clip is not selected).
    pub fn insert_midi_clip(&mut self) -> Option<ClipId> {
        let p = self.project();
        let Some(t) = self.selected_track.and_then(|t| p.track(t)) else {
            self.set_status("Select an instrument track to insert a MIDI clip", 3);
            return None;
        };
        if t.kind != TrackKind::Instrument {
            self.set_status("MIDI clips go on instrument tracks", 3);
            return None;
        }
        let track = t.id;
        let rack = t.instrument_plugin.as_ref().is_some_and(crate::stepseq::is_drum_rack);
        let start = ((self.playhead_ticks() as f64 / PPQ as f64).round() as Ticks * PPQ).max(0);
        let id = ClipId(p.next_id);
        // On a Drum Rack track the new clip is a one-bar drum pattern.
        let mut clip = if rack { MidiClip::empty_pattern(start, dstudio_core::steps::StepRows::Pads) } else { MidiClip::empty(start, p.ticks_per_bar()) };
        clip.id = id;
        if self.execute(Command::AddMidiClip { track, clip }) {
            self.set_bottom_tab(BottomTab::Clip);
            return Some(id);
        }
        None
    }

    /// ⌘R: reverse the selected clip. An audio clip plays backwards (pressing again plays it
    /// forwards); a MIDI clip's notes are mirrored in the clip, so the first note ends where the
    /// clip ends. A step pattern is left alone (its steps are reversed in the step editor).
    pub fn reverse_clip(&mut self) {
        let Some(id) = self.selected_clip else { return };
        let p = self.project();
        if let Some(m) = p.midi_clip(id) {
            if m.pattern.is_some() {
                self.set_status("Reverse: convert the pattern to MIDI first", 3);
                return;
            }
            let notes = reversed_in_clip(&m.notes, m.length, m.clip_loop.as_ref());
            if notes != m.notes {
                self.execute(Command::Batch { label: "Reverse Clip".into(), commands: vec![Command::SetMidiNotes { clip: id, notes }] });
            }
        } else if let Some(c) = p.clip(id) {
            let warp = dstudio_core::ClipWarp { reverse: !c.warp.reverse, ..c.warp.clone() };
            self.set_clip_warp(id, warp, "Reverse Clip");
        }
    }

    /// ⌘D: copy of the selected clip right after itself (MIDI or audio). Selects the copy.
    pub fn duplicate_clip(&mut self) {
        let Some(id) = self.selected_clip else { return };
        let p = self.project();
        let new_id = ClipId(p.next_id);
        let cmd = if let Some((ti, ci)) = p.find_midi_clip(id) {
            let c = &p.tracks[ti].midi_clips[ci];
            Command::AddMidiClip { track: p.tracks[ti].id, clip: MidiClip { id: new_id, start: c.end(), ..c.clone() } }
        } else if let Some((ti, ci)) = p.find_clip(id) {
            let c = &p.tracks[ti].clips[ci];
            let len = p.clip_length_ticks(c);
            Command::AddClip { track: p.tracks[ti].id, clip: dstudio_core::Clip { id: new_id, start: c.start + len, ..c.clone() } }
        } else {
            return;
        };
        if let Command::AddMidiClip { .. } | Command::AddClip { .. } = cmd
            && self.execute(Command::Batch { label: "Duplicate Clip".into(), commands: vec![cmd] })
        {
            self.select_clip(Some(new_id));
        }
    }

    /// Whether the playhead lies inside clip id, which can be split there (audio
    /// clips that are not warped or reversed, and MIDI clips that are not patterns).
    pub(crate) fn can_split(&self, id: ClipId) -> bool {
        let p = self.project();
        let at = self.playhead_ticks();
        if let Some((ti, ci)) = p.find_clip(id) {
            let c = &p.tracks[ti].clips[ci];
            return !c.warp.enabled && !c.warp.reverse && at > c.start && at < c.start + p.clip_length_ticks(c);
        }
        p.midi_clip(id).is_some_and(|c| c.pattern.is_none() && at > c.start && at < c.end())
    }

    /// ⌘E: cut the selected clip in two at the playhead and select the right part.
    pub fn split_clip(&mut self) {
        let Some(id) = self.selected_clip else { return };
        if !self.can_split(id) {
            self.set_status("Move the playhead inside an audio clip (no warp or reverse) or a MIDI clip to split it", 3);
            return;
        }
        let new_id = ClipId(self.project().next_id);
        let at = self.playhead_ticks();
        if self.execute(Command::SplitClip { clip: id, at, new_id }) {
            self.select_clip(Some(new_id));
        }
    }

    /// 0: disable the selected clip (silent, stays on the timeline), or enable it again.
    pub fn toggle_clip_disabled(&mut self) {
        let Some(id) = self.selected_clip else { return };
        let p = self.project();
        let disabled = p.find_clip(id).map(|(t, c)| p.tracks[t].clips[c].disabled).or_else(|| p.midi_clip(id).map(|c| c.disabled));
        if let Some(disabled) = disabled {
            self.execute(Command::SetClipEnabled { clip: id, enabled: disabled });
        }
    }

    /// Reveal the selected audio clip's source file in Finder.
    pub fn show_clip_in_finder(&mut self) {
        let Some(id) = self.selected_clip else { return };
        let p = self.project();
        let Some(path) = p.find_clip(id).and_then(|(t, c)| p.asset(p.tracks[t].clips[c].asset)).map(|a| a.absolute_path.clone()) else { return };
        if !std::path::Path::new(&path).exists() {
            self.set_status("The audio file is missing", 3);
            return;
        }
        if let Err(e) = crate::clip_render::reveal(&path) {
            self.set_status(format!("Show in Finder failed: {e}"), 3);
        }
    }

    pub fn copy_clip(&mut self) -> bool {
        let Some(id) = self.selected_clip else { return false };
        let p = self.project();
        self.clipboard = if let Some((ti, ci)) = p.find_midi_clip(id) {
            Some(ClipboardItem::Midi { track: p.tracks[ti].id, clip: p.tracks[ti].midi_clips[ci].clone() })
        } else if let Some((ti, ci)) = p.find_clip(id) {
            Some(ClipboardItem::Audio { track: p.tracks[ti].id, clip: p.tracks[ti].clips[ci].clone() })
        } else {
            None
        };
        self.clipboard.is_some()
    }

    pub fn cut_clip(&mut self) {
        if self.copy_clip() {
            self.delete_selection();
        }
    }

    /// ⌘V: paste at the playhead (beat-snapped) on the selected track when its kind
    /// matches, else on the clip's original track.
    pub fn paste_clip(&mut self) {
        let Some(item) = self.clipboard.clone() else { return };
        let p = self.project();
        let at = ((self.playhead_ticks() as f64 / PPQ as f64).round() as Ticks * PPQ).max(0);
        let new_id = ClipId(p.next_id);
        let (kind, orig) = match &item {
            ClipboardItem::Audio { track, .. } => (TrackKind::Audio, *track),
            ClipboardItem::Midi { track, .. } => (TrackKind::Instrument, *track),
        };
        let track = self.selected_track.and_then(|t| p.track(t)).filter(|t| t.kind == kind).map(|t| t.id).or_else(|| p.track(orig).map(|t| t.id));
        let Some(track) = track else {
            self.set_status("Paste: no matching track", 3);
            return;
        };
        let cmd = match item {
            ClipboardItem::Audio { clip, .. } => Command::AddClip { track, clip: dstudio_core::Clip { id: new_id, start: at, ..clip } },
            ClipboardItem::Midi { clip, .. } => Command::AddMidiClip { track, clip: MidiClip { id: new_id, start: at, ..clip } },
        };
        if self.execute(Command::Batch { label: "Paste Clip".into(), commands: vec![cmd] }) {
            self.select_clip(Some(new_id));
        }
    }

    /// Rename = remove + re-add with the same id in one Batch (core has no RenameClip yet).
    pub fn rename_clip(&mut self, id: ClipId, name: &str) {
        let name = name.trim();
        self.rename = None;
        self.ui_dirty = true;
        if name.is_empty() {
            return;
        }
        let p = self.project();
        let cmds = if let Some((ti, ci)) = p.find_midi_clip(id) {
            let c = &p.tracks[ti].midi_clips[ci];
            if c.name == name {
                return;
            }
            vec![Command::RemoveMidiClip { clip: id }, Command::AddMidiClip { track: p.tracks[ti].id, clip: MidiClip { name: name.into(), ..c.clone() } }]
        } else if let Some((ti, ci)) = p.find_clip(id) {
            let c = &p.tracks[ti].clips[ci];
            if c.name == name {
                return;
            }
            vec![Command::RemoveClip { clip: id }, Command::AddClip { track: p.tracks[ti].id, clip: dstudio_core::Clip { name: name.into(), ..c.clone() } }]
        } else {
            return;
        };
        self.execute(Command::Batch { label: "Rename Clip".into(), commands: cmds });
    }

    /// Whole-clip MIDI transforms from the context menu.
    fn menu_midi(&mut self, id: ClipId, f: impl FnOnce(&Self, &MidiClip) -> Option<Vec<dstudio_core::Note>>, label: &str) {
        let Some(c) = self.project().midi_clip(id).cloned() else { return };
        if c.notes.is_empty() {
            self.set_status(format!("{label}: the clip has no notes"), 2);
            return;
        }
        match f(self, &c) {
            Some(mut notes) => {
                midi::sort_notes(&mut notes);
                if notes != c.notes {
                    self.execute(Command::Batch { label: label.into(), commands: vec![Command::SetMidiNotes { clip: id, notes }] });
                }
            }
            None => self.set_status(format!("{label}: notes would leave the MIDI range"), 3),
        }
    }

    // ---------------------------------------------------------------- context menu

    /// Right-click on the lanes (lane-local x/y for hit-testing, window wx/wy for placement).
    pub fn lane_context(&mut self, x: f32, y: f32, wx: f32, wy: f32, win: (f32, f32)) {
        use crate::timeline::{Hit, hit_test};
        if self.section_context(x, y, wx, wy, win) {
            return;
        }
        let hit = hit_test(&self.view, self.lanes_size, self.project().tracks.len(), &self.clip_vis, x, y);
        let Hit::Clip { index, .. } = hit else {
            self.menu_close();
            return;
        };
        let Some(&id) = self.clip_ids.get(index) else { return };
        self.open_clip_menu(id, wx, wy, win);
    }

    /// Select clip `id` and open its context menu at window point (wx, wy).
    pub fn open_clip_menu(&mut self, id: ClipId, wx: f32, wy: f32, win: (f32, f32)) {
        if let Some(t) = self.clip_track(id) {
            self.select_track(t);
        }
        self.select_clip(Some(id));
        let p = self.project();
        let midi = p.find_midi_clip(id).is_some();
        let pattern = p.midi_clip(id).is_some_and(|c| c.pattern.is_some());
        let disabled = p.find_clip(id).map(|(t, c)| p.tracks[t].clips[c].disabled).or_else(|| p.midi_clip(id).map(|c| c.disabled)).unwrap_or(false);
        let splittable = self.can_split(id);
        use MenuAction::*;
        let mut items = vec![Cut, Copy, Duplicate, Split, Consolidate, Bounce, Delete, if disabled { Enable } else { Disable }, Rename, ShowInFinder, Export, Transpose, Quantize, Humanize];
        if midi {
            items.push(if pattern { ToMidi } else { ToPattern });
        }
        items.push(Ask);
        // PLACEHOLDER entry (disabled): Ask.
        self.menu.items = items
            .iter()
            .map(|&a| {
                let enabled = match a {
                    Cut | Copy | Duplicate | Delete | Rename | Disable | Enable => true,
                    Split => splittable,
                    ShowInFinder => !midi,
                    Consolidate => self.can_render_clip(id, false),
                    Bounce | Export => self.can_render_clip(id, true),
                    Transpose | Quantize | Humanize | ToPattern | ToMidi => midi,
                    _ => false,
                };
                (a, enabled)
            })
            .collect();
        let items = std::mem::take(&mut self.menu.items);
        self.open_menu(items, wx, wy, win);
        self.menu.clip = Some(id);
    }

    /// Open the context menu with `items` at window point (wx, wy), kept inside the window.
    pub fn open_menu(&mut self, items: Vec<(MenuAction, bool)>, wx: f32, wy: f32, win: (f32, f32)) {
        self.menu.items = items;
        let (_, h) = layout_items(self.menu.items.iter().map(|&(a, en)| (String::new(), String::new(), en, a.separated(), false)));
        let mx = (wx + 2.0).min(win.0 - MENU_W - 4.0).max(4.0);
        let my = (wy + 2.0).min(win.1 - h - 4.0).max(4.0);
        self.menu.open = true;
        self.menu.sub_open = false;
        self.menu.pos = (mx, my);
        self.menu.clip = None;
        self.menu.section = None;
        self.ui_dirty = true;
    }

    pub fn menu_close(&mut self) {
        if self.menu.open {
            self.menu.open = false;
            self.menu.sub_open = false;
            self.menu.clip = None;
            self.menu.section = None;
            self.ui_dirty = true;
        }
    }

    pub fn menu_activate(&mut self, index: usize) {
        let Some(&(action, enabled)) = self.menu.items.get(index) else { return };
        if !enabled {
            return;
        }
        if let Some(id) = self.menu.section {
            self.section_menu_activate(id, action);
            return;
        }
        let Some(id) = self.menu.clip else { return };
        if matches!(action, MenuAction::Transpose | MenuAction::Bounce) {
            self.menu.sub_open = !(self.menu.sub_open && self.menu.sub_for == Some(action));
            self.menu.sub_for = Some(action);
            self.ui_dirty = true;
            return;
        }
        self.menu_close();
        self.selected_clip = Some(id);
        match action {
            MenuAction::Cut => self.cut_clip(),
            MenuAction::Copy => {
                self.copy_clip();
            }
            MenuAction::Duplicate => self.duplicate_clip(),
            MenuAction::Delete => self.delete_selection(),
            MenuAction::Split => self.split_clip(),
            MenuAction::Consolidate => self.consolidate_clip(id),
            MenuAction::Export => self.export_clip_request(id),
            MenuAction::Disable | MenuAction::Enable => self.toggle_clip_disabled(),
            MenuAction::ShowInFinder => self.show_clip_in_finder(),
            MenuAction::Rename => {
                self.rename = Some(id);
                self.ui_dirty = true;
            }
            MenuAction::Quantize => {
                let (g, a) = (midi::grid_ticks(self.pr.grid), self.pr.quant_amount);
                self.menu_midi(id, |_, c| Some(midi::quantize(&c.notes, g, a)), "Quantize Notes");
            }
            MenuAction::Humanize => {
                let a = self.pr.humanize_amount;
                let seed = id.0 ^ self.doc.revision();
                self.menu_midi(id, |_, c| Some(midi::humanize(&c.notes, a, seed)), "Humanize Notes");
            }
            MenuAction::ToPattern | MenuAction::ToMidi => self.convert_clip(id),
            _ => self.set_status(format!("{}: not implemented yet", action.label()), 3),
        }
    }

    /// A pattern clip becomes plain MIDI (its repeats become notes); a MIDI clip becomes a
    /// pattern read from its notes (pads on a Drum Rack track, else pitch rows).
    pub fn convert_clip(&mut self, id: ClipId) {
        let Some(c) = self.project().midi_clip(id).cloned() else { return };
        let pattern = match c.pattern {
            Some(_) => None,
            None => {
                let p = StepPattern::from_notes(&c.notes, c.length, self.seq_rows_for(&c));
                if c.length > p.length() {
                    self.set_status(format!("Too long for a step pattern: {} steps at most", dstudio_core::steps::STEP_LIMIT), 3);
                    return;
                }
                // Notes kept past the clip end would be dropped by the pattern.
                if c.notes.iter().any(|n| n.start >= p.length()) {
                    self.set_status("Notes past the clip end: lengthen the clip first", 3);
                    return;
                }
                Some(p)
            }
        };
        self.execute(Command::SetStepPattern { clip: id, pattern });
    }

    pub fn submenu_activate(&mut self, index: usize) {
        let Some(id) = self.menu.clip else { return };
        if self.menu.sub_for == Some(MenuAction::Bounce) {
            if BOUNCE.get(index).is_some_and(|b| b.2) {
                self.menu_close();
                self.selected_clip = Some(id);
                self.bounce_clip(id);
            }
            return;
        }
        let Some(&(_, _, semis)) = TRANSPOSE.get(index) else { return };
        self.menu_close();
        self.menu_midi(id, |_, c| midi::transpose(&c.notes, semis), "Transpose Notes");
    }

    // ---------------------------------------------------------------- view models

    /// Clip tab, piano roll, context menu and rename popup → Slint.
    /// Open (or, when it is already open, close) a clip editor dropdown. Mode is
    /// disabled with Warp off.
    pub fn clip_menu_open(&mut self, menu: i32) {
        let warped = self.ac_clip().is_some_and(|c| c.warp.enabled);
        self.pr.menu = if (0..CLIP_MENUS.len() as i32).contains(&menu) && (menu != 2 || warped) && self.pr.menu != menu && self.clip_menu_shown(menu) { menu } else { -1 };
        self.ui_dirty = true;
    }

    /// The editor that owns the dropdown is on screen: the audio clip editor for Speed / Warp /
    /// Mode, the piano roll for Grid, Type and Genre.
    pub(crate) fn clip_menu_shown(&self, menu: i32) -> bool {
        self.bottom_tab == BottomTab::Clip
            && match menu {
                ..=2 => self.ac_clip().is_some(),
                6 | 7 => self.seq_visible(),
                4 => self.pr_visible() && !self.composer_is_drums(),
                _ => self.pr_visible(),
            }
    }

    fn clip_menu_active(&self, menu: i32) -> i32 {
        let warp = self.ac_clip().map(|c| c.warp.clone()).unwrap_or_default();
        match menu {
            0 => dstudio_core::warp::SPEED_PRESETS.iter().position(|s| (s - warp.effective_speed()).abs() < 0.01).map_or(-1, |i| i as i32),
            1 => if warp.enabled { 0 } else { 1 },
            2 => dstudio_core::WarpMode::ALL.iter().position(|&m| m == warp.mode).map_or(-1, |i| i as i32),
            3 => GRID_DENOMS.iter().position(|&d| d == self.pr.grid).map_or(-1, |i| i as i32),
            6 | 7 => self.seq_menu_active(menu == 7),
            4 | 5 => self.composer_menu_active(menu == 5),
            8 | 9 => self.pr_scale().map_or(-1, |(_, k)| if menu == 8 { k.root as i32 } else { k.kind as i32 }),
            _ => 0,
        }
    }

    pub fn clip_menu_pick(&mut self, index: i32) {
        let menu = std::mem::replace(&mut self.pr.menu, -1);
        self.ui_dirty = true;
        if menu == 3 {
            if let Some(&d) = GRID_DENOMS.get(index.max(0) as usize) {
                self.pr.grid = d;
            }
            return;
        }
        if menu < 0 || index < 0 || index == self.clip_menu_active(menu) {
            return;
        }
        let i = index.max(0) as usize;
        match menu {
            4 => return self.composer_type(i),
            5 => return self.composer_genre(i),
            6 => return self.seq_set_length(i),
            7 => return self.seq_set_rate(i),
            8 => return self.pr_scale_root(i),
            9 => return self.pr_scale_kind(i),
            _ => {}
        }
        match menu {
            0 => self.warp_pick_speed(i),
            1 => self.warp_pick_enabled(index == 0),
            2 => self.warp_pick_mode(i),
            _ => {}
        }
    }

    fn push_composer(&self, s: &Studio) {
        let Some(v) = self.sidebar_view() else { return };
        s.set_composer_drums(v.drums);
        s.set_composer_part_text(CLIP_MENUS[4].get(v.part as usize).copied().unwrap_or("Chords").into());
        s.set_composer_genre_text(CLIP_MENUS[5].get(v.genre as usize).copied().unwrap_or("Any").into());
        s.set_composer_original(v.original);
        s.set_composer_create_text(v.create_label.into());
        s.set_composer_shape_ok(v.shape_ok);
        s.set_composer_shape_tab(v.shape_tab);
        s.set_shape_drawing(self.shape_ui.drawing);
        s.set_shape_brush(self.shape_ui.brush as i32);
        s.set_shape_has_curves(self.shape_has_curves());
        if *self.composer_shape_shown.borrow() != v.shape_rows {
            *self.composer_shape_shown.borrow_mut() = v.shape_rows.clone();
            let rows: Vec<ShapeRow> = v
                .shape_rows
                .into_iter()
                .map(|r| {
                    let c = r.key.color();
                    ShapeRow {
                        label: r.key.label().into(),
                        value: r.value.into(),
                        kind: r.kind as i32,
                        norm: r.norm,
                        bipolar: r.bipolar,
                        segs: r.segs,
                        seg: r.seg,
                        enabled: r.enabled,
                        tip: r.tip.into(),
                        selected: r.selected,
                        curved: r.curved,
                        color: slint::Color::from_rgb_u8((c >> 16) as u8, (c >> 8) as u8, c as u8),
                    }
                })
                .collect();
            self.composer_shape_model.set_vec(rows);
        }
        if *self.composer_rows_shown.borrow() == v.rows {
            return;
        }
        *self.composer_rows_shown.borrow_mut() = v.rows.clone();
        let rows: Vec<PatternRow> = v
            .rows
            .into_iter()
            .map(|r| PatternRow {
                name: r.name.into(),
                playing: r.playing,
                notes: slint::ModelRc::new(slint::VecModel::from(r.preview.into_iter().map(|(x, y, w)| PreviewNote { x, y, w }).collect::<Vec<_>>())),
            })
            .collect();
        self.composer_row_model.set_vec(rows);
    }

    fn push_step_sequencer(&self, s: &Studio) {
        s.set_seq_tabs(self.pr_clip().is_some_and(|c| c.pattern.is_some()));
        s.set_seq_visible(self.seq_visible());
        let Some(v) = self.seq_view() else { return };
        s.set_seq_lane(match self.pr.seq.lane {
            None => 0,
            Some(l) => dstudio_core::steps::StepLane::ALL.iter().position(|&a| a == l).map_or(0, |i| i as i32 + 1),
        });
        if let Some((len, rate, swing)) = self.seq_footer() {
            s.set_seq_length_text(len.into());
            s.set_seq_rate_text(rate.into());
            s.set_seq_swing_text(swing.into());
        }
        let g = v.geom;
        s.set_seq_col_pitch(g.pitch);
        s.set_seq_cell_w(g.cell_w);
        s.set_seq_cols(g.cols as i32);
        s.set_seq_row_count(g.rows as i32);
        s.set_seq_scroll_y(g.scroll);
        s.set_seq_can_double(self.seq_pattern().is_some_and(|p| p.doubled().is_some()));
        s.set_seq_can_clear(self.seq_pattern().is_some_and(|p| !p.cells.is_empty()));
        s.set_seq_page(g.page as i32);
        s.set_seq_pages(g.pages as i32);
        s.set_seq_follow(!self.pr.seq.follow_off);
        sync_model(&self.seq_row_model, v.rows.into_iter().map(|r| SeqRow { y: r.y, number: r.number.into(), name: r.name.into(), muted: r.muted, soloed: r.soloed, audible: r.audible }).collect());
        sync_model(
            &self.seq_cell_model,
            v.cells.into_iter().map(|c| SeqCell { x: c.x, y: c.y, w: c.w, state: c.state, lane: c.lane, bar: c.bar, marks: c.marks, label: c.label.into(), dim: c.dim }).collect(),
        );
        sync_model(&self.seq_num_model, v.nums.into_iter().map(|n| SeqNum { x: n.x, text: n.text.into(), beat: n.beat, active: n.active }).collect());
    }

    pub fn push_clip_editor(&mut self, s: &Studio) {
        if self.ac_clip().is_some() {
            self.request_onsets();
            self.ensure_lane_peaks();
        }
        if self.pr.ac_edit.is_some_and(|(_, id)| self.ac_clip().map(|c| c.id) != Some(id)) {
            self.pr.ac_edit = None;
        }
        if self.pr.menu >= 0 && !self.clip_menu_shown(self.pr.menu) {
            self.pr.menu = -1;
        }
        s.set_clip_menu(self.pr.menu);
        if self.pr.menu >= 0 {
            let items: Vec<slint::SharedString> = CLIP_MENUS[self.pr.menu as usize].iter().map(|&t| t.into()).collect();
            s.set_clip_menu_items(slint::ModelRc::new(slint::VecModel::from(items)));
            s.set_clip_menu_active(self.clip_menu_active(self.pr.menu));
        }
        let p = self.doc.project();
        s.set_bottom_tab(if self.bottom_tab == BottomTab::Clip { 1 } else { 0 });
        let pr = self.pr_clip().cloned();
        s.set_pr_visible(pr.is_some());
        s.set_pr_pen(self.pr.pen);
        s.set_typing_on(self.typing.active);
        let (vz, hz) = self.pr_zoom_values();
        s.set_pr_vzoom(vz);
        s.set_pr_hzoom(hz);
        let ac = self.ac_clip().cloned();
        s.set_ac_visible(ac.is_some());
        // Clip header, range readouts and colour: shared by the piano roll and the audio clip editor.
        let open = match (&pr, &ac) {
            (Some(c), _) => {
                let len = self.pr.value_drag.as_ref().and_then(|d| d.length_preview()).unwrap_or(c.length);
                Some((c.name.clone(), len, p.find_midi_clip(c.id).map(|(t, _)| t)))
            }
            (None, Some(c)) => {
                s.set_ac_gain_text(format!("{:.1} dB", dstudio_core::model::gain_to_db(c.gain).max(-99.9)).into());
                s.set_ac_bpm_text(format!("{}", self.ac_original_bpm(c).round()).into());
                s.set_ac_speed_text(crate::warpedit::format_speed(c.warp.effective_speed()).into());
                s.set_ac_warped(c.warp.enabled);
                s.set_ac_mode_text(c.warp.mode.label().into());
                s.set_ac_reversed(c.warp.reverse);
                s.set_ac_transpose_text(self.ac_transpose_text(c).into());
                s.set_ac_transpose_enabled(Self::ac_transpose_editable(c));
                let edit = match self.pr.ac_edit.map(|e| e.0) {
                    Some(crate::warpedit::AcEdit::OriginalBpm) => 0,
                    Some(crate::warpedit::AcEdit::Transpose) => 1,
                    None => -1,
                };
                if edit != s.get_ac_edit_field() {
                    s.set_ac_edit_text(self.ac_edit_initial(c).into());
                    s.set_ac_edit_field(edit);
                }
                Some((c.name.clone(), p.clip_length_ticks(c), p.find_clip(c.id).map(|(t, _)| t)))
            }
            _ => None,
        };
        if let Some((name, len, ti)) = open {
            let tpb = p.ticks_per_bar();
            s.set_clip_name(name.into());
            if let Some(ti) = ti {
                let col = track_color(p.tracks[ti].color);
                s.set_clip_color(slint_color(col));
                s.set_clip_ink(slint_color(col.map(|v| v * 0.22)));
            }
            s.set_pr_start_text(pianoroll::fmt_pos(0, tpb).into());
            s.set_pr_end_text(pianoroll::fmt_pos(len, tpb).into());
            let looped = self.pr_loop_preview().filter(|l| l.on);
            s.set_pr_loop_on(looped.is_some());
            s.set_pr_loop_enabled(self.pr_clip().is_some_and(|c| c.pattern.is_none()) || ac.is_some());
            s.set_pr_position_text(pianoroll::fmt_pos(looped.map_or(0, |l| l.start), tpb).into());
            s.set_pr_length_text(pianoroll::fmt_len(looped.map_or(len, |l| l.len()), tpb).into());
            let v = self.pr_view();
            let bpb = tpb as f64 / PPQ as f64;
            let step = self.pr_steps().label_bars;
            let mut bars = Vec::new();
            let mut b = 0i64;
            loop {
                let x = v.x_of(b as f64 * bpb);
                if x > v.w || b > 4096 {
                    break;
                }
                if x >= v.gx - 2.0 {
                    bars.push(RulerLabel { x: x + 4.5, text: format!("{}", b + 1).into() });
                }
                b += step;
            }
            sync_model(&self.pr_bar_model, bars);
        }
        if let Some(c) = &pr {
            s.set_pr_has_notes(!c.notes.is_empty());
            s.set_pr_grid_text(grid_label(self.pr.grid).into());
            s.set_pr_quant_text(format!("{}%", (self.pr.quant_amount * 100.0).round()).into());
            s.set_pr_human_text(format!("{}%", (self.pr.humanize_amount * 100.0).round()).into());
            s.set_pr_vel_text(format!("{}", self.pr.velocity).into());
            let (scale, key) = self.pr_scale().unwrap_or_default();
            s.set_pr_scale_on(scale.highlight);
            s.set_pr_scale_fold(scale.highlight && scale.fold);
            s.set_pr_scale_root(key.root_name().into());
            s.set_pr_scale_type(key.kind_name().into());
            let v = self.pr_view();
            let shown: Vec<i32> = v.visible().collect();
            let keys: Vec<YLabel> = shown
                .into_iter()
                .filter(|&p| p % 12 == 0 || (scale.highlight && key.is_root(p)))
                .filter_map(|p| {
                    let y = v.y_of(p);
                    (y >= GRID_Y - 1.0 && y + ROW_H <= v.h + 1.0).then(|| YLabel { y, text: midi::note_name(p as u8).into() })
                })
                .collect();
            sync_model(&self.pr_key_model, keys);
        }
        self.push_step_sequencer(s);
        self.push_composer(s);
        // Context menu.
        s.set_menu_open(self.menu.open);
        if self.menu.open {
            let (items, h) = layout_items(self.menu.items.iter().map(|&(a, en)| (a.label().to_string(), a.shortcut().to_string(), en, a.separated(), a.submenu())));
            let sub_for = self.menu.sub_for.unwrap_or(MenuAction::Transpose);
            let sub_y = items.iter().zip(&self.menu.items).find(|(_, (a, _))| *a == sub_for).map(|(m, _)| m.y).unwrap_or(0.0);
            sync_model(&self.menu_model, items);
            s.set_menu_x(self.menu.pos.0);
            s.set_menu_y(self.menu.pos.1);
            s.set_menu_h(h);
            s.set_sub_open(self.menu.sub_open);
            s.set_sub_y(sub_y - MENU_PAD);
            let rows: Vec<(String, String, bool)> = if sub_for == MenuAction::Bounce {
                BOUNCE.iter().map(|&(l, k, on)| (l.to_string(), k.to_string(), on)).collect()
            } else {
                TRANSPOSE.iter().map(|(l, k, _)| (l.to_string(), k.to_string(), true)).collect()
            };
            let (sub, sub_h) = layout_items(rows.into_iter().map(|(l, k, on)| (l, k, on, false, false)));
            sync_model(&self.sub_model, sub);
            s.set_sub_h(sub_h);
        }
        // Rename popup.
        let renaming = self
            .rename
            .and_then(|id| p.midi_clip(id).map(|c| c.name.clone()).or_else(|| p.clip(id).map(|c| c.name.clone())))
            .or_else(|| self.rename_section.and_then(|id| p.section_index(id)).map(|i| p.sections[i].name.clone()));
        s.set_rename_title(if self.rename_section.is_some() { "Rename section" } else { "Rename clip" }.into());
        if renaming.is_some() != s.get_rename_open() {
            if let Some(n) = &renaming {
                s.set_rename_text(n.clone().into());
            }
            s.set_rename_open(renaming.is_some());
        }
    }
}

/// Notes mirrored in a clip of `length` ticks: a note that ended at the clip end starts at 0.
/// With the clip's loop on, the part before the loop, the loop itself and the part after it
/// (which never plays) are each mirrored in place, so every note that played still plays and
/// the loop repeats its own material backwards. Notes outside the clip mirror across the whole
/// clip and are clamped to 0.
pub fn reversed_in_clip(notes: &[midi::Note], length: Ticks, clip_loop: Option<&midi::ClipLoop>) -> Vec<midi::Note> {
    let regions: Vec<(Ticks, Ticks)> = match clip_loop {
        Some(l) if l.on => {
            let (s, e) = (l.start.clamp(0, length), l.end.clamp(0, length));
            [(0, s), (s, e), (e, length)].into_iter().filter(|r| r.1 > r.0).collect()
        }
        _ => vec![(0, length)],
    };
    notes
        .iter()
        .map(|n| {
            let (a, b) = regions.iter().copied().find(|r| n.start >= r.0 && n.start < r.1).unwrap_or((0, length));
            midi::Note { start: (a + b - n.start - n.length).max(a), ..*n }
        })
        .collect()
}

#[cfg(test)]
mod reverse_tests {
    use super::*;

    #[test]
    fn reversing_mirrors_notes_in_the_clip() {
        let n = |id, start, length| midi::Note { id, pitch: 60, start, length, velocity: 100 };
        let notes = vec![n(1, 0, 240), n(2, 960, 480), n(3, 3600, 240)];
        let r = reversed_in_clip(&notes, 3840, None);
        assert_eq!(r.iter().map(|n| (n.id, n.start, n.length)).collect::<Vec<_>>(), [(1, 3600, 240), (2, 2400, 480), (3, 0, 240)]);
        assert_eq!(reversed_in_clip(&r, 3840, None), notes, "twice is the original");
        assert_eq!(reversed_in_clip(&[n(4, 3800, 240)], 3840, None)[0].start, 0, "a note past the end is clamped to 0");
        let whole = midi::ClipLoop::whole(3840);
        assert_eq!(reversed_in_clip(&notes, 3840, Some(&whole)), r, "a loop over the whole clip changes nothing");
    }

    /// A loop that ends before the clip does: the reversed notes stay inside the loop, so the
    /// unrolled clip plays as many notes as before.
    #[test]
    fn reversing_keeps_a_short_loops_notes_inside_it() {
        let n = |id, start, length| midi::Note { id, pitch: 60, start, length, velocity: 100 };
        let notes = vec![n(1, 0, 240), n(2, 960, 240), n(3, 1440, 240)];
        let l = midi::ClipLoop { on: true, start: 960, end: 1920, span: 0 };
        let r = reversed_in_clip(&notes, 7680, Some(&l));
        assert_eq!(r.iter().map(|n| (n.id, n.start)).collect::<Vec<_>>(), [(1, 720), (2, 1680), (3, 1200)]);
        assert_eq!(l.unroll(&r, 7680).len(), l.unroll(&notes, 7680).len());
        assert!(r.iter().all(|n| n.start < l.end), "nothing moved past the loop end");
        assert_eq!(reversed_in_clip(&r, 7680, Some(&l)), notes, "twice is the original");
    }
}
