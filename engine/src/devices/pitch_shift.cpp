#include "pitch_shift.hpp"

#include <algorithm>
#include <cmath>
#include <cstring>

namespace saine::dsp {

namespace {
constexpr float kTwoPiF = 6.28318530717958647692f;
inline float wrap_pi(float x) noexcept {
    // to [-pi, pi)
    return x - kTwoPiF * std::floor((x + 3.14159265358979323846f) / kTwoPiF);
}
} // namespace

void PitchShifter::prepare(uint32_t fft_size) {
    n_ = fft_size;
    hop_ = n_ / 4;
    bins_ = n_ / 2 + 1;
    fft_.init(n_);
    window_.resize(n_);
    for (uint32_t i = 0; i < n_; ++i)
        window_[i] = static_cast<float>(0.5 - 0.5 * std::cos(2.0 * 3.14159265358979323846 * i / n_));
    for (auto* v : {&in_fifo_, &out_fifo_, &frame_}) v->assign(n_, 0.0f);
    accum_.assign(2 * n_, 0.0f);
    for (auto* v : {&re_, &im_, &mag_, &phase_, &last_phase_, &omega_, &out_phase_acc_, &out_re_, &out_im_, &out_mag_, &out_ph_, &env_in_, &cep_re_, &cep_im_})
        v->assign(bins_, 0.0f);
    cep_.assign(n_, 0.0f);
    peaks_.assign(bins_, 0);
    reset();
}

void PitchShifter::reset() noexcept {
    for (auto* v : {&in_fifo_, &out_fifo_, &accum_, &last_phase_, &out_phase_acc_})
        std::fill(v->begin(), v->end(), 0.0f);
    pos_ = latency();
}

void PitchShifter::set_ratio(float ratio) noexcept { ratio_ = std::clamp(ratio, 0.25f, 4.0f); }

void PitchShifter::process(const float* in, float* out, uint32_t n) noexcept {
    const uint32_t lat = latency();
    for (uint32_t i = 0; i < n; ++i) {
        const float x = in[i];
        in_fifo_[pos_] = x;
        out[i] = out_fifo_[pos_ - lat];
        if (++pos_ >= n_) {
            pos_ = lat;
            frame();
        }
    }
}

// Smooth log-magnitude envelope by cepstral liftering.
void PitchShifter::envelope(const float* mag, float* env) noexcept {
    for (uint32_t k = 0; k < bins_; ++k) {
        cep_re_[k] = std::log(mag[k] + 1e-9f);
        cep_im_[k] = 0.0f;
    }
    fft_.inverse(cep_re_.data(), cep_im_.data(), cep_.data());
    // Keep quefrencies below ~1.5 ms (formant scale), with a short taper.
    const auto keep = std::max<uint32_t>(12, n_ / 48);
    for (uint32_t q = 0; q < n_; ++q) {
        const uint32_t d = std::min(q, n_ - q);
        float w = 0.0f;
        if (d < keep) w = 1.0f;
        else if (d < keep + 4) w = 0.5f + 0.5f * std::cos(3.14159265f * static_cast<float>(d - keep) / 4.0f);
        cep_[q] *= w;
    }
    fft_.forward(cep_.data(), cep_re_.data(), cep_im_.data());
    for (uint32_t k = 0; k < bins_; ++k) env[k] = std::exp(std::clamp(cep_re_[k], -40.0f, 20.0f));
}

void PitchShifter::frame() noexcept {
    // Analysis window over the last n_ input samples (in_fifo_ is in time order).
    for (uint32_t i = 0; i < n_; ++i) frame_[i] = in_fifo_[i] * window_[i];
    fft_.forward(frame_.data(), re_.data(), im_.data());

    const float expect = kTwoPiF * static_cast<float>(hop_) / static_cast<float>(n_);
    for (uint32_t k = 0; k < bins_; ++k) {
        mag_[k] = std::sqrt(re_[k] * re_[k] + im_[k] * im_[k]);
        const float ph = std::atan2(im_[k], re_[k]);
        const float dev = wrap_pi(ph - last_phase_[k] - expect * static_cast<float>(k));
        last_phase_[k] = ph;
        phase_[k] = ph;
        omega_[k] = expect * static_cast<float>(k) + dev; // phase advance per hop
    }

    std::fill(out_mag_.begin(), out_mag_.end(), 0.0f);
    std::fill(out_ph_.begin(), out_ph_.end(), 0.0f);
    if (formant_) envelope(mag_.data(), env_in_.data());

    // Peaks: local maxima above a floor relative to the frame maximum.
    float maxm = 0.0f;
    for (uint32_t k = 0; k < bins_; ++k) maxm = std::max(maxm, mag_[k]);
    uint32_t np = 0;
    const float floor_mag = maxm * 1e-5f;
    for (uint32_t k = 2; k + 2 < bins_; ++k) {
        const float m = mag_[k];
        if (m > floor_mag && m > mag_[k - 1] && m >= mag_[k + 1] && m > mag_[k - 2] && m >= mag_[k + 2]) peaks_[np++] = static_cast<int32_t>(k);
    }

    const float r = ratio_;
    const auto bins = static_cast<int32_t>(bins_);
    // Advance the synthesis phase of each moved peak; carry its region along.
    std::vector<float>& acc = out_phase_acc_;
    for (uint32_t p = 0; p < np; ++p) {
        const int32_t k = peaks_[p];
        const int32_t lo = p == 0 ? 0 : (peaks_[p - 1] + k + 1) / 2;
        const int32_t hi = p + 1 == np ? bins - 1 : (k + peaks_[p + 1]) / 2;
        const auto target = static_cast<int32_t>(std::lround(static_cast<float>(k) * r));
        const int32_t shift = target - k;
        if (target <= 0 || target >= bins - 1) continue;
        // Phase of the peak at its new place: previous synthesis phase there + shifted advance.
        const float ph_new = wrap_pi(acc[static_cast<uint32_t>(target)] + omega_[static_cast<uint32_t>(k)] * r);
        const float rot = ph_new - phase_[static_cast<uint32_t>(k)];
        for (int32_t b = lo; b <= hi; ++b) {
            const int32_t j = b + shift;
            if (j <= 0 || j >= bins) continue;
            const auto uj = static_cast<uint32_t>(j), ub = static_cast<uint32_t>(b);
            float m = mag_[ub];
            if (formant_) m *= std::min(32.0f, env_in_[uj] / std::max(env_in_[ub], 1e-9f));
            if (m > out_mag_[uj]) {
                out_mag_[uj] = m;
                out_ph_[uj] = phase_[ub] + rot;
            }
        }
    }
    for (uint32_t j = 0; j < bins_; ++j) {
        acc[j] = wrap_pi(out_ph_[j]);
        out_re_[j] = out_mag_[j] * std::cos(out_ph_[j]);
        out_im_[j] = out_mag_[j] * std::sin(out_ph_[j]);
    }
    out_im_[0] = 0.0f;
    out_im_[bins_ - 1] = 0.0f;
    fft_.inverse(out_re_.data(), out_im_.data(), frame_.data());

    // Overlap-add (Hann^2 at 4x overlap sums to 1.5).
    constexpr float kNorm = 1.0f / 1.5f;
    for (uint32_t i = 0; i < n_; ++i) accum_[i] += frame_[i] * window_[i] * kNorm;
    std::memcpy(out_fifo_.data(), accum_.data(), hop_ * sizeof(float));
    std::memmove(accum_.data(), accum_.data() + hop_, n_ * sizeof(float));
    std::fill(accum_.begin() + n_, accum_.begin() + n_ + hop_, 0.0f);
    std::memmove(in_fifo_.data(), in_fifo_.data() + hop_, (n_ - hop_) * sizeof(float));
}

} // namespace saine::dsp
