// Offline (import-time) sample-rate conversion. Non-RT only.
//
// Band-limited interpolation with a Kaiser-windowed sinc kernel (beta 9.0,
// ~-90 dB stopband) tabulated at 512 phases per input sample and linearly
// interpolated between phases. 32 zero crossings each side of the kernel
// centre; when downsampling, the kernel is stretched so its cutoff sits at
// 0.94 x the *output* Nyquist (anti-aliasing). Passband edge is 0.94 x the
// lower Nyquist, so content above ~20.7 kHz at 44.1 kHz is attenuated.
//
// Output length is llround(input_frames * dst_rate / src_rate). Output frame
// j corresponds to input position j * src_rate / dst_rate (time aligned, no
// added latency; the kernel is symmetric and centred).
#pragma once

#include <cstddef>
#include <cstdint>
#include <vector>

namespace saine {

uint64_t resampled_length(uint64_t input_frames, double src_rate, double dst_rate) noexcept;

class SincResampler {
public:
    SincResampler(double src_rate, double dst_rate);

    // Resample one channel. `out` is resized to resampled_length().
    void process(const float* in, uint64_t in_frames, std::vector<float>& out) const;

private:
    double src_rate_;
    double dst_rate_;
    double cutoff_;     // normalised to the input rate (cycles/sample), <= 0.5
    double half_width_; // kernel half width in input samples
    std::vector<float> table_; // kernel sampled at x = k / kPhases, k = 0..half_width*kPhases+1
};

} // namespace saine
