//! The transport bar's project fields: tempo (drag, type, tap), time signature (menu) and key
//! (picker). Each gesture is one undo step: a drag or a series of taps replaces its own
//! "Tempo Change" entry instead of stacking one per move or tap.

use crate::controller::{Controller, sync_model};
use crate::{ChoiceItem, Studio};
use dstudio_core::Command;
use dstudio_core::command::{MAX_BPM, MIN_BPM};
use dstudio_core::model::ProjectKey;
use slint::VecModel;
use std::rc::Rc;
use std::time::Instant;

/// Pitch-class names for the key picker and the readout (sharps, as elsewhere in the app).
pub const KEY_NAMES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

/// Time signatures the Sig menu offers, simple then compound.
pub const SIGNATURES: [(u8, u8); 12] = [
    (2, 4),
    (3, 4),
    (4, 4),
    (5, 4),
    (6, 4),
    (7, 4),
    (3, 8),
    (5, 8),
    (6, 8),
    (7, 8),
    (9, 8),
    (12, 8),
];
/// Index of the first x/8 entry (a rule goes before it).
const FIRST_EIGHTH: usize = 6;

/// A tap more than this long after the previous one starts a new series.
const TAP_RESET_SECS: f64 = 2.0;
/// Taps remembered per series (the tempo is their average).
const TAP_MEMORY: usize = 8;
/// A new interval this far (as a fraction) from the series average starts a new series from the
/// previous tap: the player changed tempo.
const TAP_JUMP: f64 = 0.4;
/// Pixels of drag before a press on the tempo counts as a drag.
const DRAG_DEAD_ZONE: f32 = 2.0;

/// Menu metrics in design units (as the timeline snap menu).
const ROW_H: f32 = 18.7;
const PAD: f32 = 3.4;
const RULE_GAP: f32 = 6.8;
pub const SIG_MENU_W: f32 = 74.8;

pub struct State {
    /// Tap times (seconds since `epoch`) of the current series.
    taps: Vec<f64>,
    epoch: Instant,
    /// Tempo drag: the tempo at the press, and whether the pointer has moved past the dead zone.
    drag: Option<(f64, bool)>,
    /// The document revision right after this gesture's last tempo edit: while it is still
    /// current, the next edit of the same gesture replaces it.
    merge_rev: Option<u64>,
    /// The Sig menu and the key picker (they open below their fields).
    pub sig_menu: bool,
    pub key_menu: bool,
    /// The picker's Major / Minor choice when the project has no key yet.
    pub key_minor: bool,
    pub sig_model: Rc<VecModel<ChoiceItem>>,
}

impl State {
    pub fn new() -> State {
        State {
            taps: Vec::new(),
            epoch: Instant::now(),
            drag: None,
            merge_rev: None,
            sig_menu: false,
            key_menu: false,
            key_minor: false,
            sig_model: Rc::new(VecModel::default()),
        }
    }
}

/// The tempo readout: whole tempos without decimals, others with up to two.
pub fn fmt_bpm(bpm: f64) -> String {
    let r = (bpm * 100.0).round() / 100.0;
    if (r - r.round()).abs() < 1e-9 {
        format!("{}", r.round() as i64)
    } else {
        let s = format!("{r:.2}");
        s.trim_end_matches('0').to_string()
    }
}

/// A typed tempo: a number within the allowed range, kept to two decimals.
pub fn parse_bpm(text: &str) -> Option<f64> {
    let v: f64 = text
        .trim()
        .trim_end_matches(|c: char| c.is_ascii_alphabetic() || c.is_whitespace())
        .trim()
        .replace(',', ".")
        .parse()
        .ok()?;
    if !v.is_finite() {
        return None;
    }
    Some(((v * 100.0).round() / 100.0).clamp(MIN_BPM, MAX_BPM))
}

/// The key readout: "C", "F#m"; empty when the project has no key.
pub fn key_text(key: Option<ProjectKey>) -> String {
    key.map_or(String::new(), |k| {
        format!(
            "{}{}",
            KEY_NAMES[k.root as usize % 12],
            if k.minor { "m" } else { "" }
        )
    })
}

/// Add a tap at `t` seconds to `taps` and return the series' tempo (whole BPM) once it has two
/// taps. A long pause or a sudden change of pace starts a new series.
pub fn tap(taps: &mut Vec<f64>, t: f64) -> Option<f64> {
    if let Some(&last) = taps.last() {
        let gap = t - last;
        if !(0.0..=TAP_RESET_SECS).contains(&gap) {
            taps.clear();
        } else if taps.len() >= 2 {
            let avg = (last - taps[0]) / (taps.len() - 1) as f64;
            if (gap - avg).abs() > TAP_JUMP * avg {
                taps.drain(..taps.len() - 1);
            }
        }
    }
    taps.push(t);
    if taps.len() > TAP_MEMORY {
        taps.remove(0);
    }
    if taps.len() < 2 {
        return None;
    }
    let avg = (taps[taps.len() - 1] - taps[0]) / (taps.len() - 1) as f64;
    (avg > 0.0).then(|| (60.0 / avg).round().clamp(MIN_BPM, MAX_BPM))
}

/// Sig menu rows (the current signature is checked; one not in the list is shown first) and the
/// menu's height and rule y.
pub fn sig_rows(current: (u8, u8)) -> (Vec<ChoiceItem>, f32, f32) {
    let mut list: Vec<(u8, u8)> = Vec::new();
    if !SIGNATURES.contains(&current) {
        list.push(current);
    }
    let extra = list.len();
    list.extend(SIGNATURES);
    let mut rows = Vec::new();
    let mut y = PAD;
    let mut rule = 0.0;
    for (i, s) in list.iter().enumerate() {
        if i == extra + FIRST_EIGHTH {
            rule = y + RULE_GAP / 2.0;
            y += RULE_GAP;
        }
        rows.push(ChoiceItem {
            label: format!("{}/{}", s.0, s.1).into(),
            checked: *s == current,
            hint: false,
            y,
            h: ROW_H,
        });
        y += ROW_H;
    }
    (rows, y + PAD, rule)
}

impl Controller {
    fn current_sig(&self) -> (u8, u8) {
        let ts = &self.project().transport.time_signature;
        (ts.numerator, ts.denominator)
    }

    /// Set the tempo as part of the current gesture: replaces the gesture's previous edit when
    /// nothing else happened since, so the gesture is one undo step.
    fn tempo_gesture_set(&mut self, bpm: f64) {
        if (self.project().transport.bpm - bpm).abs() < 1e-9 {
            return;
        }
        if self.tf.merge_rev == Some(self.doc.revision()) {
            // Our own last edit: take it back so the new value replaces it.
            let _ = self.doc.undo();
        }
        self.doc.seal_coalescing();
        if self.execute(Command::SetTempo { bpm }) {
            self.doc.seal_coalescing();
            self.tf.merge_rev = Some(self.doc.revision());
        }
    }

    /// Tempo drag on the readout: phase 0 press, 1 move (offset from the press), 2 release.
    /// One BPM per point, right or up.
    pub fn tempo_drag(&mut self, phase: i32, dx: f32, dy: f32) {
        match phase {
            0 => {
                self.tf.drag = Some((self.project().transport.bpm.round(), false));
                self.tf.merge_rev = None;
            }
            1 => {
                let Some((start, moved)) = self.tf.drag else {
                    return;
                };
                let acc = dx - dy;
                if !moved && acc.abs() <= DRAG_DEAD_ZONE {
                    return;
                }
                self.tf.drag = Some((start, true));
                self.tempo_gesture_set((start + acc.round() as f64).clamp(MIN_BPM, MAX_BPM));
            }
            _ => {
                self.tf.drag = None;
                self.tf.merge_rev = None;
            }
        }
        self.ui_dirty = true;
    }

    /// The typed tempo (double-click on the readout).
    pub fn tempo_text(&mut self, text: &str) {
        match parse_bpm(text) {
            Some(bpm) => {
                self.tf.merge_rev = None;
                if (self.project().transport.bpm - bpm).abs() > 1e-9 {
                    self.doc.seal_coalescing();
                    self.execute(Command::SetTempo { bpm });
                    self.doc.seal_coalescing();
                }
            }
            None => self.set_status(
                format!("Tempo: type a number from {MIN_BPM} to {MAX_BPM}"),
                3,
            ),
        }
        self.ui_dirty = true;
    }

    /// TAP: two or more taps set the tempo to their average; one series is one undo step.
    pub fn tap_tempo(&mut self) {
        let t = self.tf.epoch.elapsed().as_secs_f64();
        self.tap_tempo_at(t);
    }

    pub(crate) fn tap_tempo_at(&mut self, t: f64) {
        let fresh = self.tf.taps.last().is_none_or(|&l| t - l > TAP_RESET_SECS);
        if fresh {
            self.tf.merge_rev = None;
        }
        if let Some(bpm) = tap(&mut self.tf.taps, t) {
            self.tempo_gesture_set(bpm);
        }
        self.ui_dirty = true;
    }

    pub fn sig_menu_toggle(&mut self) {
        self.tf.sig_menu = !self.tf.sig_menu;
        self.tf.key_menu = false;
        self.ui_dirty = true;
    }

    /// Pick Sig menu row `i` (rows as `sig_rows` lists them).
    pub fn sig_menu_pick(&mut self, i: usize) {
        let cur = self.current_sig();
        let (rows, _, _) = sig_rows(cur);
        self.tf.sig_menu = false;
        self.ui_dirty = true;
        let Some(r) = rows.get(i) else { return };
        let Some((n, d)) = r
            .label
            .split_once('/')
            .and_then(|(n, d)| Some((n.parse().ok()?, d.parse().ok()?)))
        else {
            return;
        };
        if (n, d) != cur {
            self.doc.seal_coalescing();
            self.execute(Command::SetTimeSignature {
                numerator: n,
                denominator: d,
            });
        }
    }

    pub fn key_menu_toggle(&mut self) {
        self.tf.key_menu = !self.tf.key_menu;
        self.tf.sig_menu = false;
        if let Some(k) = self.project().transport.key {
            self.tf.key_minor = k.minor;
        }
        self.ui_dirty = true;
    }

    /// Key picker: a root (0 = C .. 11 = B) in the picker's mode. Leaves the picker open.
    pub fn key_pick(&mut self, root: u8) {
        let key = Some(ProjectKey {
            root: root % 12,
            minor: self.tf.key_minor,
        });
        self.set_project_key(key);
    }

    /// Key picker: Major / Minor. Changes the project's key when it has one.
    pub fn key_mode(&mut self, minor: bool) {
        self.tf.key_minor = minor;
        if let Some(k) = self.project().transport.key {
            self.set_project_key(Some(ProjectKey { minor, ..k }));
        }
        self.ui_dirty = true;
    }

    /// Key picker: No key.
    pub fn key_clear(&mut self) {
        self.set_project_key(None);
        self.tf.key_menu = false;
    }

    fn set_project_key(&mut self, key: Option<ProjectKey>) {
        if self.project().transport.key != key {
            self.doc.seal_coalescing();
            self.execute(Command::SetProjectKey { key });
        }
        self.ui_dirty = true;
    }

    /// Close the Sig and Key popups; true if one was open.
    pub fn transport_menus_dismiss(&mut self) -> bool {
        let open = self.tf.sig_menu || self.tf.key_menu;
        self.tf.sig_menu = false;
        self.tf.key_menu = false;
        self.ui_dirty |= open;
        open
    }

    pub(crate) fn push_transport_fields(&mut self, s: &Studio) {
        let p = self.project();
        let (bpm, key) = (fmt_bpm(p.transport.bpm), p.transport.key);
        let cur = self.current_sig();
        if s.get_bpm_text() != bpm.as_str() {
            s.set_bpm_text(bpm.into());
        }
        let kt = key_text(key);
        if s.get_key_text() != kt.as_str() {
            s.set_key_text(kt.into());
        }
        s.set_key_root(key.map_or(-1, |k| k.root as i32));
        s.set_key_minor(key.map_or(self.tf.key_minor, |k| k.minor));
        s.set_key_menu_open(self.tf.key_menu);
        if self.tf.sig_menu {
            let (rows, h, rule) = sig_rows(cur);
            sync_model(&self.tf.sig_model, rows);
            s.set_sig_menu_h(h);
            s.set_sig_menu_w(SIG_MENU_W);
            s.set_sig_menu_rule(rule);
        }
        s.set_sig_menu_open(self.tf.sig_menu);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tempo_readout_and_typing() {
        assert_eq!(fmt_bpm(120.0), "120");
        assert_eq!(fmt_bpm(97.5), "97.5");
        assert_eq!(fmt_bpm(128.254), "128.25");
        assert_eq!(parse_bpm(" 140 "), Some(140.0));
        assert_eq!(parse_bpm("92,5 bpm"), Some(92.5));
        assert_eq!(parse_bpm("1500"), Some(MAX_BPM));
        assert_eq!(parse_bpm("5"), Some(MIN_BPM));
        assert_eq!(parse_bpm("fast"), None);
        assert_eq!(parse_bpm("NaN"), None);
    }

    #[test]
    fn taps_average_and_restart() {
        let mut taps = Vec::new();
        assert_eq!(tap(&mut taps, 10.0), None);
        // 0.5 s apart: 120 BPM.
        assert_eq!(tap(&mut taps, 10.5), Some(120.0));
        assert_eq!(tap(&mut taps, 11.0), Some(120.0));
        // Slight human jitter averages out.
        assert_eq!(tap(&mut taps, 11.52), Some(118.0));
        // A long pause starts over.
        assert_eq!(tap(&mut taps, 20.0), None);
        assert_eq!(tap(&mut taps, 20.75), Some(80.0));
        assert_eq!(tap(&mut taps, 21.5), Some(80.0));
        // Suddenly twice as fast: the new pace wins at once.
        assert_eq!(tap(&mut taps, 21.875), Some(160.0));
        // Only the last few taps count.
        for i in 1..20 {
            tap(&mut taps, 21.875 + i as f64 * 0.375);
        }
        assert!(taps.len() <= TAP_MEMORY);
    }

    #[test]
    fn sig_menu_checks_current_and_lists_unknown_first() {
        let (rows, h, rule) = sig_rows((4, 4));
        assert_eq!(rows.len(), SIGNATURES.len());
        assert_eq!(
            rows.iter()
                .filter(|r| r.checked)
                .map(|r| r.label.as_str())
                .collect::<Vec<_>>(),
            ["4/4"]
        );
        assert!(rule > rows[FIRST_EIGHTH - 1].y + ROW_H && rule < rows[FIRST_EIGHTH].y);
        assert!(h > rows.last().unwrap().y + ROW_H);
        let (rows, _, _) = sig_rows((11, 16));
        assert_eq!(rows[0].label.as_str(), "11/16");
        assert!(rows[0].checked);
    }

    #[test]
    fn key_readout() {
        assert_eq!(key_text(None), "");
        assert_eq!(
            key_text(Some(ProjectKey {
                root: 0,
                minor: false
            })),
            "C"
        );
        assert_eq!(
            key_text(Some(ProjectKey {
                root: 6,
                minor: true
            })),
            "F#m"
        );
    }
}
