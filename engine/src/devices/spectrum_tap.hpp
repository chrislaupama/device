// Band levels of a signal for card visualisers (delay EQ wash, reverb EQ
// wash): a 1024-point FFT every 1024 samples, folded into 24 log-spaced bands
// from 40 Hz to 16 kHz (the reference's band layout), in dB. Bands fall back
// at a fixed rate when the signal stops, so the wash fades rather than
// freezing. RT safe after prepare().
#pragma once

#include "fft.hpp"

#include <algorithm>
#include <cmath>
#include <cstdint>
#include <vector>

namespace saine::dsp {

class SpectrumTap {
public:
    static constexpr uint32_t kBands = 24;
    static constexpr uint32_t kSize = 1024;

    void prepare(double sample_rate) {
        sr_ = sample_rate;
        fft_.init(kSize);
        buf_.assign(kSize, 0.0f);
        win_.resize(kSize);
        for (uint32_t i = 0; i < kSize; ++i) win_[i] = static_cast<float>(0.5 - 0.5 * std::cos(2.0 * 3.14159265358979323846 * i / kSize));
        re_.assign(kSize / 2 + 1, 0.0f);
        im_.assign(kSize / 2 + 1, 0.0f);
        // Band edges in bins (geometric centres 40 * 400^((i + 0.5) / 24)).
        for (uint32_t b = 0; b <= kBands; ++b) {
            const double hz = 40.0 * std::pow(400.0, static_cast<double>(b) / kBands);
            edge_[b] = std::clamp(static_cast<uint32_t>(hz * kSize / sr_ + 0.5), 1u, kSize / 2);
        }
        reset();
    }
    void reset() noexcept {
        std::fill(buf_.begin(), buf_.end(), 0.0f);
        fill_ = 0;
        for (float& b : bands_) b = -120.0f;
    }
    void push(const float* x, uint32_t n) noexcept {
        for (uint32_t i = 0; i < n; ++i) {
            buf_[fill_++] = x[i];
            if (fill_ == kSize) {
                analyse();
                fill_ = 0;
            }
        }
    }
    const float* bands() const noexcept { return bands_; }

private:
    void analyse() noexcept {
        for (uint32_t i = 0; i < kSize; ++i) buf_[i] *= win_[i];
        fft_.forward(buf_.data(), re_.data(), im_.data());
        // Hann coherent gain 0.5; power summed over each band -> dBFS of a sine in that band.
        constexpr float kScale = 2.0f / (0.5f * static_cast<float>(kSize));
        const float fall = 12.0f * static_cast<float>(kSize / sr_) * 4.0f; // ~48 dB/s fall back
        for (uint32_t b = 0; b < kBands; ++b) {
            float p = 0.0f;
            const uint32_t hi = std::max(edge_[b + 1], edge_[b] + 1);
            for (uint32_t k = edge_[b]; k < hi && k <= kSize / 2; ++k) p += re_[k] * re_[k] + im_[k] * im_[k];
            const float db = p > 1e-20f ? 10.0f * std::log10(p * kScale * kScale) : -120.0f;
            bands_[b] = std::max(std::max(db, -120.0f), bands_[b] - fall);
        }
    }

    double sr_ = 48000.0;
    RealFft fft_;
    std::vector<float> buf_, win_, re_, im_;
    uint32_t fill_ = 0;
    uint32_t edge_[kBands + 1] = {};
    float bands_[kBands] = {};
};

} // namespace saine::dsp
