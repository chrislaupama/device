# ADR 0002: Audio input, monitoring and recording

Status: accepted (2026-11-08). Implementation in progress.

## Context
The reference lets you arm audio tracks (R), choose a per-track input ("None", "Input 1 (Mono)", ...),
monitor input (I), record from the transport, and configure output/input device, sample rate and
buffer size in Settings › Audio (reference/ui-inventory.md §11). Our engine was output-only.

## Decision
- **Model (saine-core):** `Track.arm`, `Track.monitor`, `Track.input: Option<InputSource { channel, stereo }>`,
  with `SetTrackArm`, `SetTrackMonitor` and `SetTrackInput` (audio tracks only). The snapshot carries them to the
  engine. A finished take becomes an undoable `Batch` "Record": `AddAsset` + `AddClip` on the armed track at
  the take's latency-compensated start.
- **Device setup is an app preference, not project state:** output/input device, sample rate, buffer size and
  input mute.
- **Engine API (Rust contract in `crates/saine-engine/src/lib.rs`):** `list_devices`, `configure_device`,
  `device_status`, `set_input_muted`, `start_recording(dir)`, `stop_recording() -> Vec<RecordedTake>`,
  `recording_state`, `recording_peaks`, and `render_offline_with_input` for deterministic tests.
- **Real-time path:** the input callback writes into preallocated per-track ring buffers. A non-RT writer
  thread drains them into 32-bit float WAV files. The audio thread never touches the disk, and a ring
  overrun is counted and reported, never blocking.
- **macOS duplex:** built-in mic and speakers are separate Core Audio devices with independent clocks. The
  engine must either create a private (non-persistent) aggregate device with drift correction, or run a
  separate input unit with an adaptive-rate ring buffer. Documented in engine/README.md.
- **Latency compensation:** takes are shifted earlier by input latency + output latency + safety offsets as
  reported by the device.
- **Microphone permission:** opening an input device on macOS can show the system microphone-permission
  prompt for the host application. Automated tests use injected input only. A real-input check runs
  only with `SAINE_TEST_REAL_INPUT=1`, after the user agrees.
