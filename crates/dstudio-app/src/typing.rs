//! Musical typing: the computer keyboard played as a piano while the piano roll has the focus
//! (K, Shift+K or the header button turn it on). Notes go through the clip's track instrument
//! like a key preview (`Controller::pr_preview`), several at once, and light their keys on the
//! piano roll's keyboard. Layout, octave and velocity rules follow the standard mode;
//! the floating panel and its chord, strum and follow modes are not built.

use crate::controller::Controller;

pub const DEFAULT_OCTAVE: i32 = 3;
pub const MAX_OCTAVE: i32 = 8;
pub const DEFAULT_VELOCITY: f32 = 0.7;
pub const MIN_VELOCITY: f32 = 0.05;
/// One C / V press: 10 of 127.
pub const VELOCITY_STEP: f32 = 10.0 / 127.0;

/// Semitones above the octave's C for a note key: the home row plays the white keys from C
/// (A = C ... ' = F an octave and a half up), the row above the black keys between them.
pub fn key_offset(key: char) -> Option<i32> {
    Some(match key {
        'a' => 0,
        'w' => 1,
        's' => 2,
        'e' => 3,
        'd' => 4,
        'f' => 5,
        't' => 6,
        'g' => 7,
        'y' => 8,
        'h' => 9,
        'u' => 10,
        'j' => 11,
        'k' => 12,
        'o' => 13,
        'l' => 14,
        'p' => 15,
        ';' => 16,
        '\'' => 17,
        _ => return None,
    })
}

/// MIDI note of `key` at `octave` (middle C = 60 is octave 4, so the default octave 3 starts at 48).
pub fn key_pitch(key: char, octave: i32) -> Option<u8> {
    u8::try_from((octave + 1) * 12 + key_offset(key)?).ok().filter(|&p| p < 128)
}

/// MIDI velocity of a typing velocity fraction (0.05..1).
pub fn midi_velocity(v: f32) -> u8 {
    (v * 127.0).round().clamp(1.0, 127.0) as u8
}

#[derive(Debug)]
pub struct Typing {
    pub active: bool,
    pub octave: i32,
    pub velocity: f32,
    /// Sounding notes: (key, instrument device, pitch).
    held: Vec<(char, u64, u8)>,
}

impl Default for Typing {
    fn default() -> Self {
        Typing { active: false, octave: DEFAULT_OCTAVE, velocity: DEFAULT_VELOCITY, held: Vec::new() }
    }
}

impl Typing {
    pub fn is_holding(&self) -> bool {
        !self.held.is_empty()
    }

    /// The held pitches as a bit set (bit = pitch).
    pub fn lit(&self) -> u128 {
        self.held.iter().fold(0, |m, &(_, _, pitch)| m | 1u128 << pitch)
    }
}

impl Controller {
    pub fn typing_toggle(&mut self) {
        self.typing.active = !self.typing.active;
        if !self.typing.active {
            self.typing_release_all();
        }
        self.ui_dirty = true;
    }

    /// A key went down or up (`text` is its character). Returns true when typing used it, so the
    /// shortcuts don't: only while typing is on, the piano roll has the focus, its track has an
    /// instrument and no modifier is held; then the note keys, Z / X (octave) and C / V (velocity)
    /// are taken, which is why D, G, H, L and R stop being shortcuts meanwhile. A key already held
    /// by auto-repeat is taken without sounding again. A release always lets go of its note.
    pub fn typing_key(&mut self, text: &str, down: bool, repeat: bool, modified: bool) -> bool {
        let mut chars = text.chars();
        let (Some(key), None) = (chars.next(), chars.next()) else { return false };
        let key = key.to_ascii_lowercase();
        if !down {
            return self.typing_release(key);
        }
        let claimed = key_offset(key).is_some() || matches!(key, 'z' | 'x' | 'c' | 'v');
        if !claimed || modified || !self.typing.active || !self.pr_visible() || !self.pr.focused {
            return false;
        }
        let Some(device) = self.pr_device() else { return false };
        if repeat {
            return true;
        }
        self.ui_dirty = true;
        let t = &mut self.typing;
        match key {
            'z' => t.octave = (t.octave - 1).max(0),
            'x' => t.octave = (t.octave + 1).min(MAX_OCTAVE),
            'c' => t.velocity = (t.velocity - VELOCITY_STEP).max(MIN_VELOCITY),
            'v' => t.velocity = (t.velocity + VELOCITY_STEP).min(1.0),
            _ => {
                self.typing_release(key);
                if let Some(pitch) = key_pitch(key, self.typing.octave)
                    && self.engine.preview_note(device, pitch, midi_velocity(self.typing.velocity)).is_ok()
                {
                    self.typing.held.push((key, device, pitch));
                }
            }
        }
        true
    }

    /// Lets go of the note `key` holds; the engine's note-off is retried if its queue is full.
    fn typing_release(&mut self, key: char) -> bool {
        let Some(i) = self.typing.held.iter().position(|h| h.0 == key) else { return false };
        let (_, device, pitch) = self.typing.held.remove(i);
        self.pr.preview_offs.push((device, pitch));
        self.pr_retry_note_offs();
        self.ui_dirty = true;
        true
    }

    pub fn typing_release_all(&mut self) {
        if !self.typing.is_holding() {
            return;
        }
        self.pr.preview_offs.extend(self.typing.held.drain(..).map(|(_, device, pitch)| (device, pitch)));
        self.pr_retry_note_offs();
        self.ui_dirty = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_is_a_piano_from_c() {
        let whites: Vec<i32> = "asdfghjkl;'".chars().map(|k| key_offset(k).unwrap()).collect();
        assert_eq!(whites, [0, 2, 4, 5, 7, 9, 11, 12, 14, 16, 17], "C D E F G A B C D E F");
        let blacks: Vec<i32> = "wetyuop".chars().map(|k| key_offset(k).unwrap()).collect();
        assert_eq!(blacks, [1, 3, 6, 8, 10, 13, 15], "C# D# F# G# A# C# D#");
        assert_eq!(key_offset('z'), None);
        assert_eq!(key_offset('1'), None);
    }

    #[test]
    fn octave_and_velocity_rules() {
        assert_eq!(key_pitch('a', DEFAULT_OCTAVE), Some(48));
        assert_eq!(key_pitch('a', 4), Some(60));
        assert_eq!(key_pitch('k', 4), Some(72));
        assert_eq!(key_pitch('a', 0), Some(12));
        assert_eq!(key_pitch('a', MAX_OCTAVE), Some(108));
        assert_eq!(key_pitch('\'', MAX_OCTAVE), Some(125));
        assert_eq!(key_pitch('a', 10), None, "past MIDI 127");
        assert_eq!(midi_velocity(DEFAULT_VELOCITY), 89);
        assert_eq!(midi_velocity(1.0), 127);
        assert_eq!(midi_velocity(MIN_VELOCITY), 6);
        assert_eq!(midi_velocity(DEFAULT_VELOCITY + VELOCITY_STEP), 99, "one V press is ten velocity steps");
    }

    #[test]
    fn held_notes_light_their_pitches() {
        let t = Typing { held: vec![('a', 1, 48), ('d', 1, 52), ('k', 1, 127)], ..Default::default() };
        assert_eq!(t.lit(), 1u128 << 48 | 1u128 << 52 | 1u128 << 127);
        assert_eq!(Typing::default().lit(), 0);
    }
}
