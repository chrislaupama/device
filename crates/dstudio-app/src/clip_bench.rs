//! `--bench-clip-render idle|bounce`: measures what a clip render does to the interface. A 4-bar
//! instrument clip (64 notes) on a track with the built-in synth, reverb and delay is added; after
//! a warm-up the run either does nothing (`idle`) or starts Bounce on that clip (`bounce`), and the
//! frame statistics of `--frame-stats` are split into before / during / after (main.rs, `Frame`).
//! Use it with `--demo --exit-after-frames 400` in a visible window. With `DSTUDIO_BENCH_PLUGINS=au`
//! (macOS) the track's reverb and delay are Apple's AUMatrixReverb and AUDelay instead, so the
//! bounce goes through plugin instances prepared on the UI thread (CHR-38).

use crate::controller::Controller;
use dstudio_core::{ClipId, ColorIndex, Command, Device, MidiClip, Note, PluginFormat, PluginKind, PluginRef, PPQ, Track, TrackId, TrackKind};

/// Frames before the run starts the render (idle: the window the render would take).
pub const WARMUP_FRAMES: u64 = 60;
/// Frames of the `during` phase of an idle run.
pub const IDLE_WINDOW: u64 = 40;

/// Adds the bench track and returns its clip.
pub fn setup(c: &mut Controller) -> ClipId {
    let id = c.project().next_id;
    let mut clip = MidiClip::empty(0, 16 * PPQ);
    clip.id = ClipId(id + 1);
    clip.name = "bench".into();
    clip.notes = (0..64).map(|i| Note { id: i as u32 + 1, pitch: 48 + ((i * 5) % 24) as u8, start: i as i64 * PPQ / 4, length: PPQ / 2, velocity: 100 }).collect();
    let track = Track::new(TrackId(id), TrackKind::Instrument, "Bench synth", ColorIndex::FIRST);
    c.execute(Command::Batch { label: "clip render bench".into(), commands: vec![Command::AddTrack { index: None, track }, Command::AddMidiClip { track: TrackId(id), clip }] });
    if std::env::var("DSTUDIO_BENCH_PLUGINS").as_deref() == Ok("au") {
        for (code, name) in [("aufx mrev appl", "AUMatrixReverb"), ("aufx dely appl", "AUDelay")] {
            let plugin = PluginRef { format: PluginFormat::AudioUnit, id: code.into(), path: None, name: name.into(), vendor: "Apple".into(), version: String::new(), kind: PluginKind::Effect };
            c.execute(Command::AddDevice { track: TrackId(id), index: None, device: Device::new(plugin) });
        }
        return ClipId(id + 1);
    }
    for name in ["reverb", "delay"] {
        if let Some(def) = dstudio_core::builtin::find(name) {
            c.execute(Command::AddDevice { track: TrackId(id), index: None, device: def.instantiate() });
        }
    }
    ClipId(id + 1)
}
