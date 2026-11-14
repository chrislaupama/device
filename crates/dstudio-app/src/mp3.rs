//! MP3 writer for Export (Mixdown and Stems): constant bitrate stereo, encoded with LAME
//! (libmp3lame, LGPL; built from source by the `mp3lame-sys` crate). The first frame is LAME's
//! "Info" tag, which records the encoder delay and padding so players and decoders give back
//! exactly the frames that were rendered (gapless).

use mp3lame_encoder::{Bitrate, Builder, FlushGap, InterleavedPcm, Mode, Quality};
use std::io::Write;

/// The bitrates the Export dialog offers, in kbps.
pub const BITRATES: [u32; 2] = [192, 320];
/// Frames handed to LAME per call (a multiple of the 1152-frame MP3 frame).
const CHUNK: usize = 1152 * 32;

fn bitrate(kbps: u32) -> Option<Bitrate> {
    Some(match kbps {
        128 => Bitrate::Kbps128,
        160 => Bitrate::Kbps160,
        192 => Bitrate::Kbps192,
        224 => Bitrate::Kbps224,
        256 => Bitrate::Kbps256,
        320 => Bitrate::Kbps320,
        _ => return None,
    })
}

fn err(what: &str, e: impl std::fmt::Debug) -> std::io::Error {
    std::io::Error::other(format!("MP3 encoder: {what} ({e:?})"))
}

/// Interleaved stereo `samples` at `rate` Hz as a CBR MP3 at `kbps`, into an opened file.
pub fn encode_mp3(file: std::fs::File, rate: u32, kbps: u32, samples: &[f32]) -> std::io::Result<()> {
    let brate = bitrate(kbps).ok_or_else(|| err("unsupported bitrate", kbps))?;
    let mut b = Builder::new().ok_or_else(|| err("can't start", ()))?;
    b.set_num_channels(2).map_err(|e| err("channels", e))?;
    b.set_sample_rate(rate).map_err(|e| err("sample rate", e))?;
    b.set_brate(brate).map_err(|e| err("bitrate", e))?;
    b.set_mode(Mode::JointStereo).map_err(|e| err("mode", e))?;
    // LAME's "-q 2": its recommended high-quality setting (0 is much slower for no audible gain).
    b.set_quality(Quality::NearBest).map_err(|e| err("quality", e))?;
    b.set_to_write_vbr_tag(true).map_err(|e| err("info tag", e))?;
    let mut enc = b.build().map_err(|e| err("settings", e))?;
    let frames = samples.len() / 2;
    let mut out = Vec::with_capacity(mp3lame_encoder::max_required_buffer_size(frames) + 7200);
    // LAME wants -1..1; louder samples would wrap, so clip them like the WAV writer does.
    let mut buf = Vec::with_capacity(CHUNK * 2);
    for chunk in samples[..frames * 2].chunks(CHUNK * 2) {
        buf.clear();
        buf.extend(chunk.iter().map(|s| s.clamp(-1.0, 1.0)));
        out.reserve(mp3lame_encoder::max_required_buffer_size(chunk.len() / 2));
        enc.encode_to_vec(InterleavedPcm(buf.as_slice()), &mut out).map_err(|e| err("encode", e))?;
    }
    out.reserve(7200);
    enc.flush_to_vec::<FlushGap>(&mut out).map_err(|e| err("flush", e))?;
    // The Info tag goes in the (empty) first frame LAME left for it.
    let mut tag = Vec::with_capacity(enc.lame_tag_size());
    if enc.lame_tag_encode_to_vec(&mut tag).is_some() && tag.len() <= out.len() {
        out[..tag.len()].copy_from_slice(&tag);
    }
    let mut f = std::io::BufWriter::new(file);
    f.write_all(&out)?;
    f.flush()
}

/// Bytes an MP3 of `frames` at `rate` Hz and `kbps` takes (the audio; the tag frame is small).
pub fn estimate_bytes(frames: u64, rate: f64, kbps: u32) -> f64 {
    frames as f64 / rate * kbps as f64 * 1000.0 / 8.0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One second of a 440 Hz sine at -6 dB, encoded and read back through the engine's decoder
    /// (the one used for importing MP3s): same length, same level.
    #[test]
    fn an_mp3_reads_back_with_its_length_and_level() {
        let engine = dstudio_engine::Engine::new(dstudio_engine::Config::default()).unwrap();
        let rate = engine.sample_rate().round() as u32;
        let frames = rate as usize;
        let samples: Vec<f32> = (0..frames)
            .flat_map(|i| {
                let s = 0.5 * (std::f32::consts::TAU * 440.0 * i as f32 / rate as f32).sin();
                [s, s]
            })
            .collect();
        for kbps in BITRATES {
            let path = std::env::temp_dir().join(format!("dstudio-mp3-{}-{kbps}.mp3", std::process::id()));
            encode_mp3(std::fs::File::create(&path).unwrap(), rate, kbps, &samples).unwrap();
            let size = std::fs::metadata(&path).unwrap().len() as f64;
            let info = engine.load_file(&path);
            let _ = std::fs::remove_file(&path);
            let info = info.unwrap();
            assert_eq!((info.channels, info.source_sample_rate.round() as u32), (2, rate));
            assert_eq!(info.frames, frames as u64, "{kbps} kbps: the Info tag trims the encoder delay and padding");
            let expect = estimate_bytes(frames as u64, rate as f64, kbps);
            // Off by the Info tag frame and the delay and padding frames at most.
            let frame_bytes = 144.0 * kbps as f64 * 1000.0 / rate as f64;
            assert!((size - expect).abs() < 4.0 * frame_bytes, "{kbps} kbps: {size} bytes, estimate {expect}");
            // Peak of the middle of the file (away from the fade-in LAME's filters may add).
            let peaks = engine.peaks(info.asset_id, frames as u64 / 4, (frames / 2) as u32, 1).unwrap();
            let peak = peaks[0][1].max(-peaks[0][0]);
            assert!((peak - 0.5).abs() < 0.05, "{kbps} kbps: peak {peak}");
            let _ = engine.release_asset(info.asset_id);
        }
    }

    #[test]
    fn only_standard_bitrates_are_accepted() {
        let path = std::env::temp_dir().join(format!("dstudio-mp3-bad-{}.mp3", std::process::id()));
        let r = encode_mp3(std::fs::File::create(&path).unwrap(), 48_000, 100, &[0.0; 8]);
        let _ = std::fs::remove_file(&path);
        assert!(r.is_err());
    }
}
