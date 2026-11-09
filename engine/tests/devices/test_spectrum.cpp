// Spectrum (engine/src/devices/spectrum.cpp): calibrated band levels for
// every window, slope tilt, channel modes, decay, peak hold, the analysis
// frame through the C ABI, and bit-exact pass-through.
#include "device_harness.hpp"

using namespace devtest;
using namespace testutil;

namespace {

// Defaults of saine_core::builtin::spectrum, registry order.
const std::vector<float> kDefaults = {3.f, 0.f, 0.f, 30.f, 4.5f, 0.f, 90.f, 1.f, 30.f, 50.f, 1.f, 0.f, 3.f};
constexpr double kRate = 48000.0;
constexpr std::size_t kBands = 256;

DeviceRig rig() {
    DeviceRig r("spectrum", kDefaults, kRate, 256);
    r.set("slope", 0.f);
    return r;
}
std::size_t band_of(double hz) { return static_cast<std::size_t>(std::floor(kBands * std::log(hz / 20.0) / std::log(1000.0))); }
float max_level(const std::vector<float>& a, std::size_t from = 0, std::size_t to = kBands) {
    float m = -1000.0f;
    for (std::size_t i = from; i < to && i < a.size(); ++i) m = std::max(m, a[i]);
    return m;
}

} // namespace

TEST_CASE(spectrum_reads_a_sine_at_its_level_for_every_window) {
    for (int w = 0; w < 6; ++w) {
        auto r = rig();
        r.set("window", static_cast<float>(w));
        r.set("decay", 0.f);
        r.run_mono(tone(48000, 1000.0, kRate, 0.5)); // -6.02 dBFS
        const auto a = r.analysis();
        REQUIRE(a.size() == 2 * kBands);
        const std::size_t b = band_of(1000.0);
        const float at = max_level(a, b - 1, b + 2);
        // Scalloping: Rect up to -3.9 dB, Hann -1.4, flat-top ~0.
        const double tol = w == 5 ? 4.0 : w == 4 ? 0.1 : 1.6;
        if (std::fabs(at - -6.02) > tol) std::printf("    window %d: %.2f dB\n", w, at);
        CHECK(at <= -6.02 + 0.1 && at >= -6.02 - tol);
        // Far from the tone the level is well down (leakage depends on the window).
        if (w != 5) CHECK(max_level(a, band_of(4000.0), kBands) < -60.0f);
    }
}

TEST_CASE(spectrum_slope_tilts_around_1k) {
    auto flat = rig();
    flat.set("window", 4.f);
    flat.run_mono(tone(48000, 4000.0, kRate, 0.5));
    auto tilted = rig();
    tilted.set("window", 4.f);
    tilted.set("slope", 4.5f);
    tilted.run_mono(tone(48000, 4000.0, kRate, 0.5));
    const std::size_t b = band_of(4000.0);
    CHECK_NEAR(max_level(tilted.analysis(), b - 1, b + 2) - max_level(flat.analysis(), b - 1, b + 2), 9.0, 0.3);
}

TEST_CASE(spectrum_channel_modes) {
    const Signal tone_l = tone(48000, 500.0, kRate, 0.5);
    const Signal silent(48000, 0.0f);
    const std::size_t b = band_of(500.0);
    auto level = [&](int mode, const Signal& l, const Signal& r) {
        auto d = rig();
        d.set("channel_mode", static_cast<float>(mode));
        d.set("decay", 0.f);
        d.run(l, r);
        return d.analysis();
    };
    CHECK(max_level(level(1, tone_l, silent), b - 1, b + 2) > -8.0f);  // L
    CHECK(max_level(level(2, tone_l, silent), 0, kBands) <= -119.0f);  // R
    CHECK(max_level(level(4, tone_l, tone_l), 0, kBands) <= -119.0f);  // Side of a mono signal
    CHECK(max_level(level(3, tone_l, tone_l), b - 1, b + 2) > -8.0f);  // Mid
    // Stereo: mean power of L and R (one side silent: -3 dB).
    CHECK_NEAR(max_level(level(0, tone_l, silent), b - 1, b + 2) - max_level(level(1, tone_l, silent), b - 1, b + 2), -3.01, 0.05);
    const auto ms = level(5, tone_l, silent);
    REQUIRE(ms.size() == 3 * kBands);
    CHECK(max_level(ms, 2 * kBands + b - 1, 2 * kBands + b + 2) > -14.0f); // Side trace: half amplitude
}

TEST_CASE(spectrum_decay_and_peak_hold) {
    auto r = rig();
    r.set("decay", 0.f);
    r.set("peak_hold", 2.f); // Infinite
    r.run_mono(tone(24000, 2000.0, kRate, 0.5));
    r.run_mono(Signal(48000, 0.0f));
    auto a = r.analysis();
    const std::size_t b = band_of(2000.0);
    CHECK(max_level(a, 0, kBands) <= -119.0f);                    // decay 0: gone at once
    CHECK(max_level(a, kBands + b - 1, kBands + b + 2) > -8.0f);  // the peak stays
    r.set("peak_hold", 1.f); // 2 s: the peak starts again from the new mode
    r.set("decay", 100.f);   // 3 dB/s fall
    r.run_mono(tone(24000, 2000.0, kRate, 0.5));
    r.run_mono(Signal(24000, 0.0f)); // 0.5 s later: about 1.5 dB lower, peak held
    a = r.analysis();
    const float now = max_level(a, b - 1, b + 2);
    CHECK(now < -6.5f && now > -10.0f);
    CHECK(max_level(a, kBands + b - 1, kBands + b + 2) > -8.0f);
    r.set("peak_hold", 0.f);
    r.run_mono(Signal(4800, 0.0f));
    CHECK(max_level(r.analysis(), kBands, 2 * kBands) <= -119.0f);
}

TEST_CASE(spectrum_every_fft_size_and_pass_through) {
    for (int s = 0; s < 6; ++s) {
        auto r = rig();
        r.set("fft_size", static_cast<float>(s));
        r.set("window", 4.f);
        const Signal x = tone(48000, 3000.0, kRate, 0.25);
        const auto [l, rr] = r.run(x, x);
        for (std::size_t i = 0; i < x.size(); ++i)
            if (l[i] != x[i]) {
                CHECK(l[i] == x[i]);
                break;
            }
        uint64_t seq = 0;
        const auto a = r.analysis(&seq);
        CHECK(seq > 0);
        const std::size_t b = band_of(3000.0);
        CHECK_NEAR(max_level(a, b - 1, b + 2), -12.04, 0.15);
    }
}

TEST_CASE(spectrum_analysis_through_the_engine) {
    EngineHandle e(48000, 256);
    const uint64_t a = load_asset(e, {sine(96000, 1000.0, 48000.0, 0.5), sine(96000, 1000.0, 48000.0, 0.5)});
    DeviceSession s;
    auto& t = s.add_track(1, a, 96000);
    t.d.pan = 0.0f;
    DeviceSession::add_device(t, 31, "spectrum", kDefaults);
    REQUIRE(s.publish(e) == SE_OK);
    std::vector<float> out(1024);
    size_t n = 99;
    uint64_t seq = 99;
    REQUIRE(se_builtin_analysis(e, 31, out.data(), out.size(), &n, &seq) == SE_OK);
    CHECK(n == 0 && seq == 0); // nothing analysed yet
    REQUIRE(se_transport_play(e) == SE_OK);
    render(e, 24000, 256);
    REQUIRE(se_builtin_analysis(e, 31, out.data(), out.size(), &n, &seq) == SE_OK);
    CHECK(n == 2 * kBands);
    CHECK(seq > 0);
    const uint64_t first = seq;
    REQUIRE(se_builtin_analysis(e, 31, out.data(), out.size(), &n, &seq) == SE_OK);
    CHECK(seq == first); // no new frame without processing
    out.resize(n);
    CHECK(max_level(out, band_of(1000.0) - 1, band_of(1000.0) + 2) > -12.0f);
    CHECK(se_builtin_analysis(e, 12345, out.data(), out.size(), &n, &seq) == SE_ERR_NOT_FOUND);
    // Devices without an analysis output report 0 floats.
    DeviceSession c;
    auto& ct = c.add_track(1, a, 96000);
    DeviceSession::add_device(ct, 41, "tuner", {0.f, 0.f, 0.f});
    REQUIRE(c.publish(e) == SE_OK);
    render(e, 2400, 256);
    REQUIRE(se_builtin_analysis(e, 41, out.data(), out.size(), &n, &seq) == SE_OK);
    CHECK(n == 0 && seq == 0);
}

TEST_CASE(analysis_channel_hands_over_the_newest_frame) {
    saine::AnalysisChannel ch(4);
    float out[8] = {};
    uint64_t seq = 7;
    CHECK(ch.read(out, 8, seq) == 0);
    CHECK(seq == 0);
    for (int f = 1; f <= 3; ++f) {
        float* b = ch.begin();
        for (int i = 0; i < 4; ++i) b[i] = static_cast<float>(10 * f + i);
        ch.publish(f == 3 ? 2 : 4);
    }
    CHECK(ch.read(out, 8, seq) == 2); // only the newest frame
    CHECK(seq == 3);
    CHECK(out[0] == 30.0f && out[1] == 31.0f);
    CHECK(ch.read(out, 8, seq) == 2); // again: same frame
    CHECK(seq == 3);
    const long before = rt_alloc_count();
    {
        RtAllocGuard g;
        for (int k = 0; k < 100; ++k) {
            ch.begin()[0] = static_cast<float>(k);
            ch.publish(1);
        }
    }
    CHECK_EQ(rt_alloc_count() - before, 0);
    CHECK(ch.read(out, 8, seq) == 1);
    CHECK(out[0] == 99.0f);
}

TEST_CASE(spectrum_analyses_at_least_every_half_frame) {
    for (const auto& [size, per_second] : {std::pair{0.f, 187u}, std::pair{1.f, 93u}, std::pair{3.f, 46u}}) {
        auto r = rig();
        r.set("fft_size", size);
        r.run_mono(Signal(48000, 0.0f));
        uint64_t seq = 0;
        r.analysis(&seq);
        CHECK(seq >= per_second && seq <= per_second + 1);
    }
}
