// DSP building blocks of the synth-v2 instrument (header only, RT safe:
// nothing here allocates outside the explicit resize() calls made from
// prepare()).
#pragma once

#include <algorithm>
#include <cmath>
#include <cstdint>
#include <numbers>
#include <vector>

namespace dstudio::synth_v2 {

constexpr float kPi = std::numbers::pi_v<float>;
constexpr float kTwoPi = 2.0f * std::numbers::pi_v<float>;

// xorshift32: deterministic per-voice noise.
struct Rng {
    uint32_t s = 0x9e3779b9u;
    void seed(uint32_t v) noexcept { s = v != 0 ? v : 0x9e3779b9u; }
    uint32_t next() noexcept {
        s ^= s << 13;
        s ^= s >> 17;
        s ^= s << 5;
        return s;
    }
    // Uniform in [-1, 1).
    float bipolar() noexcept { return static_cast<float>(static_cast<int32_t>(next())) * (1.0f / 2147483648.0f); }
    // Uniform in [0, 1).
    float unit() noexcept { return static_cast<float>(next() >> 8) * (1.0f / 16777216.0f); }
};

inline float wrap01(float t) noexcept { return t - std::floor(t); }

// sin(2 pi p) for any p: odd polynomial on a quarter period (error ~4e-6).
inline float sin_turns(float p) noexcept {
    float x = p - std::floor(p) - 0.5f;
    if (x > 0.25f)
        x = 0.5f - x;
    else if (x < -0.25f)
        x = -0.5f - x;
    const float z = kTwoPi * x;
    const float z2 = z * z;
    return -z * (1.0f + z2 * (-1.0f / 6.0f + z2 * (1.0f / 120.0f + z2 * (-1.0f / 5040.0f + z2 * (1.0f / 362880.0f)))));
}

// tanh, rational approximation (max error ~2e-4, exact limits +-1).
inline float fast_tanh(float x) noexcept {
    x = std::clamp(x, -4.97f, 4.97f);
    const float x2 = x * x;
    const float a = x * (135135.0f + x2 * (17325.0f + x2 * (378.0f + x2)));
    const float b = 135135.0f + x2 * (62370.0f + x2 * (3150.0f + x2 * 28.0f));
    return std::clamp(a / b, -1.0f, 1.0f);
}

// 2-sample polynomial band-limited step residual for a +2 step at phase 0
// (t in [0, 1), dt = phase increment).
inline float blep(float t, float dt) noexcept {
    if (t < dt) {
        t /= dt;
        return t + t - t * t - 1.0f;
    }
    if (t > 1.0f - dt) {
        t = (t - 1.0f) / dt;
        return t * t + t + t + 1.0f;
    }
    return 0.0f;
}

// Integrated polyBLEP residual for a unit slope change at phase 0.
inline float blamp(float t, float dt) noexcept {
    if (t < dt) {
        t = t / dt - 1.0f;
        return -(1.0f / 3.0f) * t * t * t;
    }
    if (t > 1.0f - dt) {
        t = (t - 1.0f) / dt + 1.0f;
        return (1.0f / 3.0f) * t * t * t;
    }
    return 0.0f;
}

// One-pole low-pass: y += a * (x - y).
struct OnePole {
    float y = 0.0f;
    float a = 1.0f;
    void set_hz(float hz, float sr) noexcept { a = 1.0f - std::exp(-kTwoPi * std::clamp(hz, 1.0f, 0.49f * sr) / sr); }
    float lp(float x) noexcept { return y += a * (x - y); }
    float hp(float x) noexcept { return x - lp(x); }
    void reset(float v = 0.0f) noexcept { y = v; }
    void flush() noexcept {
        if (std::fabs(y) < 1e-20f || !std::isfinite(y)) y = 0.0f;
    }
};

// DC blocker (~10 Hz).
struct DcBlock {
    float x1 = 0.0f, y1 = 0.0f, r = 0.9987f;
    void set(float sr) noexcept { r = 1.0f - kTwoPi * 10.0f / sr; }
    float run(float x) noexcept {
        const float y = x - x1 + r * y1;
        x1 = x;
        y1 = std::fabs(y) < 1e-20f ? 0.0f : y;
        return y;
    }
    void reset() noexcept { x1 = y1 = 0.0f; }
};

// Topology-preserving state-variable filter (Simper / Zavalishin form):
// low, band and high outputs; k = 1 / Q.
struct Svf {
    float ic1 = 0.0f, ic2 = 0.0f;
    float g = 0.0f, k = 1.4f, a1 = 0.0f, a2 = 0.0f, a3 = 0.0f;
    void set(float hz, float q_inv, float sr) noexcept { set_g(std::tan(kPi * std::clamp(hz, 5.0f, 0.49f * sr) / sr), q_inv); }
    void set_g(float g_, float q_inv) noexcept {
        g = g_;
        k = q_inv;
        a1 = 1.0f / (1.0f + g * (g + k));
        a2 = g * a1;
        a3 = g * a2;
    }
    // Returns the low-pass output; band and high through the references.
    float run(float v0, float& band, float& high) noexcept {
        const float v3 = v0 - ic2;
        const float v1 = a1 * ic1 + a2 * v3;
        const float v2 = ic2 + a2 * ic1 + a3 * v3;
        ic1 = 2.0f * v1 - ic1;
        ic2 = 2.0f * v2 - ic2;
        band = v1;
        high = v0 - k * v1 - v2;
        return v2;
    }
    void reset() noexcept { ic1 = ic2 = 0.0f; }
    void flush() noexcept {
        if (std::fabs(ic1) < 1e-20f) ic1 = 0.0f;
        if (std::fabs(ic2) < 1e-20f) ic2 = 0.0f;
        if (!std::isfinite(ic1) || !std::isfinite(ic2)) reset();
    }
};

// Four-pole zero-delay-feedback ladder low-pass with a saturating input
// stage. g = tan(pi fc / sr); k = resonance feedback (0 .. ~4).
struct Ladder {
    float s[4] = {0, 0, 0, 0};
    float G = 0.0f, g1 = 0.0f; // g / (1 + g)
    float k = 0.0f;
    void set_g(float g, float k_) noexcept {
        g1 = g / (1.0f + g);
        G = g1 * g1 * g1 * g1;
        k = k_;
    }
    float run(float x) noexcept {
        // Each TPT one-pole: y = g1 * x + (1 - g1) * s. The feedback path is
        // solved for the linear part, then the input stage saturates.
        const float b = 1.0f - g1;
        const float S = b * (g1 * g1 * g1 * s[0] + g1 * g1 * s[1] + g1 * s[2] + s[3]);
        float u = (x - k * S) / (1.0f + k * G);
        u = fast_tanh(u);
        float y = u;
        for (float& st : s) {
            const float v = g1 * (y - st);
            const float out = v + st;
            st = out + v;
            y = out;
        }
        return y;
    }
    void reset() noexcept { s[0] = s[1] = s[2] = s[3] = 0.0f; }
    void flush() noexcept {
        for (float& v : s) {
            if (!std::isfinite(v)) {
                reset();
                return;
            }
            if (std::fabs(v) < 1e-20f) v = 0.0f;
        }
    }
};

// First-order anti-derivative anti-aliasing (ADAA) around a static
// waveshaper f with antiderivative F.
template <float (*F)(float), float (*Fn)(float)>
struct Adaa {
    float x1 = 0.0f;
    float run(float x) noexcept {
        const float d = x - x1;
        const float y = std::fabs(d) > 1e-4f ? (F(x) - F(x1)) / d : Fn(0.5f * (x + x1));
        x1 = x;
        return y;
    }
    void reset() noexcept { x1 = 0.0f; }
};

inline float tanh_f(float x) noexcept { return fast_tanh(x); }
// log(cosh(x)), stable for large |x|.
inline float logcosh(float x) noexcept {
    const float a = std::fabs(x);
    return a > 8.0f ? a - 0.69314718f : std::log(std::cosh(a));
}
inline float hardclip_f(float x) noexcept { return std::clamp(x, -1.0f, 1.0f); }
inline float hardclip_F(float x) noexcept {
    const float a = std::fabs(x);
    return a <= 1.0f ? 0.5f * x * x : a - 0.5f;
}
// Cubic soft clip x - x^3/3 inside [-1, 1], +-2/3 outside.
inline float cubic_f(float x) noexcept {
    const float c = std::clamp(x, -1.0f, 1.0f);
    return c - c * c * c / 3.0f;
}
inline float cubic_F(float x) noexcept {
    const float a = std::fabs(x);
    if (a <= 1.0f) return 0.5f * x * x - x * x * x * x / 12.0f;
    return 5.0f / 12.0f + (2.0f / 3.0f) * (a - 1.0f);
}
// Asymmetric "tube" curve: tanh(x) above zero, tanh(1.6 x) / 1.6 below.
inline float tube_f(float x) noexcept { return x >= 0.0f ? fast_tanh(x) : fast_tanh(1.6f * x) / 1.6f; }
inline float tube_F(float x) noexcept { return x >= 0.0f ? logcosh(x) : logcosh(1.6f * x) / (1.6f * 1.6f); }

using AdaaTanh = Adaa<logcosh, tanh_f>;
using AdaaHard = Adaa<hardclip_F, hardclip_f>;
using AdaaCubic = Adaa<cubic_F, cubic_f>;
using AdaaTube = Adaa<tube_F, tube_f>;

// Power-of-two circular delay buffer with cubic (Hermite) reads.
struct DelayBuf {
    std::vector<float> buf;
    uint32_t mask = 0, w = 0;
    void resize(std::size_t min_len) {
        std::size_t n = 1;
        while (n < min_len + 4) n <<= 1;
        buf.assign(n, 0.0f);
        mask = static_cast<uint32_t>(n - 1);
        w = 0;
    }
    void clear() noexcept { std::fill(buf.begin(), buf.end(), 0.0f); }
    void push(float x) noexcept {
        buf[w] = x;
        w = (w + 1) & mask;
    }
    // The sample written d samples ago (d >= 1, fractional).
    float read(float d) const noexcept {
        d = std::clamp(d, 1.0f, static_cast<float>(mask - 3));
        const float pos = static_cast<float>(w) - d;
        const float fl = std::floor(pos);
        const float t = pos - fl;
        const auto i = static_cast<uint32_t>(static_cast<int64_t>(fl)) & mask;
        const float xm1 = buf[(i - 1) & mask], x0 = buf[i], x1 = buf[(i + 1) & mask], x2 = buf[(i + 2) & mask];
        const float c1 = 0.5f * (x1 - xm1);
        const float c2 = xm1 - 2.5f * x0 + 2.0f * x1 - 0.5f * x2;
        const float c3 = 0.5f * (x2 - xm1) + 1.5f * (x0 - x1);
        return ((c3 * t + c2) * t + c1) * t + x0;
    }
    float tap(uint32_t d) const noexcept { return buf[(w - d) & mask]; }
};

// Equal-power pan gains for p in [-1, 1].
inline void pan_gains(float p, float& gl, float& gr) noexcept {
    const float a = (std::clamp(p, -1.0f, 1.0f) + 1.0f) * 0.125f; // 0 .. a quarter turn
    gl = sin_turns(a + 0.25f);
    gr = sin_turns(a);
}

inline float db_to_gain(float db) noexcept { return std::exp2(db * 0.166096404744368f); }

// Beats per cycle of the LFO note values (8 Bars .. 1/32, 17 entries).
inline float lfo_note_beats(int i) noexcept {
    constexpr float k[17] = {32.0f, 16.0f, 8.0f, 4.0f, 3.0f, 2.0f, 4.0f / 3.0f, 1.5f, 1.0f, 2.0f / 3.0f, 0.75f, 0.5f, 1.0f / 3.0f, 0.375f, 0.25f, 1.0f / 6.0f, 0.125f};
    return k[std::clamp(i, 0, 16)];
}
// Beats of the delay note values (1/32 .. 1/1, 13 entries).
inline float delay_note_beats(int i) noexcept {
    constexpr float k[13] = {0.125f, 1.0f / 6.0f, 0.25f, 0.375f, 1.0f / 3.0f, 0.5f, 0.75f, 2.0f / 3.0f, 1.0f, 1.5f, 2.0f, 3.0f, 4.0f};
    return k[std::clamp(i, 0, 12)];
}

// LFO shapes: 0 Sine, 1 Triangle, 2 Saw Up, 3 Saw Down, 4 Square, 5 Random
// (sample and hold per cycle), 6 Smooth Random (cosine from this cycle's
// value a to the next one, b).
inline float lfo_shape(int shape, float p, float a, float b) noexcept {
    switch (shape) {
    case 1: return p < 0.25f ? 4.0f * p : (p < 0.75f ? 2.0f - 4.0f * p : 4.0f * p - 4.0f);
    case 2: return 2.0f * p - 1.0f;
    case 3: return 1.0f - 2.0f * p;
    case 4: return p < 0.5f ? 1.0f : -1.0f;
    case 5: return a;
    case 6: {
        const float w = 0.5f - 0.5f * sin_turns(p * 0.5f + 0.25f);
        return a + (b - a) * w;
    }
    default: return sin_turns(p);
    }
}

} // namespace dstudio::synth_v2
