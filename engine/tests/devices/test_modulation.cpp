// Modulation devices (engine/src/devices/{tremolo,autopanner,chorus,
// flanger,phaser,ring_mod}.cpp) through the shared device harness: gain and
// pan laws, LFO rates and tempo lock, comb / all-pass responses, ring
// modulation sidebands, band-limiting, latency and transparency at Mix 0.
#include "device_harness.hpp"
#include "devices/dsp.hpp"

using namespace devtest;

namespace {

constexpr double kRate = 48000.0;

std::vector<float> defaults_of(const char* id) {
    const saine::BuiltinDescriptor* d = saine::find_builtin(id);
    if (d == nullptr || d->defaults.size() != d->params.size()) {
        std::fprintf(stderr, "no defaults for %s\n", id);
        std::abort();
    }
    return {d->defaults.begin(), d->defaults.end()};
}
DeviceRig rig(const char* id, uint32_t block = 256) { return DeviceRig(id, defaults_of(id), kRate, block); }
Signal dc(std::size_t n, float v) { return Signal(n, v); }

// Index of the largest |s| in [from, to).
std::size_t argmax(const Signal& s, std::size_t from = 0, std::size_t to = SIZE_MAX) {
    std::size_t best = from;
    for (std::size_t i = from; i < std::min(to, s.size()); ++i)
        if (std::fabs(s[i]) > std::fabs(s[best])) best = i;
    return best;
}
// Frames between successive upward crossings of the mean of s[from, to).
double period_of(const Signal& s, std::size_t from, std::size_t to) {
    double mean = 0.0;
    for (std::size_t i = from; i < to; ++i) mean += s[i];
    mean /= static_cast<double>(to - from);
    std::vector<std::size_t> ups;
    for (std::size_t i = from + 1; i < to; ++i)
        if (s[i - 1] < mean && s[i] >= mean) ups.push_back(i);
    if (ups.size() < 2) return 0.0;
    return static_cast<double>(ups.back() - ups.front()) / static_cast<double>(ups.size() - 1);
}
double max_step(const Signal& s) {
    double m = 0.0;
    for (std::size_t i = 1; i < s.size(); ++i) m = std::max(m, static_cast<double>(std::fabs(s[i] - s[i - 1])));
    return m;
}
bool identical(const Signal& a, const Signal& b) {
    for (std::size_t i = 0; i < a.size(); ++i)
        if (a[i] != b[i]) return false;
    return true;
}

} // namespace

// ---------------------------------------------------------------- tremolo

TEST_CASE(tremolo_gain_law_and_rate) {
    auto r = rig("tremolo"); // Sine, 5 Hz, depth 70%, mix 100%
    const auto out = r.run_mono(dc(48000, 1.0f));
    // Gain swings between 1 - depth and 1 (the field caption: 10.5 dB).
    double lo = 1.0, hi = 0.0;
    for (std::size_t i = 4800; i < out.size(); ++i) {
        lo = std::min(lo, static_cast<double>(out[i]));
        hi = std::max(hi, static_cast<double>(out[i]));
    }
    CHECK_NEAR(lo, 0.3, 0.01);
    CHECK_NEAR(hi, 1.0, 0.01);
    CHECK_NEAR(db(hi / lo), 10.46, 0.3);
    CHECK_NEAR(period_of(out, 4800, out.size()), kRate / 5.0, 30.0);
    const float ph = r.readouts()[0];
    CHECK(ph >= 0.0f && ph < 1.0f);
}

TEST_CASE(tremolo_depth_or_mix_zero_is_transparent) {
    for (const char* p : {"depth", "mix"}) {
        auto r = rig("tremolo");
        r.set(p, 0.0f);
        const Signal x = noise(9600, 0.5f, 3);
        CHECK(identical(r.run_mono(x), x));
    }
}

TEST_CASE(tremolo_stereo_phase_alternates_channels) {
    auto r = rig("tremolo");
    r.set("stereo_phase", 180.0f);
    r.set("depth", 100.0f);
    auto [l, rr] = r.run(dc(24000, 1.0f), dc(24000, 1.0f));
    // Opposite phases: the sum of the two sine gains stays at 1.
    for (std::size_t i = 4800; i < l.size(); i += 97) CHECK_NEAR(l[i] + rr[i], 1.0, 0.02);
}

TEST_CASE(tremolo_square_and_saw_do_not_click) {
    for (float shape : {2.0f, 3.0f}) {
        auto r = rig("tremolo");
        r.set("shape", shape);
        r.set("depth", 100.0f);
        r.set("rate", 20.0f);
        const auto out = r.run_mono(dc(48000, 1.0f));
        // The 0.3 ms edge pole limits any full-depth step to < 7% per sample.
        CHECK(max_step(out) < 0.07);
        CHECK(*std::min_element(out.begin(), out.end()) < 0.02f);
    }
}

TEST_CASE(tremolo_sync_locks_to_the_song_position) {
    // Sync 1/4 at 120 bpm: 2 Hz, phase 0 on every beat. Two runs that start
    // at different song positions agree once aligned to the timeline.
    auto a = rig("tremolo");
    a.set("sync", 1.0f);
    a.set_transport(true, 120.0, 0);
    const auto oa = a.run_mono(dc(48000, 1.0f));
    CHECK_NEAR(period_of(oa, 0, oa.size()), 24000.0, 20.0);
    auto b = rig("tremolo");
    b.set("sync", 1.0f);
    b.set_transport(true, 120.0, 6000);
    const auto ob = b.run_mono(dc(42000, 1.0f));
    double err = 0.0;
    for (std::size_t i = 2000; i < ob.size(); ++i) err = std::max(err, static_cast<double>(std::fabs(ob[i] - oa[i + 6000])));
    CHECK(err < 1e-3);
    // 1/8T at 90 bpm: a cycle of 1/3 beat = 0.2222 s.
    auto c = rig("tremolo");
    c.set("sync", 1.0f);
    c.set("rate_sync", 12.0f);
    c.set_transport(true, 90.0, 0);
    const auto oc = c.run_mono(dc(48000, 1.0f));
    CHECK_NEAR(period_of(oc, 0, oc.size()), kRate / (90.0 / 60.0 * 3.0), 20.0);
}

// ---------------------------------------------------------------- auto-panner

TEST_CASE(autopanner_equal_power_law) {
    auto r = rig("autopanner"); // Sync 1/4, sine, depth 80%
    r.set("depth", 100.0f);
    auto [l, rr] = r.run(dc(48000, 1.0f), dc(48000, 1.0f));
    // L^2 + R^2 = 2 (equal power, unity in the centre) all along the sweep.
    for (std::size_t i = 2000; i < l.size(); i += 101) CHECK_NEAR(l[i] * l[i] + rr[i] * rr[i], 2.0, 0.02);
    // The sweep reaches both speakers: one side near sqrt(2), the other near 0.
    CHECK(peak(l) > 1.40 && peak(rr) > 1.40);
    CHECK(*std::min_element(l.begin() + 2000, l.end()) < 0.05f);
    CHECK_NEAR(period_of(l, 2000, l.size()), 24000.0, 40.0);
}

TEST_CASE(autopanner_square_holds_left_then_right) {
    auto r = rig("autopanner");
    r.set("shape", 2.0f);
    r.set("depth", 100.0f);
    r.set("sync", 0.0f);
    r.set("rate", 1.0f);
    auto [l, rr] = r.run(dc(48000, 1.0f), dc(48000, 1.0f));
    // First half cycle: hard left (after the 5 ms glide); second: hard right.
    CHECK_NEAR(l[12000], 1.41421, 1e-3);
    CHECK_NEAR(rr[12000], 0.0, 1e-3);
    CHECK_NEAR(l[36000], 0.0, 1e-3);
    CHECK_NEAR(rr[36000], 1.41421, 1e-3);
    // The glide keeps the switch click-free.
    CHECK(max_step(l) < 0.01 && max_step(rr) < 0.01);
}

TEST_CASE(autopanner_depth_or_mix_zero_is_unity) {
    for (const char* p : {"depth", "mix"}) {
        auto r = rig("autopanner");
        r.set(p, 0.0f);
        const Signal x = noise(9600, 0.5f, 5);
        auto [l, rr] = r.run(x, x);
        double err = 0.0;
        for (std::size_t i = 0; i < x.size(); ++i) err = std::max(err, static_cast<double>(std::fabs(l[i] - x[i]) + std::fabs(rr[i] - x[i])));
        CHECK(err < 1e-5);
    }
}

TEST_CASE(autopanner_random_picks_new_positions_per_cycle) {
    auto r = rig("autopanner");
    r.set("shape", 3.0f);
    r.set("depth", 100.0f);
    r.set("sync", 0.0f);
    r.set("rate", 4.0f);
    auto [l, rr] = r.run(dc(48000, 1.0f), dc(48000, 1.0f));
    // Within a cycle the position holds (after the glide); across cycles it changes.
    int changes = 0;
    for (int c = 1; c < 4; ++c) {
        const std::size_t a = static_cast<std::size_t>(c) * 12000 + 6000;
        CHECK_NEAR(l[a], l[a + 4000], 1e-3);
        if (std::fabs(l[a] - l[a + 12000]) > 0.05f) ++changes;
    }
    CHECK(changes >= 2);
}

// ---------------------------------------------------------------- chorus

TEST_CASE(chorus_mix_zero_is_dry_times_output) {
    auto r = rig("chorus");
    r.set("mix", 0.0f);
    r.set("output_gain", 6.0f);
    const Signal x = noise(9600, 0.5f, 7);
    const auto out = r.run_mono(x);
    double err = 0.0;
    for (std::size_t i = 0; i < x.size(); ++i) err = std::max(err, std::fabs(out[i] - x[i] * 1.99526));
    CHECK(err < 1e-5);
}

TEST_CASE(chorus_voice_delay_is_base_plus_depth) {
    // Depth 0, one voice, mix 100: the wet path is the input 1.3 ms later
    // (through the 35 Hz - 4.7 kHz band limit).
    auto r = rig("chorus");
    r.set("depth", 0.0f);
    r.set("mix", 100.0f);
    const auto out = r.run_mono(impulse(4800, 1.0f, 100));
    CHECK_NEAR(static_cast<double>(argmax(out)), 100.0 + 1.3e-3 * kRate, 3.0);
    // Full depth reaches up to 1.3 + 8 ms.
    auto d = rig("chorus");
    d.set("mix", 100.0f);
    d.set("depth", 8.0f);
    d.set("rate", 0.1f);
    d.instance().processor().reset();
    std::size_t latest = 0;
    for (int k = 0; k < 12; ++k) {
        const auto o = d.run_mono(impulse(24000, 1.0f, 100));
        latest = std::max(latest, argmax(o) - 100);
    }
    CHECK(latest > static_cast<std::size_t>(6e-3 * kRate) && latest < static_cast<std::size_t>(9.6e-3 * kRate));
}

TEST_CASE(chorus_feedback_rings_and_width_decorrelates) {
    auto a = rig("chorus");
    a.set("mix", 100.0f);
    a.set("depth", 0.0f);
    const double plain = rms(a.run_mono(impulse(9600, 1.0f)), 400);
    auto b = rig("chorus");
    b.set("mix", 100.0f);
    b.set("depth", 0.0f);
    b.set("feedback", 90.0f);
    const double fed = rms(b.run_mono(impulse(9600, 1.0f)), 400);
    CHECK(fed > 4.0 * plain);
    const Signal x = noise(24000, 0.5f, 9);
    auto w0 = rig("chorus");
    w0.set("width", 0.0f);
    auto [l0, r0] = w0.run(x, x);
    CHECK(identical(l0, r0));
    auto w1 = rig("chorus");
    auto [l1, r1] = w1.run(x, x);
    CHECK(!identical(l1, r1));
}

TEST_CASE(chorus_voices_stay_level_matched) {
    // Equal-power mix and v^0.35 voice gain: 1 vs 4 voices within a few dB.
    double lv[2];
    for (int k = 0; k < 2; ++k) {
        auto r = rig("chorus");
        r.set("voices", k == 0 ? 0.0f : 3.0f);
        lv[k] = rms(r.run_mono(noise(48000, 0.3f, 11)), 4800);
    }
    CHECK(std::fabs(db(lv[1] / lv[0])) < 3.0);
}

// ---------------------------------------------------------------- flanger

TEST_CASE(flanger_comb_notch_at_half_inverse_delay) {
    // Classic, depth 0, feedback 0, mix 50: dry + 1 ms copy cancels 500 Hz.
    auto r = rig("flanger");
    r.set("depth", 0.0f);
    r.set("feedback", 0.0f);
    const auto notch = r.run_mono(tone(48000, 500.0, kRate, 0.5));
    auto q = rig("flanger");
    q.set("depth", 0.0f);
    q.set("feedback", 0.0f);
    const auto peak_out = q.run_mono(tone(48000, 1000.0, kRate, 0.5));
    CHECK(db(rms(notch, 4800) / rms(peak_out, 4800)) < -30.0);
}

TEST_CASE(flanger_thru_zero_delays_the_dry_path) {
    // Depth 0: wet and dry are both 1 ms late and add up.
    auto r = rig("flanger");
    r.set("mode", 2.0f);
    r.set("depth", 0.0f);
    r.set("feedback", 0.0f);
    const auto out = r.run_mono(impulse(4800, 1.0f, 100));
    CHECK_EQ(argmax(out), static_cast<std::size_t>(148));
    float dry = 0.0f, wet = 0.0f;
    saine::dsp::mix_gains(0.5f, -0.15f, dry, wet);
    CHECK_NEAR(out[148], dry + 1.318f * wet, 1e-3);
}

TEST_CASE(flanger_extreme_feedback_is_stable) {
    for (float fb : {-90.0f, 90.0f}) {
        for (float mode : {0.0f, 1.0f, 2.0f}) {
            auto r = rig("flanger");
            r.set("feedback", fb);
            r.set("mode", mode);
            r.set("depth", 0.2f);
            const auto out = r.run_mono(noise(96000, 0.5f, 13));
            CHECK(all_finite(out));
            CHECK(peak(out, 48000) < 20.0);
        }
    }
}

TEST_CASE(flanger_mix_zero_is_dry) {
    auto r = rig("flanger");
    r.set("mix", 0.0f);
    const Signal x = noise(9600, 0.5f, 15);
    CHECK(identical(r.run_mono(x), x));
}

// ---------------------------------------------------------------- phaser

TEST_CASE(phaser_first_notch_follows_the_allpass_law) {
    // Depth 0: the break frequency rests at sqrt(200 * 8000) Hz; four stages
    // reach -pi (a notch at mix 50%) at fs/pi atan(tan(pi fc / fs) tan(pi / 8)).
    const double fc = std::sqrt(200.0 * 8000.0);
    const double notch = kRate / 3.14159265358979 * std::atan(std::tan(3.14159265358979 * fc / kRate) * std::tan(3.14159265358979 / 8.0));
    auto r = rig("phaser");
    r.set("depth", 0.0f);
    r.set("feedback", 0.0f);
    const auto at = r.run_mono(tone(48000, notch, kRate, 0.5));
    auto q = rig("phaser");
    q.set("depth", 0.0f);
    q.set("feedback", 0.0f);
    const auto low = q.run_mono(tone(48000, 60.0, kRate, 0.5));
    CHECK(db(rms(at, 4800) / rms(low, 4800)) < -30.0);
}

TEST_CASE(phaser_every_stage_count_and_feedback_is_stable) {
    for (float st : {0.0f, 1.0f, 2.0f, 3.0f}) {
        for (float fb : {-95.0f, 95.0f}) {
            auto r = rig("phaser");
            r.set("stages", st);
            r.set("feedback", fb);
            r.set("depth", 100.0f);
            r.set("rate", 10.0f);
            const auto out = r.run_mono(noise(96000, 0.5f, 17));
            CHECK(all_finite(out));
            CHECK(peak(out, 48000) < 40.0);
        }
    }
}

TEST_CASE(phaser_mix_zero_and_output_gain) {
    auto r = rig("phaser");
    r.set("mix", 0.0f);
    r.set("output_gain", -6.0f);
    const Signal x = noise(9600, 0.5f, 19);
    const auto out = r.run_mono(x);
    double err = 0.0;
    for (std::size_t i = 0; i < x.size(); ++i) err = std::max(err, std::fabs(out[i] - x[i] * 0.501187));
    CHECK(err < 1e-5);
}

// ---------------------------------------------------------------- ring mod

TEST_CASE(ring_mod_reports_latency_and_aligns_dry) {
    auto r = rig("ring-mod");
    CHECK_EQ(r.instance().latency(), 23u);
    r.set("mix", 0.0f);
    const auto out = r.run_mono(impulse(512, 1.0f, 10));
    CHECK_EQ(argmax(out), static_cast<std::size_t>(33));
    CHECK_NEAR(out[33], 1.0, 1e-6);
}

TEST_CASE(ring_mod_sine_makes_sum_and_difference_tones) {
    auto r = rig("ring-mod"); // 440 Hz sine, mix 100
    const auto out = r.run_mono(tone(48000, 1000.0, kRate, 0.5));
    const double sum = tone_level(out, 1440.0, kRate, 4800), diff = tone_level(out, 560.0, kRate, 4800);
    const double carrier_in = tone_level(out, 1000.0, kRate, 4800);
    CHECK_NEAR(sum, 0.25, 0.01);
    CHECK_NEAR(diff, 0.25, 0.01);
    CHECK(carrier_in < 1e-3);
}

TEST_CASE(ring_mod_bias_keeps_the_dry_note) {
    auto r = rig("ring-mod");
    r.set("bias", 100.0f);
    const auto out = r.run_mono(tone(48000, 1000.0, kRate, 0.5));
    // (w + 1) / 2: half the input plus sidebands at a quarter.
    CHECK_NEAR(tone_level(out, 1000.0, kRate, 4800), 0.25, 0.01);
    CHECK_NEAR(tone_level(out, 1440.0, kRate, 4800), 0.125, 0.01);
}

TEST_CASE(ring_mod_square_carrier_is_band_limited) {
    // A 4.9 kHz square: harmonics above Nyquist must not fold back (the 9th
    // at 44.1 kHz would alias to 3.9 kHz, the 11th to 5.9 kHz).
    auto r = rig("ring-mod");
    r.set("modulator", 2.0f);
    r.set("frequency", 4900.0f);
    const auto out = r.run_mono(dc(48000, 0.5f));
    const double fund = tone_level(out, 4900.0, kRate, 4800);
    CHECK_NEAR(fund, 0.5 * 4.0 / 3.14159, 0.03);
    CHECK(db(tone_level(out, 3900.0, kRate, 4800) / fund) < -55.0);
    CHECK(db(tone_level(out, 5900.0, kRate, 4800) / fund) < -55.0);
}

TEST_CASE(ring_mod_rectify_and_spread) {
    auto r = rig("ring-mod");
    r.set("rectify", 100.0f);
    r.set("frequency", 100.0f);
    const auto out = r.run_mono(dc(48000, 1.0f));
    double mean = 0.0;
    for (std::size_t i = 4800; i < out.size(); ++i) mean += out[i];
    mean /= static_cast<double>(out.size() - 4800);
    CHECK_NEAR(mean, 2.0 / 3.14159, 0.01); // mean of |sin|
    auto s = rig("ring-mod");
    s.set("spread", 100.0f);
    const Signal x = tone(24000, 300.0, kRate, 0.5);
    auto [l, rr] = s.run(x, x);
    CHECK(!identical(l, rr));
    // Left carrier a semitone down, right a semitone up.
    CHECK(tone_level(l, 300.0 + 440.0 / 1.0594631, kRate, 2400) > 0.2);
    CHECK(tone_level(rr, 300.0 + 440.0 * 1.0594631, kRate, 2400) > 0.2);
}

TEST_CASE(ring_mod_every_modulator_is_bounded) {
    for (float m = 0.0f; m <= 5.0f; m += 1.0f) {
        for (float f : {1.0f, 440.0f, 20000.0f}) {
            auto r = rig("ring-mod");
            r.set("modulator", m);
            r.set("frequency", f);
            r.set("rectify", -60.0f);
            const auto out = r.run_mono(noise(24000, 0.5f, 21));
            CHECK(all_finite(out));
            CHECK(peak(out) < 1.0);
        }
    }
}
