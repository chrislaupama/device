// Auto-Panner: an LFO moves the signal between the speakers. Parameters in
// the order of dstudio_core::builtin::autopanner (checked by a Rust test).
//
// Per sample:
//   v      LFO position -1 (left) .. +1 (right): Sine and Triangle start in
//          the centre and move left first, Square holds left for the first
//          half cycle and right for the second, Random picks a new position
//          each cycle (sample and hold); Phase shifts the cycle's start
//   v     *= depth, then a 5 ms one-pole so steps (Square, Random) glide
//   pos    (0.5 + 0.5 v) * pi / 2
//   gains  L: 1 - mix + mix * sqrt(2) cos(pos), R: 1 - mix + mix * sqrt(2) sin(pos)
// (an equal-power pan law with unity in the centre, blended with the dry
// signal by Mix). The panel preview draws the same law. Readout: LFO phase.
#include "builtin.hpp"
#include "dsp.hpp"

#include <algorithm>
#include <cmath>

namespace dstudio {

namespace {

enum Param : uint32_t { kRate, kDepth, kShape, kPhase, kMix, kSync, kRateSync, kParamCount };
enum Readout : uint32_t { kLfoPhase, kReadoutCount };
enum Shape : int { kSine, kTriangle, kSquare, kRandom };

constexpr const char* kParamIds[kParamCount] = {"rate", "depth", "shape", "phase", "mix", "sync", "rate_sync"};
constexpr const char* kReadoutIds[kReadoutCount] = {"lfo_phase"};
constexpr float kDefaults[kParamCount] = {2.0f, 80.0f, 0.0f, 0.0f, 100.0f, 1.0f, 8.0f};
constexpr float kSqrt2 = 1.41421356237f;

class Autopanner final : public BuiltinProcessor {
public:
    void prepare(double sample_rate, uint32_t) override {
        sr_ = sample_rate;
        glide_ = dsp::pole(5.0f, sr_);
        mix_.set_time(15.0f, sr_);
    }

    void reset() noexcept override {
        clock_.reset();
        rng_.seed(0x5eed);
        held_ = rng_.next();
        last_cycle_ph_ = 0.0f;
        primed_ = false;
    }

    void process(float* l, float* r, uint32_t n, const float* p, const BuiltinContext& ctx, float* ro) noexcept override {
        const float rate = std::clamp(p[kRate], 0.01f, 100.0f);
        const float depth = std::clamp(p[kDepth], 0.0f, 100.0f) * 0.01f;
        const float mix = std::clamp(p[kMix], 0.0f, 100.0f) * 0.01f;
        const float offset = std::clamp(p[kPhase], 0.0f, 360.0f) / 360.0f;
        const int shape = std::clamp(static_cast<int>(std::lround(p[kShape])), 0, 3);
        clock_.begin(rate, p[kSync] >= 0.5f, dsp::table_at(dsp::kDivisionQuarters, p[kRateSync]), ctx.transport, ctx.offset, sr_);
        if (!primed_) {
            v_ = depth * position(shape, dsp::wrap01(static_cast<float>(clock_.phase) + offset));
            mix_.snap(mix);
            primed_ = true;
        }
        float v = v_;
        for (uint32_t i = 0; i < n; ++i) {
            const float ph = dsp::wrap01(clock_.tick() + offset);
            if (ph < last_cycle_ph_) held_ = rng_.next(); // new cycle: next random position
            last_cycle_ph_ = ph;
            const float target = depth * position(shape, ph);
            v = target + glide_ * (v - target);
            const float m = mix_.next(mix);
            const float pos = 0.125f + 0.125f * v; // (0.5 + 0.5 v) * pi / 2, in cycles
            const float gl = 1.0f - m + m * kSqrt2 * dsp::cos2pi(pos);
            const float gr = 1.0f - m + m * kSqrt2 * dsp::sin2pi(pos);
            l[i] *= gl;
            r[i] *= gr;
        }
        v_ = v;
        ro[kLfoPhase] = static_cast<float>(clock_.phase);
    }

private:
    float position(int shape, float ph) const noexcept {
        switch (shape) {
        case kTriangle: return 4.0f * std::fabs(dsp::wrap01(ph + 0.25f) - 0.5f) - 1.0f;
        case kSquare: return ph < 0.5f ? -1.0f : 1.0f;
        case kRandom: return held_;
        default: return -dsp::sin2pi(ph);
        }
    }

    double sr_ = 48000.0;
    dsp::LfoClock clock_;
    dsp::Rng rng_;
    dsp::Smoother mix_;
    float glide_ = 0.0f, v_ = 0.0f, held_ = 0.0f, last_cycle_ph_ = 0.0f;
    bool primed_ = false;
};

std::unique_ptr<BuiltinProcessor> create() { return std::make_unique<Autopanner>(); }

} // namespace

extern const BuiltinDescriptor kAutopannerDescriptor;
const BuiltinDescriptor kAutopannerDescriptor{"autopanner", kParamIds, kReadoutIds, &create, {}, kDefaults};

} // namespace dstudio
