// Restoration devices (engine/src/devices/declipper.cpp, deesser.cpp,
// dehummer.cpp, denoiser.cpp) and their spectral helpers (spectral.hpp),
// through the shared device harness.
#include "device_harness.hpp"
#include "devices/spectral.hpp"

using namespace devtest;

namespace {

constexpr double kRate = 48000.0;
const std::vector<float> kDeClipper = {-1.f, -0.3f, 1.f, 100.f};
const std::vector<float> kDeEsser = {7000.f, 1.f, -36.f, 8.f, 0.f, 100.f};
const std::vector<float> kDeHummer = {60.f, 4.f, 5.f, -30.f, 100.f};
const std::vector<float> kDeNoiser = {-40.f, -20.f, 50.f, 5.f, 50.f, 100.f};

// Level change (dB) of a steady tone through a device, measured after the settle point.
double tone_gain_db(DeviceRig& r, double hz, double amp, std::size_t n = 48000, std::size_t settle = 24000) {
    const Signal x = tone(n, hz, kRate, amp);
    const Signal y = r.run_mono(x);
    return db(rms(y, settle) / rms(x, settle));
}

// Gaussian-ish noise (sum of 4 uniforms) with the given RMS.
Signal soft_noise(std::size_t n, double rms_level, uint32_t seed) {
    Signal s(n, 0.0f);
    for (uint32_t k = 0; k < 4; ++k) {
        const Signal u = noise(n, 1.0f, seed + 17 * k);
        for (std::size_t i = 0; i < n; ++i) s[i] += u[i];
    }
    // A sum of 4 uniforms on [-1, 1] has variance 4 / 3.
    const double g = rms_level / std::sqrt(4.0 / 3.0);
    for (float& v : s) v = static_cast<float>(v * g);
    return s;
}

} // namespace

// ---------------------------------------------------------------- spectral helpers

TEST_CASE(real_fft_round_trips_and_finds_a_bin) {
    dstudio::dsp::RealFft fft;
    fft.prepare(256);
    const Signal x = noise(256, 1.0f, 9);
    std::vector<float> re(129), im(129), y(256);
    fft.forward(x.data(), re.data(), im.data());
    fft.inverse(re.data(), im.data(), y.data());
    double err = 0.0;
    for (std::size_t i = 0; i < 256; ++i) err = std::max(err, static_cast<double>(std::fabs(y[i] - x[i])));
    CHECK(err < 1e-5);
    // A cosine on bin 10: |X[10]| = N / 2, the others ~0.
    Signal c(256);
    for (std::size_t i = 0; i < 256; ++i) c[i] = static_cast<float>(std::cos(2.0 * 3.14159265358979 * 10.0 * static_cast<double>(i) / 256.0));
    fft.forward(c.data(), re.data(), im.data());
    CHECK_NEAR(std::hypot(re[10], im[10]), 128.0, 1e-3);
    CHECK(std::hypot(re[11], im[11]) < 1e-3);
    CHECK(std::fabs(re[0]) < 1e-3);
}

TEST_CASE(spectrum_tap_reads_a_full_scale_sine_at_0_db) {
    dstudio::dsp::SpectrumTap tap;
    tap.prepare(kRate);
    const Signal x = tone(16384, 1000.0, kRate, 1.0);
    tap.push(x.data(), x.data(), static_cast<uint32_t>(x.size()));
    float bands[dstudio::dsp::kSpectrumBands];
    tap.write(bands);
    uint32_t best = 0;
    for (uint32_t b = 0; b < dstudio::dsp::kSpectrumBands; ++b)
        if (bands[b] > bands[best]) best = b;
    const float hz = dstudio::dsp::spectrum_band_hz(best);
    CHECK(hz > 850.0f && hz < 1180.0f);
    CHECK_NEAR(bands[best], 0.0, 1.5);
    CHECK(bands[0] < -60.0f);
    CHECK(bands[dstudio::dsp::kSpectrumBands - 1] < -60.0f);
}

// ---------------------------------------------------------------- De-Esser

TEST_CASE(deesser_is_transparent_below_the_threshold) {
    DeviceRig r("deesser", kDeEsser, kRate);
    const Signal x = noise(24000, 0.001f, 4); // band level far under -36 dB
    const Signal y = r.run_mono(x);
    double err = 0.0;
    for (std::size_t i = 0; i < x.size(); ++i) err = std::max(err, static_cast<double>(std::fabs(y[i] - x[i])));
    CHECK(err < 1e-7);
    CHECK_NEAR(r.readouts()[0], 0.0, 1e-6);
}

TEST_CASE(deesser_cuts_a_loud_band_up_to_the_reduction) {
    DeviceRig r("deesser", kDeEsser, kRate);
    // -6 dBFS at the centre: 30 dB over the threshold, so the cut sits near its 8 dB cap.
    const double g = tone_gain_db(r, 7000.0, 0.5);
    CHECK(g < -6.5 && g > -8.05);
    CHECK(r.readouts()[0] < -6.5f);
    // More Reduction cuts deeper.
    DeviceRig deep("deesser", kDeEsser, kRate);
    deep.set("reduction", 20.f);
    CHECK(tone_gain_db(deep, 7000.0, 0.5) < g - 8.0);
}

TEST_CASE(deesser_leaves_the_rest_of_the_spectrum_alone) {
    DeviceRig r("deesser", kDeEsser, kRate);
    CHECK_NEAR(tone_gain_db(r, 200.0, 0.5), 0.0, 0.1);
    // A sibilant and a low tone together: the low tone keeps its level.
    DeviceRig both("deesser", kDeEsser, kRate);
    const Signal lo = tone(48000, 200.0, kRate, 0.3);
    const Signal hi = tone(48000, 7000.0, kRate, 0.5);
    Signal x(48000);
    for (std::size_t i = 0; i < x.size(); ++i) x[i] = lo[i] + hi[i];
    const Signal y = both.run_mono(x);
    // A 7-tap moving average removes most of 7 kHz and barely touches 200 Hz.
    auto smooth = [](const Signal& s) {
        Signal o(s.size(), 0.0f);
        for (std::size_t i = 7; i < s.size(); ++i) {
            double a = 0.0;
            for (std::size_t k = 0; k < 7; ++k) a += s[i - k];
            o[i] = static_cast<float>(a / 7.0);
        }
        return o;
    };
    CHECK_NEAR(db(rms(smooth(y), 24000) / rms(smooth(lo), 24000)), 0.0, 0.25);
}

TEST_CASE(deesser_listen_plays_the_band_and_mix_zero_is_dry) {
    DeviceRig r("deesser", kDeEsser, kRate);
    r.set("listen", 1.f);
    r.set("reduction", 0.f);
    CHECK_NEAR(tone_gain_db(r, 7000.0, 0.1), 0.0, 0.1);
    DeviceRig lo("deesser", kDeEsser, kRate);
    lo.set("listen", 1.f);
    CHECK(tone_gain_db(lo, 300.0, 0.1) < -25.0);
    DeviceRig dry("deesser", kDeEsser, kRate);
    dry.set("mix", 0.f);
    CHECK_NEAR(tone_gain_db(dry, 7000.0, 0.5), 0.0, 1e-3);
}

TEST_CASE(deesser_reports_the_input_spectrum) {
    DeviceRig r("deesser", kDeEsser, kRate);
    (void)r.run_mono(tone(24000, 7000.0, kRate, 0.5));
    const auto ro = r.readouts();
    REQUIRE(ro.size() == 1 + dstudio::dsp::kSpectrumBands);
    uint32_t best = 0;
    for (uint32_t b = 0; b < dstudio::dsp::kSpectrumBands; ++b)
        if (ro[1 + b] > ro[1 + best]) best = b;
    CHECK(std::fabs(std::log2(dstudio::dsp::spectrum_band_hz(best) / 7000.0)) < 0.15);
    CHECK_NEAR(ro[1 + best], db(0.5), 1.5);
}

// ---------------------------------------------------------------- De-Hummer

TEST_CASE(dehummer_notches_the_fundamental_and_harmonics_to_depth) {
    for (double h : {60.0, 120.0, 180.0, 240.0}) {
        DeviceRig r("dehummer", kDeHummer, kRate);
        CHECK_NEAR(tone_gain_db(r, h, 0.5, 96000, 72000), -30.0, 1.0);
    }
    DeviceRig r("dehummer", kDeHummer, kRate);
    CHECK_NEAR(tone_gain_db(r, 1500.0, 0.5), 0.0, 0.3);
    // Harmonics = 4: the 7th is untouched; with 8 it is notched.
    DeviceRig four("dehummer", kDeHummer, kRate);
    CHECK(tone_gain_db(four, 420.0, 0.5, 96000, 72000) > -1.0);
    DeviceRig eight("dehummer", kDeHummer, kRate);
    eight.set("num_harmonics", 8.f);
    // The 6th and 8th notches deepen the 7th a little (the drawn product).
    const double g7 = tone_gain_db(eight, 420.0, 0.5, 96000, 72000);
    CHECK(g7 < -29.0 && g7 > -36.0);
}

TEST_CASE(dehummer_follows_frequency_depth_and_mix) {
    DeviceRig fifty("dehummer", kDeHummer, kRate);
    fifty.set("frequency", 50.f);
    fifty.set("depth", -50.f);
    CHECK_NEAR(tone_gain_db(fifty, 100.0, 0.5, 96000, 72000), -50.0, 1.5);
    DeviceRig flat("dehummer", kDeHummer, kRate);
    flat.set("depth", 0.f);
    CHECK_NEAR(tone_gain_db(flat, 60.0, 0.5), 0.0, 1e-3);
    DeviceRig half("dehummer", kDeHummer, kRate);
    half.set("mix", 50.f);
    // (1 - mix) + mix * H at the notch: 0.5 + 0.5 * 0.0316 -> -5.75 dB.
    CHECK_NEAR(tone_gain_db(half, 60.0, 0.5, 96000, 72000), -5.75, 0.3);
}

TEST_CASE(dehummer_harmonic_changes_do_not_click) {
    DeviceRig r("dehummer", kDeHummer, kRate);
    const Signal x = tone(48000, 1000.0, kRate, 0.5);
    std::vector<std::pair<std::size_t, std::pair<uint32_t, float>>> ev = {{12000, {1, 8.f}}, {24000, {1, 1.f}}, {36000, {0, 50.f}}};
    const Signal y = r.run(x, x, nullptr, nullptr, ev).first;
    double step = 0.0;
    for (std::size_t i = 1; i < y.size(); ++i) step = std::max(step, static_cast<double>(std::fabs(y[i] - y[i - 1])));
    // A 1 kHz sine at 0.5 moves at most 0.065 per sample; a click would jump far more.
    CHECK(step < 0.08);
}

// ---------------------------------------------------------------- De-Noiser

TEST_CASE(denoiser_reports_latency_and_reconstructs_when_open) {
    DeviceRig r("denoiser", kDeNoiser, kRate);
    CHECK_EQ(r.instance().latency(), 1024u);
    r.set("reduction", 0.f);
    const Signal x = concat(tone(48000, 440.0, kRate, 0.5), Signal(1024, 0.0f));
    const Signal y = r.run_mono(x);
    double err = 0.0;
    for (std::size_t i = 2048; i < 48000; ++i) err = std::max(err, static_cast<double>(std::fabs(y[i + 1024] - x[i])));
    CHECK(err < 1e-5);
}

TEST_CASE(denoiser_turns_down_steady_noise_and_keeps_the_tone) {
    DeviceRig r("denoiser", kDeNoiser, kRate);
    const Signal n = soft_noise(96000, 0.01, 21); // -40 dBFS RMS: about -63 dB per bin
    const Signal y = r.run_mono(n);
    const double cut = db(rms(y, 48000) / rms(n, 48000));
    CHECK(cut < -16.0 && cut > -21.0);
    CHECK(r.readouts()[0] < -15.0f);
    DeviceRig t("denoiser", kDeNoiser, kRate);
    CHECK_NEAR(tone_gain_db(t, 1000.0, 0.25, 96000, 48000), 0.0, 0.3);
    CHECK(t.readouts()[0] > -1.0f);
    // Deeper reduction, deeper cut.
    DeviceRig deep("denoiser", kDeNoiser, kRate);
    deep.set("reduction", -40.f);
    const Signal yd = deep.run_mono(n);
    CHECK(db(rms(yd, 48000) / rms(n, 48000)) < cut - 10.0);
}

TEST_CASE(denoiser_mix_zero_is_the_delayed_input) {
    DeviceRig r("denoiser", kDeNoiser, kRate);
    r.set("mix", 0.f);
    const Signal x = soft_noise(24000, 0.01, 5);
    const Signal y = r.run_mono(x);
    double err = 0.0;
    for (std::size_t i = 2048; i + 1024 < x.size(); ++i) err = std::max(err, static_cast<double>(std::fabs(y[i + 1024] - x[i])));
    CHECK(err < 1e-5);
}

// ---------------------------------------------------------------- De-Clipper

TEST_CASE(declipper_is_transparent_below_the_threshold) {
    DeviceRig r("declipper", kDeClipper, kRate);
    const uint32_t lat = r.instance().latency();
    CHECK_EQ(lat, 104u);
    const Signal x = concat(tone(24000, 440.0, kRate, 0.5), Signal(lat, 0.0f));
    const Signal y = r.run_mono(x);
    double err = 0.0;
    for (std::size_t i = 0; i < 24000; ++i) err = std::max(err, static_cast<double>(std::fabs(y[i + lat] - x[i])));
    CHECK(err < 1e-6);
}

TEST_CASE(declipper_rebuilds_flattened_peaks) {
    DeviceRig r("declipper", kDeClipper, kRate);
    const uint32_t lat = r.instance().latency();
    r.set("threshold", -5.5f);
    r.set("ceiling", 0.f);
    const Signal clean = concat(tone(24000, 500.0, kRate, 0.8), Signal(lat, 0.0f));
    Signal clipped = clean;
    for (float& v : clipped) v = std::clamp(v, -0.55f, 0.55f);
    const Signal y = r.run_mono(clipped);
    double e_in = 0.0, e_out = 0.0, pk = 0.0;
    for (std::size_t i = 4800; i < 24000; ++i) {
        e_in += (clipped[i] - clean[i]) * (clipped[i] - clean[i]);
        e_out += (y[i + lat] - clean[i]) * (y[i + lat] - clean[i]);
        pk = std::max(pk, static_cast<double>(std::fabs(y[i + lat])));
    }
    CHECK(db(std::sqrt(e_out / e_in)) < -10.0); // the repair is >= 10 dB closer to the original
    CHECK(pk > 0.72);
}

TEST_CASE(declipper_keeps_the_polarity_of_adjacent_opposite_runs) {
    DeviceRig r("declipper", kDeClipper, kRate);
    const uint32_t lat = r.instance().latency();
    r.set("threshold", -5.5f);
    r.set("ceiling", 0.f);
    // A positive clipped run that ends on a negative clipped sample, and back.
    const float cycle[] = {0.0f, 0.3f, 0.55f, 0.55f, -0.55f, -0.55f, -0.3f, 0.0f};
    Signal x(9600 + lat, 0.0f);
    for (std::size_t i = 0; i < 9600; ++i) x[i] = cycle[i % 8];
    const Signal y = r.run_mono(x);
    int flipped = 0;
    for (std::size_t i = 960; i < 9600; ++i)
        if (std::fabs(x[i]) >= 0.5f && x[i] * y[i + lat] < 0.0f) ++flipped;
    CHECK(flipped == 0);
}

TEST_CASE(declipper_holds_the_ceiling_with_true_peak) {
    for (float ceiling : {-0.3f, -3.f}) {
        DeviceRig r("declipper", kDeClipper, kRate);
        r.set("threshold", 0.f);
        r.set("ceiling", ceiling);
        const Signal y = r.run_mono(tone(24000, 300.0, kRate, 1.5));
        CHECK(peak(y) <= std::pow(10.0, ceiling / 20.0) + 1e-6);
    }
    // A 12 kHz sine sampled 45 degrees off its peaks: samples at 0.85, true peak 1.2.
    for (float factor : {0.f, 2.f}) {
        DeviceRig r("declipper", kDeClipper, kRate);
        r.set("oversampling", factor);
        Signal x(24000);
        for (std::size_t i = 0; i < x.size(); ++i) x[i] = static_cast<float>(1.2 * std::sin(2.0 * 3.14159265358979 * 0.25 * static_cast<double>(i) + 0.785398));
        const Signal y = r.run_mono(x);
        // Held to the ceiling as a true peak: the samples sit near 0.966 * 0.707.
        CHECK(peak(y, 12000) < 0.72);
        CHECK(peak(y, 12000) > 0.62);
    }
}

TEST_CASE(declipper_mix_blends_with_the_original) {
    DeviceRig r("declipper", kDeClipper, kRate);
    const uint32_t lat = r.instance().latency();
    r.set("threshold", -5.5f);
    r.set("ceiling", 0.f);
    r.set("mix", 0.f);
    Signal clipped = concat(tone(24000, 500.0, kRate, 0.8), Signal(lat, 0.0f));
    for (float& v : clipped) v = std::clamp(v, -0.55f, 0.55f);
    const Signal y = r.run_mono(clipped);
    double err = 0.0;
    for (std::size_t i = 4800; i < 24000; ++i) err = std::max(err, static_cast<double>(std::fabs(y[i + lat] - clipped[i])));
    CHECK(err < 1e-6);
}
