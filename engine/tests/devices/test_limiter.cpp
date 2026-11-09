// Limiter (engine/src/devices/limiter.cpp): ceiling, true peak, lookahead,
// release, fixed latency, gains and readouts, through the shared harness.
#include "device_harness.hpp"
#include "devices/dsp_dynamics.hpp"

using namespace devtest;

namespace {

// Defaults of saine_core::builtin::limiter, registry order:
// ceiling, release, lookahead, true_peak, mix, input_gain, output_gain.
const std::vector<float> kDefaults = {-0.3f, 100.f, 5.f, 1.f, 100.f, 0.f, 0.f};
constexpr double kRate = 48000.0;

DeviceRig rig() { return DeviceRig("limiter", kDefaults, kRate, 256); }

double true_peak(const Signal& s, std::size_t from = 0) {
    saine::dsp::TruePeak4x tp;
    double m = 0.0;
    for (std::size_t i = 0; i < s.size(); ++i) {
        const double v = tp.next(s[i]);
        if (i >= from + saine::dsp::TruePeak4x::kDelay) m = std::max(m, v);
    }
    return m;
}

} // namespace

TEST_CASE(true_peak_never_reads_below_the_sample_peak) {
    saine::dsp::TruePeak4x tp;
    float m = 0.0f;
    for (int i = 0; i < 32; ++i) m = std::max(m, tp.next(i == 3 ? 1.0f : 0.0f));
    CHECK(m >= 1.0f);
}

TEST_CASE(limiter_reports_a_fixed_latency_and_passes_quiet_audio_unchanged) {
    auto r = rig();
    const uint32_t lat = r.instance().latency();
    CHECK_EQ(lat, 960u + saine::dsp::TruePeak4x::kDelay); // 20 ms at 48 kHz + the true-peak filter
    r.set("lookahead", 0.f);
    CHECK_EQ(r.instance().latency(), lat); // lookahead never changes PDC
    const Signal x = tone(9600, 1000.0, kRate, 0.3);
    const auto y = r.run_mono(x);
    double err = 0.0;
    for (std::size_t i = lat; i < x.size(); ++i) err = std::max(err, static_cast<double>(std::fabs(y[i] - x[i - lat])));
    CHECK(err < 1e-6);
}

TEST_CASE(limiter_holds_the_ceiling_on_samples_and_between_them) {
    for (float tp : {0.f, 1.f}) {
        auto r = rig();
        r.set("true_peak", tp);
        r.set("input_gain", 18.f);
        r.set("ceiling", -1.f);
        const Signal x = concat(noise(24000, 0.8f, 3), tone(24000, 11025.0, kRate, 0.9));
        const auto y = r.run_mono(x);
        CHECK(all_finite(y));
        CHECK(peak(y) <= std::pow(10.0, -1.0 / 20.0) + 1e-6);
        if (tp > 0.5f) CHECK(db(true_peak(y)) <= -1.0 + 0.25); // inter-sample overs held within 0.25 dB
    }
    // Sample-peak mode lets inter-sample peaks through (fs/4 sine at 45 degrees).
    auto s = rig();
    s.set("true_peak", 0.f);
    s.set("ceiling", -1.f);
    Signal x(24000);
    for (std::size_t i = 0; i < x.size(); ++i) x[i] = static_cast<float>(1.2 * std::sin(saine::dsp::kPi * 0.5 * static_cast<double>(i) + saine::dsp::kPi / 4));
    CHECK(db(true_peak(s.run_mono(x), 4800)) > -0.5);
}

TEST_CASE(limiter_lookahead_fades_in_before_the_peak) {
    auto r = rig();
    r.set("ceiling", -6.f);
    r.set("true_peak", 0.f); // a step overshoots between samples; measure the sample-peak fade
    const uint32_t lat = r.instance().latency();
    Signal x(9600, 0.1f);
    for (std::size_t i = 4800; i < x.size(); ++i) x[i] = 1.0f; // a step 14 dB over the ceiling
    const auto y = r.run_mono(x);
    const double c = std::pow(10.0, -6.0 / 20.0);
    // At the step the gain has already reached the ceiling: no sample over it, no clamp needed.
    CHECK(std::fabs(y[4800 + lat]) <= c + 1e-4);
    CHECK_NEAR(std::fabs(y[4800 + lat]), c, 0.01);
    // 2.5 ms before it the fade is under way, 6 ms before it nothing has happened yet.
    CHECK(y[4800 + lat - 120] < 0.1f * 0.9f);
    CHECK_NEAR(y[4800 + lat - 288], 0.1f, 1e-6);
}

TEST_CASE(limiter_release_sets_the_recovery_time) {
    const auto recovery = [](float release) {
        auto r = rig();
        r.set("release", release);
        r.set("ceiling", -12.f);
        const uint32_t lat = r.instance().latency();
        Signal x = concat(Signal(4800, 0.9f), Signal(96000, 0.05f));
        const auto y = r.run_mono(x);
        for (std::size_t i = 4800 + lat; i < y.size(); ++i)
            if (db(std::fabs(y[i]) / 0.05) >= -1.0) return i - 4800 - lat;
        return y.size();
    };
    const std::size_t fast = recovery(20.f), slow = recovery(500.f);
    CHECK(fast < slow);
    CHECK(static_cast<double>(slow) > 0.4 * kRate);
    CHECK(static_cast<double>(fast) < 0.1 * kRate);
}

TEST_CASE(limiter_gains_mix_and_readouts) {
    auto r = rig();
    r.set("input_gain", 12.f);
    r.set("ceiling", -3.f);
    const auto y = r.run_mono(Signal(48000, 0.5f));
    CHECK_NEAR(db(y.back()), -3.0, 0.01);
    const auto ro = r.readouts();
    REQUIRE(ro.size() == 3);
    CHECK_NEAR(ro[0], db(0.5) + 12.0, 0.01);   // input after the input gain
    CHECK_NEAR(ro[2], -3.0 - (db(0.5) + 12.0), 0.05); // reduction to the ceiling
    CHECK_NEAR(ro[1], -3.0, 0.05);
    auto o = rig();
    o.set("output_gain", -6.f);
    CHECK_NEAR(db(o.run_mono(Signal(48000, 0.1f)).back() / 0.1), -6.0, 0.01);
    auto m = rig();
    m.set("mix", 0.f);
    m.set("input_gain", 12.f);
    CHECK_NEAR(m.run_mono(Signal(48000, 0.5f)).back(), 0.5f * std::pow(10.0, 12.0 / 20.0), 1e-3); // dry (after the input gain)
}
