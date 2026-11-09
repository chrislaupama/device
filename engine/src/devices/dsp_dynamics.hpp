// DSP building blocks shared by the dynamics devices (limiter, gate, glue
// compressor, transient shaper, mastering limiter). Header only. prepare()
// methods may allocate; everything else is real-time safe.
#pragma once

#include "dsp.hpp"

#include <algorithm>
#include <array>
#include <cmath>
#include <cstdint>
#include <cstring>
#include <vector>

namespace saine::dsp {

constexpr double kPiD = 3.14159265358979323846;

// fast_log2 / fast_exp2 / fast_tanh come from dsp.hpp.
inline float fast_db_to_gain(float db) noexcept { return fast_exp2(db * kLog2_10_over_20); }
inline float fast_gain_to_db(float g) noexcept { return g > 1e-6f ? kDbPerLog2 * fast_log2(g) : kSilenceDb; }

// Milliseconds -> samples (fractional).
inline double ms_to_samples(float ms, double rate) noexcept { return static_cast<double>(ms) * 1e-3 * rate; }

// Smallest power of two >= n.
inline uint32_t pow2_at_least(uint32_t n) noexcept {
    uint32_t p = 1;
    while (p < n) p <<= 1;
    return p;
}

// Second-order section, transposed direct form II.
struct Biquad {
    float b0 = 1.0f, b1 = 0.0f, b2 = 0.0f, a1 = 0.0f, a2 = 0.0f;
    float z1 = 0.0f, z2 = 0.0f;

    void reset() noexcept { z1 = z2 = 0.0f; }
    void pass() noexcept {
        b0 = 1.0f;
        b1 = b2 = a1 = a2 = 0.0f;
    }
    // Butterworth high-pass (RBJ cookbook form).
    void highpass(double hz, double rate, double q = 0.70710678) noexcept {
        const double w = 2.0 * kPiD * std::clamp(hz, 1.0, 0.45 * rate) / rate;
        const double c = std::cos(w), a = std::sin(w) / (2.0 * q), n = 1.0 / (1.0 + a);
        b0 = static_cast<float>((1.0 + c) * 0.5 * n);
        b1 = static_cast<float>(-(1.0 + c) * n);
        b2 = b0;
        a1 = static_cast<float>(-2.0 * c * n);
        a2 = static_cast<float>((1.0 - a) * n);
    }
    float tick(float x) noexcept {
        const float y = b0 * x + z1;
        z1 = b1 * x - a1 * y + z2;
        z2 = b2 * x - a2 * y;
        return y;
    }
    void flush_denormals() noexcept {
        if (std::fabs(z1) < 1e-20f) z1 = 0.0f;
        if (std::fabs(z2) < 1e-20f) z2 = 0.0f;
    }
};

// Delay line with a fixed capacity (power of two); delays up to the
// prepared maximum.
class FixedDelay {
public:
    void prepare(uint32_t max_delay) {
        buf_.assign(pow2_at_least(max_delay + 1), 0.0f);
        mask_ = static_cast<uint32_t>(buf_.size()) - 1;
        w_ = 0;
    }
    void reset() noexcept { std::fill(buf_.begin(), buf_.end(), 0.0f); }
    // Writes x and returns the sample written `delay` calls ago (0 = x).
    float process(float x, uint32_t delay) noexcept {
        buf_[w_] = x;
        const float y = buf_[(w_ - std::min(delay, mask_)) & mask_];
        w_ = (w_ + 1) & mask_;
        return y;
    }

private:
    std::vector<float> buf_{0.0f};
    uint32_t mask_ = 0, w_ = 0;
};

// Minimum over the last `window` pushed values (monotonic queue, O(1)
// amortised). The window may change between calls; capacity is fixed.
class SlidingMin {
public:
    void prepare(uint32_t max_window) {
        const uint32_t n = pow2_at_least(max_window + 1);
        val_.assign(n, 0.0f);
        at_.assign(n, 0);
        mask_ = n - 1;
        reset();
    }
    void reset() noexcept {
        head_ = tail_ = 0;
        t_ = 0;
    }
    float push(float x, uint32_t window) noexcept {
        while (tail_ != head_ && val_[(tail_ - 1) & mask_] >= x) --tail_;
        val_[tail_ & mask_] = x;
        at_[tail_ & mask_] = t_;
        ++tail_;
        const uint64_t w = std::clamp<uint32_t>(window, 1, mask_);
        while (t_ - at_[head_ & mask_] >= w) ++head_;
        ++t_;
        return val_[head_ & mask_];
    }

private:
    std::vector<float> val_{0.0f};
    std::vector<uint64_t> at_{0};
    uint32_t mask_ = 0;
    uint64_t head_ = 0, tail_ = 0, t_ = 0;
};

// Mean of the last `window` values (box filter); the window may change.
class MovingAverage {
public:
    void prepare(uint32_t max_window, float fill) {
        buf_.assign(pow2_at_least(max_window + 1), fill);
        mask_ = static_cast<uint32_t>(buf_.size()) - 1;
        fill_ = fill;
        reset();
    }
    void reset() noexcept {
        std::fill(buf_.begin(), buf_.end(), fill_);
        w_ = 0;
        window_ = 1;
        sum_ = static_cast<double>(fill_);
    }
    float push(float x, uint32_t window) noexcept {
        window = std::clamp<uint32_t>(window, 1, mask_);
        buf_[w_] = x;
        if (window != window_) { // re-sum the new window (rare: lookahead changes)
            window_ = window;
            sum_ = 0.0;
            for (uint32_t i = 0; i < window; ++i) sum_ += static_cast<double>(buf_[(w_ - i) & mask_]);
        } else {
            sum_ += static_cast<double>(x) - static_cast<double>(buf_[(w_ - window) & mask_]);
        }
        w_ = (w_ + 1) & mask_;
        return static_cast<float>(sum_ / window);
    }

private:
    std::vector<float> buf_{0.0f};
    uint32_t mask_ = 0, w_ = 0, window_ = 1;
    float fill_ = 0.0f;
    double sum_ = 0.0;
};

// Gain computer of a lookahead brick-wall stage, in log2 gain units (<= 0):
// hold the lowest need over the window, release exponentially towards 0,
// then fade with a box filter over the same window. The returned gain
// reaches each need exactly when that sample is `window - 1` pushes old,
// so delay the audio by window - 1 (plus any detector delay).
class LookaheadGain {
public:
    void prepare(uint32_t max_window) {
        hold_.prepare(max_window);
        box_.prepare(max_window, 0.0f);
        reset();
    }
    void reset() noexcept {
        hold_.reset();
        box_.reset();
        follow_ = 0.0f;
    }
    float push(float need, uint32_t window, float release_pole) noexcept {
        follow_ = std::min(hold_.push(need, window), release_pole * follow_);
        if (follow_ > -1e-9f) follow_ = 0.0f;
        return box_.push(follow_, window);
    }

private:
    SlidingMin hold_;
    MovingAverage box_;
    float follow_ = 0.0f;
};

// Kaiser window value at x in -1..1 (position / half length).
inline double kaiser(double x, double beta) noexcept {
    auto i0 = [](double v) {
        double s = 1.0, t = 1.0;
        for (int k = 1; k < 40; ++k) {
            t *= (v / (2.0 * k)) * (v / (2.0 * k));
            s += t;
        }
        return s;
    };
    return i0(beta * std::sqrt(std::max(0.0, 1.0 - x * x))) / i0(beta);
}

// Inter-sample peak estimate: 4x polyphase interpolation (48-tap
// Kaiser-windowed sinc), max |value| over the four phases. The estimate lags
// the input by kDelay samples.
class TruePeak4x {
public:
    static constexpr int kTaps = 12; // per phase
    static constexpr uint32_t kDelay = 6;

    TruePeak4x() noexcept {
        constexpr int n = 4 * kTaps;
        const double c = (n - 1) * 0.5;
        for (int i = 0; i < n; ++i) {
            const double t = (i - c) / 4.0;
            const double s = std::sin(kPiD * t) / (kPiD * t); // t is never 0 (c is fractional)
            h_[i % 4][i / 4] = static_cast<float>(s * kaiser((i - c) / (c + 1.0), 7.0));
        }
        for (auto& p : h_) { // unity DC gain per phase
            float sum = 0.0f;
            for (float v : p) sum += v;
            for (float& v : p) v /= sum;
        }
        reset();
    }
    void reset() noexcept {
        x_.fill(0.0f);
        w_ = 0;
    }
    float next(float x) noexcept {
        x_[w_] = x;
        x_[w_ + kTaps] = x; // mirrored history: a contiguous window, no wrap in the loop
        w_ = (w_ + 1) % kTaps;
        const float* hist = &x_[w_]; // oldest .. newest
        float m = 0.0f;
        for (const auto& p : h_) {
            float acc = 0.0f;
            for (int k = 0; k < kTaps; ++k) acc += p[kTaps - 1 - k] * hist[k];
            m = std::max(m, std::fabs(acc));
        }
        return m;
    }

private:
    std::array<std::array<float, kTaps>, 4> h_{};
    std::array<float, 2 * kTaps> x_{};
    int w_ = 0;
};

// One 2x stage of a linear-phase half-band FIR oversampler of length
// 2M + 1 (M even, only odd taps and the centre are non-zero). up() turns one
// sample into two (odd phase first), down() two into one. Up + down delay
// the signal by exactly M low-rate samples.
template <int M>
class HalfbandStage {
    static_assert(M % 2 == 0, "M must be even for an integral delay");

public:
    static constexpr uint32_t kRoundTrip = M; // low-rate samples

    explicit HalfbandStage(double beta) noexcept {
        double c[M];
        double sum = 0.5;
        for (int j = 0; j < M; ++j) {
            const int n = 2 * j - (M - 1); // odd offsets -(M-1) .. M-1
            c[j] = std::sin(kPiD * n / 2.0) / (kPiD * n) * kaiser(static_cast<double>(n) / (M + 1), beta);
            sum += c[j];
        }
        for (int j = 0; j < M; ++j) odd_[j] = static_cast<float>(c[j] / sum);
        centre_ = static_cast<float>(0.5 / sum);
        reset();
    }
    void reset() noexcept {
        up_.fill(0.0f);
        dn_odd_.fill(0.0f);
        dn_even_.fill(0.0f);
        uw_ = dw_ = ew_ = 0;
    }
    // x[m] -> v[2m-1], v[2m] (unity passband gain).
    void up(float x, float* out) noexcept {
        constexpr int L = M + 1;
        up_[uw_] = x;
        up_[uw_ + L] = x;
        uw_ = (uw_ + 1) % L;
        const float* w = &up_[uw_]; // w[L-1] = x[m]
        float acc = 0.0f;
        for (int j = 0; j < M; ++j) acc += odd_[j] * w[L - 2 - j];
        out[0] = 2.0f * acc;
        out[1] = 2.0f * centre_ * w[L - 1 - M / 2];
    }
    // v[2m-1], v[2m] -> y[m].
    float down(const float* in) noexcept {
        constexpr int E = M / 2 + 1;
        dn_odd_[dw_] = in[0];
        dn_odd_[dw_ + M] = in[0];
        dw_ = (dw_ + 1) % M;
        dn_even_[ew_] = in[1];
        ew_ = (ew_ + 1) % E;
        const float* w = &dn_odd_[dw_];
        float acc = 0.0f;
        for (int j = 0; j < M; ++j) acc += odd_[j] * w[M - 1 - j];
        return acc + centre_ * dn_even_[ew_]; // oldest even sample: v[2(m - M/2)]
    }

private:
    std::array<float, M> odd_{};
    float centre_ = 0.5f;
    std::array<float, 2 * (M + 1)> up_{};
    std::array<float, 2 * M> dn_odd_{};
    std::array<float, M / 2 + 1> dn_even_{};
    int uw_ = 0, dw_ = 0, ew_ = 0;
};

// Mono 1x / 2x / 4x / 8x oversampler: a long first stage (65 taps, images
// below -100 dB), shorter later stages (33 taps) that only reject images far
// above the audio band.
class Oversampler {
    using S1 = HalfbandStage<32>;
    using S2 = HalfbandStage<16>;

public:
    static constexpr uint32_t kMaxFactor = 8;
    // Up + down delay in low-rate samples for factor 1, 2, 4, 8.
    static constexpr uint32_t latency(uint32_t factor) noexcept {
        return factor >= 8 ? S1::kRoundTrip + S2::kRoundTrip / 2 + S2::kRoundTrip / 4
             : factor >= 4 ? S1::kRoundTrip + S2::kRoundTrip / 2
             : factor >= 2 ? S1::kRoundTrip
                           : 0;
    }
    void reset() noexcept {
        s1_.reset();
        s2_.reset();
        s3_.reset();
    }
    // x -> `factor` samples in out.
    void up(float x, float* out, uint32_t factor) noexcept {
        if (factor <= 1) {
            out[0] = x;
            return;
        }
        float a[2];
        s1_.up(x, a);
        if (factor == 2) {
            out[0] = a[0];
            out[1] = a[1];
            return;
        }
        float b[4];
        s2_.up(a[0], b);
        s2_.up(a[1], b + 2);
        if (factor == 4) {
            std::copy(b, b + 4, out);
            return;
        }
        for (int i = 0; i < 4; ++i) s3_.up(b[i], out + 2 * i);
    }
    float down(const float* in, uint32_t factor) noexcept {
        if (factor <= 1) return in[0];
        float a[2];
        if (factor == 2) {
            a[0] = in[0];
            a[1] = in[1];
        } else {
            float b[4];
            if (factor == 4) {
                std::copy(in, in + 4, b);
            } else {
                for (int i = 0; i < 4; ++i) b[i] = s3_.down(in + 2 * i);
            }
            a[0] = s2_.down(b);
            a[1] = s2_.down(b + 2);
        }
        return s1_.down(a);
    }

private:
    S1 s1_{10.0};
    S2 s2_{9.0};
    S2 s3_{9.0};
};

// Soft clipper: linear up to `knee`, then a tanh bend that approaches
// `ceiling` (0 < knee < ceiling). Run it oversampled.
inline float soft_clip(float x, float knee, float ceiling) noexcept {
    const float a = std::fabs(x);
    if (a <= knee) return x;
    const float s = ceiling - knee;
    return std::copysign(knee + s * fast_tanh((a - knee) / s), x);
}

// Peak value held over `hold` samples, for readouts the UI polls slower
// than the block rate (a hit between two polls is not lost).
struct HeldPeak {
    float cur = 0.0f, shown = 0.0f;
    uint32_t count = 0;
    // Feed one block's value; returns what to publish.
    float update(float v, uint32_t frames, uint32_t hold) noexcept {
        cur = std::max(cur, v);
        count += frames;
        if (count >= hold) {
            shown = cur;
            cur = 0.0f;
            count = 0;
            return shown;
        }
        return std::max(shown, cur);
    }
    void reset() noexcept { cur = shown = 0.0f, count = 0; }
};

// Like HeldPeak for a signed value: holds the largest magnitude with its sign.
struct HeldSigned {
    float cur = 0.0f, shown = 0.0f;
    uint32_t count = 0;
    float update(float v, uint32_t frames, uint32_t hold) noexcept {
        if (std::fabs(v) > std::fabs(cur)) cur = v;
        count += frames;
        if (count >= hold) {
            shown = cur;
            cur = 0.0f;
            count = 0;
            return shown;
        }
        return std::fabs(shown) >= std::fabs(cur) ? shown : cur;
    }
    void reset() noexcept { cur = shown = 0.0f, count = 0; }
};

} // namespace saine::dsp
