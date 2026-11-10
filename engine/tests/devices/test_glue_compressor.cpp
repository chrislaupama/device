// Glue Compressor (engine/src/devices/glue_compressor.cpp): stepped curve
// with its threshold offset and knee, range, attack / release steps, Auto
// release, color, detector high-pass, sidechain, mix and latency.
#include "device_harness.hpp"
#include "devices/dsp_dynamics.hpp"

using namespace devtest;

namespace {

// Defaults of dstudio_core::builtin::glue_compressor, registry order: threshold,
// ratio, attack, release, makeup, mix, color, range, sc_hpf, sidechain, output_gain.
const std::vector<float> kDefaults = {-15.f, 1.f, 5.f, 4.f, 0.f, 100.f, 50.f, 24.f, 0.f, 0.f, 0.f};
constexpr double kRate = 48000.0;
constexpr uint32_t kLatency = 32;

DeviceRig rig() {
    DeviceRig r("glue-compressor", kDefaults, kRate, 256);
    r.set("color", 0.f); // exact levels: no saturation
    r.set("attack", 0.f);
    return r;
}
double settled_db(const Signal& y, float in) { return db(std::fabs(y.back()) / in); }

} // namespace

TEST_CASE(glue_static_curve_uses_the_stepped_ratio_and_offset_threshold) {
    const float ratios[] = {2.f, 4.f, 10.f};
    for (int i = 0; i < 3; ++i) {
        auto r = rig();
        r.set("ratio", static_cast<float>(i));
        r.set("release", 0.f);
        const double t = -15.0 - 3.0 * (10.0 - ratios[i]) / 8.0;
        const double expect = (db(0.5) - t) * (1.0 / ratios[i] - 1.0);
        CHECK_NEAR(settled_db(r.run_mono(Signal(48000, 0.5f)), 0.5f), expect, 0.02);
    }
    // Inside the 4 dB knee: quadratic. At the (offset) threshold: slope * 2^2 / 8.
    auto k = rig();
    const float at = static_cast<float>(std::pow(10.0, (-15.0 - 2.25) / 20.0));
    CHECK_NEAR(settled_db(k.run_mono(Signal(96000, at)), at), -0.75 * 0.5, 0.02);
}

TEST_CASE(glue_range_limits_the_reduction) {
    auto r = rig();
    r.set("ratio", 2.f);
    r.set("threshold", -40.f);
    r.set("range", 6.f);
    CHECK_NEAR(settled_db(r.run_mono(Signal(48000, 0.5f)), 0.5f), -6.0, 0.02);
}

TEST_CASE(glue_attack_steps_and_auto_release) {
    const auto time_to = [](float attack) {
        auto r = rig();
        r.set("attack", attack);
        r.set("ratio", 2.f);
        r.run_mono(Signal(4800, 0.001f));
        const auto y = r.run_mono(Signal(48000, 0.5f));
        const double target = settled_db(y, 0.5f);
        for (std::size_t i = kLatency; i < y.size(); ++i)
            if (db(std::fabs(y[i]) / 0.5) <= 0.632 * target) return i - kLatency;
        return y.size();
    };
    CHECK(time_to(0.f) < 48);                                          // 0.1 ms
    CHECK_NEAR(static_cast<double>(time_to(5.f)), 0.030 * kRate, 150.0); // 30 ms
    // Auto: a short burst recovers faster than a long squeeze.
    const auto recovery = [](std::size_t burst) {
        auto r = rig();
        r.set("ratio", 2.f);
        r.set("threshold", -30.f);
        r.set("release", 4.f);
        r.run_mono(Signal(4800, 0.001f));
        r.run_mono(Signal(burst, 0.5f));
        const auto y = r.run_mono(Signal(192000, 0.001f));
        for (std::size_t i = kLatency; i < y.size(); ++i)
            if (db(std::fabs(y[i]) / 0.001) >= -1.0) return i;
        return y.size();
    };
    CHECK(recovery(480) * 2 < recovery(96000));
}

TEST_CASE(glue_color_adds_harmonics_at_unity_level) {
    const auto harmonic_ratio = [](float color, double* level) {
        auto r = rig();
        r.set("threshold", 0.f); // no compression
        r.set("color", color);
        const Signal x = tone(48000, 1000.0, kRate, 0.25);
        const auto y = r.run_mono(x);
        // 3rd-harmonic and fundamental magnitudes by correlation over the last 0.5 s.
        auto mag = [&](double hz) {
            double re = 0, im = 0;
            for (std::size_t i = 24000; i < y.size(); ++i) {
                re += y[i] * std::cos(2 * dstudio::dsp::kPi * hz * static_cast<double>(i) / kRate);
                im += y[i] * std::sin(2 * dstudio::dsp::kPi * hz * static_cast<double>(i) / kRate);
            }
            return std::sqrt(re * re + im * im);
        };
        *level = db(mag(1000.0) / 12000.0 / 0.25);
        return db((mag(3000.0) + mag(2000.0)) / mag(1000.0));
    };
    double l0 = 0, l1 = 0;
    const double clean = harmonic_ratio(0.f, &l0), warm = harmonic_ratio(100.f, &l1);
    CHECK(clean < -90.0);
    CHECK(warm > -50.0);
    CHECK_NEAR(l0, 0.0, 0.05);
    CHECK_NEAR(l1, 0.0, 1.0); // level compensated drive
}

TEST_CASE(glue_detector_hpf_and_sidechain) {
    const Signal bass = tone(48000, 40.0, kRate, 0.5);
    auto off = rig();
    off.set("threshold", -30.f);
    auto filtered = rig();
    filtered.set("threshold", -30.f);
    filtered.set("sc_hpf", 4.f); // 300 Hz
    CHECK(peak(filtered.run_mono(bass), 24000) > 2.0 * peak(off.run_mono(bass), 24000));
    const Signal main(48000, 0.01f), side(48000, 0.5f);
    auto keyed = rig();
    keyed.set("sidechain", 1.f);
    // The key is 11.25 dB over the offset threshold (-17.25) at 4:1.
    CHECK_NEAR(settled_db(keyed.run(main, main, &side, &side).first, 0.01f), (db(0.5) + 17.25) * -0.75, 0.05);
    auto own = rig();
    CHECK_NEAR(settled_db(own.run(main, main, &side, &side).first, 0.01f), 0.0, 1e-3);
}

TEST_CASE(glue_latency_keeps_dry_and_wet_aligned) {
    auto r = rig();
    CHECK_EQ(r.instance().latency(), kLatency);
    r.set("threshold", 0.f);
    r.set("mix", 50.f);
    r.set("makeup", 0.f);
    const Signal x = tone(9600, 15000.0, kRate, 0.25); // a comb would cancel here
    const auto y = r.run_mono(x);
    double err = 0.0;
    for (std::size_t i = 4800; i < x.size(); ++i) err = std::max(err, static_cast<double>(std::fabs(y[i] - x[i - kLatency])));
    CHECK(err < 1e-3);
}
