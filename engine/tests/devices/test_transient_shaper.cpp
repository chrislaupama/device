// Transient Shaper (engine/src/devices/transient_shaper.cpp): attack and
// sustain shaping, level independence, neutral settings, clip, readouts,
// scope columns.
#include "device_harness.hpp"
#include "devices/dsp_dynamics.hpp"

using namespace devtest;

namespace {

// Defaults of dstudio_core::builtin::transient_shaper, registry order: attack,
// sustain, sensitivity, output_gain, mix, attack_time, sustain_time, clip.
const std::vector<float> kDefaults = {0.f, 0.f, 50.f, 0.f, 100.f, 30.f, 2000.f, 0.f};
constexpr double kRate = 48000.0;
constexpr uint32_t kLatency = 32;

DeviceRig rig() { return DeviceRig("transient-shaper", kDefaults, kRate, 256); }

// Decaying 200 Hz hits every 250 ms (a drum-like test signal).
Signal hits(std::size_t n, double amp) {
    Signal s(n);
    for (std::size_t i = 0; i < n; ++i) {
        const double t = static_cast<double>(i % 12000) / kRate;
        s[i] = static_cast<float>(amp * std::exp(-t * 18.0) * std::sin(2 * dstudio::dsp::kPi * 200.0 * t));
    }
    return s;
}
// Output / input level (dB) over a window of the last hit, latency aligned.
double win_db(const Signal& y, const Signal& x, std::size_t from, std::size_t to) {
    const std::size_t base = 36000;
    return db(rms(y, base + from + kLatency, base + to + kLatency) / rms(x, base + from, base + to));
}

} // namespace

TEST_CASE(transient_shaper_neutral_is_a_pure_delay) {
    auto r = rig();
    CHECK_EQ(r.instance().latency(), kLatency);
    const Signal x = hits(48000, 0.5);
    const auto y = r.run_mono(x);
    double err = 0.0;
    for (std::size_t i = 4800; i < x.size(); ++i) err = std::max(err, static_cast<double>(std::fabs(y[i] - x[i - kLatency])));
    CHECK(err < 2e-3);
}

TEST_CASE(transient_shaper_attack_shapes_the_start_of_each_hit) {
    const Signal x = hits(48000, 0.5);
    auto up = rig();
    up.set("attack", 100.f);
    auto down = rig();
    down.set("attack", -100.f);
    const double boost = win_db(up.run_mono(x), x, 0, 480), cut = win_db(down.run_mono(x), x, 0, 480);
    CHECK(boost > 4.0);
    CHECK(cut < -4.0);
    // The tail (after Attack Time) is left alone.
    auto tail = rig();
    tail.set("attack", 100.f);
    CHECK_NEAR(win_db(tail.run_mono(x), x, 7200, 9600), 0.0, 1.0);
}

TEST_CASE(transient_shaper_sustain_shapes_the_tail) {
    const Signal x = hits(48000, 0.5);
    auto longer = rig();
    longer.set("sustain", 100.f);
    auto shorter = rig();
    shorter.set("sustain", -100.f);
    CHECK(win_db(longer.run_mono(x), x, 4800, 9600) > 6.0);
    CHECK(win_db(shorter.run_mono(x), x, 4800, 9600) < -6.0);
    // Sustain Time limits how long the change lasts.
    auto brief = rig();
    brief.set("sustain", 100.f);
    brief.set("sustain_time", 20.f);
    CHECK(win_db(brief.run_mono(x), x, 4800, 9600) < 3.0);
}

TEST_CASE(transient_shaper_works_at_any_level) {
    const auto boost = [](double amp) {
        auto r = rig();
        r.set("attack", 60.f);
        const Signal x = hits(48000, amp);
        return win_db(r.run_mono(x), x, 0, 480);
    };
    CHECK_NEAR(boost(0.5), boost(0.05), 0.5);
}

TEST_CASE(transient_shaper_clip_holds_0_dbfs) {
    auto r = rig();
    r.set("clip", 1.f);
    r.set("output_gain", 12.f);
    const auto y = r.run_mono(tone(24000, 300.0, kRate, 0.9));
    CHECK(peak(y, 4800) <= 1.01);
    CHECK(peak(y, 4800) > 0.95);
    const auto ro = r.readouts();
    REQUIRE(ro.size() == 3);
    CHECK(ro[1] > 3.0f); // pre-clip peak (+12 dB on 0.9)
    CHECK(ro[0] <= 1.01f);
    // Below the knee the clipper is transparent.
    auto q = rig();
    q.set("clip", 1.f);
    const Signal x = tone(9600, 300.0, kRate, 0.5);
    const auto z = q.run_mono(x);
    double err = 0.0;
    for (std::size_t i = 4800; i < x.size(); ++i) err = std::max(err, static_cast<double>(std::fabs(z[i] - x[i - kLatency])));
    CHECK(err < 2e-3);
}

TEST_CASE(transient_shaper_reports_the_applied_gain) {
    auto r = rig();
    r.set("attack", -100.f);
    const Signal x = hits(12000, 0.5);
    r.run_mono(Signal(x.begin(), x.begin() + 1024));
    CHECK(r.readouts()[2] < -3.0f);
    auto s = rig();
    s.set("sustain", 100.f);
    s.run_mono(Signal(x.begin(), x.begin() + 9000));
    CHECK(s.readouts()[2] > 3.0f);
}

TEST_CASE(transient_shaper_scope_columns_at_750_per_second) {
    auto r = rig();
    r.set("clip", 1.f);
    r.set("output_gain", 12.f);
    r.run_mono(tone(48000, 300.0, kRate, 0.9));
    const auto a = r.analysis();
    REQUIRE(a.size() == 2 + 3 * 256);
    CHECK_EQ(static_cast<uint64_t>(a[0]) << 20 | static_cast<uint64_t>(a[1]), uint64_t{750});
    // The newest column: output held near 0 dBFS, pre-clip about +12 dB on 0.9.
    const float* c = a.data() + a.size() - 3;
    CHECK(c[0] > 0.95f && c[0] <= 1.01f);
    CHECK(c[1] > 3.0f);
    CHECK_NEAR(c[2], 0.0f, 0.05f);
    // 44.1 kHz: still 750 columns a second (58.8 samples each).
    DeviceRig s("transient-shaper", kDefaults, 44100.0, 256);
    s.run_mono(Signal(44100, 0.0f));
    const auto b = s.analysis();
    REQUIRE(b.size() >= 2);
    CHECK_EQ(static_cast<uint64_t>(b[0]) << 20 | static_cast<uint64_t>(b[1]), uint64_t{750});
}
