//! Clip sidebar, Patterns tab: Type and Genre fields, the pattern list and the
//! Create pill, backed by Saine's own library (`saine_core::library`; no online service
//! service is called). Every action is one `SetComposer` command, so one undo step.
//!
//! A MIDI clip lists the Type's set (Chords, Bass, Lead, Arp); a pattern clip
//! (Drum Rack) lists drum grooves and shows Genre only. Loading a row plays it
//! in place of the clip's own notes, which the card keeps behind its Original pill.
//!
//! Shape tab (MIDI clips): Octave, Voices, Density, Length, Velocity, Nudge,
//! Repeat and Arp reshape what plays (`saine_core::shape`); each part, and the
//! kept notes, keep their own Shape. The rack's other controls show disabled.

use crate::controller::Controller;
use saine_core::command::ComposerEdit;
use saine_core::library::{self, Composer, LibPattern, MadeSet, PartType};
use saine_core::midi::{MidiClip, Note};
use saine_core::shape::{self, Shape};
use saine_core::{Command, PPQ};

/// One list row: name, lit while it plays, and up to PREVIEW_MAX notes as
/// (x, y, w) fractions of the preview box.
#[derive(Debug, Clone, PartialEq)]
pub struct RowView {
    pub name: String,
    pub playing: bool,
    pub preview: Vec<(f32, f32, f32)>,
}

const PREVIEW_MAX: usize = 96;

/// How a Shape row's control works.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShapeKind {
    Slider,
    Stepper,
    Segments,
}

/// One row of the Shape rack.
#[derive(Debug, Clone, PartialEq)]
pub struct ShapeRowView {
    pub label: &'static str,
    pub value: String,
    pub kind: ShapeKind,
    /// Slider position 0..1.
    pub norm: f32,
    /// The slider fills from its middle (Nudge).
    pub bipolar: bool,
    /// Segment count and the lit one.
    pub segs: i32,
    pub seg: i32,
    pub enabled: bool,
    pub tip: &'static str,
}

/// Rack rows in the reference's order. Rows past Nudge's neighbours that Saine
/// does not shape yet show disabled.
pub const SHAPE_ROWS: [(&str, ShapeKind); 14] = [
    ("Octave", ShapeKind::Stepper),
    ("Voices", ShapeKind::Segments),
    ("Density", ShapeKind::Slider),
    ("Length", ShapeKind::Slider),
    ("Velocity", ShapeKind::Slider),
    ("Vary", ShapeKind::Slider),
    ("Nudge", ShapeKind::Slider),
    ("Repeat", ShapeKind::Stepper),
    ("Brightness", ShapeKind::Slider),
    ("Weight", ShapeKind::Slider),
    ("Voicing", ShapeKind::Slider),
    ("Tension", ShapeKind::Slider),
    ("Inversion", ShapeKind::Stepper),
    ("Arp", ShapeKind::Stepper),
];

const NO_VOICES: &str = "This pattern has no extra voices to add";
const NOT_AVAILABLE: &str = "Not available for this pattern";

/// Shape callback ops: slider press / drag / release, stepper step, segment pick, reset.
pub mod shape_op {
    pub const PRESS: i32 = 0;
    pub const DRAG: i32 = 1;
    pub const RELEASE: i32 = 2;
    pub const STEP: i32 = 3;
    pub const PICK: i32 = 4;
    pub const RESET: i32 = 5;
}

fn shape_rows(s: Shape, voices: bool) -> Vec<ShapeRowView> {
    let pct = |v: f32| format!("{}%", (v * 100.0).round() as i32);
    SHAPE_ROWS
        .iter()
        .enumerate()
        .map(|(i, &(label, kind))| {
            let mut r = ShapeRowView { label, value: String::new(), kind, norm: 0.0, bipolar: false, segs: 0, seg: 0, enabled: true, tip: "" };
            match i {
                0 => r.value = if s.octave > 0 { format!("+{}", s.octave) } else { s.octave.to_string().replace('-', "−") },
                1 => {
                    r.segs = shape::MAX_VOICES as i32;
                    r.seg = s.voices as i32 - 1;
                    r.enabled = voices;
                    r.tip = if voices { "" } else { NO_VOICES };
                }
                2 => (r.norm, r.value) = (s.density, pct(s.density)),
                3 => (r.norm, r.value) = (s.length, pct(s.length)),
                4 => (r.norm, r.value) = (s.velocity, pct(s.velocity)),
                6 => (r.norm, r.value, r.bipolar) = (s.nudge, format!("{:+}%", ((s.nudge - 0.5) * 200.0).round() as i32), true),
                7 => r.value = shape::REPEAT_LABELS[s.repeat as usize].to_string(),
                12 => (r.value, r.enabled, r.tip) = ("Root".to_string(), false, NOT_AVAILABLE),
                13 => r.value = shape::ARP_LABELS[s.arp as usize].to_string(),
                _ => (r.enabled, r.tip) = (false, NOT_AVAILABLE),
            }
            r
        })
        .collect()
}

pub struct SidebarView {
    pub drums: bool,
    pub part: i32,
    pub genre: i32,
    pub rows: Vec<RowView>,
    /// The kept notes' pill (shown while a pattern plays).
    pub original: bool,
    pub create_label: String,
    /// Shape can open (a MIDI clip with notes) and is the tab showing.
    pub shape_ok: bool,
    pub shape_tab: bool,
    pub shape_rows: Vec<ShapeRowView>,
}

fn preview(notes: &[saine_core::Note], length: i64) -> Vec<(f32, f32, f32)> {
    let (lo, hi) = notes.iter().fold((127u8, 0u8), |(lo, hi), n| (lo.min(n.pitch), hi.max(n.pitch)));
    let span = (hi.saturating_sub(lo) as f32).max(6.0);
    let len = length.max(PPQ) as f32;
    notes
        .iter()
        .take(PREVIEW_MAX)
        .map(|n| (n.start as f32 / len, 1.0 - (n.pitch - lo) as f32 / span, (n.length as f32 / len).max(0.01)))
        .collect()
}

impl Controller {
    fn composer_clip(&self) -> Option<MidiClip> {
        self.pr_clip().cloned()
    }

    fn is_drum_clip(c: &MidiClip) -> bool {
        c.pattern.as_ref().is_some_and(|p| p.rows == saine_core::steps::StepRows::Pads)
    }

    /// The shown part's made set, if any.
    fn shown_set(c: &MidiClip) -> Option<MadeSet> {
        let comp = c.composer.as_ref()?;
        comp.set_for(if Self::is_drum_clip(c) { None } else { Some(comp.part) })
    }

    fn set_patterns(&self, c: &MidiClip, made: MadeSet) -> Vec<LibPattern> {
        let p = self.project();
        library::patterns_of(made, p.transport.key, c.length, p.ticks_per_bar())
    }

    pub fn sidebar_view(&self) -> Option<SidebarView> {
        let c = self.composer_clip()?;
        let drums = Self::is_drum_clip(&c);
        let comp = c.composer.clone().unwrap_or_default();
        let made = Self::shown_set(&c);
        let rows: Vec<RowView> = made
            .map(|m| self.set_patterns(&c, m))
            .unwrap_or_default()
            .into_iter()
            .enumerate()
            .map(|(i, lp)| RowView { name: lp.name, playing: comp.playing == Some(i as u8), preview: preview(&lp.notes, c.length) })
            .collect();
        let create_label = if drums {
            "Create new patterns".to_string()
        } else if rows.is_empty() {
            format!("Create {}", comp.part.plural())
        } else {
            format!("Create new {}", comp.part.plural())
        };
        let shape_ok = !drums && !c.notes.is_empty();
        let (base, source, shape) = self.shape_state(&c);
        Some(SidebarView {
            drums,
            part: PartType::ALL.iter().position(|&p| p == comp.part).unwrap_or(0) as i32,
            genre: comp.genre as i32,
            original: comp.playing.is_some(),
            rows,
            create_label,
            shape_ok,
            shape_tab: shape_ok && self.composer_shape_tab.unwrap_or(comp.playing.is_none()),
            shape_rows: if shape_ok { shape_rows(shape, shape::has_voices(&base, source)) } else { Vec::new() },
        })
    }

    fn shaped(&self, notes: &[Note], shape: Shape, source: Option<PartType>, c: &MidiClip) -> Vec<Note> {
        shape::apply(notes, shape, source, self.project().transport.key, c.length)
    }

    /// The unshaped notes of what plays, its source and its Shape. Notes edited
    /// since they were shaped play as they are, under a neutral Shape.
    fn shape_state(&self, c: &MidiClip) -> (Vec<Note>, Option<PartType>, Shape) {
        let comp = c.composer.clone().unwrap_or_default();
        let source = comp.shape_source();
        let shape = comp.shape_of(source);
        if shape == Shape::neutral(source) {
            return (c.notes.clone(), source, shape);
        }
        if !comp.base.is_empty() && self.shaped(&comp.base, shape, source, c) == c.notes {
            return (comp.base, source, shape);
        }
        (c.notes.clone(), source, Shape::neutral(source))
    }

    /// Commit a composer edit; a loaded pattern plays under its source's Shape.
    fn commit_composer(&mut self, c: &MidiClip, edit: ComposerEdit, mut comp: Composer, load: Option<LibPattern>) {
        let (notes, pattern) = match load {
            Some(lp) => {
                let source = comp.shape_source();
                let shape = comp.shape_of(source);
                comp.base.clear();
                if Self::is_drum_clip(c) || shape == Shape::neutral(source) {
                    (lp.notes, lp.pattern)
                } else {
                    let notes = self.shaped(&lp.notes, shape, source, c);
                    comp.base = lp.notes;
                    (notes, lp.pattern)
                }
            }
            None => (c.notes.clone(), c.pattern.clone()),
        };
        self.execute(Command::SetComposer { clip: c.id, edit, composer: Some(comp), notes, pattern });
        self.ui_dirty = true;
    }

    /// The composer with the clip's own (unshaped) notes kept, when they are the ones playing.
    fn composer_keeping(&self, c: &MidiClip) -> Composer {
        let mut comp = c.composer.clone().unwrap_or_default();
        if comp.playing.is_none() {
            let (base, _, shape) = self.shape_state(c);
            if shape == Shape::neutral(None) {
                comp.set_shape(None, shape);
            }
            comp.kept = base;
            comp.kept_pattern = c.pattern.clone();
        }
        comp
    }

    /// Patterns / Shape tabs: the pick holds on every clip.
    pub fn composer_tab(&mut self, shape: bool) {
        self.composer_shape_tab = Some(shape);
        self.ui_dirty = true;
    }

    /// A Shape rack control (`shape_op`): sliders take a 0..1 value and coalesce into
    /// one undo step until released; steppers step by `value`'s sign; segments pick
    /// `value` (0-based); reset puts the control back to neutral.
    pub fn composer_shape(&mut self, row: i32, op: i32, value: f32) {
        use shape_op::*;
        self.ui_dirty = true;
        if op == RELEASE {
            self.doc.seal_coalescing();
            return;
        }
        let Some(c) = self.composer_clip() else { return };
        if Self::is_drum_clip(&c) || c.notes.is_empty() {
            return;
        }
        let (base, source, old) = self.shape_state(&c);
        let neutral = Shape::neutral(source);
        let mut s = old;
        let unit = value.clamp(0.0, 1.0);
        let step = |i: u8, len: usize| (i as i32 + value.signum() as i32).clamp(0, len as i32 - 1) as u8;
        match (row, op) {
            (0, STEP) => s.octave = (s.octave + value.signum() as i8).clamp(-shape::MAX_OCTAVE, shape::MAX_OCTAVE),
            (0, RESET) => s.octave = neutral.octave,
            (1, PICK) if shape::has_voices(&base, source) => s.voices = (value.round() as i32 + 1).clamp(1, shape::MAX_VOICES as i32) as u8,
            (1, RESET) => s.voices = neutral.voices,
            (2, PRESS | DRAG) => s.density = unit,
            (2, RESET) => s.density = neutral.density,
            (3, PRESS | DRAG) => s.length = unit,
            (3, RESET) => s.length = neutral.length,
            (4, PRESS | DRAG) => s.velocity = unit,
            (4, RESET) => s.velocity = neutral.velocity,
            (6, PRESS | DRAG) => s.nudge = unit,
            (6, RESET) => s.nudge = neutral.nudge,
            (7, STEP) => s.repeat = step(s.repeat, shape::REPEAT_BEATS.len()),
            (7, RESET) => s.repeat = neutral.repeat,
            (13, STEP) => s.arp = step(s.arp, shape::ARP_BEATS.len()),
            (13, RESET) => s.arp = neutral.arp,
            _ => return,
        }
        if s == old {
            return;
        }
        let mut comp = c.composer.clone().unwrap_or_default();
        comp.set_shape(source, s);
        comp.base = if s == neutral { Vec::new() } else { base.clone() };
        let notes = self.shaped(&base, s, source, &c);
        self.execute(Command::SetComposer { clip: c.id, edit: ComposerEdit::Shape, composer: Some(comp), notes, pattern: c.pattern.clone() });
        if op != PRESS && op != DRAG {
            self.doc.seal_coalescing();
        }
    }

    /// Create: a new set of six for the shown part on the Genre field's genre; the first plays.
    pub fn composer_create(&mut self) {
        let Some(c) = self.composer_clip() else { return };
        let mut comp = self.composer_keeping(&c);
        let part = if Self::is_drum_clip(&c) { None } else { Some(comp.part) };
        let set = comp.set_for(part).map_or(0, |m| m.set + 1);
        comp.made.retain(|m| m.part != part);
        let made = MadeSet { part, genre: comp.genre, set, last: 0 };
        comp.made.push(made);
        comp.playing = Some(0);
        let first = self.set_patterns(&c, made).into_iter().next();
        self.commit_composer(&c, ComposerEdit::Create, comp, first);
    }

    /// Click a list row: it plays in place of the clip.
    pub fn composer_load(&mut self, row: usize) {
        let Some(c) = self.composer_clip() else { return };
        let Some(made) = Self::shown_set(&c) else { return };
        let Some(lp) = self.set_patterns(&c, made).into_iter().nth(row) else { return };
        let mut comp = self.composer_keeping(&c);
        if comp.playing == Some(row as u8) {
            return;
        }
        comp.playing = Some(row as u8);
        if let Some(m) = comp.made.iter_mut().find(|m| m.part == made.part) {
            m.last = row as u8;
        }
        self.commit_composer(&c, ComposerEdit::Load, comp, Some(lp));
    }

    /// The card's Original pill: the kept notes play again.
    pub fn composer_original(&mut self) {
        let Some(c) = self.composer_clip() else { return };
        let Some(mut comp) = c.composer.clone().filter(|k| k.playing.is_some()) else { return };
        comp.playing = None;
        let kept = LibPattern { name: String::new(), notes: std::mem::take(&mut comp.kept), pattern: comp.kept_pattern.take() };
        self.commit_composer(&c, ComposerEdit::Original, comp, Some(kept));
    }

    /// Type field: show another part and play the row of it last played; a part with no
    /// set yet gets one at once (its first row plays).
    pub fn composer_type(&mut self, index: usize) {
        let Some(c) = self.composer_clip() else { return };
        let Some(&part) = PartType::ALL.get(index) else { return };
        let mut comp = self.composer_keeping(&c);
        if comp.part == part {
            return;
        }
        comp.part = part;
        let made = match comp.set_for(Some(part)) {
            Some(m) => m,
            None => {
                let m = MadeSet { part: Some(part), genre: comp.genre, set: 0, last: 0 };
                comp.made.push(m);
                m
            }
        };
        comp.playing = Some(made.last);
        let lp = self.set_patterns(&c, made).into_iter().nth(made.last as usize);
        self.commit_composer(&c, ComposerEdit::Type, comp, lp);
    }

    /// Genre field: the genre the next Create builds on.
    pub fn composer_genre(&mut self, index: usize) {
        let Some(c) = self.composer_clip() else { return };
        if index >= library::GENRES.len() {
            return;
        }
        let mut comp = c.composer.clone().unwrap_or_default();
        if comp.genre as usize == index {
            return;
        }
        comp.genre = index as u8;
        self.execute(Command::SetComposer { clip: c.id, edit: ComposerEdit::Genre, composer: Some(comp), notes: c.notes.clone(), pattern: c.pattern.clone() });
        self.ui_dirty = true;
    }

    /// Active rows of the Type and Genre menus.
    pub fn composer_menu_active(&self, genre: bool) -> i32 {
        self.sidebar_view().map_or(0, |v| if genre { v.genre } else { v.part })
    }

    pub fn composer_is_drums(&self) -> bool {
        self.pr_clip().is_some_and(Self::is_drum_clip)
    }
}
