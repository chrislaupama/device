// Tremolo: gain modulation by an LFO (Sine, Triangle, soft-edged Square,
// Saw Down), depth, dry/wet mix, a right-channel phase offset (Stereo
// Phase) and tempo sync. Parameters in the order of
// saine_core::builtin::tremolo (checked by a Rust test).
//
// Per channel and sample:
//   w      unit shape of the LFO phase (0 = bottom of the dip, 1 = top)
//   gain   1 - mix + mix * (1 - depth * (1 - w))
//   out    in * gain
// The shapes follow the reference's drawing law: sine 0.5 + 0.5 sin, a
// triangle peaking at half a cycle, a square that is high for 48% of the
// cycle with raised-cosine edges 4% of a cycle wide, and a falling ramp.
// The ramp's reset and the square's edges are additionally rounded by a
// 0.3 ms one-pole so no setting clicks. Readout: the LFO phase (for the
// panel's live dot).
#include "builtin.hpp"
#include "dsp.hpp"

#include <algorithm>
#include <cmath>

namespace saine {

namespace {

enum Param : uint32_t { kRate, kDepth, kShape, kStereo, kMix, kSync, kRateSync, kParamCount };
enum Readout : uint32_t { kPhase, kReadoutCount };
enum Shape : int { kSine, kTriangle, kSquare, kSawDown };

constexpr const char* kParamIds[kParamCount] = {"rate", "depth", "shape", "stereo_phase", "mix", "sync", "rate_sync"};
constexpr const char* kReadoutIds[kReadoutCount] = {"lfo_phase"};
constexpr float kDefaults[kParamCount] = {5.0f, 70.0f, 0.0f, 0.0f, 100.0f, 0.0f, 8.0f};

// Unit shape (0..1) at phase p (cycles).
inline float shape_value(int shape, float p) noexcept {
    p = dsp::wrap01(p);
    switch (shape) {
    case kTriangle: return 1.0f - 2.0f * std::fabs(p - 0.5f);
    case kSquare: {
        // High for 48% of the cycle, a raised-cosine fall over 48..52%, low
        // for the last 48%; the rise at the wrap is rounded by the edge pole.
        constexpr float e = 0.02f;
        if (p < 0.5f - e) return 1.0f;
        if (p < 0.5f + e) return 0.5f + 0.5f * dsp::cos2pi(0.5f * (p - (0.5f - e)) / (2.0f * e));
        return 0.0f;
    }
    case kSawDown: return 1.0f - p;
    default: return 0.5f + 0.5f * dsp::sin2pi(p);
    }
}

class Tremolo final : public BuiltinProcessor {
public:
    void prepare(double sample_rate, uint32_t) override {
        sr_ = sample_rate;
        depth_.set_time(15.0f, sr_);
        mix_.set_time(15.0f, sr_);
        stereo_.set_time(30.0f, sr_);
        edge_ = dsp::pole(0.3f, sr_);
    }

    void reset() noexcept override {
        clock_.reset();
        primed_ = false;
        gl_ = gr_ = 1.0f;
    }

    void process(float* l, float* r, uint32_t n, const float* p, const BuiltinContext& ctx, float* ro) noexcept override {
        const float rate = std::clamp(p[kRate], 0.01f, 100.0f);
        const float depth = std::clamp(p[kDepth], 0.0f, 100.0f) * 0.01f;
        const float mix = std::clamp(p[kMix], 0.0f, 100.0f) * 0.01f;
        const float stereo = std::clamp(p[kStereo], 0.0f, 360.0f) / 360.0f;
        const int shape = std::clamp(static_cast<int>(std::lround(p[kShape])), 0, 3);
        const bool synced = p[kSync] >= 0.5f;
        clock_.begin(rate, synced, dsp::table_at(dsp::kDivisionQuarters, p[kRateSync]), ctx.transport, ctx.offset, sr_);
        if (!primed_) {
            depth_.snap(depth);
            mix_.snap(mix);
            stereo_.snap(stereo);
            gl_ = target_gain(shape, static_cast<float>(clock_.phase), 0.0f, depth, mix);
            gr_ = target_gain(shape, static_cast<float>(clock_.phase), stereo, depth, mix);
            primed_ = true;
        }
        float gl = gl_, gr = gr_;
        const float a = edge_;
        for (uint32_t i = 0; i < n; ++i) {
            const float ph = clock_.tick();
            const float d = depth_.next(depth), m = mix_.next(mix), s = stereo_.next(stereo);
            const float tl = target_gain(shape, ph, 0.0f, d, m);
            const float tr = target_gain(shape, ph, s, d, m);
            gl = tl + a * (gl - tl);
            gr = tr + a * (gr - tr);
            l[i] *= gl;
            r[i] *= gr;
        }
        gl_ = gl;
        gr_ = gr;
        ro[kPhase] = static_cast<float>(clock_.phase);
    }

private:
    static float target_gain(int shape, float ph, float offset, float depth, float mix) noexcept {
        const float w = shape_value(shape, ph + offset);
        return 1.0f - mix + mix * (1.0f - depth * (1.0f - w));
    }

    double sr_ = 48000.0;
    dsp::LfoClock clock_;
    dsp::Smoother depth_, mix_, stereo_;
    float edge_ = 0.0f, gl_ = 1.0f, gr_ = 1.0f;
    bool primed_ = false;
};

std::unique_ptr<BuiltinProcessor> create() { return std::make_unique<Tremolo>(); }

} // namespace

extern const BuiltinDescriptor kTremoloDescriptor;
const BuiltinDescriptor kTremoloDescriptor{"tremolo", kParamIds, kReadoutIds, &create, {}, kDefaults};

} // namespace saine
