// Phase-vocoder pitch shifter (mono), shared by the Pitch Shifter device and
// the Reverb's Shimmer mode.
//
// Analysis / resynthesis with a Hann window at 4x overlap. Spectral peaks
// are moved to their new frequency together with their region of influence
// (Laroche & Dolson "shifting regions" with identity phase locking), which
// keeps partials coherent and avoids the phasiness of bin-by-bin vocoders.
// With formant preservation on, the spectral envelope (cepstrum, low
// quefrencies) of the input is kept in place: every moved bin is rescaled by
// envelope(new) / envelope(old).
//
// Latency: fft_size - hop samples. prepare() allocates; reset() and
// process() are RT safe.
#pragma once

#include "fft.hpp"

#include <cstdint>
#include <vector>

namespace saine::dsp {

class PitchShifter {
public:
    // fft_size: power of two (1024..8192).
    void prepare(uint32_t fft_size);
    void reset() noexcept;
    // Pitch ratio (2 = one octave up), clamped to 0.25..4.
    void set_ratio(float ratio) noexcept;
    void set_formant(bool on) noexcept { formant_ = on; }
    // Mono, in-place allowed (in == out).
    void process(const float* in, float* out, uint32_t n) noexcept;
    uint32_t latency() const noexcept { return n_ - hop_; }
    uint32_t fft_size() const noexcept { return n_; }

private:
    void frame() noexcept;
    void envelope(const float* mag, float* env) noexcept;

    RealFft fft_;
    uint32_t n_ = 0, hop_ = 0, bins_ = 0, pos_ = 0;
    float ratio_ = 1.0f;
    bool formant_ = false;
    std::vector<float> window_, in_fifo_, out_fifo_, accum_, frame_;
    std::vector<float> re_, im_, mag_, phase_, last_phase_, omega_, out_phase_acc_;
    std::vector<float> out_re_, out_im_, out_mag_, out_ph_, env_in_, cep_, cep_re_, cep_im_;
    std::vector<int32_t> peaks_;
};

} // namespace saine::dsp
