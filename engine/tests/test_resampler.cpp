#include "resampler.hpp"
#include "test_harness.hpp"
#include "test_util.hpp"

#include <cmath>
#include <numbers>

using namespace testutil;

namespace {

// Amplitude of `freq` in x (single-bin DFT with a Hann window), normalised
// so a full-scale sine of amplitude A reports ~A.
double tone_amplitude(const std::vector<float>& x, std::size_t begin, std::size_t n, double freq, double rate) {
    double re = 0.0, im = 0.0, wsum = 0.0;
    for (std::size_t i = 0; i < n; ++i) {
        const double w = 0.5 - 0.5 * std::cos(2.0 * std::numbers::pi * static_cast<double>(i) / static_cast<double>(n));
        const double ph = 2.0 * std::numbers::pi * freq * static_cast<double>(begin + i) / rate;
        re += w * x[begin + i] * std::cos(ph);
        im += w * x[begin + i] * std::sin(ph);
        wsum += w;
    }
    return 2.0 * std::sqrt(re * re + im * im) / wsum;
}

// Estimates the dominant frequency by counting rising zero crossings with
// linear interpolation between samples.
double estimate_frequency(const std::vector<float>& x, std::size_t begin, std::size_t end, double rate) {
    double first = -1.0, last = -1.0;
    int crossings = 0;
    for (std::size_t i = begin + 1; i < end; ++i) {
        if (x[i - 1] < 0.0f && x[i] >= 0.0f) {
            const double t = static_cast<double>(i - 1) + x[i - 1] / static_cast<double>(x[i - 1] - x[i]);
            if (first < 0.0) first = t;
            last = t;
            ++crossings;
        }
    }
    return crossings > 1 ? (crossings - 1) * rate / (last - first) : 0.0;
}

} // namespace

TEST_CASE(resampler_length_and_frequency_44k1_to_48k) {
    const double src = 44100.0, dst = 48000.0, f = 1000.0;
    const std::size_t n = 44100;
    const auto in = sine(n, f, src, 0.5);
    std::vector<float> out;
    saine::SincResampler(src, dst).process(in.data(), n, out);
    CHECK_EQ(out.size(), static_cast<std::size_t>(48000));
    CHECK_EQ(saine::resampled_length(n, src, dst), 48000u);
    // Away from the edges (kernel half-width ~34 samples).
    const double est = estimate_frequency(out, 1000, out.size() - 1000, dst);
    CHECK_NEAR(est, f, 0.01);
    CHECK_NEAR(tone_amplitude(out, 1000, 32768, f, dst), 0.5, 0.0025); // within ~0.05 dB

    // Sample-accurate phase: compare against an ideal sine at the output rate.
    double max_err = 0.0;
    for (std::size_t i = 200; i < out.size() - 200; ++i) {
        const double ideal = 0.5 * std::sin(2.0 * std::numbers::pi * f * static_cast<double>(i) / dst);
        max_err = std::max(max_err, std::fabs(out[i] - ideal));
    }
    CHECK(max_err < 1e-3); // better than -66 dB re full scale
}

TEST_CASE(resampler_downsample_96k_to_48k_rejects_alias) {
    const double src = 96000.0, dst = 48000.0;
    const std::size_t n = 96000;
    // 5 kHz (passband) + 30 kHz (above the new Nyquist; would alias to 18 kHz).
    auto in = sine(n, 5000.0, src, 0.4);
    const auto hi = sine(n, 30000.0, src, 0.4);
    for (std::size_t i = 0; i < n; ++i) in[i] += hi[i];
    std::vector<float> out;
    saine::SincResampler(src, dst).process(in.data(), n, out);
    CHECK_EQ(out.size(), static_cast<std::size_t>(48000));
    CHECK_NEAR(tone_amplitude(out, 2000, 32768, 5000.0, dst), 0.4, 0.002);
    const double alias = tone_amplitude(out, 2000, 32768, 18000.0, dst);
    CHECK(alias < 0.4 * 1e-4); // > 80 dB rejection
}

TEST_CASE(resampler_odd_ratio_lengths) {
    std::vector<float> out;
    const auto in = sine(12345, 300.0, 22050.0);
    saine::SincResampler(22050.0, 48000.0).process(in.data(), in.size(), out);
    CHECK_EQ(out.size(), static_cast<std::size_t>(std::llround(12345.0 * 48000.0 / 22050.0)));
    saine::SincResampler(48000.0, 44100.0).process(in.data(), in.size(), out);
    CHECK_EQ(out.size(), static_cast<std::size_t>(std::llround(12345.0 * 44100.0 / 48000.0)));
    saine::SincResampler(48000.0, 44100.0).process(in.data(), 0, out);
    CHECK(out.empty());
}

TEST_CASE(asset_import_resamples_to_engine_rate) {
    EngineHandle e(48000.0);
    TempFile f("44k.wav");
    write_float_wav(f.path, {sine(44100, 1000.0, 44100.0, 0.5), sine(44100, 2000.0, 44100.0, 0.25)}, 44100);
    se_asset_info info{};
    REQUIRE(se_asset_load_file(e, f.path.c_str(), &info) == SE_OK);
    CHECK_EQ(info.frames, 48000u);
    CHECK_EQ(info.channels, 2u);
    CHECK(info.source_sample_rate == 44100.0);

    // Play it hard-left/right on two tracks and check frequencies at 48k.
    Session s;
    s.add_track(1, {clip(1, info.asset_id, 0, 0, 48000)}, 1.0f, -1.0f);
    REQUIRE(s.publish(e) == SE_OK);
    REQUIRE(se_transport_play(e) == SE_OK);
    const auto out = render(e, 48000, 512);
    std::vector<float> l(48000), r(48000);
    for (std::size_t i = 0; i < 48000; ++i) {
        l[i] = out[2 * i];
        r[i] = out[2 * i + 1];
    }
    CHECK_NEAR(estimate_frequency(l, 1000, 47000, 48000.0), 1000.0, 0.01);
    CHECK(tone_amplitude(r, 1000, 32768, 2000.0, 48000.0) < 1e-6); // hard left: right silent
}
