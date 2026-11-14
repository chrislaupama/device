//! `--showcase <view>`: put an opened project into one of the views used for the website and
//! README artwork (tools/art/demo_song.py writes the song). Applied after `--open`, so the
//! views work on any project; tracks are found by name with sensible fallbacks.

use crate::controller::{BottomTab, Controller};
use crate::devices::Item;
use crate::recording;
use dstudio_core::{TrackId, TrackKind};

pub const VIEWS: &[&str] = &["arrange", "pianoroll", "mixer", "export", "recording", "analyzers", "lfo"];

fn track_named(c: &Controller, name: &str, kind: TrackKind) -> Option<TrackId> {
    let tracks = &c.project().tracks;
    tracks.iter().find(|t| t.name.eq_ignore_ascii_case(name)).or_else(|| tracks.iter().rev().find(|t| t.kind == kind)).map(|t| t.id)
}

const RATE: f64 = 48_000.0;

/// Feeds `seconds` of injected input (`signal(t)`, t in seconds) through the offline engine in
/// 30 ms steps, polling the device views (meters, analysers) after each step as if it played live.
fn feed(c: &mut Controller, seconds: f64, mut signal: impl FnMut(f64) -> f64) -> Result<(), String> {
    let step = (RATE * 0.03) as usize;
    let mut at = std::time::Instant::now() - std::time::Duration::from_secs_f64(seconds);
    for k in 0..(seconds * RATE) as usize / step {
        let input: Vec<f32> = (0..step).map(|i| signal((k * step + i) as f64 / RATE) as f32).collect();
        let mut out = vec![0f32; 2 * step];
        c.engine.render_offline_with_input(&input, 1, &mut out).map_err(|e| format!("--showcase: {e}"))?;
        at += std::time::Duration::from_millis(30);
        c.panel_poll_views_at(at);
    }
    Ok(())
}

pub fn apply(c: &mut Controller, view: &str) -> Result<(), String> {
    let lead = track_named(c, "Lead", TrackKind::Instrument);
    let bar = |b: f64| b * 4.0;
    match view {
        "arrange" => {
            // The whole song, with the lead's Synth open below.
            if let Some(t) = lead {
                c.select_track(t);
            }
            c.set_bottom_tab(BottomTab::Devices);
            c.seek_beats(bar(14.5));
        }
        "pianoroll" => {
            // The lead's first clip open in the piano roll.
            let clip = lead.and_then(|t| c.project().track(t)).and_then(|t| t.midi_clips.first()).map(|m| (m.id, m.start));
            let (Some(t), Some((clip, start))) = (lead, clip) else {
                return Err("--showcase pianoroll: no instrument track with a MIDI clip".into());
            };
            c.select_track(t);
            c.select_clip(Some(clip));
            c.seek_beats(start as f64 / dstudio_core::PPQ as f64 + 2.0);
            c.pr_sync_clip();
            // Zoomed out a little, the phrase's top note a row under the ruler.
            c.pr_zoom(0, 0.2);
            if let Some(hi) = c.pr_clip().and_then(|m| m.notes.iter().map(|n| n.pitch as i32).max()) {
                c.pr.scroll = 0.0;
                let v = c.pr_view();
                c.pr.scroll = (v.y_of(hi) - crate::pianoroll::GRID_Y - v.row_h).clamp(0.0, v.max_scroll());
            }
        }
        "mixer" => {
            if let Some(t) = lead {
                c.select_track(t);
            }
            c.mixer_open = true;
            c.ui_dirty = true;
        }
        "export" => {
            c.export_open();
            c.export_pick_recipe(1);
        }
        "recording" => {
            // A new take on the vocal track in the break, 4 s of input so far
            // (offline engine, injected input: no audio device is opened).
            let Some(t) = track_named(c, "Vocal", TrackKind::Audio) else {
                return Err("--showcase recording: no audio track".into());
            };
            c.rec.policy = recording::InputPolicy::InjectedOnly;
            c.rec.dir_override = Some(std::env::temp_dir().join(format!("dstudio-showcase-rec-{}", std::process::id())));
            c.select_track(t);
            c.execute(dstudio_core::Command::SetTrackArm { track: t, arm: true });
            // Monitoring on, so the take plays through the chain and its analysers.
            c.execute(dstudio_core::Command::SetTrackMonitor { track: t, monitor: true });
            c.set_track_input(t, Some(dstudio_core::InputSource { channel: 0, stereo: false }));
            // The break, where the vocal track is empty (else the last section).
            let sections = &c.project().sections;
            let at = sections.iter().position(|s| s.name.eq_ignore_ascii_case("Break")).unwrap_or(sections.len().saturating_sub(1));
            let start: dstudio_core::Ticks = sections.iter().take(at).map(|s| s.length).sum();
            c.seek_beats(start as f64 / dstudio_core::PPQ as f64);
            c.start_recording();
            if c.rec.recording {
                // A sung phrase every 0.6 s, a little vibrato.
                feed(c, 4.0, |t| {
                    let phrase = (t * 1.6).fract();
                    let env = if phrase < 0.75 { (phrase * 12.0).min(1.0) * (1.0 - phrase / 0.75).sqrt() } else { 0.0 };
                    let ph = std::f64::consts::TAU * 247.0 * t + 0.012 * 247.0 / 5.5 * (1.0 - (std::f64::consts::TAU * 5.5 * t).cos());
                    0.6 * env * (ph.sin() + 0.5 * (2.0 * ph).sin() + 0.3 * (3.0 * ph).sin())
                })?;
            }
        }
        "analyzers" => {
            // The vocal track's chain (the demo song's is Pitch Corrector, Tuner, Spectrum) fed two
            // seconds of a sung line through the input (offline engine, injected input), the
            // views polled as it plays.
            let Some(t) = track_named(c, "Vocal", TrackKind::Audio) else {
                return Err("--showcase analyzers: no audio track".into());
            };
            c.rec.policy = recording::InputPolicy::InjectedOnly;
            c.select_track(t);
            c.execute(dstudio_core::Command::SetTrackMonitor { track: t, monitor: true });
            c.set_track_input(t, Some(dstudio_core::InputSource { channel: 0, stereo: false }));
            c.set_bottom_tab(BottomTab::Devices);
            c.seek_beats(bar(12.0));
            // A3 a little sharp with vibrato, then a held C4 a little flat.
            let mut phase = 0.0f64;
            feed(c, 2.0, move |t| {
                let midi = if t < 1.0 { 57.35 + 0.25 * (std::f64::consts::TAU * 5.0 * t).sin() } else { 59.7 };
                phase += std::f64::consts::TAU * 440.0 * 2f64.powf((midi - 69.0) / 12.0) / RATE;
                (1..=8).map(|h| (phase * h as f64).sin() / h as f64).sum::<f64>() * 0.2
            })?;
        }
        "lfo" => {
            // An LFO on the FX track's Auto Filter frequency (added when the track has none).
            let Some(t) = track_named(c, "FX", TrackKind::Audio) else {
                return Err("--showcase lfo: no audio track".into());
            };
            c.select_track(t);
            let filter = c.project().track(t).and_then(|tr| tr.devices.iter().find(|d| d.plugin.id == "auto-filter")).map(|d| d.id);
            let filter = match filter {
                Some(id) => id,
                None => {
                    let def = dstudio_core::builtin::find("auto-filter").ok_or("--showcase lfo: no Auto Filter")?;
                    c.add_item_to_track(&Item::Builtin(def), t, None);
                    c.project().track(t).and_then(|tr| tr.devices.last()).map(|d| d.id).ok_or("--showcase lfo: no device")?
                }
            };
            c.mod_new_lfo(filter, dstudio_core::ModParam::Builtin("frequency".into()));
            c.set_bottom_tab(BottomTab::Devices);
            c.seek_beats(bar(6.0));
        }
        v => return Err(format!("unknown --showcase view {v} (supported: {})", VIEWS.join(", "))),
    }
    c.ui_dirty = true;
    Ok(())
}
