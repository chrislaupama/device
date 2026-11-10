// Channel EQ and Character EQ (engine/src/devices/channel_eq.cpp,
// character_eq.cpp, eq_sections.hpp): the section design against the panel
// model's reference points, measured band responses, switching without
// clicks, listen, dynamics, drive, tamer, warmth, valve, auto gain, bypass
// and latency.
#include "device_harness.hpp"
#include "devices/eq_sections.hpp"

#include <limits>

using namespace devtest;

namespace {

constexpr double kRate = 48000.0;

std::vector<float> channel_defaults() {
    return {20.f,   0.f, 0.71f, 0.f, 75.f,   0.f, 1.f,  1.f, 100.f,  0.f, 0.6f,  1.f, 250.f, 0.f, 0.3f,
            1.f,    1040.f, 0.f, 0.41f, 1.f, 2500.f, 0.f, 0.2f, 1.f, 7500.f, 0.f, 1.f, 1.f, 20000.f, 0.f,
            0.71f,  0.f, 0.f, 4.f, 1.f, 0.f, 0.f, 0.f, 0.f, 2.f, 5.f, 0.f, 0.f, 0.f};
}

std::vector<float> character_defaults() {
    std::vector<float> v;
    for (int b = 0; b < 12; ++b) v.insert(v.end(), {0.f, 0.f, 1000.f, 0.f, 0.7071f, 1.f, 0.f, 0.f, 0.f});
    v.insert(v.end(), {0.f, 0.f, 1.f, 300.f, 0.f, 1.f, 100.f, 0.f, 0.f, 0.f, 0.f});
    for (int b = 0; b < 12; ++b) v.insert(v.end(), {0.f, 0.f, 1.f, -30.f, 15.f, 150.f});
    v.insert(v.end(), {0.f, 50.f});
    const float tame_f[4] = {250.f, 1000.f, 4000.f, 10000.f};
    for (float f : tame_f) v.insert(v.end(), {0.f, f, 1.f, 0.f});
    for (int b = 0; b < 12; ++b) v.push_back(0.f);
    v.push_back(0.f);
    return v;
}

DeviceRig channel() { return DeviceRig("channeleq", channel_defaults(), kRate, 256); }
DeviceRig character() { return DeviceRig("character-eq", character_defaults(), kRate, 256); }

// Steady-state gain (dB) of a sine through the rig.
double gain_at(DeviceRig& r, double hz, double amp = 0.05) {
    const std::size_t n = 24000;
    const Signal x = tone(n, hz, kRate, amp);
    const Signal y = r.run_mono(x);
    return db(rms(y, n / 2) / rms(x, n / 2));
}

// Energy outside the fundamental: residual after removing the best-fit sine (dB re the output).
double distortion_db(const Signal& y, double hz, std::size_t from) {
    double s = 0.0, c = 0.0, e = 0.0;
    const std::size_t n = y.size() - from;
    for (std::size_t i = from; i < y.size(); ++i) {
        const double ph = 2.0 * 3.14159265358979323846 * hz * static_cast<double>(i) / kRate;
        s += y[i] * std::sin(ph);
        c += y[i] * std::cos(ph);
        e += static_cast<double>(y[i]) * y[i];
    }
    s *= 2.0 / static_cast<double>(n);
    c *= 2.0 / static_cast<double>(n);
    double res = 0.0;
    for (std::size_t i = from; i < y.size(); ++i) {
        const double ph = 2.0 * 3.14159265358979323846 * hz * static_cast<double>(i) / kRate;
        const double d = y[i] - (s * std::sin(ph) + c * std::cos(ph));
        res += d * d;
    }
    return 10.0 * std::log10(std::max(res, 1e-30) / std::max(e, 1e-30));
}

double design_db(const dstudio::eq::Band& b, double f) {
    const dstudio::eq::Design d = dstudio::eq::design(b, kRate);
    double m2 = 1.0;
    for (int i = 0; i < d.n; ++i) m2 *= dstudio::eq::mag2(d.s[i], f, kRate);
    return 10.0 * std::log10(m2);
}

} // namespace

TEST_CASE(eq_design_matches_the_panel_model) {
    // The same points as dstudio_core::builtin::eq::tests::engine_contract_points.
    using dstudio::eq::Band;
    using dstudio::eq::Shape;
    Band a{Shape::LowShelf, 200.f, 9.f, 0.7071f, 18.f, false, 0.5f, 0.f};
    Band b{Shape::HighPass, 1000.f, 0.f, 2.f, 36.f, false, 0.f, 0.f};
    Band c{Shape::DualLow, 60.f, 8.f, 0.7071f, 12.f, false, 0.f, 6.f};
    Band d{Shape::Bell, 2500.f, -7.5f, 1.3f, 12.f, true, 0.f, 0.f};
    CHECK_NEAR(design_db(a, 100.0), 7.843498, 2e-3);
    CHECK_NEAR(design_db(a, 330.0), -0.675584, 2e-3);
    CHECK_NEAR(design_db(b, 900.0), -0.567544, 2e-3);
    CHECK_NEAR(design_db(b, 400.0), -47.592860, 5e-3);
    CHECK_NEAR(design_db(c, 30.0), 3.508999, 2e-3);
    CHECK_NEAR(design_db(c, 120.0), -0.137253, 2e-3);
    CHECK_NEAR(design_db(d, 2500.0), -7.5, 2e-3);
    CHECK_NEAR(design_db(d, 3500.0), -2.570398, 2e-3);
}

TEST_CASE(channel_eq_defaults_are_bit_transparent) {
    auto r = channel();
    const Signal x = noise(4096, 0.5f);
    auto [l, rr] = r.run(x, x);
    bool same = true;
    for (std::size_t i = 0; i < x.size(); ++i) same = same && l[i] == x[i] && rr[i] == x[i];
    CHECK(same);
    CHECK(r.instance().latency() == 0u);
}

TEST_CASE(channel_eq_bands_measure_as_designed) {
    {
        auto r = channel();
        r.set("band3_gain2", 6.f);
        r.set("band3_q", 1.f);
        CHECK_NEAR(gain_at(r, 250.0), 6.0, 0.05);
    }
    {
        auto r = channel();
        r.set("band0_enabled", 1.f);
        r.set("band0_freq", 1000.f);
        r.set("hp_slope", 3.f); // 48 dB/oct
        const dstudio::eq::Band hp{dstudio::eq::Shape::HighPass, 1000.f, 0.f, 0.71f, 48.f, false, 0.f, 0.f};
        CHECK_NEAR(gain_at(r, 1000.0), design_db(hp, 1000.0), 0.05);
        CHECK_NEAR(design_db(hp, 1000.0), -3.0, 0.1);
        CHECK_NEAR(gain_at(r, 500.0), -48.0, 1.5);
        CHECK_NEAR(gain_at(r, 8000.0), 0.0, 0.05);
    }
    {
        // A shelf at a measured point equals the design (the panel's curve).
        auto r = channel();
        r.set("band6_gain2", -9.f);
        r.set("band6_freq", 3000.f);
        dstudio::eq::Band b{dstudio::eq::Shape::HighShelf, 3000.f, -9.f, 1.f, 12.f, false, 0.f, 0.f};
        for (double f : {1000.0, 3000.0, 9000.0}) CHECK_NEAR(gain_at(r, f), design_db(b, f), 0.05);
    }
    {
        auto r = channel();
        r.set("output_gain", -6.f);
        CHECK_NEAR(gain_at(r, 1000.0), -6.0, 0.01);
    }
}

TEST_CASE(channel_eq_type_and_slope_follow_their_params) {
    auto r = channel();
    r.set("band4_type", 3.f); // Notch at 1040 Hz
    r.set("band4_q", 4.f);
    CHECK(gain_at(r, 1040.0) < -40.0);
    auto lp = channel();
    lp.set("band7_enabled", 1.f);
    lp.set("band7_freq", 1000.f);
    lp.set("lp_slope", 1.f); // 24 dB/oct
    CHECK_NEAR(gain_at(lp, 2000.0), -24.0, 1.5);
}

TEST_CASE(channel_eq_switching_a_band_does_not_click) {
    auto r = channel();
    r.set("band4_gain2", 18.f);
    r.set("band4_enabled", 0.f);
    const std::size_t n = 19200;
    const Signal x = tone(n, 1040.0, kRate, 0.05);
    const Signal a(x.begin(), x.begin() + 4800), b(x.begin() + 4800, x.end());
    Signal y = r.run_mono(a);
    r.set("band4_enabled", 1.f);
    const Signal y2 = r.run_mono(b);
    y.insert(y.end(), y2.begin(), y2.end());
    double jump = 0.0;
    for (std::size_t i = 4790; i < 6000; ++i) jump = std::max(jump, static_cast<double>(std::fabs(y[i] - y[i - 1])));
    // Steepest a 1040 Hz sine at the boosted level can move per sample.
    const double bound = 2.0 * 3.14159265 * 1040.0 / kRate * 0.05 * std::pow(10.0, 18.0 / 20.0);
    CHECK(jump < 1.1 * bound);
    CHECK_NEAR(db(rms(y, 14400) / rms(x, 14400)), 18.0, 0.05);
}

TEST_CASE(channel_eq_listen_solos_the_band_region) {
    auto r = channel();
    r.set("listen", 5.f); // band 4 (index 4, 1040 Hz)
    CHECK_NEAR(gain_at(r, 1040.0), 0.0, 0.1);
    auto r2 = channel();
    r2.set("listen", 5.f);
    CHECK(gain_at(r2, 60.0) < -12.0);
}

TEST_CASE(character_eq_defaults_are_a_clean_delay) {
    auto r = character();
    CHECK(r.instance().latency() == 27u);
    const auto y = r.run_mono(impulse(512, 1.f, 10));
    CHECK(y[37] == 1.f);
    double other = 0.0;
    for (std::size_t i = 0; i < y.size(); ++i)
        if (i != 37) other = std::max(other, static_cast<double>(std::fabs(y[i])));
    CHECK(other == 0.0);
}

TEST_CASE(character_eq_bands_match_the_design) {
    struct Case {
        float type, freq, gain, q, slope, propq, overshoot, atten;
        dstudio::eq::Band band;
        double at[2];
    };
    using dstudio::eq::Shape;
    const Case cases[] = {
        {1.f, 200.f, 9.f, 0.7071f, 2.f, 0.f, 50.f, 0.f, {Shape::LowShelf, 200.f, 9.f, 0.7071f, 18.f, false, 0.5f, 0.f}, {100.0, 330.0}},
        {3.f, 1000.f, 0.f, 2.f, 4.f, 0.f, 0.f, 0.f, {Shape::HighPass, 1000.f, 0.f, 2.f, 36.f, false, 0.f, 0.f}, {900.0, 1500.0}},
        {6.f, 60.f, 8.f, 0.7071f, 1.f, 0.f, 0.f, 6.f, {Shape::DualLow, 60.f, 8.f, 0.7071f, 12.f, false, 0.f, 6.f}, {30.0, 120.0}},
        {0.f, 2500.f, -7.5f, 1.3f, 1.f, 1.f, 0.f, 0.f, {Shape::Bell, 2500.f, -7.5f, 1.3f, 12.f, true, 0.f, 0.f}, {2500.0, 3500.0}},
    };
    for (const Case& c : cases) {
        for (double f : c.at) {
            auto r = character();
            r.set("band5_enabled", 1.f);
            r.set("band5_type", c.type);
            r.set("band5_freq", c.freq);
            r.set("band5_gain", c.gain);
            r.set("band5_q", c.q);
            r.set("band5_slope", c.slope);
            r.set("band5_propq", c.propq);
            r.set("band5_overshoot", c.overshoot);
            r.set("band5_low_atten", c.atten);
            CHECK_NEAR(gain_at(r, f), design_db(c.band, f), 0.05);
        }
    }
}

TEST_CASE(character_eq_bank_voices_the_bands) {
    auto free_bank = character();
    free_bank.set("band1_enabled", 1.f);
    free_bank.set("band1_gain", 9.f);
    free_bank.set("band1_q", 2.f);
    const double narrow = gain_at(free_bank, 1500.0);
    auto broad = character();
    broad.set("band1_enabled", 1.f);
    broad.set("band1_gain", 9.f);
    broad.set("band1_q", 2.f);
    broad.set("bank", 9.f); // Passive Mid: broader (Q x 0.7)
    CHECK(gain_at(broad, 1500.0) > narrow + 0.5);
}

TEST_CASE(character_eq_dynamic_band_reacts_to_its_region) {
    auto r = character();
    r.set("band2_enabled", 1.f);
    r.set("band2_freq", 2000.f);
    r.set("band2_q", 1.f);
    r.set("band2_dyn", 1.f);
    r.set("band2_dyn_range", -9.f);
    r.set("band2_dyn_auto", 0.f);
    r.set("band2_dyn_thresh", -40.f);
    r.set("band2_dyn_attack", 1.f);
    const double loud = gain_at(r, 2000.0, 0.5); // ~-9 dBFS: far above the threshold
    CHECK_NEAR(loud, -9.0, 0.3);
    CHECK(r.readouts()[1] < -8.5f);
    auto q = character();
    q.set("band2_enabled", 1.f);
    q.set("band2_freq", 2000.f);
    q.set("band2_dyn", 1.f);
    q.set("band2_dyn_range", -9.f);
    q.set("band2_dyn_auto", 0.f);
    q.set("band2_dyn_thresh", -20.f);
    CHECK_NEAR(gain_at(q, 2000.0, 0.005), 0.0, 0.1); // -49 dBFS: below the threshold
}

TEST_CASE(character_eq_drive_adds_harmonics_only_when_set) {
    for (float drive : {0.f, 100.f}) {
        auto r = character();
        r.set("band1_enabled", 1.f);
        r.set("band1_freq", 300.f);
        r.set("band1_gain", 12.f);
        r.set("band1_drive", drive);
        const Signal y = r.run_mono(tone(24000, 300.0, kRate, 0.25));
        const double thd = distortion_db(y, 300.0, 12000);
        if (drive == 0.f) CHECK(thd < -90.0);
        else CHECK(thd > -40.0);
    }
}

TEST_CASE(character_eq_drive_recovers_from_a_non_finite_last_sample) {
    auto r = character();
    r.set("band1_enabled", 1.f);
    r.set("band1_freq", 300.f);
    r.set("band1_gain", 12.f);
    r.set("band1_drive", 100.f);
    Signal bad = tone(256, 300.0, kRate, 0.25);
    bad.back() = std::numeric_limits<float>::quiet_NaN();
    r.run_mono(bad);
    // The latency delay passes the one NaN through on the dry path; the drive must not spread it.
    const Signal y = r.run_mono(tone(4096, 300.0, kRate, 0.25));
    CHECK(std::count_if(y.begin(), y.end(), [](float v) { return !std::isfinite(v); }) <= 1);
}

TEST_CASE(character_eq_valve_warmth_and_mix) {
    for (float os : {0.f, 1.f}) {
        auto r = character();
        r.set("valve_enabled", 1.f);
        r.set("valve_trim", 2.f);
        r.set("oversampling", os);
        const Signal y = r.run_mono(tone(24000, 1000.0, kRate, 0.5));
        CHECK(distortion_db(y, 1000.0, 12000) > -45.0);
        CHECK(all_finite(y));
    }
    {
        // Small signals pass the oversampled path at unity and on time.
        for (float os : {0.f, 1.f}) {
            auto r = character();
            r.set("valve_enabled", 1.f);
            r.set("valve_trim", 0.f);
            r.set("oversampling", os);
            const auto y = r.run_mono(impulse(4096, 0.01f, 2000));
            std::size_t at = 0;
            for (std::size_t i = 0; i < y.size(); ++i)
                if (std::fabs(y[i]) > std::fabs(y[at])) at = i;
            CHECK(at == 2027u);
            CHECK_NEAR(gain_at(r, 1000.0), 0.0, 0.05);
        }
    }
    {
        auto r = character();
        r.set("valve_enabled", 1.f);
        r.set("valve_trim", 2.f);
        r.set("valve_mix", 0.f);
        CHECK(distortion_db(r.run_mono(tone(24000, 1000.0, kRate, 0.5)), 1000.0, 12000) < -90.0);
    }
    {
        auto lo = character();
        lo.set("warmth_amount", 100.f);
        CHECK(distortion_db(lo.run_mono(tone(24000, 80.0, kRate, 0.7)), 80.0, 12000) > -50.0);
        auto hi = character();
        hi.set("warmth_amount", 100.f);
        CHECK(distortion_db(hi.run_mono(tone(24000, 5000.0, kRate, 0.7)), 5000.0, 12000) < -60.0);
    }
}

TEST_CASE(character_eq_auto_gain_cancels_the_average_boost) {
    auto r = character();
    r.set("band1_enabled", 1.f);
    r.set("band1_type", 2.f); // High Shelf +12 dB at 1 kHz
    r.set("band1_gain", 12.f);
    const double plain = gain_at(r, 100.0);
    auto a = character();
    a.set("band1_enabled", 1.f);
    a.set("band1_type", 2.f);
    a.set("band1_gain", 12.f);
    a.set("auto_gain", 1.f);
    const double leveled = gain_at(a, 100.0);
    CHECK(plain > -0.1);
    CHECK(leveled < -3.0 && leveled > -12.0);
}

TEST_CASE(character_eq_tamer_pulls_down_a_resonance) {
    auto r = character();
    r.set("tame_enabled", 1.f);
    r.set("tame_amount", 100.f);
    r.set("tame2_enabled", 1.f);
    r.set("tame2_freq", 1000.f);
    r.set("tame2_q", 4.f);
    const std::size_t n = 48000;
    Signal x = noise(n, 0.05f, 7);
    const Signal t = tone(n, 1000.0, kRate, 0.2);
    for (std::size_t i = 0; i < n; ++i) x[i] += t[i];
    const Signal y = r.run_mono(x);
    CHECK(r.readouts()[13] < -3.0f);
    CHECK(all_finite(y));
    auto flat = character();
    flat.set("tame_enabled", 1.f);
    flat.set("tame_amount", 100.f);
    flat.set("tame2_enabled", 1.f);
    flat.set("tame2_freq", 1000.f);
    flat.run_mono(noise(n, 0.1f, 9));
    CHECK(flat.readouts()[13] > -1.5f);

    // Opposite polarity in the two channels: each channel still has the resonance.
    auto anti = character();
    anti.set("tame_enabled", 1.f);
    anti.set("tame_amount", 100.f);
    anti.set("tame2_enabled", 1.f);
    anti.set("tame2_freq", 1000.f);
    anti.set("tame2_q", 4.f);
    Signal neg(n);
    for (std::size_t i = 0; i < n; ++i) neg[i] = -x[i];
    anti.run(x, neg);
    CHECK(anti.readouts()[13] < -3.0f);
}

TEST_CASE(character_eq_shape_and_oversampling_switches_stay_bounded) {
    // A slope / shape change and a 2x <-> 4x switch while running start the
    // new design from rest: the output stays near the input level.
    auto r = character();
    r.set("band1_enabled", 1.f);
    r.set("band1_type", 3.f); // high pass
    r.set("band1_freq", 200.f);
    r.set("band1_slope", 5.f); // 48 dB/oct
    r.set("valve_enabled", 1.f);
    const Signal x = noise(4800, 0.3f, 3);
    float peak = 0.f;
    for (int k = 0; k < 12; ++k) {
        r.set("band1_slope", (k % 2) ? 0.f : 5.f);
        r.set("band1_type", (k % 3) == 2 ? 6.f : 3.f);
        r.set("oversampling", (k % 2) ? 1.f : 0.f);
        const Signal y = r.run_mono(x);
        CHECK(all_finite(y));
        for (float v : y) peak = std::max(peak, std::fabs(v));
    }
    CHECK(peak < 4.0f);
}

TEST_CASE(character_eq_listen_and_bypass) {
    auto r = character();
    r.set("band3_enabled", 1.f);
    r.set("band3_freq", 4000.f);
    r.set("band3_gain", 12.f);
    r.set("listen_band", 3.f);
    CHECK_NEAR(gain_at(r, 4000.0), 0.0, 0.1);
    auto b = character();
    b.set("band3_enabled", 1.f);
    b.set("band3_gain", 12.f);
    b.set("valve_enabled", 1.f);
    b.set("bypass", 1.f);
    const auto y = b.run_mono(impulse(256, 1.f, 3));
    CHECK(y[30] == 1.f);
}
