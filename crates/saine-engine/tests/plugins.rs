//! End-to-end plugin hosting from the project model: AddDevice (the CLAP
//! test gain built with the engine) → snapshot → publish → offline render;
//! undo of a removal restores the device with the same id and state;
//! plugin_states + sync_device_states round-trip through persist.

use saine_core::*;
use saine_engine::{Config, Engine};
use std::collections::HashMap;
use std::path::Path;

fn write_dc_wav(path: &Path, frames: u32, v: i16) {
    let data_len = frames * 4;
    let mut b = Vec::new();
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data_len).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&48_000u32.to_le_bytes());
    b.extend_from_slice(&(48_000u32 * 4).to_le_bytes());
    b.extend_from_slice(&4u16.to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data_len.to_le_bytes());
    for _ in 0..frames * 2 {
        b.extend_from_slice(&v.to_le_bytes());
    }
    std::fs::write(path, b).unwrap();
}

fn clap_gain() -> PluginRef {
    PluginRef {
        format: PluginFormat::Clap,
        id: "studio.saine.test.gain".into(),
        path: Some(saine_engine::test_plugin_dir().join("SaineTestGain.clap").to_string_lossy().into_owned()),
        name: "Saine Test Gain".into(),
        vendor: "Saine".into(),
        version: "1.0.0".into(),
        kind: PluginKind::Effect,
    }
}

/// CLAP test gain state: the gain as a little-endian f32.
fn gain_state(g: f32) -> String {
    device::b64::encode(&g.to_le_bytes())
}

struct Rig {
    engine: Engine,
    doc: Document,
    map: HashMap<AssetId, u64>,
}

impl Rig {
    fn new(project: Project) -> Self {
        Rig { engine: Engine::new(Config { sample_rate: 48_000.0, max_block_size: 512 }).unwrap(), doc: Document::new(project), map: HashMap::new() }
    }
    fn import(&mut self, path: &Path) -> AssetId {
        let info = self.engine.load_file(path).unwrap();
        self.doc
            .execute(
                Command::AddAsset { asset: Asset { id: AssetId(0), relative_path: None, absolute_path: path.to_string_lossy().into(), sample_rate: 48_000.0, channels: 2, frames: info.frames } },
                Origin::User,
            )
            .unwrap();
        let id = self.doc.project().assets.last().unwrap().id;
        self.map.insert(id, info.asset_id);
        id
    }
    fn map_existing_assets(&mut self) {
        for a in self.doc.project().assets.clone() {
            let info = self.engine.load_file(Path::new(&a.absolute_path)).unwrap();
            self.map.insert(a.id, info.asset_id);
        }
    }
    fn publish(&self) {
        let (snap, skipped) = snapshot::build(self.doc.project(), self.engine.sample_rate(), &self.map);
        assert!(skipped.is_empty());
        self.engine.publish(&snap).unwrap();
    }
    /// Left channel of `frames` frames rendered from the start.
    fn render_left(&self, frames: usize) -> Vec<f32> {
        self.engine.seek(0).unwrap();
        self.engine.play().unwrap();
        let mut out = vec![0f32; frames * 2];
        for chunk in out.chunks_mut(2 * 300) {
            self.engine.render_offline(chunk).unwrap();
        }
        out.iter().step_by(2).copied().collect()
    }
}

const PAN_C: f32 = std::f32::consts::FRAC_1_SQRT_2;

#[test]
fn clap_gain_from_the_model_halves_the_signal_and_survives_undo_and_persist() {
    let dir = tempfile::tempdir().unwrap();
    let wav = dir.path().join("dc.wav");
    write_dc_wav(&wav, 9600, 16384); // 0.5 full scale
    let mut rig = Rig::new(Project::default());
    let asset = rig.import(&wav);
    let track = rig.doc.project().tracks[0].id;
    rig.doc.execute(Command::AddClip { track, clip: Clip { id: ClipId(0), asset, name: "dc".into(), start: 0, source_offset: 0, source_length: 9600, gain: 1.0, warp: Default::default() } }, Origin::User).unwrap();
    rig.publish();
    let dry = rig.render_left(4800);
    assert!((dry[100] - 0.5 * PAN_C).abs() < 1e-4, "dry {}", dry[100]);

    // AddDevice with the CLAP test gain (default gain 0.5).
    rig.doc.execute(Command::AddDevice { track, index: None, device: Device::new(clap_gain()) }, Origin::User).unwrap();
    let dev = rig.doc.project().tracks[0].devices[0].clone();
    assert_ne!(dev.id.0, 0);
    rig.publish();
    assert!(rig.engine.plugin_load_errors().is_empty(), "{:?}", rig.engine.plugin_load_errors());
    let wet = rig.render_left(4800);
    for i in [0, 100, 4799] {
        assert!((wet[i] - 0.5 * dry[i]).abs() < 1e-6, "frame {i}: {} vs {}", wet[i], dry[i]);
    }
    assert_eq!(rig.engine.plugin_latency(dev.id), 0);
    assert!(!rig.engine.plugin_editor_is_open(dev.id));

    // The engine's state of the live instance (default gain 0.5).
    // (The default instrument track's built-in Synth reports its state too.)
    let states: Vec<_> = rig.engine.plugin_states().into_iter().filter(|(id, _)| *id == dev.id).collect();
    assert_eq!(states, vec![(dev.id, gain_state(0.5))]);

    // Bypass → dry signal bit-exact.
    rig.doc.execute(Command::SetDeviceEnabled { device: dev.id, enabled: false }, Origin::User).unwrap();
    rig.publish();
    assert_eq!(rig.render_left(4800), dry);
    rig.doc.undo().unwrap();

    // A device whose model state says gain 0.25: replace it via remove + add.
    rig.doc.execute(Command::RemoveDevice { device: dev.id }, Origin::User).unwrap();
    let mut quarter = Device::new(clap_gain());
    quarter.state = Some(gain_state(0.25));
    rig.doc.execute(Command::AddDevice { track, index: None, device: quarter }, Origin::User).unwrap();
    let q = rig.doc.project().tracks[0].devices[0].clone();
    rig.publish();
    let out = rig.render_left(4800);
    assert!((out[100] - 0.25 * dry[100]).abs() < 1e-6, "state applied at instantiation: {}", out[100]);

    // Remove it, then undo: same id and state come back; the engine
    // instantiates it again with that state.
    rig.doc.execute(Command::RemoveDevice { device: q.id }, Origin::User).unwrap();
    rig.publish();
    assert_eq!(rig.render_left(4800), dry);
    assert!(rig.engine.plugin_states().iter().all(|(id, _)| *id != q.id));
    rig.doc.undo().unwrap();
    let restored = rig.doc.project().tracks[0].devices[0].clone();
    assert_eq!(restored.id, q.id);
    assert_eq!(restored.state, Some(gain_state(0.25)));
    rig.publish();
    let out = rig.render_left(4800);
    assert!((out[100] - 0.25 * dry[100]).abs() < 1e-6, "undo restored the state: {}", out[100]);

    // plugin_states → sync_device_states → persist → load → new engine.
    let states: Vec<_> = rig.engine.plugin_states().into_iter().filter(|(id, _)| *id == q.id).collect();
    assert_eq!(states, vec![(q.id, gain_state(0.25))]);
    rig.doc.sync_device_states(&states);
    let proj_dir = dir.path().join("project");
    persist::save(rig.doc.project(), &proj_dir).unwrap();
    let loaded = persist::load(&proj_dir).unwrap();
    assert!(loaded.missing_media.is_empty());
    assert_eq!(loaded.project.tracks[0].devices[0].state, Some(gain_state(0.25)));
    let mut rig2 = Rig::new(loaded.project);
    rig2.map_existing_assets();
    rig2.publish();
    let out2 = rig2.render_left(4800);
    assert_eq!(out2, out, "reloaded project renders identically");
}

#[test]
fn missing_plugin_is_reported_and_passes_through() {
    let rig = Rig::new(Project::default());
    let mut doc = rig.doc;
    let track = doc.project().tracks[0].id;
    let mut missing = clap_gain();
    missing.path = Some("/nonexistent/Missing.clap".into());
    doc.execute(Command::AddDevice { track, index: None, device: Device::new(missing) }, Origin::User).unwrap();
    let id = doc.project().tracks[0].devices[0].id;
    let (snap, _) = snapshot::build(doc.project(), rig.engine.sample_rate(), &rig.map);
    rig.engine.publish(&snap).unwrap();
    let errs = rig.engine.plugin_load_errors();
    assert_eq!(errs.len(), 1);
    assert_eq!(errs[0].0, id);
    assert!(errs[0].1.contains("Missing.clap"), "{}", errs[0].1);
    assert!(rig.engine.plugin_states().iter().all(|(d, _)| *d != id));
    assert!(rig.engine.open_plugin_editor(id).is_err());
}

/// The out-of-process scanner finds the test plugins through
/// `$SAINE_PLUGIN_PATH` and blocklists the bundle that crashes while scanned.
#[test]
fn scanner_finds_test_plugins_and_blocklists_the_crash() {
    // SAINE_PLUGIN_PATH is read by the scanner child process.
    // SAFETY: set before spawning; no other test in this binary reads it.
    unsafe { std::env::set_var("SAINE_PLUGIN_PATH", saine_engine::test_plugin_dir()) };
    let dir = tempfile::tempdir().unwrap();
    let cache = dir.path().join("plugins.json");
    let mut lines = Vec::new();
    let report = saine_engine::scan_plugins(&cache, false, &mut |s| lines.push(s.to_string())).unwrap();
    assert!(!report.from_cache);
    assert!(!lines.is_empty());
    let find = |id: &str| report.plugins.iter().find(|p| p.plugin.id == id).cloned();
    let gain = find("studio.saine.test.gain").expect("CLAP gain");
    assert_eq!(gain.plugin.format, PluginFormat::Clap);
    assert_eq!(gain.plugin.kind, PluginKind::Effect);
    assert_eq!((gain.audio_inputs, gain.audio_outputs), (2, 2));
    assert_eq!(find("studio.saine.test.sine").unwrap().plugin.kind, PluginKind::Instrument);
    assert_eq!(find("5A17E5A16E0000018A1E7E576A170001").unwrap().plugin.format, PluginFormat::Vst3);
    // Audio Units exist only on macOS.
    #[cfg(target_os = "macos")]
    {
        let lowpass = find("aufx lpas appl").expect("Apple AULowpass");
        assert_eq!(lowpass.plugin.format, PluginFormat::AudioUnit);
        assert!(lowpass.plugin.path.is_none());
    }
    #[cfg(not(target_os = "macos"))]
    assert!(report.plugins.iter().all(|p| p.plugin.format != PluginFormat::AudioUnit));
    assert!(report.failed.iter().any(|(t, why)| t.ends_with("SaineTestCrash.clap") && why.contains("crashed")), "{:?}", report.failed);
    let again = saine_engine::scan_plugins(&cache, false, &mut |_| {}).unwrap();
    assert!(again.from_cache);
    assert_eq!(again.plugins.len(), report.plugins.len());
    assert_eq!(again.failed, report.failed);
}
