# Device public name

Status: Done · Spec: 004 · Issue: none

## Problem

The project is presented as Device Studio across GitHub, the website, desktop interface and release packages.
The maintainer has chosen the shorter public name Device. Mixed names make the product harder to recognize.

## Behaviour

The organization, profile, website, current repository documentation, artwork, desktop windows,
menus, update messages, plugin host labels and installer labels present the product as Device.
The waveform logo and "Built for real time." tagline stay the same.

macOS packages contain Device.app. Release downloads use Device-<version> names and release titles
say Device. Windows installation and shortcuts use Device; the fixed installer identity remains
the same so upgrades continue to target the existing installation. The Linux launcher displays Device.

Existing projects, preferences, plugin caches and configured project folders continue to work.
This naming change adds no project edits or undo entries. The project schema and identifiers do not change.

## Requirements

- R1. Public GitHub organization name, profile text, website content, metadata, and current repo
  documentation and artwork must use Device.
- R2. Desktop title, menus, About/update copy, assistant context, exported creator labels, built-in
  device vendor and OS audio/MIDI/plugin host display names must use Device.
- R3. macOS bundle display name and bundle path, Windows installer/shortcut display names, Linux
  launcher name, release titles and downloadable archive names must use Device consistently.
- R4. Existing .dsproj projects and built-in device identifiers/parameters must remain readable
  without a schema migration. Keep the device-studio-project format identifier.
- R5. Keep existing preference/cache locations, default project folder, recording/render scratch
  locations, macOS bundle identity and Windows installer AppId so current user data and permissions
  remain associated with this app. Explain retained legacy paths in the documentation.
- R6. Keep dstudio crates/namespaces/options, the device-studio executable, desktop-file identity
  and private window classes stable. GitHub addresses and repository names stay unchanged.
- R7. Packaging workflows and smoke checks must agree with renamed bundle/archive paths. Windows
  upgrades should remove old Device Studio shortcut labels while retaining the existing installation.
- R8. Preserve the original brand in historical progress entries and completed historical specs/ADRs.
  Current documentation should use Device and distinguish compatibility names where necessary.
- R9. Contribution restrictions, security settings, licensing and public feature claims must remain intact.

## Acceptance checks

- A1 (R1, R8, R9): Inspect current docs/profile/site text and regenerated hero/social artwork. Check
  the live org and deployed website, including title, logo alt text, links and contribution policy.
  Search remaining old-brand occurrences and classify each as historical or a compatibility identifier.
- A2 (R2): Build the app and render an existing review state; verify Device in the title/menu.
  Inspect exported creator and host display strings. Run the existing app self-test.
- A3 (R3, R5, R7): Exercise macOS bundle creation with controlled fixture executables; verify
  Device.app, plist display names, stable identity and bundled scanner. Check release script syntax
  and workflow/package references. Verify Linux package launcher naming and stable executable paths.
  Report Windows/Linux execution limits if those platforms are unavailable.
- A4 (R4, R5, R6): Run existing persistence/preferences tests and inspect the unchanged compatibility
  identifiers and paths. No bulk rename of stored format, crate, namespace, environment option or executable.
- A5 (R2, R4, R7): Run the repository's required build/test checks appropriate to touched code and
  report any pre-existing or platform-specific failures separately.

## Out of scope

- Renaming devicedaw, device, devicedaw.github.io or changing their URLs.
- Renaming internal dstudio identifiers or the device-studio executable.
- Moving user files, changing project format, adding migration logic or changing permissions.
- Publishing a release, paying for services, changing the logo design or rewriting historical records.

## Open questions

None. The maintainer approved the compatibility approach and authorized all remaining phases.

