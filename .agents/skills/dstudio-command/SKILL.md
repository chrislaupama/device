---
name: dstudio-command
description: Add or change an edit in Device Studio's application core - a dstudio_core::Command with validation, exact inverse, coalescing, undo label and JSON form - plus the model, persistence, migration and snapshot changes it needs. Use whenever a change lets the user (or the assistant, or a script) modify a project.
---

# Adding or changing an edit

Every change to a project goes through one path: a `Command` executed on the `Document`. The UI,
keyboard shortcuts, scripts, tests and the AI assistant all use it, so getting a command right once
gives undo, redo, coalescing, labels and scripting everywhere. Background:
`docs/architecture.md` › Command system and › Persistence.

Where things live:

| What | Where |
|---|---|
| The `Command` enum, `apply`, `label`, `coalesce_key`, `EditError` | `crates/dstudio-core/src/command.rs` |
| `Document`: execute, undo/redo, coalescing window (750 ms), revisions | `crates/dstudio-core/src/history.rs` |
| Model types (`Project`, `Track`, `Clip`, ...) | `crates/dstudio-core/src/model.rs` (devices: `device.rs`) |
| Save, load, `SCHEMA_VERSION`, `migrate` | `crates/dstudio-core/src/persist.rs` |
| Musical time to frames, what the engine receives | `crates/dstudio-core/src/snapshot.rs` |
| Assistant tool schema and edit summaries | `crates/dstudio-core/src/assistant.rs` |
| The app's entry point for edits | `Controller::execute` in `crates/dstudio-app/src/controller.rs` |

## 1. Reuse before adding

Check whether an existing command, or a `Command::Batch` of existing ones, already expresses the
edit. A batch is atomic, rolls back on the first failure, and takes one label. Add a new variant
only when the edit is a new kind of change.

## 2. The variant

- Add it to `Command` with a doc comment. The enum is serialised with `#[serde(tag = "op",
  rename_all = "snake_case")]`, so the variant name becomes its JSON `op`; pick it as carefully as
  a public API name, because scripts and saved assistant turns use it.
- Use the id newtypes (`TrackId`, `ClipId`, `DeviceId`, ...). A new object gets id 0 in the command
  and a fresh id inside `apply`; ids are assigned before execution so redo reproduces them, and are
  never recycled.

## 3. `apply`

`apply(self, &mut Project) -> Result<Command, EditError>` must:

- **Validate everything first** (existence, ranges, kind compatibility, asset bounds) and return an
  `EditError` before touching the project. On error the project is unchanged. Add validation limits
  as named constants next to the existing ones.
- **Return the exact inverse**: applying the inverse restores the project bit for bit, including
  order, ids and defaults. For multi-step edits the inverse is usually a `Batch` in reverse order.
- Stay free of I/O, clocks and randomness. Anything like that is computed by the caller and passed
  in, so redo and tests are deterministic.

## 4. Label and coalescing

- `label()` gives the "Undo ⟨label⟩" text: title case, the user's words ("Add Audio Track",
  "Key Change").
- If the edit is driven by a continuous gesture (drags, scrubs, typing into a value), give it a
  `coalesce_key()` that identifies the target (`"devparam:{device}:{param}"`). Edits with the same
  key inside the coalescing window merge into one undo step. Discrete edits return `None`.

## 5. Persistence

- New model fields get `#[serde(default)]` (and `skip_serializing_if` where the default is the
  common case) so projects saved before the field existed still load.
- If the saved shape changes in a way defaults can't absorb (a rename, a move, a change of
  meaning), bump `SCHEMA_VERSION` and add the step to `persist::migrate`. Never make an old project
  fail to load or load wrongly; a newer-than-supported file is refused.
- Built-in device parameters are stored by id and are append-only (`dstudio-builtin-device`).

## 6. Snapshot

If the engine has to hear the change, extend `snapshot.rs` so the `SessionSnapshot` carries it in
frames at the engine rate. The engine never sees ticks, tempo or the `Project`. Crossing the ABI is
`dstudio-engine-abi`.

## 7. Assistant and app

- If the assistant may perform the edit, cover the variant in `assistant.rs` (its summary text,
  and the tool schema if it is exposed). Teach mode never executes commands.
- In the app, call `Controller::execute` (it publishes the new snapshot). During a drag, send live
  values to the engine directly and commit one command on release.

## 8. Tests

In `crates/dstudio-core/tests/` (see `core.rs` for the style), add a few sharp tests:

- apply, then undo restores the project exactly, then redo reproduces the same ids;
- the undo label;
- each validation error leaves the project unchanged;
- coalescing merges a burst of edits into one step (when it has a key);
- a JSON round trip of the command, and of a saved project containing the new state (and loading a
  project saved without it).

Run `cargo test -p dstudio-core`.
