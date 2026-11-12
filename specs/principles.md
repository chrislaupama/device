# Principles

The rules every change to Device follows, from people and agents alike. Each one points at
the document that owns it, so this file stays short and the owning document stays the authority.
A spec's plan checks itself against this list (`plan.md` › Principles check).

## 1. Real-time safety

The audio callback never allocates, locks, blocks, does I/O, logs or calls into Rust. Data reaches
it as immutable session snapshots swapped in with an atomic exchange, through the bounded lock-free
transport queue, or as lock-free parameter writes; it reports back through atomics and triple
buffers. `engine/tests/test_rt_safety.cpp` and `every_builtin_is_rt_safe_and_robust` enforce it.
Source: [docs/architecture.md](../docs/architecture.md) › Threading and real-time rules.

## 2. One mutation path

Every edit to a project is a `dstudio_core::Command` executed on the `Document`: validated before it
changes anything, returning its exact inverse, coalescing continuous gestures, labelled for the
Undo menu, and serialisable as JSON. The UI holds no authoritative state. Source:
[docs/architecture.md](../docs/architecture.md) › Command system.

## 3. Projects keep loading

Saved projects are versioned (`persist::SCHEMA_VERSION`) and migrated step by step; a newer file is
refused, never misread. Built-in device parameters are stored by id and are append-only: never
reorder, rename or remove a shipped parameter. Source:
[docs/architecture.md](../docs/architecture.md) › Persistence, [docs/devices.md](../docs/devices.md) › Adding a device.

## 4. The C ABI is the only boundary

Rust and C++ meet only at `engine/include/dstudio_engine.h`: plain C types, no exceptions across it.
The Rust side is hand-written in `crates/dstudio-engine/src/ffi.rs` and its layout tests must match
the header. Source: [docs/architecture.md](../docs/architecture.md) › Technology adjustments.

## 5. Cross-platform from the first commit

macOS, Windows and Linux build from the same code. Platform code stays in `engine/src/platform/`
and behind `#[cfg]` in Rust. The engine builds with warnings as errors on clang, MSVC and GCC, and
CI runs every platform. Source: [docs/architecture.md](../docs/architecture.md) › Cross-platform strategy.

## 6. Original work only

No code, artwork, icons, fonts, presets, samples or prose from other products. Facts (a range, a
default, a measurement) can inform a design; the expression is always our own. New third-party
code is listed with its licence in [engine/third_party_licenses.md](../engine/third_party_licenses.md).

## 7. Evidence in every pull request

A visible change carries before and after screenshots. A change that could affect performance
carries before and after benchmark numbers, and extends the benchmarks when none cover it. A fixed
bug gets a few small tests that would have caught it, not a large number of shallow ones. See the
`dstudio-pr` and `dstudio-bench` skills.

## 8. Docs move with the code

A change that adds, finishes or removes something the README, PROGRESS.md or `docs/` describe
updates them in the same pull request. Source: [README.md](../README.md) › Keeping this README up to date.

## 9. Naming

Internal names use `dstudio` (crates, the `dstudio::` namespace, `DSTUDIO_*` options). Bare
`device` is never an identifier prefix: it already names effects and instruments.

## 10. Rules are not bent to fit a feature

If a principle seems to stand in the way, the design changes or the question goes to the
maintainer. A principle is not relaxed "for now" with a comment explaining why.

Changing this file needs a pull request that changes the owning document first.
