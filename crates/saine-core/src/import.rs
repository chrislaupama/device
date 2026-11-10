//! Copying a track out of another project (the Files browser's "add track").
//!
//! Every id the track carries (track, clips, devices, assets) is replaced by a
//! fresh one from the destination's allocator, and references between them
//! (device chains, modulation targets, automation targets, sample slots) are
//! rewritten to match. A sidechain source in the other project has no meaning
//! here, so it is cleared.

use crate::automation::AutomationTarget;
use crate::device::{Device, DeviceId};
use crate::model::{Asset, AssetId, ClipId, Project, Track, TrackId};
use std::collections::HashMap;

/// The track `track` of `src` with fresh ids taken from `next_id` (advanced),
/// plus the assets it uses (fresh ids, absolute paths only). None when `src`
/// has no such track.
pub fn import_track(src: &Project, track: TrackId, next_id: &mut u64) -> Option<(Track, Vec<Asset>)> {
    let mut t = src.track(track)?.clone();
    let mut ids: HashMap<u64, u64> = HashMap::new();
    let mut fresh = |old: u64| {
        *ids.entry(old).or_insert_with(|| {
            let id = *next_id;
            *next_id += 1;
            id
        })
    };
    let mut assets = Vec::new();
    let mut asset = |old: AssetId, fresh: &mut dyn FnMut(u64) -> u64| {
        let id = AssetId(fresh(old.0));
        if !assets.iter().any(|a: &Asset| a.id == id)
            && let Some(a) = src.asset(old)
        {
            assets.push(Asset { id, relative_path: None, ..a.clone() });
        }
        id
    };
    t.id = TrackId(fresh(t.id.0));
    for c in &mut t.clips {
        c.id = ClipId(fresh(c.id.0));
        c.asset = asset(c.asset, &mut fresh);
    }
    for c in &mut t.midi_clips {
        c.id = ClipId(fresh(c.id.0));
    }
    let devices: Vec<&mut Device> = t.devices.iter_mut().chain(t.instrument_plugin.as_mut()).collect();
    for d in devices {
        d.id = DeviceId(fresh(d.id.0));
        d.chain = d.chain.map(|c| DeviceId(fresh(c.0)));
        for l in &mut d.links {
            l.target = DeviceId(fresh(l.target.0));
        }
        for s in &mut d.samples {
            s.asset = asset(s.asset, &mut fresh);
        }
        d.sidechain = None;
    }
    for lane in &mut t.automation {
        if let AutomationTarget::Plugin { device, .. } | AutomationTarget::Builtin { device, .. } = &mut lane.target {
            *device = DeviceId(fresh(device.0));
        }
    }
    t.arm = false;
    t.solo = false;
    Some((t, assets))
}
