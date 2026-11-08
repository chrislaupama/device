//! Saine Studio application core.
//!
//! * [`model`]: the authoritative project data model (application thread only)
//! * [`command`]: validated, invertible, serialisable edit commands
//! * [`history`]: undo/redo document wrapper
//! * [`persist`]: versioned project files with external media references
//! * [`snapshot`]: derivation of immutable engine session descriptions

pub mod assistant;
pub mod command;
pub mod device;
pub mod history;
pub mod instrument;
pub mod keymap;
pub mod midi;
pub mod model;
pub mod persist;
pub mod snapshot;

pub use command::{Command, EditError};
pub use history::{Document, Origin};
pub use device::{Device, DeviceId, PluginFormat, PluginKind, PluginRef};
pub use instrument::SynthParams;
pub use midi::{MidiClip, Note};
pub use model::*;
