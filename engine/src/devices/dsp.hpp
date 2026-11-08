// Small DSP helpers shared by the built-in processors (RT safe, header only).
#pragma once

#include <algorithm>
#include <cmath>
#include <cstddef>
#include <cstdint>
#include <vector>

namespace saine {
struct RtTransport;
}

namespace saine::dsp {

constexpr float kLog2_10_over_20 = 0.166096404744368f; // dB -> log2(gain)
constexpr float kDbPerLog2 = 6.020599913279624f;        // log2(gain) -> dB
constexpr float kSilenceDb = -120.0f;

inline float db_to_gain(float db) noexcept { return std::exp2(db * kLog2_10_over_20); }
inline float gain_to_db(float g) noexcept { return g > 1e-6f ? kDbPerLog2 * std::log2(g) : kSilenceDb; }
// Mean square (power) -> dB.
inline float power_to_db(float p) noexcept { return p > 1e-12f ? 0.5f * kDbPerLog2 * std::log2(p) : kSilenceDb; }

// One-pole coefficient for a time constant in milliseconds: y += (1 - a) * (x - y).
inline float pole(float ms, double sample_rate) noexcept {
    return ms <= 0.0f ? 0.0f : static_cast<float>(std::exp(-1000.0 / (static_cast<double>(ms) * sample_rate)));
}

// Per-sample parameter smoother (one pole, ~ms time constant).
struct Smoother {
    float y = 0.0f;
    float a = 0.0f;
    void set_time(float ms, double sample_rate) noexcept { a = pole(ms, sample_rate); }
    void snap(float v) noexcept { y = v; }
    float next(float target) noexcept { return y = target + a * (y - target); }
    bool settled(float target) const noexcept { return std::fabs(y - target) <= 1e-6f * std::max(1.0f, std::fabs(target)); }
};

// Peak of max(|l|, |r|) over n samples.
inline float stereo_peak(const float* l, const float* r, uint32_t n) noexcept {
    float m = 0.0f;
    for (uint32_t i = 0; i < n; ++i) m = std::max(m, std::max(std::fabs(l[i]), std::fabs(r[i])));
    return m;
}

constexpr float kPi = 3.14159265358979323846f;
constexpr float kTwoPi = 6.28318530717958647692f;

// Fractional part in [0, 1) (also for negative inputs).
inline float wrap01(float x) noexcept { return x - std::floor(x); }
inline double wrap01(double x) noexcept { return x - std::floor(x); }

// sin(2 pi p) for a phase in cycles: quarter-wave folding and a 9th-order
// odd polynomial (max error 4e-6), branch-light so loops vectorise.
inline float sin2pi(float p) noexcept {
    float t = p - std::floor(p) - 0.5f;              // [-0.5, 0.5): sin(2 pi p) = -sin(2 pi t)
    t = t > 0.25f ? 0.5f - t : (t < -0.25f ? -0.5f - t : t);
    const float z = kTwoPi * t, z2 = z * z;
    const float s = z * (1.0f + z2 * (-1.0f / 6.0f + z2 * (1.0f / 120.0f + z2 * (-1.0f / 5040.0f + z2 * (1.0f / 362880.0f)))));
    return -s;
}
inline float cos2pi(float p) noexcept { return sin2pi(p + 0.25f); }

// Unit LFO shapes over a phase in cycles, -1..1.
inline float tri(float p) noexcept { return 1.0f - 4.0f * std::fabs(wrap01(p + 0.25f) - 0.5f); } // 0 at p = 0, rising

// Note values of a synced LFO in quarter notes: the reference's LFO_DIVISION
// table (8/1 ... 1/32, then 1/64., 1/64, 1/64T) in parameter index order.
inline constexpr float kDivisionQuarters[] = {32.0f, 16.0f, 8.0f, 4.0f, 3.0f, 2.0f, 4.0f / 3.0f, 1.5f, 1.0f, 2.0f / 3.0f,
                                              0.75f, 0.5f, 1.0f / 3.0f, 0.375f, 0.25f, 1.0f / 6.0f, 0.125f, 0.09375f, 0.0625f, 1.0f / 24.0f};
// Character Mod's MOD_NOTE table (2/1. ... 1/32T) in quarter notes.
inline constexpr float kModNoteQuarters[] = {12.0f, 8.0f, 16.0f / 3.0f, 6.0f, 4.0f, 8.0f / 3.0f, 3.0f, 2.0f, 4.0f / 3.0f, 1.5f, 1.0f,
                                             2.0f / 3.0f, 0.75f, 0.5f, 1.0f / 3.0f, 0.375f, 0.25f, 1.0f / 6.0f, 0.1875f, 0.125f, 1.0f / 12.0f};

template <std::size_t N>
inline float table_at(const float (&t)[N], float index) noexcept {
    const long i = std::lround(index);
    return t[static_cast<std::size_t>(std::clamp<long>(i, 0, static_cast<long>(N) - 1))];
}

// LFO phase clock shared by the modulation devices. Free: the phase runs at
// `hz`. Synced: the cycle is `quarters` quarter notes long; while the
// transport plays the phase is locked to the song position (so the LFO
// lands on the beat after seeks and loops), otherwise it free-runs at the
// synced rate. Phase is in cycles [0, 1).
struct LfoClock {
    double phase = 0.0;
    double inc = 0.0; // cycles per sample for this segment

    // Call at the start of every process() segment. `offset` = frames from
    // the transport's position to the segment start.
    void begin(float hz, bool synced, float quarters, const RtTransport* tr, uint32_t offset, double sample_rate) noexcept;
    float hz(double sample_rate) const noexcept { return static_cast<float>(inc * sample_rate); }
    // Phase of the current sample, then advance.
    float tick() noexcept {
        const float p = static_cast<float>(phase);
        phase += inc;
        if (phase >= 1.0) phase -= std::floor(phase);
        return p;
    }
    void reset() noexcept { phase = 0.0; }
};

// Power-of-two circular delay line with 4-point Hermite interpolation. The
// buffer is sized in prepare(); reads/writes are RT safe.
class DelayLine {
public:
    void prepare(std::size_t min_len) {
        std::size_t n = 16;
        while (n < min_len + 4) n <<= 1;
        buf_.assign(n, 0.0f);
        mask_ = n - 1;
        w_ = 0;
    }
    void reset() noexcept { std::fill(buf_.begin(), buf_.end(), 0.0f); }
    void push(float x) noexcept {
        w_ = (w_ + 1) & mask_;
        buf_[w_] = x;
    }
    // Sample `d` samples back from the newest (d >= 0; clamped to the line).
    float read(float d) const noexcept {
        d = std::clamp(d, 1.0f, static_cast<float>(mask_ - 3));
        const auto di = static_cast<std::size_t>(d);
        const float f = d - static_cast<float>(di);
        const std::size_t i0 = (w_ - di) & mask_; // newer
        const float xm1 = buf_[(i0 + 1) & mask_], x0 = buf_[i0], x1 = buf_[(i0 - 1) & mask_], x2 = buf_[(i0 - 2) & mask_];
        const float c1 = 0.5f * (x1 - xm1);
        const float c2 = xm1 - 2.5f * x0 + 2.0f * x1 - 0.5f * x2;
        const float c3 = 0.5f * (x2 - xm1) + 1.5f * (x0 - x1);
        return ((c3 * f + c2) * f + c1) * f + x0;
    }
    float at(std::size_t d) const noexcept { return buf_[(w_ - d) & mask_]; }
    std::size_t size() const noexcept { return buf_.size(); }

private:
    std::vector<float> buf_;
    std::size_t mask_ = 0;
    std::size_t w_ = 0;
};

// Topology-preserving one-pole filter (low-pass output; high-pass = x - lp).
struct OnePole {
    float g = 0.0f, s = 0.0f;
    void set(float hz, double sample_rate) noexcept {
        const float w = std::tan(kPi * std::min(hz, 0.49f * static_cast<float>(sample_rate)) / static_cast<float>(sample_rate));
        g = w / (1.0f + w);
    }
    float lp(float x) noexcept {
        const float v = g * (x - s);
        const float y = v + s;
        s = y + v;
        return y;
    }
    float hp(float x) noexcept { return x - lp(x); }
    void reset() noexcept { s = 0.0f; }
};

// Small fast PRNG (xorshift32), uniform in [-1, 1).
struct Rng {
    uint32_t s = 0x9e3779b9u;
    void seed(uint32_t v) noexcept { s = v != 0 ? v * 2654435761u + 1u : 0x9e3779b9u; }
    float next() noexcept {
        s ^= s << 13;
        s ^= s >> 17;
        s ^= s << 5;
        return static_cast<float>(static_cast<int32_t>(s)) * (1.0f / 2147483648.0f);
    }
};

// Smooth random wander (cosine interpolated random targets at `hz`), -1..1.
struct Wander {
    Rng rng;
    float a = 0.0f, b = 0.0f, t = 0.0f;
    void seed(uint32_t v) noexcept {
        rng.seed(v);
        a = rng.next();
        b = rng.next();
        t = 0.0f;
    }
    float tick(float inc) noexcept {
        t += inc;
        if (t >= 1.0f) {
            t -= std::floor(t);
            a = b;
            b = rng.next();
        }
        const float w = 0.5f - 0.5f * cos2pi(0.5f * t);
        return a + (b - a) * w;
    }
};

// Equal-power style dry / wet gains of the reference's mix law with a
// correlation term: d = 1 - m, n = sqrt(d^2 + m^2 + 2 m d c).
inline void mix_gains(float mix, float correlation, float& dry, float& wet) noexcept {
    const float m = std::clamp(mix, 0.0f, 1.0f), d = 1.0f - m;
    const float n = std::sqrt(std::max(1e-9f, d * d + m * m + 2.0f * m * d * correlation));
    dry = d / n;
    wet = m / n;
}

// 2x oversampler: linear-phase half-band FIR up and down (Kaiser windowed
// sinc). Latency = (taps - 1) / 2 base-rate samples for the round trip.
class Oversampler2x {
public:
    static constexpr int kTaps = 47; // 4k + 3: odd taps away from the centre are zero
    static constexpr uint32_t kLatency = (kTaps - 1) / 2;
    Oversampler2x();
    void reset() noexcept;
    // One base-rate sample in, two 2x samples out.
    void up(float x, float& y0, float& y1) noexcept;
    // Two 2x samples in, one base-rate sample out.
    float down(float y0, float y1) noexcept;

private:
    float h_[kTaps]{};
    float up_hist_[kTaps]{};   // zero-stuffed input history (circular, doubled)
    float dn_hist_[2 * kTaps]{};
    int up_pos_ = 0, dn_pos_ = 0;
};

} // namespace saine::dsp
