//! Saine Studio application core.
//!
//! * [`model`]: the authoritative project data model (application thread only)
//! * [`command`]: validated, invertible, serialisable edit commands
//! * [`history`]: undo/redo document wrapper
//! * [`persist`]: versioned project files with external media references
//! * [`snapshot`]: derivation of immutable engine session descriptions

pub mod command;
pub mod history;
pub mod model;
pub mod persist;
pub mod snapshot;

pub use command::{Command, EditError};
pub use history::{Document, Origin};
pub use model::*;
