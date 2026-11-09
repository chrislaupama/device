// Spectral helpers for the built-in devices (RT safe after prepare()):
//  - RealFft: in-place radix-2 FFT of a real frame (split-radix packing of an
//    N/2-point complex transform, per-stage contiguous twiddles so the
//    butterflies vectorise), with an exact inverse.
//  - SpectrumTap: the analyser behind the restoration devices' spectrum
//    overlay. It reduces a Hann-windowed FFT of the input to kSpectrumBands
//    log-spaced bands (20 Hz .. 20 kHz) in dB, where a full-scale sine reads
//    0 dB, and writes them as the last kSpectrumBands readouts of a device.
//    The panel applies the display ballistics (rise / fall, decay).
#pragma once

#include <algorithm>
#include <cmath>
#include <cstdint>
#include <vector>

namespace saine::dsp {

constexpr double kPi = 3.14159265358979323846;

class RealFft {
public:
    // Non-RT: n is a power of two >= 8.
    void prepare(uint32_t n) {
        n_ = n;
        m_ = n / 2;
        uint32_t bits = 0;
        while ((1u << bits) < m_) ++bits;
        rev_.assign(m_, 0);
        for (uint32_t i = 0; i < m_; ++i) {
            uint32_t r = 0;
            for (uint32_t b = 0; b < bits; ++b) r |= ((i >> b) & 1u) << (bits - 1 - b);
            rev_[i] = r;
        }
        tw_re_.clear();
        tw_im_.clear();
        for (uint32_t len = 2; len <= m_; len <<= 1)
            for (uint32_t j = 0; j < len / 2; ++j) {
                const double a = -2.0 * kPi * j / len;
                tw_re_.push_back(static_cast<float>(std::cos(a)));
                tw_im_.push_back(static_cast<float>(std::sin(a)));
            }
        split_re_.resize(m_ + 1);
        split_im_.resize(m_ + 1);
        for (uint32_t k = 0; k <= m_; ++k) {
            const double a = -2.0 * kPi * k / n;
            split_re_[k] = static_cast<float>(std::cos(a));
            split_im_[k] = static_cast<float>(std::sin(a));
        }
        zr_.assign(m_, 0.0f);
        zi_.assign(m_, 0.0f);
    }
    uint32_t size() const noexcept { return n_; }
    uint32_t bins() const noexcept { return m_ + 1; }

    // x: n samples. re / im: n/2 + 1 bins (unnormalised DFT).
    void forward(const float* x, float* re, float* im) noexcept {
        for (uint32_t k = 0; k < m_; ++k) {
            zr_[rev_[k]] = x[2 * k];
            zi_[rev_[k]] = x[2 * k + 1];
        }
        butterflies(zr_.data(), zi_.data());
        re[0] = zr_[0] + zi_[0];
        im[0] = 0.0f;
        re[m_] = zr_[0] - zi_[0];
        im[m_] = 0.0f;
        for (uint32_t k = 1; k < m_; ++k) {
            const float ar = zr_[k], ai = zi_[k], br = zr_[m_ - k], bi = -zi_[m_ - k];
            const float er = 0.5f * (ar + br), ei = 0.5f * (ai + bi);
            const float or_ = 0.5f * (ai - bi), oi = -0.5f * (ar - br);
            const float wr = split_re_[k], wi = split_im_[k];
            re[k] = er + wr * or_ - wi * oi;
            im[k] = ei + wr * oi + wi * or_;
        }
    }

    // re / im: n/2 + 1 bins -> x: n samples, so that inverse(forward(x)) == x.
    void inverse(const float* re, const float* im, float* x) noexcept {
        for (uint32_t k = 0; k < m_; ++k) {
            const float ar = re[k], ai = im[k], br = re[m_ - k], bi = -im[m_ - k];
            const float er = 0.5f * (ar + br), ei = 0.5f * (ai + bi);
            const float dr = 0.5f * (ar - br), di = 0.5f * (ai - bi);
            // O = D * conj(w): w = split twiddle (e^-i2pik/n), so conj gives e^+i2pik/n.
            const float wr = split_re_[k], wi = -split_im_[k];
            const float or_ = dr * wr - di * wi, oi = dr * wi + di * wr;
            // Z = E + iO; the inverse transform runs as a forward one on conj(Z).
            zr_[rev_[k]] = er - oi;
            zi_[rev_[k]] = -(ei + or_);
        }
        butterflies(zr_.data(), zi_.data());
        const float s = 1.0f / static_cast<float>(m_);
        for (uint32_t k = 0; k < m_; ++k) {
            x[2 * k] = zr_[k] * s;
            x[2 * k + 1] = -zi_[k] * s;
        }
    }

private:
    void butterflies(float* re, float* im) noexcept {
        const float* twr = tw_re_.data();
        const float* twi = tw_im_.data();
        for (uint32_t len = 2; len <= m_; len <<= 1) {
            const uint32_t half = len / 2;
            for (uint32_t i = 0; i < m_; i += len) {
                float* ar = re + i;
                float* ai = im + i;
                float* br = re + i + half;
                float* bi = im + i + half;
                for (uint32_t j = 0; j < half; ++j) {
                    const float tr = br[j] * twr[j] - bi[j] * twi[j];
                    const float ti = br[j] * twi[j] + bi[j] * twr[j];
                    br[j] = ar[j] - tr;
                    bi[j] = ai[j] - ti;
                    ar[j] += tr;
                    ai[j] += ti;
                }
            }
            twr += half;
            twi += half;
        }
    }

    uint32_t n_ = 0, m_ = 0;
    std::vector<uint32_t> rev_;
    std::vector<float> tw_re_, tw_im_, split_re_, split_im_, zr_, zi_;
};

// Smallest power of two >= v.
inline uint32_t pow2_at_least(uint32_t v) noexcept {
    uint32_t p = 1;
    while (p < v) p <<= 1;
    return p;
}

constexpr uint32_t kSpectrumBands = 48;
constexpr float kSpectrumLoHz = 20.0f;
constexpr float kSpectrumHiHz = 20000.0f;
// Readout ids of the spectrum bands (appended after a device's own readouts).
inline constexpr const char* kSpectrumIds[kSpectrumBands] = {
    "spec_00", "spec_01", "spec_02", "spec_03", "spec_04", "spec_05", "spec_06", "spec_07",
    "spec_08", "spec_09", "spec_10", "spec_11", "spec_12", "spec_13", "spec_14", "spec_15",
    "spec_16", "spec_17", "spec_18", "spec_19", "spec_20", "spec_21", "spec_22", "spec_23",
    "spec_24", "spec_25", "spec_26", "spec_27", "spec_28", "spec_29", "spec_30", "spec_31",
    "spec_32", "spec_33", "spec_34", "spec_35", "spec_36", "spec_37", "spec_38", "spec_39",
    "spec_40", "spec_41", "spec_42", "spec_43", "spec_44", "spec_45", "spec_46", "spec_47",
};

// Centre frequency of spectrum band i (log spaced, kSpectrumLoHz .. kSpectrumHiHz).
inline float spectrum_band_hz(uint32_t i) noexcept {
    return kSpectrumLoHz * std::pow(kSpectrumHiHz / kSpectrumLoHz, static_cast<float>(i) / (kSpectrumBands - 1));
}

class SpectrumTap {
public:
    // Non-RT. A 2048-point frame at 48 kHz (the frame length follows the
    // rate so the resolution in Hz stays the same), 50% overlap.
    void prepare(double sample_rate) {
        const uint32_t n = pow2_at_least(static_cast<uint32_t>(std::max(1.0, std::round(sample_rate / 48000.0))) * 2048u);
        fft_.prepare(n);
        hop_ = n / 2;
        ring_.assign(n, 0.0f);
        frame_.assign(n, 0.0f);
        re_.assign(fft_.bins(), 0.0f);
        im_.assign(fft_.bins(), 0.0f);
        window_.resize(n);
        double sum = 0.0;
        for (uint32_t i = 0; i < n; ++i) {
            window_[i] = static_cast<float>(0.5 - 0.5 * std::cos(2.0 * kPi * i / n));
            sum += static_cast<double>(window_[i]);
        }
        // A full-scale sine peaks at |X| = sum(w) / 2.
        const double ref = sum / 2.0;
        norm_ = static_cast<float>(1.0 / (ref * ref));
        const double bin_hz = sample_rate / n;
        for (uint32_t b = 0; b < kSpectrumBands; ++b) {
            const double lo = static_cast<double>(spectrum_band_hz(b)) / std::sqrt(std::pow(kSpectrumHiHz / kSpectrumLoHz, 1.0 / (kSpectrumBands - 1)));
            const double hi = lo * std::pow(kSpectrumHiHz / kSpectrumLoHz, 1.0 / (kSpectrumBands - 1));
            auto first = static_cast<uint32_t>(std::ceil(lo / bin_hz));
            auto last = static_cast<uint32_t>(std::ceil(hi / bin_hz));
            const uint32_t nyq = fft_.bins() - 1;
            if (last <= first) { // narrower than a bin: the nearest bin
                first = std::min(nyq, static_cast<uint32_t>(std::lround(static_cast<double>(spectrum_band_hz(b)) / bin_hz)));
                last = first + 1;
            }
            first_[b] = std::min(first, nyq);
            last_[b] = std::min(std::max(last, first_[b] + 1), nyq + 1);
        }
        reset();
    }

    void reset() noexcept {
        std::fill(ring_.begin(), ring_.end(), 0.0f);
        pos_ = 0;
        fill_ = 0;
        for (float& b : bands_) b = -120.0f;
    }

    // Feeds the mono sum of a stereo block.
    void push(const float* l, const float* r, uint32_t n) noexcept {
        const auto size = static_cast<uint32_t>(ring_.size());
        for (uint32_t i = 0; i < n; ++i) {
            ring_[pos_] = 0.5f * (l[i] + r[i]);
            pos_ = pos_ + 1 == size ? 0 : pos_ + 1;
            if (++fill_ == hop_) {
                fill_ = 0;
                analyse();
            }
        }
    }

    // Latest bands (dB) into out[0 .. kSpectrumBands).
    void write(float* out) const noexcept { std::copy(bands_, bands_ + kSpectrumBands, out); }

private:
    void analyse() noexcept {
        const auto size = static_cast<uint32_t>(ring_.size());
        for (uint32_t i = 0; i < size; ++i) {
            const uint32_t j = pos_ + i < size ? pos_ + i : pos_ + i - size;
            frame_[i] = ring_[j] * window_[i];
        }
        fft_.forward(frame_.data(), re_.data(), im_.data());
        for (uint32_t b = 0; b < kSpectrumBands; ++b) {
            float p = 0.0f;
            for (uint32_t k = first_[b]; k < last_[b]; ++k) p = std::max(p, re_[k] * re_[k] + im_[k] * im_[k]);
            const float v = p * norm_;
            bands_[b] = v > 1e-12f ? std::max(-120.0f, 10.0f * std::log10(v)) : -120.0f;
        }
    }

    RealFft fft_;
    uint32_t hop_ = 1024, pos_ = 0, fill_ = 0;
    float norm_ = 1.0f;
    std::vector<float> ring_, frame_, re_, im_, window_;
    uint32_t first_[kSpectrumBands] = {}, last_[kSpectrumBands] = {};
    float bands_[kSpectrumBands] = {};
};

} // namespace saine::dsp
