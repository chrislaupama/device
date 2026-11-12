//! Step patterns: the grid behind the step sequencer on Drum Rack clips
//! (and, by pitch, on instrument tracks).
//!
//! A pattern clip is an ordinary MIDI clip whose notes are baked from a
//! [StepPattern]. The pattern is what the step editor shows and edits; the
//! notes are what the piano roll shows and the engine plays. Every step edit
//! re-bakes the notes ([bake_into]); a piano-roll edit re-reads only the rows
//! whose notes changed ([rederive]). A pattern shorter than its clip repeats
//! (see snapshot::build and [tile]).
//!
//! Lanes (per cell): Velocity 1..=127 (default 100), Offset -49..=49 % of a
//! step (0), Repeat 1..=16 hits inside the step (1), Tie 0..=63 following
//! steps held (0). Swing pushes every second step late by up to 49 %.

use crate::builtin::drum_rack;
use crate::midi::Note;
use crate::model::{PPQ, Ticks};
use serde::{Deserialize, Serialize};

/// Step rate ("Rate"), in the order of its menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum StepRate {
    #[serde(rename = "1/1")]
    Whole,
    #[serde(rename = "1/2")]
    Half,
    #[serde(rename = "1/4")]
    Quarter,
    #[serde(rename = "1/8")]
    Eighth,
    #[default]
    #[serde(rename = "1/16")]
    Sixteenth,
    #[serde(rename = "1/32")]
    ThirtySecond,
    #[serde(rename = "1/8T")]
    EighthTriplet,
    #[serde(rename = "1/16T")]
    SixteenthTriplet,
    #[serde(rename = "1/32T")]
    ThirtySecondTriplet,
}

impl StepRate {
    pub const ALL: [StepRate; 9] = [
        StepRate::Whole,
        StepRate::Half,
        StepRate::Quarter,
        StepRate::Eighth,
        StepRate::Sixteenth,
        StepRate::ThirtySecond,
        StepRate::EighthTriplet,
        StepRate::SixteenthTriplet,
        StepRate::ThirtySecondTriplet,
    ];

    pub fn label(self) -> &'static str {
        match self {
            StepRate::Whole => "1/1",
            StepRate::Half => "1/2",
            StepRate::Quarter => "1/4",
            StepRate::Eighth => "1/8",
            StepRate::Sixteenth => "1/16",
            StepRate::ThirtySecond => "1/32",
            StepRate::EighthTriplet => "1/8T",
            StepRate::SixteenthTriplet => "1/16T",
            StepRate::ThirtySecondTriplet => "1/32T",
        }
    }

    pub fn from_label(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|r| r.label() == s)
    }

    /// Length of one step.
    pub fn ticks(self) -> Ticks {
        match self {
            StepRate::Whole => PPQ * 4,
            StepRate::Half => PPQ * 2,
            StepRate::Quarter => PPQ,
            StepRate::Eighth => PPQ / 2,
            StepRate::Sixteenth => PPQ / 4,
            StepRate::ThirtySecond => PPQ / 8,
            StepRate::EighthTriplet => PPQ / 3,
            StepRate::SixteenthTriplet => PPQ / 6,
            StepRate::ThirtySecondTriplet => PPQ / 12,
        }
    }

    /// Steps per beat (a quarter note): where the editor's ruler numbers are bright.
    pub fn steps_per_beat(self) -> f64 {
        PPQ as f64 / self.ticks() as f64
    }
}

/// The "Length" choices (steps).
pub const STEP_COUNTS: [u16; 8] = [8, 12, 16, 24, 32, 36, 48, 64];
/// Longest Length choice and the longest pattern Double makes.
pub const MAX_STEPS: u16 = 64;
/// Longest pattern: a clip longer than MAX_STEPS converts with its own step
/// count (the editor keeps it and pages through it).
pub const STEP_LIMIT: u16 = 4096;
/// Default for a new pattern clip: 16 steps of 1/16 (one bar of 4/4).
pub const DEFAULT_STEPS: u16 = 16;
/// Gap left between repeated hits and before a following hit (0.02 beat).
pub const GATE_GAP: Ticks = PPQ / 50;

/// Per-cell parameter lanes, in the editor's tab order after "Steps".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum StepLane {
    Velocity,
    Offset,
    Repeat,
    Tie,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LaneSpec {
    pub label: &'static str,
    pub min: i32,
    pub max: i32,
    pub default: i32,
    /// Tooltip of the lane's tab.
    pub tip: &'static str,
}

impl StepLane {
    pub const ALL: [StepLane; 4] = [StepLane::Velocity, StepLane::Offset, StepLane::Repeat, StepLane::Tie];

    pub fn spec(self) -> LaneSpec {
        match self {
            StepLane::Velocity => {
                LaneSpec { label: "Velocity", min: 1, max: 127, default: 100, tip: "How hard each step hits: drag a cell" }
            }
            StepLane::Offset => {
                LaneSpec { label: "Offset", min: -49, max: 49, default: 0, tip: "Push each step early or late: drag a cell" }
            }
            StepLane::Repeat => {
                LaneSpec { label: "Repeat", min: 1, max: 16, default: 1, tip: "Repeat each hit within its step: drag a cell" }
            }
            StepLane::Tie => LaneSpec { label: "Tie", min: 0, max: 63, default: 0, tip: "Hold each step into the next: drag a cell" },
        }
    }

    /// Value as the editor's drag tooltip shows it.
    pub fn format(self, v: i32) -> String {
        match self {
            StepLane::Offset if v > 0 => format!("+{v}%"),
            StepLane::Offset => format!("{v}%"),
            StepLane::Tie if v == 0 => "off".into(),
            StepLane::Tie => format!("\u{2192} {v} step{}", if v == 1 { "" } else { "s" }),
            _ => v.to_string(),
        }
    }
}

/// One active step of one row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepCell {
    /// MIDI note the row plays (a drum pad's note, or a pitch).
    pub key: u8,
    pub step: u16,
    pub velocity: u8,
    pub offset: i8,
    pub repeat: u8,
    pub tie: u8,
}

impl StepCell {
    pub fn new(key: u8, step: u16) -> Self {
        StepCell { key, step, velocity: 100, offset: 0, repeat: 1, tie: 0 }
    }

    pub fn get(&self, lane: StepLane) -> i32 {
        match lane {
            StepLane::Velocity => self.velocity as i32,
            StepLane::Offset => self.offset as i32,
            StepLane::Repeat => self.repeat as i32,
            StepLane::Tie => self.tie as i32,
        }
    }

    /// Set a lane value, clamped to the lane's range.
    pub fn set(&mut self, lane: StepLane, v: i32) {
        let s = lane.spec();
        let v = v.clamp(s.min, s.max);
        match lane {
            StepLane::Velocity => self.velocity = v as u8,
            StepLane::Offset => self.offset = v as i8,
            StepLane::Repeat => self.repeat = v as u8,
            StepLane::Tie => self.tie = v as u8,
        }
    }
}

/// What the rows are: one per Drum Rack pad (notes 36..=51),
/// or one per pitch (for instrument tracks).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum StepRows {
    #[default]
    Pads,
    Pitches,
}

impl StepRows {
    pub fn contains(self, key: u8) -> bool {
        match self {
            StepRows::Pads => (drum_rack::FIRST_NOTE..drum_rack::FIRST_NOTE + drum_rack::PADS as u8).contains(&key),
            StepRows::Pitches => key <= 127,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StepPattern {
    pub rate: StepRate,
    /// Pattern length in steps (1..=STEP_LIMIT; the editor offers STEP_COUNTS).
    pub steps: u16,
    /// 0..=1: every second step is pushed late by swing x 49 % of a step.
    #[serde(default)]
    pub swing: f32,
    #[serde(default)]
    pub rows: StepRows,
    /// Active cells, sorted by (key, step), at most one per (key, step).
    #[serde(default)]
    pub cells: Vec<StepCell>,
    /// Rows muted / soloed with the editor's M / S buttons. Their notes stay in
    /// the clip; playback leaves them out (`StepPattern::audible`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub muted: Vec<u8>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub soloed: Vec<u8>,
}

impl Default for StepPattern {
    fn default() -> Self {
        StepPattern::new(StepRows::Pads)
    }
}

impl StepPattern {
    pub fn new(rows: StepRows) -> Self {
        StepPattern {
            rate: StepRate::default(),
            steps: DEFAULT_STEPS,
            swing: 0.0,
            rows,
            cells: Vec::new(),
            muted: Vec::new(),
            soloed: Vec::new(),
        }
    }

    /// Pattern length (ticks); the clip repeats it.
    pub fn length(&self) -> Ticks {
        self.steps as Ticks * self.rate.ticks()
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.steps == 0 || self.steps > STEP_LIMIT {
            return Err(format!("steps {} outside 1..={STEP_LIMIT}", self.steps));
        }
        if !self.swing.is_finite() || !(0.0..=1.0).contains(&self.swing) {
            return Err(format!("swing {} outside 0..=1", self.swing));
        }
        if let Some(k) = self.muted.iter().chain(&self.soloed).find(|&&k| !self.rows.contains(k)) {
            return Err(format!("muted or soloed key {k} is not a row of this pattern"));
        }
        if [&self.muted, &self.soloed].iter().any(|l| l.windows(2).any(|w| w[0] >= w[1])) {
            return Err("muted and soloed keys must be sorted and unique".into());
        }
        for (i, c) in self.cells.iter().enumerate() {
            if !self.rows.contains(c.key) {
                return Err(format!("cell key {} is not a row of this pattern", c.key));
            }
            if c.step >= self.steps {
                return Err(format!("cell step {} past the pattern's {} steps", c.step, self.steps));
            }
            for lane in StepLane::ALL {
                let s = lane.spec();
                let v = c.get(lane);
                if v < s.min || v > s.max {
                    return Err(format!("cell {}:{} {} {v} outside {}..={}", c.key, c.step, s.label, s.min, s.max));
                }
            }
            if let Some(p) = i.checked_sub(1).map(|j| &self.cells[j]) {
                if (p.key, p.step) >= (c.key, c.step) {
                    return Err(format!("cells not sorted or duplicated at {}:{}", c.key, c.step));
                }
            }
        }
        Ok(())
    }

    fn sort(&mut self) {
        self.cells.sort_by_key(|c| (c.key, c.step));
        self.cells.dedup_by_key(|c| (c.key, c.step));
    }

    pub fn cell(&self, key: u8, step: u16) -> Option<&StepCell> {
        self.cells.binary_search_by_key(&(key, step), |c| (c.key, c.step)).ok().map(|i| &self.cells[i])
    }

    fn row(&self, key: u8) -> &[StepCell] {
        let a = self.cells.partition_point(|c| c.key < key);
        let b = self.cells.partition_point(|c| c.key <= key);
        &self.cells[a..b]
    }

    /// Keys that have at least one cell.
    pub fn keys(&self) -> Vec<u8> {
        let mut k: Vec<u8> = self.cells.iter().map(|c| c.key).collect();
        k.dedup();
        k
    }

    /// True when (key, step) is held by an earlier cell's tie (the editor
    /// draws it as part of that cell; it does not sound on its own).
    pub fn is_covered(&self, key: u8, step: u16) -> bool {
        self.sounding(key).iter().any(|&(c, d)| c.step < step && step <= c.step + d)
    }

    /// The row's cells that sound, each with its tie clamped to the pattern;
    /// cells under an earlier tie are left out.
    pub fn sounding(&self, key: u8) -> Vec<(StepCell, u16)> {
        let mut out = Vec::new();
        let mut held_until: i32 = -1;
        for c in self.row(key) {
            if (c.step as i32) <= held_until {
                continue;
            }
            let d = (c.tie as u16).min(self.steps - 1 - c.step);
            held_until = (c.step + d) as i32;
            out.push((*c, d));
        }
        out
    }

    /// Swing's share of a step's offset (percent of a step).
    pub fn swing_offset(&self, step: u16) -> i32 {
        if step % 2 == 1 { (self.swing as f64 * 49.0).round() as i32 } else { 0 }
    }

    /// The offset a cell plays with: its own plus swing, within +-49 % (the
    /// first step can't play before the clip starts).
    pub fn effective_offset(&self, c: &StepCell) -> i32 {
        let lo = if c.step == 0 { 0 } else { -49 };
        (c.offset as i32 + self.swing_offset(c.step)).clamp(lo, 49)
    }

    /// The row plays: not muted, and soloed when any row is.
    pub fn audible(&self, key: u8) -> bool {
        !self.muted.contains(&key) && (self.soloed.is_empty() || self.soloed.contains(&key))
    }

    /// Row M / S button: toggle the key in the muted (`solo` false) or soloed list.
    pub fn with_row_flag(&self, key: u8, solo: bool) -> StepPattern {
        let mut p = self.clone();
        let list = if solo { &mut p.soloed } else { &mut p.muted };
        match list.binary_search(&key) {
            Ok(i) => {
                list.remove(i);
            }
            Err(i) => list.insert(i, key),
        }
        p
    }

    /// Turn a step on (default lane values) or off.
    pub fn toggled(&self, key: u8, step: u16) -> StepPattern {
        let mut p = self.clone();
        match p.cells.binary_search_by_key(&(key, step), |c| (c.key, c.step)) {
            Ok(i) => {
                p.cells.remove(i);
            }
            Err(i) => p.cells.insert(i, StepCell::new(key, step)),
        }
        p
    }

    /// Set one lane of an active cell (no change when the step is off).
    pub fn with_lane(&self, key: u8, step: u16, lane: StepLane, v: i32) -> StepPattern {
        let mut p = self.clone();
        if let Ok(i) = p.cells.binary_search_by_key(&(key, step), |c| (c.key, c.step)) {
            p.cells[i].set(lane, v);
        }
        p
    }

    /// Change the length: cells past the new end are dropped, new steps are empty.
    pub fn resized(&self, steps: u16) -> StepPattern {
        let steps = steps.clamp(1, STEP_LIMIT);
        let mut p = self.clone();
        p.steps = steps;
        p.cells.retain(|c| c.step < steps);
        p
    }

    /// "Double": twice the length with the steps copied after themselves.
    /// None when that would pass MAX_STEPS ("Pattern already at max length").
    pub fn doubled(&self) -> Option<StepPattern> {
        let steps = self.steps.checked_mul(2).filter(|&s| s <= MAX_STEPS)?;
        let mut p = self.clone();
        p.steps = steps;
        p.cells.extend(self.cells.iter().map(|c| StepCell { step: c.step + self.steps, ..*c }));
        p.sort();
        Some(p)
    }

    /// "Clear": every cell off; length, rate and swing stay.
    pub fn cleared(&self) -> StepPattern {
        StepPattern { cells: Vec::new(), ..self.clone() }
    }

    /// Notes of the pattern (one pass; ids from 1, sorted).
    pub fn bake(&self) -> Vec<Note> {
        let mut out = Vec::new();
        for key in self.keys() {
            self.bake_row(key, &mut out);
        }
        finish(out, 1)
    }

    fn bake_row(&self, key: u8, out: &mut Vec<Note>) {
        let s = self.rate.ticks() as f64;
        let len = self.length() as f64;
        let gap = GATE_GAP as f64;
        let onset = |c: &StepCell| (c.step as f64 * s + self.effective_offset(c) as f64 * s / 100.0).round();
        let row = self.sounding(key);
        for (i, &(c, d)) in row.iter().enumerate() {
            let next_step = c.step + d + 1;
            // A hit on the very next step (wrapping to the pattern start)
            // cuts this one short so the two never overlap.
            let next = if next_step == self.steps {
                row.first().filter(|(n, _)| n.step == 0).map(|(n, _)| onset(n).max(0.0) + len)
            } else {
                row.get(i + 1).filter(|(n, _)| n.step == next_step).map(|(n, _)| onset(n))
            };
            let hits = c.repeat.max(1) as f64;
            let span = (1.0 + d as f64) * s / hits;
            let gate = if hits > 1.0 { (span * 0.5).max(span - gap) } else { span };
            let h = onset(&c);
            for k in 0..c.repeat.max(1) {
                let start = (h + k as f64 * span).max(0.0).round();
                // A hit with no room left before the next one is dropped.
                if start >= len || next.is_some_and(|n| n - gap - start < gap.min(gate) - 0.5) {
                    continue;
                }
                let length = match next {
                    Some(n) => gate.min(n - gap - start),
                    None => gate,
                };
                out.push(Note {
                    id: 0,
                    pitch: key,
                    start: start.round() as Ticks,
                    length: (length.round() as Ticks).max(1),
                    velocity: c.velocity,
                });
            }
        }
    }

    /// Read a pattern back from notes: hits are snapped to their nearest step
    /// (the offset keeps the difference), runs of equal hits closer than a
    /// gate gap become Repeat, and a hit's length becomes Tie. Notes on keys
    /// that are not rows, at or past the pattern end, or on a step already
    /// taken are left out.
    pub fn derive(notes: &[Note], rate: StepRate, steps: u16, swing: f32, rows: StepRows) -> StepPattern {
        let mut p = StepPattern { rate, steps: steps.clamp(1, STEP_LIMIT), swing, ..StepPattern::new(rows) };
        let mut keys: Vec<u8> = notes.iter().map(|n| n.pitch).filter(|&k| rows.contains(k)).collect();
        keys.sort_unstable();
        keys.dedup();
        for key in keys {
            p.derive_row(key, notes);
        }
        p.sort();
        p
    }

    fn derive_row(&mut self, key: u8, notes: &[Note]) {
        let s = self.rate.ticks();
        let len = self.length();
        let mut row: Vec<Note> = notes.iter().filter(|n| n.pitch == key && n.start < len).copied().collect();
        row.sort_by_key(|n| (n.start, n.length));
        let mut held_until: i64 = -1;
        let mut i = 0;
        while i < row.len() {
            let lead = row[i];
            let step = ((lead.start as f64 / s as f64).round() as i64).clamp(0, self.steps as i64 - 1);
            // Read the longest run of hits that bakes back to the same hits
            // as one cell's repeats (with, then without, a cut last hit);
            // else the lead alone.
            let run = (0..2).flat_map(|cut| (i + 2..=self.run_end(&row, i, cut == 0)).rev()).find_map(|j| {
                let (hits, tie) = self.resolve(&row, i, j, step, true);
                self.bakes_back(&row, i, j, step, hits, tie).then_some((j, hits, tie))
            });
            let (j, hits, tie) = run.unwrap_or_else(|| {
                let (hits, tie) = self.resolve(&row, i, i + 1, step, true);
                if hits > 1 && !self.bakes_back(&row, i, i + 1, step, hits, tie) {
                    let (hits, tie) = self.resolve(&row, i, i + 1, step, false);
                    return (i + 1, hits, tie);
                }
                (i + 1, hits, tie)
            });
            if step <= held_until {
                i = j;
                continue;
            }
            let tie = tie.min(self.steps as i64 - 1 - step).min(63);
            // The offset whose onset lands on the lead's tick (ticks are coarser than 1 %).
            let swing = self.swing_offset(step as u16);
            let measured = ((lead.start - step * s) as f64 * 100.0 / s as f64).round() as i32 - swing;
            let lands =
                |o: i32| ((step * s) as f64 + (o + swing).clamp(-49, 49) as f64 * s as f64 / 100.0).max(0.0).round() as i64 == lead.start;
            let offset = [measured, measured - 1, measured + 1].into_iter().find(|&o| lands(o)).unwrap_or(measured);
            let mut c = StepCell::new(key, step as u16);
            c.set(StepLane::Velocity, lead.velocity as i32);
            c.set(StepLane::Offset, offset);
            c.set(StepLane::Repeat, hits as i32);
            c.set(StepLane::Tie, tie as i32);
            self.cells.push(c);
            held_until = step + tie;
            i = j;
        }
    }

    /// Where the hit after row[j - 1] lands: the next hit of the row, or the
    /// first hit of the pattern's next repeat.
    fn next_hit(&self, row: &[Note], i: usize, j: usize) -> Option<Ticks> {
        row.get(j)
            .map(|n| n.start)
            .or_else(|| row.first().filter(|f| f.start < self.rate.ticks() / 2 && i > 0).map(|f| f.start + self.length()))
    }

    /// End (exclusive) of the longest run from row[i] that could be one
    /// cell's repeats: equal length (the last may be cut by the next hit),
    /// velocity and spacing, a gate gap between hits, and a spacing that
    /// splits whole steps into 2..=16. A hit a whole number of steps long is
    /// never a repeat.
    fn run_end(&self, row: &[Note], i: usize, with_cut: bool) -> usize {
        let s = self.rate.ticks();
        let lead = row[i];
        let Some(spacing) = row.get(i + 1).map(|n| n.start - lead.start) else { return i + 1 };
        let splits_steps = (2..=16).any(|f: i64| {
            let k = ((spacing * f) as f64 / s as f64).round() as i64;
            k >= 1 && (spacing * f - k * s).abs() <= f / 2 + 1
        });
        let gated = (lead.length - (spacing - GATE_GAP.min(spacing / 2))).abs() <= 1;
        if lead.length % s == 0 || !splits_steps || !gated {
            return i + 1;
        }
        let mut j = i + 1;
        while j < row.len() && j - i < 16 {
            let (prev, hit) = (row[j - 1], row[j]);
            let space = hit.start - (prev.start + prev.length);
            let gap_ok = space > 0 && ((space - GATE_GAP).abs() <= 1 || (space - lead.length).abs() <= 1);
            // A shorter last hit cut by the next one, unless it leads its own run of repeats.
            let leads_run = row.get(j + 1).is_some_and(|f| {
                f.velocity == hit.velocity && (f.length - hit.length).abs() <= 1 && f.start - (hit.start + hit.length) <= GATE_GAP + 1
            });
            let cut = with_cut
                && hit.length < lead.length
                && !leads_run
                && self.next_hit(row, i, j + 1).is_some_and(|at| (at - (hit.start + hit.length) - GATE_GAP).abs() <= 1);
            let even = ((hit.start - prev.start) - spacing).abs() <= 2;
            if !(gap_ok && (hit.length == lead.length || cut) && even && hit.velocity == lead.velocity) {
                break;
            }
            j += 1;
            if cut {
                break;
            }
        }
        j
    }

    /// Repeat and Tie of a cell whose hits are row[i..j].
    fn resolve(&self, row: &[Note], i: usize, j: usize, step: i64, lone_repeats: bool) -> (i64, i64) {
        let s = self.rate.ticks();
        let lead = row[i];
        let last = row[j - 1];
        let mut hits = (j - i) as i64;
        let mut tie = (((last.start + last.length - lead.start) as f64 / s as f64).round() as i64 - 1).max(0);
        if hits > 1 {
            // Hits cut off by the pattern end or the next hit still count:
            // the smallest split of whole steps that gives this spacing.
            let avg = (last.start - lead.start) as f64 / (hits - 1) as f64;
            if let Some((f, k)) = (hits..=16).find_map(|f| {
                let k = (avg * f as f64 / s as f64).round() as i64;
                (k >= 1 && (avg * f as f64 - (k * s) as f64).abs() <= f as f64 / (hits - 1) as f64 + 0.5).then_some((f, k))
            }) {
                hits = f;
                tie = k - 1;
            }
            return (hits, tie);
        }
        let next = self.next_hit(row, i, j);
        if let Some(at) = next {
            let next_step = if j < row.len() { (at as f64 / s as f64).round() as i64 } else { self.steps as i64 };
            if (at - (last.start + last.length) - GATE_GAP).abs() <= 1 && next_step > step {
                tie = next_step - step - 1;
            }
        }
        // A lone hit shorter than a whole step and than its room, or with no
        // room for a whole step's hit, is the first of repeats whose later
        // hits had no room.
        let room = next.map(|at| at - GATE_GAP - lead.start);
        if lone_repeats && lead.length % s != 0 && room.is_some_and(|r| lead.length < r || (r as f64) < GATE_GAP.min(s) as f64 - 0.5) {
            let p = lead.length + GATE_GAP.min(lead.length);
            if let Some((f, k)) = (2..=16).find_map(|f: i64| {
                let k = ((p * f) as f64 / s as f64).round() as i64;
                (k >= 1 && (p * f - k * s).abs() <= f).then_some((f, k))
            }) {
                hits = f;
                tie = k - 1;
            }
        }
        (hits, tie)
    }

    /// True when a cell at `step` with these repeats and tie bakes to
    /// exactly the starts of row[i..j] and no other hit.
    fn bakes_back(&self, row: &[Note], i: usize, j: usize, step: i64, hits: i64, tie: i64) -> bool {
        let s = self.rate.ticks() as f64;
        let lead = row[i];
        let span = (1 + tie.min(self.steps as i64 - 1 - step)) as f64 * s / hits as f64;
        let gate = (span * 0.5).max(span - GATE_GAP as f64);
        let next = self.next_hit(row, i, j).map_or(f64::MAX, |at| at as f64 - GATE_GAP as f64 - (GATE_GAP as f64).min(gate) + 0.5);
        let starts: Vec<i64> = (0..hits)
            .map(|k| (lead.start as f64 + k as f64 * span).round())
            .take_while(|&at| at < self.length() as f64 && at <= next)
            .map(|at| at as i64)
            .collect();
        starts.len() == j - i
            && starts.iter().zip(&row[i..j]).all(|(&a, n)| a == n.start)
            && row.get(j).is_none_or(|n| ((n.start as f64 / s).round() as i64) > step + tie)
    }

    /// A plain MIDI clip's notes as a new pattern (rate 1/16, the shortest
    /// length choice that covers the clip; a clip longer than 64 steps keeps
    /// its own step count, up to STEP_LIMIT).
    pub fn from_notes(notes: &[Note], clip_length: Ticks, rows: StepRows) -> StepPattern {
        let rate = StepRate::default();
        let need = (clip_length + rate.ticks() - 1) / rate.ticks();
        let steps = STEP_COUNTS.into_iter().find(|&c| c as Ticks >= need).unwrap_or(need.min(STEP_LIMIT as Ticks) as u16);
        StepPattern::derive(notes, rate, steps, 0.0, rows)
    }
}

/// Sort notes and number them from `first_id`.
fn finish(mut notes: Vec<Note>, first_id: u32) -> Vec<Note> {
    notes.sort_by_key(|n| (n.start, n.pitch, n.length));
    for (i, n) in notes.iter_mut().enumerate() {
        n.id = first_id + i as u32;
    }
    notes
}

/// The clip's new notes after a step edit: the pattern's notes, plus the
/// existing notes the editor can't show (keys that aren't rows), kept as they were.
pub fn bake_into(pattern: &StepPattern, existing: &[Note]) -> Vec<Note> {
    let mut out: Vec<Note> = existing.iter().filter(|n| !pattern.rows.contains(n.pitch)).copied().collect();
    for key in pattern.keys() {
        pattern.bake_row(key, &mut out);
    }
    finish(out, 1)
}

/// The pattern after a piano-roll edit from `old` to `new` notes: rows whose
/// notes did not change keep their cells exactly; changed rows are read back
/// from the new notes.
pub fn rederive(pattern: &StepPattern, old: &[Note], new: &[Note]) -> StepPattern {
    let shape = |notes: &[Note], k: u8| {
        let mut v: Vec<(Ticks, Ticks, u8)> = notes.iter().filter(|n| n.pitch == k).map(|n| (n.start, n.length, n.velocity)).collect();
        v.sort_unstable();
        v
    };
    let mut keys: Vec<u8> = old.iter().chain(new).map(|n| n.pitch).filter(|&k| pattern.rows.contains(k)).collect();
    keys.sort_unstable();
    keys.dedup();
    let mut p = pattern.clone();
    for k in keys {
        if shape(old, k) == shape(new, k) {
            continue;
        }
        p.cells.retain(|c| c.key != k);
        p.derive_row(k, new);
    }
    p.sort();
    p
}

/// Notes of a pattern clip as they play: the first `period` ticks repeated
/// up to `clip_length` (notes are cut at the clip end).
pub fn tile(notes: &[Note], period: Ticks, clip_length: Ticks) -> Vec<Note> {
    if period <= 0 {
        return notes.to_vec();
    }
    let mut out = Vec::new();
    let mut at = 0;
    while at < clip_length {
        for n in notes.iter().filter(|n| n.start < period) {
            let start = at + n.start;
            if start < clip_length {
                out.push(Note { start, length: n.length.min(clip_length - start), ..*n });
            }
        }
        at += period;
    }
    finish(out, 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pad(n: u8) -> u8 {
        drum_rack::FIRST_NOTE + n
    }

    fn four_on_floor() -> StepPattern {
        let mut p = StepPattern::default();
        for s in [0, 4, 8, 12] {
            p = p.toggled(pad(0), s);
        }
        p.toggled(pad(1), 4).toggled(pad(2), 8)
    }

    #[test]
    fn rates_and_lengths() {
        assert_eq!(StepRate::Sixteenth.ticks(), 240);
        assert_eq!(StepRate::EighthTriplet.ticks(), 320);
        assert_eq!(StepRate::ThirtySecondTriplet.ticks(), 80);
        assert_eq!(StepRate::from_label("1/16T"), Some(StepRate::SixteenthTriplet));
        assert_eq!(StepPattern::default().length(), 4 * PPQ);
        assert_eq!(StepLane::Tie.format(1), "\u{2192} 1 step");
        assert_eq!(StepLane::Offset.format(12), "+12%");
        assert_eq!(StepLane::Offset.format(-3), "-3%");
    }

    #[test]
    fn bake_plain_steps() {
        let notes = four_on_floor().bake();
        assert_eq!(notes.len(), 6);
        assert!(notes.iter().all(|n| n.length == 240 && n.velocity == 100));
        assert_eq!(notes.iter().filter(|n| n.pitch == pad(0)).map(|n| n.start).collect::<Vec<_>>(), [0, 960, 1920, 2880]);
    }

    #[test]
    fn bake_lanes() {
        let p = StepPattern::default().toggled(pad(0), 2).with_lane(pad(0), 2, StepLane::Repeat, 3).with_lane(
            pad(0),
            2,
            StepLane::Velocity,
            64,
        );
        let n = p.bake();
        assert_eq!(n.iter().map(|n| n.start).collect::<Vec<_>>(), [480, 560, 640]);
        assert!(n.iter().all(|n| n.length == 80 - GATE_GAP && n.velocity == 64));
        let t = StepPattern::default().toggled(pad(0), 0).with_lane(pad(0), 0, StepLane::Tie, 3).bake();
        assert_eq!((t[0].start, t[0].length), (0, 960));
        let o = StepPattern::default().toggled(pad(0), 4).with_lane(pad(0), 4, StepLane::Offset, -25).bake();
        assert_eq!(o[0].start, 960 - 60);
    }

    #[test]
    fn tie_covers_and_is_cut_by_the_next_hit() {
        let p = StepPattern::default().toggled(pad(0), 0).toggled(pad(0), 2).toggled(pad(0), 3).with_lane(pad(0), 0, StepLane::Tie, 2);
        assert!(p.is_covered(pad(0), 2) && !p.is_covered(pad(0), 3));
        let n = p.bake();
        assert_eq!(n.len(), 2, "the covered step does not sound");
        assert_eq!((n[0].start, n[0].length), (0, 720 - GATE_GAP));
        let wrap = StepPattern::default().toggled(pad(0), 15).toggled(pad(0), 0).with_lane(pad(0), 0, StepLane::Offset, -20).with_lane(
            pad(0),
            15,
            StepLane::Tie,
            0,
        );
        let n = wrap.bake();
        let last = n.iter().find(|n| n.start >= 3600).unwrap();
        assert_eq!(last.length, 240 - GATE_GAP, "cut by the next loop's first hit (played at the pattern start)");
    }

    #[test]
    fn swing_pushes_odd_steps() {
        let mut p = StepPattern::default().toggled(pad(3), 0).toggled(pad(3), 1);
        p.swing = 0.5;
        let n = p.bake();
        assert_eq!(n[1].start, 240 + 60, "round(0.5 x 49) = 25 % of a step");
    }

    #[test]
    fn double_resize_clear() {
        let p = four_on_floor();
        let d = p.doubled().unwrap();
        assert_eq!((d.steps, d.cells.len()), (32, 12));
        assert!(d.validate().is_ok());
        assert!(d.resized(64).doubled().is_none(), "already at max length");
        assert_eq!(d.resized(8).cells.len(), 3);
        assert!(p.cleared().cells.is_empty() && p.cleared().steps == 16);
    }

    #[test]
    fn derive_is_the_inverse_of_bake() {
        let mut p = four_on_floor()
            .toggled(pad(5), 1)
            .with_lane(pad(5), 1, StepLane::Repeat, 4)
            .with_lane(pad(5), 1, StepLane::Tie, 1)
            .toggled(pad(6), 6)
            .with_lane(pad(6), 6, StepLane::Offset, 20)
            .with_lane(pad(6), 6, StepLane::Velocity, 30)
            .toggled(pad(6), 9)
            .with_lane(pad(6), 9, StepLane::Tie, 2)
            .toggled(pad(6), 12)
            .with_lane(pad(6), 12, StepLane::Offset, -30);
        p.swing = 0.3;
        let back = StepPattern::derive(&p.bake(), p.rate, p.steps, p.swing, p.rows);
        assert_eq!(back, p);
    }

    #[test]
    fn derive_is_stable_over_random_patterns() {
        let mut s: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut rnd = |m: u32| {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            (s % m as u64) as i32
        };
        let (mut rows, mut bad) = (0, 0);
        for _ in 0..5000 {
            let mut p = StepPattern::default();
            p.rate = StepRate::ALL[rnd(9) as usize];
            p.steps = STEP_COUNTS[rnd(8) as usize];
            p.swing = rnd(5) as f32 / 4.0;
            for _ in 0..rnd(40) {
                let (k, st) = (pad(rnd(16) as u8), rnd(p.steps as u32) as u16);
                if p.cell(k, st).is_some() {
                    continue;
                }
                p = p.toggled(k, st);
                for lane in StepLane::ALL {
                    if rnd(3) == 0 {
                        let sp = lane.spec();
                        // Up to 4 repeats, at most every 12 ticks (a 1/320 note).
                        // Dense repeats next to other hits can read back in
                        // more than one way, see derive().
                        let hi = match lane {
                            StepLane::Tie => 4,
                            StepLane::Repeat => (p.rate.ticks() / 12).clamp(1, 4) as i32,
                            _ => sp.max,
                        };
                        p = p.with_lane(k, st, lane, sp.min + rnd((hi - sp.min + 1) as u32));
                    }
                }
            }
            assert!(p.validate().is_ok());
            let notes = p.bake();
            assert!(crate::midi::validate_notes(&notes).is_ok());
            let back = StepPattern::derive(&notes, p.rate, p.steps, p.swing, p.rows);
            // The same hits (start, pitch, velocity); a length may differ by
            // up to the gate gap where two readings sound the same.
            let again = back.bake();
            let same = |a: &Note, b: &Note| {
                (a.start, a.pitch, a.velocity) == (b.start, b.pitch, b.velocity) && (a.length - b.length).abs() <= GATE_GAP
            };
            let row_eq = |k: u8| {
                let (x, y): (Vec<&Note>, Vec<&Note>) =
                    (again.iter().filter(|n| n.pitch == k).collect(), notes.iter().filter(|n| n.pitch == k).collect());
                x.len() == y.len() && x.iter().zip(&y).all(|(a, b)| same(a, b))
            };
            for k in p.keys() {
                rows += 1;
                if !row_eq(k) {
                    bad += 1;
                }
            }
        }
        // A few dense rows have two readings that bake differently (e.g. a
        // lone hit right before evenly spaced repeats of the same length and
        // velocity); allow 1 in 10 000.
        assert!(rows > 40_000 && bad * 10_000 <= rows, "{bad} of {rows} rows read back differently");
    }

    #[test]
    fn rederive_keeps_untouched_rows() {
        let p = four_on_floor().toggled(pad(4), 3).with_lane(pad(4), 3, StepLane::Repeat, 2).with_lane(pad(4), 3, StepLane::Tie, 5);
        let old = p.bake();
        let mut new = old.clone();
        let snare = new.iter_mut().find(|n| n.pitch == pad(1)).unwrap();
        snare.start += 960;
        let q = rederive(&p, &old, &new);
        assert_eq!(q.cell(pad(1), 8).map(|c| c.step), Some(8));
        assert!(q.cell(pad(1), 4).is_none());
        assert_eq!(q.cell(pad(4), 3), p.cell(pad(4), 3));
    }

    #[test]
    fn bake_into_keeps_notes_off_the_rows() {
        let odd = Note { id: 9, pitch: 60, start: 100, length: 50, velocity: 90 };
        let n = bake_into(&four_on_floor(), &[odd]);
        assert_eq!(n.len(), 7);
        assert!(n.iter().any(|x| x.pitch == 60 && x.start == 100));
    }

    #[test]
    fn a_clip_longer_than_64_steps_keeps_its_own_step_count() {
        let s = PPQ / 4;
        let notes = [
            Note { id: 1, pitch: pad(0), start: 0, length: s, velocity: 90 },
            Note { id: 2, pitch: pad(1), start: 100 * s, length: s, velocity: 70 },
        ];
        let p = StepPattern::from_notes(&notes, 25 * PPQ + 1, StepRows::Pads);
        assert_eq!(p.steps, 101, "no snapping past 64 steps");
        assert!(p.validate().is_ok());
        assert_eq!(p.cell(pad(1), 100).map(|c| c.velocity), Some(70));
        assert!(p.doubled().is_none(), "Double stays within 64 steps");
        assert_eq!(StepPattern::from_notes(&notes, 9000 * PPQ, StepRows::Pads).steps, STEP_LIMIT);
        assert_eq!(StepPattern::from_notes(&notes, 9 * PPQ, StepRows::Pads).steps, 36, "a short clip still takes a Length choice");
    }

    #[test]
    fn from_notes_and_tile() {
        let notes = four_on_floor().bake();
        let p = StepPattern::from_notes(&notes, 4 * PPQ, StepRows::Pads);
        assert_eq!(p, four_on_floor());
        let t = tile(&notes, 4 * PPQ, 10 * PPQ);
        assert_eq!(t.len(), 6 + 6 + 3);
        assert!(t.iter().all(|n| n.start + n.length <= 10 * PPQ));
        let pitched =
            StepPattern::from_notes(&[Note { id: 1, pitch: 60, start: 0, length: 240, velocity: 80 }], 6 * PPQ, StepRows::Pitches);
        assert_eq!(pitched.steps, 24);
        assert_eq!(pitched.cells.len(), 1);
    }
}
