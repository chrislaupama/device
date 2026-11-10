// Built-in synth parameters (se_synth_params, se_session_publish_v3):
// ABI layout, validation, oscillators, filter, envelopes, LFO, voice modes,
// smoothing of republished parameters, and RT safety with parameter changes.
//
// DSP tests mostly drive dstudio::Synth directly with a "clean" patch (one sine
// or one waveform, filter wide open, no key tracking) so that each property
// can be measured in isolation; engine-level tests go through
// se_session_publish_v3.
#include "synth.hpp"
#include "test_harness.hpp"
#include "test_util.hpp"

#include <algorithm>
#include <atomic>
#include <cmath>
#include <cstddef>
#include <numbers>
#include <thread>

using namespace testutil;
using dstudio::NoteEvent;

// ---------------------------------------------------------------- ABI layout
// Mirrors the Rust abi_layout test (crates/dstudio-engine/src/ffi.rs).
static_assert(sizeof(se_synth_params) == 128 && alignof(se_synth_params) == 4);
static_assert(offsetof(se_synth_params, osc1_wave) == 0 && offsetof(se_synth_params, osc2_wave) == 3);
static_assert(offsetof(se_synth_params, filter_mode) == 6 && offsetof(se_synth_params, lfo_wave) == 7);
static_assert(offsetof(se_synth_params, polyphony) == 8 && offsetof(se_synth_params, unison) == 9);
static_assert(offsetof(se_synth_params, _pad) == 10 && offsetof(se_synth_params, osc1_shape) == 16);
static_assert(offsetof(se_synth_params, osc2_shape) == 28 && offsetof(se_synth_params, cross_fm) == 40);
static_assert(offsetof(se_synth_params, filter_cutoff_hz) == 52 && offsetof(se_synth_params, amp_attack) == 68);
static_assert(offsetof(se_synth_params, filter_attack) == 84 && offsetof(se_synth_params, lfo_rate_hz) == 100);
static_assert(offsetof(se_synth_params, glide) == 116 && offsetof(se_synth_params, output) == 124);
static_assert(sizeof(se_track_desc_v3) == 56 && alignof(se_track_desc_v3) == 8);
static_assert(offsetof(se_track_desc_v3, notes) == 40 && offsetof(se_track_desc_v3, synth) == 48);
static_assert(sizeof(se_session_desc_v3) == 16);

namespace {

constexpr double kRate = 48000.0;
constexpr double kPi = std::numbers::pi;

double note_hz(double pitch) { return 440.0 * std::exp2((pitch - 69.0) / 12.0); }
double db(double r) { return 20.0 * std::log10(r); }

// One oscillator, filter wide open (LP 20 kHz, no key tracking, Butterworth),
// near-instant attack, sustain 1.
se_synth_params clean(uint8_t wave = SE_WAVE_SINE) {
    se_synth_params p{};
    se_synth_params_default(&p);
    p.osc1_wave = wave;
    p.osc1_fine_cents = 0.0f;
    p.osc1_level = 1.0f;
    p.osc2_level = 0.0f;
    p.osc2_fine_cents = 0.0f;
    p.filter_cutoff_hz = 20000.0f;
    p.filter_key_track = 0.0f;
    p.filter_resonance = 0.0f;
    p.amp_attack = 0.001f;
    p.amp_decay = 0.001f;
    p.amp_sustain = 1.0f;
    p.amp_release = 0.05f;
    return p;
}

NoteEvent ev(int64_t start, int64_t end, uint8_t pitch, uint8_t vel = 127) { return NoteEvent{start, end, pitch, vel}; }

// Renders `notes` through a fresh Synth in blocks of `block`.
std::vector<float> run(const se_synth_params& p, std::vector<NoteEvent> notes, uint32_t frames,
                       uint32_t block = 256) {
    std::sort(notes.begin(), notes.end(), [](const NoteEvent& a, const NoteEvent& b) {
        return a.start != b.start ? a.start < b.start : a.pitch < b.pitch;
    });
    dstudio::Synth s(kRate);
    std::vector<float> out(frames, 0.0f);
    for (uint32_t pos = 0; pos < frames; pos += block) {
        const uint32_t n = std::min(block, frames - pos);
        s.render(notes.data(), static_cast<uint32_t>(notes.size()), &p, 1, 0, pos, n, true, out.data() + pos);
    }
    return out;
}

// Amplitude of the sinusoid at `hz` in x[from, from + n) (Hann window).
double tone_amp(const std::vector<float>& x, std::size_t from, std::size_t n, double hz) {
    double re = 0.0, im = 0.0, wsum = 0.0;
    const double w0 = 2.0 * kPi * hz / kRate;
    for (std::size_t i = 0; i < n; ++i) {
        const double w = 0.5 - 0.5 * std::cos(2.0 * kPi * static_cast<double>(i) / static_cast<double>(n));
        const double ph = w0 * static_cast<double>(from + i);
        re += w * x[from + i] * std::cos(ph);
        im -= w * x[from + i] * std::sin(ph);
        wsum += w;
    }
    return 2.0 * std::sqrt(re * re + im * im) / wsum;
}

// Harmonic k amplitude relative to the fundamental, in dB.
double harmonic_db(const std::vector<float>& x, double f0, int k) {
    const double h1 = tone_amp(x, 4800, 24000, f0);
    return db(tone_amp(x, 4800, 24000, f0 * k) / h1);
}

// Upward zero crossings (linearly interpolated) in [from, to).
std::vector<double> crossings(const std::vector<float>& x, std::size_t from, std::size_t to) {
    std::vector<double> c;
    for (std::size_t i = std::max<std::size_t>(from, 1); i < to; ++i)
        if (x[i - 1] < 0.0f && x[i] >= 0.0f)
            c.push_back(static_cast<double>(i - 1) + x[i - 1] / static_cast<double>(x[i - 1] - x[i]));
    return c;
}

// Mean frequency from zero crossings (pure tones only).
double zc_hz(const std::vector<float>& x, std::size_t from, std::size_t to) {
    const auto c = crossings(x, from, to);
    if (c.size() < 2) return 0.0;
    return static_cast<double>(c.size() - 1) * kRate / (c.back() - c.front());
}

// Per-period frequency (pure tones only): {time of period start, Hz}.
std::vector<std::pair<double, double>> period_hz(const std::vector<float>& x, std::size_t from, std::size_t to) {
    const auto c = crossings(x, from, to);
    std::vector<std::pair<double, double>> f;
    for (std::size_t i = 1; i < c.size(); ++i) f.emplace_back(c[i - 1], kRate / (c[i] - c[i - 1]));
    return f;
}

double hz_at(const std::vector<std::pair<double, double>>& f, double t) {
    for (const auto& [at, hz] : f)
        if (at >= t) return hz;
    return 0.0;
}

double peak(const std::vector<float>& x, std::size_t from, std::size_t n) {
    double m = 0.0;
    for (std::size_t i = from; i < std::min(from + n, x.size()); ++i) m = std::max(m, std::fabs(double{x[i]}));
    return m;
}

double rms(const std::vector<float>& x, std::size_t from, std::size_t n) {
    double s = 0.0;
    for (std::size_t i = from; i < from + n; ++i) s += static_cast<double>(x[i]) * x[i];
    return std::sqrt(s / static_cast<double>(n));
}

float max_jump(const std::vector<float>& x, std::size_t from, std::size_t to) {
    float m = 0.0f;
    for (std::size_t i = std::max<std::size_t>(from, 1); i < std::min(to, x.size()); ++i)
        m = std::max(m, std::fabs(x[i] - x[i - 1]));
    return m;
}

// Full-scale sine amplitude of one voice at velocity 127, output 1.
constexpr double kFull = dstudio::Synth::kVoiceGain;

se_note_desc note(int64_t start, int64_t end, uint8_t pitch, uint8_t velocity = 100) {
    se_note_desc n{};
    n.start = start;
    n.end = end;
    n.pitch = pitch;
    n.velocity = velocity;
    return n;
}

// Owns v3 session storage.
struct V3Session {
    struct T {
        se_track_desc_v3 d{};
        std::vector<se_note_desc> notes;
        se_synth_params params{};
        bool has_params = false;
    };
    std::vector<T> tracks;
    float master_gain = 1.0f;

    T& add(uint64_t id, std::vector<se_note_desc> notes, const se_synth_params* p, float gain = 1.0f,
           float pan = -1.0f) {
        T t;
        t.d.track_id = id;
        t.d.gain = gain;
        t.d.pan = pan;
        t.d.instrument = 1;
        t.notes = std::move(notes);
        if (p != nullptr) {
            t.params = *p;
            t.has_params = true;
        }
        tracks.push_back(std::move(t));
        return tracks.back();
    }
    int32_t publish(se_engine* e) {
        std::vector<se_track_desc_v3> ds;
        for (T& t : tracks) {
            t.d.notes = t.notes.data();
            t.d.note_count = static_cast<uint32_t>(t.notes.size());
            t.d.synth = t.has_params ? &t.params : nullptr;
            ds.push_back(t.d);
        }
        se_session_desc_v3 s{ds.data(), static_cast<uint32_t>(ds.size()), master_gain};
        return se_session_publish_v3(e, &s);
    }
};

std::vector<float> left(const std::vector<float>& inter) {
    std::vector<float> l(inter.size() / 2);
    for (std::size_t i = 0; i < l.size(); ++i) l[i] = inter[2 * i];
    return l;
}

} // namespace

// ---------------------------------------------------------------- ABI / validation

TEST_CASE(synth_v3_defaults_and_validation) {
    se_synth_params d{};
    se_synth_params_default(&d);
    se_synth_params_default(nullptr); // no-op
    // Same values as dstudio_core::instrument::SynthParams::default().
    CHECK(d.osc1_wave == SE_WAVE_SAW && d.osc2_wave == SE_WAVE_SQUARE);
    CHECK(d.osc1_fine_cents == -5.0f && d.osc2_fine_cents == 5.0f && d.osc1_level == 0.6f && d.osc2_level == 0.5f);
    CHECK(d.filter_mode == SE_FILTER_LOWPASS && d.filter_cutoff_hz == 2093.0f && d.filter_key_track == 1.0f);
    CHECK(d.polyphony == 16 && d.unison == 1 && d.output == 1.0f && d.amp_sustain == 0.6f);
    CHECK(dstudio::validate_synth_params(d) == nullptr);

    EngineHandle e(kRate, 256);
    auto publish_with = [&](auto mutate) {
        se_synth_params p = d;
        mutate(p);
        V3Session s;
        s.add(1, {note(0, 100, 60)}, &p);
        return s.publish(e);
    };
    CHECK_EQ(publish_with([](se_synth_params&) {}), SE_OK);
    CHECK_EQ(publish_with([](se_synth_params& p) { p._pad[3] = 0xAB; }), SE_OK); // reserved bytes ignored
    CHECK_EQ(publish_with([](se_synth_params& p) { p.osc1_wave = 4; }), SE_ERR_INVALID_ARG);
    CHECK_EQ(publish_with([](se_synth_params& p) { p.lfo_wave = 9; }), SE_ERR_INVALID_ARG);
    CHECK_EQ(publish_with([](se_synth_params& p) { p.filter_mode = 3; }), SE_ERR_INVALID_ARG);
    CHECK_EQ(publish_with([](se_synth_params& p) { p.polyphony = 0; }), SE_ERR_INVALID_ARG);
    CHECK_EQ(publish_with([](se_synth_params& p) { p.polyphony = 17; }), SE_ERR_INVALID_ARG);
    CHECK_EQ(publish_with([](se_synth_params& p) { p.unison = 5; }), SE_ERR_INVALID_ARG);
    CHECK_EQ(publish_with([](se_synth_params& p) { p.osc2_octave = 4; }), SE_ERR_INVALID_ARG);
    CHECK_EQ(publish_with([](se_synth_params& p) { p.osc1_semitones = -13; }), SE_ERR_INVALID_ARG);
    CHECK_EQ(publish_with([](se_synth_params& p) { p.filter_cutoff_hz = 10.0f; }), SE_ERR_INVALID_ARG);
    CHECK_EQ(publish_with([](se_synth_params& p) { p.filter_cutoff_hz = NAN; }), SE_ERR_INVALID_ARG);
    CHECK_EQ(publish_with([](se_synth_params& p) { p.amp_release = 0.0f; }), SE_ERR_INVALID_ARG);
    CHECK_EQ(publish_with([](se_synth_params& p) { p.output = 2.5f; }), SE_ERR_INVALID_ARG);
    CHECK_EQ(publish_with([](se_synth_params& p) { p.lfo_to_pitch_cents = INFINITY; }), SE_ERR_INVALID_ARG);
    CHECK_EQ(publish_with([](se_synth_params& p) { p.filter_env_amount = -1.0f; }), SE_OK);
    CHECK(se_last_error() != nullptr);
    CHECK_EQ(se_session_publish_v3(e, nullptr), SE_ERR_INVALID_ARG);
}

TEST_CASE(synth_v2_v3_null_and_v3_defaults_render_identically) {
    const std::vector<se_note_desc> notes = {note(100, 20000, 60), note(5000, 30000, 67, 80)};
    auto run_v3 = [&](bool explicit_defaults) {
        EngineHandle e(kRate, 256);
        se_synth_params d{};
        se_synth_params_default(&d);
        V3Session s;
        s.add(1, notes, explicit_defaults ? &d : nullptr);
        REQUIRE(s.publish(e) == SE_OK);
        REQUIRE(se_transport_play(e) == SE_OK);
        return render(e, 40000, 256);
    };
    EngineHandle e(kRate, 256);
    se_track_desc_v2 t{};
    t.track_id = 1;
    t.gain = 1.0f;
    t.pan = -1.0f;
    t.instrument = 1;
    t.notes = notes.data();
    t.note_count = static_cast<uint32_t>(notes.size());
    se_session_desc_v2 s2{&t, 1, 1.0f};
    REQUIRE(se_session_publish_v2(e, &s2) == SE_OK);
    REQUIRE(se_transport_play(e) == SE_OK);
    const auto v2 = render(e, 40000, 256);
    CHECK(v2 == run_v3(false));
    CHECK(v2 == run_v3(true));
    CHECK(rms(left(v2), 1000, 10000) > 0.01);
}

// ---------------------------------------------------------------- oscillators

TEST_CASE(synth_osc_waveform_spectra) {
    const double f = note_hz(57); // 220 Hz
    {
        const auto x = run(clean(SE_WAVE_SINE), {ev(0, 48000, 57)}, 30000);
        CHECK_NEAR(tone_amp(x, 4800, 24000, f), kFull, 0.002);
        for (int k = 2; k <= 6; ++k) CHECK(harmonic_db(x, f, k) < -80.0); // near-pure
    }
    {
        const auto x = run(clean(SE_WAVE_SQUARE), {ev(0, 48000, 57)}, 30000);
        for (int k : {2, 4, 6}) CHECK(harmonic_db(x, f, k) < -40.0); // odd harmonics only
        CHECK_NEAR(harmonic_db(x, f, 3), db(1.0 / 3.0), 0.3);
        CHECK_NEAR(harmonic_db(x, f, 5), db(1.0 / 5.0), 0.3);
    }
    {
        const auto x = run(clean(SE_WAVE_SAW), {ev(0, 48000, 57)}, 30000);
        CHECK_NEAR(harmonic_db(x, f, 2), db(1.0 / 2.0), 0.3); // 1/n, all harmonics
        CHECK_NEAR(harmonic_db(x, f, 3), db(1.0 / 3.0), 0.3);
        CHECK_NEAR(harmonic_db(x, f, 4), db(1.0 / 4.0), 0.3);
    }
    {
        const auto x = run(clean(SE_WAVE_TRIANGLE), {ev(0, 48000, 57)}, 30000);
        CHECK(harmonic_db(x, f, 2) < -40.0);
        CHECK_NEAR(harmonic_db(x, f, 3), db(1.0 / 9.0), 0.3); // 1/n^2, odd only
        CHECK_NEAR(harmonic_db(x, f, 5), db(1.0 / 25.0), 0.5);
    }
}

TEST_CASE(synth_osc_shape) {
    const double f = note_hz(57);
    // Square: shape 0.6 -> pulse width 0.5 - 0.45 * 0.6 = 23%. Harmonic n of
    // a pulse is proportional to |sin(pi n pw)| / n.
    {
        se_synth_params p = clean(SE_WAVE_SQUARE);
        p.osc1_shape = 0.6f;
        const auto x = run(p, {ev(0, 48000, 57)}, 30000);
        const double pw = 0.5 - 0.45 * 0.6;
        for (int k : {2, 3, 4})
            CHECK_NEAR(harmonic_db(x, f, k),
                       db(std::fabs(std::sin(kPi * k * pw)) / (k * std::sin(kPi * pw))), 0.4);
        // 5%: the most extreme width still sounds and has strong even harmonics.
        p.osc1_shape = 1.0f;
        const auto y = run(p, {ev(0, 48000, 57)}, 30000);
        CHECK(harmonic_db(y, f, 2) > -1.0);
    }
    // Saw shape 1 = triangle, triangle shape 1 = saw.
    {
        se_synth_params p = clean(SE_WAVE_SAW);
        p.osc1_shape = 1.0f;
        const auto x = run(p, {ev(0, 48000, 57)}, 30000);
        CHECK(harmonic_db(x, f, 2) < -40.0);
        CHECK_NEAR(harmonic_db(x, f, 3), db(1.0 / 9.0), 0.3);
        p.osc1_wave = SE_WAVE_TRIANGLE;
        const auto y = run(p, {ev(0, 48000, 57)}, 30000);
        CHECK_NEAR(harmonic_db(y, f, 2), db(1.0 / 2.0), 0.3);
        // Half way: in between.
        p.osc1_shape = 0.5f;
        const auto z = run(p, {ev(0, 48000, 57)}, 30000);
        const double h2 = harmonic_db(z, f, 2);
        std::fprintf(stderr, "  triangle shape 0.5: 2nd harmonic %.1f dB\n", h2);
        CHECK(h2 > -30.0 && h2 < db(1.0 / 2.0) - 1.0);
    }
    // Sine fold: odd harmonics appear, grow with shape; even stay absent.
    {
        se_synth_params p = clean(SE_WAVE_SINE);
        p.osc1_shape = 0.2f;
        const double h3_low = harmonic_db(run(p, {ev(0, 48000, 57)}, 30000), f, 3);
        p.osc1_shape = 0.7f;
        const auto x = run(p, {ev(0, 48000, 57)}, 30000);
        const double h3_high = std::max(harmonic_db(x, f, 3), harmonic_db(x, f, 5));
        CHECK(h3_low > -60.0);
        CHECK(h3_high > -12.0);
        CHECK(h3_high > h3_low + 10.0);
        CHECK(harmonic_db(x, f, 2) < -60.0);
    }
}

TEST_CASE(synth_pitch_octave_semitone_fine) {
    struct Case {
        int osc; // 1 or 2
        int8_t octave, semis;
        float fine;
        uint8_t pitch;
    };
    for (const Case c : {Case{1, 0, 0, 0.0f, 69}, Case{1, 1, 7, 50.0f, 57}, Case{1, -2, -5, -30.0f, 81},
                         Case{2, 0, 0, 0.0f, 60}, Case{2, -1, 12, 100.0f, 64}, Case{2, 3, -12, -100.0f, 45},
                         Case{1, -3, 0, 0.0f, 100}}) {
        se_synth_params p = clean(SE_WAVE_SINE);
        if (c.osc == 2) {
            p.osc1_level = 0.0f;
            p.osc2_level = 1.0f;
            p.osc2_wave = SE_WAVE_SINE;
            p.osc2_octave = c.octave;
            p.osc2_semitones = c.semis;
            p.osc2_fine_cents = c.fine;
        } else {
            p.osc1_octave = c.octave;
            p.osc1_semitones = c.semis;
            p.osc1_fine_cents = c.fine;
        }
        const auto x = run(p, {ev(0, 48000, c.pitch)}, 48000);
        const double want = note_hz(c.pitch + 12.0 * c.octave + c.semis + c.fine / 100.0);
        const double hz = zc_hz(x, 2400, 48000);
        if (std::fabs(hz - want) > 0.5) std::fprintf(stderr, "  osc%d: %.4f Hz, want %.4f\n", c.osc, hz, want);
        CHECK_NEAR(hz, want, 0.5);
    }
}

TEST_CASE(synth_sub_noise_and_cross_fm) {
    const double f = note_hz(69);
    // Sub: a sine one octave below osc 1.
    {
        se_synth_params p = clean(SE_WAVE_SAW);
        p.osc1_level = 0.0f;
        p.sub_level = 1.0f;
        const auto x = run(p, {ev(0, 48000, 69)}, 30000);
        CHECK_NEAR(tone_amp(x, 4800, 24000, f / 2.0), kFull, 0.003);
        CHECK(tone_amp(x, 4800, 24000, f) < 1e-4);
        CHECK_NEAR(zc_hz(x, 2400, 30000), f / 2.0, 0.1);
    }
    // Noise: broadband, roughly uncorrelated sample to sample.
    {
        se_synth_params p = clean();
        p.osc1_level = 0.0f;
        p.noise_level = 1.0f;
        const auto x = run(p, {ev(0, 48000, 69)}, 24000);
        const double r = rms(x, 2400, 20000);
        double c1 = 0.0;
        for (std::size_t i = 2401; i < 22400; ++i) c1 += static_cast<double>(x[i]) * x[i - 1];
        c1 /= 20000.0 * r * r;
        std::fprintf(stderr, "  noise rms %.4f, lag-1 correlation %.3f\n", r, c1);
        CHECK(r > 0.1 && r < 0.2); // uniform +-1 x 0.25 -> 0.144 rms
        CHECK(std::fabs(c1) < 0.3); // the 20 kHz low-pass correlates neighbours a little
        CHECK(tone_amp(x, 2400, 20000, f) < 0.01);
    }
    // Cross FM (osc2 sine, silent, at the same pitch, modulating osc1 sine):
    // sidebands at multiples of f appear; the pitch stays.
    {
        se_synth_params p = clean(SE_WAVE_SINE);
        p.osc2_wave = SE_WAVE_SINE;
        const auto dry = run(p, {ev(0, 48000, 69)}, 30000);
        p.cross_fm = 1.0f;
        const auto x = run(p, {ev(0, 48000, 69)}, 30000);
        CHECK(harmonic_db(dry, f, 2) < -80.0);
        const double h2 = harmonic_db(x, f, 2);
        std::fprintf(stderr, "  cross_fm 1: 2nd harmonic %.1f dB\n", h2);
        CHECK(h2 > -14.0);
        CHECK(tone_amp(x, 4800, 24000, f * 1.5) < 1e-3); // harmonic spectrum (no inharmonic sidebands)
        p.cross_fm = 0.3f;
        const double h2_low = harmonic_db(run(p, {ev(0, 48000, 69)}, 30000), f, 2);
        CHECK(h2_low < h2 - 6.0 && h2_low > -40.0);
    }
}

// ---------------------------------------------------------------- filter

TEST_CASE(synth_filter_modes_attenuation) {
    // Sine tone through the filter (no key tracking), amplitude vs. unfiltered.
    auto gain_db = [](uint8_t mode, float cutoff, uint8_t pitch, float resonance = 0.0f) {
        se_synth_params p = clean(SE_WAVE_SINE);
        p.filter_mode = mode;
        p.filter_cutoff_hz = cutoff;
        p.filter_resonance = resonance;
        const auto x = run(p, {ev(0, 48000, pitch)}, 30000);
        return db(tone_amp(x, 4800, 24000, note_hz(pitch)) / kFull);
    };
    // 12 dB/oct Butterworth: |H| at 2 octaves = 1 / sqrt(1 + 4^4) = -24.1 dB.
    CHECK_NEAR(gain_db(SE_FILTER_LOWPASS, 220.0f, 81), -24.1, 0.5);
    CHECK_NEAR(gain_db(SE_FILTER_LOWPASS, 880.0f, 57), 0.0, 0.3);
    CHECK_NEAR(gain_db(SE_FILTER_LOWPASS, 440.0f, 69), -3.01, 0.2);
    CHECK_NEAR(gain_db(SE_FILTER_HIGHPASS, 880.0f, 57), -24.1, 0.5);
    CHECK_NEAR(gain_db(SE_FILTER_HIGHPASS, 220.0f, 81), 0.0, 0.3);
    CHECK_NEAR(gain_db(SE_FILTER_HIGHPASS, 440.0f, 69), -3.01, 0.2);
    // Band-pass, unity at the centre, Q 0.707: -3.3 dB at 1 octave, -9 dB at 2.
    CHECK_NEAR(gain_db(SE_FILTER_BANDPASS, 440.0f, 69), 0.0, 0.2);
    CHECK_NEAR(gain_db(SE_FILTER_BANDPASS, 440.0f, 81), -3.3, 0.3);
    CHECK_NEAR(gain_db(SE_FILTER_BANDPASS, 440.0f, 93), -9.0, 0.5);
    CHECK_NEAR(gain_db(SE_FILTER_BANDPASS, 440.0f, 45), -9.0, 0.5);
    // Resonance raises the response at the cutoff: Q = 0.707 * 35.36^res
    // (res 0.5 -> Q 4.2 = +12.5 dB, res 1 -> Q 25 = +28 dB); far below the
    // cutoff it changes little.
    CHECK_NEAR(gain_db(SE_FILTER_LOWPASS, 440.0f, 69, 0.5f), 12.47, 0.5);
    CHECK_NEAR(gain_db(SE_FILTER_LOWPASS, 440.0f, 69, 1.0f), 27.96, 0.5);
    CHECK_NEAR(gain_db(SE_FILTER_LOWPASS, 1760.0f, 45, 1.0f), 0.0, 0.5);
    // Cutoff is clamped below Nyquist: 20 kHz at a 22.05 kHz-ish rate stays stable.
    {
        se_synth_params p = clean(SE_WAVE_SAW);
        p.filter_cutoff_hz = 20000.0f;
        p.filter_key_track = 1.0f;
        p.filter_resonance = 1.0f;
        p.filter_env_amount = 1.0f;
        p.filter_sustain = 1.0f;
        const auto x = run(p, {ev(0, 48000, 127)}, 24000);
        for (float v : x) CHECK(std::isfinite(v) && std::fabs(v) < 100.0f);
    }
}

TEST_CASE(synth_filter_key_tracking_and_envelope) {
    // Key tracking: cutoff_hz is the cutoff at C4. A5 (880 Hz) with
    // cutoff_hz = C4 and full tracking -> cutoff 880 Hz -> -3 dB.
    auto amp_db = [](const se_synth_params& p, uint8_t pitch, std::size_t from = 9600, std::size_t n = 9600) {
        const auto x = run(p, {ev(0, 48000, pitch)}, 30000);
        return db(tone_amp(x, from, n, note_hz(pitch)) / kFull);
    };
    se_synth_params p = clean(SE_WAVE_SINE);
    p.filter_cutoff_hz = static_cast<float>(note_hz(60));
    p.filter_key_track = 1.0f;
    CHECK_NEAR(amp_db(p, 81), -3.01, 0.3);
    CHECK_NEAR(amp_db(p, 45), -3.01, 0.3);
    p.filter_key_track = 0.5f; // cutoff half way (1.75 octaves for A5) -> 21 / 2 semitones
    CHECK_NEAR(amp_db(p, 81), db(1.0 / std::sqrt(1.0 + std::pow(std::exp2(10.5 / 12.0), 4.0))), 0.3);
    p.filter_key_track = 0.0f; // 880 / 261.6 = 1.75 octaves above: about -21 dB
    CHECK_NEAR(amp_db(p, 81), db(1.0 / std::sqrt(1.0 + std::pow(880.0 / note_hz(60), 4.0))), 0.4);

    // Filter envelope: env_amount 0.5 = +3 octaves at full envelope. With a
    // held envelope (sustain 1) a 110 Hz cutoff opens to 880 Hz -> -3 dB at
    // A5; env_amount -0.5 closes 7040 Hz down to 880 Hz.
    p = clean(SE_WAVE_SINE);
    p.filter_attack = 0.001f;
    p.filter_decay = 10.0f;
    p.filter_sustain = 1.0f;
    p.filter_cutoff_hz = 110.0f;
    CHECK(amp_db(p, 81) < -34.0);
    p.filter_env_amount = 0.5f;
    CHECK_NEAR(amp_db(p, 81), -3.01, 0.3);
    p.filter_cutoff_hz = 7040.0f;
    p.filter_env_amount = -0.5f;
    CHECK_NEAR(amp_db(p, 81), -3.01, 0.3);
    p.filter_env_amount = 1.0f; // 6 octaves: 7040 * 64 -> clamped below Nyquist
    CHECK_NEAR(amp_db(p, 81), 0.0, 0.3);
    // A decaying filter envelope sweeps the cutoff down over time.
    p = clean(SE_WAVE_SINE);
    p.filter_cutoff_hz = 110.0f;
    p.filter_env_amount = 0.5f;
    p.filter_attack = 0.001f;
    p.filter_decay = 0.3f;
    p.filter_sustain = 0.0f;
    const double early = amp_db(p, 81, 240, 480);
    const double late = amp_db(p, 81, 24000, 4800);
    std::fprintf(stderr, "  filter env sweep: early %.1f dB, late %.1f dB\n", early, late);
    CHECK(early > -10.0); // 5..15 ms: cutoff already ~2 dB of 60 dB into its decay
    CHECK(late < -30.0);
}

// ---------------------------------------------------------------- envelopes, LFO

TEST_CASE(synth_amp_envelope_timing) {
    se_synth_params p = clean(SE_WAVE_SINE);
    p.amp_attack = 0.1f;
    p.amp_decay = 0.2f;
    p.amp_sustain = 0.5f;
    p.amp_release = 0.3f;
    const auto x = run(p, {ev(0, 24000, 81)}, 48000); // 880 Hz, note-off at 0.5 s
    auto env = [&](double seconds) { return peak(x, static_cast<std::size_t>(seconds * kRate) - 48, 96) / kFull; };
    CHECK_NEAR(env(0.050), 0.5, 0.03); // linear attack
    CHECK_NEAR(env(0.100), 1.0, 0.02);
    CHECK_NEAR(env(0.150), 0.5 + 0.5 * std::pow(1e-3, 0.25), 0.03); // exponential decay, 60 dB per decay time
    CHECK_NEAR(env(0.300), 0.5, 0.01);
    CHECK_NEAR(env(0.450), 0.5, 0.01);                // sustain
    CHECK_NEAR(env(0.600), 0.5 * 0.1, 0.006);         // release: -20 dB after 1/3 of the release time
    const double tail = env(0.795);
    CHECK(tail < 0.5 * 2e-3 && tail > 0.5 * 4e-4); // ~ -60 dB at the release time
    // Filter envelope timing shows through a cutoff sweep (attack 0.1 s, +3
    // octaves from 110 Hz): the tone at 880 Hz reaches -3 dB at the peak.
    se_synth_params q = clean(SE_WAVE_SINE);
    q.filter_cutoff_hz = 110.0f;
    q.filter_env_amount = 0.5f;
    q.filter_attack = 0.1f;
    q.filter_decay = 10.0f;
    q.filter_sustain = 1.0f;
    const auto y = run(q, {ev(0, 48000, 81)}, 12000);
    CHECK(peak(y, 1200, 96) / kFull < 0.1);
    CHECK_NEAR(peak(y, 4800 + 960, 96) / kFull, std::sqrt(0.5), 0.03);
}

TEST_CASE(synth_lfo_vibrato_tremolo_cutoff) {
    // Vibrato: +-100 cents at 5 Hz on a 440 Hz sine.
    {
        se_synth_params p = clean(SE_WAVE_SINE);
        p.lfo_rate_hz = 5.0f;
        p.lfo_to_pitch_cents = 100.0f;
        const auto x = run(p, {ev(0, 96000, 69)}, 72000);
        double lo = 1e9, hi = 0.0;
        for (const auto& [t, hz] : period_hz(x, 2400, 72000)) lo = std::min(lo, hz), hi = std::max(hi, hz);
        std::fprintf(stderr, "  vibrato %.2f .. %.2f Hz\n", lo, hi);
        CHECK_NEAR(hi, 440.0 * std::exp2(1.0 / 12.0), 2.0);
        CHECK_NEAR(lo, 440.0 * std::exp2(-1.0 / 12.0), 2.0);
        CHECK_NEAR(zc_hz(x, 2400, 2400 + 48000), 440.0, 1.0); // centred (whole LFO periods)
        // Per-voice LFO, phase 0 at note-on: the first quarter period goes up.
        const auto f = period_hz(x, 0, 4000);
        CHECK(hz_at(f, 2000) > 445.0);
    }
    // Tremolo: depth d -> gain between 1 - d and 1.
    for (float depth : {0.5f, 1.0f}) {
        se_synth_params p = clean(SE_WAVE_SINE);
        p.lfo_rate_hz = 2.0f;
        p.lfo_to_amp = depth;
        const auto x = run(p, {ev(0, 96000, 81)}, 72000);
        double lo = 1e9, hi = 0.0;
        for (std::size_t w = 2400; w + 96 < 72000; w += 96) {
            const double a = peak(x, w, 96) / kFull;
            lo = std::min(lo, a), hi = std::max(hi, a);
        }
        CHECK_NEAR(hi, 1.0, 0.02);
        CHECK_NEAR(lo, 1.0 - depth, 0.03);
    }
    // Cutoff: square LFO, +-2 octaves around 880 Hz on an 880 Hz tone:
    // first half period ~0 dB (cutoff 3.5 kHz), second half ~-24 dB (220 Hz).
    {
        se_synth_params p = clean(SE_WAVE_SINE);
        p.filter_cutoff_hz = 880.0f;
        p.lfo_wave = SE_WAVE_SQUARE;
        p.lfo_rate_hz = 1.0f;
        p.lfo_to_cutoff_oct = 2.0f;
        const auto x = run(p, {ev(0, 96000, 81)}, 48000);
        CHECK_NEAR(db(tone_amp(x, 4800, 14400, 880.0) / kFull), -0.3, 0.4);
        CHECK_NEAR(db(tone_amp(x, 28800, 14400, 880.0) / kFull), -24.1, 0.6);
    }
}

// ---------------------------------------------------------------- voices

TEST_CASE(synth_mono_legato_and_glide) {
    se_synth_params p = clean(SE_WAVE_SINE);
    p.polyphony = 1;
    p.glide = 0.2f;
    // Overlapping notes: A3 then A4 (legato): one voice glides 12 semitones
    // linearly in pitch over 0.2 s, without retriggering the envelope.
    {
        const auto x = run(p, {ev(0, 48000, 57), ev(24000, 72000, 69)}, 72000);
        const auto f = period_hz(x, 2400, 72000);
        CHECK_NEAR(hz_at(f, 20000), 220.0, 0.5);
        CHECK(hz_at(f, 24000 + 480) < 235.0);                                  // just started gliding
        CHECK_NEAR(12.0 * std::log2(hz_at(f, 24000 + 4800) / 220.0), 6.0, 0.3); // half way (6 semitones)
        CHECK_NEAR(hz_at(f, 24000 + 9600 + 1200), 440.0, 0.5);                 // arrived after ~0.2 s
        double t_arrive = 0.0;
        for (const auto& [t, hz] : f)
            if (t > 24000 && hz < 440.0 * std::exp2(-10.0 / 1200.0)) t_arrive = t;
        CHECK_NEAR((t_arrive - 24000) / kRate, 0.2, 0.02);
        // One voice, no retrigger: level stays at sustain through the legato.
        double lo = 1e9;
        for (std::size_t w = 20000; w < 40000; w += 96) lo = std::min(lo, peak(x, w, 120) / kFull);
        CHECK(lo > 0.97);
    }
    // Envelope retrigger rules: overlapping (legato) vs touching notes.
    {
        se_synth_params q = p;
        q.glide = 0.0f;
        q.amp_attack = 0.02f;
        q.amp_decay = 0.05f;
        q.amp_sustain = 0.5f;
        const auto legato = run(q, {ev(0, 30000, 57), ev(24000, 48000, 64)}, 40000);
        const auto touching = run(q, {ev(0, 24000, 57), ev(24000, 48000, 64)}, 40000);
        const double peak_legato = peak(legato, 24000, 4800) / kFull;
        const double peak_retrig = peak(touching, 24000, 4800) / kFull;
        std::fprintf(stderr, "  after note 2: legato peak %.3f, retrigger peak %.3f\n", peak_legato, peak_retrig);
        CHECK(peak_legato < 0.52);
        CHECK(peak_retrig > 0.95);
        // Without glide the pitch jumps at the note-on (within one control tick).
        const auto f = period_hz(legato, 20000, 30000);
        CHECK_NEAR(hz_at(f, 24000 + 64), note_hz(64), 0.5);
        // Retrigger restarts the attack from the current level: no click.
        // (bound: the steepest slope of the new note at full level)
        const double bound = 1.1 * kFull * 2.0 * kPi * note_hz(64) / kRate;
        CHECK(max_jump(touching, 23900, 24400) < bound);
    }
    // Mono: a chord collapses to one voice (last note wins).
    {
        dstudio::Synth s(kRate);
        std::vector<NoteEvent> chord = {ev(0, 48000, 60), ev(0, 48000, 64), ev(0, 48000, 67)};
        std::vector<float> buf(256);
        s.render(chord.data(), 3, &p, 1, 0, 0, 256, true, buf.data());
        CHECK_EQ(s.note_voices(), 1u);
        CHECK_EQ(s.active_slots(), 1u);
    }
    // Poly glide: a new voice glides from the previous note's pitch.
    {
        se_synth_params q = clean(SE_WAVE_SINE);
        q.glide = 0.1f;
        const auto x = run(q, {ev(0, 10000, 57), ev(24000, 48000, 69)}, 48000);
        const auto f = period_hz(x, 24000, 48000);
        CHECK(hz_at(f, 24000 + 240) < 240.0);
        CHECK_NEAR(12.0 * std::log2(hz_at(f, 24000 + 2400) / 220.0), 6.0, 0.5); // half way, in semitones
        CHECK_NEAR(hz_at(f, 24000 + 7200), 440.0, 0.5);
    }
}

TEST_CASE(synth_unison_detunes_and_keeps_level) {
    const double f = note_hz(69);
    se_synth_params p = clean(SE_WAVE_SINE);
    const auto one = run(p, {ev(0, 96000, 69)}, 96000);
    p.unison = 4;
    p.unison_detune_cents = 50.0f;
    const auto four = run(p, {ev(0, 96000, 69)}, 96000);
    // Stacks at -25, -8.3, +8.3, +25 cents, each at 1/sqrt(4) of the level.
    const double side = f * std::exp2(25.0 / 1200.0), side_lo = f * std::exp2(-25.0 / 1200.0);
    const double a1 = tone_amp(one, 4800, 90000, side), a4 = tone_amp(four, 4800, 90000, side);
    const double a4lo = tone_amp(four, 4800, 90000, side_lo);
    std::fprintf(stderr, "  +25 cents: unison 1 %.5f, unison 4 %.5f / %.5f\n", a1, a4, a4lo);
    CHECK(a1 < 0.002);
    CHECK_NEAR(a4, 0.5 * kFull, 0.02);
    CHECK_NEAR(a4lo, 0.5 * kFull, 0.02);
    // Level: normalised by 1/sqrt(n), so the RMS stays about the same while
    // the stacks beat against each other.
    const double r1 = rms(one, 4800, 90000), r4 = rms(four, 4800, 90000);
    CHECK(r4 > 0.8 * r1 && r4 < 1.25 * r1);
    double lo = 1e9, hi = 0.0;
    for (std::size_t w = 4800; w + 120 < 96000; w += 120) {
        const double a = peak(four, w, 120);
        lo = std::min(lo, a), hi = std::max(hi, a);
    }
    CHECK(hi > 1.5 * lo); // beating
    // Unison does not use extra voice slots.
    dstudio::Synth s(kRate);
    std::vector<NoteEvent> notes;
    for (int i = 0; i < 16; ++i) notes.push_back(ev(0, 48000, static_cast<uint8_t>(40 + i)));
    std::vector<float> buf(256);
    s.render(notes.data(), 16, &p, 1, 0, 0, 256, true, buf.data());
    CHECK_EQ(s.note_voices(), 16u);
    CHECK_EQ(s.active_slots(), 16u);
}

TEST_CASE(synth_polyphony_limit_and_reduction) {
    se_synth_params p = clean(SE_WAVE_SAW);
    p.polyphony = 4;
    dstudio::Synth s(kRate);
    std::vector<NoteEvent> notes;
    for (int i = 0; i < 8; ++i) notes.push_back(ev(i * 480, 1000000, static_cast<uint8_t>(48 + i)));
    std::vector<float> buf(64);
    uint32_t max_notes = 0;
    int64_t pos = 0;
    for (; pos < 9600; pos += 64) {
        s.render(notes.data(), 8, &p, 1, 0, pos, 64, true, buf.data());
        max_notes = std::max(max_notes, s.note_voices());
    }
    CHECK_EQ(max_notes, 4u);
    CHECK_EQ(s.note_voices(), 4u);
    CHECK_EQ(s.active_slots(), 4u); // stolen voices have faded
    CHECK_EQ(s.polyphony(), 4u);
    // Republish with polyphony 2: the two oldest held voices are released
    // (normal release, no cut), then freed.
    p.polyphony = 2;
    s.render(notes.data(), 8, &p, 2, 0, pos, 64, true, buf.data());
    pos += 64;
    CHECK_EQ(s.polyphony(), 2u);
    for (; pos < 9600 + 9600; pos += 64) s.render(notes.data(), 8, &p, 2, 0, pos, 64, true, buf.data());
    CHECK_EQ(s.note_voices(), 2u);
    CHECK_EQ(s.active_slots(), 2u);
}

// ---------------------------------------------------------------- republish / smoothing

TEST_CASE(synth_param_republish_is_smoothed) {
    // A held 220 Hz sine; every 3 blocks of 256 the output level jumps between
    // 0.2 and 2.0 and the cutoff between 300 Hz and 8 kHz (both extremes of a
    // fast knob sweep). Smoothed, the largest sample-to-sample jump stays
    // within the steady signal's own maximum at the loudest setting; a hard
    // switch would step by up to ~0.45.
    se_synth_params lo = clean(SE_WAVE_SINE), hi = clean(SE_WAVE_SINE);
    lo.output = 0.2f;
    lo.filter_cutoff_hz = 300.0f;
    hi.output = 2.0f;
    hi.filter_cutoff_hz = 8000.0f;
    const std::vector<se_note_desc> notes = {note(0, 10000000, 57, 127)};

    EngineHandle steady_e(kRate, 256);
    V3Session steady;
    steady.add(1, notes, &hi);
    REQUIRE(steady.publish(steady_e) == SE_OK);
    REQUIRE(se_transport_play(steady_e) == SE_OK);
    const auto ref = left(render(steady_e, 96000, 256));
    const float steady_jump = max_jump(ref, 4800, 96000);

    EngineHandle e(kRate, 256);
    V3Session s;
    s.add(1, notes, &lo);
    REQUIRE(s.publish(e) == SE_OK);
    REQUIRE(se_transport_play(e) == SE_OK);
    std::vector<float> out;
    for (int b = 0; b < 375; ++b) { // 2 s
        if (b % 3 == 0 && b < 250) {
            s.tracks[0].params = (b / 3) % 2 == 0 ? hi : lo;
            REQUIRE(s.publish(e) == SE_OK);
        }
        if (b == 250) { // then hold the loud setting
            s.tracks[0].params = hi;
            REQUIRE(s.publish(e) == SE_OK);
        }
        const auto blk = left(render(e, 256, 256));
        out.insert(out.end(), blk.begin(), blk.end());
    }
    const float sweep_jump = max_jump(out, 4800, out.size());
    std::fprintf(stderr, "  steady max jump %.5f, sweep max jump %.5f\n", static_cast<double>(steady_jump),
                 static_cast<double>(sweep_jump));
    CHECK(sweep_jump <= 1.1f * steady_jump);
    // The level really follows: after the hold it matches the steady render.
    CHECK_NEAR(rms(out, out.size() - 9600, 9600) / rms(ref, 48000, 9600), 1.0, 0.01);
    // ... and during the sweep it moved substantially.
    double lo_r = 1e9;
    for (std::size_t w = 4800; w + 768 < 250 * 256; w += 768) lo_r = std::min(lo_r, rms(out, w, 768));
    CHECK(lo_r < 0.5 * rms(ref, 48000, 9600));
    // Identical republishes change nothing (bit-identical to never republishing).
    EngineHandle e2(kRate, 256);
    REQUIRE(steady.publish(e2) == SE_OK);
    REQUIRE(se_transport_play(e2) == SE_OK);
    std::vector<float> same;
    for (int b = 0; b < 375; ++b) {
        if (b % 5 == 0) REQUIRE(steady.publish(e2) == SE_OK);
        const auto blk = left(render(e2, 256, 256));
        same.insert(same.end(), blk.begin(), blk.end());
    }
    CHECK(std::equal(same.begin(), same.end(), ref.begin()));
}

TEST_CASE(synth_params_change_sound_through_engine) {
    // Low cutoff via v3 removes high-frequency energy vs the defaults.
    auto hf = [](const se_synth_params* p) {
        EngineHandle e(kRate, 256);
        V3Session s;
        s.add(1, {note(0, 48000, 60, 110)}, p);
        REQUIRE(s.publish(e) == SE_OK);
        REQUIRE(se_transport_play(e) == SE_OK);
        const auto l = left(render(e, 24000, 256));
        // Second difference: a 12 dB/oct high-pass "brightness" measure.
        double d = 0.0, a = 0.0;
        for (std::size_t i = 4802; i < 24000; ++i) {
            const double d2 = static_cast<double>(l[i]) - 2.0 * l[i - 1] + l[i - 2];
            d += d2 * d2;
            a += static_cast<double>(l[i]) * l[i];
        }
        return std::sqrt(d / a);
    };
    se_synth_params dark{};
    se_synth_params_default(&dark);
    dark.filter_cutoff_hz = 200.0f;
    const double h_default = hf(nullptr), h_dark = hf(&dark);
    std::fprintf(stderr, "  HF ratio default %.4f, cutoff 200 Hz %.4f\n", h_default, h_dark);
    CHECK(h_dark < 0.3 * h_default);
}

// ---------------------------------------------------------------- RT safety

TEST_CASE(rt_synth_param_republish_does_not_allocate) {
    EngineHandle e(kRate, 256);
    auto make = [](int variant) {
        V3Session s;
        for (int t = 0; t < 8; ++t) {
            std::vector<se_note_desc> notes;
            for (int k = 0; k < 120; ++k) {
                const int64_t start = k * 307 + t * 11;
                notes.push_back(note(start, start + 500 + (k * 89) % 3000,
                                     static_cast<uint8_t>(36 + (k * 5 + t) % 48), static_cast<uint8_t>(30 + k % 90)));
            }
            se_synth_params p{};
            se_synth_params_default(&p);
            p.osc1_wave = static_cast<uint8_t>((variant + t) % 4);
            p.osc2_wave = static_cast<uint8_t>((variant + t + 1) % 4);
            p.osc1_shape = 0.1f * static_cast<float>(variant % 10);
            p.filter_mode = static_cast<uint8_t>((variant + t) % 3);
            p.filter_cutoff_hz = 200.0f + 900.0f * static_cast<float>(variant % 9);
            p.filter_resonance = 0.1f * static_cast<float>(variant % 10);
            p.filter_env_amount = variant % 2 ? 0.6f : -0.3f;
            p.cross_fm = 0.25f * static_cast<float>(variant % 5);
            p.sub_level = 0.3f;
            p.noise_level = 0.05f;
            p.lfo_wave = static_cast<uint8_t>(variant % 4);
            p.lfo_to_pitch_cents = 20.0f;
            p.lfo_to_cutoff_oct = 1.0f;
            p.lfo_to_amp = 0.3f;
            p.polyphony = static_cast<uint8_t>(t == 3 ? 1 : 1 + (variant * 5 + t) % 16);
            p.glide = t == 3 ? 0.05f : 0.0f;
            p.unison = static_cast<uint8_t>(1 + (variant + t) % 4);
            p.output = 0.5f + 0.1f * static_cast<float>(variant % 10);
            s.add(static_cast<uint64_t>(t + 1), std::move(notes), &p, 0.3f, 0.0f).d.mute = t == 5 ? 1 : 0;
        }
        return s;
    };
    V3Session s0 = make(0);
    REQUIRE(s0.publish(e) == SE_OK);
    REQUIRE(se_transport_set_loop(e, 1, 1000, 30000) == SE_OK);
    REQUIRE(se_transport_play(e) == SE_OK);
    std::vector<float> buf(2 * 8192);
    REQUIRE(se_render_offline(e, buf.data(), 256) == SE_OK); // warm-up

    std::atomic<bool> stop{false};
    std::atomic<int> publishes{0};
    std::thread publisher([&] {
        std::vector<V3Session> sessions;
        for (int v = 0; v < 6; ++v) sessions.push_back(make(v));
        int i = 0;
        while (!stop.load()) {
            sessions[static_cast<std::size_t>(i % 6)].publish(e);
            publishes.fetch_add(1);
            ++i;
            std::this_thread::yield();
        }
    });
    std::vector<V3Session> mine;
    for (int v = 6; v < 9; ++v) mine.push_back(make(v));

    const long allocs_before = rt_alloc_count();
    const long frees_before = rt_free_count();
    const uint32_t sizes[] = {256, 1, 333, 4096, 64, 255, 77};
    for (int i = 0; i < 1200; ++i) {
        if (i % 3 == 0) mine[static_cast<std::size_t>(i / 3) % 3].publish(e); // outside the RT scope
        if (i % 97 == 50) se_transport_seek(e, 2000 + (i * 31) % 20000);
        RtAllocGuard g;
        if (se_render_offline(e, buf.data(), sizes[i % 7]) != SE_OK) {
            CHECK(false);
            break;
        }
    }
    stop = true;
    publisher.join();
    CHECK_EQ(rt_alloc_count() - allocs_before, 0);
    CHECK_EQ(rt_free_count() - frees_before, 0);
    CHECK(publishes.load() > 10);
    for (float v : buf) CHECK(std::isfinite(v));
}
