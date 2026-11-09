#include "pitch_detect.hpp"

#include <algorithm>
#include <cmath>

namespace saine::dsp {

namespace {

constexpr double kPi = 3.14159265358979323846;

// RBJ biquads (Q = 1/sqrt 2 Butterworth sections).
void lowpass(float& b0, float& b1, float& b2, float& a1, float& a2, double fc, double rate, double q) {
    const double w = 2.0 * kPi * fc / rate, c = std::cos(w), al = std::sin(w) / (2.0 * q), a0 = 1.0 + al;
    b0 = static_cast<float>((1.0 - c) / 2.0 / a0);
    b1 = static_cast<float>((1.0 - c) / a0);
    b2 = b0;
    a1 = static_cast<float>(-2.0 * c / a0);
    a2 = static_cast<float>((1.0 - al) / a0);
}
void highpass(float& b0, float& b1, float& b2, float& a1, float& a2, double fc, double rate, double q) {
    const double w = 2.0 * kPi * fc / rate, c = std::cos(w), al = std::sin(w) / (2.0 * q), a0 = 1.0 + al;
    b0 = static_cast<float>((1.0 + c) / 2.0 / a0);
    b1 = static_cast<float>(-(1.0 + c) / a0);
    b2 = b0;
    a1 = static_cast<float>(-2.0 * c / a0);
    a2 = static_cast<float>((1.0 - al) / a0);
}

uint32_t pow2_at_least(double v) {
    uint32_t n = 8;
    while (n < v) n <<= 1;
    return n;
}

} // namespace

void PitchDetector::prepare(double sample_rate, float min_hz, float max_hz, float hop_ms) {
    const double sr = sample_rate > 0.0 ? sample_rate : 48000.0;
    decim_ = std::max<uint32_t>(1, static_cast<uint32_t>(sr / 20000.0));
    rate_ = sr / decim_;
    const double lo = std::max(20.0, static_cast<double>(min_hz));
    const double hi = std::min(rate_ / 4.0, std::max(lo * 2.0, static_cast<double>(max_hz)));
    w_ = pow2_at_least(2.0 * rate_ / lo);
    hop_ = std::max<uint32_t>(16, static_cast<uint32_t>(static_cast<double>(hop_ms) * 1e-3 * rate_));
    tau_min_ = std::max<uint32_t>(2, static_cast<uint32_t>(rate_ / hi));
    tau_max_ = std::min<uint32_t>(w_ / 2, static_cast<uint32_t>(std::ceil(rate_ / lo)) + 2);
    lag_ = (w_ / 2) * decim_;
    constexpr double q = 0.70710678;
    highpass(hp_.b0, hp_.b1, hp_.b2, hp_.a1, hp_.a2, std::min(lo * 0.5, 30.0), sr, q);
    const double fc = std::min(0.42 * rate_, 0.45 * sr);
    lowpass(lp1_.b0, lp1_.b1, lp1_.b2, lp1_.a1, lp1_.a2, fc, sr, 0.54119610);
    lowpass(lp2_.b0, lp2_.b1, lp2_.b2, lp2_.a1, lp2_.a2, fc, sr, 1.3065630);
    ring_.assign(w_, 0.0f);
    fft_.prepare(2 * w_);
    frame_.assign(2 * w_, 0.0f);
    re_.assign(w_ + 1, 0.0f);
    im_.assign(w_ + 1, 0.0f);
    acf_.assign(2 * w_, 0.0f);
    nsdf_.assign(tau_max_ + 2, 0.0f);
    reset();
}

void PitchDetector::reset() noexcept {
    for (Biquad* b : {&hp_, &lp1_, &lp2_}) b->z1 = b->z2 = 0.0f;
    std::fill(ring_.begin(), ring_.end(), 0.0f);
    pos_ = 0;
    phase_ = 0;
    since_ = 0;
    est_ = PitchEstimate{};
}

uint32_t PitchDetector::push(const float* x, uint32_t n) noexcept {
    uint32_t runs = 0;
    const uint32_t mask = w_ - 1;
    for (uint32_t i = 0; i < n; ++i) {
        float v = hp_.run(x[i]);
        if (decim_ > 1) v = lp2_.run(lp1_.run(v));
        if (++phase_ < decim_) continue;
        phase_ = 0;
        ring_[pos_] = v;
        pos_ = (pos_ + 1) & mask;
        if (++since_ >= hop_) {
            since_ = 0;
            analyse();
            ++runs;
        }
    }
    // Keep the filter states out of the denormal range during silence.
    for (Biquad* b : {&hp_, &lp1_, &lp2_}) {
        if (std::fabs(b->z1) < 1e-20f) b->z1 = 0.0f;
        if (std::fabs(b->z2) < 1e-20f) b->z2 = 0.0f;
    }
    return runs;
}

void PitchDetector::analyse() noexcept {
    const uint32_t w = w_, mask = w_ - 1;
    float* const f = frame_.data();
    double energy = 0.0;
    for (uint32_t j = 0; j < w; ++j) {
        const float v = ring_[(pos_ + j) & mask]; // oldest first
        f[j] = v;
        energy += static_cast<double>(v * v);
    }
    std::fill(f + w, f + 2 * w, 0.0f);
    est_.level_db = energy > 1e-20 ? static_cast<float>(10.0 * std::log10(energy / w)) : -120.0f;
    const bool was_voiced = est_.voiced;
    est_.voiced = false;
    est_.clarity = 0.0f;
    if (est_.level_db < kMinLevelDb) {
        est_.hz = 0.0f;
        return;
    }
    // Autocorrelation through the power spectrum.
    fft_.forward(f, re_.data(), im_.data());
    for (uint32_t k = 0; k <= w; ++k) {
        re_[k] = re_[k] * re_[k] + im_[k] * im_[k];
        im_[k] = 0.0f;
    }
    fft_.inverse(re_.data(), im_.data(), acf_.data());
    // nsdf with the running energy m(t).
    double m = 2.0 * energy;
    float* const nsdf = nsdf_.data();
    nsdf[0] = 1.0f;
    for (uint32_t t = 1; t <= tau_max_; ++t) {
        m -= static_cast<double>(f[t - 1] * f[t - 1] + f[w - t] * f[w - t]);
        nsdf[t] = m > 1e-12 ? static_cast<float>(2.0 * static_cast<double>(acf_[t]) / m) : 0.0f;
    }
    // Key maxima: the highest point of each positive lobe after the first
    // negative-going zero crossing.
    uint32_t t = 1;
    while (t <= tau_max_ && nsdf[t] > 0.0f) ++t;
    float best = 0.0f;
    uint32_t cand[64];
    uint32_t nc = 0;
    while (t <= tau_max_) {
        while (t <= tau_max_ && nsdf[t] <= 0.0f) ++t;
        uint32_t peak = 0;
        float pv = -1.0f;
        while (t <= tau_max_ && nsdf[t] > 0.0f) {
            if (nsdf[t] > pv) {
                pv = nsdf[t];
                peak = t;
            }
            ++t;
        }
        if (peak >= tau_min_ && peak < tau_max_ && nc < 64) {
            cand[nc++] = peak;
            best = std::max(best, pv);
        }
    }
    if (nc == 0 || best <= 0.0f) {
        est_.hz = 0.0f;
        return;
    }
    const float thresh = kPeakRatio * best;
    uint32_t p = cand[0];
    for (uint32_t i = 0; i < nc; ++i)
        if (nsdf[cand[i]] >= thresh) {
            p = cand[i];
            break;
        }
    // Parabolic refinement.
    const float a = nsdf[p - 1], b = nsdf[p], c = nsdf[p + 1];
    const float den = a - 2.0f * b + c;
    float delta = 0.0f, height = b;
    if (std::fabs(den) > 1e-9f) {
        delta = std::clamp(0.5f * (a - c) / den, -0.5f, 0.5f);
        height = b - 0.25f * (a - c) * delta;
    }
    const float period = static_cast<float>(p) + delta;
    est_.clarity = std::clamp(height, 0.0f, 1.0f);
    est_.hz = static_cast<float>(rate_ / static_cast<double>(period));
    est_.voiced = est_.clarity >= (was_voiced ? kHoldClarity : kVoicedClarity);
    if (!est_.voiced) est_.hz = 0.0f;
}

} // namespace saine::dsp
