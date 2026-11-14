//! Clip menu › Send to Sampler: an audio clip's sound on a new instrument track, played by
//! a Sampler that holds the clip's file, its start and end set to the part the clip plays.
//! The file isn't copied or rendered; one undo step.

use crate::controller::{BottomTab, Controller, next_track_color};
use dstudio_core::builtin::sampler::SAMPLER;
use dstudio_core::{ClipId, Command, DeviceSample, Track, TrackId, TrackKind};

/// Sampler start and end (fractions of the file) for a clip playing `length` frames from
/// `offset` in a file of `frames` frames.
pub fn sample_range(offset: u64, length: u64, frames: u64) -> (f32, f32) {
    if frames == 0 {
        return (0.0, 1.0);
    }
    let f = frames as f64;
    let start = (offset as f64 / f).clamp(0.0, 1.0);
    let end = ((offset + length) as f64 / f).clamp(start, 1.0);
    (start as f32, end as f32)
}

impl Controller {
    /// Send to Sampler: a new instrument track below the clip's track, its instrument a
    /// Sampler loaded with the clip's audio. Selects the track and shows its devices.
    pub fn send_to_sampler(&mut self, id: ClipId) -> Option<TrackId> {
        let p = self.project();
        let (ti, ci) = p.find_clip(id)?;
        let clip = &p.tracks[ti].clips[ci];
        let Some(asset) = p.asset(clip.asset) else {
            self.set_status("Send to Sampler: the clip's audio is missing", 3);
            return None;
        };
        let (start, end) = sample_range(clip.source_offset, clip.source_length, asset.frames);
        let name = if clip.name.trim().is_empty() { "Sampler".to_string() } else { clip.name.clone() };
        let mut device = SAMPLER.instantiate();
        device.params.insert("start".into(), start);
        device.params.insert("end".into(), end);
        device.samples.push(DeviceSample { slot: 0, asset: asset.id, name: name.clone(), slices: Vec::new() });
        let track_id = TrackId(p.next_id);
        let track = Track::new(track_id, TrackKind::Instrument, &name, next_track_color(p));
        // Below the clip's track, inside its group if it has one.
        let index = Some((ti + 1).min(p.ordinary_track_count()));
        let output = p.tracks[ti].output.filter(|g| p.track(*g).is_some_and(|g| g.is_group()));
        let commands = vec![
            Command::AddTrack { index, track: Track { output, ..track } },
            Command::SetInstrumentPlugin { track: track_id, device: Some(device) },
        ];
        if !self.execute(Command::Batch { label: "Send to Sampler".into(), commands }) {
            self.set_status("Send to Sampler failed", 3);
            return None;
        }
        self.select_track(track_id);
        self.select_clip(None);
        self.set_bottom_tab(BottomTab::Devices);
        Some(track_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sample_range_is_the_part_the_clip_plays() {
        assert_eq!(sample_range(0, 1000, 1000), (0.0, 1.0));
        assert_eq!(sample_range(250, 500, 1000), (0.25, 0.75));
        // A clip running past the file end stops at the end.
        assert_eq!(sample_range(500, 1000, 1000), (0.5, 1.0));
        assert_eq!(sample_range(0, 10, 0), (0.0, 1.0));
    }
}
