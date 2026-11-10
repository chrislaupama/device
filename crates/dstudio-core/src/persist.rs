//! Project persistence.
//!
//! A project is a directory `Name.saineproj/` (portable, VCS-friendly) holding
//! `project.json`. Media stays where it is and is referenced by both a
//! project-relative and an absolute path. The JSON envelope carries a schema
//! version; `migrate` upgrades older documents step by step.
//!
//! SQLite is reserved for large, query-heavy metadata (sample library index,
//! analysis caches), not for the project document itself. See docs/architecture.md.

use crate::model::Project;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use thiserror::Error;

pub const FORMAT: &str = "saine-project";
pub const SCHEMA_VERSION: u32 = 1;
pub const PROJECT_FILE: &str = "project.json";
pub const EXTENSION: &str = "saineproj";

#[derive(Debug, Error)]
pub enum PersistError {
    #[error("i/o error at {path}: {source}")]
    Io { path: PathBuf, source: std::io::Error },
    #[error("malformed project file: {0}")]
    Format(#[from] serde_json::Error),
    #[error("not a DStudio project (format = {0:?})")]
    WrongFormat(String),
    #[error("project schema {found} is newer than supported {supported}")]
    TooNew { found: u32, supported: u32 },
}

fn io(path: &Path) -> impl FnOnce(std::io::Error) -> PersistError + '_ {
    move |source| PersistError::Io { path: path.to_path_buf(), source }
}

#[derive(Serialize, Deserialize)]
struct Envelope {
    format: String,
    schema_version: u32,
    saved_with: String,
    project: serde_json::Value,
}

/// Result of loading: the project plus media that couldn't be found.
#[derive(Debug)]
pub struct Loaded {
    pub project: Project,
    pub missing_media: Vec<String>,
}

/// Save atomically (write temp file, fsync, rename).
pub fn save(project: &Project, dir: &Path) -> Result<(), PersistError> {
    fs::create_dir_all(dir).map_err(io(dir))?;
    let mut p = project.clone();
    let abs_dir = absolute(dir);
    for a in &mut p.assets {
        a.relative_path = relative_to(&abs_dir, Path::new(&a.absolute_path));
    }
    let env = Envelope {
        format: FORMAT.into(),
        schema_version: SCHEMA_VERSION,
        saved_with: concat!("dstudio-core ", env!("CARGO_PKG_VERSION")).into(),
        project: serde_json::to_value(&p)?,
    };
    let json = serde_json::to_vec_pretty(&env)?;
    let target = dir.join(PROJECT_FILE);
    let tmp = dir.join(format!(".{PROJECT_FILE}.tmp"));
    {
        let mut f = fs::File::create(&tmp).map_err(io(&tmp))?;
        f.write_all(&json).map_err(io(&tmp))?;
        f.sync_all().map_err(io(&tmp))?;
    }
    fs::rename(&tmp, &target).map_err(io(&target))?;
    Ok(())
}

pub fn load(dir: &Path) -> Result<Loaded, PersistError> {
    let path = dir.join(PROJECT_FILE);
    let bytes = fs::read(&path).map_err(io(&path))?;
    let env: Envelope = serde_json::from_slice(&bytes)?;
    if env.format != FORMAT {
        return Err(PersistError::WrongFormat(env.format));
    }
    if env.schema_version > SCHEMA_VERSION {
        return Err(PersistError::TooNew { found: env.schema_version, supported: SCHEMA_VERSION });
    }
    let value = migrate(env.schema_version, env.project);
    let mut project: Project = serde_json::from_value(value)?;
    // Instrument tracks saved with the classic synth get the Synth device.
    crate::builtin::synth_v2::upgrade_project(&mut project);
    // A step pattern the editor can't hold (hand-edited or damaged file) is dropped: the
    // notes are the clip's content, so the clip loads as plain MIDI with nothing lost.
    for c in project.tracks.iter_mut().flat_map(|t| &mut t.midi_clips) {
        if c.pattern.as_ref().is_some_and(|p| p.validate().is_err()) {
            c.pattern = None;
        }
        // Likewise a pattern list the sidebar can't show: the clip keeps the notes it plays.
        if c.composer.as_ref().is_some_and(|k| k.validate().is_err()) {
            c.composer = None;
        }
    }
    // Resolve media: prefer the project-relative path (project was moved
    // together with its media), fall back to the absolute path.
    let abs_dir = absolute(dir);
    let mut missing = Vec::new();
    for a in &mut project.assets {
        let rel = a.relative_path.as_ref().map(|r| normalize(&abs_dir.join(r)));
        if let Some(r) = rel.filter(|r| r.exists()) {
            a.absolute_path = r.to_string_lossy().into_owned();
        } else if !Path::new(&a.absolute_path).exists() {
            missing.push(a.absolute_path.clone());
        }
    }
    Ok(Loaded { project, missing_media: missing })
}

/// Upgrade a project JSON value from `from` to the current schema.
fn migrate(from: u32, value: serde_json::Value) -> serde_json::Value {
    #[allow(clippy::match_single_binding)]
    match from {
        // v1 is current. Future: `0 => migrate(1, v0_to_v1(value))`.
        _ => value,
    }
}

fn absolute(p: &Path) -> PathBuf {
    if p.is_absolute() { p.to_path_buf() } else { std::env::current_dir().map(|c| c.join(p)).unwrap_or_else(|_| p.to_path_buf()) }
}

/// Lexically resolve `.` and `..` (no filesystem access, symlinks untouched).
fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Lexical relative path from `base` dir to `target` (both absolute).
fn relative_to(base: &Path, target: &Path) -> Option<String> {
    if !target.is_absolute() {
        return None;
    }
    let b: Vec<Component> = base.components().collect();
    let t: Vec<Component> = target.components().collect();
    // Different roots/prefixes (e.g. Windows drives) → no relative path.
    if b.first() != t.first() {
        return None;
    }
    let common = b.iter().zip(&t).take_while(|(x, y)| x == y).count();
    let mut rel = PathBuf::new();
    for _ in common..b.len() {
        rel.push("..");
    }
    for c in &t[common..] {
        rel.push(c.as_os_str());
    }
    // Store with forward slashes for portability across platforms.
    Some(rel.to_string_lossy().replace('\\', "/"))
}

#[cfg(test)]
mod tests {
    use super::*;
    /// An absolute path on this platform ("/a/b" on Unix, "C:/a/b" on Windows,
    /// where a path without a drive is not absolute).
    fn abs(p: &str) -> PathBuf {
        if cfg!(windows) { PathBuf::from(format!("C:{p}")) } else { PathBuf::from(p) }
    }

    #[test]
    fn relative_paths() {
        assert_eq!(relative_to(&abs("/a/b/p.dsproj"), &abs("/a/b/media/x.wav")).as_deref(), Some("../media/x.wav"));
        assert_eq!(relative_to(&abs("/a/p"), &abs("/a/p/x.wav")).as_deref(), Some("x.wav"));
    }
}
