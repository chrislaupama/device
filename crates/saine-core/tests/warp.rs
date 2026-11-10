//! Audio clip warping in the model: tempo following, speed, original BPM,
//! warp markers, transient edits, the command with undo, persistence, and
//! the engine snapshot (map points and slices).

use saine_core::snapshot::{self, WarpSnapshot};
use saine_core::warp::{ClipWarp, Transient, WarpMode};
use saine_core::*;
use std::collections::HashMap;
use std::time::{Duration, Instant};

const RATE: f64 = 48_000.0;

fn doc() -> (Document, ClipId, AssetId) {
    let mut d = Document::new(Project::default());
    let track = d.project().tracks[0].id;
    let asset = Asset { id: AssetId(0), relative_path: None, absolute_path: "/nonexistent/loop.wav".into(), sample_rate: 44_100.0, channels: 2, frames: 44_100 * 8 };
    d.execute(Command::AddAsset { asset }, Origin::User).unwrap();
    let a = d.project().assets[0].id;
    let clip = Clip { id: ClipId(0), asset: a, name: "loop".into(), start: PPQ * 4, source_offset: 0, source_length: 44_100 * 4, gain: 1.0, warp: ClipWarp::default() };
    d.execute(Command::AddClip { track, clip }, Origin::User).unwrap();
    let id = d.project().tracks[0].clips[0].id;
    (d, id, a)
}

fn warp(d: &Document, c: ClipId) -> ClipWarp {
    d.project().clip(c).unwrap().warp.clone()
}

fn set(d: &mut Document, c: ClipId, w: ClipWarp, label: &str) {
    d.execute(Command::SetClipWarp { clip: c, warp: w, label: label.into() }, Origin::User).unwrap();
}

fn snap_warp(d: &Document, c: ClipId, transients: &HashMap<AssetId, Vec<Transient>>) -> (i64, i64, Option<WarpSnapshot>) {
    let a = d.project().clip(c).unwrap().asset;
    let (s, skipped) = snapshot::build_with_transients(d.project(), RATE, &HashMap::from([(a, 7)]), transients);
    assert!(skipped.is_empty());
    let cs = s.tracks[0].clips.iter().find(|x| x.clip_id == c.0).unwrap();
    (cs.timeline_start, cs.length, cs.warp.clone())
}

fn slope(points: &[(i64, f64)]) -> f64 {
    let (a, b) = (points[0], points[points.len() - 1]);
    (b.1 - a.1) / (b.0 - a.0) as f64
}

#[test]
fn warp_off_plays_one_to_one() {
    let (d, c, _) = doc();
    let (start, len, w) = snap_warp(&d, c, &HashMap::new());
    assert_eq!((start, len, w), (96_000, 192_000, None));
    assert_eq!(d.project().clip_length_ticks(d.project().clip(c).unwrap()), 8 * PPQ);
}

#[test]
fn warp_on_at_the_project_tempo_keeps_the_clip_and_follows_tempo_changes() {
    let (mut d, c, _) = doc();
    set(&mut d, c, ClipWarp { enabled: true, original_bpm: 120.0, ..ClipWarp::default() }, "Set clip warp mode");
    let (start, len, w) = snap_warp(&d, c, &HashMap::new());
    let w = w.unwrap();
    assert_eq!((start, len), (96_000, 192_000));
    assert_eq!(w.mode, 2, "Auto without transients stretches");
    assert!((slope(&w.points) - 1.0).abs() < 1e-9);
    assert_eq!(w.points[0].0, 96_000);
    assert!(w.points[0].1.abs() < 1e-9);
    // 120 -> 90 BPM: the 8 beats now take 5.33 s and the source plays slower.
    d.execute(Command::SetTempo { bpm: 90.0 }, Origin::User).unwrap();
    let (start, len, w) = snap_warp(&d, c, &HashMap::new());
    assert_eq!(d.project().clip_length_ticks(d.project().clip(c).unwrap()), 8 * PPQ);
    assert_eq!((start, len), (128_000, 256_000));
    assert!((slope(&w.unwrap().points) - 0.75).abs() < 1e-6);
}

#[test]
fn speed_and_original_bpm_scale_the_footprint() {
    let (mut d, c, _) = doc();
    let on = ClipWarp { enabled: true, original_bpm: 120.0, mode: WarpMode::Melody, ..ClipWarp::default() };
    for (speed, beats) in [(2.0, 4), (0.5, 16), (1.5, 5)] {
        set(&mut d, c, ClipWarp { speed, ..on.clone() }, "Set clip speed");
        let clip = d.project().clip(c).unwrap();
        assert_eq!(d.project().clip_length_ticks(clip), (8.0 / speed * PPQ as f64).round() as i64, "speed {speed} ~ {beats} beats");
        let (_, _, w) = snap_warp(&d, c, &HashMap::new());
        assert!((slope(&w.unwrap().points) - speed).abs() < 1e-3);
    }
    // A 60 BPM loop in a 120 BPM project plays twice as fast: 4 beats.
    set(&mut d, c, ClipWarp { original_bpm: 60.0, ..on.clone() }, "Set original BPM");
    assert_eq!(d.project().clip_length_ticks(d.project().clip(c).unwrap()), 4 * PPQ);
    assert_eq!(snap_warp(&d, c, &HashMap::new()).1, 96_000);
}

#[test]
fn source_and_ticks_convert_both_ways_for_trims() {
    let (mut d, c, _) = doc();
    for w in [ClipWarp::default(), ClipWarp { enabled: true, original_bpm: 90.0, speed: 1.5, ..ClipWarp::default() }] {
        set(&mut d, c, w, "Set clip speed");
        let p = d.project();
        let clip = p.clip(c).unwrap();
        assert!(p.clip_source_at(clip, 0.0).abs() < 1e-6);
        let end = p.clip_length_ticks(clip) as f64;
        assert!((p.clip_source_at(clip, end) - clip.source_length as f64).abs() < 50.0, "the clip end plays the last source frame");
        for t in [-960.0, 100.0, 5000.0] {
            assert!((p.clip_ticks_at(clip, p.clip_source_at(clip, t)) - t).abs() < 1e-6);
        }
    }
}

#[test]
fn markers_pin_source_frames_to_beats() {
    let mut w = ClipWarp { enabled: true, original_bpm: 120.0, ..ClipWarp::default() };
    // Source second 1 (beat 2) pinned to beat 3: the first second stretches.
    assert!(w.set_marker(0, 0.0));
    assert!(w.set_marker(44_100, 3.0));
    assert!(!w.set_marker(22_050, 3.5), "crossing a neighbour is refused");
    assert!((w.beat_at(22_050.0, 44_100.0) - 1.5).abs() < 1e-9);
    assert!((w.source_at(1.5, 44_100.0) - 22_050.0).abs() < 1e-6);
    // Past the last marker the last segment continues.
    assert!((w.beat_at(88_200.0, 44_100.0) - 6.0).abs() < 1e-9);
    let (mut d, c, _) = doc();
    set(&mut d, c, w.clone(), "Move warp marker");
    let (start, _, s) = snap_warp(&d, c, &HashMap::new());
    let pts = s.unwrap().points;
    assert_eq!(pts.len(), 3, "clip start, the marker at 1 s, clip end: {pts:?}");
    assert_eq!(pts[1].0 - start, 72_000, "beat 3 at 120 BPM");
    // The last segment (3 beats per source second) continues: 4 s = 12 beats -> 6 s at 120 BPM.
    assert_eq!(d.project().clip_length_ticks(d.project().clip(c).unwrap()), 12 * PPQ);
    assert_eq!(pts[2].0 - start, 288_000);
    // A marker inside the clip appears in the map.
    let mut w2 = w.clone();
    w2.markers.clear();
    assert!(w2.set_marker(44_100, 2.0) && w2.set_marker(88_200, 5.0));
    set(&mut d, c, w2, "Move warp marker");
    let pts = snap_warp(&d, c, &HashMap::new()).2.unwrap().points;
    assert_eq!(pts.len(), 4, "{pts:?}");
    assert!((pts[1].1 - 48_000.0).abs() < 1e-6 && (pts[2].1 - 96_000.0).abs() < 1e-6, "{pts:?}");
    // The first segment (3 beats/s) extends back: the clip start is beat -1.
    assert_eq!((pts[1].0 - start, pts[2].0 - start), (72_000, 144_000));
    assert!(w.remove_marker(1) && !w.remove_marker(5));
}

#[test]
fn transient_edits_add_remove_and_move_with_a_1ms_tolerance() {
    let det = [Transient { source: 1000, strength: 1.0, percussive: true }, Transient { source: 30_000, strength: 0.5, percussive: false }];
    let rate = 44_100.0;
    let mut w = ClipWarp::default();
    assert!(!w.add_transient(&det, 1030, rate), "within 1 ms of a detected one");
    assert!(w.add_transient(&det, 40_000, rate));
    assert_eq!(w.transients(&det, rate).iter().map(|t| (t.source, t.percussive)).collect::<Vec<_>>(), [(1000, true), (30_000, false), (40_000, false)]);
    assert!(w.remove_transient(&det, 1010, rate));
    assert_eq!((w.transients_added.clone(), w.transients_removed.clone()), (vec![40_000], vec![1000]));
    assert!(w.add_transient(&det, 1000, rate), "re-adding undoes the removal");
    assert!(w.transients_removed.is_empty());
    assert!(w.move_transient(&det, 40_000, 50_000, rate));
    assert_eq!(w.transients_added, vec![50_000]);
    assert!(w.move_transient(&det, 30_000, 20_000, rate));
    assert_eq!((w.transients_added.clone(), w.transients_removed.clone()), (vec![20_000, 50_000], vec![30_000]));
    assert!(!w.move_transient(&det, 7, 9, rate), "nothing at 7");
}

#[test]
fn drums_and_auto_slice_at_the_transients_in_the_clip() {
    let (mut d, c, a) = doc();
    let det = vec![
        Transient { source: 0, strength: 1.0, percussive: true },
        Transient { source: 22_050, strength: 1.0, percussive: true },
        Transient { source: 88_200, strength: 1.0, percussive: false },
        Transient { source: 300_000, strength: 1.0, percussive: true },
    ];
    let tr = HashMap::from([(a, det)]);
    set(&mut d, c, ClipWarp { enabled: true, mode: WarpMode::Drums, ..ClipWarp::default() }, "Set clip mode");
    let w = snap_warp(&d, c, &tr).2.unwrap();
    assert_eq!(w.mode, 1);
    let k = RATE / 44_100.0;
    assert_eq!(w.slices, vec![(0.0, true), (22_050.0 * k, true), (88_200.0 * k, false)]);
    // Trimmed start: the first slice starts at the clip start, flagged like its passage.
    d.execute(Command::TrimClip { clip: c, start: PPQ * 4, source_offset: 44_100, source_length: 44_100 * 3 }, Origin::User).unwrap();
    set(&mut d, c, ClipWarp { enabled: true, mode: WarpMode::Auto, ..ClipWarp::default() }, "Set clip mode");
    let w = snap_warp(&d, c, &tr).2.unwrap();
    assert_eq!(w.mode, 4);
    assert_eq!(w.slices, vec![(44_100.0 * k, true), (88_200.0 * k, false)]);
    assert_eq!(w.points[0].0, 96_000);
    assert!((w.points[0].1 - 48_000.0).abs() < 1e-6);
    for (mode, engine) in [(WarpMode::Melody, 2), (WarpMode::Vocal, 3), (WarpMode::Repitch, 0)] {
        set(&mut d, c, ClipWarp { enabled: true, mode, ..ClipWarp::default() }, "Set clip mode");
        let w = snap_warp(&d, c, &tr).2.unwrap();
        assert_eq!((w.mode, w.slices.len()), (engine, 0));
    }
}

#[test]
fn set_clip_warp_undoes_validates_and_coalesces_drags() {
    let (mut d, c, _) = doc();
    let t0 = Instant::now();
    let on = ClipWarp { enabled: true, ..ClipWarp::default() };
    d.execute_at(Command::SetClipWarp { clip: c, warp: on.clone(), label: "Set clip warp mode".into() }, t0).unwrap();
    assert_eq!(d.undo_label(), Some("Set clip warp mode"));
    // An original-BPM drag is one undo step; the next kind of edit is another.
    for (i, bpm) in [121.0, 122.0, 123.0].into_iter().enumerate() {
        d.execute_at(Command::SetClipWarp { clip: c, warp: ClipWarp { original_bpm: bpm, ..on.clone() }, label: "Set original BPM".into() }, t0 + Duration::from_millis(16 * (i as u64 + 1))).unwrap();
    }
    d.execute_at(Command::SetClipWarp { clip: c, warp: ClipWarp { original_bpm: 123.0, speed: 2.0, ..on.clone() }, label: "Set clip speed".into() }, t0 + Duration::from_millis(80)).unwrap();
    d.undo().unwrap();
    assert_eq!(warp(&d, c).original_bpm, 123.0);
    d.undo().unwrap();
    assert_eq!(warp(&d, c), on);
    d.undo().unwrap();
    assert_eq!(warp(&d, c), ClipWarp::default());
    d.redo().unwrap();
    assert!(warp(&d, c).enabled);
    for bad in [
        ClipWarp { original_bpm: 10.0, ..on.clone() },
        ClipWarp { original_bpm: f64::NAN, ..on.clone() },
        ClipWarp { speed: 0.0, ..on.clone() },
        ClipWarp { markers: vec![WarpMarker { source: 10, beat: 1.0 }, WarpMarker { source: 20, beat: 0.5 }], ..on.clone() },
        ClipWarp { transients_added: vec![44_100 * 9], ..on.clone() },
        ClipWarp { transients_removed: vec![5, 5], ..on.clone() },
    ] {
        assert!(d.execute(Command::SetClipWarp { clip: c, warp: bad, label: "x".into() }, Origin::User).is_err());
    }
    assert!(d.execute(Command::SetClipWarp { clip: ClipId(999), warp: on, label: "x".into() }, Origin::User).is_err());
}

#[test]
fn warp_settings_persist_and_old_projects_load_unwarped() {
    let (mut d, c, _) = doc();
    let mut w = ClipWarp { enabled: true, mode: WarpMode::Vocal, original_bpm: 96.5, speed: 0.75, ..ClipWarp::default() };
    assert!(w.set_marker(4410, 1.0));
    w.transients_added = vec![100];
    w.transients_removed = vec![200];
    set(&mut d, c, w.clone(), "Set clip speed");
    let dir = tempfile::tempdir().unwrap();
    persist::save(d.project(), dir.path()).unwrap();
    let loaded = persist::load(dir.path()).unwrap().project;
    assert_eq!(loaded.clip(c), d.project().clip(c));
    assert_eq!(loaded.clip(c).unwrap().warp, w);
    // An unwarped clip stores nothing, and a clip without the key loads unwarped.
    let json = serde_json::to_value(Clip { warp: ClipWarp::default(), ..loaded.clip(c).unwrap().clone() }).unwrap();
    assert!(json.get("warp").is_none());
    let back: Clip = serde_json::from_value(json).unwrap();
    assert_eq!(back.warp, ClipWarp::default());
    let partial: ClipWarp = serde_json::from_str(r#"{"enabled":true}"#).unwrap();
    assert_eq!((partial.speed, partial.original_bpm, partial.mode), (1.0, 120.0, WarpMode::Auto));
}
