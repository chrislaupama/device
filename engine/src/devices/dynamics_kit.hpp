// DSP building blocks of the dynamics devices (Character Compressor,
// Multiband Compressor): dB helpers over dsp.hpp fast math, biquads (scalar and four lanes),
// linear-phase halfband oversampling with an integer latency, delay lines.
// Header only; everything here is real-time safe after construction /
// prepare (no allocation, locks or I/O on the processing calls).
#pragma once

#include <algorithm>
#include <cmath>
#include <cstdint>
#include <cstring>
#include <vector>

#include "dsp.hpp"

namespace saine::dsp {

// ---------------------------------------------------------------- fast math

inline float fast_db_to_gain(float db) noexcept { return fast_exp2(db * 0.166096404744368f); }
inline float fast_gain_to_db(float g) noexcept { return 6.020599913279624f * fast_log2(std::max(g, 1e-30f)); }

// Rational tanh approximation (exact sign, |error| < 0.03, monotonic, saturates at +-1).
inline float soft_tanh(float x) noexcept {
    x = std::clamp(x, -3.0f, 3.0f);
    const float x2 = x * x;
    return x * (27.0f + x2) / (27.0f + 9.0f * x2);
}

// ---------------------------------------------------------------- four lanes

// Four floats processed together. Plain loops: the compiler turns each
// operator into one vector instruction (NEON / SSE) at -O2.
struct F4 {
    float v[4];
    static F4 all(float x) noexcept { return {{x, x, x, x}}; }
    float operator[](int i) const noexcept { return v[i]; }
    float& operator[](int i) noexcept { return v[i]; }
};
inline F4 operator+(F4 a, F4 b) noexcept {
    for (int i = 0; i < 4; ++i) a.v[i] += b.v[i];
    return a;
}
inline F4 operator-(F4 a, F4 b) noexcept {
    for (int i = 0; i < 4; ++i) a.v[i] -= b.v[i];
    return a;
}
inline F4 operator*(F4 a, F4 b) noexcept {
    for (int i = 0; i < 4; ++i) a.v[i] *= b.v[i];
    return a;
}
inline F4 min4(F4 a, F4 b) noexcept {
    for (int i = 0; i < 4; ++i) a.v[i] = std::min(a.v[i], b.v[i]);
    return a;
}
inline F4 max4(F4 a, F4 b) noexcept {
    for (int i = 0; i < 4; ++i) a.v[i] = std::max(a.v[i], b.v[i]);
    return a;
}

// ---------------------------------------------------------------- biquads

struct BiquadCoefs {
    float b0 = 1.0f, b1 = 0.0f, b2 = 0.0f, a1 = 0.0f, a2 = 0.0f;
};

// RBJ cookbook designs; f in Hz (clamped below Nyquist).
enum class BiquadKind { Lowpass, Highpass, Allpass, Peak, HighShelf };

inline BiquadCoefs design_biquad(BiquadKind kind, float f, float q, float gain_db, double rate) noexcept {
    const double w = 2.0 * 3.14159265358979323846 * std::clamp(static_cast<double>(f), 1.0, 0.49 * rate) / rate;
    const double cw = std::cos(w), sw = std::sin(w);
    const double alpha = sw / (2.0 * std::max(static_cast<double>(q), 1e-3));
    const double a = std::pow(10.0, static_cast<double>(gain_db) / 40.0);
    double b0 = 1, b1 = 0, b2 = 0, a0 = 1, a1 = 0, a2 = 0;
    switch (kind) {
    case BiquadKind::Lowpass:
        b0 = (1 - cw) / 2, b1 = 1 - cw, b2 = (1 - cw) / 2, a0 = 1 + alpha, a1 = -2 * cw, a2 = 1 - alpha;
        break;
    case BiquadKind::Highpass:
        b0 = (1 + cw) / 2, b1 = -(1 + cw), b2 = (1 + cw) / 2, a0 = 1 + alpha, a1 = -2 * cw, a2 = 1 - alpha;
        break;
    case BiquadKind::Allpass:
        b0 = 1 - alpha, b1 = -2 * cw, b2 = 1 + alpha, a0 = 1 + alpha, a1 = -2 * cw, a2 = 1 - alpha;
        break;
    case BiquadKind::Peak:
        b0 = 1 + alpha * a, b1 = -2 * cw, b2 = 1 - alpha * a, a0 = 1 + alpha / a, a1 = -2 * cw, a2 = 1 - alpha / a;
        break;
    case BiquadKind::HighShelf: {
        const double s = 2 * std::sqrt(a) * alpha;
        b0 = a * ((a + 1) + (a - 1) * cw + s), b1 = -2 * a * ((a - 1) + (a + 1) * cw), b2 = a * ((a + 1) + (a - 1) * cw - s);
        a0 = (a + 1) - (a - 1) * cw + s, a1 = 2 * ((a - 1) - (a + 1) * cw), a2 = (a + 1) - (a - 1) * cw - s;
        break;
    }
    }
    return {static_cast<float>(b0 / a0), static_cast<float>(b1 / a0), static_cast<float>(b2 / a0), static_cast<float>(a1 / a0),
            static_cast<float>(a2 / a0)};
}

// Transposed direct form II, one channel.
struct Biquad {
    BiquadCoefs c;
    float z1 = 0.0f, z2 = 0.0f;
    float process(float x) noexcept {
        const float y = c.b0 * x + z1;
        z1 = c.b1 * x - c.a1 * y + z2;
        z2 = c.b2 * x - c.a2 * y;
        return y;
    }
    void reset() noexcept { z1 = z2 = 0.0f; }
    void flush() noexcept {
        if (std::fabs(z1) < 1e-20f) z1 = 0.0f;
        if (std::fabs(z2) < 1e-20f) z2 = 0.0f;
    }
};

// Four independent biquads (own coefficients and state per lane).
struct Biquad4 {
    F4 b0 = F4::all(1.0f), b1{}, b2{}, a1{}, a2{};
    F4 z1{}, z2{};
    void set(int lane, const BiquadCoefs& c) noexcept {
        b0[lane] = c.b0;
        b1[lane] = c.b1;
        b2[lane] = c.b2;
        a1[lane] = c.a1;
        a2[lane] = c.a2;
    }
    F4 process(F4 x) noexcept {
        const F4 y = b0 * x + z1;
        z1 = b1 * x - a1 * y + z2;
        z2 = b2 * x - a2 * y;
        return y;
    }
    void reset() noexcept { z1 = z2 = F4{}; }
    void flush() noexcept {
        for (int i = 0; i < 4; ++i) {
            if (std::fabs(z1[i]) < 1e-20f) z1[i] = 0.0f;
            if (std::fabs(z2[i]) < 1e-20f) z2[i] = 0.0f;
        }
    }
};

// ---------------------------------------------------------------- oversampling

// One 2x stage: Kaiser-windowed linear-phase halfband FIR of `taps` taps
// (taps = 4k + 3, so the centre tap is odd), polyphase. Interpolation and
// decimation each delay by (taps - 1) / 2 samples of the faster rate.
class Halfband {
public:
    void prepare(uint32_t taps, double beta) {
        taps_ = taps;
        centre_ = (taps - 1) / 2;
        const uint32_t m = (taps + 1) / 2; // even-index taps
        even_.assign(m, 0.0f);
        double sum = 0.0;
        std::vector<double> h(taps);
        for (uint32_t j = 0; j < taps; ++j) {
            const double k = static_cast<double>(j) - static_cast<double>(centre_);
            const double x = 3.14159265358979323846 * k / 2.0;
            const double sinc = k == 0.0 ? 1.0 : std::sin(x) / x;
            const double r = k / static_cast<double>(centre_);
            h[j] = 0.5 * sinc * bessel_i0(beta * std::sqrt(std::max(0.0, 1.0 - r * r))) / bessel_i0(beta);
            sum += h[j];
        }
        for (uint32_t j = 0; j < taps; j += 2) even_[j / 2] = static_cast<float>(h[j] / sum);
        centre_tap_ = static_cast<float>(h[centre_] / sum);
        m_ = m;
        up_.assign(2 * m, 0.0f);
        dn_.assign(2 * m, 0.0f);
        odd_.assign(2 * (centre_ + 1) / 2 + 2, 0.0f);
        reset();
    }
    void reset() noexcept {
        std::fill(up_.begin(), up_.end(), 0.0f);
        std::fill(dn_.begin(), dn_.end(), 0.0f);
        std::fill(odd_.begin(), odd_.end(), 0.0f);
        up_pos_ = dn_pos_ = odd_pos_ = 0;
    }
    // One input sample -> two output samples at twice the rate.
    void up(float x, float& y0, float& y1) noexcept {
        up_pos_ = up_pos_ == 0 ? m_ - 1 : up_pos_ - 1;
        up_[up_pos_] = x;
        up_[up_pos_ + m_] = x;
        y0 = 2.0f * dot(&up_[up_pos_]);
        y1 = 2.0f * centre_tap_ * up_[up_pos_ + (centre_ - 1) / 2];
    }
    // Two input samples (even, odd) -> one output sample at half the rate.
    float down(float v0, float v1) noexcept {
        dn_pos_ = dn_pos_ == 0 ? m_ - 1 : dn_pos_ - 1;
        dn_[dn_pos_] = v0;
        dn_[dn_pos_ + m_] = v0;
        const auto n_odd = static_cast<uint32_t>(odd_.size() / 2);
        odd_pos_ = odd_pos_ == 0 ? n_odd - 1 : odd_pos_ - 1;
        odd_[odd_pos_] = v1;
        odd_[odd_pos_ + n_odd] = v1;
        return dot(&dn_[dn_pos_]) + centre_tap_ * odd_[odd_pos_ + (centre_ + 1) / 2];
    }
    uint32_t taps() const noexcept { return taps_; }

private:
    static double bessel_i0(double x) {
        double s = 1.0, t = 1.0;
        for (int k = 1; k < 40; ++k) {
            t *= (x / (2.0 * k)) * (x / (2.0 * k));
            s += t;
        }
        return s;
    }
    float dot(const float* h) const noexcept {
        float acc[4] = {0.0f, 0.0f, 0.0f, 0.0f};
        uint32_t j = 0;
        for (; j + 4 <= m_; j += 4)
            for (uint32_t k = 0; k < 4; ++k) acc[k] += even_[j + k] * h[j + k];
        float s = (acc[0] + acc[1]) + (acc[2] + acc[3]);
        for (; j < m_; ++j) s += even_[j] * h[j];
        return s;
    }
    std::vector<float> even_, up_, dn_, odd_;
    float centre_tap_ = 0.5f;
    uint32_t taps_ = 0, centre_ = 0, m_ = 0, up_pos_ = 0, dn_pos_ = 0, odd_pos_ = 0;
};

// Stereo 2x / 4x oversampling around a per-sample function, with a whole
// number of base-rate samples of latency: 2x = (63 - 1) / 2 = 31; 4x adds the
// second stage's 4.5 samples plus half a sample of delay at 2x = 36.
class Oversampler {
public:
    static constexpr uint32_t kTaps1 = 63, kTaps2 = 19;
    void prepare() {
        for (auto& h : s1_) h.prepare(kTaps1, 8.0);
        for (auto& h : s2_) h.prepare(kTaps2, 8.0);
        reset();
    }
    void reset() noexcept {
        for (auto& h : s1_) h.reset();
        for (auto& h : s2_) h.reset();
        prev_[0] = prev_[1] = 0.0f;
    }
    static constexpr uint32_t latency(int factor) noexcept { return factor >= 4 ? 36u : 31u; }

    // f(l, r) runs at the oversampled rate on each sample pair (in place).
    template <class F>
    void process(float* l, float* r, uint32_t n, int factor, F&& f) noexcept {
        float* ch[2] = {l, r};
        for (uint32_t i = 0; i < n; ++i) {
            float a[2][2];
            for (int c = 0; c < 2; ++c) s1_[c].up(ch[c][i], a[c][0], a[c][1]);
            if (factor < 4) {
                f(a[0][0], a[1][0]);
                f(a[0][1], a[1][1]);
                for (int c = 0; c < 2; ++c) ch[c][i] = s1_[c].down(a[c][0], a[c][1]);
                continue;
            }
            float s[2][4];
            for (int c = 0; c < 2; ++c) {
                s2_[c].up(a[c][0], s[c][0], s[c][1]);
                s2_[c].up(a[c][1], s[c][2], s[c][3]);
            }
            for (int k = 0; k < 4; ++k) f(s[0][k], s[1][k]);
            for (int c = 0; c < 2; ++c) {
                const float b0 = s2_[c].down(s[c][0], s[c][1]);
                const float b1 = s2_[c].down(s[c][2], s[c][3]);
                ch[c][i] = s1_[c].down(prev_[c], b0);
                prev_[c] = b1;
            }
        }
    }

private:
    Halfband s1_[2], s2_[2];
    float prev_[2] = {0.0f, 0.0f};
};

} // namespace saine::dsp
