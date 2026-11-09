// Shared sampler engine for the built-in Sampler, Pad Sampler and Drum Rack
// (docs/devices.md). Everything here is RT safe: no allocation, locks or I/O
// after prepare().
//
// Building blocks:
//  - read_frame: band-limited sample playback. A 16-tap Kaiser-windowed sinc
//    with 128 phases (linear between phases) and ten cutoff variants, picked
//    from the read rate so pitching up never aliases (rate r reads through a
//    table whose cutoff is at most 0.95 / r).
//  - Adsr: linear attack, exponential decay and release ("fall constant 5":
//    a stage of time T falls by e^-5 over T).
//  - Svf: Zavalishin's trapezoidal state-variable filter (LP 12, LP 24 as two
//    cascaded stages, HP 12, BP 12) with an anti-aliased (ADAA) drive stage.
//  - lfo_shape: sine, triangle, square, saw.
//  - Voice: one playing note: region (start, end, loop, reverse, fades),
//    pitch (key, transpose, glide, bend, LFO), warp (granular time stretch
//    with correlation-aligned grains so pitch and length are independent),
//    amp and filter envelopes, filter, pan and gain.
//  - VoicePool: voice allocation and stealing with a short declick.
#pragma once

#include "builtin.hpp"
#include "dsp.hpp"

#include <algorithm>
#include <array>
#include <cmath>
#include <cstdint>

namespace saine::sampler {

constexpr float kPi = 3.14159265358979323846f;

// ---------------------------------------------------------------- interpolation

constexpr int kTaps = 16;
constexpr int kHalf = kTaps / 2;
constexpr int kPhases = 128;
constexpr int kCutoffs = 10;

struct SincTables {
    float cutoff[kCutoffs];
    float coef[kCutoffs][kPhases + 1][kTaps];
};

// Built on first use; call from prepare() so the RT path never builds it.
const SincTables& sinc_tables() noexcept;

// Table index for a read rate (source frames per output frame).
inline int cutoff_index(const SincTables& t, double rate) noexcept {
    if (rate <= 1.0 + 1e-9) return 0; // full band (table 0 interpolates exactly at whole frames)
    const double want = 0.95 / rate;
    for (int k = 0; k < kCutoffs; ++k)
        if (static_cast<double>(t.cutoff[k]) <= want + 1e-6) return k;
    return kCutoffs - 1;
}

// One stereo frame at fractional position pos (outside 0..frames reads
// silence). ch1 may equal ch0 (mono).
inline void read_frame(const SincTables& t, int k, const float* ch0, const float* ch1, int64_t frames, double pos,
                       float& out0, float& out1) noexcept {
    const double fl = std::floor(pos);
    const auto i = static_cast<int64_t>(fl);
    if (fl == pos && k == 0) { // whole frame, full band: the sample itself
        const bool in = i >= 0 && i < frames;
        out0 = in ? ch0[i] : 0.0f;
        out1 = in ? ch1[i] : 0.0f;
        return;
    }
    const float ph = static_cast<float>(pos - fl) * static_cast<float>(kPhases);
    const int p = std::min(kPhases - 1, static_cast<int>(ph));
    const float f = ph - static_cast<float>(p);
    const float* c0 = t.coef[k][p];
    const float* c1 = t.coef[k][p + 1];
    float c[kTaps];
    for (int j = 0; j < kTaps; ++j) c[j] = c0[j] + f * (c1[j] - c0[j]);
    const int64_t first = i - (kHalf - 1);
    float a = 0.0f, b = 0.0f;
    if (first >= 0 && first + kTaps <= frames) {
        const float* s0 = ch0 + first;
        const float* s1 = ch1 + first;
        for (int j = 0; j < kTaps; ++j) {
            a += c[j] * s0[j];
            b += c[j] * s1[j];
        }
    } else {
        for (int j = 0; j < kTaps; ++j) {
            const int64_t n = first + j;
            if (n < 0 || n >= frames) continue;
            a += c[j] * ch0[n];
            b += c[j] * ch1[n];
        }
    }
    out0 = a;
    out1 = b;
}

// ---------------------------------------------------------------- envelope

class Adsr {
public:
    enum Stage : uint8_t { Idle, Attack, Decay, Sustain, Release };

    // Times in ms, sustain 0..1. Takes effect on the running stage too.
    void set(float attack_ms, float decay_ms, float sustain, float release_ms, double sr) noexcept {
        attack_inc_ = attack_ms <= 0.05f ? 1.0f : static_cast<float>(1000.0 / (static_cast<double>(attack_ms) * sr));
        decay_k_ = fall(decay_ms, sr);
        release_k_ = fall(release_ms, sr);
        sustain_ = std::clamp(sustain, 0.0f, 1.0f);
    }
    void gate_on(bool from_zero) noexcept {
        if (from_zero) level_ = 0.0f;
        stage_ = Attack;
    }
    void gate_off() noexcept {
        if (stage_ != Idle) stage_ = Release;
    }
    void kill() noexcept {
        stage_ = Idle;
        level_ = 0.0f;
    }
    Stage stage() const noexcept { return stage_; }
    bool active() const noexcept { return stage_ != Idle; }
    float level() const noexcept { return level_; }

    float next() noexcept {
        switch (stage_) {
        case Idle: return 0.0f;
        case Attack:
            level_ += attack_inc_;
            if (level_ >= 1.0f) {
                level_ = 1.0f;
                stage_ = Decay;
            }
            return level_;
        case Decay:
            level_ = sustain_ + (level_ - sustain_) * decay_k_;
            if (level_ - sustain_ < 1e-5f) {
                level_ = sustain_;
                stage_ = Sustain;
            }
            return level_;
        case Sustain:
            level_ = sustain_; // follows sustain edits while held
            return level_;
        case Release:
            level_ *= release_k_;
            if (level_ < 1e-5f) kill();
            return level_;
        }
        return 0.0f;
    }

private:
    static float fall(float ms, double sr) noexcept {
        return ms <= 0.05f ? 0.0f : static_cast<float>(std::exp(-5000.0 / (static_cast<double>(ms) * sr)));
    }
    Stage stage_ = Idle;
    float level_ = 0.0f;
    float attack_inc_ = 1.0f;
    float decay_k_ = 0.0f;
    float release_k_ = 0.0f;
    float sustain_ = 1.0f;
};

// ---------------------------------------------------------------- filter

enum class FilterType : uint8_t { Lp12 = 0, Lp24 = 1, Hp12 = 2, Bp12 = 3 };

class Svf {
public:
    // Cutoff in Hz, resonance 0..1, drive 0..1.
    void set(FilterType type, float hz, float resonance, float drive, double sr) noexcept {
        type_ = type;
        const float nyq = static_cast<float>(sr) * 0.49f;
        const float fc = std::clamp(hz, 10.0f, nyq);
        g_ = std::tan(kPi * fc / static_cast<float>(sr));
        // Q from 0.707 (Butterworth) to about 20, just below self-oscillation.
        const float q = 0.7071f * std::pow(28.0f, std::clamp(resonance, 0.0f, 1.0f));
        k_ = 1.0f / q;
        a1_ = 1.0f / (1.0f + g_ * (g_ + k_));
        const float d = std::clamp(drive, 0.0f, 1.0f);
        drive_gain_ = 1.0f + 15.0f * d * d;
        drive_on_ = d > 1e-4f;
        // Keep the level roughly constant as drive rises.
        drive_makeup_ = drive_on_ ? 1.0f / std::sqrt(drive_gain_) : 1.0f;
    }
    void reset() noexcept {
        for (auto& s : s_) s = {};
        x1_[0] = x1_[1] = 0.0f;
        f1_[0] = f1_[1] = 0.0f;
    }
    // c = channel 0 / 1.
    float process(float x, int c) noexcept {
        if (drive_on_) x = drive(x * drive_gain_, c) * drive_makeup_;
        switch (type_) {
        case FilterType::Lp12: return tick(s_[c], x).lp;
        case FilterType::Lp24: return tick(s_[2 + c], tick(s_[c], x).lp).lp;
        case FilterType::Hp12: return tick(s_[c], x).hp;
        case FilterType::Bp12: return tick(s_[c], x).bp;
        }
        return x;
    }

private:
    struct State {
        float ic1 = 0.0f, ic2 = 0.0f;
    };
    struct Out {
        float lp, bp, hp;
    };
    Out tick(State& s, float x) noexcept {
        const float v3 = x - s.ic2;
        const float v1 = a1_ * (s.ic1 + g_ * v3);
        const float v2 = s.ic2 + g_ * v1;
        s.ic1 = 2.0f * v1 - s.ic1;
        s.ic2 = 2.0f * v2 - s.ic2;
        if (std::fabs(s.ic1) < 1e-20f) s.ic1 = 0.0f;
        if (std::fabs(s.ic2) < 1e-20f) s.ic2 = 0.0f;
        return Out{v2, v1, x - k_ * v1 - v2};
    }
    // First-order antiderivative anti-aliasing of tanh: F(x) = log(cosh(x)).
    static float logcosh(float x) noexcept {
        const float a = std::fabs(x);
        return a > 9.0f ? a - 0.69314718f : std::log(std::cosh(x));
    }
    float drive(float x, int c) noexcept {
        const float d = x - x1_[c];
        const float f = logcosh(x);
        const float y = std::fabs(d) > 1e-4f ? (f - f1_[c]) / d : std::tanh(0.5f * (x + x1_[c]));
        x1_[c] = x;
        f1_[c] = f;
        return y;
    }
    FilterType type_ = FilterType::Lp24;
    float g_ = 1.0f, k_ = 1.4142f, a1_ = 0.5f;
    float drive_gain_ = 1.0f, drive_makeup_ = 1.0f;
    bool drive_on_ = false;
    State s_[4];
    float x1_[2] = {}, f1_[2] = {};
};

// ---------------------------------------------------------------- LFO

enum class LfoShape : uint8_t { Sine = 0, Triangle = 1, Square = 2, Saw = 3 };

// Bipolar shape value (-1..1) at phase 0..1.
inline float lfo_shape(LfoShape s, float ph) noexcept {
    switch (s) {
    case LfoShape::Sine: return std::sin(2.0f * kPi * ph);
    case LfoShape::Triangle: return ph < 0.25f ? 4.0f * ph : ph < 0.75f ? 2.0f - 4.0f * ph : 4.0f * ph - 4.0f;
    case LfoShape::Square: return ph < 0.5f ? 1.0f : -1.0f;
    case LfoShape::Saw: return 2.0f * ph - 1.0f;
    }
    return 0.0f;
}

// ---------------------------------------------------------------- voice

// Settings shared by every voice of a device, recomputed once per block.
struct VoiceSettings {
    double sr = 48000.0;
    float gain = 1.0f;          // linear (device gain)
    float pan = 0.0f;           // -1..1
    float transpose = 0.0f;     // semitones (transpose + fine)
    float bend_range = 2.0f;    // semitones
    float velocity_sens = 1.0f; // 0..1
    float glide_ms = 0.0f;
    bool key_tracks = true;     // pitch follows the key (relative to root)
    int root = 60;
    bool use_amp_env = true;    // false: 1 ms declick attack, gate release 30 ms
    float amp_a = 2.0f, amp_d = 120.0f, amp_s = 1.0f, amp_r = 48.0f; // ms, sustain 0..1
    bool filter_on = false;
    FilterType filter_type = FilterType::Lp24;
    float cutoff = 20000.0f, resonance = 0.0f, drive = 0.0f, key_track = 0.0f, env_depth = 0.0f; // env_depth -1..1 (x48 st)
    float flt_a = 0.0f, flt_d = 300.0f, flt_s = 0.0f, flt_r = 200.0f;
    LfoShape lfo_shape = LfoShape::Sine;
    float lfo_rate = 2.0f, lfo_amount = 0.0f, lfo_fade_ms = 0.0f, lfo_phase = 0.0f;
    int lfo_target = 0; // 0 pitch, 1 cutoff, 2 pan, 3 volume
    float fade_in = 0.0f, fade_out = 0.0f; // fractions of the region
    // Warp: the region lasts this many output frames whatever the pitch (0 = re-pitch).
    double warp_frames = 0.0;
};

// Where a note plays inside a sample (resolved by the device at note-on).
struct Region {
    uint32_t slot = 0;      // index into the device's samples
    double start = 0.0; // frames
    double end = 0.0;   // frames (exclusive)
    double loop_start = 0.0;
    bool loop = false;
    bool reverse = false;
    bool one_shot = false;  // note-off is ignored (plays to the end)
    float gain = 1.0f;      // linear (slice gain)
    float pan = 0.0f;       // slice pan
    float pitch = 0.0f;     // semitones (slice pitch)
};

class Voice {
public:
    static constexpr int kKillFrames = 96; // declick when a voice is stolen
    static constexpr int kLoopXfade = 256; // click-free loop seam

    void prepare(double sr) noexcept;
    bool active() const noexcept { return active_; }
    bool releasing() const noexcept { return amp_.stage() == Adsr::Release || killing_ > 0; }
    bool killing() const noexcept { return killing_ > 0; }
    bool held() const noexcept { return held_; }
    uint8_t note() const noexcept { return note_; }
    uint64_t age() const noexcept { return age_; }
    uint32_t slot() const noexcept { return region_.slot; }
    uint64_t token() const noexcept { return token_; }
    // Playback position as a fraction of the sample (for the playhead).
    float position() const noexcept;
    float level() const noexcept { return amp_.level(); }

    // from_pitch: semitones the glide starts from (NaN = no glide). s is
    // the region's sample (voices keep only its slot: sample memory can
    // change between blocks).
    void start(const VoiceSettings& vs, const BuiltinSample& s, const Region& r, uint8_t note, uint8_t velocity,
               float from_pitch, float lfo_phase, uint64_t age) noexcept;
    // Legato: move to a new note without restarting the sample.
    void retarget(const VoiceSettings& vs, uint8_t note) noexcept;
    void release() noexcept;   // note-off (one-shot regions keep playing)
    void force_release() noexcept;
    void kill() noexcept;      // fast fade, then free
    void stop() noexcept { active_ = false; }
    float current_pitch() const noexcept { return glide_pitch_; }

    // Adds this voice to out_l/out_r. bend in semitones; s is samples[slot()].
    void render(const VoiceSettings& vs, const BuiltinSample& s, float bend, float* out_l, float* out_r,
                uint32_t frames) noexcept;

private:
    double src_of(double off) const noexcept { return region_.reverse ? region_.end - 1.0 - off : region_.start + off; }
    void read_off(const BuiltinSample& s, double off, int k, float& l, float& r) const noexcept;
    void read_seam(const BuiltinSample& s, double off, int k, float& l, float& r) const noexcept;
    float fades(double off) const noexcept;
    void stretch_frame(const BuiltinSample& s, double rate, int k, float& l, float& r) noexcept;
    double aligned_start(const BuiltinSample& s, double want, double rate) const noexcept;
    double wrap(double off) const noexcept;

    const SincTables* tab_ = nullptr;
    double sr_ = 48000.0;
    bool active_ = false;
    bool held_ = false;
    uint8_t note_ = 0;
    uint64_t age_ = 0;
    uint64_t token_ = 0;
    Region region_{};
    // Playback runs in an offset domain: 0 = the first frame played (region
    // start, or its end when reversed), increasing in playback direction.
    double off_ = 0.0;        // read position (re-pitch) or stretch read head (warp)
    double len_ = 0.0;        // region length
    double play_end_ = 0.0;   // offset where playback ends or loops
    double loop_back_ = 0.0;  // offset playback jumps back to when looping
    bool looping_ = false;
    double fade_in_ = 0.0, fade_out_ = 0.0; // frames
    uint64_t sample_frames_ = 0;
    bool finished_ = false;   // reached the end: no new grains / reads
    float vel_gain_ = 1.0f;
    float target_pitch_ = 0.0f; // semitones from root (key + slice pitch)
    float glide_pitch_ = 0.0f;
    Adsr amp_, flt_env_;
    Svf filter_;
    float lfo_phase_ = 0.0f;
    double elapsed_ = 0.0; // seconds since start (LFO fade-in)
    int killing_ = 0;
    // Warp (granular stretch): two Hann grains at 50% overlap.
    bool stretch_ = false;
    double stretch_rate_ = 1.0; // source frames per output frame for the read head
    int grain_len_ = 2048;
    struct Grain {
        double src = 0.0; // source position of the grain's next frame
        int age = -1;     // frames since the grain started; < 0 = idle
    };
    Grain grains_[2];
    int hop_ = 1024;
    int since_hop_ = 0;
};

// Polyphonic note handling shared by the sampler devices: voice pool and
// stealing with a short declick.
class VoicePool {
public:
    static constexpr int kMaxVoices = 40; // 32 playable + room for declicking stolen voices
    void prepare(double sr) noexcept;
    void reset() noexcept;

    Voice* voices() noexcept { return voices_.data(); }
    const Voice* voices() const noexcept { return voices_.data(); }
    int sounding() const noexcept; // active and not being killed
    // A free voice; when limit voices already sound, the oldest is stolen
    // (it fades out over Voice::kKillFrames).
    Voice* allocate(int limit) noexcept;
    void release_note(uint8_t note) noexcept;
    void all_off(bool hard) noexcept;
    // Stop voices whose slot audio changed (token differs) or vanished.
    void drop_stale(const BuiltinSample* samples, uint32_t count) noexcept;
    void render(const VoiceSettings& vs, const BuiltinSample* samples, uint32_t count, float bend, float* l, float* r,
                uint32_t frames) noexcept;
    uint64_t next_age() noexcept { return ++age_; }

private:
    std::array<Voice, kMaxVoices> voices_{};
    uint64_t age_ = 0;
};

// MIDI pitch bend (14-bit) -> -1..1.
inline float bend_value(uint8_t lsb, uint8_t msb) noexcept {
    const int v = (static_cast<int>(msb) << 7 | lsb) - 8192;
    return static_cast<float>(v) / (v < 0 ? 8192.0f : 8191.0f);
}

// Musical length of a warp choice in quarter notes (0 = re-pitch).
inline double warp_quarters(int choice) noexcept {
    switch (choice) {
    case 1: return 4.0; // whole note
    case 2: return 2.0;
    case 3: return 1.0;
    case 4: return 0.5;
    default: return 0.0;
    }
}

inline float clampf(float v, float lo, float hi) noexcept { return std::isfinite(v) ? std::clamp(v, lo, hi) : lo; }

// Identity of the audio in a slot: changes when the slot gets other audio.
inline uint64_t sample_token(const BuiltinSample& s) noexcept {
    return s.frames == 0 ? 0 : (reinterpret_cast<uintptr_t>(s.ch[0]) ^ (s.frames << 1));
}

} // namespace saine::sampler
