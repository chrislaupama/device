// Pitch Corrector (engine/src/devices/pitch_corrector.cpp): latency and
// transparency, scale snapping, Speed, Retune Speed, Humanize, Mix and the
// trace readouts, through the shared device harness.
#include "device_harness.hpp"

using namespace devtest;
using namespace testutil;

namespace {

// Defaults of dstudio_core::builtin::pitch_corrector, registry order:
// speed, key, scale, retune_speed_ms, humanize, mix, key_follow.
const std::vector<float> kDefaults = {50.f, 0.f, 0.f, 50.f, 20.f, 100.f, 1.f};
constexpr double kRate = 48000.0;

DeviceRig rig() {
    DeviceRig r("pitch-corrector", kDefaults, kRate, 256);
    return r;
}
DeviceRig hard() {
    auto r = rig();
    r.set("speed", 100.f);
    r.set("retune_speed_ms", 0.f);
    r.set("humanize", 0.f);
    return r;
}
double midi_hz(double m) { return 440.0 * std::pow(2.0, (m - 69.0) / 12.0); }
double cents(double hz, double ref) { return 1200.0 * std::log2(hz / ref); }

} // namespace

TEST_CASE(pitch_corrector_reports_latency_and_is_transparent_without_correction) {
    auto r = rig();
    const uint32_t lat = r.instance().latency();
    CHECK(lat == 1024);
    r.set("speed", 0.f);
    const Signal x = noise(48000, 0.5f, 3);
    const auto [l, rr] = r.run(x, x);
    double err = 0;
    for (std::size_t i = lat + 4000; i < x.size(); ++i) err = std::max(err, static_cast<double>(std::fabs(l[i] - x[i - lat])));
    CHECK(err < 1e-6);
    // Silence and unvoiced input glide back to no shift.
    CHECK(r.readouts()[0] == 0.0f);
}

TEST_CASE(pitch_corrector_hard_tune_snaps_chromatic_and_scales) {
    struct Case {
        double in_midi;
        int key, scale;
        double want_midi;
    };
    // Chromatic: A4 +40 ct -> A4; C major: C#4 +10 ct -> D4 (0.9 st away vs 1.1);
    // A minor: G#4 -> A4 or G4 (G#4 -0.3 st is nearer G4).
    const Case cases[] = {{69.4, 0, 0, 69.0}, {61.1, 0, 1, 62.0}, {67.7, 9, 2, 67.0}, {64.3, 0, 1, 64.0}};
    for (const Case& c : cases) {
        auto r = hard();
        r.set("key", static_cast<float>(c.key));
        r.set("scale", static_cast<float>(c.scale));
        const Signal x = tone(48000, midi_hz(c.in_midi), kRate, 0.4);
        const Signal y = r.run_mono(x);
        const double got = zero_cross_hz(y, kRate, 24000, 48000);
        const double err = cents(got, midi_hz(c.want_midi));
        if (std::fabs(err) > 3.0) std::printf("    in %.2f -> %.2f Hz, want %.2f Hz (%.1f ct)\n", c.in_midi, got, midi_hz(c.want_midi), err);
        CHECK(std::fabs(err) < 3.0);
        const auto ro = r.readouts();
        CHECK(ro[0] == 1.0f);
        CHECK_NEAR(ro[1], c.in_midi, 0.02);
        CHECK_NEAR(ro[3], c.want_midi, 1e-6);
        CHECK_NEAR(ro[2], c.want_midi - c.in_midi, 0.02);
    }
}

TEST_CASE(pitch_corrector_splices_are_clean) {
    // A rich tone 40 ct sharp of A3, hard-tuned: the heads splice every few
    // periods. The short-term level stays flat and no step exceeds the
    // input's own steepest slope.
    Signal x(96000, 0.0f);
    const double f0 = midi_hz(57.4);
    double max_step = 0;
    for (std::size_t i = 0; i < x.size(); ++i) {
        for (int h = 1; h <= 12; ++h) x[i] += static_cast<float>(0.25 / h * std::sin(2.0 * 3.14159265358979323846 * h * f0 * static_cast<double>(i) / kRate));
        if (i > 0) max_step = std::max(max_step, static_cast<double>(std::fabs(x[i] - x[i - 1])));
    }
    auto r = hard();
    const Signal y = r.run_mono(x);
    double lo = 1e9, hi = -1e9, worst = 0;
    for (std::size_t s = 48000; s + 960 <= y.size(); s += 480) {
        const double v = db(rms(y, s, s + 960));
        lo = std::min(lo, v);
        hi = std::max(hi, v);
    }
    for (std::size_t i = 48001; i < y.size(); ++i) worst = std::max(worst, static_cast<double>(std::fabs(y[i] - y[i - 1])));
    double xlo = 1e9, xhi = -1e9;
    for (std::size_t s = 48000; s + 960 <= x.size(); s += 480) {
        const double v = db(rms(x, s, s + 960));
        xlo = std::min(xlo, v);
        xhi = std::max(xhi, v);
    }
    CHECK(hi - lo < xhi - xlo + 0.2); // no level dips beyond the window ripple of the input itself
    CHECK(worst < 1.2 * max_step);
    CHECK(std::fabs(cents(zero_cross_hz(y, kRate, 48000, 96000), midi_hz(57.0))) < 3.0);
}

TEST_CASE(pitch_corrector_speed_scales_the_correction) {
    auto r = hard();
    r.set("speed", 50.f);
    const Signal y = r.run_mono(tone(48000, midi_hz(69.4), kRate, 0.4));
    // Half of -40 ct.
    CHECK(std::fabs(cents(zero_cross_hz(y, kRate, 24000, 48000), midi_hz(69.2))) < 3.0);
}

TEST_CASE(pitch_corrector_retune_speed_sets_the_glide) {
    auto measure = [](float retune) {
        auto r = hard();
        r.set("retune_speed_ms", retune);
        const Signal y = r.run_mono(tone(48000, midi_hz(69.45), kRate, 0.4));
        // 60..100 ms after the estimate starts (detector lag + latency ~ 2 x 21 ms).
        return cents(zero_cross_hz(y, kRate, 4800, 6720), midi_hz(69.45));
    };
    const double fast = measure(0.f), slow = measure(400.f);
    CHECK(fast < -40.0);              // essentially at A4 already
    CHECK(slow > -20.0 && slow < 0.0); // still on its way
}

TEST_CASE(pitch_corrector_humanize_keeps_vibrato) {
    // A4 + 25 ct centre with a 5.5 Hz vibrato of +-30 ct.
    const auto hz = [](double t) { return midi_hz(69.25 + 0.3 * std::sin(2.0 * 3.14159265358979323846 * 5.5 * t)); };
    auto depth = [&](float human) {
        auto r = hard();
        r.set("humanize", human);
        const Signal y = r.run_mono(sweep(96000, kRate, 0.4, hz));
        // Pitch in 10 ms frames over the last second: depth = max - min (ct).
        double lo = 1e9, hi = -1e9, sum = 0;
        int n = 0;
        for (std::size_t s = 48000; s + 480 < y.size(); s += 480) {
            const double c = cents(zero_cross_hz(y, kRate, s, s + 960), 440.0);
            lo = std::min(lo, c);
            hi = std::max(hi, c);
            sum += c;
            ++n;
        }
        return std::pair<double, double>{hi - lo, sum / n};
    };
    const auto [flat, flat_mean] = depth(0.f);
    const auto [kept, kept_mean] = depth(100.f);
    CHECK(flat < 25.0);  // vibrato largely flattened (input swing 60 ct)
    CHECK(kept > 40.0);  // vibrato kept
    CHECK(std::fabs(flat_mean) < 6.0);
    CHECK(std::fabs(kept_mean) < 8.0); // the centre is still pulled onto A4
}

TEST_CASE(pitch_corrector_mix_blends_the_delayed_dry_signal) {
    auto r = hard();
    r.set("mix", 0.f);
    const Signal x = tone(24000, midi_hz(69.4), kRate, 0.4);
    const Signal y = r.run_mono(x);
    const uint32_t lat = r.instance().latency();
    double err = 0;
    for (std::size_t i = lat + 2000; i < x.size(); ++i) err = std::max(err, static_cast<double>(std::fabs(y[i] - x[i - lat])));
    CHECK(err < 1e-6);
}

TEST_CASE(pitch_corrector_releases_on_unvoiced_input) {
    auto r = hard();
    r.run_mono(tone(24000, midi_hz(69.4), kRate, 0.4));
    CHECK(r.readouts()[0] == 1.0f);
    r.run_mono(noise(24000, 0.3f, 5));
    const auto ro = r.readouts();
    CHECK(ro[0] == 0.0f);
    CHECK(std::fabs(ro[2]) < 0.01f);
}
