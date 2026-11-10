// Shared monophonic pitch detector for the built-ins (Pitch Corrector,
// Tuner; docs/devices.md). prepare() allocates; reset() and push() are RT
// safe.
//
// Method (McLeod's normalised square difference function, computed with the
// FFT):
//   1. the input is high-passed (DC and rumble), low-passed and decimated
//      to an analysis rate of 20..32 kHz (pitch content stays well below);
//   2. every hop, the last W analysis samples (W covers two periods of the
//      lowest pitch) are autocorrelated through a zero-padded real FFT:
//      r(t) = IFFT(|FFT(x)|^2), and normalised by the running energy
//      m(t) = sum x[j]^2 + x[j+t]^2, giving nsdf(t) = 2 r(t) / m(t) in -1..1;
//   3. the first "key maximum" (highest point of each positive lobe) above
//      kPeakRatio of the largest one is the period, refined by parabolic
//      interpolation; its height is the clarity (1 = perfectly periodic);
//   4. the frame is voiced when clarity and level pass their thresholds
//      (with hysteresis, so a held note does not flicker).
#pragma once

#include "spectral.hpp"

#include <cstdint>
#include <vector>

namespace dstudio::dsp {

struct PitchEstimate {
    float hz = 0.0f;       // fundamental (0 when unvoiced)
    float clarity = 0.0f;  // nsdf peak height, 0..1
    float level_db = -120.0f; // RMS of the analysis window
    bool voiced = false;
};

class PitchDetector {
public:
    // min_hz / max_hz: search range. hop_ms: analysis interval. Non-RT.
    void prepare(double sample_rate, float min_hz, float max_hz, float hop_ms);
    void reset() noexcept;
    // Feeds mono input. Returns the number of analyses that ran (0 when no
    // hop completed in this call); estimate() holds the newest.
    uint32_t push(const float* x, uint32_t n) noexcept;
    const PitchEstimate& estimate() const noexcept { return est_; }
    // Input frames from the newest sample back to the centre of the analysis
    // window: how far an estimate lags the audio.
    uint32_t lag() const noexcept { return lag_; }
    // Input frames between analyses.
    uint32_t hop() const noexcept { return hop_ * decim_; }

    // Thresholds (public for tests and tuning).
    static constexpr float kPeakRatio = 0.9f;
    static constexpr float kVoicedClarity = 0.80f;
    static constexpr float kHoldClarity = 0.62f;
    static constexpr float kMinLevelDb = -60.0f;

private:
    void analyse() noexcept;

    struct Biquad {
        float b0 = 1, b1 = 0, b2 = 0, a1 = 0, a2 = 0, z1 = 0, z2 = 0;
        float run(float x) noexcept {
            const float y = b0 * x + z1;
            z1 = b1 * x - a1 * y + z2;
            z2 = b2 * x - a2 * y;
            return y;
        }
    };

    double rate_ = 48000.0; // analysis rate
    uint32_t decim_ = 1, phase_ = 0;
    uint32_t w_ = 1024, hop_ = 128, since_ = 0, lag_ = 0;
    uint32_t tau_min_ = 2, tau_max_ = 512;
    Biquad hp_, lp1_, lp2_;
    std::vector<float> ring_; // w_ analysis samples (power of two)
    uint32_t pos_ = 0;
    RealFft fft_;
    std::vector<float> frame_, re_, im_, acf_, nsdf_;
    PitchEstimate est_;
};

} // namespace dstudio::dsp
