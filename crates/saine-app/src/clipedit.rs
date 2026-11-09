//! Clip-level editing: insert MIDI clip (⌘⇧M), duplicate (⌘D), the internal
//! clipboard (⌘X / ⌘C / ⌘V), rename, the clip context menu, and pushing the
//! Clip tab / piano-roll view models to Slint. Every edit is one Command.

use crate::controller::{BottomTab, ClipboardItem, Controller, MenuAction, sync_model, track_color};
use crate::pianoroll::{self, GRID_Y, ROW_H};
use crate::{MenuItem, RulerLabel, Studio, YLabel};
use saine_core::midi::{self, MidiClip};
use saine_core::{ClipId, Command, PPQ, Ticks, TrackKind};

/// Context-menu geometry (window points), measured from `35-clip-context-menu.png`.
pub const MENU_W: f32 = 209.0;
pub const MENU_PAD: f32 = 4.0;
pub const MENU_ROW: f32 = 20.5;
pub const MENU_SEP: f32 = 10.5;

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
            MenuAction::Rename => "Rename…",
            MenuAction::Export => "Export audio to Finder",
            MenuAction::Transpose => "Transpose",
            MenuAction::Quantize => "Quantize",
            MenuAction::Humanize => "Humanize",
            MenuAction::Ask => "Ask",
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
            MenuAction::Disable => "0",
            MenuAction::Ask => "⌘ K",
            _ => "",
        }
    }
    /// A separator is drawn above these entries (reference grouping).
    fn separated(self) -> bool {
        matches!(self, MenuAction::Delete | MenuAction::Disable | MenuAction::Export | MenuAction::Transpose | MenuAction::Ask)
    }
    fn submenu(self) -> bool {
        matches!(self, MenuAction::Bounce | MenuAction::Transpose)
    }
}

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
    /// selected instrument track. Opens the Clip tab (the clip is not selected,
    /// as in the reference).
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
        let start = ((self.playhead_ticks() as f64 / PPQ as f64).round() as Ticks * PPQ).max(0);
        let id = ClipId(p.next_id);
        let mut clip = MidiClip::empty(start, p.ticks_per_bar());
        clip.id = id;
        if self.execute(Command::AddMidiClip { track, clip }) {
            self.set_bottom_tab(BottomTab::Clip);
            return Some(id);
        }
        None
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
            Command::AddClip { track: p.tracks[ti].id, clip: saine_core::Clip { id: new_id, start: c.start + len, ..c.clone() } }
        } else {
            return;
        };
        if let Command::AddMidiClip { .. } | Command::AddClip { .. } = cmd
            && self.execute(Command::Batch { label: "Duplicate Clip".into(), commands: vec![cmd] })
        {
            self.select_clip(Some(new_id));
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
            ClipboardItem::Audio { clip, .. } => Command::AddClip { track, clip: saine_core::Clip { id: new_id, start: at, ..clip } },
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
            vec![Command::RemoveClip { clip: id }, Command::AddClip { track: p.tracks[ti].id, clip: saine_core::Clip { name: name.into(), ..c.clone() } }]
        } else {
            return;
        };
        self.execute(Command::Batch { label: "Rename Clip".into(), commands: cmds });
    }

    /// Whole-clip MIDI transforms from the context menu.
    fn menu_midi(&mut self, id: ClipId, f: impl FnOnce(&Self, &MidiClip) -> Option<Vec<saine_core::Note>>, label: &str) {
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
        let hit = hit_test(&self.view, self.lanes_size, self.project().tracks.len(), &self.clip_vis, x, y);
        let Hit::Clip { index, .. } = hit else {
            self.menu_close();
            return;
        };
        let Some(&id) = self.clip_ids.get(index) else { return };
        if let Some(t) = self.clip_track(id) {
            self.select_track(t);
        }
        self.select_clip(Some(id));
        let midi = self.project().find_midi_clip(id).is_some();
        use MenuAction::*;
        let items = [Cut, Copy, Duplicate, Split, Consolidate, Bounce, Delete, Disable, Rename, Export, Transpose, Quantize, Humanize, Ask];
        // PLACEHOLDER entries (disabled): Split, Consolidate, Bounce, Disable, Export, Ask.
        self.menu.items = items
            .iter()
            .map(|&a| {
                let enabled = match a {
                    Cut | Copy | Duplicate | Delete | Rename => true,
                    Transpose | Quantize | Humanize => midi,
                    _ => false,
                };
                (a, enabled)
            })
            .collect();
        let (_, h) = layout_items(self.menu.items.iter().map(|&(a, en)| (String::new(), String::new(), en, a.separated(), false)));
        let mx = (wx + 2.0).min(win.0 - MENU_W - 4.0).max(4.0);
        let my = (wy + 2.0).min(win.1 - h - 4.0).max(4.0);
        self.menu.open = true;
        self.menu.sub_open = false;
        self.menu.pos = (mx, my);
        self.menu.clip = Some(id);
        self.ui_dirty = true;
    }

    pub fn menu_close(&mut self) {
        if self.menu.open {
            self.menu.open = false;
            self.menu.sub_open = false;
            self.menu.clip = None;
            self.ui_dirty = true;
        }
    }

    pub fn menu_activate(&mut self, index: usize) {
        let Some(&(action, enabled)) = self.menu.items.get(index) else { return };
        let Some(id) = self.menu.clip else { return };
        if !enabled {
            return;
        }
        if action == MenuAction::Transpose {
            self.menu.sub_open = !self.menu.sub_open;
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
            _ => self.set_status(format!("{}: not implemented yet", action.label()), 3),
        }
    }

    pub fn submenu_activate(&mut self, index: usize) {
        let Some(id) = self.menu.clip else { return };
        let Some(&(_, _, semis)) = TRANSPOSE.get(index) else { return };
        self.menu_close();
        self.menu_midi(id, |_, c| midi::transpose(&c.notes, semis), "Transpose Notes");
    }

    // ---------------------------------------------------------------- view models

    /// Clip tab, piano roll, context menu and rename popup → Slint.
    pub fn push_clip_editor(&mut self, s: &Studio) {
        let p = self.doc.project();
        s.set_bottom_tab(if self.bottom_tab == BottomTab::Clip { 1 } else { 0 });
        let pr = self.pr_clip().cloned();
        s.set_pr_visible(pr.is_some());
        s.set_pr_pen(self.pr.pen);
        let ac = self.ac_clip().cloned();
        s.set_ac_visible(ac.is_some());
        // Clip header, range readouts and colour: shared by the piano roll and the audio clip editor.
        let open = match (&pr, &ac) {
            (Some(c), _) => {
                let len = self.pr.value_drag.as_ref().and_then(|d| d.length_preview()).unwrap_or(c.length);
                Some((c.name.clone(), len, p.find_midi_clip(c.id).map(|(t, _)| t)))
            }
            (None, Some(c)) => {
                s.set_ac_gain_text(format!("{:.1} dB", saine_core::model::gain_to_db(c.gain).max(-99.9)).into());
                s.set_ac_bpm_text(format!("{}", p.transport.bpm.round()).into());
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
            s.set_pr_position_text(pianoroll::fmt_pos(0, tpb).into());
            s.set_pr_length_text(pianoroll::fmt_len(len, tpb).into());
            let v = self.pr_view();
            let bpb = tpb as f64 / PPQ as f64;
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
                b += 1;
            }
            sync_model(&self.pr_bar_model, bars);
        }
        if let Some(c) = &pr {
            s.set_pr_has_notes(!c.notes.is_empty());
            s.set_pr_grid_text(format!("1/{}", self.pr.grid).into());
            s.set_pr_quant_text(format!("{}%", (self.pr.quant_amount * 100.0).round()).into());
            s.set_pr_human_text(format!("{}%", (self.pr.humanize_amount * 100.0).round()).into());
            s.set_pr_vel_text(format!("{}", self.pr.velocity).into());
            let v = self.pr_view();
            let keys: Vec<YLabel> = (0..=127)
                .filter(|p| p % 12 == 0)
                .filter_map(|p| {
                    let y = v.y_of(p);
                    (y >= GRID_Y - 1.0 && y + ROW_H <= v.h + 1.0).then(|| YLabel { y, text: midi::note_name(p as u8).into() })
                })
                .collect();
            sync_model(&self.pr_key_model, keys);
        }
        // Context menu.
        s.set_menu_open(self.menu.open);
        if self.menu.open {
            let (items, h) = layout_items(self.menu.items.iter().map(|&(a, en)| (a.label().to_string(), a.shortcut().to_string(), en, a.separated(), a.submenu())));
            let sub_y = items.iter().zip(&self.menu.items).find(|(_, (a, _))| *a == MenuAction::Transpose).map(|(m, _)| m.y).unwrap_or(0.0);
            sync_model(&self.menu_model, items);
            s.set_menu_x(self.menu.pos.0);
            s.set_menu_y(self.menu.pos.1);
            s.set_menu_h(h);
            s.set_sub_open(self.menu.sub_open);
            s.set_sub_y(sub_y - MENU_PAD);
            let (sub, sub_h) = layout_items(TRANSPOSE.iter().map(|(l, k, _)| (l.to_string(), k.to_string(), true, false, false)));
            sync_model(&self.sub_model, sub);
            s.set_sub_h(sub_h);
        }
        // Rename popup.
        let renaming = self.rename.and_then(|id| p.midi_clip(id).map(|c| c.name.clone()).or_else(|| p.clip(id).map(|c| c.name.clone())));
        if renaming.is_some() != s.get_rename_open() {
            if let Some(n) = &renaming {
                s.set_rename_text(n.clone().into());
            }
            s.set_rename_open(renaming.is_some());
        }
    }
}

