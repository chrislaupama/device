// Time and pitch built-ins: Delay, Echo, Reverb, Pitch Shifter, Grains, and
// their shared blocks (FFT, phase-vocoder pitch shifter), through the shared
// device harness.
#include "device_harness.hpp"
#include "devices/spectral.hpp"
#include "devices/pitch_shift.hpp"

using namespace devtest;

namespace {

constexpr double kRate = 48000.0;

std::vector<float> defaults_of(const char* id) {
    const dstudio::BuiltinDescriptor* d = dstudio::find_builtin(id);
    return {d->defaults.begin(), d->defaults.end()};
}

const std::vector<float> kDelay = defaults_of("delay");
const std::vector<float> kReverb = defaults_of("reverb");
const std::vector<float> kPitch = defaults_of("pitch-shifter");
const std::vector<float> kGrains = defaults_of("grains");
std::vector<float> echo_defaults() { return defaults_of("echo"); }

// Frequency of the strongest bin of x[from, from + 8192).
double dominant_hz(const Signal& x, std::size_t from) {
    constexpr uint32_t n = 8192;
    dstudio::dsp::RealFft fft;
    fft.prepare(n);
    std::vector<float> f(n), re(n / 2 + 1), im(n / 2 + 1);
    for (uint32_t i = 0; i < n; ++i) f[i] = x[from + i] * static_cast<float>(0.5 - 0.5 * std::cos(2.0 * 3.14159265358979 * i / n));
    fft.forward(f.data(), re.data(), im.data());
    std::vector<double> m(n / 2 + 1);
    for (uint32_t k = 0; k <= n / 2; ++k) m[k] = std::log(1e-12 + static_cast<double>(re[k]) * re[k] + static_cast<double>(im[k]) * im[k]);
    uint32_t best = 1;
    for (uint32_t k = 1; k < n / 2; ++k)
        if (m[k] > m[best]) best = k;
    // Parabolic interpolation of the log-power peak.
    const double a = m[best - 1], b = m[best], c = m[best + 1];
    const double off = 0.5 * (a - c) / (a - 2.0 * b + c);
    return (best + off) * kRate / n;
}

std::size_t argmax_abs(const Signal& s, std::size_t from = 0, std::size_t to = SIZE_MAX) {
    std::size_t at = from;
    for (std::size_t i = from; i < std::min(to, s.size()); ++i)
        if (std::fabs(s[i]) > std::fabs(s[at])) at = i;
    return at;
}

} // namespace

// ---------------------------------------------------------------- shared blocks

TEST_CASE(real_fft_round_trips_and_finds_a_sine) {
    dstudio::dsp::RealFft fft;
    fft.prepare(1024);
    const Signal x = noise(1024, 1.0f, 9);
    std::vector<float> re(513), im(513), y(1024);
    fft.forward(x.data(), re.data(), im.data());
    fft.inverse(re.data(), im.data(), y.data());
    double err = 0.0;
    for (std::size_t i = 0; i < x.size(); ++i) err = std::max(err, static_cast<double>(std::fabs(x[i] - y[i])));
    CHECK(err < 1e-5);
    // Parseval-free check: a bin-centred sine lands in its bin.
    const Signal s = tone(1024, 50.0 * kRate / 1024.0, kRate, 1.0);
    fft.forward(s.data(), re.data(), im.data());
    CHECK_NEAR(std::sqrt(re[50] * re[50] + im[50] * im[50]), 512.0, 1.0);
    CHECK(std::fabs(re[49]) + std::fabs(im[49]) < 1e-2);
}

TEST_CASE(phase_vocoder_shifts_a_sine_by_the_ratio) {
    for (float ratio : {2.0f, 0.5f, 1.4983f}) {
        dstudio::dsp::PitchShifter ps;
        ps.prepare(2048);
        ps.set_ratio(ratio);
        const Signal x = tone(48000, 440.0, kRate, 0.5);
        Signal y(x.size());
        ps.process(x.data(), y.data(), static_cast<uint32_t>(x.size()));
        CHECK_NEAR(dominant_hz(y, 20000), 440.0 * ratio, 440.0 * ratio * 0.012);
        CHECK(all_finite(y));
        CHECK(rms(y, 20000) > 0.2);
    }
}

// ---------------------------------------------------------------- pitch shifter

TEST_CASE(pitch_shifter_octave_up_and_down) {
    for (float st : {12.f, -12.f, 7.f}) {
        for (float formant : {0.f, 1.f}) {
            DeviceRig r("pitch-shifter", kPitch, kRate, 256);
            r.set("semitones", st);
            r.set("formant_preserve", formant);
            const Signal x = tone(48000, 330.0, kRate, 0.4);
            const auto y = r.run(x, x).first;
            const double want = 330.0 * std::exp2(st / 12.0);
            CHECK_NEAR(dominant_hz(y, 24000), want, want * 0.015);
        }
    }
}

TEST_CASE(pitch_shifter_reports_latency_and_unity_is_a_pure_delay) {
    DeviceRig r("pitch-shifter", kPitch, kRate, 256);
    const uint32_t lat = r.instance().latency();
    CHECK_EQ(lat, 2048u);
    const Signal x = noise(9600, 0.5f, 11);
    const auto y = r.run(x, x).first;
    for (std::size_t i = lat; i < x.size(); ++i) {
        if (y[i] != x[i - lat]) {
            CHECK(y[i] == x[i - lat]);
            break;
        }
    }
    // Mix 0: the delayed dry, also exact.
    DeviceRig d("pitch-shifter", kPitch, kRate, 256);
    d.set("semitones", 5.f);
    d.set("mix", 0.f);
    const auto z = d.run(x, x).first;
    CHECK(z[lat + 1000] == x[1000]);
}

TEST_CASE(pitch_shifter_vocoder_path_lines_up_with_the_dry_path) {
    // Formants on at zero shift runs the vocoder; its output must sit at the reported latency.
    DeviceRig r("pitch-shifter", kPitch, kRate, 256);
    r.set("formant_preserve", 1.f);
    const uint32_t lat = r.instance().latency();
    const Signal x = noise(48000, 0.5f, 12);
    const auto y = r.run(x, x).first;
    const auto corr = [&](std::size_t lag) {
        double xy = 0.0, xx = 0.0, yy = 0.0;
        for (std::size_t i = 24000; i < 48000; ++i) {
            xy += static_cast<double>(y[i]) * x[i - lag];
            xx += static_cast<double>(x[i - lag]) * x[i - lag];
            yy += static_cast<double>(y[i]) * y[i];
        }
        return xy / std::sqrt(xx * yy + 1e-12);
    };
    CHECK(corr(lat) > 0.8);
    CHECK(corr(lat) > corr(lat - 512) + 0.3);
}

// ---------------------------------------------------------------- delay

TEST_CASE(delay_free_time_feedback_and_dry) {
    DeviceRig r("delay", kDelay, kRate, 256);
    r.set("sync", 0.f);
    r.set("mode", 1.f); // Digital
    r.set("timeMs", 100.f);
    r.set("feedback", 50.f);
    r.set("mix", 100.f);
    r.set("dry", 0.f);
    const auto y = r.run_mono(impulse(24000));
    const std::size_t first = argmax_abs(y, 0, 7200);
    CHECK(first >= 4798 && first <= 4803);
    CHECK_NEAR(peak(y, 4700, 4900), 1.0, 0.08);
    CHECK_NEAR(peak(y, 9500, 9700), 0.5, 0.06);
    CHECK(peak(y, 0, 4700) < 1e-3); // dry off
}

TEST_CASE(delay_sync_follows_the_tempo_and_unlinked_sides_differ) {
    DeviceRig r("delay", kDelay, kRate, 256);
    r.set("mode", 1.f);
    r.set("timeDivision", 6.f); // 1/4 at 120 BPM = 0.5 s
    r.set("feedback", 0.f);
    r.set("mix", 100.f);
    r.set("dry", 0.f);
    r.set("link", 0.f);
    r.set("timeDivisionR", 9.f); // 1/8 = 0.25 s
    auto [l, rr] = r.run(impulse(36000), impulse(36000));
    const std::size_t al = argmax_abs(l), ar = argmax_abs(rr);
    CHECK(al >= 23998 && al <= 24003);
    CHECK(ar >= 11998 && ar <= 12003);
}

TEST_CASE(delay_ping_pong_alternates_sides_and_mono_width_is_mono) {
    DeviceRig r("delay", kDelay, kRate, 256);
    r.set("sync", 0.f);
    r.set("mode", 1.f);
    r.set("timeMs", 50.f);
    r.set("feedback", 80.f);
    r.set("stereoWidth", 200.f);
    r.set("mix", 100.f);
    r.set("dry", 0.f);
    auto [l, rr] = r.run(impulse(12000), impulse(12000));
    CHECK(peak(l, 2300, 2500) > 0.5);
    CHECK(peak(rr, 2300, 2500) < 0.05);
    CHECK(peak(rr, 4700, 4900) > 0.3);
    CHECK(peak(l, 4700, 4900) < 0.05);

    DeviceRig m("delay", kDelay, kRate, 256);
    m.set("stereoWidth", 0.f);
    auto [ml, mr] = m.run(noise(24000, 0.5f, 1), noise(24000, 0.5f, 2));
    m.set("mix", 100.f);
    m.set("dry", 0.f);
    auto [ml2, mr2] = m.run(noise(24000, 0.5f, 3), noise(24000, 0.5f, 4));
    double diff = 0.0;
    for (std::size_t i = 12000; i < ml2.size(); ++i) diff = std::max(diff, static_cast<double>(std::fabs(ml2[i] - mr2[i])));
    CHECK(diff < 1e-4);
}

TEST_CASE(delay_full_feedback_stays_bounded_in_every_mode) {
    for (float mode : {0.f, 1.f, 2.f}) {
        DeviceRig r("delay", kDelay, kRate, 512);
        r.set("mode", mode);
        r.set("sync", 0.f);
        r.set("timeMs", 30.f);
        r.set("feedback", 100.f);
        r.set("grit", 100.f);
        r.set("eq2Gain", 15.f);
        r.set("modDepth", 100.f);
        r.set("modSpeed", 10.f);
        const auto y = r.run_mono(noise(48000 * 4, 1.0f, 7));
        CHECK(all_finite(y));
        CHECK(peak(y) < 3.0);
    }
}

TEST_CASE(delay_eq_shapes_only_the_repeats) {
    DeviceRig r("delay", kDelay, kRate, 256);
    r.set("sync", 0.f);
    r.set("mode", 1.f);
    r.set("timeMs", 200.f);
    r.set("feedback", 0.f);
    r.set("mix", 100.f);
    r.set("dry", 0.f);
    r.set("eq5Enabled", 1.f);
    r.set("eq5Freq", 500.f);
    const Signal hi = tone(48000, 6000.0, kRate, 0.5);
    const auto y = r.run_mono(hi);
    CHECK(rms(y, 24000) < 0.5 * 0.7071 * 0.05); // ~ -30 dB at 6 kHz through a 500 Hz low-pass
    DeviceRig dry("delay", kDelay, kRate, 256);
    dry.set("mix", 0.f);
    dry.set("eq5Enabled", 1.f);
    dry.set("eq5Freq", 500.f);
    const auto z = dry.run_mono(hi);
    CHECK(z[30000] == hi[30000]);
}

// ---------------------------------------------------------------- echo

TEST_CASE(echo_head_times_follow_the_machine_constants) {
    auto quiet = [](DeviceRig& r) {
        for (const char* id : {"art_hiss", "art_wf", "art_splice", "art_scrape"}) r.set(id, 0.f);
        r.set("mix", 100.f);
        r.set("kill_dry", 1.f);
        r.set("intensity", 0.f);
        r.set("tail_on", 0.f);
    };
    {
        DeviceRig r("echo", echo_defaults(), kRate, 256);
        quiet(r);
        const auto y = r.run_mono(impulse(24000));
        // Tape Loop at Motor 0.5: 260 mm/s; heads 2 and 3 at 52.4 and 75.6 mm.
        const std::size_t h2 = argmax_abs(y, 9000, 11000), h3 = argmax_abs(y, 13000, 15000);
        CHECK(std::abs(static_cast<long>(h2) - 9674) <= 6);
        CHECK(std::abs(static_cast<long>(h3) - 13957) <= 6);
        CHECK(peak(y, 0, 9000) < 0.01);
    }
    {
        DeviceRig r("echo", echo_defaults(), kRate, 256);
        quiet(r);
        r.set("machine", 2.f);
        r.set("motor", 0.f); // record head 162.56 mm before the playback head at 203.2 mm/s = 0.8 s
        const auto y = r.run_mono(impulse(48000));
        const std::size_t at = argmax_abs(y, 30000, 48000);
        CHECK(std::abs(static_cast<long>(at) - 38400) <= 8);
    }
    {
        DeviceRig r("echo", echo_defaults(), kRate, 256);
        quiet(r);
        r.set("sync", 1.f);
        r.set("note_l", 10.f); // 1/4 at 120 BPM
        const auto y = r.run_mono(impulse(36000));
        const std::size_t at = argmax_abs(y, 0, 30000);
        CHECK(std::abs(static_cast<long>(at) - 24000) <= 30);
    }
}

TEST_CASE(echo_intensity_builds_repeats_and_runaway_stays_bounded) {
    auto energy_late = [](float intensity) {
        DeviceRig r("echo", echo_defaults(), kRate, 256);
        r.set("intensity", intensity);
        r.set("mix", 100.f);
        r.set("kill_dry", 1.f);
        const auto y = r.run_mono(concat(tone(4800, 300.0, kRate, 0.5), Signal(96000, 0.0f)));
        return rms(y, 72000);
    };
    CHECK(energy_late(7.0f) > 4.0 * energy_late(3.0f));
    DeviceRig r("echo", echo_defaults(), kRate, 512);
    r.set("intensity", 10.f);
    r.set("generations", 1.f);
    r.set("spring_on", 1.f);
    r.set("spring", 100.f);
    const auto y = r.run_mono(noise(48000 * 6, 1.0f, 3));
    CHECK(all_finite(y));
    CHECK(peak(y) < 6.0);
}

TEST_CASE(echo_ducking_and_throw) {
    auto wet_while_loud = [](float duck, float throw_on) {
        DeviceRig r("echo", echo_defaults(), kRate, 256);
        r.set("duck_on", duck);
        r.set("duck_amount", 100.f);
        r.set("throw", throw_on);
        r.set("mix", 100.f);
        r.set("kill_dry", 1.f);
        const auto y = r.run_mono(tone(48000, 200.0, kRate, 0.5));
        return rms(y, 24000);
    };
    CHECK(wet_while_loud(1.f, 0.f) < 0.2 * wet_while_loud(0.f, 0.f));
    CHECK(wet_while_loud(0.f, 1.f) > 1.3 * wet_while_loud(0.f, 0.f));
}

TEST_CASE(echo_authored_heads_override_the_machine) {
    auto v = echo_defaults();
    DeviceRig r("echo", v, kRate, 256);
    for (const char* id : {"art_hiss", "art_wf", "art_splice", "art_scrape"}) r.set(id, 0.f);
    r.set("mix", 100.f);
    r.set("kill_dry", 1.f);
    r.set("intensity", 0.f);
    r.set("tail_on", 0.f);
    r.set("heads", 1.f);
    r.set("heads_machine", 1.f); // Tape Loop
    r.set("h1_present", 1.f);
    r.set("h1_on", 1.f);
    r.set("h1_level", 1.f);
    // Position 0.1 of [0.1, 1600 - 18 - 1] mm at 260 mm/s.
    r.set("h1_pos", 0.1f);
    const double mm = 0.1 + 0.1 * (1600.0 - 18.0 - 1.0 - 0.1);
    const auto y = r.run_mono(impulse(48000));
    const auto at = static_cast<long>(argmax_abs(y));
    CHECK(std::abs(at - static_cast<long>(mm / 260.0 * kRate)) <= 8);
}

// ---------------------------------------------------------------- reverb

TEST_CASE(reverb_tail_decays_at_the_decay_time) {
    DeviceRig r("reverb", kReverb, kRate, 256);
    r.set("mix", 100.f);
    r.set("dry", 0.f);
    r.set("decay", 2.f);
    r.set("damping", 0.f);
    r.set("moddepth", 0.f);
    const auto y = r.run_mono(concat(noise(4800, 0.5f, 5), Signal(48000 * 3, 0.0f)));
    const double a = rms(y, 4800 + 24000, 4800 + 33600), b = rms(y, 4800 + 72000, 4800 + 81600);
    // 1 s apart: -30 dB for a 2 s T60 (allow the build-up and modal spread).
    CHECK_NEAR(db(b / a), -30.0, 6.0);
}

TEST_CASE(reverb_every_mode_rings_and_stays_bounded) {
    for (float mode = 0; mode < 6; mode += 1) {
        DeviceRig r("reverb", kReverb, kRate, 256);
        r.set("mode2", mode);
        r.set("mix", 100.f);
        r.set("dry", 0.f);
        r.set("decay", 10.f);
        r.set("shimmer", 100.f);
        r.set("moddepth", 200.f);
        r.set("diffusion", 100.f);
        const auto y = r.run_mono(concat(noise(9600, 0.8f, 2), Signal(48000 * 2, 0.0f)));
        CHECK(all_finite(y));
        CHECK(rms(y, 48000, 96000) > 1e-3);
        CHECK(peak(y) < 4.0);
    }
}

TEST_CASE(reverb_predelay_dry_and_silence_gate) {
    DeviceRig r("reverb", kReverb, kRate, 256);
    r.set("mix", 100.f);
    r.set("dry", 0.f);
    r.set("predelay", 50.f);
    r.set("mode2", 2.f); // Plate: no early reflections before the tail
    const auto y = r.run_mono(impulse(9600));
    CHECK(peak(y, 0, 2350) < 1e-6);
    CHECK(peak(y, 2400, 9600) > 1e-3);
    // Mix 0 with Dry on: the input, exactly.
    DeviceRig d("reverb", kReverb, kRate, 256);
    d.set("mix", 0.f);
    const Signal x = noise(4800, 0.5f, 8);
    const auto z = d.run_mono(x);
    CHECK(z[4000] == x[4000]);
    // Long silence after sound: the network sleeps and the wet output is exactly zero.
    DeviceRig s("reverb", kReverb, kRate, 256);
    s.set("mix", 100.f);
    s.set("dry", 0.f);
    s.set("decay", 0.5f);
    const auto q = s.run_mono(concat(noise(4800, 0.5f, 1), Signal(48000 * 3, 0.0f)));
    CHECK(peak(q, q.size() - 4800) == 0.0);
}

TEST_CASE(reverb_cuts_and_shimmer_adds_upper_octave) {
    DeviceRig r("reverb", kReverb, kRate, 256);
    r.set("mix", 100.f);
    r.set("dry", 0.f);
    r.set("highcut2", 500.f);
    const auto lo = r.run_mono(noise(48000, 0.5f, 3));
    DeviceRig open("reverb", kReverb, kRate, 256);
    open.set("mix", 100.f);
    open.set("dry", 0.f);
    const auto full = open.run_mono(noise(48000, 0.5f, 3));
    CHECK(rms(lo, 24000) < 0.7 * rms(full, 24000));

    auto readout_band = [](float mode) {
        DeviceRig s("reverb", kReverb, kRate, 256);
        s.set("mode2", mode);
        s.set("mix", 100.f);
        s.set("dry", 0.f);
        s.set("decay", 6.f);
        s.set("shimmer", 100.f);
        s.set("shift2", 0.f);
        s.run_mono(tone(48000 * 2, 300.0, kRate, 0.3));
        return s.readouts();
    };
    // The spectrum band nearest the shimmer's +1 octave (600 Hz).
    uint32_t b = 0;
    while (b + 1 < dstudio::dsp::kSpectrumBands && dstudio::dsp::spectrum_band_hz(b + 1) < 600.0f) ++b;
    const auto hall = readout_band(1.f), shim = readout_band(4.f);
    CHECK(shim[1 + b] > hall[1 + b] + 3.0f);
}

// ---------------------------------------------------------------- grains

TEST_CASE(reverb_shimmer_does_not_depend_on_the_block_split) {
    const Signal x = noise(48000, 0.5f, 21);
    const auto render = [&](uint32_t block) {
        DeviceRig r("reverb", kReverb, kRate, block);
        r.set("mode2", 4.f);
        r.set("shimmer", 100.f);
        return r.run(x, x).first;
    };
    const auto a = render(256), b = render(37);
    double diff = 0.0;
    for (std::size_t i = 0; i < a.size(); ++i) diff = std::max(diff, static_cast<double>(std::fabs(a[i] - b[i])));
    CHECK(diff < 1e-4);
}

TEST_CASE(grains_play_the_input_and_mix_zero_is_dry) {
    DeviceRig r("grains", kGrains, kRate, 256);
    r.set("mix", 100.f);
    const auto y = r.run_mono(tone(48000, 440.0, kRate, 0.5));
    CHECK(rms(y, 24000) > 0.05);
    CHECK(r.readouts()[1] > 0.0f);
    DeviceRig d("grains", kGrains, kRate, 256);
    d.set("mix", 0.f);
    const Signal x = noise(24000, 0.5f, 4);
    const auto z = d.run_mono(x);
    CHECK(std::fabs(z[20000] - x[20000]) < 1e-6f);
    DeviceRig s("grains", kGrains, kRate, 256);
    s.set("mix", 100.f);
    CHECK(peak(s.run_mono(Signal(48000, 0.0f))) == 0.0);
}

TEST_CASE(grains_freeze_holds_the_sound_after_the_input_stops) {
    DeviceRig r("grains", kGrains, kRate, 256);
    r.set("mix", 100.f);
    r.run_mono(tone(48000, 440.0, kRate, 0.5));
    r.set("freeze", 1.f);
    const auto held = r.run_mono(Signal(48000 * 2, 0.0f));
    CHECK(rms(held, 48000) > 0.05);
    DeviceRig o("grains", kGrains, kRate, 256);
    o.set("mix", 100.f);
    o.run_mono(tone(48000, 440.0, kRate, 0.5));
    const auto gone = o.run_mono(Signal(48000 * 2, 0.0f));
    CHECK(rms(gone, 48000) < 1e-4);
}

TEST_CASE(grains_frozen_fast_grains_stay_in_the_capture) {
    // A 2 s grain two octaves up reads 8 s; frozen, it must start far enough back to stay in the audio.
    DeviceRig r("grains", kGrains, kRate, 256);
    r.set("mix", 100.f);
    r.set("size", 2000.f);
    r.set("pitch", 24.f);
    r.set("spray", 0.f);
    r.run_mono(tone(48000 * 10, 200.0, kRate, 0.5));
    r.set("freeze", 1.f);
    const auto held = r.run_mono(Signal(48000 * 3, 0.0f));
    CHECK(rms(held, 48000, 48000 * 3) > 0.05);
    CHECK(peak(held) < 4.0);
}

TEST_CASE(grains_pitch_and_rhythm_steps) {
    DeviceRig r("grains", kGrains, kRate, 256);
    r.set("mix", 100.f);
    r.set("pitch", 12.f);
    r.set("spray", 0.f);
    r.set("shape", 100.f);
    r.set("size", 200.f);
    const auto y = r.run_mono(tone(48000 * 2, 300.0, kRate, 0.5));
    CHECK_NEAR(dominant_hz(y, 48000), 600.0, 12.0);

    DeviceRig h("grains", kGrains, kRate, 128);
    h.set("mode", 1.f);
    h.set("mix", 100.f);
    h.set("rate", 5.f); // 1/8 = 0.25 s at 120 BPM
    h.set("gate", 20.f);
    h.set("shape", 0.f);
    h.set("spray", 0.f);
    const auto z = h.run_mono(tone(48000 * 2, 300.0, kRate, 0.5));
    // Gated grains: 50 ms on, 200 ms off per step.
    CHECK(rms(z, 48000 + 3000, 48000 + 9000) < 0.2 * rms(z, 48000 + 12000 + 300, 48000 + 12000 + 2000) + 1e-4);
}
