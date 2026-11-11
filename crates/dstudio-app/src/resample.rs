//! Sample-rate conversion for exports: band-limited polyphase resampling (Kaiser-windowed sinc)
//! between two rates whose ratio reduces to small whole numbers, such as 48 kHz and 44.1 kHz
//! (160 : 147) and their multiples.

use std::borrow::Cow;

/// Half the kernel length, in input samples.
const HALF_TAPS: usize = 64;
/// Kaiser window shape: about 85 dB of stopband attenuation.
const BETA: f64 = 8.6;
/// Kernel cutoff as a fraction of the lower Nyquist frequency: the passband stays flat to about
/// 19.5 kHz at 44.1 kHz, and everything above the lower Nyquist frequency is stopped.
const ROLLOFF: f64 = 0.93;
/// The largest number of kernel phases (the reduced output side of the ratio) accepted.
const MAX_PHASES: usize = 2048;

/// A converter from one rate to another, its kernel computed once.
pub struct Resampler {
    /// Output samples per `down` input samples (the ratio in lowest terms).
    up: usize,
    down: usize,
    /// Kernel taps per phase (2 × HALF_TAPS), and the phases one after another.
    taps: usize,
    table: Vec<f32>,
}

fn gcd(a: u64, b: u64) -> u64 {
    if b == 0 { a } else { gcd(b, a % b) }
}

/// Zeroth-order modified Bessel function of the first kind (series; converges fast for the
/// window's arguments).
fn bessel_i0(x: f64) -> f64 {
    let (mut sum, mut term, q) = (1.0, 1.0, x * x / 4.0);
    for k in 1..64 {
        term *= q / (k * k) as f64;
        sum += term;
        if term < sum * 1e-17 {
            break;
        }
    }
    sum
}

impl Resampler {
    /// A converter from `from` Hz to `to` Hz, or None when the ratio doesn't reduce to at most
    /// MAX_PHASES output steps (rates that share no useful common factor).
    pub fn new(from: u32, to: u32) -> Option<Resampler> {
        if from == 0 || to == 0 {
            return None;
        }
        let g = gcd(from as u64, to as u64);
        let (up, down) = ((to as u64 / g) as usize, (from as u64 / g) as usize);
        if up > MAX_PHASES {
            return None;
        }
        // Cutoff in cycles per input sample (1.0 = the input Nyquist frequency).
        let fc = ROLLOFF * (up as f64 / down as f64).min(1.0);
        let taps = 2 * HALF_TAPS;
        let norm = bessel_i0(BETA);
        let mut table = Vec::with_capacity(up * taps);
        for phase in 0..up {
            let frac = phase as f64 / up as f64;
            let row: Vec<f64> = (0..taps)
                .map(|j| {
                    // Distance from the output instant to input sample n + j - (HALF_TAPS - 1).
                    let x = j as f64 - (HALF_TAPS as f64 - 1.0) - frac;
                    let u = x / (HALF_TAPS as f64);
                    if u.abs() >= 1.0 {
                        return 0.0;
                    }
                    let a = std::f64::consts::PI * fc * x;
                    let sinc = if a.abs() < 1e-12 { 1.0 } else { a.sin() / a };
                    fc * sinc * bessel_i0(BETA * (1.0 - u * u).sqrt()) / norm
                })
                .collect();
            // Unity gain at DC in every phase.
            let sum: f64 = row.iter().sum();
            table.extend(row.iter().map(|h| (h / sum) as f32));
        }
        Some(Resampler { up, down, taps, table })
    }

    /// Frames out for `frames` in: every output instant inside the input's span.
    pub fn out_frames(&self, frames: usize) -> usize {
        (frames * self.up).div_ceil(self.down)
    }

    /// Converts interleaved `input` with `channels` channels. Samples outside the input count
    /// as silence. When the rates match, the input is handed back as it is, without a copy (a long
    /// export holds one full buffer, not two).
    pub fn process<'a>(&self, input: &'a [f32], channels: usize) -> Cow<'a, [f32]> {
        let channels = channels.max(1);
        let frames = input.len() / channels;
        if self.up == self.down {
            return Cow::Borrowed(&input[..frames * channels]);
        }
        let out_frames = self.out_frames(frames);
        let mut out = vec![0f32; out_frames * channels];
        let first = HALF_TAPS as i64 - 1;
        for k in 0..out_frames {
            let pos = k * self.down;
            let (n, phase) = ((pos / self.up) as i64, pos % self.up);
            let h = &self.table[phase * self.taps..(phase + 1) * self.taps];
            let start = n - first;
            for c in 0..channels {
                let mut acc = 0f32;
                if start >= 0 && start as usize + self.taps <= frames {
                    let base = start as usize * channels + c;
                    for (j, w) in h.iter().enumerate() {
                        acc += w * input[base + j * channels];
                    }
                } else {
                    for (j, w) in h.iter().enumerate() {
                        let i = start + j as i64;
                        if i >= 0 && (i as usize) < frames {
                            acc += w * input[i as usize * channels + c];
                        }
                    }
                }
                out[k * channels + c] = acc;
            }
        }
        Cow::Owned(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::TAU;

    fn sine(rate: u32, hz: f32, frames: usize) -> Vec<f32> {
        (0..frames).map(|i| 0.5 * (TAU * hz * i as f32 / rate as f32).sin()).collect()
    }

    /// Largest difference from the ideal sine over the middle of the output (the ends see the
    /// silence around the input).
    fn worst_error(out: &[f32], rate: u32, hz: f32) -> f32 {
        let n = out.len();
        (n / 4..3 * n / 4).map(|i| (out[i] - 0.5 * (TAU * hz * i as f32 / rate as f32).sin()).abs()).fold(0.0, f32::max)
    }

    fn rms_middle(out: &[f32]) -> f32 {
        let n = out.len();
        let mid = &out[n / 4..3 * n / 4];
        (mid.iter().map(|s| s * s).sum::<f32>() / mid.len() as f32).sqrt()
    }

    #[test]
    fn the_same_rate_copies_the_input() {
        let r = Resampler::new(48_000, 48_000).unwrap();
        let input = sine(48_000, 1000.0, 1000);
        let out = r.process(&input, 1);
        assert!(matches!(out, Cow::Borrowed(_)), "the same rate made a copy");
        assert_eq!(out, input);
        assert_eq!(r.out_frames(1000), 1000);
    }

    #[test]
    fn a_tone_keeps_its_pitch_and_level_both_ways() {
        for (from, to) in [(48_000, 44_100), (44_100, 48_000), (96_000, 44_100)] {
            let r = Resampler::new(from, to).unwrap();
            let input = sine(from, 1000.0, from as usize / 2);
            let out = r.process(&input, 1);
            assert_eq!(out.len(), (input.len() * to as usize).div_ceil(from as usize), "{from} -> {to}");
            let err = worst_error(&out, to, 1000.0);
            assert!(err < 1e-3, "{from} -> {to}: error {err}");
        }
    }

    #[test]
    fn high_tones_pass_and_tones_above_the_new_nyquist_are_stopped() {
        let r = Resampler::new(48_000, 44_100).unwrap();
        let pass = rms_middle(&r.process(&sine(48_000, 18_000.0, 24_000), 1));
        assert!((pass / (0.5 / 2f32.sqrt()) - 1.0).abs() < 0.01, "18 kHz rms {pass}");
        // 23 kHz has no place at 44.1 kHz: it would fold back to 21.1 kHz.
        let stop = rms_middle(&r.process(&sine(48_000, 23_000.0, 24_000), 1));
        assert!(stop < 0.5e-3, "23 kHz leaks at rms {stop}");
    }

    #[test]
    fn channels_stay_apart() {
        let r = Resampler::new(48_000, 44_100).unwrap();
        let left = sine(48_000, 440.0, 4800);
        let stereo: Vec<f32> = left.iter().flat_map(|&s| [s, 0.0]).collect();
        let out = r.process(&stereo, 2);
        assert_eq!(out.len(), 2 * r.out_frames(4800));
        assert!(out.iter().skip(1).step_by(2).all(|s| *s == 0.0), "the silent channel picked up sound");
        assert!(out.iter().step_by(2).any(|s| s.abs() > 0.4));
    }

    /// Throughput for the PR's benchmark table: `cargo test --release -p dstudio-app
    /// resample::tests::bench -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn bench_three_minutes_of_stereo() {
        for (from, to) in [(48_000, 44_100), (44_100, 48_000)] {
            let frames = from as usize * 180;
            let input: Vec<f32> = (0..frames * 2).map(|i| ((i as f32) * 0.001).sin() * 0.5).collect();
            let r = Resampler::new(from, to).unwrap();
            let t = std::time::Instant::now();
            let out = r.process(&input, 2);
            let s = t.elapsed().as_secs_f64();
            println!("{from} -> {to}: 3 min stereo in {:.0} ms ({:.0}x real time), {} frames out", s * 1000.0, 180.0 / s, out.len() / 2);
        }
    }

    #[test]
    fn rates_without_a_small_ratio_are_refused() {
        assert!(Resampler::new(44_100, 47_999).is_none());
        assert!(Resampler::new(0, 48_000).is_none());
    }
}
