// Shared spectral and pitch kernels (engine/src/devices/spectral.hpp,
// pitch_detect.hpp) used by the Pitch Corrector, Tuner and Spectrum.
#include "device_harness.hpp"
#include "devices/pitch_detect.hpp"
#include "devices/spectral.hpp"

using namespace devtest;
using namespace testutil;

namespace {

// Feeds a signal in 128-frame pieces, returns the estimate after the last piece.
saine::dsp::PitchEstimate detect(saine::dsp::PitchDetector& d, const Signal& x) {
    for (std::size_t i = 0; i < x.size(); i += 128) d.push(x.data() + i, static_cast<uint32_t>(std::min<std::size_t>(128, x.size() - i)));
    return d.estimate();
}

Signal harmonic(std::size_t n, double hz, double rate, int harmonics) {
    Signal s(n, 0.0f);
    for (int h = 1; h <= harmonics && h * hz < rate / 2; ++h)
        for (std::size_t i = 0; i < n; ++i)
            s[i] += static_cast<float>(0.3 / h * std::sin(2.0 * 3.14159265358979323846 * h * hz * static_cast<double>(i) / rate));
    return s;
}

} // namespace

TEST_CASE(fft_round_trips_and_matches_a_direct_dft) {
    for (uint32_t n : {8u, 16u, 64u, 1024u, 16384u}) {
        saine::dsp::RealFft f;
        f.prepare(n);
        Signal x = noise(n, 1.0f, n);
        std::vector<float> re(n / 2 + 1), im(n / 2 + 1), y(n);
        f.forward(x.data(), re.data(), im.data());
        if (n <= 1024) {
            for (uint32_t k = 0; k <= n / 2; k += (n >= 64 ? 7 : 1)) {
                double dr = 0, di = 0;
                for (uint32_t i = 0; i < n; ++i) {
                    const double a = -2.0 * 3.14159265358979323846 * k * i / n;
                    dr += x[i] * std::cos(a);
                    di += x[i] * std::sin(a);
                }
                CHECK_NEAR(re[k], dr, 1e-3 * std::sqrt(n));
                CHECK_NEAR(im[k], di, 1e-3 * std::sqrt(n));
            }
        }
        f.inverse(re.data(), im.data(), y.data());
        double err = 0;
        for (uint32_t i = 0; i < n; ++i) err = std::max(err, static_cast<double>(std::fabs(y[i] - x[i])));
        CHECK(err < 1e-4);
    }
}

TEST_CASE(windows_have_their_coherent_gain) {
    std::vector<float> w(4096);
    CHECK_NEAR(saine::dsp::fill_window(saine::dsp::Window::Hann, 4096, w.data()), 0.5, 1e-6);
    CHECK_NEAR(saine::dsp::fill_window(saine::dsp::Window::Hamming, 4096, w.data()), 0.54, 1e-6);
    CHECK_NEAR(saine::dsp::fill_window(saine::dsp::Window::Blackman, 4096, w.data()), 0.42, 1e-6);
    CHECK_NEAR(saine::dsp::fill_window(saine::dsp::Window::Rect, 4096, w.data()), 1.0, 1e-9);
    CHECK_NEAR(w[0], 1.0, 1e-9);
    saine::dsp::fill_window(saine::dsp::Window::Hann, 4096, w.data());
    CHECK_NEAR(w[0], 0.0, 1e-9);
    CHECK_NEAR(w[2048], 1.0, 1e-6);
}

TEST_CASE(pitch_detector_finds_sines_across_the_range_and_rates) {
    for (double rate : {44100.0, 48000.0, 96000.0}) {
        saine::dsp::PitchDetector d;
        d.prepare(rate, 30.0f, 1500.0f, 5.0f);
        for (double hz : {41.2, 82.41, 110.0, 196.0, 440.0, 659.3, 1046.5, 1400.0}) {
            d.reset();
            const auto e = detect(d, tone(static_cast<std::size_t>(0.4 * rate), hz, rate, 0.5));
            CHECK(e.voiced);
            const double cents = 1200.0 * std::log2(e.hz / hz);
            if (std::fabs(cents) > 1.0) std::printf("    %.0f Hz @ %.0f: %.3f Hz (%.2f ct)\n", hz, rate, e.hz, cents);
            CHECK(std::fabs(cents) < 1.0);
        }
    }
}

TEST_CASE(pitch_detector_avoids_octave_errors_and_rejects_noise) {
    saine::dsp::PitchDetector d;
    d.prepare(48000.0, 55.0f, 1400.0f, 3.0f);
    // Rich harmonic tones (saw-like) and a tone with a missing fundamental.
    for (double hz : {98.0, 147.0, 220.0, 330.0}) {
        d.reset();
        const auto e = detect(d, harmonic(19200, hz, 48000.0, 20));
        CHECK(e.voiced);
        CHECK(std::fabs(1200.0 * std::log2(e.hz / hz)) < 2.0);
    }
    d.reset();
    CHECK(!detect(d, noise(19200, 0.5f, 9)).voiced);
    d.reset();
    CHECK(!detect(d, Signal(19200, 0.0f)).voiced);
    d.reset();
    CHECK(!detect(d, tone(19200, 220.0, 48000.0, 1e-4)).voiced); // below the level gate
    // Lag: half the analysis window, as input frames.
    CHECK(d.lag() == 1024);
    CHECK(d.hop() > 0 && d.hop() <= 160);
}

TEST_CASE(pitch_detector_is_allocation_free) {
    saine::dsp::PitchDetector d;
    d.prepare(48000.0, 30.0f, 1500.0f, 5.0f);
    const Signal x = harmonic(48000, 150.0, 48000.0, 8);
    const long before = rt_alloc_count();
    {
        RtAllocGuard g;
        detect(d, x);
        d.reset();
    }
    CHECK_EQ(rt_alloc_count() - before, 0);
}
