// Character Mod (engine/src/devices/character_mod.cpp): every circuit and
// mode stays finite and bounded, Mix 0 is the dry signal, Width 0 is mono,
// Mono Audition sums to mono and the seed makes the wander repeatable.
#include "device_harness.hpp"

using namespace devtest;

namespace {

constexpr double kRate = 48000.0;
constexpr const char* kModeIds[] = {"mode_single", "mode_crossed", "mode_twin", nullptr, "mode_tape", "mode_photocell",
                                    "mode_flanger", "mode_phaser", "mode_trem", "mode_rotary", "mode_panner"};
constexpr int kModeCounts[] = {2, 4, 3, 1, 3, 2, 3, 5, 3, 3, 2};

DeviceRig rig() {
    const saine::BuiltinDescriptor* d = saine::find_builtin("character-mod");
    if (d == nullptr || d->defaults.size() != d->params.size()) {
        std::fprintf(stderr, "no character-mod defaults\n");
        std::abort();
    }
    return DeviceRig("character-mod", {d->defaults.begin(), d->defaults.end()}, kRate, 256);
}
bool identical(const Signal& a, const Signal& b) {
    for (std::size_t i = 0; i < a.size(); ++i)
        if (a[i] != b[i]) return false;
    return true;
}

} // namespace

TEST_CASE(character_mod_every_circuit_and_mode_is_bounded) {
    const Signal x = noise(48000, 0.5f, 31);
    for (int c = 0; c < 11; ++c) {
        for (int m = 0; m < kModeCounts[c]; ++m) {
            for (float depth : {50.0f, 100.0f}) {
                auto r = rig();
                r.set("model", static_cast<float>(c));
                if (kModeIds[c] != nullptr) r.set(kModeIds[c], static_cast<float>(m));
                r.set("depth", depth);
                r.set("mix", 100.0f);
                r.set("input", 2.0f);
                auto [l, rr] = r.run(x, x);
                CHECK(all_finite(l) && all_finite(rr));
                CHECK(peak(l) < 4.0 && peak(rr) < 4.0);
                CHECK(rms(l, 4800) > 0.01);
            }
        }
    }
}

TEST_CASE(character_mod_mix_zero_is_dry) {
    auto r = rig();
    r.set("mix", 0.0f);
    const Signal x = noise(9600, 0.5f, 33);
    const auto out = r.run_mono(x);
    double err = 0.0;
    for (std::size_t i = 0; i < x.size(); ++i) err = std::max(err, static_cast<double>(std::fabs(out[i] - x[i])));
    CHECK(err < 1e-4);
}

TEST_CASE(character_mod_width_zero_and_mono_audition_are_mono) {
    const Signal x = noise(24000, 0.5f, 35);
    auto w = rig();
    w.set("width", 0.0f);
    w.set("art_hiss", 0.0f);
    w.set("drift", 0.0f);
    auto [l0, r0] = w.run(x, x);
    double err = 0.0;
    for (std::size_t i = 0; i < x.size(); ++i) err = std::max(err, static_cast<double>(std::fabs(l0[i] - r0[i])));
    CHECK(err < 1e-4);
    auto m = rig();
    m.set("mono_audition", 1.0f);
    auto [l1, r1] = m.run(x, noise(24000, 0.5f, 36));
    CHECK(identical(l1, r1));
}

TEST_CASE(character_mod_seed_makes_the_wander_repeatable) {
    const Signal x = noise(24000, 0.5f, 37);
    auto a = rig();
    auto b = rig();
    auto c = rig();
    c.set("seed", 77.0f);
    const auto oa = a.run_mono(x), ob = b.run_mono(x), oc = c.run_mono(x);
    CHECK(identical(oa, ob));
    CHECK(!identical(oa, oc));
}

TEST_CASE(character_mod_rotary_stop_settles) {
    auto r = rig();
    r.set("model", 9.0f);
    r.set("mode_rotary", 2.0f);
    r.set("art_hiss", 0.0f);
    r.set("drift", 0.0f);
    const auto out = r.run_mono(Signal(48000 * 8, 0.5f));
    CHECK(all_finite(out));
    // Rotors stopped: the output of a steady input is steady.
    double lo = 1e9, hi = -1e9;
    for (std::size_t i = out.size() - 4800; i < out.size(); ++i) {
        lo = std::min(lo, static_cast<double>(out[i]));
        hi = std::max(hi, static_cast<double>(out[i]));
    }
    CHECK(hi - lo < 0.01);
}
