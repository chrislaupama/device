// Tuner (engine/src/devices/tuner.cpp): note and cents readouts, the A4
// reference, string tunings, hold, and the pass-through / mute output.
#include "device_harness.hpp"

using namespace devtest;
using namespace testutil;

namespace {

// Defaults of dstudio_core::builtin::tuner, registry order: reference, tuning, mute_output.
const std::vector<float> kDefaults = {0.f, 0.f, 0.f};
constexpr double kRate = 48000.0;
enum { kVoiced, kNote, kCents, kInTune, kString, kHolding };

DeviceRig rig() { return DeviceRig("tuner", kDefaults, kRate, 256); }

std::vector<float> read(DeviceRig& r, double hz, double seconds = 0.5) {
    r.run_mono(tone(static_cast<std::size_t>(seconds * kRate), hz, kRate, 0.3));
    return r.readouts();
}

} // namespace

TEST_CASE(tuner_names_notes_and_cents) {
    auto r = rig();
    auto ro = read(r, 440.0);
    CHECK(ro[kVoiced] == 1.0f);
    CHECK(ro[kNote] == 69.0f);
    CHECK(std::fabs(ro[kCents]) < 0.5f);
    CHECK(ro[kInTune] == 1.0f);
    CHECK(ro[kString] == -1.0f);
    ro = read(r, 445.0);
    CHECK(ro[kNote] == 69.0f);
    CHECK_NEAR(ro[kCents], 1200.0 * std::log2(445.0 / 440.0), 0.5);
    CHECK(ro[kInTune] == 0.0f);
    ro = read(r, 41.2034); // E1: the lowest bass string
    CHECK(ro[kNote] == 28.0f);
    CHECK(std::fabs(ro[kCents]) < 1.0f);
    ro = read(r, 1318.51); // E6
    CHECK(ro[kNote] == 88.0f);
    CHECK(std::fabs(ro[kCents]) < 1.0f);
}

TEST_CASE(tuner_reference_432) {
    auto r = rig();
    r.set("reference", 1.f);
    auto ro = read(r, 432.0);
    CHECK(ro[kNote] == 69.0f);
    CHECK(std::fabs(ro[kCents]) < 0.5f);
    ro = read(r, 440.0);
    CHECK_NEAR(ro[kCents], 1200.0 * std::log2(440.0 / 432.0), 0.5);
}

TEST_CASE(tuner_string_tunings_pick_the_nearest_string) {
    auto r = rig();
    r.set("tuning", 1.f); // Standard
    auto ro = read(r, 110.0 * std::pow(2.0, 0.31 / 12.0)); // A2 + 31 ct
    CHECK(ro[kString] == 1.0f);
    CHECK(ro[kNote] == 45.0f);
    CHECK_NEAR(ro[kCents], 31.0, 1.0);
    ro = read(r, 98.0); // G2: between E2 and A2, nearer A2 in pitch (2 st vs 3)
    CHECK(ro[kString] == 1.0f);
    CHECK(ro[kCents] == -50.0f); // clamped
    r.set("tuning", 2.f); // Drop D
    ro = read(r, 73.4162);
    CHECK(ro[kString] == 0.0f);
    CHECK(ro[kNote] == 38.0f);
    CHECK(std::fabs(ro[kCents]) < 1.0f);
    r.set("tuning", 3.f); // DADGAD: B3 string is A3
    ro = read(r, 220.0);
    CHECK(ro[kString] == 4.0f);
    CHECK(ro[kNote] == 57.0f);
    r.set("tuning", 4.f); // Sweetened: B3 2 ct flat
    ro = read(r, 246.9417 * std::pow(2.0, -0.02 / 12.0));
    CHECK(ro[kString] == 4.0f);
    CHECK(std::fabs(ro[kCents]) < 0.5f);
}

TEST_CASE(tuner_holds_then_clears) {
    auto r = rig();
    read(r, 330.0);
    r.run_mono(Signal(24000, 0.0f)); // 0.5 s of silence
    auto ro = r.readouts();
    CHECK(ro[kVoiced] == 1.0f);
    CHECK(ro[kHolding] == 1.0f);
    CHECK(ro[kNote] == 64.0f);
    r.run_mono(Signal(48000, 0.0f));
    ro = r.readouts();
    CHECK(ro[kVoiced] == 0.0f);
    CHECK(ro[kHolding] == 0.0f);
    CHECK(ro[kString] == -1.0f);
}

TEST_CASE(tuner_passes_audio_or_mutes) {
    auto r = rig();
    const Signal x = noise(9600, 0.5f, 2);
    const auto [l, rr] = r.run(x, x);
    for (std::size_t i = 0; i < x.size(); ++i)
        if (l[i] != x[i]) {
            CHECK(l[i] == x[i]);
            break;
        }
    r.set("mute_output", 1.f);
    const Signal y = r.run_mono(x);
    CHECK(peak(y, 2400) == 0.0);
    // Detection keeps working while muted.
    auto ro = read(r, 196.0);
    CHECK(ro[kNote] == 55.0f);
}

TEST_CASE(tuner_readouts_come_from_the_last_automation_segment) {
    auto r = rig();
    r.set("tuning", 1.f); // Standard
    CHECK(read(r, 330.0)[kString] == 5.0f);
    // Automation switches to Chromatic inside the last block: the readouts are
    // one state, so the string comes from the last segment, not the first.
    const std::size_t n = 24000;
    r.run(tone(n, 330.0, kRate, 0.3), tone(n, 330.0, kRate, 0.3), nullptr, nullptr, {{n - 64, {1u, 0.f}}});
    const auto ro = r.readouts();
    CHECK(ro[kString] == -1.0f);
    CHECK(ro[kNote] == 64.0f);
}
