// Gate: downward expander with hold, lookahead and a detector high-pass.
// Parameters in the order of dstudio_core::builtin::gate (checked by a Rust
// test).
//
//   detector   high-passed L/R (Sidechain HPF, 12 dB/oct), max(|L|, |R|),
//              peak follower (instant rise, 10 ms fall), in dB
//   state      open at the threshold; stays open while within 2 dB below it
//              (hysteresis, no chatter) and for Hold after that
//   target     0 dB while open, else (level - threshold) * (ratio - 1),
//              floored at -120 dB (ratio 100 is a full gate)
//   gain       Attack while opening (linear gain, so the gate is fully open
//              after a few Attack times), Release while closing (in dB, a
//              natural fade)
//   lookahead  the audio is delayed by the maximum (20 ms) so latency never
//              changes; the gain is delayed by 20 ms - Lookahead, so the gate
//              opens Lookahead before the hit
//   output     dry + mix * (gated - dry)
#include "builtin.hpp"
#include "dsp.hpp"
#include "dsp_dynamics.hpp"

#include <algorithm>
#include <cmath>

namespace dstudio {

namespace {

enum Param : uint32_t { kThreshold, kRatio, kAttack, kHold, kRelease, kLookahead, kScHpf, kMix, kParamCount };
enum Readout : uint32_t { kInDb, kOutDb, kGrDb, kReadoutCount };

constexpr const char* kParamIds[kParamCount] = {"threshold", "ratio", "attack", "hold", "release", "lookahead", "sidechain_hpf", "mix"};
constexpr const char* kReadoutIds[kReadoutCount] = {"in_db", "out_db", "gr_db"};
constexpr float kMaxLookaheadMs = 20.0f;
constexpr float kHysteresisDb = 2.0f;
constexpr float kFloorDb = -120.0f;

class Gate final : public BuiltinProcessor {
public:
    void prepare(double sample_rate, uint32_t) override {
        sr_ = sample_rate;
        latency_ = static_cast<uint32_t>(std::ceil(dsp::ms_to_samples(kMaxLookaheadMs, sr_)));
        for (auto& d : delay_) d.prepare(latency_);
        gain_delay_.prepare(latency_);
        a_det_ = dsp::pole(10.0f, sr_);
        mix_.set_time(20.0f, sr_);
        hpf_hz_ = -1.0f;
        reset();
    }

    void reset() noexcept override {
        for (auto& d : delay_) d.reset();
        gain_delay_.reset();
        for (auto& f : hpf_) f.reset();
        env_ = 0.0f;
        gain_db_ = kFloorDb;
        hold_left_ = 0;
        open_ = false;
        primed_ = false;
    }

    uint32_t latency() const noexcept override { return latency_; }

    void process(float* l, float* r, uint32_t n, const float* p, const BuiltinContext&, float* ro) noexcept override {
        const float threshold = std::clamp(p[kThreshold], -120.0f, 0.0f);
        const float slope = std::clamp(p[kRatio], 1.0f, 1000.0f) - 1.0f;
        const float a_att = dsp::pole(std::clamp(p[kAttack], 0.001f, 1000.0f), sr_);
        const float a_rel = dsp::pole(std::clamp(p[kRelease], 0.1f, 10000.0f), sr_);
        const auto hold = static_cast<uint32_t>(dsp::ms_to_samples(std::clamp(p[kHold], 0.0f, 5000.0f), sr_));
        const auto la = static_cast<uint32_t>(std::lround(dsp::ms_to_samples(std::clamp(p[kLookahead], 0.0f, kMaxLookaheadMs), sr_)));
        const uint32_t gain_delay = latency_ - std::min(la, latency_);
        const float hpf = std::clamp(p[kScHpf], 10.0f, 2000.0f);
        if (hpf != hpf_hz_) {
            hpf_hz_ = hpf;
            for (auto& f : hpf_) f.highpass(static_cast<double>(hpf), sr_);
        }
        const float mix = std::clamp(p[kMix], 0.0f, 100.0f) * 0.01f;
        if (!primed_) {
            mix_.snap(mix);
            primed_ = true;
        }

        float in_peak = 0.0f, out_peak = 0.0f, gr_min = 0.0f;
        float env = env_, gain_db = gain_db_;
        for (uint32_t i = 0; i < n; ++i) {
            const float xl = l[i], xr = r[i];
            in_peak = std::max(in_peak, std::max(std::fabs(xl), std::fabs(xr)));
            const float d = std::max(std::fabs(hpf_[0].tick(xl)), std::fabs(hpf_[1].tick(xr)));
            env = std::max(d, a_det_ * env);
            const float level = dsp::fast_gain_to_db(env);
            if (level >= threshold) {
                open_ = true;
                hold_left_ = hold;
            } else if (open_ && level < threshold - kHysteresisDb) {
                open_ = false;
            }
            float target = 0.0f;
            if (!open_) {
                if (hold_left_ > 0) {
                    --hold_left_;
                } else {
                    target = std::max(kFloorDb, (level - threshold) * slope);
                }
            }
            if (target > gain_db) {
                const float g0 = dsp::fast_db_to_gain(gain_db), t = dsp::fast_db_to_gain(target);
                gain_db = std::max(kFloorDb, dsp::fast_gain_to_db(t + a_att * (g0 - t)));
            } else {
                gain_db = target + a_rel * (gain_db - target);
            }
            const float g_db = gain_delay_.process(gain_db, gain_delay);
            gr_min = std::min(gr_min, g_db);
            const float g = g_db < 0.0f ? dsp::fast_db_to_gain(g_db) : 1.0f;
            const float dl = delay_[0].process(xl, latency_);
            const float dr = delay_[1].process(xr, latency_);
            const float m = mix_.next(mix);
            const float yl = dl + m * (dl * g - dl);
            const float yr = dr + m * (dr * g - dr);
            l[i] = yl;
            r[i] = yr;
            out_peak = std::max(out_peak, std::max(std::fabs(yl), std::fabs(yr)));
        }
        for (auto& f : hpf_) f.flush_denormals();
        if (env < 1e-12f) env = 0.0f;
        env_ = env;
        gain_db_ = gain_db;
        ro[kInDb] = dsp::gain_to_db(in_peak);
        ro[kOutDb] = dsp::gain_to_db(out_peak);
        ro[kGrDb] = in_peak > 1e-5f ? gr_min : 0.0f; // a closed gate on silence reduces nothing audible
    }

private:
    double sr_ = 48000.0;
    uint32_t latency_ = 0, hold_left_ = 0;
    dsp::FixedDelay delay_[2], gain_delay_;
    dsp::Biquad hpf_[2];
    float hpf_hz_ = -1.0f, a_det_ = 0.0f, env_ = 0.0f, gain_db_ = kFloorDb;
    dsp::Smoother mix_;
    bool open_ = false, primed_ = false;
};

std::unique_ptr<BuiltinProcessor> create_gate() { return std::make_unique<Gate>(); }

} // namespace

extern const BuiltinDescriptor kGateDescriptor;
constexpr float kIdleReadouts[] = {-120.0f, -120.0f, 0.0f};
// Rust definition defaults, in parameter order.
constexpr float kDefaults[] = {-40.f, 10.f, 0.5f, 50.f, 100.f, 5.f, 80.f, 100.f};
const BuiltinDescriptor kGateDescriptor{"gate", kParamIds, kReadoutIds, &create_gate, kIdleReadouts, kDefaults};

} // namespace dstudio
