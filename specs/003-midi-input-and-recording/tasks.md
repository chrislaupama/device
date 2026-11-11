# Tasks: MIDI input and recording

Plan: [plan.md](plan.md)

- [ ] T1. Engine: `MidiInputHub`, platform backends (Core MIDI, WinMM, ALSA, stub), CMake — verify: builds on all three CI platforms
- [ ] T2. Engine: routing in `render_block`, `TrackMidi`, `LiveSynth`, held-note release — verify: A2, A3
- [ ] T3. Engine: capture ring, latency compensation and loop fold — verify: A5
- [ ] T4. C ABI + Rust wrapper + layout tests — verify: `cargo test -p dstudio-engine`
- [ ] T5. Bench: MIDI input case in `dstudio_engine_bench`, numbers before and after — verify: A8
- [ ] T6. Core: arm/monitor on instrument tracks, `midi_record` — verify: A4
- [ ] T7. App: R/I, recording pass, live clip, focus, Settings › MIDI, preferences — verify: A6, A7
- [ ] T8. Docs: README, PROGRESS.md, architecture, engine README, building — verify: read them against the change
- [ ] T9. Evidence: screenshots and bench numbers in the pull requests; manual Core MIDI check on the Mac mini — verify: `spec-verify`
