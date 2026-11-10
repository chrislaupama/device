// Linear-phase 4x oversampling for the nonlinear built-in devices (header
// only). Two half-band FIR stages (2x, then 2x again) in polyphase form: every
// stage runs one short symmetric FIR per input sample (SIMD dot product) plus
// a pure delay for the other phase.
//
// Design: Kaiser-windowed sinc half-bands. Stage 1 (base <-> 2x) has 18
// non-zero taps per side (71-tap prototype, ~90 dB stopband from 28 kHz at
// 48 kHz in); stage 2 (2x <-> 4x) 5 per side (19-tap prototype, ~80 dB, its
// transition band is wide because stage 1 already removed 28..48 kHz).
// The round trip delays the signal by exactly kLatency base-rate samples
// (35 + 4.5, plus one 2x-rate sample to make it whole), so a dry path delayed
// by kLatency lines up sample for sample (Mix) and the device reports it for
// delay compensation.
//
// prepare() allocates; up() / down() are RT safe.
#pragma once

#include "simd.hpp"

#include <algorithm>
#include <cmath>
#include <cstdint>
#include <cstring>
#include <vector>

namespace dstudio::dsp {

// Modified Bessel function of the first kind, order 0 (window design only).
inline double bessel_i0(double x) noexcept {
    double sum = 1.0, term = 1.0;
    for (int k = 1; k < 64; ++k) {
        term *= (x / (2.0 * k)) * (x / (2.0 * k));
        sum += term;
        if (term < 1e-12 * sum) break;
    }
    return sum;
}

// Half-band filter with P non-zero odd taps per side (prototype length 4P - 1,
// M = 2P - 1). The odd-tap branch has L = 2P taps; padded to a multiple of 4.
template <int P>
struct HalfbandCoeffs {
    static constexpr int L = 2 * P;
    static constexpr int LP = (L + 3) & ~3;
    // c[i] multiplies the i-th oldest sample of an LP-sample window (zeros
    // first for the padding), already scaled by 2 (interpolation gain).
    alignas(16) float c[LP] = {};
    explicit HalfbandCoeffs(double beta) {
        constexpr double kPiD = 3.14159265358979323846;
        const int M = 2 * P - 1;
        double taps[L];
        for (int k = 0; k < L; ++k) {
            const int n = -M + 2 * k; // odd offsets -M..M
            const double sinc = std::sin(kPiD * n / 2.0) / (kPiD * n);
            const double r = static_cast<double>(n) / (M + 1);
            const double w = bessel_i0(beta * std::sqrt(std::max(0.0, 1.0 - r * r))) / bessel_i0(beta);
            taps[k] = sinc * w;
        }
        // Normalise: odd taps sum to 0.5 so DC gain is exactly 1.
        double s = 0.0;
        for (double t : taps) s += t;
        for (int k = 0; k < L; ++k) c[LP - L + k] = static_cast<float>(2.0 * taps[k] * (0.5 / s));
    }
};

// One channel of a 2x interpolator: per input sample, two output samples.
template <int P>
struct HalfbandUp {
    static constexpr int LP = HalfbandCoeffs<P>::LP;
    float hist[2 * LP] = {};
    int pos = 0;
    void reset() noexcept {
        std::memset(hist, 0, sizeof hist);
        pos = 0;
    }
    // out[0], out[1]: the two output samples for input x, delay 2P - 1 at 2x.
    void process(float x, const HalfbandCoeffs<P>& k, float* out) noexcept {
        hist[pos] = x;
        hist[pos + LP] = x;
        pos = pos + 1 == LP ? 0 : pos + 1;
        const float* w = hist + pos; // oldest .. newest
        out[0] = dot4(w, k.c, LP);
        out[1] = w[LP - P]; // the centre tap: x delayed by P - 1
    }
};

// One channel of a 2x decimator: per two input samples, one output sample.
template <int P>
struct HalfbandDown {
    static constexpr int LP = HalfbandCoeffs<P>::LP;
    float even[2 * LP] = {};
    float odd[P] = {};
    int pos = 0, opos = 0;
    void reset() noexcept {
        std::memset(even, 0, sizeof even);
        std::memset(odd, 0, sizeof odd);
        pos = opos = 0;
    }
    float process(float a, float b, const HalfbandCoeffs<P>& k) noexcept {
        even[pos] = a;
        even[pos + LP] = a;
        pos = pos + 1 == LP ? 0 : pos + 1;
        const float* w = even + pos;
        // Odd-phase sample delayed by P (the centre tap), coefficient 1/2.
        const float centre = odd[opos];
        odd[opos] = b;
        opos = opos + 1 == P ? 0 : opos + 1;
        return 0.5f * dot4(w, k.c, LP) + 0.5f * centre;
    }
};

constexpr int kStage1Taps = 18;
constexpr int kStage2Taps = 5;

inline const HalfbandCoeffs<kStage1Taps>& stage1_coeffs() {
    static const HalfbandCoeffs<kStage1Taps> c(9.2);
    return c;
}
inline const HalfbandCoeffs<kStage2Taps>& stage2_coeffs() {
    static const HalfbandCoeffs<kStage2Taps> c(8.0);
    return c;
}

// Stereo 4x oversampler. Usage per block of n base samples:
//   float* hi = os.up(l, r, n);   // 4n interleaved-by-channel samples: hi[0..4n) left, hi[4n..8n) right
//   ... process hi in place ...
//   os.down(l, r, n);             // back to n base samples (delayed by kLatency)
class Oversampler4x {
public:
    static constexpr uint32_t kLatency = 40;

    void prepare(uint32_t max_block) {
        max_ = max_block;
        buf_.assign(8 * static_cast<std::size_t>(max_block), 0.0f);
        (void)stage1_coeffs();
        (void)stage2_coeffs();
        reset();
    }
    void reset() noexcept {
        for (int c = 0; c < 2; ++c) {
            up1_[c].reset();
            up2_[c].reset();
            down1_[c].reset();
            down2_[c].reset();
            z_[c] = 0.0f;
        }
    }
    uint32_t max_block() const noexcept { return max_; }
    float* left() noexcept { return buf_.data(); }
    float* right() noexcept { return buf_.data() + 4 * static_cast<std::size_t>(max_); }

    // n <= max_block. Writes 4n samples per channel into left() / right().
    void up(const float* l, const float* r, uint32_t n) noexcept {
        const auto& k1 = stage1_coeffs();
        const auto& k2 = stage2_coeffs();
        for (int c = 0; c < 2; ++c) {
            const float* in = c == 0 ? l : r;
            float* out = c == 0 ? left() : right();
            for (uint32_t i = 0; i < n; ++i) {
                float two[2];
                up1_[c].process(in[i], k1, two);
                up2_[c].process(two[0], k2, out + 4 * i);
                up2_[c].process(two[1], k2, out + 4 * i + 2);
            }
        }
    }
    // Decimates left() / right() (4n samples each) into l / r (n samples).
    void down(float* l, float* r, uint32_t n) noexcept {
        const auto& k1 = stage1_coeffs();
        const auto& k2 = stage2_coeffs();
        for (int c = 0; c < 2; ++c) {
            const float* in = c == 0 ? left() : right();
            float* out = c == 0 ? l : r;
            float z = z_[c];
            for (uint32_t i = 0; i < n; ++i) {
                const float a = down2_[c].process(in[4 * i], in[4 * i + 1], k2);
                const float b = down2_[c].process(in[4 * i + 2], in[4 * i + 3], k2);
                // One 2x-rate sample of delay makes the round trip a whole number of base samples.
                out[i] = down1_[c].process(z, a, k1);
                z = b;
            }
            z_[c] = z;
        }
    }

private:
    uint32_t max_ = 0;
    std::vector<float> buf_;
    HalfbandUp<kStage1Taps> up1_[2];
    HalfbandUp<kStage2Taps> up2_[2];
    HalfbandDown<kStage1Taps> down1_[2];
    HalfbandDown<kStage2Taps> down2_[2];
    float z_[2] = {0.0f, 0.0f};
};

} // namespace dstudio::dsp
