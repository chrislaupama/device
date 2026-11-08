// Small DSP helpers shared by the built-in processors (RT safe, header only).
#pragma once

#include <algorithm>
#include <cmath>
#include <cstdint>

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

} // namespace saine::dsp
