// Small DSP helpers shared by the built-in processors (RT safe, header only).
#pragma once

#include <algorithm>
#include <cmath>
#include <cstdint>
#include <cstring>

namespace saine::dsp {

constexpr float kLog2_10_over_20 = 0.166096404744368f; // dB -> log2(gain)
constexpr float kDbPerLog2 = 6.020599913279624f;        // log2(gain) -> dB
constexpr float kSilenceDb = -120.0f;

inline float db_to_gain(float db) noexcept { return std::exp2(db * kLog2_10_over_20); }
inline float gain_to_db(float g) noexcept { return g > 1e-6f ? kDbPerLog2 * std::log2(g) : kSilenceDb; }
// Mean square (power) -> dB.
inline float power_to_db(float p) noexcept { return p > 1e-12f ? 0.5f * kDbPerLog2 * std::log2(p) : kSilenceDb; }

// One-pole coefficient for a time constant in milliseconds: y += (1 - a) * (x - y).
inline float pole(float ms, double sample_rate) noexcept {
    return ms <= 0.0f ? 0.0f : static_cast<float>(std::exp(-1000.0 / (static_cast<double>(ms) * sample_rate)));
}

// Per-sample parameter smoother (one pole, ~ms time constant).
struct Smoother {
    float y = 0.0f;
    float a = 0.0f;
    void set_time(float ms, double sample_rate) noexcept { a = pole(ms, sample_rate); }
    void snap(float v) noexcept { y = v; }
    float next(float target) noexcept { return y = target + a * (y - target); }
    bool settled(float target) const noexcept { return std::fabs(y - target) <= 1e-6f * std::max(1.0f, std::fabs(target)); }
};

// ---------------------------------------------------------------- fast math
// Branch-light approximations for per-sample (and oversampled) use.

// 2^x, relative error < 2e-7 over the float range (x clamped to +-126).
inline float fast_exp2(float x) noexcept {
    x = std::clamp(x, -126.0f, 126.0f);
    const float xi = std::floor(x);
    const float f = x - xi;
    float p = 1.535336188319500e-4f;
    p = p * f + 1.339887440266574e-3f;
    p = p * f + 9.618437357674640e-3f;
    p = p * f + 5.550332471162809e-2f;
    p = p * f + 2.402264791363012e-1f;
    p = p * f + 6.931472028550421e-1f;
    p = p * f + 1.0f;
    const int32_t e = (static_cast<int32_t>(xi) + 127) << 23;
    float scale;
    std::memcpy(&scale, &e, 4);
    return p * scale;
}

// log2(x) for x > 0 (normal floats), absolute error < 1e-6.
inline float fast_log2(float x) noexcept {
    int32_t bits;
    std::memcpy(&bits, &x, 4);
    int32_t e = ((bits >> 23) & 0xff) - 127;
    bits = (bits & 0x007fffff) | 0x3f800000; // mantissa in [1, 2)
    float m;
    std::memcpy(&m, &bits, 4);
    if (m > 1.41421356f) { // keep m in [0.707, 1.414) so t stays small
        m *= 0.5f;
        ++e;
    }
    const float t = (m - 1.0f) / (m + 1.0f);
    const float t2 = t * t;
    const float poly = 2.0f + t2 * (0.666666667f + t2 * (0.4f + t2 * (0.285714286f + t2 * 0.222222222f)));
    return static_cast<float>(e) + 1.44269504f * t * poly;
}

// tanh(x), absolute error < 3e-7.
inline float fast_tanh(float x) noexcept {
    const float a = std::fabs(x);
    if (a < 0.03f) return x * (1.0f - a * a * (1.0f / 3.0f)); // keeps the slope exact near 0
    const float e = fast_exp2(std::min(a, 10.0f) * 2.88539008f); // e^(2a)
    return std::copysign(1.0f - 2.0f / (e + 1.0f), x);
}

// ---------------------------------------------------------------- filters

// One-pole high-pass (DC blocker): y = R (y + x - x1).
struct DcBlocker {
    float x1 = 0.0f, y1 = 0.0f, r = 0.999f;
    void set(float hz, double sample_rate) noexcept { r = static_cast<float>(std::exp(-6.283185307179586 * static_cast<double>(hz) / sample_rate)); }
    void reset() noexcept { x1 = y1 = 0.0f; }
    float process(float x) noexcept {
        const float y = r * (y1 + x - x1);
        x1 = x;
        y1 = y;
        return y;
    }
    void flush_denormals() noexcept {
        if (std::fabs(y1) < 1e-20f) y1 = 0.0f;
    }
};

// Coefficients of a trapezoidal state-variable filter (Simper): one form for
// low-pass, high-pass, bell and shelves, stable under modulation, so a block
// can interpolate between two coefficient sets.
struct SvfCoeffs {
    float a1 = 1.0f, a2 = 0.0f, a3 = 0.0f, m0 = 1.0f, m1 = 0.0f, m2 = 0.0f;

    static float prewarp(float hz, double sample_rate) noexcept {
        const double f = std::clamp(static_cast<double>(hz), 1.0, 0.49 * sample_rate);
        return static_cast<float>(std::tan(3.141592653589793 * f / sample_rate));
    }
    static SvfCoeffs make(float g, float k, float m0, float m1, float m2) noexcept {
        SvfCoeffs c;
        c.a1 = 1.0f / (1.0f + g * (g + k));
        c.a2 = g * c.a1;
        c.a3 = g * c.a2;
        c.m0 = m0;
        c.m1 = m1;
        c.m2 = m2;
        return c;
    }
    static SvfCoeffs lowpass(float hz, float q, double sr) noexcept { return make(prewarp(hz, sr), 1.0f / q, 0.0f, 0.0f, 1.0f); }
    static SvfCoeffs highpass(float hz, float q, double sr) noexcept {
        const float k = 1.0f / q;
        return make(prewarp(hz, sr), k, 1.0f, -k, -1.0f);
    }
    static SvfCoeffs bell(float hz, float q, float db, double sr) noexcept {
        const float a = std::pow(10.0f, db / 40.0f);
        const float k = 1.0f / (q * a);
        return make(prewarp(hz, sr), k, 1.0f, k * (a * a - 1.0f), 0.0f);
    }
    static SvfCoeffs lowshelf(float hz, float q, float db, double sr) noexcept {
        const float a = std::pow(10.0f, db / 40.0f);
        const float k = 1.0f / q;
        return make(prewarp(hz, sr) / std::sqrt(a), k, 1.0f, k * (a - 1.0f), a * a - 1.0f);
    }
    static SvfCoeffs highshelf(float hz, float q, float db, double sr) noexcept {
        const float a = std::pow(10.0f, db / 40.0f);
        const float k = 1.0f / q;
        return make(prewarp(hz, sr) * std::sqrt(a), k, a * a, k * (1.0f - a) * a, 1.0f - a * a);
    }
    // Linear blend (t in 0..1) for per-sample interpolation across a block.
    static SvfCoeffs lerp(const SvfCoeffs& x, const SvfCoeffs& y, float t) noexcept {
        auto l = [t](float a, float b) { return a + t * (b - a); };
        SvfCoeffs c;
        c.a1 = l(x.a1, y.a1);
        c.a2 = l(x.a2, y.a2);
        c.a3 = l(x.a3, y.a3);
        c.m0 = l(x.m0, y.m0);
        c.m1 = l(x.m1, y.m1);
        c.m2 = l(x.m2, y.m2);
        return c;
    }
};

struct Svf {
    float ic1 = 0.0f, ic2 = 0.0f;
    void reset() noexcept { ic1 = ic2 = 0.0f; }
    float process(float v0, const SvfCoeffs& c) noexcept {
        const float v3 = v0 - ic2;
        const float v1 = c.a1 * ic1 + c.a2 * v3;
        const float v2 = ic2 + c.a2 * ic1 + c.a3 * v3;
        ic1 = 2.0f * v1 - ic1;
        ic2 = 2.0f * v2 - ic2;
        return c.m0 * v0 + c.m1 * v1 + c.m2 * v2;
    }
    void flush_denormals() noexcept {
        if (std::fabs(ic1) < 1e-20f) ic1 = 0.0f;
        if (std::fabs(ic2) < 1e-20f) ic2 = 0.0f;
    }
};

// Deterministic white noise in -1..1 (RT safe, per-instance state).
struct Noise {
    uint32_t s = 0x9e3779b9u;
    float next() noexcept {
        s ^= s << 13;
        s ^= s >> 17;
        s ^= s << 5;
        return static_cast<float>(static_cast<int32_t>(s)) * (1.0f / 2147483648.0f);
    }
};

// Peak of max(|l|, |r|) over n samples.
inline float stereo_peak(const float* l, const float* r, uint32_t n) noexcept {
    float m = 0.0f;
    for (uint32_t i = 0; i < n; ++i) m = std::max(m, std::max(std::fabs(l[i]), std::fabs(r[i])));
    return m;
}

} // namespace saine::dsp
