// Building blocks of the time-based built-ins (delay, echo, reverb, grains,
// pitch shifter): delay lines with fractional reads, biquads, one-poles,
// allpasses, a small RNG and soft clipping. RT safe: allocation happens only
// in the allocate() / prepare-time calls.
#pragma once

#include <algorithm>
#include "dsp.hpp"

#include <cmath>
#include <cstdint>
#include <cstring>
#include <vector>

namespace saine::dsp {

// xorshift32: deterministic, cheap, good enough for modulation and scatter.
struct UnitRng {
    uint32_t s = 0x9e3779b9u;
    void seed(uint32_t v) noexcept { s = v != 0 ? v : 0x9e3779b9u; }
    uint32_t next() noexcept {
        s ^= s << 13;
        s ^= s >> 17;
        s ^= s << 5;
        return s;
    }
    // [0, 1)
    float uni() noexcept { return static_cast<float>(next() >> 8) * (1.0f / 16777216.0f); }
    // [-1, 1)
    float bi() noexcept { return 2.0f * uni() - 1.0f; }
};

// Smooth saturation: tanh-like, unity slope at 0, bounded to +-1.
inline float soft_clip(float x) noexcept {
    const float c = std::clamp(x, -3.0f, 3.0f);
    return c * (27.0f + c * c) / (27.0f + 9.0f * c * c);
}

// Transparent up to 1.0, then a soft knee that never exceeds 1.5.
inline float soft_limit(float x) noexcept {
    const float a = std::fabs(x);
    if (a <= 1.0f) return x;
    const float y = 1.0f + 0.5f * soft_clip((a - 1.0f) * 2.0f);
    return x < 0.0f ? -y : y;
}

inline float flush(float x) noexcept { return std::fabs(x) < 1e-15f ? 0.0f : x; }

// 4-point, 3rd-order Hermite interpolation between y0 and y1 at t in [0, 1).
inline float hermite(float ym1, float y0, float y1, float y2, float t) noexcept {
    const float c1 = 0.5f * (y1 - ym1);
    const float c2 = ym1 - 2.5f * y0 + 2.0f * y1 - 0.5f * y2;
    const float c3 = 0.5f * (y2 - ym1) + 1.5f * (y0 - y1);
    return ((c3 * t + c2) * t + c1) * t + y0;
}

// Transposed direct form II biquad with RBJ cookbook designs.
struct Biquad {
    float b0 = 1.0f, b1 = 0.0f, b2 = 0.0f, a1 = 0.0f, a2 = 0.0f;
    float z1 = 0.0f, z2 = 0.0f;

    void reset() noexcept { z1 = z2 = 0.0f; }
    float process(float x) noexcept {
        const float y = b0 * x + z1;
        z1 = b1 * x - a1 * y + z2;
        z2 = b2 * x - a2 * y;
        return y;
    }
    void flush_state() noexcept {
        z1 = flush(z1);
        z2 = flush(z2);
    }
    void identity() noexcept {
        b0 = 1.0f;
        b1 = b2 = a1 = a2 = 0.0f;
    }

    enum Type { kLowpass, kHighpass, kBandpass, kPeak, kLowShelf, kHighShelf };
    // f in Hz, q > 0, gain in dB (peak / shelves).
    void set(Type type, float f, float q, float gain_db, double sample_rate) noexcept {
        const double fs = sample_rate;
        const double fc = std::clamp(static_cast<double>(f), 1.0, 0.49 * fs);
        const double w0 = 2.0 * 3.14159265358979323846 * fc / fs;
        const double cw = std::cos(w0), sw = std::sin(w0);
        const double qq = std::max(0.025, static_cast<double>(q));
        const double alpha = sw / (2.0 * qq);
        const double A = std::pow(10.0, static_cast<double>(gain_db) / 40.0);
        double nb0 = 1, nb1 = 0, nb2 = 0, na0 = 1, na1 = 0, na2 = 0;
        switch (type) {
        case kLowpass:
            nb0 = (1 - cw) / 2, nb1 = 1 - cw, nb2 = (1 - cw) / 2, na0 = 1 + alpha, na1 = -2 * cw, na2 = 1 - alpha;
            break;
        case kHighpass:
            nb0 = (1 + cw) / 2, nb1 = -(1 + cw), nb2 = (1 + cw) / 2, na0 = 1 + alpha, na1 = -2 * cw, na2 = 1 - alpha;
            break;
        case kBandpass:
            nb0 = alpha, nb1 = 0, nb2 = -alpha, na0 = 1 + alpha, na1 = -2 * cw, na2 = 1 - alpha;
            break;
        case kPeak:
            nb0 = 1 + alpha * A, nb1 = -2 * cw, nb2 = 1 - alpha * A, na0 = 1 + alpha / A, na1 = -2 * cw, na2 = 1 - alpha / A;
            break;
        case kLowShelf: {
            const double sa = 2 * std::sqrt(A) * alpha;
            nb0 = A * ((A + 1) - (A - 1) * cw + sa), nb1 = 2 * A * ((A - 1) - (A + 1) * cw), nb2 = A * ((A + 1) - (A - 1) * cw - sa);
            na0 = (A + 1) + (A - 1) * cw + sa, na1 = -2 * ((A - 1) + (A + 1) * cw), na2 = (A + 1) + (A - 1) * cw - sa;
            break;
        }
        case kHighShelf: {
            const double sa = 2 * std::sqrt(A) * alpha;
            nb0 = A * ((A + 1) + (A - 1) * cw + sa), nb1 = -2 * A * ((A - 1) + (A + 1) * cw), nb2 = A * ((A + 1) + (A - 1) * cw - sa);
            na0 = (A + 1) - (A - 1) * cw + sa, na1 = 2 * ((A - 1) - (A + 1) * cw), na2 = (A + 1) - (A - 1) * cw - sa;
            break;
        }
        }
        b0 = static_cast<float>(nb0 / na0);
        b1 = static_cast<float>(nb1 / na0);
        b2 = static_cast<float>(nb2 / na0);
        a1 = static_cast<float>(na1 / na0);
        a2 = static_cast<float>(na2 / na0);
    }
    // Magnitude (dB) of the current design at f.
    float response_db(float f, double sample_rate) const noexcept {
        const double w = 2.0 * 3.14159265358979323846 * static_cast<double>(f) / sample_rate;
        const double c1 = std::cos(w), s1 = std::sin(w), c2 = std::cos(2 * w), s2 = std::sin(2 * w);
        const auto B0 = static_cast<double>(b0), B1 = static_cast<double>(b1), B2 = static_cast<double>(b2), A1 = static_cast<double>(a1), A2 = static_cast<double>(a2);
        const double nr = B0 + B1 * c1 + B2 * c2, ni = -(B1 * s1 + B2 * s2);
        const double dr = 1 + A1 * c1 + A2 * c2, di = -(A1 * s1 + A2 * s2);
        return static_cast<float>(10.0 * std::log10((nr * nr + ni * ni) / std::max(1e-30, dr * dr + di * di)));
    }
};

// One-pole low-pass / high-pass (6 dB/oct), matched at the cutoff.
struct ExpOnePole {
    float a = 0.0f; // feedback coefficient
    float y = 0.0f;
    void set_hz(float hz, double sample_rate) noexcept {
        const double f = std::clamp(static_cast<double>(hz), 0.1, 0.49 * sample_rate);
        a = static_cast<float>(std::exp(-2.0 * 3.14159265358979323846 * f / sample_rate));
    }
    float lp(float x) noexcept { return y = x + a * (y - x); }
    float hp(float x) noexcept { return x - lp(x); }
    void reset() noexcept { y = 0.0f; }
};

// Schroeder allpass on its own buffer (fixed or modulated delay).
class Allpass {
public:
    void allocate(std::size_t max_len) { line_.prepare(max_len + 2); }
    void clear() noexcept { line_.reset(); }
    float process(float x, float delay, float g) noexcept {
        const float d = line_.read_linear(delay - 1.0f);
        const float v = x + g * d;
        line_.push(flush(v));
        return d - g * v;
    }

private:
    DelayLine line_;
};

// Sine-ish LFO from a phase accumulator (parabolic approximation, cheap and smooth).
inline float fast_sin(float phase01) noexcept {
    float x = phase01 - std::floor(phase01); // [0, 1)
    x = x * 2.0f - 1.0f;                     // [-1, 1): sin(pi * x) with sign flip
    const float y = 4.0f * x * (1.0f - std::fabs(x));
    return -(y * (0.775f + 0.225f * std::fabs(y)));
}

} // namespace saine::dsp
