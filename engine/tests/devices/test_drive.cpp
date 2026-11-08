// Distortion-family devices (engine/src/devices/{distortion, tape_saturation,
// soft_clipper, bitcrusher}.cpp) and the shared 4x oversampler, through the
// shared device harness.
#include "device_harness.hpp"
#include "devices/oversample.hpp"

using namespace devtest;

namespace {

constexpr double kRate = 48000.0;
constexpr double kPi = 3.14159265358979323846;
constexpr std::size_t kLat = saine::dsp::Oversampler4x::kLatency;

// Defaults of the saine_core definitions, registry order.
const std::vector<float> kDistortion = {30.f, 4000.f, 100.f, 0.f, 0.f, 0.f, 0.f};
const std::vector<float> kTape = {30.f, 40.f, 30.f, -60.f, 100.f, 0.f, 0.f};
const std::vector<float> kClipper = {0.f, 0.f, 50.f, 0.f, 0.f, 100.f, 0.f};
const std::vector<float> kCrusher = {16.f, 1.f, 100.f, 0.f, 0.f};

// Power of the DFT bin at hz over s[from, from + n) (n samples, coherent).
double bin_power(const Signal& s, std::size_t from, std::size_t n, double hz) {
    double re = 0, im = 0;
    for (std::size_t i = 0; i < n; ++i) {
        const double ph = 2.0 * kPi * hz * static_cast<double>(i) / kRate;
        re += s[from + i] * std::cos(ph);
        im -= s[from + i] * std::sin(ph);
    }
    const double a = 2.0 * std::sqrt(re * re + im * im) / static_cast<double>(n);
    return 0.5 * a * a;
}

// Power that is neither DC nor a harmonic of f0 (aliasing and noise), in dB
// relative to the fundamental. f0 in whole Hz so the 1 s window is coherent
// (and not a divisor of 192 kHz, or the aliases land on harmonic bins).
double inharmonic_db(const Signal& s, double f0) {
    const std::size_t from = 9600, n = 48000;
    double total = 0, dc = 0;
    for (std::size_t i = 0; i < n; ++i) {
        total += static_cast<double>(s[from + i]) * s[from + i];
        dc += s[from + i];
    }
    total /= static_cast<double>(n);
    dc /= static_cast<double>(n);
    double harm = 0;
    for (double f = f0; f < kRate / 2; f += f0) harm += bin_power(s, from, n, f);
    const double fund = bin_power(s, from, n, f0);
    return 10.0 * std::log10(std::max(1e-30, total - dc * dc - harm) / fund);
}

double distortion_model(double x, double drive, double bias, int mode) {
    const double g = 1.0 + 10.0 * drive / 100.0, b = bias / 100.0, o = 0.5 * b;
    auto f = [&](double v) {
        switch (mode) {
        case 1: return v >= 0 ? std::tanh(2 * v) : std::atan(3 * v) / (kPi / 2);
        case 2: return v / (1 + std::fabs(v)) - 0.5 * std::tanh(2 * b) * v * v;
        case 3: return std::clamp(v, -1.0, 1.0);
        default: return std::tanh(v);
        }
    };
    return f(x * g + o) - f(o);
}

} // namespace

// ---------------------------------------------------------------- oversampler

TEST_CASE(oversampler_round_trip_is_a_pure_delay) {
    saine::dsp::Oversampler4x os;
    os.prepare(256);
    const Signal x = tone(9600, 1000.0, kRate, 0.8);
    Signal l = x, r = x;
    for (std::size_t pos = 0; pos < x.size(); pos += 256) {
        os.up(l.data() + pos, r.data() + pos, 256);
        os.down(l.data() + pos, r.data() + pos, 256);
    }
    double err = 0;
    for (std::size_t i = 1000; i < x.size(); ++i) err = std::max(err, static_cast<double>(std::fabs(l[i] - x[i - kLat])));
    CHECK(err < 2e-4);
    // Near the top of the band: still flat (< 0.1 dB at 18 kHz).
    const Signal hi = tone(48000, 18000.0, kRate, 0.5);
    Signal hl = hi, hr = hi;
    for (std::size_t pos = 0; pos < hi.size(); pos += 256) {
        os.up(hl.data() + pos, hr.data() + pos, 256);
        os.down(hl.data() + pos, hr.data() + pos, 256);
    }
    CHECK_NEAR(db(rms(hl, 9600) / rms(hi, 9600)), 0.0, 0.1);
}

// ---------------------------------------------------------------- distortion

TEST_CASE(distortion_follows_the_drawn_curve_and_reports_latency) {
    // The low harmonics of a 100 Hz tone match the panel's static model
    // (the model is not band-limited, so only harmonics far below Nyquist
    // are compared; the 10 Hz DC blocker and 20 kHz low-pass barely matter).
    for (int mode = 0; mode < 4; ++mode) {
        DeviceRig rig("distortion", kDistortion, kRate);
        CHECK_EQ(rig.instance().latency(), kLat);
        rig.set("mode", static_cast<float>(mode));
        rig.set("tone", 20000.f);
        rig.set("bias", 40.f);
        const Signal x = tone(60000, 100.0, kRate, 0.6);
        const Signal y = rig.run_mono(x);
        Signal model(x.size());
        for (std::size_t i = 0; i < x.size(); ++i) model[i] = static_cast<float>(distortion_model(x[i], 30, 40, mode));
        for (double h : {100.0, 200.0, 300.0, 500.0}) {
            const double want = bin_power(model, 9600, 48000, h), got = bin_power(y, 9600, 48000, h);
            if (want < 1e-6 * bin_power(model, 9600, 48000, 100.0)) continue;
            const double d = 10.0 * std::log10(got / want);
            if (std::fabs(d) > 0.3) std::fprintf(stderr, "distortion mode %d harmonic %.0f Hz: %+.2f dB vs model\n", mode, h, d);
            CHECK(std::fabs(d) < 0.3);
        }
    }
}

TEST_CASE(distortion_mix_zero_is_the_delayed_dry_signal) {
    DeviceRig rig("distortion", kDistortion, kRate);
    rig.set("mix", 0.f);
    rig.set("drive", 100.f);
    const Signal x = noise(9600, 0.7f, 3);
    const Signal y = rig.run_mono(x);
    double err = 0;
    for (std::size_t i = kLat; i < x.size(); ++i) err = std::max(err, static_cast<double>(std::fabs(y[i] - x[i - kLat])));
    CHECK(err == 0.0);
}

// Worst case for aliasing: a 7 kHz tone driven as hard as each device goes.
// Everything that is not a harmonic (aliases, noise) relative to the
// fundamental; printed for the PR's quality table.
TEST_CASE(drive_family_keeps_aliasing_down) {
    const char* names[] = {"Overdrive", "Fuzz", "Tube", "Hard Clip"};
    const double dist_max[] = {-80.0, -55.0, -60.0, -55.0};
    for (int m = 0; m < 4; ++m) {
        DeviceRig rig("distortion", kDistortion, kRate);
        rig.set("drive", 100.f);
        rig.set("tone", 20000.f);
        rig.set("mode", static_cast<float>(m));
        const double a = inharmonic_db(rig.run_mono(tone(60000, 7001.0, kRate, 0.5)), 7001.0);
        std::fprintf(stderr, "aliasing: distortion %-9s drive 100%%      %6.1f dB\n", names[m], a);
        CHECK(a < dist_max[m]);
    }
    const char* shapes[] = {"Smooth", "Warm", "Hard"};
    for (int sh = 0; sh < 3; ++sh) {
        for (float soft : {0.f, 50.f}) {
            DeviceRig rig("soft-clipper", kClipper, kRate);
            rig.set("input", 24.f);
            rig.set("softness", soft);
            rig.set("shape", static_cast<float>(sh));
            const double a = inharmonic_db(rig.run_mono(tone(60000, 7001.0, kRate, 0.5)), 7001.0);
            std::fprintf(stderr, "aliasing: soft clipper %-6s +24 dB, soft %2.0f%% %6.1f dB\n", shapes[sh], static_cast<double>(soft), a);
            CHECK(a < -40.0);
        }
    }
    const char* modes[] = {"Reel", "Cassette", "VHS"};
    for (int md = 0; md < 3; ++md) {
        DeviceRig rig("tapesaturation", kTape, kRate);
        rig.set("drive", 100.f);
        rig.set("mode", static_cast<float>(md));
        const double a = inharmonic_db(rig.run_mono(tone(60000, 7001.0, kRate, 0.5)), 7001.0);
        std::fprintf(stderr, "aliasing: tape %-8s drive 100%%           %6.1f dB\n", modes[md], a);
        CHECK(a < -45.0);
    }
}

TEST_CASE(distortion_bias_adds_even_harmonics_without_dc) {
    DeviceRig rig("distortion", kDistortion, kRate);
    rig.set("tone", 20000.f);
    const Signal clean = rig.run_mono(tone(60000, 1000.0, kRate, 0.5));
    DeviceRig biased("distortion", kDistortion, kRate);
    biased.set("tone", 20000.f);
    biased.set("bias", 80.f);
    const Signal y = biased.run_mono(tone(60000, 1000.0, kRate, 0.5));
    const double h2_clean = bin_power(clean, 9600, 48000, 2000.0) / bin_power(clean, 9600, 48000, 1000.0);
    const double h2 = bin_power(y, 9600, 48000, 2000.0) / bin_power(y, 9600, 48000, 1000.0);
    CHECK(10 * std::log10(h2_clean + 1e-30) < -80.0); // symmetric curve: no 2nd harmonic
    CHECK(10 * std::log10(h2) > -40.0);
    double mean = 0;
    for (std::size_t i = 24000; i < 60000; ++i) mean += y[i];
    CHECK(std::fabs(mean / 36000.0) < 1e-3);
}

TEST_CASE(distortion_tone_darkens_and_gains_apply) {
    DeviceRig bright("distortion", kDistortion, kRate);
    bright.set("tone", 20000.f);
    DeviceRig dark("distortion", kDistortion, kRate);
    dark.set("tone", 500.f);
    const Signal x = tone(60000, 1000.0, kRate, 0.5);
    const Signal yb = bright.run_mono(x), yd = dark.run_mono(x);
    const double h5b = bin_power(yb, 9600, 48000, 5000.0), h5d = bin_power(yd, 9600, 48000, 5000.0);
    CHECK(10 * std::log10(h5d / h5b) < -20.0);
    // Gain (wet) and Output both scale; quiet signal in the near-linear region.
    DeviceRig g("distortion", kDistortion, kRate);
    g.set("drive", 0.f);
    g.set("tone", 20000.f);
    g.set("output", -6.f);
    g.set("output_gain", 3.f);
    const Signal q = tone(24000, 1000.0, kRate, 0.01);
    CHECK_NEAR(db(rms(g.run_mono(q), 9600) / rms(q, 9600)), -3.0, 0.05);
}

TEST_CASE(distortion_mode_change_does_not_click) {
    DeviceRig rig("distortion", kDistortion, kRate);
    rig.set("tone", 20000.f);
    rig.set("drive", 60.f);
    const Signal x = tone(9600, 200.0, kRate, 0.5);
    Signal a = rig.run_mono(x);
    rig.set("mode", 3.f);
    Signal b = rig.run_mono(x);
    // Largest sample-to-sample step stays near what the waveform itself does.
    double step = 0;
    for (std::size_t i = 1; i < b.size(); ++i) step = std::max(step, static_cast<double>(std::fabs(b[i] - b[i - 1])));
    double step_a = 0;
    for (std::size_t i = 1; i < a.size(); ++i) step_a = std::max(step_a, static_cast<double>(std::fabs(a[i] - a[i - 1])));
    CHECK(step < 2.0 * step_a + 0.05);
}

// ---------------------------------------------------------------- tape

TEST_CASE(tape_saturates_compresses_and_reports_latency) {
    DeviceRig rig("tapesaturation", kTape, kRate);
    CHECK_EQ(rig.instance().latency(), kLat);
    // Quiet input: the drive gain g = 1 + 5 * 0.3 of the drawn model (the
    // curve is linear there), warmth EQ aside; no hiss at the -60 dB minimum.
    const Signal quiet = tone(48000, 1000.0, kRate, 0.01);
    const Signal yq = rig.run_mono(quiet);
    CHECK_NEAR(db(rms(yq, 9600) / rms(quiet, 9600)), db(2.5), 0.5);
    const Signal silence = rig.run_mono(Signal(48000, 0.0f));
    CHECK(peak(silence, 4800) < 1e-6);
    // Loud input: peaks held under the compression ceiling (1 - 0.5 * 0.3).
    DeviceRig loud("tapesaturation", kTape, kRate);
    loud.set("warmth", 0.f);
    const Signal y = loud.run_mono(tone(48000, 200.0, kRate, 1.0));
    CHECK(peak(y, 9600) < 0.95);
    // More drive -> more harmonics.
    DeviceRig hot("tapesaturation", kTape, kRate);
    hot.set("drive", 90.f);
    const Signal x = tone(60000, 1000.0, kRate, 0.5);
    const Signal yc = rig.run_mono(x), yh = hot.run_mono(x);
    const double h3c = bin_power(yc, 9600, 48000, 3000.0) / bin_power(yc, 9600, 48000, 1000.0);
    const double h3h = bin_power(yh, 9600, 48000, 3000.0) / bin_power(yh, 9600, 48000, 1000.0);
    CHECK(h3h > 4.0 * h3c);
}

TEST_CASE(tape_noise_and_warmth) {
    DeviceRig rig("tapesaturation", kTape, kRate);
    rig.set("noise", -30.f);
    const Signal hiss = rig.run_mono(Signal(48000, 0.0f));
    const double level = db(rms(hiss, 9600));
    CHECK(level > -42.0 && level < -28.0);
    // Warmth: more low end (head bump) and less top.
    DeviceRig flat("tapesaturation", kTape, kRate), warm("tapesaturation", kTape, kRate);
    flat.set("warmth", 0.f);
    warm.set("warmth", 100.f);
    const Signal lo = tone(48000, 90.0, kRate, 0.01), hi = tone(48000, 12000.0, kRate, 0.01);
    CHECK(db(rms(warm.run_mono(lo), 9600) / rms(flat.run_mono(lo), 9600)) > 3.0);
    CHECK(db(rms(warm.run_mono(hi), 9600) / rms(flat.run_mono(hi), 9600)) < -3.0);
    // Modes: VHS has less bandwidth than Reel-to-Reel.
    DeviceRig vhs("tapesaturation", kTape, kRate);
    vhs.set("mode", 2.f);
    vhs.set("warmth", 0.f);
    CHECK(db(rms(vhs.run_mono(hi), 9600) / rms(flat.run_mono(hi), 9600)) < -3.0);
}

// ---------------------------------------------------------------- soft clipper

TEST_CASE(soft_clipper_respects_the_ceiling) {
    for (int shape = 0; shape < 3; ++shape) {
        DeviceRig rig("soft-clipper", kClipper, kRate);
        CHECK_EQ(rig.instance().latency(), kLat);
        rig.set("shape", static_cast<float>(shape));
        rig.set("input", 18.f);
        rig.set("ceiling", -6.f);
        rig.set("softness", 30.f);
        const Signal y = rig.run_mono(tone(48000, 200.0, kRate, 0.9));
        // Ceiling -6 dB = 0.501; the reconstruction filters may overshoot slightly.
        const double pk = peak(y, 9600);
        if (pk > 0.53 || pk < 0.45) std::fprintf(stderr, "soft clipper shape %d peak %.4f\n", shape, pk);
        CHECK(pk < 0.53 && pk > 0.45);
    }
}

TEST_CASE(soft_clipper_is_transparent_below_the_knee_and_gain_matches) {
    DeviceRig rig("soft-clipper", kClipper, kRate);
    rig.set("shape", 2.f); // Hard: exactly linear below 1 - w
    rig.set("softness", 0.f);
    const Signal x = tone(24000, 1000.0, kRate, 0.5);
    const Signal y = rig.run_mono(x);
    double err = 0;
    for (std::size_t i = 4800; i < x.size(); ++i) err = std::max(err, static_cast<double>(std::fabs(y[i] - x[i - kLat])));
    CHECK(err < 5e-4);
    // Gain Match: +12 dB in, matched back out (signal far below the ceiling).
    DeviceRig m("soft-clipper", kClipper, kRate);
    m.set("input", 12.f);
    m.set("gain_match", 1.f);
    m.set("shape", 2.f);
    m.set("softness", 0.f);
    const Signal q = tone(24000, 1000.0, kRate, 0.05);
    CHECK_NEAR(db(rms(m.run_mono(q), 9600) / rms(q, 9600)), 0.0, 0.05);
}

TEST_CASE(soft_clipper_warm_is_asymmetric_without_dc) {
    DeviceRig rig("soft-clipper", kClipper, kRate);
    rig.set("shape", 1.f);
    rig.set("input", 12.f);
    rig.set("softness", 80.f);
    const Signal y = rig.run_mono(tone(96000, 1000.0, kRate, 0.5));
    const double h2 = bin_power(y, 48000, 48000, 2000.0) / bin_power(y, 48000, 48000, 1000.0);
    CHECK(10 * std::log10(h2) > -40.0);
    double mean = 0;
    for (std::size_t i = 48000; i < 96000; ++i) mean += y[i];
    CHECK(std::fabs(mean / 48000.0) < 2e-3);
}

TEST_CASE(soft_clipper_softness_matches_the_drawn_curve) {
    // Smooth at default softness 50 %: k = 6, a sine at the ceiling peaks at
    // 1 / 2^(1/6) = -1.0 dB (the panel's "Clipping" readout at defaults).
    DeviceRig rig("soft-clipper", kClipper, kRate);
    const Signal y = rig.run_mono(tone(48000, 100.0, kRate, 1.0));
    CHECK_NEAR(db(peak(y, 9600)), -1.0, 0.05);
}

// ---------------------------------------------------------------- bit crusher

TEST_CASE(bitcrusher_defaults_are_transparent_enough) {
    DeviceRig rig("bitcrusher", kCrusher, kRate);
    CHECK_EQ(rig.instance().latency(), 0u);
    const Signal x = tone(9600, 1000.0, kRate, 0.5);
    const Signal y = rig.run_mono(x);
    double err = 0;
    for (std::size_t i = 0; i < x.size(); ++i) err = std::max(err, static_cast<double>(std::fabs(y[i] - x[i])));
    CHECK(err <= 1.0 / 32768.0 + 1e-7); // 16 bits: half a step of 2^-15
    DeviceRig full("bitcrusher", kCrusher, kRate);
    full.set("bitdepth", 32.f);
    const Signal yf = full.run_mono(x);
    double errf = 0;
    for (std::size_t i = 0; i < x.size(); ++i) errf = std::max(errf, static_cast<double>(std::fabs(yf[i] - x[i])));
    CHECK(errf <= std::ldexp(1.0, -32)); // 32 bits: below float resolution of the signal
}

TEST_CASE(bitcrusher_quantises_to_the_bit_depth) {
    DeviceRig rig("bitcrusher", kCrusher, kRate);
    rig.set("bitdepth", 3.f); // 4 levels per side: multiples of 0.25
    const Signal y = rig.run_mono(tone(9600, 300.0, kRate, 0.9));
    bool on_grid = true;
    for (std::size_t i = 4800; i < y.size(); ++i) {
        const double k = y[i] * 4.0;
        if (std::fabs(k - std::round(k)) > 1e-5) on_grid = false;
    }
    CHECK(on_grid);
}

TEST_CASE(bitcrusher_downsample_holds_and_jitter_varies) {
    DeviceRig rig("bitcrusher", kCrusher, kRate);
    rig.set("bitdepth", 32.f);
    rig.set("downsample", 4.f);
    const Signal y = rig.run_mono(tone(4800, 500.0, kRate, 0.5));
    // Runs of exactly four equal samples.
    std::size_t changes = 0;
    for (std::size_t i = 2401; i < 4800; ++i)
        if (y[i] != y[i - 1]) ++changes;
    CHECK_NEAR(static_cast<double>(changes), 2399.0 / 4.0, 2.0);
    DeviceRig j("bitcrusher", kCrusher, kRate);
    j.set("bitdepth", 32.f);
    j.set("downsample", 8.f);
    j.set("jitter", 100.f);
    const Signal yj = j.run_mono(tone(48000, 500.0, kRate, 0.5));
    std::vector<int> runs;
    int run = 1;
    for (std::size_t i = 1; i < yj.size(); ++i) {
        if (yj[i] == yj[i - 1]) {
            ++run;
        } else {
            runs.push_back(run);
            run = 1;
        }
    }
    const auto [mn, mx] = std::minmax_element(runs.begin() + 1, runs.end());
    CHECK(*mn <= 5 && *mx >= 11);
}

TEST_CASE(bitcrusher_dither_decorrelates_the_error) {
    // A slow ramp through a 4-bit quantiser: without dither the error is a
    // deterministic sawtooth; with full dither its mean over a step is ~0.
    DeviceRig plain("bitcrusher", kCrusher, kRate), dith("bitcrusher", kCrusher, kRate);
    plain.set("bitdepth", 4.f);
    dith.set("bitdepth", 4.f);
    dith.set("dither", 100.f);
    Signal x(48000);
    for (std::size_t i = 0; i < x.size(); ++i) x[i] = 0.1f + 0.0625f * static_cast<float>(i) / 48000.0f; // within one step
    const Signal yp = plain.run_mono(x), yd = dith.run_mono(x);
    double ep = 0, ed = 0;
    for (std::size_t i = 0; i < x.size(); ++i) {
        ep += yp[i] - x[i];
        ed += yd[i] - x[i];
    }
    CHECK(std::fabs(ed / 48000.0) < 0.3 * std::fabs(ep / 48000.0) + 1e-3);
}
