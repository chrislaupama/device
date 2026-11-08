// Phaser: a cascade of 2, 4, 6 or 12 first-order all-pass sections swept by
// an LFO, with feedback, stereo width, dry/wet mix and output gain.
// Parameters in the order of saine_core::builtin::phaser (checked by a Rust
// test).
//
// LFO phase p (sine), per channel (right channel at p + width / 2):
//   sweep    f = exp(ln fc + h (2 l - 1)), l = 0.5 + 0.5 sin 2 pi p,
//            fc = sqrt(200 * 8000) Hz, h = 0.5 ln(40) depth: at 100% depth
//            the break frequency sweeps 200 Hz .. 8 kHz exponentially
//   stage    y = a x + s, s = x - a y, a = (tan(pi f / fs) - 1) / (tan(pi f / fs) + 1)
//   loop     the cascade's input is x + fb * y[n - 1]
//   out      (dry * x + wet * y) * output; dry / wet from Mix with the
//            equal-power law and a 0.2 correlation term
// The coefficient is computed every 16 samples and interpolated linearly in
// between (smooth sweeps at a fraction of the cost). Readout: LFO phase.
#include "builtin.hpp"
#include "dsp.hpp"

#include <algorithm>
#include <cmath>

namespace saine {

namespace {

enum Param : uint32_t { kRate, kDepth, kFeedback, kMix, kStages, kWidth, kSync, kRateSync, kOutput, kParamCount };
enum Readout : uint32_t { kPhase, kReadoutCount };

constexpr const char* kParamIds[kParamCount] = {"rate", "depth", "feedback", "mix", "stages", "width", "sync", "rate_sync", "output_gain"};
constexpr const char* kReadoutIds[kReadoutCount] = {"lfo_phase"};
constexpr float kDefaults[kParamCount] = {0.5f, 70.0f, 40.0f, 50.0f, 1.0f, 50.0f, 0.0f, 8.0f, 0.0f};
constexpr int kStageCounts[] = {2, 4, 6, 12};
constexpr int kMaxStages = 12;
constexpr uint32_t kControl = 16;

class Phaser final : public BuiltinProcessor {
public:
    void prepare(double sample_rate, uint32_t) override {
        sr_ = sample_rate;
        for (auto* s : {&depth_, &width_, &fb_, &dry_, &wet_, &out_}) s->set_time(25.0f, sr_);
        ln_fc_ = 0.5f * std::log(200.0f * 8000.0f);
        f_max_ = 0.45f * static_cast<float>(sr_);
    }

    void reset() noexcept override {
        for (auto& ch : s_)
            for (float& v : ch) v = 0.0f;
        y_[0] = y_[1] = 0.0f;
        clock_.reset();
        primed_ = false;
        countdown_ = 0;
    }

    void process(float* l, float* r, uint32_t n, const float* p, const BuiltinContext& ctx, float* ro) noexcept override {
        const int stages = kStageCounts[std::clamp(static_cast<int>(std::lround(p[kStages])), 0, 3)];
        const float depth = std::clamp(p[kDepth], 0.0f, 100.0f) * 0.01f;
        const float fb = std::clamp(p[kFeedback], -95.0f, 95.0f) * 0.01f;
        const float width = std::clamp(p[kWidth], 0.0f, 100.0f) * 0.005f;
        const float out = dsp::db_to_gain(std::clamp(p[kOutput], -48.0f, 48.0f));
        float dry = 1.0f, wet = 0.0f;
        dsp::mix_gains(std::clamp(p[kMix], 0.0f, 100.0f) * 0.01f, 0.2f, dry, wet);
        clock_.begin(std::clamp(p[kRate], 0.001f, 50.0f), p[kSync] >= 0.5f, dsp::table_at(dsp::kDivisionQuarters, p[kRateSync]),
                     ctx.transport, ctx.offset, sr_);
        if (!primed_) {
            depth_.snap(depth);
            width_.snap(width);
            fb_.snap(fb);
            dry_.snap(dry);
            wet_.snap(wet);
            out_.snap(out);
            for (int c = 0; c < 2; ++c) a_[c] = coefficient(static_cast<float>(clock_.phase) + (c == 0 ? 0.0f : width), depth);
            primed_ = true;
            countdown_ = 0;
        }
        float* io[2] = {l, r};
        for (uint32_t i = 0; i < n; ++i) {
            const float ph = clock_.tick();
            const float dep = depth_.next(depth), w = width_.next(width), g = fb_.next(fb);
            const float gd = dry_.next(dry), gw = wet_.next(wet), go = out_.next(out);
            if (countdown_ == 0) {
                // Next control point kControl samples ahead; step towards it per sample.
                const float ahead = ph + static_cast<float>(clock_.inc) * static_cast<float>(kControl);
                for (int c = 0; c < 2; ++c) da_[c] = (coefficient(ahead + (c == 0 ? 0.0f : w), dep) - a_[c]) / static_cast<float>(kControl);
                countdown_ = kControl;
            }
            --countdown_;
            for (int c = 0; c < 2; ++c) {
                a_[c] += da_[c];
                const float a = a_[c];
                const float x = io[c][i];
                float v = x + g * y_[c];
                float* s = s_[c];
                for (int k = 0; k < stages; ++k) {
                    const float y = a * v + s[k];
                    s[k] = v - a * y;
                    v = y;
                }
                y_[c] = v;
                io[c][i] = (gd * x + gw * v) * go;
            }
        }
        ro[kPhase] = static_cast<float>(clock_.phase);
    }

private:
    float coefficient(float ph, float depth) const noexcept {
        const float lfo = 0.5f + 0.5f * dsp::sin2pi(ph);
        const float h = 0.5f * std::log(40.0f) * depth;
        const float f = std::min(std::exp(ln_fc_ + h * (2.0f * lfo - 1.0f)), f_max_);
        const float t = std::tan(dsp::kPi * f / static_cast<float>(sr_));
        return (t - 1.0f) / (t + 1.0f);
    }

    double sr_ = 48000.0;
    float ln_fc_ = 0.0f, f_max_ = 20000.0f;
    dsp::LfoClock clock_;
    dsp::Smoother depth_, width_, fb_, dry_, wet_, out_;
    float s_[2][kMaxStages]{};
    float y_[2]{};
    float a_[2]{}, da_[2]{};
    uint32_t countdown_ = 0;
    bool primed_ = false;
};

std::unique_ptr<BuiltinProcessor> create() { return std::make_unique<Phaser>(); }

} // namespace

extern const BuiltinDescriptor kPhaserDescriptor;
const BuiltinDescriptor kPhaserDescriptor{"phaser", kParamIds, kReadoutIds, &create, {}, kDefaults};

} // namespace saine
