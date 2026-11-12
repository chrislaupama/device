---
name: dstudio-engine-abi
description: Change the boundary between Device's Rust code and its C++ engine (engine/include/dstudio_engine.h, crates/dstudio-engine) or anything that runs on the audio thread - real-time rules, how data crosses threads, layout tests, allocation guards. Use for any engine feature, new se_* function or struct, or audio-callback change.
---

# The C ABI and the audio thread

The engine is a C++23 static library. Rust reaches it only through the C header
`engine/include/dstudio_engine.h`, mirrored by hand in `crates/dstudio-engine/src/ffi.rs` (no
bindgen, so no libclang build dependency) and wrapped safely in `crates/dstudio-engine/src/lib.rs`.
Read `docs/architecture.md` › Threading and real-time rules, and `engine/README.md` for the engine's
layout and options.

## Real-time rules

The audio callback, and everything it calls, may not allocate, free, lock, wait, block, do I/O, log,
throw, or call into Rust. That includes hidden allocations: `std::vector` growth, `std::function`
captures, `std::string`, `shared_ptr` copies, `std::map` inserts, exceptions.

How data gets to and from the callback:

| Need | Mechanism |
|---|---|
| Project structure, clips, devices, lanes | a `SessionSnapshot` published with `se_session_publish`; the engine builds an immutable render graph off the audio thread and swaps it in atomically; retired graphs are freed off the audio thread |
| Transport (play, stop, locate) | the bounded lock-free command queue, drained at block start |
| Live parameter moves during a drag | lock-free setters such as `se_builtin_set_param`, applied at the next block |
| Meters, position, stats | atomics written by the callback, polled by the UI each frame |
| Larger frames (spectrum, pitch trace) | `AnalysisChannel`, a lock-free triple buffer |

Allocation is fine in `prepare` and on worker threads (decode, graph build, scan). Pre-size in
`prepare`; the callback only uses what exists.

## Adding to the ABI

1. **Header.** Add the function or struct to `dstudio_engine.h` with the `se_` prefix, plain C
   types and fixed-width integers, a doc comment saying which thread may call it, and error results
   as the existing `SE_ERR_*` codes. Prefer appending a new function to changing an existing
   signature. Read the header's notes on versioning before touching a struct that already ships.
2. **Implementation.** `engine/src/c_api.cpp` is the C entry layer; keep C++ types and exceptions
   on the engine side of it.
3. **Rust mirror.** Add the declaration to `ffi.rs` with `#[repr(C)]` structs field for field.
   Extend the `abi_layout` tests at the bottom of `ffi.rs` with the size, alignment and field
   offsets of every new struct; they are what catches a mismatch on another compiler.
4. **Safe wrapper.** Add a method on `Engine` in `lib.rs` that owns the unsafety, checks results and
   returns `Result<_, EngineError>`.
5. **Tests.** An engine test in `engine/tests/` for the behaviour, and an end-to-end test in
   `crates/dstudio-engine/tests/` when the Rust side matters.

## Proving real-time safety

- Engine tests can wrap the render path in `RtAllocGuard` (`engine/tests/test_util.hpp`) and assert
  `rt_alloc_count()` did not move; `engine/tests/test_rt_safety.cpp` shows the pattern. Any new
  code on the callback path gets covered this way.
- Run the sanitizer build when you touch memory handling:
  `cmake --preset asan && cmake --build --preset asan && ctest --preset asan` (from `engine/`).

## Cross-platform

Platform code stays in `engine/src/platform/` (Core Audio, WASAPI, PipeWire, the OS layer, all
UTF-8 at the interface). The engine builds with warnings as errors on Apple clang, MSVC and GCC, and
they disagree: watch for narrowing, unused results, sign comparison, missing `override`, and two
classes with the same name in one namespace (an ODR violation that only shows as heap corruption).
If you can only build one compiler locally, read CI's result for the other two before calling it
done.

## Performance

Anything on the render path needs before and after numbers: see `dstudio-bench`.
