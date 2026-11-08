// Flanger: a short modulated delay with feedback, mixed with the dry signal,
// in three modes. Parameters in the order of saine_core::builtin::flanger
// (checked by a Rust test).
//
// LFO phase p (sine), per channel (right channel at p + width / 2):
//   Classic   delay 1 ms + depth (0.5 + 0.5 sin 2 pi p), feedback fb
//   Jet       the same with 0.6 x the depth and feedback 1.2 fb (limited
//             to +-0.85): a tighter, more resonant sweep
//   Thru-Zero the dry path is delayed by c = 1 ms + depth and the wet tap
//             sweeps c + depth sin 2 pi p, crossing the dry delay (the
//             through-zero cancellation); feedback limited to +-0.85, wet
//             lifted by 2.4 dB
//   wet      tap of a delay line written with x + fb * wet (H / (1 - fb H))
//   out      (dry * x_dry + wet * tap) * output; dry / wet from Mix with
//            the equal-power law and a -0.15 correlation term
// Taps use 4-point Hermite interpolation. Readout: the LFO phase.
#include "builtin.hpp"
#include "dsp.hpp"

#include <algorithm>
#include <cmath>

namespace saine {

namespace {

enum Param : uint32_t { kRate, kDepth, kFeedback, kMix, kWidth, kMode, kSync, kRateSync, kOutput, kParamCount };
enum Readout : uint32_t { kPhase, kReadoutCount };
enum Mode : int { kClassic, kJet, kThruZero };

constexpr const char* kParamIds[kParamCount] = {"rate", "depth", "feedback", "mix", "width", "mode", "sync", "rate_sync", "output_gain"};
constexpr const char* kReadoutIds[kReadoutCount] = {"lfo_phase"};
constexpr float kDefaults[kParamCount] = {0.3f, 3.0f, 60.0f, 50.0f, 50.0f, 0.0f, 0.0f, 8.0f, 0.0f};
constexpr float kBaseMs = 1.0f;
constexpr float kMaxDepthMs = 10.0f;

class Flanger final : public BuiltinProcessor {
public:
    void prepare(double sample_rate, uint32_t) override {
        sr_ = sample_rate;
        ms_ = static_cast<float>(1e-3 * sr_);
        const auto len = static_cast<std::size_t>(std::ceil((kBaseMs + 2.0f * kMaxDepthMs) * ms_)) + 8;
        for (auto& d : line_) d.prepare(len);
        for (auto& d : dry_line_) d.prepare(len);
        for (auto* s : {&depth_, &width_, &fb_, &dry_, &wet_, &out_, &centre_}) s->set_time(30.0f, sr_);
    }

    void reset() noexcept override {
        for (auto& d : line_) d.reset();
        for (auto& d : dry_line_) d.reset();
        clock_.reset();
        primed_ = false;
    }

    void process(float* l, float* r, uint32_t n, const float* p, const BuiltinContext& ctx, float* ro) noexcept override {
        const int mode = std::clamp(static_cast<int>(std::lround(p[kMode])), 0, 2);
        float depth = std::clamp(p[kDepth], 0.0f, kMaxDepthMs) * ms_;
        float fb = std::clamp(p[kFeedback], -90.0f, 90.0f) * 0.01f;
        float dry = 1.0f, wet = 0.0f;
        dsp::mix_gains(std::clamp(p[kMix], 0.0f, 100.0f) * 0.01f, -0.15f, dry, wet);
        float centre = 0.0f;
        if (mode == kJet) {
            depth *= 0.6f;
            fb = std::clamp(fb * 1.2f, -0.85f, 0.85f);
        } else if (mode == kThruZero) {
            fb = std::clamp(fb, -0.85f, 0.85f);
            wet *= 1.318f;
            centre = kBaseMs * ms_ + depth;
        }
        const float width = std::clamp(p[kWidth], 0.0f, 100.0f) * 0.005f;
        const float out = dsp::db_to_gain(std::clamp(p[kOutput], -48.0f, 48.0f));
        clock_.begin(std::clamp(p[kRate], 0.001f, 50.0f), p[kSync] >= 0.5f, dsp::table_at(dsp::kDivisionQuarters, p[kRateSync]),
                     ctx.transport, ctx.offset, sr_);
        if (!primed_) {
            depth_.snap(depth);
            width_.snap(width);
            fb_.snap(fb);
            dry_.snap(dry);
            wet_.snap(wet);
            out_.snap(out);
            centre_.snap(centre);
            primed_ = true;
        }
        const bool tz = mode == kThruZero;
        const float base = kBaseMs * ms_ - 1.0f; // taps are read before the sample is written
        float* io[2] = {l, r};
        for (uint32_t i = 0; i < n; ++i) {
            const float ph = clock_.tick();
            const float dep = depth_.next(depth), w = width_.next(width), g = fb_.next(fb);
            const float gd = dry_.next(dry), gw = wet_.next(wet), go = out_.next(out), c0 = centre_.next(centre);
            for (int c = 0; c < 2; ++c) {
                const float x = io[c][i];
                const float s = dsp::sin2pi(ph + (c == 0 ? 0.0f : w));
                dry_line_[c].push(x); // always written so a switch to Thru-Zero starts clean
                float d, xd;
                if (tz) {
                    d = c0 - 1.0f + dep * s;
                    xd = dry_line_[c].read(c0);
                } else {
                    d = base + dep * (0.5f + 0.5f * s);
                    xd = x;
                }
                const float y = line_[c].read(d);
                line_[c].push(x + g * y);
                io[c][i] = (gd * xd + gw * y) * go;
            }
        }
        ro[kPhase] = static_cast<float>(clock_.phase);
    }

private:
    double sr_ = 48000.0;
    float ms_ = 48.0f;
    dsp::LfoClock clock_;
    dsp::DelayLine line_[2], dry_line_[2];
    dsp::Smoother depth_, width_, fb_, dry_, wet_, out_, centre_;
    bool primed_ = false;
};

std::unique_ptr<BuiltinProcessor> create() { return std::make_unique<Flanger>(); }

} // namespace

extern const BuiltinDescriptor kFlangerDescriptor;
const BuiltinDescriptor kFlangerDescriptor{"flanger", kParamIds, kReadoutIds, &create, {}, kDefaults};

} // namespace saine
