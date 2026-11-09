// Shared spectral kernels for the built-ins (docs/devices.md): a real-input
// FFT and the analysis windows. init() / make_window() allocate nothing
// beyond the tables sized in init() (call them from prepare()); forward(),
// inverse() and fill_window() are RT safe.
//
// The FFT is an iterative radix-2 complex transform of size n/2 on split
// real / imaginary arrays, wrapped as a real transform of size n. Twiddles
// are stored per stage, contiguous, so every butterfly loop from the third
// stage on runs over unit-stride arrays and vectorises (NEON / SSE / AVX)
// without intrinsics.
#pragma once

#include <cmath>
#include <cstdint>
#include <vector>

namespace saine::dsp {

class Fft {
public:
    // n: power of two, 8 <= n. Non-RT (allocates).
    void init(uint32_t n) {
        n_ = n;
        m_ = n / 2;
        uint32_t bits = 0;
        while ((1u << bits) < m_) ++bits;
        rev_.assign(m_, 0);
        for (uint32_t i = 0; i < m_; ++i) {
            uint32_t r = 0;
            for (uint32_t b = 0; b < bits; ++b)
                if ((i >> b) & 1u) r |= 1u << (bits - 1 - b);
            rev_[i] = r;
        }
        // Stage twiddles: for half-length h, exp(-i pi j / h), j < h, at offset h - 1.
        twr_.assign(m_ > 1 ? m_ - 1 : 1, 1.0f);
        twi_.assign(twr_.size(), 0.0f);
        for (uint32_t h = 1; h < m_; h <<= 1)
            for (uint32_t j = 0; j < h; ++j) {
                const double a = -kPi * j / h;
                twr_[h - 1 + j] = static_cast<float>(std::cos(a));
                twi_[h - 1 + j] = static_cast<float>(std::sin(a));
            }
        // Real split: W^k = exp(-2 pi i k / n), k <= m.
        spr_.resize(m_ + 1);
        spi_.resize(m_ + 1);
        for (uint32_t k = 0; k <= m_; ++k) {
            const double a = -2.0 * kPi * k / n_;
            spr_[k] = static_cast<float>(std::cos(a));
            spi_[k] = static_cast<float>(std::sin(a));
        }
        zr_.assign(m_ + 1, 0.0f);
        zi_.assign(m_ + 1, 0.0f);
    }

    uint32_t size() const noexcept { return n_; }

    // x[n] -> re[0..n/2], im[0..n/2] (unscaled; im[0] = im[n/2] = 0).
    void forward(const float* x, float* re, float* im) noexcept {
        for (uint32_t i = 0; i < m_; ++i) {
            zr_[rev_[i]] = x[2 * i];
            zi_[rev_[i]] = x[2 * i + 1];
        }
        complex_pass(1.0f);
        zr_[m_] = zr_[0];
        zi_[m_] = zi_[0];
        for (uint32_t k = 0; k <= m_; ++k) {
            // Even / odd halves from Z[k] and conj(Z[m - k]).
            const float ar = zr_[k], ai = zi_[k];
            const float br = zr_[m_ - k], bi = -zi_[m_ - k];
            const float er = 0.5f * (ar + br), ei = 0.5f * (ai + bi);
            const float orr = 0.5f * (ai - bi), oi = -0.5f * (ar - br);
            re[k] = er + spr_[k] * orr - spi_[k] * oi;
            im[k] = ei + spr_[k] * oi + spi_[k] * orr;
        }
    }

    // re/im[0..n/2] -> x[n], scaled by 1/n (inverse(forward(x)) == x).
    void inverse(const float* re, const float* im, float* x) noexcept {
        for (uint32_t k = 0; k < m_; ++k) {
            const float ar = re[k], ai = im[k];
            const float br = re[m_ - k], bi = -im[m_ - k];
            const float er = 0.5f * (ar + br), ei = 0.5f * (ai + bi);
            const float dr = 0.5f * (ar - br), di = 0.5f * (ai - bi);
            // odd = d * conj(W^k); Z = even + i * odd
            const float orr = dr * spr_[k] + di * spi_[k], oi = di * spr_[k] - dr * spi_[k];
            zr_[rev_[k]] = er - oi;
            zi_[rev_[k]] = ei + orr;
        }
        complex_pass(-1.0f);
        const float s = 1.0f / static_cast<float>(m_);
        for (uint32_t i = 0; i < m_; ++i) {
            x[2 * i] = zr_[i] * s;
            x[2 * i + 1] = zi_[i] * s;
        }
    }

private:
    static constexpr double kPi = 3.14159265358979323846;

    // In place on zr_/zi_ (bit-reversed input). sign -1 = inverse (conjugate twiddles).
    void complex_pass(float sign) noexcept {
        float* const zr = zr_.data();
        float* const zi = zi_.data();
        if (m_ >= 2) // h = 1: twiddle 1
            for (uint32_t i = 0; i < m_; i += 2) {
                const float xr = zr[i + 1], xi = zi[i + 1];
                zr[i + 1] = zr[i] - xr;
                zi[i + 1] = zi[i] - xi;
                zr[i] += xr;
                zi[i] += xi;
            }
        if (m_ >= 4) // h = 2: twiddles 1 and -i (forward) / +i (inverse)
            for (uint32_t i = 0; i < m_; i += 4) {
                float xr = zr[i + 2], xi = zi[i + 2];
                zr[i + 2] = zr[i] - xr;
                zi[i + 2] = zi[i] - xi;
                zr[i] += xr;
                zi[i] += xi;
                xr = sign * zi[i + 3];
                xi = -sign * zr[i + 3];
                zr[i + 3] = zr[i + 1] - xr;
                zi[i + 3] = zi[i + 1] - xi;
                zr[i + 1] += xr;
                zi[i + 1] += xi;
            }
        for (uint32_t h = 4; h < m_; h <<= 1) {
            const float* const wr = twr_.data() + h - 1;
            const float* const wi0 = twi_.data() + h - 1;
            for (uint32_t i = 0; i < m_; i += 2 * h) {
                float* const ar = zr + i;
                float* const ai = zi + i;
                float* const br = zr + i + h;
                float* const bi = zi + i + h;
                for (uint32_t j = 0; j < h; ++j) {
                    const float wi = sign * wi0[j];
                    const float xr = br[j] * wr[j] - bi[j] * wi;
                    const float xi = br[j] * wi + bi[j] * wr[j];
                    br[j] = ar[j] - xr;
                    bi[j] = ai[j] - xi;
                    ar[j] += xr;
                    ai[j] += xi;
                }
            }
        }
    }

    uint32_t n_ = 0, m_ = 0;
    std::vector<uint32_t> rev_;
    std::vector<float> twr_, twi_, spr_, spi_, zr_, zi_;
};

// Analysis windows (periodic form, as for spectral analysis).
enum class Window : int { Hann, Hamming, Blackman, BlackmanHarris, FlatTop, Rect };

// Fills w[0..n) (RT safe: no allocation). Returns the coherent gain (mean of
// the window), so a full-scale sine reads 0 dB after dividing by n * gain / 2.
inline float fill_window(Window kind, uint32_t n, float* w) noexcept {
    constexpr double kTau = 6.283185307179586476925;
    double sum = 0.0;
    for (uint32_t i = 0; i < n; ++i) {
        const double x = kTau * i / n;
        double v = 1.0;
        switch (kind) {
        case Window::Hann: v = 0.5 - 0.5 * std::cos(x); break;
        case Window::Hamming: v = 0.54 - 0.46 * std::cos(x); break;
        case Window::Blackman: v = 0.42 - 0.5 * std::cos(x) + 0.08 * std::cos(2 * x); break;
        case Window::BlackmanHarris:
            v = 0.35875 - 0.48829 * std::cos(x) + 0.14128 * std::cos(2 * x) - 0.01168 * std::cos(3 * x);
            break;
        case Window::FlatTop:
            v = 0.21557895 - 0.41663158 * std::cos(x) + 0.277263158 * std::cos(2 * x) - 0.083578947 * std::cos(3 * x) +
                0.006947368 * std::cos(4 * x);
            break;
        case Window::Rect: break;
        }
        w[i] = static_cast<float>(v);
        sum += v;
    }
    return n > 0 ? static_cast<float>(sum / n) : 1.0f;
}

} // namespace saine::dsp
