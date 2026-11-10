// Bit Crusher: sample-and-hold downsampling and amplitude quantisation, with
// TPDF dither, clock jitter and a dry/wet mix. Parameters in the order of
// dstudio_core::builtin::bitcrusher (checked by a Rust test).
//
//   hold      a new input sample is taken every h = Downsample frames
//             (fractional h: a phase counter; Jitter varies each hold length
//             by up to +-jitter * h / 2, at least one frame); both channels
//             share the clock
//   quantise  levels = 2^(bits - 1) per side: round((x + dither) * levels) / levels,
//             dither = triangular noise of +-1 step scaled by Dither
//   out       dry * (1 - mix) + crushed * mix
// No anti-aliasing on purpose (the aliasing is the sound), no latency. The
// model matches the panel's picture (dstudio_core crush_picture).
#include "builtin.hpp"
#include "dsp.hpp"

#include <algorithm>
#include <cmath>

namespace dstudio {

namespace {

enum Param : uint32_t { kBits, kDownsample, kMix, kDither, kJitter, kParamCount };

constexpr const char* kParamIds[kParamCount] = {"bitdepth", "downsample", "mix", "dither", "jitter"};

class BitCrusher final : public BuiltinProcessor {
public:
    void prepare(double sample_rate, uint32_t) override {
        sr_ = sample_rate;
        bits_.set_time(10.0f, sr_);
        mix_.set_time(20.0f, sr_);
        dither_.set_time(20.0f, sr_);
    }

    void reset() noexcept override {
        held_[0] = held_[1] = 0.0f;
        count_ = 1e9f; // take a sample on the first frame
        hold_ = 1.0f;
        seed_ = 1;
        primed_ = false;
    }

    void process(float* l, float* r, uint32_t n, const float* p, const BuiltinContext&, float*) noexcept override {
        const float bits = std::clamp(p[kBits], 1.0f, 32.0f);
        const float h = std::clamp(p[kDownsample], 1.0f, 100.0f);
        const float mix = std::clamp(p[kMix], 0.0f, 100.0f) * 0.01f;
        const float dither = std::clamp(p[kDither], 0.0f, 100.0f) * 0.01f;
        const float jitter = std::clamp(p[kJitter], 0.0f, 100.0f) * 0.01f;
        if (!primed_) {
            bits_.snap(bits);
            mix_.snap(mix);
            dither_.snap(dither);
            primed_ = true;
        }
        float count = count_, hold = hold_;
        for (uint32_t i = 0; i < n; ++i) {
            const float b = bits_.next(bits);
            const float levels = dsp::fast_exp2(b - 1.0f);
            const float inv = 1.0f / levels;
            const float dth = dither_.next(dither) * inv;
            const float mx = mix_.next(mix);
            count += 1.0f;
            if (count >= hold) {
                count = count - hold < 1.0f ? count - hold : 0.0f; // keep the fraction (the first frame starts at 0)
                hold = std::max(1.0f, h + rand() * jitter * h * 0.5f);
                const float dl = 0.5f * (rand() + rand()) * dth;
                const float dr = 0.5f * (rand() + rand()) * dth;
                held_[0] = std::floor((l[i] + dl) * levels + 0.5f) * inv;
                held_[1] = std::floor((r[i] + dr) * levels + 0.5f) * inv;
            }
            l[i] += mx * (held_[0] - l[i]);
            r[i] += mx * (held_[1] - r[i]);
        }
        count_ = count;
        hold_ = hold;
    }

private:
    // -1..1, linear congruential.
    float rand() noexcept {
        seed_ = seed_ * 1664525u + 1013904223u;
        return static_cast<float>(seed_ >> 8) * (1.0f / 8388608.0f) - 1.0f;
    }

    double sr_ = 48000.0;
    dsp::Smoother bits_, mix_, dither_;
    float held_[2] = {0.0f, 0.0f};
    float count_ = 1e9f, hold_ = 1.0f;
    uint32_t seed_ = 1;
    bool primed_ = false;
};

std::unique_ptr<BuiltinProcessor> create_bitcrusher() { return std::make_unique<BitCrusher>(); }

} // namespace

extern const BuiltinDescriptor kBitCrusherDescriptor;
constexpr float kDefaults[] = {16.f, 1.f, 100.f, 0.f, 0.f};
const BuiltinDescriptor kBitCrusherDescriptor{"bitcrusher", kParamIds, {}, &create_bitcrusher, {}, kDefaults};

} // namespace dstudio
