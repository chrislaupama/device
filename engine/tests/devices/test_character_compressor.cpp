// Character Compressor (engine/src/devices/character_compressor.cpp): the six
// model curves (values from saine_core::builtin::character_transfer),
// timing, Range, Hold, Adapt, lookahead and oversampling latency, saturation
// and aliasing, stereo link, M/S, sidechain and its filters, listen / delta /
// bypass, mix up to 200 %, auto gain, the level rider and readouts.
#include "device_harness.hpp"

using namespace devtest;

namespace {

// Defaults of saine_core::builtin::character_compressor, registry order.
const std::vector<float> kDefaults = {
    0.f,  -18.f, 4.f, 6.f,  10.f, 200.f, 0.f, 0.f,   24.f, 1.f,    100.f, 0.f, 0.f, 0.f, 0.f, 100.f, //
    0.f,  0.f,   0.f, 80.f, 1000.f, 0.f, 1.f, 0.f,   0.f,  0.f,    50.f,  8.f, 0.f,                  //
    0.f,  150.f, 1.f, 0.f,  0.f,  600.f, 1.f, 0.f,   0.f,  2500.f, 1.f,   0.f, 0.f, 8000.f, 1.f, 0.f, //
    0.f};
constexpr double kRate = 48000.0;

DeviceRig rig() { return DeviceRig("character-compressor", kDefaults, kRate, 256); }
Signal dc(std::size_t n, float v) { return Signal(n, v); }
double settled_gain_db(const Signal& out, float in) { return db(std::fabs(out.back()) / in); }
float from_db(double d) { return static_cast<float>(std::pow(10.0, d / 20.0)); }

// Steady gain (dB) of a model for a DC level, drive off, fast attack.
double model_gain(int model, double level_db, float ratio, float knee = 0.f, float range = 24.f) {
    auto r = rig();
    r.set("model", static_cast<float>(model));
    r.set("ratio", ratio);
    r.set("knee", knee);
    r.set("range", range);
    r.set("drive", 0.f);
    r.set("attack", 0.05f);
    const float x = from_db(level_db);
    return settled_gain_db(r.run_mono(dc(96000, x)), x);
}

// Amplitude of frequency f in x[from..] (single-bin DFT).
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

} // namespace

TEST_CASE(character_clean_below_threshold_is_bit_transparent_with_no_latency) {
    auto r = rig();
    const Signal x = tone(9600, 440.0, kRate, 0.01);
    auto [l, rr] = r.run(x, x);
    bool same = true;
    for (std::size_t i = 0; i < x.size(); ++i) same = same && l[i] == x[i] && rr[i] == x[i];
    CHECK(same);
    CHECK(r.instance().latency() == 0u);
}

TEST_CASE(character_model_curves_match_the_core_laws) {
    // Clean: soft knee, ratio 4, -6 dB in, threshold -18: 12 dB over -> -9 dB.
    CHECK_NEAR(model_gain(0, -6.0, 4.f), -9.0, 0.05);
    CHECK_NEAR(model_gain(0, -18.0, 4.f, 12.f), -0.75 * 36.0 / 24.0, 0.05); // at the threshold, 12 dB knee
    // Bus VCA: 4 dB knee, threshold lowered by 3 (10 - R) / 8 = 2.25 dB at 4:1.
    CHECK_NEAR(model_gain(2, -6.0, 4.f), -0.75 * (12.0 + 2.25), 0.05);
    // FET below "all": 4 dB knee.
    CHECK_NEAR(model_gain(3, -6.0, 8.f), -0.875 * 12.0, 0.05);
    // Opto: snaps to 3:1 (Comp) below 5.5, 8:1 (Limit) above.
    CHECK_NEAR(model_gain(4, -6.0, 4.f), -(2.0 / 3.0) * 12.0, 0.05);
    CHECK_NEAR(model_gain(4, -6.0, 20.f), -0.875 * 12.0, 0.05);
    // RMS VCA: knee 10 + 10 (R - 1) / 9 = 13.33 dB at 4:1, 12 dB over is inside it... use 20 dB over.
    CHECK_NEAR(model_gain(5, 2.0, 4.f), -0.75 * 20.0, 0.1);
    // Vari-Mu: integrated variable-ratio law (values from character_transfer).
    CHECK_NEAR(model_gain(1, -6.0, 4.f), -9.4286, 0.1);
    CHECK_NEAR(model_gain(1, -14.0, 4.f), -2.6659, 0.1);
    CHECK_NEAR(model_gain(1, -12.0, 10.f), -4.5059, 0.1);
    CHECK_NEAR(model_gain(1, 0.0, 2.f), -15.145, 0.15);
    // Range caps every model.
    CHECK_NEAR(model_gain(0, 0.0, 30.f, 0.f, 6.f), -6.0, 0.05);
    CHECK_NEAR(model_gain(3, 0.0, 30.f, 0.f, 3.f), -3.0, 0.05);
}

TEST_CASE(character_attack_release_and_hold) {
    auto r = rig();
    r.set("knee", 0.f);
    r.set("attack", 10.f);
    r.set("release", 100.f);
    r.run_mono(dc(4800, 0.001f));
    const auto up = r.run_mono(dc(48000, 0.5f));
    const double target = (db(0.5) + 18.0) * (0.25 - 1.0);
    std::size_t hit = 0;
    for (std::size_t i = 0; i < up.size(); ++i)
        if (db(std::fabs(up[i]) / 0.5) <= 0.632 * target) {
            hit = i;
            break;
        }
    CHECK_NEAR(static_cast<double>(hit), 0.010 * kRate, 0.1 * 0.010 * kRate);
    const auto release_at = [](float hold) {
        auto h = rig();
        h.set("knee", 0.f);
        h.set("attack", 0.1f);
        h.set("release", 50.f);
        h.set("hold", hold);
        h.run_mono(dc(24000, 0.5f));
        const auto down = h.run_mono(dc(48000, 0.001f));
        for (std::size_t i = 0; i < down.size(); ++i)
            if (db(std::fabs(down[i]) / 0.001) >= -1.0) return i;
        return down.size();
    };
    const std::size_t no_hold = release_at(0.f), held = release_at(200.f);
    CHECK_NEAR(static_cast<double>(held - no_hold), 0.200 * kRate, 0.01 * kRate);
}

TEST_CASE(character_adapt_and_opto_release_depend_on_the_program) {
    const auto recovery = [](int model, float adapt, std::size_t loud) {
        auto r = rig();
        r.set("model", static_cast<float>(model));
        r.set("drive", 0.f);
        r.set("threshold", -40.f);
        r.set("ratio", 8.f);
        r.set("knee", 0.f);
        r.set("attack", 1.f);
        r.set("release", 100.f);
        r.set("adapt", adapt);
        r.run_mono(dc(loud, 0.5f));
        const auto down = r.run_mono(dc(8 * 48000, 0.001f));
        // Skip the oversampling delay (the loud signal still coming out).
        for (std::size_t i = r.instance().latency() + 1; i < down.size(); ++i)
            if (db(std::fabs(down[i]) / 0.001) >= -1.0) return i;
        return down.size();
    };
    CHECK(recovery(0, 1.f, 3 * 48000) > recovery(0, 0.f, 3 * 48000) * 2);  // sustained reduction -> slower
    CHECK(recovery(0, 1.f, 3 * 48000) > recovery(0, 1.f, 2400));            // ... than after a short burst
    CHECK(recovery(4, 0.f, 5 * 48000) > recovery(4, 0.f, 2400) * 3 / 2);    // Opto memory
    CHECK(recovery(2, 1.f, 3 * 48000) > recovery(2, 0.f, 3 * 48000));       // Bus VCA auto release
}

TEST_CASE(character_lookahead_and_oversampling_report_latency) {
    auto la = rig();
    la.set("lookahead", 5.f);
    la.set("knee", 0.f);
    la.set("attack", 0.5f);
    la.run_mono(dc(256, 0.0f));
    CHECK(la.instance().latency() == 240u);
    // A step: the delayed output is already reduced when the step arrives.
    const auto out = la.run_mono(concat(dc(1000, 0.0f), dc(4000, 0.5f)));
    CHECK(std::fabs(out[1000 + 240 + 2]) < 0.4f);
    // Non-Clean models oversample: 31 samples (2x), 36 (4x); an impulse in
    // the linear region comes out exactly that late.
    for (float os : {0.f, 1.f}) {
        auto r = rig();
        r.set("model", 3.f);
        r.set("oversampling", os);
        r.set("threshold", 0.f);
        const auto y = r.run_mono(impulse(512, 0.01f, 100));
        const uint32_t lat = r.instance().latency();
        CHECK(lat == (os > 0.5f ? 36u : 31u));
        std::size_t at = 0;
        for (std::size_t i = 0; i < y.size(); ++i)
            if (std::fabs(y[i]) > std::fabs(y[at])) at = i;
        CHECK(at == 100 + lat);
        CHECK_NEAR(y[at], 0.01, 0.0015);
    }
}

TEST_CASE(character_drive_saturates_without_aliasing) {
    // Harmonics grow with Drive (Vari-Mu, quiet enough not to compress).
    const auto h2 = [](float drive) {
        auto r = rig();
        r.set("model", 1.f);
        r.set("threshold", 0.f);
        r.set("drive", drive);
        const auto y = r.run_mono(tone(48000, 1000.0, kRate, 0.7));
        return tone_amp(y, 2000.0, 8000) / tone_amp(y, 1000.0, 8000);
    };
    CHECK(h2(0.f) < 1e-4);
    CHECK(h2(2.f) > 10.0 * h2(0.5f));
    CHECK(h2(2.f) > 3e-3);
    // FET at full drive on a 15 kHz tone: the 3rd harmonic (45 kHz) would
    // fold to 3 kHz without oversampling; it stays far below the tone.
    for (float os : {0.f, 1.f}) {
        auto r = rig();
        r.set("model", 3.f);
        r.set("threshold", 0.f);
        r.set("drive", 2.f);
        r.set("oversampling", os);
        const auto y = r.run_mono(tone(48000, 15000.0, kRate, 0.9));
        CHECK(db(tone_amp(y, 3000.0, 8000) / tone_amp(y, 15000.0, 8000)) < -70.0);
    }
}

TEST_CASE(character_stereo_link_and_mid_side) {
    const Signal loud = dc(48000, 0.5f), quiet = dc(48000, 0.01f);
    for (float link : {0.f, 100.f}) {
        auto r = rig();
        r.set("knee", 0.f);
        r.set("attack", 0.1f);
        r.set("stereo_link", link);
        const auto [l, rr] = r.run(loud, quiet);
        const double right = settled_gain_db(rr, 0.01f);
        if (link > 50.f) CHECK_NEAR(right, settled_gain_db(l, 0.5f), 0.05); // shares the left's reduction
        else CHECK_NEAR(right, 0.0, 1e-3);
    }
    // M/S, unlinked: a loud centre is compressed, the side component is not.
    auto ms = rig();
    ms.set("knee", 0.f);
    ms.set("attack", 0.1f);
    ms.set("ms_mode", 1.f);
    ms.set("stereo_link", 0.f);
    const auto [l, rr] = ms.run(dc(48000, 0.51f), dc(48000, 0.49f)); // mid 0.5, side 0.01
    const double side = (l.back() - rr.back()) / 2.0, mid = (l.back() + rr.back()) / 2.0;
    CHECK_NEAR(side, 0.01, 1e-4);
    CHECK(mid < 0.2);
}

TEST_CASE(character_sidechain_filters_listen_delta_bypass) {
    const Signal main = dc(48000, 0.01f), side = dc(48000, 0.5f);
    auto ext = rig();
    ext.set("knee", 0.f);
    ext.set("attack", 0.1f);
    ext.set("sc_source", 1.f);
    CHECK_NEAR(settled_gain_db(ext.run(main, main, &side, &side).first, 0.01f), (db(0.5) + 18.0) * -0.75, 0.05);
    // SC HPF at 200 Hz ignores a loud 40 Hz signal in the detector.
    const auto hpf_gain = [](float on) {
        auto r = rig();
        r.set("knee", 0.f);
        r.set("sc_hpf_on", on);
        r.set("sc_hpf_freq", 400.f);
        const auto y = r.run_mono(tone(48000, 40.0, kRate, 0.5));
        return db(peak(y, 36000) / 0.5);
    };
    CHECK(hpf_gain(0.f) < -6.0);
    CHECK(hpf_gain(1.f) > -2.0);
    // SC EQ boost makes the detector hotter at its frequency.
    auto eq = rig();
    eq.set("sc_eq_gain", 12.f);
    eq.set("sc_eq_freq", 1000.f);
    eq.set("knee", 0.f);
    auto flat = rig();
    flat.set("knee", 0.f);
    const Signal t = tone(48000, 1000.0, kRate, 0.2);
    CHECK(peak(eq.run_mono(t), 36000) < 0.8 * peak(flat.run_mono(t), 36000));
    // Listen plays the detector signal; delta plays what was removed; bypass the dry.
    auto li = rig();
    li.set("sc_source", 1.f);
    li.set("sc_listen", 1.f);
    CHECK_NEAR(li.run(main, main, &side, &side).first.back(), 0.5, 1e-6);
    auto de = rig();
    de.set("knee", 0.f);
    de.set("attack", 0.1f);
    de.set("delta_monitor", 1.f);
    const double g = std::pow(10.0, (db(0.5) + 18.0) * -0.75 / 20.0);
    CHECK_NEAR(de.run_mono(dc(48000, 0.5f)).back(), 0.5 * (1.0 - g), 1e-3);
    auto by = rig();
    by.set("bypass", 1.f);
    by.set("threshold", -60.f);
    CHECK(by.run_mono(dc(4800, 0.5f)).back() == 0.5f);
}

TEST_CASE(character_mix_makeup_and_auto_gain) {
    const double g = std::pow(10.0, (db(0.5) + 18.0) * -0.75 / 20.0);
    for (float mix : {0.f, 50.f, 200.f}) {
        auto r = rig();
        r.set("knee", 0.f);
        r.set("attack", 0.1f);
        r.set("mix", mix);
        const double m = mix / 100.0;
        CHECK_NEAR(r.run_mono(dc(48000, 0.5f)).back(), 0.5 + m * (0.5 * g - 0.5), 1e-3);
    }
    auto mk = rig();
    mk.set("makeup", 6.f);
    CHECK_NEAR(settled_gain_db(mk.run_mono(dc(9600, 0.01f)), 0.01f), 6.0, 0.01);
    auto ag = rig();
    ag.set("knee", 0.f);
    ag.set("auto_gain", 1.f);
    // -0.5 x curve(0 dBFS) = -0.5 x (18 x -0.75) = +6.75 dB below the threshold.
    CHECK_NEAR(settled_gain_db(ag.run_mono(dc(9600, 0.01f)), 0.01f), 6.75, 0.02);
}

TEST_CASE(character_level_rider_moves_long_term_level_towards_threshold) {
    auto r = rig();
    r.set("ratio", 1.f);
    r.set("threshold", -18.f);
    r.set("level_enabled", 1.f);
    r.set("level_amount", 100.f);
    r.set("level_response", 1.f);
    const auto y = r.run_mono(noise(10 * 48000, 0.055f, 4)); // about -30 dB RMS
    const double in_rms = db(rms(noise(10 * 48000, 0.055f, 4), 9 * 48000));
    const double out_rms = db(rms(y, 9 * 48000));
    CHECK(out_rms - in_rms > 9.0);  // raised by most of the 12 dB distance
    CHECK(out_rms - in_rms < 12.1); // never past the rider's +12 dB
    CHECK(r.readouts()[3] > 9.0f);
    // Silence does not pull the rider up.
    auto s = rig();
    s.set("level_enabled", 1.f);
    s.set("level_response", 1.f);
    s.run_mono(dc(5 * 48000, 0.0f));
    CHECK(std::fabs(s.readouts()[3]) < 0.01f);
}

TEST_CASE(character_readouts) {
    auto r = rig();
    r.set("knee", 0.f);
    r.set("attack", 0.1f);
    r.run_mono(dc(48000, 0.5f));
    const auto ro = r.readouts();
    REQUIRE(ro.size() == 4);
    CHECK_NEAR(ro[0], db(0.5), 0.01);
    CHECK_NEAR(ro[2], (db(0.5) + 18.0) * -0.75, 0.05);
    CHECK_NEAR(ro[1], ro[0] + ro[2], 0.05);
    r.run_mono(dc(48000, 0.0f));
    CHECK(r.readouts()[0] <= -119.0f);
}
