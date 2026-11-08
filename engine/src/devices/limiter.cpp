// Limiter: input gain, lookahead peak limiter with an optional true-peak
// (inter-sample) detector, release, dry/wet mix and output gain. Parameters
// in the order of saine_core::builtin::limiter (checked by a Rust test).
//
// Gain computer (stereo linked, in log2 gain units so the fade is smooth):
//   need[n]   = min(0, log2(ceiling / peak[n]))        peak: sample or 4x true peak
//   hold[n]   = min(need) over the last L samples      L = lookahead
//   follow[n] = min(hold[n], release toward 0)         exponential release
//   gain[n]   = mean(follow) over the last L samples   box filter: the fade
//                                                      reaches need[k] exactly
//                                                      when sample k is output
// The audio is delayed by the maximum lookahead (20 ms) plus the true-peak
// filter delay whatever the setting, so latency (PDC) never changes; the
// gain is delayed by the difference. A final clamp catches what the fade
// cannot (lookahead 0).
#include "builtin.hpp"
#include "dsp.hpp"
#include "dsp_dynamics.hpp"

#include <algorithm>
#include <cmath>

namespace saine {

namespace {

enum Param : uint32_t { kCeiling, kRelease, kLookahead, kTruePeak, kMix, kInput, kOutput, kParamCount };
enum Readout : uint32_t { kInDb, kOutDb, kGrDb, kReadoutCount };

constexpr const char* kParamIds[kParamCount] = {"ceiling", "release", "lookahead", "true_peak", "mix", "input_gain", "output_gain"};
constexpr const char* kReadoutIds[kReadoutCount] = {"in_db", "out_db", "gr_db"};
constexpr float kMaxLookaheadMs = 20.0f;

class Limiter final : public BuiltinProcessor {
public:
    void prepare(double sample_rate, uint32_t) override {
        sr_ = sample_rate;
        max_la_ = static_cast<uint32_t>(std::ceil(dsp::ms_to_samples(kMaxLookaheadMs, sr_)));
        latency_ = max_la_ + dsp::TruePeak4x::kDelay;
        for (auto& d : delay_) d.prepare(latency_);
        for (auto& d : det_delay_) d.prepare(dsp::TruePeak4x::kDelay);
        gain_delay_.prepare(latency_ + 1);
        gain_.prepare(max_la_ + 2);
        for (auto* s : {&input_, &mix_, &output_}) s->set_time(20.0f, sr_);
        reset();
    }

    void reset() noexcept override {
        for (auto& d : delay_) d.reset();
        for (auto& d : det_delay_) d.reset();
        gain_delay_.reset();
        gain_.reset();
        tp_[0].reset();
        tp_[1].reset();
        primed_ = false;
    }

    uint32_t latency() const noexcept override { return latency_; }

    void process(float* l, float* r, uint32_t n, const float* p, const BuiltinContext&, float* ro) noexcept override {
        const float ceiling = dsp::db_to_gain(std::clamp(p[kCeiling], -60.0f, 0.0f));
        const float log2_ceiling = std::log2(ceiling);
        const float a_rel = dsp::pole(std::clamp(p[kRelease], 0.1f, 10000.0f), sr_);
        const uint32_t la = std::min(max_la_, static_cast<uint32_t>(std::lround(dsp::ms_to_samples(std::clamp(p[kLookahead], 0.0f, kMaxLookaheadMs), sr_))));
        const uint32_t window = la + 1; // hold and fade over L + 1 samples (covers the newest one)
        const bool true_peak = p[kTruePeak] >= 0.5f;
        const float input = dsp::db_to_gain(std::clamp(p[kInput], -48.0f, 48.0f));
        const float mix = std::clamp(p[kMix], 0.0f, 100.0f) * 0.01f;
        const float output = dsp::db_to_gain(std::clamp(p[kOutput], -48.0f, 48.0f));
        if (!primed_) {
            input_.snap(input);
            mix_.snap(mix);
            output_.snap(output);
            primed_ = true;
        }
        // The fade for the sample at the head of the window is ready `la`
        // samples after detection; delay it to the fixed audio latency.
        const uint32_t gain_delay = latency_ - dsp::TruePeak4x::kDelay - la;

        float in_peak = 0.0f, out_peak = 0.0f, gr_min = 0.0f;
        for (uint32_t i = 0; i < n; ++i) {
            const float g_in = input_.next(input);
            const float xl = l[i] * g_in, xr = r[i] * g_in;
            in_peak = std::max(in_peak, std::max(std::fabs(xl), std::fabs(xr)));
            // Detector: true peak (lags by kDelay) or the sample peak delayed to match.
            const float tpl = tp_[0].next(xl), tpr = tp_[1].next(xr);
            const float sl = det_delay_[0].process(xl, dsp::TruePeak4x::kDelay);
            const float sr = det_delay_[1].process(xr, dsp::TruePeak4x::kDelay);
            const float peak = true_peak ? std::max(tpl, tpr) : std::max(std::fabs(sl), std::fabs(sr));
            const float need = peak > ceiling ? log2_ceiling - dsp::fast_log2(peak) : 0.0f;
            const float g_log2 = gain_delay_.process(gain_.push(need, window, a_rel), gain_delay);
            const float g = g_log2 < 0.0f ? dsp::fast_exp2(g_log2) : 1.0f;
            gr_min = std::min(gr_min, g_log2);

            const float dl = delay_[0].process(xl, latency_);
            const float dr = delay_[1].process(xr, latency_);
            const float wl = std::clamp(dl * g, -ceiling, ceiling);
            const float wr = std::clamp(dr * g, -ceiling, ceiling);
            const float m = mix_.next(mix), o = output_.next(output);
            const float yl = (dl + m * (wl - dl)) * o;
            const float yr = (dr + m * (wr - dr)) * o;
            l[i] = yl;
            r[i] = yr;
            out_peak = std::max(out_peak, std::max(std::fabs(yl), std::fabs(yr)));
        }
        ro[kInDb] = dsp::gain_to_db(in_peak);
        ro[kOutDb] = dsp::gain_to_db(out_peak);
        ro[kGrDb] = gr_min * dsp::kDbPerLog2;
    }

private:
    double sr_ = 48000.0;
    uint32_t max_la_ = 0, latency_ = 0;
    dsp::DelayLine delay_[2], det_delay_[2], gain_delay_;
    dsp::LookaheadGain gain_;
    dsp::TruePeak4x tp_[2];
    dsp::Smoother input_, mix_, output_;
    bool primed_ = false;
};

std::unique_ptr<BuiltinProcessor> create_limiter() { return std::make_unique<Limiter>(); }

} // namespace

extern const BuiltinDescriptor kLimiterDescriptor;
constexpr float kIdleReadouts[] = {-120.0f, -120.0f, 0.0f};
const BuiltinDescriptor kLimiterDescriptor{"limiter", kParamIds, kReadoutIds, &create_limiter, kIdleReadouts};

} // namespace saine
