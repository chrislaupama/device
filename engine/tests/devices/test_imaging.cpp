// Utility and Stereo Widener (engine/src/devices/{utility, widener}.cpp)
// through the shared device harness.
#include "device_harness.hpp"

using namespace devtest;

namespace {

constexpr double kRate = 48000.0;

// Defaults of the saine_core definitions, registry order.
const std::vector<float> kUtility = {0.f, 0.f, 100.f, 0.f, 0.f, 0.f, 0.f, 0.f, 120.f, 0.f, 0.f, 0.f};
const std::vector<float> kWidener = {200.f, 0.f, 0.f, 120.f, 100.f};

double max_diff(const Signal& a, const Signal& b, std::size_t from = 0) {
    double e = 0;
    for (std::size_t i = from; i < std::min(a.size(), b.size()); ++i) e = std::max(e, static_cast<double>(std::fabs(a[i] - b[i])));
    return e;
}

// |H(f)| of the widener's one-pole high-pass at fc.
double one_pole_hp(double fc, double f) {
    const double pole = std::exp(-2.0 * 3.14159265358979 * fc / kRate);
    const double w = 2.0 * 3.14159265358979 * f / kRate;
    return 2.0 * std::fabs(std::sin(w / 2)) * pole / std::sqrt(1.0 - 2.0 * pole * std::cos(w) + pole * pole);
}

Signal scaled(const Signal& s, float g) {
    Signal o = s;
    for (float& v : o) v *= g;
    return o;
}

} // namespace

// ---------------------------------------------------------------- utility

TEST_CASE(utility_defaults_are_bit_transparent) {
    DeviceRig rig("utility", kUtility, kRate);
    CHECK_EQ(rig.instance().latency(), 0u);
    const Signal l = noise(9600, 0.8f, 1), r = noise(9600, 0.8f, 2);
    auto [yl, yr] = rig.run(l, r);
    CHECK(max_diff(yl, l) < 1e-6 && max_diff(yr, r) < 1e-6);
}

TEST_CASE(utility_gain_balance_and_mute) {
    DeviceRig rig("utility", kUtility, kRate);
    rig.set("gain", -6.f);
    rig.set("balance", 25.f); // half way right: left at 50 %
    const Signal x = tone(24000, 1000.0, kRate, 0.5);
    auto [yl, yr] = rig.run(x, x);
    CHECK_NEAR(db(rms(yr, 4800) / rms(x, 4800)), -6.0, 0.01);
    CHECK_NEAR(db(rms(yl, 4800) / rms(x, 4800)), -6.0 + db(0.5), 0.01);
    rig.set("mute", 1.f);
    auto [ml, mr] = rig.run(x, x);
    CHECK(peak(ml, 4800) < 1e-4 && peak(mr, 4800) < 1e-4);
    CHECK(peak(ml, 0, 10) > 0.0); // fades out, no hard cut
    auto [sl, sr] = rig.run(x, x);
    CHECK(peak(sl) == 0.0 && peak(sr) == 0.0); // then exactly silent
}

TEST_CASE(utility_channel_modes_and_polarity) {
    const Signal l = tone(9600, 300.0, kRate, 0.5), r = tone(9600, 700.0, kRate, 0.3);
    auto run = [&](float mode, float pl, float pr) {
        DeviceRig rig("utility", kUtility, kRate);
        rig.set("channel_mode", mode);
        rig.set("phase_l", pl);
        rig.set("phase_r", pr);
        return rig.run(l, r);
    };
    auto [a1, a2] = run(1, 0, 0); // Left on both
    CHECK(max_diff(a1, l) < 1e-6 && max_diff(a2, l) < 1e-6);
    auto [b1, b2] = run(2, 0, 0); // Right on both
    CHECK(max_diff(b1, r) < 1e-6 && max_diff(b2, r) < 1e-6);
    auto [c1, c2] = run(3, 0, 0); // Swap
    CHECK(max_diff(c1, r) < 1e-6 && max_diff(c2, l) < 1e-6);
    auto [d1, d2] = run(0, 1, 0); // Phase L
    CHECK(max_diff(d1, scaled(l, -1.f)) < 1e-6 && max_diff(d2, r) < 1e-6);
}

TEST_CASE(utility_width_mono_and_bass_mono) {
    const Signal l = tone(48000, 1000.0, kRate, 0.5), r = scaled(l, -1.f); // pure side
    DeviceRig mono("utility", kUtility, kRate);
    mono.set("mono", 1.f);
    auto [ml, mr] = mono.run(l, r);
    CHECK(peak(ml, 4800) < 1e-5 && peak(mr, 4800) < 1e-5);
    DeviceRig wide("utility", kUtility, kRate);
    wide.set("width", 200.f);
    auto [wl, wr] = wide.run(l, r);
    CHECK_NEAR(db(rms(wl, 4800) / rms(l, 4800)), db(2.0), 0.01);
    // Bass mono at 200 Hz: a 50 Hz side signal disappears, a 2 kHz one stays.
    DeviceRig bass("utility", kUtility, kRate);
    bass.set("bass_mono", 1.f);
    bass.set("bass_mono_freq", 200.f);
    const Signal low = tone(48000, 50.0, kRate, 0.5), high = tone(48000, 2000.0, kRate, 0.5);
    auto [bl, br] = bass.run(low, scaled(low, -1.f));
    CHECK(db(rms(bl, 24000) / rms(low, 24000)) < -20.0);
    DeviceRig bass2("utility", kUtility, kRate);
    bass2.set("bass_mono", 1.f);
    bass2.set("bass_mono_freq", 200.f);
    auto [hl, hr] = bass2.run(high, scaled(high, -1.f));
    CHECK_NEAR(db(rms(hl, 24000) / rms(high, 24000)), 0.0, 0.1);
    // A mono (mid) signal is untouched by bass mono.
    DeviceRig bass3("utility", kUtility, kRate);
    bass3.set("bass_mono", 1.f);
    auto [cl, cr] = bass3.run(low, low);
    CHECK(max_diff(cl, low, 4800) < 1e-5);
    // Audition: only the mid below the frequency.
    DeviceRig aud("utility", kUtility, kRate);
    aud.set("bass_mono_audition", 1.f);
    aud.set("bass_mono_freq", 200.f);
    auto [al, ar] = aud.run(high, high);
    CHECK(db(rms(al, 24000) / rms(high, 24000)) < -40.0);
}

TEST_CASE(utility_dc_filter_removes_offset) {
    DeviceRig rig("utility", kUtility, kRate);
    rig.set("dc_filter", 1.f);
    Signal x = tone(96000, 1000.0, kRate, 0.3);
    for (float& v : x) v += 0.2f;
    auto [yl, yr] = rig.run(x, x);
    double mean = 0;
    for (std::size_t i = 72000; i < 96000; ++i) mean += yl[i];
    CHECK(std::fabs(mean / 24000.0) < 1e-3);
}

// ---------------------------------------------------------------- widener

TEST_CASE(widener_scales_the_side_above_the_low_cut) {
    // Pure side at 10 kHz, width 200 %: doubled, times the one-pole
    // high-pass at 120 Hz (-0.07 dB there, as the panel's curve shows).
    DeviceRig rig("widener", kWidener, kRate);
    CHECK_EQ(rig.instance().latency(), 0u);
    const Signal s = tone(48000, 10000.0, kRate, 0.25);
    auto [l, r] = rig.run(s, scaled(s, -1.f));
    CHECK_NEAR(rms(l, 9600) / rms(s, 9600), 2.0 * one_pole_hp(120.0, 10000.0), 1e-3);
    // Mid untouched.
    DeviceRig mid("widener", kWidener, kRate);
    auto [ml, mr] = mid.run(s, s);
    CHECK(max_diff(ml, s, 100) < 1e-6);
    // Below the low cut the side falls away (bass stays centred).
    DeviceRig low("widener", kWidener, kRate);
    low.set("lowcut", 400.f);
    const Signal b = tone(48000, 30.0, kRate, 0.25);
    auto [bl, br] = low.run(b, scaled(b, -1.f));
    CHECK(db(rms(bl, 24000) / rms(b, 24000)) < -10.0);
}

TEST_CASE(widener_matches_the_drawn_response) {
    // Effective width at f (side out / side in, relative to the mid gain)
    // equals saine_core widener_response: 100 * w * s * |H(f)| / m at mix 100 %.
    const double fc = 120.0, f = 300.0;
    DeviceRig rig("widener", kWidener, kRate);
    rig.set("sidegain", 3.f);
    rig.set("midgain", -2.f);
    const Signal s = tone(48000, f, kRate, 0.2);
    auto [l, r] = rig.run(s, scaled(s, -1.f));
    const double expect = 2.0 * std::pow(10.0, 3.0 / 20.0) * one_pole_hp(fc, f);
    CHECK_NEAR(rms(l, 9600) / rms(s, 9600), expect, 0.005);
    DeviceRig m("widener", kWidener, kRate);
    m.set("midgain", -2.f);
    auto [ml, mr] = m.run(s, s);
    CHECK_NEAR(db(rms(ml, 9600) / rms(s, 9600)), -2.0, 0.01);
}

TEST_CASE(widener_mix_zero_and_width_zero) {
    const Signal l = noise(9600, 0.5f, 7), r = noise(9600, 0.5f, 8);
    DeviceRig dry("widener", kWidener, kRate);
    dry.set("mix", 0.f);
    auto [dl, dr] = dry.run(l, r);
    CHECK(max_diff(dl, l) < 1e-6 && max_diff(dr, r) < 1e-6);
    DeviceRig mono("widener", kWidener, kRate);
    mono.set("width2", 0.f);
    auto [ml, mr] = mono.run(l, r);
    CHECK(max_diff(ml, mr, 2000) < 1e-6);
}
