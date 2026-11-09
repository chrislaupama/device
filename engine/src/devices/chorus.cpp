// Chorus: 1-4 modulated delay voices per channel, spread evenly over the
// LFO cycle, with a band-limited wet path, feedback, stereo width, dry/wet
// mix and output gain. Parameters in the order of saine_core::builtin::chorus
// (checked by a Rust test).
//
// Per channel and sample (v voices, LFO phase p, channel offset o):
//   delay_i  1.3 ms + depth * (0.5 + 0.5 s(p + o + i / v)),
//            s(x) = (sin 2 pi x - 0.063 sin 6 pi x) / 1.063 (a sine with a
//            slightly flattened top, which keeps the pitch sweep even)
//   wet      mean of the voices through a 35 Hz high-pass and a 4.7 kHz
//            low-pass (one-pole each); feedback writes x + fb * wet into the
//            delay line, so the response is H / (1 - fb H)
//   out      (dry * x + wet_gain * v^0.35 * wet) * output
// Right channel offset o = width * 0.5 / v; dry / wet gains are
// equal-power: d = 1 - mix, dry = d / n, wet = mix / n, n = sqrt(d^2 + mix^2).
// Delay taps use 4-point Hermite interpolation. Readout: the LFO phase.
#include "builtin.hpp"
#include "dsp.hpp"

#include <algorithm>
#include <cmath>

namespace saine {

namespace {

enum Param : uint32_t { kRate, kDepth, kMix, kVoices, kWidth, kFeedback, kSync, kRateSync, kOutput, kParamCount };
enum Readout : uint32_t { kPhase, kReadoutCount };

constexpr const char* kParamIds[kParamCount] = {"rate", "depth", "mix", "voices", "width", "feedback", "sync", "rate_sync", "output_gain"};
constexpr const char* kReadoutIds[kReadoutCount] = {"lfo_phase"};
constexpr float kDefaults[kParamCount] = {0.5f, 4.0f, 50.0f, 0.0f, 100.0f, 0.0f, 0.0f, 8.0f, 0.0f};
constexpr float kBaseMs = 1.3f;
constexpr float kMaxDepthMs = 8.0f;

inline float voice_shape(float x) noexcept { return (dsp::sin2pi(x) - 0.063f * dsp::sin2pi(3.0f * x)) * (1.0f / 1.063f); }

class Chorus final : public BuiltinProcessor {
public:
    void prepare(double sample_rate, uint32_t) override {
        sr_ = sample_rate;
        const auto len = static_cast<std::size_t>(std::ceil(static_cast<double>(kBaseMs + kMaxDepthMs) * 1e-3 * sr_)) + 8;
        for (auto& d : line_) d.prepare(len);
        for (auto& f : hp_) f.set(35.0f, sr_);
        for (auto& f : lp_) f.set(4700.0f, sr_);
        depth_.set_time(40.0f, sr_);
        width_.set_time(40.0f, sr_);
        dry_.set_time(15.0f, sr_);
        wet_.set_time(15.0f, sr_);
        fb_.set_time(15.0f, sr_);
        out_.set_time(15.0f, sr_);
        ms_to_samples_ = static_cast<float>(1e-3 * sr_);
    }

    void reset() noexcept override {
        for (auto& d : line_) d.reset();
        for (auto& f : hp_) f.reset();
        for (auto& f : lp_) f.reset();
        clock_.reset();
        primed_ = false;
    }

    void process(float* l, float* r, uint32_t n, const float* p, const BuiltinContext& ctx, float* ro) noexcept override {
        const float depth = std::clamp(p[kDepth], 0.0f, kMaxDepthMs) * ms_to_samples_;
        const int voices = std::clamp(static_cast<int>(std::lround(p[kVoices])), 0, 3) + 1;
        const float inv_v = 1.0f / static_cast<float>(voices);
        const float width = std::clamp(p[kWidth], 0.0f, 100.0f) * 0.01f * 0.5f * inv_v;
        const float fb = std::clamp(p[kFeedback], 0.0f, 90.0f) * 0.01f;
        const float out = dsp::db_to_gain(std::clamp(p[kOutput], -48.0f, 48.0f));
        float dry = 1.0f, wet = 0.0f;
        dsp::mix_gains(std::clamp(p[kMix], 0.0f, 100.0f) * 0.01f, 0.0f, dry, wet);
        wet *= std::pow(static_cast<float>(voices), 0.35f);
        clock_.begin(std::clamp(p[kRate], 0.01f, 50.0f), p[kSync] >= 0.5f, dsp::table_at(dsp::kDivisionQuarters, p[kRateSync]),
                     ctx.transport, ctx.offset, sr_);
        if (!primed_) {
            depth_.snap(depth);
            width_.snap(width);
            dry_.snap(dry);
            wet_.snap(wet);
            fb_.snap(fb);
            out_.snap(out);
            primed_ = true;
        }
        const float base = kBaseMs * ms_to_samples_ - 1.0f; // taps are read before this sample is written
        float* io[2] = {l, r};
        for (uint32_t i = 0; i < n; ++i) {
            const float ph = clock_.tick();
            const float dep = depth_.next(depth), w = width_.next(width), g = fb_.next(fb);
            const float gd = dry_.next(dry), gw = wet_.next(wet), go = out_.next(out);
            for (int c = 0; c < 2; ++c) {
                const float x = io[c][i];
                const float o = c == 0 ? 0.0f : w;
                float sum = 0.0f;
                for (int k = 0; k < voices; ++k) {
                    const float s = voice_shape(ph + o + static_cast<float>(k) * inv_v);
                    sum += line_[c].read(base + dep * (0.5f + 0.5f * s));
                }
                const float y = lp_[c].lp(hp_[c].hp(sum * inv_v));
                line_[c].push(x + g * y);
                io[c][i] = (gd * x + gw * y) * go;
            }
        }
        ro[kPhase] = static_cast<float>(clock_.phase);
    }

private:
    double sr_ = 48000.0;
    float ms_to_samples_ = 48.0f;
    dsp::LfoClock clock_;
    dsp::DelayLine line_[2];
    dsp::OnePole hp_[2], lp_[2];
    dsp::Smoother depth_, width_, dry_, wet_, fb_, out_;
    bool primed_ = false;
};

std::unique_ptr<BuiltinProcessor> create() { return std::make_unique<Chorus>(); }

} // namespace

extern const BuiltinDescriptor kChorusDescriptor;
const BuiltinDescriptor kChorusDescriptor{"chorus", kParamIds, kReadoutIds, &create, {}, kDefaults};

} // namespace saine
