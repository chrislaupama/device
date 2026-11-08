//! Parameters of the built-in instrument ("Synth") on instrument tracks.
//!
//! This is the contract between the Devices panel (UI), the project file and
//! the engine: the panel edits a `SynthParams` value through
//! `Command::SetSynthParams` (one undo step per gesture, knob drags
//! coalesce), the snapshot carries it to the engine, and the engine maps it
//! onto its DSP. The sound design and preset names are original.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Waveform {
    #[default]
    Saw,
    Square,
    Triangle,
    Sine,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum FilterMode {
    #[default]
    LowPass,
    HighPass,
    BandPass,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Osc {
    pub wave: Waveform,
    /// 0..1. Square: pulse width 50%→5%; Saw/Triangle: morph toward the other
    /// shape; Sine: wavefold amount.
    pub shape: f32,
    /// -3..=3 octaves.
    pub octave: i8,
    /// -12..=12 semitones.
    pub semitones: i8,
    /// -100..=100 cents.
    pub fine_cents: f32,
    /// 0..1 linear.
    pub level: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Adsr {
    /// Seconds, 0.001..=10.
    pub attack: f32,
    /// Seconds, 0.001..=10.
    pub decay: f32,
    /// 0..1.
    pub sustain: f32,
    /// Seconds, 0.001..=20.
    pub release: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Filter {
    pub mode: FilterMode,
    /// Hz, 20..=20000 (at key C4 when key tracking is on).
    pub cutoff_hz: f32,
    /// 0..1 (1 = just below self-oscillation).
    pub resonance: f32,
    /// 0..1: how much cutoff follows the played key (1 = one octave per octave).
    pub key_track: f32,
    /// -1..1: filter-envelope depth, scaled to ±6 octaves.
    pub env_amount: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Lfo {
    pub wave: Waveform,
    /// Hz, 0.01..=20.
    pub rate_hz: f32,
    /// Vibrato depth, cents 0..=1200.
    pub to_pitch_cents: f32,
    /// Filter sweep depth, octaves 0..=6.
    pub to_cutoff_oct: f32,
    /// Tremolo depth 0..1.
    pub to_amp: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Voice {
    /// 1..=16.
    pub polyphony: u8,
    /// Portamento time in seconds, 0..=2.
    pub glide: f32,
    /// Stacked voices per note, 1..=4.
    pub unison: u8,
    /// Spread of the unison stack, cents 0..=50.
    pub unison_detune_cents: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SynthParams {
    pub osc: [Osc; 2],
    /// Osc 2 frequency-modulates osc 1, 0..1.
    pub cross_fm: f32,
    /// Sine one octave below osc 1, 0..1.
    pub sub_level: f32,
    /// White noise, 0..1.
    pub noise_level: f32,
    pub filter: Filter,
    pub amp_env: Adsr,
    pub filter_env: Adsr,
    pub lfo: Lfo,
    pub voice: Voice,
    /// Output level, linear 0..=2.
    pub output: f32,
    /// Name of the preset the parameters came from (display only).
    pub preset: String,
}

impl Default for SynthParams {
    /// "Init": two slightly detuned oscillators through a key-tracked low-pass.
    fn default() -> Self {
        SynthParams {
            osc: [
                Osc { wave: Waveform::Saw, shape: 0.0, octave: 0, semitones: 0, fine_cents: -5.0, level: 0.6 },
                Osc { wave: Waveform::Square, shape: 0.0, octave: 0, semitones: 0, fine_cents: 5.0, level: 0.5 },
            ],
            cross_fm: 0.0,
            sub_level: 0.0,
            noise_level: 0.0,
            filter: Filter { mode: FilterMode::LowPass, cutoff_hz: 2093.0, resonance: 0.1, key_track: 1.0, env_amount: 0.0 },
            amp_env: Adsr { attack: 0.005, decay: 0.2, sustain: 0.6, release: 0.25 },
            filter_env: Adsr { attack: 0.005, decay: 0.3, sustain: 0.0, release: 0.3 },
            lfo: Lfo { wave: Waveform::Sine, rate_hz: 5.0, to_pitch_cents: 0.0, to_cutoff_oct: 0.0, to_amp: 0.0 },
            voice: Voice { polyphony: 16, glide: 0.0, unison: 1, unison_detune_cents: 12.0 },
            output: 1.0,
            preset: "Init".into(),
        }
    }
}

fn check(field: &str, v: f32, lo: f32, hi: f32) -> Result<(), String> {
    if v.is_finite() && (lo..=hi).contains(&v) { Ok(()) } else { Err(format!("{field} = {v} outside {lo}..={hi}")) }
}

impl SynthParams {
    pub fn validate(&self) -> Result<(), String> {
        for (i, o) in self.osc.iter().enumerate() {
            check(&format!("osc{}.shape", i + 1), o.shape, 0.0, 1.0)?;
            check(&format!("osc{}.octave", i + 1), o.octave as f32, -3.0, 3.0)?;
            check(&format!("osc{}.semitones", i + 1), o.semitones as f32, -12.0, 12.0)?;
            check(&format!("osc{}.fine_cents", i + 1), o.fine_cents, -100.0, 100.0)?;
            check(&format!("osc{}.level", i + 1), o.level, 0.0, 1.0)?;
        }
        check("cross_fm", self.cross_fm, 0.0, 1.0)?;
        check("sub_level", self.sub_level, 0.0, 1.0)?;
        check("noise_level", self.noise_level, 0.0, 1.0)?;
        let f = &self.filter;
        check("filter.cutoff_hz", f.cutoff_hz, 20.0, 20_000.0)?;
        check("filter.resonance", f.resonance, 0.0, 1.0)?;
        check("filter.key_track", f.key_track, 0.0, 1.0)?;
        check("filter.env_amount", f.env_amount, -1.0, 1.0)?;
        for (name, e) in [("amp_env", &self.amp_env), ("filter_env", &self.filter_env)] {
            check(&format!("{name}.attack"), e.attack, 0.001, 10.0)?;
            check(&format!("{name}.decay"), e.decay, 0.001, 10.0)?;
            check(&format!("{name}.sustain"), e.sustain, 0.0, 1.0)?;
            check(&format!("{name}.release"), e.release, 0.001, 20.0)?;
        }
        let l = &self.lfo;
        check("lfo.rate_hz", l.rate_hz, 0.01, 20.0)?;
        check("lfo.to_pitch_cents", l.to_pitch_cents, 0.0, 1200.0)?;
        check("lfo.to_cutoff_oct", l.to_cutoff_oct, 0.0, 6.0)?;
        check("lfo.to_amp", l.to_amp, 0.0, 1.0)?;
        let v = &self.voice;
        check("voice.polyphony", v.polyphony as f32, 1.0, 16.0)?;
        check("voice.glide", v.glide, 0.0, 2.0)?;
        check("voice.unison", v.unison as f32, 1.0, 4.0)?;
        check("voice.unison_detune_cents", v.unison_detune_cents, 0.0, 50.0)?;
        check("output", self.output, 0.0, 2.0)?;
        if self.preset.chars().count() > 64 {
            return Err("preset name longer than 64 characters".into());
        }
        Ok(())
    }
}

/// Factory presets (original sound design and names).
pub fn factory_presets() -> Vec<SynthParams> {
    let init = SynthParams::default();
    let mut v = vec![init.clone()];

    let mut p = init.clone();
    p.preset = "Felt Keys".into();
    p.osc[0] = Osc { wave: Waveform::Triangle, shape: 0.3, octave: 0, semitones: 0, fine_cents: 0.0, level: 0.7 };
    p.osc[1] = Osc { wave: Waveform::Sine, shape: 0.0, octave: 1, semitones: 0, fine_cents: 3.0, level: 0.25 };
    p.filter = Filter { mode: FilterMode::LowPass, cutoff_hz: 1500.0, resonance: 0.05, key_track: 0.8, env_amount: 0.3 };
    p.amp_env = Adsr { attack: 0.003, decay: 1.2, sustain: 0.25, release: 0.4 };
    p.filter_env = Adsr { attack: 0.001, decay: 0.6, sustain: 0.0, release: 0.4 };
    v.push(p);

    let mut p = init.clone();
    p.preset = "Glass Pluck".into();
    p.osc[0] = Osc { wave: Waveform::Square, shape: 0.6, octave: 0, semitones: 0, fine_cents: 0.0, level: 0.6 };
    p.osc[1] = Osc { wave: Waveform::Saw, shape: 0.0, octave: 1, semitones: 7, fine_cents: 0.0, level: 0.3 };
    p.filter = Filter { mode: FilterMode::LowPass, cutoff_hz: 600.0, resonance: 0.35, key_track: 1.0, env_amount: 0.7 };
    p.amp_env = Adsr { attack: 0.001, decay: 0.35, sustain: 0.0, release: 0.3 };
    p.filter_env = Adsr { attack: 0.001, decay: 0.18, sustain: 0.0, release: 0.2 };
    v.push(p);

    let mut p = init.clone();
    p.preset = "Round Bass".into();
    p.osc[0] = Osc { wave: Waveform::Saw, shape: 0.0, octave: -1, semitones: 0, fine_cents: 0.0, level: 0.6 };
    p.osc[1] = Osc { wave: Waveform::Square, shape: 0.2, octave: -1, semitones: 0, fine_cents: 7.0, level: 0.4 };
    p.sub_level = 0.6;
    p.filter = Filter { mode: FilterMode::LowPass, cutoff_hz: 400.0, resonance: 0.2, key_track: 0.5, env_amount: 0.4 };
    p.amp_env = Adsr { attack: 0.002, decay: 0.3, sustain: 0.8, release: 0.12 };
    p.voice = Voice { polyphony: 1, glide: 0.04, unison: 1, unison_detune_cents: 0.0 };
    v.push(p);

    let mut p = init.clone();
    p.preset = "Slow Tide Pad".into();
    p.osc[0] = Osc { wave: Waveform::Saw, shape: 0.2, octave: 0, semitones: 0, fine_cents: -8.0, level: 0.5 };
    p.osc[1] = Osc { wave: Waveform::Saw, shape: 0.2, octave: 0, semitones: 0, fine_cents: 8.0, level: 0.5 };
    p.noise_level = 0.04;
    p.filter = Filter { mode: FilterMode::LowPass, cutoff_hz: 900.0, resonance: 0.15, key_track: 0.4, env_amount: 0.25 };
    p.amp_env = Adsr { attack: 1.2, decay: 1.5, sustain: 0.8, release: 2.5 };
    p.filter_env = Adsr { attack: 2.0, decay: 2.0, sustain: 0.5, release: 2.5 };
    p.lfo = Lfo { wave: Waveform::Triangle, rate_hz: 0.25, to_pitch_cents: 0.0, to_cutoff_oct: 0.8, to_amp: 0.0 };
    p.voice = Voice { polyphony: 16, glide: 0.0, unison: 2, unison_detune_cents: 14.0 };
    v.push(p);

    let mut p = init;
    p.preset = "Neon Lead".into();
    p.osc[0] = Osc { wave: Waveform::Square, shape: 0.35, octave: 0, semitones: 0, fine_cents: 0.0, level: 0.6 };
    p.osc[1] = Osc { wave: Waveform::Saw, shape: 0.0, octave: 0, semitones: 0, fine_cents: 9.0, level: 0.45 };
    p.filter = Filter { mode: FilterMode::LowPass, cutoff_hz: 3000.0, resonance: 0.3, key_track: 0.6, env_amount: 0.2 };
    p.amp_env = Adsr { attack: 0.01, decay: 0.1, sustain: 0.85, release: 0.15 };
    p.lfo = Lfo { wave: Waveform::Sine, rate_hz: 5.5, to_pitch_cents: 12.0, to_cutoff_oct: 0.0, to_amp: 0.0 };
    p.voice = Voice { polyphony: 1, glide: 0.06, unison: 2, unison_detune_cents: 10.0 };
    v.push(p);
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_presets_are_valid_and_unique() {
        let ps = factory_presets();
        assert!(ps.iter().all(|p| p.validate().is_ok()), "{:?}", ps.iter().map(|p| p.validate()).collect::<Vec<_>>());
        let mut names: Vec<_> = ps.iter().map(|p| p.preset.as_str()).collect();
        names.dedup();
        assert_eq!(names.len(), ps.len());
        assert_eq!(ps[0], SynthParams::default());
    }

    #[test]
    fn validation_catches_out_of_range() {
        let mut p = SynthParams::default();
        p.filter.cutoff_hz = 5.0;
        assert!(p.validate().unwrap_err().contains("cutoff"));
        let mut p = SynthParams::default();
        p.voice.polyphony = 0;
        assert!(p.validate().is_err());
        let mut p = SynthParams::default();
        p.osc[1].level = f32::NAN;
        assert!(p.validate().is_err());
    }

    #[test]
    fn serde_round_trip() {
        let p = factory_presets()[3].clone();
        let back: SynthParams = serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        assert_eq!(p, back);
    }
}
