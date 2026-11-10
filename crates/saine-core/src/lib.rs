//! Saine Studio application core.
//!
//! * [`model`]: the authoritative project data model (application thread only)
//! * [`command`]: validated, invertible, serialisable edit commands
//! * [`history`]: undo/redo document wrapper
//! * [`persist`]: versioned project files with external media references
//! * [`snapshot`]: derivation of immutable engine session descriptions
//! * [`warp`]: audio clip warping (tempo following, speed, markers, transients)

pub mod assistant;
pub mod automation;
pub mod builtin;
pub mod command;
pub mod device;
pub mod history;
pub mod import;
pub mod instrument;
pub mod keymap;
pub mod library;
pub mod midi;
pub mod model;
pub mod modulation;
pub mod persist;
pub mod render_plan;
pub mod scale;
pub mod shape;
pub mod snapshot;
pub mod steps;
pub mod warp;

pub use command::{Command, EditError};
pub use history::{Document, Origin};
pub use automation::{AutoPoint, AutomationLane, AutomationTarget, SynthParam};
pub use device::{Device, DeviceId, DeviceSample, ModLink, ModParam, PluginFormat, PluginKind, PluginRef, Slice};
pub use instrument::SynthParams;
pub use midi::{MidiClip, Note};
pub use steps::{StepCell, StepLane, StepPattern, StepRate, StepRows};
pub use model::*;
pub use warp::{ClipWarp, Transient, WarpMarker, WarpMode};
