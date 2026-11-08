//! Test-signal WAV writer for `--demo` (16-bit PCM). Generated at 44.1 kHz on
//! purpose so the demo also exercises the engine's resampler (engine runs at 48 kHz).

use std::io::Write;
use std::path::Path;

pub fn write_wav(path: &Path, rate: u32, channels: u16, samples: &[f32]) -> std::io::Result<()> {
    let data_len = (samples.len() * 2) as u32;
    let mut f = std::io::BufWriter::new(std::fs::File::create(path)?);
    f.write_all(b"RIFF")?;
    f.write_all(&(36 + data_len).to_le_bytes())?;
    f.write_all(b"WAVEfmt ")?;
    f.write_all(&16u32.to_le_bytes())?;
    f.write_all(&1u16.to_le_bytes())?; // PCM
    f.write_all(&channels.to_le_bytes())?;
    f.write_all(&rate.to_le_bytes())?;
    f.write_all(&(rate * channels as u32 * 2).to_le_bytes())?;
    f.write_all(&(channels * 2).to_le_bytes())?;
    f.write_all(&16u16.to_le_bytes())?;
    f.write_all(b"data")?;
    f.write_all(&data_len.to_le_bytes())?;
    for s in samples {
        f.write_all(&((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes())?;
    }
    f.flush()
}

/// 4 s mono: a 220 Hz sine with a beat-synchronous (120 BPM) decaying envelope.
pub fn sine_bursts(rate: u32) -> Vec<f32> {
    let n = rate as usize * 4;
    (0..n)
        .map(|i| {
            let t = i as f32 / rate as f32;
            let ph = (t * 2.0).fract(); // beats at 120 BPM
            let env = 0.15 + 0.75 * (-ph * 5.0).exp();
            env * (std::f32::consts::TAU * 220.0 * t).sin()
        })
        .collect()
}

/// 3 s stereo (interleaved): white noise with a slow tremolo, different per channel.
pub fn noise(rate: u32) -> Vec<f32> {
    let n = rate as usize * 3;
    let mut s: u32 = 0x1234_5678;
    let mut out = Vec::with_capacity(n * 2);
    for i in 0..n {
        let t = i as f32 / rate as f32;
        for ch in 0..2 {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            let r = s as f32 / u32::MAX as f32 * 2.0 - 1.0;
            let trem = 0.35 + 0.3 * (t * std::f32::consts::TAU * (0.75 + ch as f32 * 0.25)).sin();
            out.push(r * trem);
        }
    }
    out
}
