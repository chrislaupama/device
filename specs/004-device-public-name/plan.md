# Device name implementation plan

The maintainer approved the spec's compatibility choices and explicitly authorized completing all
remaining phases without further design checkpoints.

## Design

- Core: change descriptive labels, package descriptions, built-in vendor and invalid-format error
  text. Keep FORMAT, schema, device ids and parameters unchanged; no commands, snapshot or ABI changes.
- Engine: replace human-readable CLAP, Core Audio, Core MIDI, ALSA and PipeWire names and the test
  plugin description. Keep platform window classes, aggregate uid prefix and PipeWire node-group key.
  Follow the existing display-string sites; nothing changes on the real-time callback.
- App: replace Slint titles/update copy, native menu HOLDER, assistant context and exported creator
  labels. Keep binary name, prefs/plugin-cache paths, default project dir and temporary media paths.
- Packages: produce Device.app and Device-<version> archives, keep macOS bundle id and Windows AppId,
  executable and Linux desktop-file identity. Inno Setup uses Device as the new default directory and
  group, reuses an existing install directory through its stable AppId, and removes legacy Device Studio
  shortcut files on upgrade. Update workflow paths/titles and smoke scripts together.
- Docs: current branding uses Device; add a compatibility note to building docs and a current progress
  entry. Completed specs, ADRs and historical progress entries keep their original wording.
- Public org/site/docs artwork were updated in separate cosmetic commits before app implementation.
  Keep that result and the contribution/security policies intact.

## Principles check

1. Real-time safety: only fixed display literals change, no new callback work.
2. One mutation path: no project edits or commands are added.
3. Projects keep loading: format, schema and device ids/parameters unchanged.
4. C ABI: no declarations or layout changes.
5. Cross-platform: all display/package names and corresponding workflow references updated together.
6. Original work: reuse project-owned artwork.
7. Evidence: screenshots, package fixture checks and existing full test gate; no performance benchmark
   needed because naming literals do not alter algorithms, drawing, allocation, DSP or threading.
8. Docs: current docs and PROGRESS updated with code.
9. Naming: dstudio internal names and device-studio executable remain stable.
10. Rules: preserve all principles; approval covered compatibility and remaining workflow.

## Verification

Run formatting, clippy and workspace tests; configure/build/test the C++ engine; build the release
app and run its self-test. Capture before/after review states and native app title evidence when
available. Exercise macOS bundling with fixture executables and plist checks, syntax-check shell scripts,
and inspect all workflow package paths. Windows/Linux native installer execution is delegated to
existing CI and must be reported accurately.

## Alternatives

A full identifier/path rename was rejected because it would disrupt projects, scripts, settings,
OS grants and upgrade identity. Migration logic is unnecessary for a display-name change.

