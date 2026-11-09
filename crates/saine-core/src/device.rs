//! Track device chains: built-in devices (crate::builtin) and third-party
//! plugins (Audio Unit, VST3, CLAP) used as insert effects, plus an optional
//! instrument plugin on instrument tracks.
//!
//! Built-in devices keep their parameter values in `Device::params` (plain
//! values by parameter id, edited with `Command::SetDeviceParam`), so they
//! have full undo; the rest of this note is about third-party plugins.
//!
//! The model only *names* plugins and stores their opaque state; the engine
//! owns instances. Instances are keyed by `DeviceId` and survive republishes,
//! so `state` is applied only when an instance is created (project load,
//! undo of a removal, duplicate). Editor changes are pulled back into the
//! model with `Document::sync_device_states` before saving (no undo step:
//! plugin editors have their own internal undo and the host cannot observe
//! individual edits).

use crate::model::TrackId;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DeviceId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginFormat {
    AudioUnit,
    Vst3,
    Clap,
    /// A built-in device; `PluginRef::id` is its definition id (crate::builtin).
    Builtin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginKind {
    Effect,
    Instrument,
}

/// Identifies a plugin independently of the machine it was scanned on.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PluginRef {
    pub format: PluginFormat,
    /// Format-specific id: AU "type subtype manufacturer" four-char codes
    /// (e.g. "aufx dely appl"); VST3 class UID (32 hex chars); CLAP plugin id.
    pub id: String,
    /// Bundle path where it was found (VST3/CLAP; informational for AU).
    #[serde(default)]
    pub path: Option<String>,
    pub name: String,
    pub vendor: String,
    #[serde(default)]
    pub version: String,
    pub kind: PluginKind,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Device {
    pub id: DeviceId,
    pub plugin: PluginRef,
    /// Bypass when false (reference device header power toggle).
    pub enabled: bool,
    /// Opaque plugin state, base64 (AU class info plist / VST3 component +
    /// controller state / CLAP state stream, as produced by the engine).
    #[serde(default)]
    pub state: Option<String>,
    /// Built-in devices: plain parameter values by parameter id (missing ids
    /// read as the definition's default).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, f32>,
    /// Built-in devices with a sidechain input: the source track.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sidechain: Option<TrackId>,
    /// Name of the preset last loaded or saved (shown in the card header).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset: Option<String>,
    /// Member of the chain (rack) whose head device has this id. Members sit
    /// right after their head in the track's chain (crate::modulation).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chain: Option<DeviceId>,
    /// Modulation sources (LFO, chain head macros): the parameters this
    /// device moves, in the order they were added.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<ModLink>,
    /// Display name chosen by the user or generated ("LFO 2", "Chain").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Chain heads: macro names (empty or missing = "Macro N").
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub macro_names: Vec<String>,
    /// Card folded to a narrow strip (header fold button, double-click).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub folded: bool,
    /// Chain heads: member cards hidden.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub collapsed: bool,
    /// Third-party plugins: parameter ids shown on the card, in the order added.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub card_params: Vec<u32>,
    /// Third-party plugins: (parameter id, normalized base) of each modulated
    /// parameter when saved; `state` holds the moving value instead.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mod_bases: Vec<(u32, f32)>,
}

/// The parameter a modulation link moves.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModParam {
    /// Built-in device parameter by id.
    Builtin(String),
    /// Third-party plugin parameter by its format id (normalized 0..1 domain).
    Plugin(u32),
}

/// One modulation link of a source device (crate::modulation): the target
/// parameter and its range in the target's normalized 0..1 domain.
///
/// LFO: `min` / `max` span -1..1 around the knob's own value (created as
/// `-amount..+amount`); the offset is `(min + (max - min) * wave) * depth`
/// with wave in 0..1. Chain macro: the parameter's normalized value becomes
/// `min + (max - min) * macro` (min above max inverts).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModLink {
    /// Source output: the macro index on a chain head, 0 on an LFO.
    #[serde(default)]
    pub slot: u8,
    pub target: DeviceId,
    pub param: ModParam,
    pub min: f32,
    pub max: f32,
}

impl Device {
    pub fn new(plugin: PluginRef) -> Self {
        Device {
            id: DeviceId(0),
            plugin,
            enabled: true,
            state: None,
            params: BTreeMap::new(),
            sidechain: None,
            preset: None,
            chain: None,
            links: Vec::new(),
            label: None,
            macro_names: Vec::new(),
            folded: false,
            collapsed: false,
            card_params: Vec::new(),
            mod_bases: Vec::new(),
        }
    }

    /// Name shown in the card header: the user's label, else the plugin name.
    pub fn display_name(&self) -> &str {
        self.label.as_deref().unwrap_or(&self.plugin.name)
    }
}

/// Minimal standard base64 (RFC 4648, with padding) so core needs no extra
/// dependency for plugin state blobs.
pub mod b64 {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    pub fn encode(data: &[u8]) -> String {
        let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
        for c in data.chunks(3) {
            let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
            out.push(T[(n >> 18) as usize & 63] as char);
            out.push(T[(n >> 12) as usize & 63] as char);
            out.push(if c.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
            out.push(if c.len() > 2 { T[n as usize & 63] as char } else { '=' });
        }
        out
    }

    pub fn decode(s: &str) -> Option<Vec<u8>> {
        let s = s.trim_end_matches('=');
        let val = |c: u8| T.iter().position(|&t| t == c).map(|p| p as u32);
        let mut out = Vec::with_capacity(s.len() * 3 / 4);
        let bytes: Vec<u8> = s.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
        for c in bytes.chunks(4) {
            if c.len() == 1 {
                return None;
            }
            let mut n = 0u32;
            for (i, &b) in c.iter().enumerate() {
                n |= val(b)? << (18 - 6 * i);
            }
            out.push((n >> 16) as u8);
            if c.len() > 2 {
                out.push((n >> 8) as u8);
            }
            if c.len() > 3 {
                out.push(n as u8);
            }
        }
        Some(out)
    }

    #[cfg(test)]
    mod tests {
        #[test]
        fn round_trip() {
            for s in ["", "f", "fo", "foo", "foob", "fooba", "foobar"] {
                assert_eq!(super::decode(&super::encode(s.as_bytes())).unwrap(), s.as_bytes());
            }
            assert_eq!(super::encode(b"foobar"), "Zm9vYmFy");
            assert_eq!(super::encode(b"fo"), "Zm8=");
            let all: Vec<u8> = (0..=255).collect();
            assert_eq!(super::decode(&super::encode(&all)).unwrap(), all);
            assert!(super::decode("a").is_none());
            assert!(super::decode("@@@@").is_none());
        }
    }
}
