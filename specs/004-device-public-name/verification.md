# Device name verification

## Local results (Apple M4, macOS)

- Formatting and diff whitespace checks passed.
- `cargo clippy --workspace` passed with existing warnings; no warning-bearing logic changed.
- `cargo test --workspace`: 552 passed, 0 failed, 10 ignored (existing hardware/platform exclusions).
- CMake default configure/build and CTest: 510 passed, 3 skipped, 0 failed out of 513 registered checks.
  Skips: hardware duplex loopback and Win32/X11 editor checks unavailable on this machine.
- Release app built successfully; `--self-test --no-audio`: **636 / 636 checks passed**.
- All 11 repository skill files passed their consistency check.
- Shell syntax checks passed for bundle/release scripts.
- A fixture macOS bundle verified `Device.app`, Device plist display names, scanner inclusion,
  the unchanged `com.dstudio.studio` identity and the unchanged `device-studio` executable.
- Linux package staging/install layout passed a fixture check, including archive and desktop name.
  The test used a strip stub for fixture shell scripts on macOS; it does not claim native ELF
  execution or a Linux runtime test.
- A direct comparison with the base commit proved preferences/cache/project/scratch code paths,
  persistence format/schema, macOS bundle identifier and Windows AppId unchanged.
- Native Windows installer execution and Linux runtime checks are provided by the existing Release
  pull-request workflow. A native legacy Windows upgrade was not run locally.

## Requirements

- R1: public org display name and profile verified live; website deployment succeeded and was checked
  in Chrome. Cosmetic repo commit 2bedea2, profile commit e029dea and website commit eb7fc19 use Device.
  Both the README hero and social-preview artwork were rendered from the project's HTML and visually
  inspected. The repository's configured social preview was uploaded and verified in GitHub.
- R2: fixed display literals in Slint, native menu holder, export metadata, assistant context,
  built-in vendor and platform/plugin host descriptions use Device. The full app self-test passed.
- R3, R7: macOS bundle and Linux staging fixtures passed; all release workflow paths match their
  producer scripts. Windows keeps AppId, reuses previous application directories and explicitly changes
  the Start Menu group; only legacy default shortcut files and their empty folder are removed.
- R4–R6: stable format/schema, ids, parameters, preference/media/project locations, executable,
  crate names, namespace/options, private window classes and installation identities are retained.
- R8: current prose and skill descriptions use Device. Historical progress entries, completed
  historical specs and ADRs retain their original wording.
- R9: contribution text changed only the product name. No security settings, licensing, permissions
  or feature capabilities were changed.

## Visual evidence

Both app images use `reference-default`, `--no-audio --offscreen`, the same default zoom and a
2880×1640 output. The before image was built from base commit 2bedea2; the after image was built
from this branch. The macOS native app menu/window title are outside the app's screenshot capture,
so these images confirm the workstation body remains stable; title/menu changes are verified in
the Slint/native-menu diff and bundle plist checks.

Media commit: 02610c633ffce8128df7a0f5b852058df5dc0cad on the append-only pr-screenshots branch.

## Principles

No DSP, audio callback, threading, rendering algorithm, command, project data or ABI layout changes.
No performance benchmark is required for display-literal and installer-label changes. No new
third-party artwork or dependencies. Internal naming and project compatibility principles are retained.

## Installer reference

Inno Setup preserves an existing directory through its unchanged AppId and default
[UsePreviousAppDir](https://jrsoftware.org/ishelp/topic_setup_usepreviousappdir.htm).
[UsePreviousGroup](https://jrsoftware.org/ishelp/topic_setup_usepreviousgroup.htm) is explicitly
disabled so the public Start Menu folder gets the new Device name.

