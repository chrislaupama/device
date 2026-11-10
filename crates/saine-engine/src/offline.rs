//! Offline render of a project on a private engine, for jobs that run on a worker thread (the
//! clip menu's Consolidate, Bounce and Export audio, src/clip_render.rs in the app).
//!
//! The engine of the running app is never touched: the render makes its own [`Engine`] without
//! an audio device, loads the project's audio files into it, publishes the project and plays it
//! through [`Engine::render_offline`] in chunks. Plugin devices are instantiated by that engine
//! from their saved state (`Device::state`); a caller that wants a live plugin's current sound
//! copies [`Engine::plugin_states`] into the project first.

use crate::{Config, Engine, EngineError};
use saine_core::snapshot;
use saine_core::Transient;
use saine_core::{AssetId, Project};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

/// Frames per `render_offline` call; the cancel flag is looked at between calls.
pub const CHUNK: usize = 4096;
/// How long a render waits for the engine's background warp renders (a warped clip is silent
/// until its render is attached).
const WARP_WAIT: Duration = Duration::from_secs(60);

/// What to render: `frames` stereo frames of `project` from timeline frame `start`.
pub struct OfflineRender {
    pub project: Project,
    /// Sample rate of the render and of the frame numbers.
    pub rate: f64,
    /// Detected transients by asset, for warped clips (as in the live session).
    pub transients: HashMap<AssetId, Vec<Transient>>,
    pub start: i64,
    pub frames: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OfflineError {
    /// The cancel flag was set; nothing was produced.
    Cancelled,
    Failed(String),
}

impl From<EngineError> for OfflineError {
    fn from(e: EngineError) -> Self {
        OfflineError::Failed(e.to_string())
    }
}

/// Renders `job` to interleaved stereo f32. `progress` counts the frames done (up to
/// `job.frames`); `cancel` is looked at before each file load and between chunks.
pub fn render(job: &OfflineRender, cancel: &AtomicBool, progress: &AtomicUsize) -> Result<Vec<f32>, OfflineError> {
    let check = || if cancel.load(Ordering::Relaxed) { Err(OfflineError::Cancelled) } else { Ok(()) };
    check()?;
    let engine = Engine::new(Config { sample_rate: job.rate, ..Config::default() })?;
    let p = &job.project;

    let used: HashSet<AssetId> = p
        .tracks
        .iter()
        .flat_map(|t| t.clips.iter().filter(|c| !c.disabled).map(|c| c.asset).chain(t.devices.iter().chain(t.instrument_plugin.iter()).flat_map(|d| d.samples.iter().map(|s| s.asset))))
        .collect();
    let mut assets = HashMap::new();
    for id in used {
        check()?;
        let a = p.asset(id).ok_or_else(|| OfflineError::Failed(format!("asset {} is not in the project", id.0)))?;
        let info = engine.load_file(Path::new(&a.absolute_path)).map_err(|e| OfflineError::Failed(format!("{}: {e}", a.absolute_path)))?;
        assets.insert(id, info.asset_id);
    }
    let mut reversed = HashMap::new();
    for c in p.tracks.iter().flat_map(|t| &t.clips).filter(|c| c.warp.reverse && !c.disabled) {
        if let (Some(&ea), false) = (assets.get(&c.asset), reversed.contains_key(&c.asset)) {
            reversed.insert(c.asset, engine.reverse_asset(ea)?.asset_id);
        }
    }

    let (snap, _) = snapshot::build_full(p, job.rate, &assets, &reversed, &job.transients);
    engine.publish(&snap)?;
    let waited = Instant::now();
    while engine.warp_renders_pending() > 0 {
        check()?;
        if waited.elapsed() > WARP_WAIT {
            return Err(OfflineError::Failed("warped audio took too long to prepare".into()));
        }
        engine.plugin_idle();
        std::thread::sleep(Duration::from_millis(2));
    }
    engine.set_loop(false, 0, 0)?;
    engine.stop()?;
    engine.seek(job.start)?;
    engine.play()?;
    let mut out = vec![0f32; job.frames * 2];
    for (i, chunk) in out.chunks_mut(CHUNK * 2).enumerate() {
        check()?;
        engine.render_offline(chunk)?;
        progress.store((i * CHUNK + chunk.len() / 2).min(job.frames), Ordering::Relaxed);
    }
    Ok(out)
}
