// Compressor (engine/src/devices/compressor.cpp): static curve, knee, ratio,
// timing, characters, gains, mix, sidechain, readouts and automation
// splitting, through the shared device harness.
#include "device_harness.hpp"

using namespace devtest;

namespace {

// Defaults of saine_core::builtin::compressor, registry order.
const std::vector<float> kDefaults = {-24.f, 4.f, 10.f, 100.f, 6.f, 0.f, 100.f, 0.f, 0.f, 0.f, 0.f};
constexpr double kRate = 48000.0;

DeviceRig rig() { return DeviceRig("compressor", kDefaults, kRate, 256); }
Signal dc(std::size_t n, float v) { return Signal(n, v); }
// Gain in dB of the last sample (DC in, steady state).
double settled_gain_db(const Signal& out, float in) { return db(std::fabs(out.back()) / in); }

} // namespace

TEST_CASE(compressor_below_threshold_is_bit_transparent) {
    auto r = rig();
    const Signal x = tone(9600, 440.0, kRate, 0.01); // -40 dBFS peak, threshold -24, knee 6
    auto [l, rr] = r.run(x, x);
    for (std::size_t i = 0; i < x.size(); ++i) {
        if (l[i] != x[i]) {
            CHECK(l[i] == x[i]);
            break;
        }
    }
}

TEST_CASE(compressor_static_curve_matches_ratio_and_threshold) {
    for (float ratio : {2.f, 4.f, 10.f}) {
        auto r = rig();
        r.set("knee", 0.f);
        r.set("ratio", ratio);
        r.set("attack", 1.f);
        const auto out = r.run_mono(dc(48000, 0.5f));
        const double in_db = db(0.5);
        const double expect = (in_db - -24.0) * (1.0 / ratio - 1.0);
        CHECK_NEAR(settled_gain_db(out, 0.5f), expect, 0.02);
    }
}

TEST_CASE(compressor_soft_knee_is_quadratic_around_threshold) {
    auto r = rig();
    r.set("knee", 12.f);
    r.set("ratio", 4.f);
    r.set("attack", 1.f);
    const float at_threshold = static_cast<float>(std::pow(10.0, -24.0 / 20.0));
    // At the threshold: slope * (W/2)^2 / (2W) = -0.75 * 36 / 24 = -1.125 dB.
    CHECK_NEAR(settled_gain_db(r.run_mono(dc(48000, at_threshold)), at_threshold), -1.125, 0.02);
    // Below the knee: nothing; above it: the straight line.
    auto below = rig();
    below.set("knee", 12.f);
    const float under = static_cast<float>(std::pow(10.0, -31.0 / 20.0));
    CHECK_NEAR(settled_gain_db(below.run_mono(dc(48000, under)), under), 0.0, 1e-4);
}

TEST_CASE(compressor_ratio_one_is_transparent) {
    auto r = rig();
    r.set("ratio", 1.f);
    const Signal x = noise(9600, 0.9f, 9);
    auto [l, rr] = r.run(x, x);
    CHECK(peak(l) > 0.0);
    double err = 0;
    for (std::size_t i = 0; i < x.size(); ++i) err = std::max(err, static_cast<double>(std::fabs(l[i] - x[i])));
    CHECK(err < 1e-6);
}

TEST_CASE(compressor_makeup_output_and_mix) {
    auto r = rig();
    r.set("makeup_gain", 6.f);
    CHECK_NEAR(settled_gain_db(r.run_mono(dc(9600, 0.01f)), 0.01f), 6.0, 0.01);
    auto o = rig();
    o.set("output_gain", -6.f);
    CHECK_NEAR(settled_gain_db(o.run_mono(dc(9600, 0.01f)), 0.01f), -6.0, 0.01);
    // Mix 0: the dry signal even under heavy compression.
    auto m = rig();
    m.set("mix", 0.f);
    m.set("ratio", 20.f);
    m.set("threshold", -60.f);
    const Signal x = tone(9600, 200.0, kRate, 0.5);
    auto [l, rr] = m.run(x, x);
    double err = 0;
    for (std::size_t i = 0; i < x.size(); ++i) err = std::max(err, static_cast<double>(std::fabs(l[i] - x[i])));
    CHECK(err < 1e-6);
    // Mix 50 %: halfway between dry and wet in amplitude.
    auto h = rig();
    h.set("mix", 50.f);
    h.set("knee", 0.f);
    h.set("ratio", 20.f);
    h.set("attack", 1.f);
    const double wet = std::pow(10.0, (db(0.5) + 24.0) * (1.0 / 20.0 - 1.0) / 20.0);
    CHECK_NEAR(std::fabs(h.run_mono(dc(48000, 0.5f)).back()) / 0.5, 0.5 + 0.5 * wet, 1e-3);
    // Gain changes glide (no step): 20 ms smoothing.
    auto g = rig();
    g.run_mono(dc(4800, 0.01f));
    g.set("makeup_gain", 12.f);
    const auto after = g.run_mono(dc(19200, 0.01f));
    CHECK(after[0] < 0.0105f);
    CHECK_NEAR(after.back() / 0.01f, std::pow(10.0, 12.0 / 20.0), 0.01);
}

TEST_CASE(compressor_attack_and_release_times) {
    auto r = rig();
    r.set("knee", 0.f);
    r.set("attack", 10.f);
    r.set("release", 100.f);
    r.run_mono(dc(4800, 0.001f));
    const auto up = r.run_mono(dc(48000, 0.5f));
    const double target = (db(0.5) + 24.0) * (0.25 - 1.0);
    std::size_t hit = 0;
    for (std::size_t i = 0; i < up.size(); ++i)
        if (db(std::fabs(up[i]) / 0.5) <= 0.632 * target) {
            hit = i;
            break;
        }
    CHECK_NEAR(static_cast<double>(hit), 0.010 * kRate, 0.1 * 0.010 * kRate); // one time constant
    const auto down = r.run_mono(dc(48000, 0.001f));
    std::size_t rel = 0;
    for (std::size_t i = 0; i < down.size(); ++i)
        if (db(std::fabs(down[i]) / 0.001) >= 0.368 * target) {
            rel = i;
            break;
        }
    CHECK_NEAR(static_cast<double>(rel), 0.100 * kRate, 0.1 * 0.100 * kRate);
}

// Recovery time (to within 1 dB) after heavy compression, per character.
static std::size_t recovery_samples(int style) {
    auto r = rig();
    r.set("style", static_cast<float>(style));
    r.set("threshold", -40.f);
    r.set("ratio", 20.f);
    r.set("knee", 0.f);
    r.set("release", 100.f);
    r.run_mono(dc(48000, 0.5f));
    const auto down = r.run_mono(dc(96000, 0.001f));
    for (std::size_t i = 0; i < down.size(); ++i)
        if (db(std::fabs(down[i]) / 0.001) >= -1.0) return i;
    return down.size();
}

TEST_CASE(compressor_characters_differ_as_documented) {
    const std::size_t clean = recovery_samples(0), opto = recovery_samples(1), punch = recovery_samples(2),
                      smooth = recovery_samples(3);
    CHECK(opto > clean);   // program-dependent slow release after deep reduction
    CHECK(punch < clean);  // fast recovery
    CHECK(smooth > clean); // slower overall
    // Punch lets more of a transient through than Clean (slower attack).
    const auto first_ms_peak = [](int style) {
        auto r = rig();
        r.set("style", static_cast<float>(style));
        r.set("knee", 0.f);
        r.set("attack", 5.f);
        r.run_mono(dc(4800, 0.001f));
        return peak(r.run_mono(dc(4800, 0.5f)), 48, 240);
    };
    CHECK(first_ms_peak(2) > first_ms_peak(0));
}

TEST_CASE(compressor_sidechain_drives_the_detector) {
    const Signal main = dc(48000, 0.01f), side = dc(48000, 0.5f);
    auto off = rig();
    off.set("knee", 0.f);
    CHECK_NEAR(settled_gain_db(off.run(main, main, &side, &side).first, 0.01f), 0.0, 1e-4); // enable off: own signal
    auto on = rig();
    on.set("knee", 0.f);
    on.set("attack", 1.f);
    on.set("sidechainEnable", 1.f);
    CHECK_NEAR(settled_gain_db(on.run(main, main, &side, &side).first, 0.01f), (db(0.5) + 24.0) * -0.75, 0.02);
    auto scaled = rig();
    scaled.set("knee", 0.f);
    scaled.set("attack", 1.f);
    scaled.set("sidechainEnable", 1.f);
    scaled.set("sidechain_gain", -12.f);
    CHECK_NEAR(settled_gain_db(scaled.run(main, main, &side, &side).first, 0.01f), (db(0.5) - 12.0 + 24.0) * -0.75, 0.02);
    // Enabled but no sidechain buffer (no source picked): own signal.
    auto none = rig();
    none.set("sidechainEnable", 1.f);
    CHECK_NEAR(settled_gain_db(none.run_mono(main), 0.01f), 0.0, 1e-4);
}

TEST_CASE(compressor_readouts_report_levels_and_reduction) {
    auto r = rig();
    r.set("knee", 0.f);
    r.set("attack", 1.f);
    r.run_mono(dc(48000, 0.5f));
    const auto ro = r.readouts();
    REQUIRE(ro.size() == 3);
    CHECK_NEAR(ro[0], db(0.5), 0.01);
    CHECK_NEAR(ro[2], (db(0.5) + 24.0) * -0.75, 0.05);
    CHECK_NEAR(ro[1], ro[0] + ro[2], 0.05);
    r.run_mono(dc(48000, 0.0f));
    CHECK(r.readouts()[0] <= -119.0f);
}

TEST_CASE(compressor_automation_events_split_blocks) {
    auto r = rig();
    r.set("ratio", 1.f);
    r.set("knee", 0.f);
    r.set("attack", 0.01f);
    // Ratio 20 from frame 1000 (inside the 4th block of 256).
    const auto out = r.run(dc(2048, 0.5f), dc(2048, 0.5f), nullptr, nullptr, {{1000, {1, 20.0f}}}).first;
    CHECK(out[999] == 0.5f);
    CHECK(out[1010] < 0.1f);
}

TEST_CASE(compressor_impulse_and_reset) {
    auto r = rig();
    const auto out = r.run_mono(impulse(4800, 1.0f, 10));
    CHECK(all_finite(out));
    CHECK(peak(out) <= 1.0 + 1e-6);
    r.run_mono(dc(4800, 0.9f));
    r.reset();
    r.set("threshold", -24.f);
    // After reset no reduction is held over: a quiet signal passes unchanged.
    const auto q = r.run_mono(dc(256, 0.01f));
    CHECK(q[0] == 0.01f);
}
