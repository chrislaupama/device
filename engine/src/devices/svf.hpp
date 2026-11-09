// Topology-preserving state-variable filter (trapezoidal integration), used
// by the restoration devices for band-pass detection and notch cuts. Stable
// under per-sample coefficient changes, exact at the tuned frequency up to
// Nyquist. RT safe, header only.
#pragma once

#include <algorithm>
#include <cmath>

namespace saine::dsp {

struct SvfCoeffs {
    float a1 = 1.0f, a2 = 0.0f, a3 = 0.0f, k = 1.0f;
    // hz is clamped below Nyquist; q > 0.
    static SvfCoeffs band(float hz, float q, double sample_rate) noexcept {
        const double f = std::clamp(static_cast<double>(hz), 1.0, 0.49 * sample_rate);
        const auto g = static_cast<float>(std::tan(3.14159265358979323846 * f / sample_rate));
        SvfCoeffs c;
        c.k = 1.0f / std::max(q, 1e-3f);
        c.a1 = 1.0f / (1.0f + g * (g + c.k));
        c.a2 = g * c.a1;
        c.a3 = g * c.a2;
        return c;
    }
};

struct Svf {
    float ic1 = 0.0f, ic2 = 0.0f;
    void reset() noexcept { ic1 = ic2 = 0.0f; }
    // Band-pass with 0 dB gain at the centre (k * v1). x - band(x) is the notch.
    float band(float x, const SvfCoeffs& c) noexcept {
        const float v3 = x - ic2;
        const float v1 = c.a1 * ic1 + c.a2 * v3;
        const float v2 = ic2 + c.a2 * ic1 + c.a3 * v3;
        ic1 = 2.0f * v1 - ic1;
        ic2 = 2.0f * v2 - ic2;
        return c.k * v1;
    }
    // Flush denormal-range state (call once per block).
    void flush() noexcept {
        if (std::fabs(ic1) < 1e-20f) ic1 = 0.0f;
        if (std::fabs(ic2) < 1e-20f) ic2 = 0.0f;
    }
};

} // namespace saine::dsp
