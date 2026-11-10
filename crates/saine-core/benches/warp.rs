//! Snapshot cost of warped clips: `snapshot::build_with_transients` for a
//! project of N audio tracks, each with one 8-bar clip, unwarped vs warped
//! (Auto, 8 warp markers, 400 detected transients), and the cost of one
//! `SetClipWarp` edit with its undo. Prints measurements only.
//!
//! usage: cargo bench -p saine-core --bench warp

use saine_core::snapshot;
use saine_core::warp::{ClipWarp, Transient, WarpMode};
use saine_core::*;
use std::collections::HashMap;
use std::hint::black_box;
use std::time::Instant;

fn project(tracks: usize, warped: bool) -> (Document, HashMap<AssetId, u64>, HashMap<AssetId, Vec<Transient>>) {
    let mut d = Document::new(Project::default());
    d.execute(Command::AddAsset { asset: Asset { id: AssetId(0), relative_path: None, absolute_path: "/x.wav".into(), sample_rate: 44_100.0, channels: 2, frames: 44_100 * 60 } }, Origin::User).unwrap();
    let a = d.project().assets[0].id;
    let detected: Vec<Transient> = (0..400).map(|i| Transient { source: i * 6_615, strength: 1.0, percussive: i % 3 != 0 }).collect();
    let mut warp = ClipWarp { enabled: warped, mode: WarpMode::Auto, original_bpm: 96.0, ..ClipWarp::default() };
    for i in 0..8u64 {
        warp.set_marker(i * 44_100 * 2, i as f64 * 3.3);
    }
    for t in 0..tracks {
        let track = if t < 1 {
            d.project().tracks[0].id
        } else {
            d.execute(Command::AddTrack { index: None, track: Track::new(TrackId(0), TrackKind::Audio, "A", ColorIndex(1)) }, Origin::User).unwrap();
            d.project().tracks.last().unwrap().id
        };
        let clip = Clip { id: ClipId(0), asset: a, name: "c".into(), start: 0, source_offset: 0, source_length: 44_100 * 16, gain: 1.0, warp: if warped { warp.clone() } else { ClipWarp::default() } };
        d.execute(Command::AddClip { track, clip }, Origin::User).unwrap();
    }
    (d, HashMap::from([(a, 1)]), HashMap::from([(a, detected)]))
}

fn time<F: FnMut()>(iters: u32, mut f: F) -> f64 {
    f();
    let t0 = Instant::now();
    for _ in 0..iters {
        f();
    }
    t0.elapsed().as_secs_f64() * 1e6 / iters as f64
}

fn main() {
    println!("| tracks | unwarped build µs | warped build µs | per warped clip µs | SetClipWarp + undo µs |");
    println!("|---:|---:|---:|---:|---:|");
    for tracks in [1usize, 16, 64] {
        let (plain, map, tr) = project(tracks, false);
        let (mut warped, _, _) = project(tracks, true);
        let a = time(200, || {
            black_box(snapshot::build_with_transients(plain.project(), 48_000.0, &map, &tr));
        });
        let b = time(200, || {
            black_box(snapshot::build_with_transients(warped.project(), 48_000.0, &map, &tr));
        });
        let clip = warped.project().tracks[0].clips[0].id;
        let mut w = warped.project().clip(clip).unwrap().warp.clone();
        w.speed = 2.0;
        let e = time(200, || {
            warped.execute(Command::SetClipWarp { clip, warp: w.clone(), label: "Set clip speed".into() }, Origin::User).unwrap();
            warped.undo().unwrap();
        });
        println!("| {tracks} | {a:.1} | {b:.1} | {:.2} | {e:.1} |", (b - a) / tracks as f64);
    }
}
