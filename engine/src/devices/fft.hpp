// Real FFT for the built-ins (pitch shifter, spectrum taps). Radix-2,
// iterative, tables built in init() (non-RT); forward()/inverse() are RT safe.
//
// forward(x[n]) -> re[0..n/2], im[0..n/2] (unscaled).
// inverse(re, im) -> x[n], scaled by 1/n so inverse(forward(x)) == x.
#pragma once

#include <cmath>
#include <cstdint>
#include <vector>

namespace saine::dsp {

class RealFft {
public:
    void init(uint32_t n) {
        n_ = n;
        m_ = n / 2;
        log2m_ = 0;
        while ((1u << log2m_) < m_) ++log2m_;
        rev_.assign(m_, 0);
        for (uint32_t i = 0; i < m_; ++i) {
            uint32_t r = 0;
            for (uint32_t b = 0; b < log2m_; ++b)
                if ((i >> b) & 1u) r |= 1u << (log2m_ - 1 - b);
            rev_[i] = r;
        }
        // Twiddles of the half-size complex transform and of the real split.
        cm_.resize(m_ / 2 + 1);
        sm_.resize(m_ / 2 + 1);
        for (uint32_t k = 0; k <= m_ / 2; ++k) {
            const double a = -2.0 * 3.14159265358979323846 * k / m_;
            cm_[k] = static_cast<float>(std::cos(a));
            sm_[k] = static_cast<float>(std::sin(a));
        }
        cn_.resize(m_ + 1);
        sn_.resize(m_ + 1);
        for (uint32_t k = 0; k <= m_; ++k) {
            const double a = -2.0 * 3.14159265358979323846 * k / n_;
            cn_[k] = static_cast<float>(std::cos(a));
            sn_[k] = static_cast<float>(std::sin(a));
        }
        zr_.assign(m_ + 1, 0.0f);
        zi_.assign(m_ + 1, 0.0f);
    }

    uint32_t size() const noexcept { return n_; }

    void forward(const float* x, float* re, float* im) noexcept {
        for (uint32_t i = 0; i < m_; ++i) {
            zr_[rev_[i]] = x[2 * i];
            zi_[rev_[i]] = x[2 * i + 1];
        }
        complex_pass(false);
        zr_[m_] = zr_[0];
        zi_[m_] = zi_[0];
        for (uint32_t k = 0; k <= m_; ++k) {
            const float ar = zr_[k], ai = zi_[k];
            const float br = zr_[m_ - k], bi = -zi_[m_ - k]; // conj(Z[M-k])
            const float er = 0.5f * (ar + br), ei = 0.5f * (ai + bi);
            // odd = -i/2 (Z[k] - conj Z[M-k])
            const float orr = 0.5f * (ai - bi), oi = -0.5f * (ar - br);
            const float wr = cn_[k], wi = sn_[k];
            re[k] = er + (wr * orr - wi * oi);
            im[k] = ei + (wr * oi + wi * orr);
        }
    }

    void inverse(const float* re, const float* im, float* x) noexcept {
        for (uint32_t k = 0; k < m_; ++k) {
            const float ar = re[k], ai = im[k];
            const float br = re[m_ - k], bi = -im[m_ - k]; // conj(X[M-k])
            const float er = 0.5f * (ar + br), ei = 0.5f * (ai + bi);
            const float dr = 0.5f * (ar - br), di = 0.5f * (ai - bi);
            // odd = d / W^k = d * conj(W^k)
            const float wr = cn_[k], wi = -sn_[k];
            const float orr = dr * wr - di * wi, oi = dr * wi + di * wr;
            // Z = even + i * odd
            const float zr = er - oi, zi = ei + orr;
            zr_[rev_[k]] = zr;
            zi_[rev_[k]] = zi;
        }
        complex_pass(true);
        const float s = 1.0f / static_cast<float>(m_);
        for (uint32_t i = 0; i < m_; ++i) {
            x[2 * i] = zr_[i] * s;
            x[2 * i + 1] = zi_[i] * s;
        }
    }

private:
    // In-place iterative radix-2 on zr_/zi_ (already in bit-reversed order).
    void complex_pass(bool inverse) noexcept {
        const float sign = inverse ? -1.0f : 1.0f;
        for (uint32_t len = 2; len <= m_; len <<= 1) {
            const uint32_t half = len >> 1;
            const uint32_t step = m_ / len;
            for (uint32_t i = 0; i < m_; i += len) {
                for (uint32_t j = 0; j < half; ++j) {
                    const uint32_t t = j * step;
                    const float wr = t <= m_ / 2 ? cm_[t] : -cm_[m_ - t];
                    const float wi = sign * (t <= m_ / 2 ? sm_[t] : sm_[m_ - t]);
                    const uint32_t a = i + j, b = a + half;
                    const float xr = zr_[b] * wr - zi_[b] * wi;
                    const float xi = zr_[b] * wi + zi_[b] * wr;
                    zr_[b] = zr_[a] - xr;
                    zi_[b] = zi_[a] - xi;
                    zr_[a] += xr;
                    zi_[a] += xi;
                }
            }
        }
    }

    uint32_t n_ = 0, m_ = 0, log2m_ = 0;
    std::vector<uint32_t> rev_;
    std::vector<float> cm_, sm_, cn_, sn_, zr_, zi_;
};

} // namespace saine::dsp
