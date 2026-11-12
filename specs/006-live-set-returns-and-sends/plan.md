# Plan: Return tracks and sends in the Live set export

Spec: [spec.md](spec.md)

## Approach

Only the app layer changes. The set writer (`live_set.rs`) learns return tracks and sends, and the exporter
(`dialogs.rs`) fills them in from the project. No model, command, engine or file-format change.

## Changes by layer

- **`live_set.rs`**: `LiveSet::returns` (`SetReturn`: name, colour, on, volume, pan, pre) and per track `sends`
  (one level per return) and `sends_only`. `mixer_body` writes one `TrackSendHolder` per return (the main and
  pre-hear tracks keep an empty list; a return's own sends are inactive, as Live saves them). `return_track` writes
  a `ReturnTrack` after the ordinary tracks. `SendsPre` gets one `SendPreBool` per return. `validate` checks all of
  it and `SetSummary` counts return tracks and sends.
- **`dialogs.rs`**: `write_project` maps sends to Live levels, marks tracks routed to a return "Sends Only", adds
  returns, renders fed returns with devices and writes the notes. `stem_snapshot` given a return solos it instead
  of making a dry stem (the engine's rule: a soloed return plays alone, fed by every unmuted track).
- **Self-test**: the Project export section adds two returns, sends and an Out change first, and undoes them after.

## Principles check

- Real-time safety: untouched (offline render path only).
- Undo: nothing new is editable.
- File formats: the project format is unchanged. The Live set stays within the elements Live 11 writes.
- Original work: element names and defaults are the public structure of Live's own files, nothing copied.

## Tests and evidence

See the acceptance checks. No screenshots (no visible change in the app) and no benchmark (one more offline render
pass per rendered return, same cost as a track stem).

## Risks

- The `ReturnTrack` layout is written from the documented structure of Live 11 sets and passes the structure check,
  but has not been opened in Live from this environment. A4 stays open until someone opens an exported set.
- "Sends Only" is written as output target `AudioOut/None`.
