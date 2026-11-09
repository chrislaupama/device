// Multiband Compressor (engine/src/devices/multiband_compressor.cpp): flat
// crossover sum, band isolation, downward and upward laws (values from
// saine_core::builtin::multiband_gain), silence handling, Depth / Downward /
// Time / Mix / band input, and readouts.
#include "device_harness.hpp"

using namespace devtest;

namespace {

// Defaults of saine_core::builtin::multiband_compressor, registry order.
const std::vector<float> kDefaults = {
    120.f, 2500.f,                          //
    -18.f, 2.5f, 30.f, 200.f, 0.f,          // low
    -20.f, 2.f,  15.f, 120.f, 0.f,          // mid
    -22.f, 2.f,  5.f,  80.f,  0.f,          // high
    100.f, 0.f,                             // mix, output
    0.f,   -60.f, 1.f, 0.f, -60.f, 1.f, 0.f, -60.f, 1.f, // band input, floor, lift
    0.f,   100.f, 100.f, 100.f, 100.f};     // input, time, upward, downward, depth
constexpr double kRate = 48000.0;

DeviceRig rig() { return DeviceRig("multiband-compressor", kDefaults, kRate, 256); }

// Amplitude of frequency f in x[from..] (Hann-windowed single-bin DFT).
double tone_amp(const Signal& x, double f, std::size_t from) {
    double re = 0, im = 0;
    const std::size_t n = x.size() - from;
    for (std::size_t i = 0; i < n; ++i) {
        const double w = 2.0 * 3.14159265358979323846 * f * static_cast<double>(i) / kRate;
        const double win = 0.5 - 0.5 * std::cos(2.0 * 3.14159265358979323846 * static_cast<double>(i) / static_cast<double>(n));
        re += win * x[from + i] * std::cos(w);
        im += win * x[from + i] * std::sin(w);
    }
    return 4.0 * std::sqrt(re * re + im * im) / static_cast<double>(n);
}

// Steady gain (dB) of a tone through the device.
double tone_gain(DeviceRig& r, double hz, double amp) {
    const auto y = r.run_mono(tone(96000, hz, kRate, amp));
    return db(tone_amp(y, hz, 48000) / amp);
}

void no_compression(DeviceRig& r) {
    for (const char* id : {"low_ratio", "mid_ratio", "high_ratio"}) r.set(id, 1.f);
}

} // namespace

TEST_CASE(multiband_crossover_sum_is_flat) {
    for (double hz : {40.0, 120.0, 400.0, 1000.0, 2500.0, 6000.0, 15000.0}) {
        auto r = rig();
        no_compression(r);
        CHECK_NEAR(tone_gain(r, hz, 0.25), 0.0, 0.05);
    }
    // White noise: same RMS in and out.
    auto r = rig();
    no_compression(r);
    const Signal x = noise(96000, 0.3f, 2);
    CHECK_NEAR(db(rms(r.run_mono(x), 4800)) - db(rms(x, 4800)), 0.0, 0.05);
}

TEST_CASE(multiband_bands_compress_independently) {
    // A loud 50 Hz tone is reduced by the low band law; a quiet 6 kHz tone
    // alongside it keeps its level.
    auto r = rig();
    const Signal lo = tone(96000, 50.0, kRate, 0.5), hi = tone(96000, 6000.0, kRate, 0.01);
    Signal x(lo.size());
    for (std::size_t i = 0; i < x.size(); ++i) x[i] = lo[i] + hi[i];
    const auto y = r.run_mono(x);
    const auto ro = r.readouts();
    // Low band: the 30 ms attack envelope reads a little under the -6 dB
    // peak; its gain follows the band law (threshold -18, 2.5:1).
    CHECK(ro[2] < db(0.5) && ro[2] > db(0.5) - 3.0);
    CHECK_NEAR(ro[5], -0.6 * (ro[2] + 18.0), 0.4);
    CHECK(ro[5] < -4.0f);
    CHECK_NEAR(ro[7], 0.0, 1e-3); // high band untouched
    CHECK_NEAR(db(tone_amp(y, 6000.0, 48000) / 0.01), 0.0, 0.05);
    CHECK_NEAR(db(tone_amp(y, 50.0, 48000) / 0.5), ro[5], 1.0);
}

TEST_CASE(multiband_upward_lifts_below_the_floor_but_not_silence) {
    auto r = rig();
    no_compression(r);
    r.set("mid_up_threshold", -45.f);
    r.set("mid_up_ratio", 4.f);
    // 1 kHz tone, peak -60 dB: about 15 dB under the floor, x 0.75 (4:1 lift).
    const double g = tone_gain(r, 1000.0, 0.001);
    const auto ro = r.readouts();
    CHECK_NEAR(ro[6], 0.75 * (-45.0 - ro[3]), 0.3);
    CHECK_NEAR(g, ro[6], 0.5);
    CHECK(g > 11.0);
    // Upward scales.
    auto half = rig();
    no_compression(half);
    half.set("mid_up_threshold", -45.f);
    half.set("mid_up_ratio", 4.f);
    half.set("upward", 50.f);
    CHECK_NEAR(tone_gain(half, 1000.0, 0.001), g / 2.0, 0.5);
    // Digital silence stays silent; -120 dB noise is not lifted.
    auto s = rig();
    for (const char* id : {"low_up_threshold", "mid_up_threshold", "high_up_threshold"}) s.set(id, 0.f);
    for (const char* id : {"low_up_ratio", "mid_up_ratio", "high_up_ratio"}) s.set(id, 20.f);
    s.set("upward", 200.f);
    const auto z = s.run_mono(Signal(48000, 0.0f));
    CHECK(peak(z) == 0.0);
    const Signal faint = noise(48000, 1e-6f, 8);
    CHECK(db(rms(s.run_mono(faint), 24000) / rms(faint, 24000)) < 3.0);
}

TEST_CASE(multiband_depth_downward_time_mix_and_band_input) {
    const auto low_gain = [](const char* id, float v) {
        auto r = rig();
        r.set(id, v);
        r.run_mono(tone(96000, 50.0, kRate, 0.5));
        return static_cast<double>(r.readouts()[5]);
    };
    const double full = low_gain("depth", 100.f);
    CHECK_NEAR(low_gain("depth", 50.f), full / 2.0, 0.3);
    CHECK_NEAR(low_gain("depth", 0.f), 0.0, 1e-3);
    CHECK_NEAR(low_gain("downward", 0.f), 0.0, 1e-3);
    // Time scales the attack: slower to reach the reduction at 300 %.
    const auto attack_samples = [](float time) {
        auto r = rig();
        r.set("time", time);
        r.set("low_attack", 10.f);
        r.run_mono(Signal(4800, 0.0f));
        const auto y = r.run_mono(tone(48000, 50.0, kRate, 0.5));
        // First 20 ms window whose peak is within 1 dB of the final peak.
        const double fin = peak(y, 40000);
        for (std::size_t w = 0; w + 960 < y.size(); w += 240)
            if (peak(y, w, w + 960) < fin * 1.122) return w;
        return y.size();
    };
    CHECK(attack_samples(300.f) > attack_samples(100.f));
    // Mix 0: the dry all-pass sum, the same as with every ratio at 1:1.
    auto dry = rig();
    dry.set("mix", 0.f);
    auto ref = rig();
    no_compression(ref);
    const Signal x = noise(24000, 0.5f, 3);
    const auto a = dry.run_mono(x), b = ref.run_mono(x);
    double err = 0;
    for (std::size_t i = 0; i < a.size(); ++i) err = std::max(err, static_cast<double>(std::fabs(a[i] - b[i])));
    CHECK(err < 1e-4);
    // Band input lifts its band (below the threshold), output gain the whole.
    auto bi = rig();
    no_compression(bi);
    bi.set("mid_input", 12.f);
    CHECK_NEAR(tone_gain(bi, 800.0, 0.01), 12.0, 0.1);
    auto og = rig();
    no_compression(og);
    og.set("output_gain", -6.f);
    CHECK_NEAR(tone_gain(og, 800.0, 0.01), -6.0, 0.05);
    // Makeup.
    auto mk = rig();
    no_compression(mk);
    mk.set("high_makeup", 4.f);
    CHECK_NEAR(tone_gain(mk, 9000.0, 0.01), 4.0, 0.1);
}

TEST_CASE(multiband_depth_and_band_gain_edits_glide) {
    auto r = rig();
    Signal y = r.run_mono(tone(24000, 100.0, kRate, 0.9));
    r.set("depth", 0.f);       // removes about 10 dB of reduction
    r.set("low_makeup", 12.f); // and adds 12 dB
    y = concat(y, r.run_mono(tone(9600, 100.0, kRate, 0.9)));
    r.set("low_input", -24.f);
    y = concat(y, r.run_mono(tone(9600, 100.0, kRate, 0.9)));
    float step = 0.0f;
    for (std::size_t i = 12000; i < y.size(); ++i) step = std::max(step, std::fabs(y[i] - y[i - 1]));
    CHECK(step < 0.1f); // a 100 Hz tone at full makeup moves at most ~0.05 per sample; a gain jump would be ~2
}

TEST_CASE(multiband_input_gain_never_reaches_the_dry_path) {
    const Signal x = noise(48000, 0.2f, 7);
    auto a = rig();
    a.set("mix", 0.f);
    auto b = rig();
    b.set("mix", 0.f);
    const uint32_t gi = param_index(b.desc(), "input_gain");
    const auto ya = a.run_mono(x);
    const auto yb = b.run(x, x, nullptr, nullptr, {{10000, {gi, 12.f}}, {30000, {gi, -12.f}}}).first;
    float diff = 0.0f;
    for (std::size_t i = 0; i < x.size(); ++i) diff = std::max(diff, std::fabs(ya[i] - yb[i]));
    CHECK(diff < 1e-5f);
}

TEST_CASE(multiband_readouts_are_at_rest_in_silence) {
    auto r = rig();
    r.run_mono(Signal(4800, 0.0f));
    const auto ro = r.readouts();
    REQUIRE(ro.size() == 8);
    CHECK(ro[0] <= -119.0f);
    for (int b = 0; b < 3; ++b) {
        CHECK(ro[2 + b] <= -99.0f);
        CHECK(ro[5 + b] == 0.0f);
    }
    CHECK(r.instance().latency() == 0u);
}
