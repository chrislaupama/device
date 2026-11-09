// Mastering Limiter: a two-stage, oversampled, true-peak limiter with
// stereo / mid-side linking, auto release, monitors and dither. Parameters in
// the order of saine_core::builtin::mastering_limiter (checked by a Rust test).
//
// Signal flow (base rate unless noted):
//   push       input * (ceiling - threshold) dB: the threshold lands on the ceiling
//   oversample 1x / 2x / 4x / 8x (linear-phase half-band FIR cascade)
//   at the oversampled rate, per channel (L/R, or M/S in Mid-Side mode):
//     need     log2 gain that brings the peak to the ceiling (<= 0); peaks
//              are true peaks (4x estimate) when True Peak is on and the
//              rate is below 4x, else sample peaks of the oversampled signal
//     link     need = link * linked + (1 - link) * own; linked = the L/R
//              need of the louder channel (Release Link for the slow stage,
//              Transient Link for the fast stage)
//     slow     a share of the need (style), smoothed with Attack / Release;
//              Auto Release runs a fast and a slow release path and the
//              deeper one wins
//     fast     what the slow stage leaves: min over the lookahead window,
//              release, then a box fade over the window (reaches the need
//              exactly when the peak is output)
//   downsample
//   guard      a short (16-sample) linked lookahead stage on the base-rate
//              output that catches what band-limiting and Mid-Side leave
//              over the ceiling (true peaks when True Peak is on); it
//              rarely moves more than a fraction of a dB
//   monitors   Unity: undo the push (hear the limiting level-matched);
//              Delta: push * input - output (hear what is removed)
//   dither     TPDF at 24 / 20 / 16 bits, error-feedback noise shaping
//              (Flat, Light: 1 - 0.5 z^-1, Normal: 1 - z^-1,
//              Strong: (1 - z^-1)^2)
// Styles (share of the slow stage, release scale): Clean 0.5 / 1, Loud
// 0.35 / 0.6, Gentle 1.0 / 1.5 (one slow stage; the fast one only catches
// overs), Dense 0.8 / 1.
// Latency: oversampler delay + 5 ms (the maximum lookahead, whatever the
// setting) + the true-peak estimator delay below 4x + the guard; it depends
// only on the oversampling factor (latency_for).
#include "builtin.hpp"
#include "dsp.hpp"
#include "dsp_dynamics.hpp"

#include <algorithm>
#include <cmath>

namespace saine {

namespace {

enum Param : uint32_t {
    kThreshold,
    kCeiling,
    kRelease,
    kAutoRelease,
    kStyle,
    kLookahead,
    kAttack,
    kTransientLink,
    kReleaseLink,
    kMsMode,
    kOversampling,
    kTruePeak,
    kDitherBits,
    kDitherShape,
    kBypass,
    kUnityMonitor,
    kDeltaMonitor,
    kParamCount
};
enum Readout : uint32_t { kInDb, kOutDb, kGrDb, kReadoutCount };

constexpr const char* kParamIds[kParamCount] = {
    "threshold",      "ceiling",      "release",    "auto_release", "style",       "lookahead",
    "attack",         "transient_link", "release_link", "ms_mode",    "oversampling", "true_peak",
    "dither_bits",    "dither_shape", "bypass",     "unity_monitor", "delta_monitor"};
constexpr const char* kReadoutIds[kReadoutCount] = {"in_db", "out_db", "gr_db"};

constexpr float kMaxLookaheadMs = 5.0f;
constexpr float kSlowShare[] = {0.5f, 0.35f, 1.0f, 0.8f};
constexpr float kReleaseScale[] = {1.0f, 0.6f, 1.5f, 1.0f};
constexpr uint32_t kFactors[] = {1, 2, 4, 8};
constexpr int kWordLength[] = {0, 24, 20, 16};
constexpr uint32_t kGuardWindow = 16;
constexpr uint32_t kGuardDelay = kGuardWindow - 1 + dsp::TruePeak4x::kDelay;

uint32_t factor_of(float v) noexcept { return kFactors[std::clamp(static_cast<int>(std::lround(v)), 0, 3)]; }
// True-peak estimator delay at the oversampled rate (only used below 4x).
constexpr uint32_t tp_delay(uint32_t f) noexcept { return f < 4 ? dsp::TruePeak4x::kDelay : 0; }

class MasteringLimiter final : public BuiltinProcessor {
public:
    void prepare(double sample_rate, uint32_t) override {
        sr_ = sample_rate;
        la_base_ = static_cast<uint32_t>(std::ceil(dsp::ms_to_samples(kMaxLookaheadMs, sr_)));
        const uint32_t max_la = la_base_ * dsp::Oversampler::kMaxFactor;
        for (int c = 0; c < 2; ++c) {
            audio_[c].prepare(max_la + dsp::TruePeak4x::kDelay);
            det_[c].prepare(dsp::TruePeak4x::kDelay);
            slow_delay_[c].prepare(max_la);
            fast_delay_[c].prepare(max_la);
            fast_[c].prepare(max_la + 2);
            guard_audio_[c].prepare(kGuardDelay);
            guard_det_[c].prepare(dsp::TruePeak4x::kDelay);
            dry_[c].prepare(latency_of(8));
        }
        push_.set_time(20.0f, sr_);
        guard_.prepare(kGuardWindow + 1);
        guard_rel_ = dsp::pole(10.0f, sr_);
        factor_ = 0;
        reset();
    }

    void reset() noexcept override {
        for (int c = 0; c < 2; ++c) {
            audio_[c].reset();
            det_[c].reset();
            slow_delay_[c].reset();
            fast_delay_[c].reset();
            fast_[c].reset();
            guard_audio_[c].reset();
            guard_det_[c].reset();
            guard_tp_[c].reset();
            dry_[c].reset();
            os_[c].reset();
            tp_[c].reset();
            tp_lr_[c].reset();
            slow_[c] = slow_long_[c] = 0.0f;
            err_[c][0] = err_[c][1] = 0.0f;
        }
        guard_.reset();
        primed_ = false;
    }

    uint32_t latency() const noexcept override { return latency_of(factor_ == 0 ? 4 : factor_); }
    uint32_t latency_for(const float* p) const noexcept override { return latency_of(factor_of(p[kOversampling])); }

    void process(float* l, float* r, uint32_t n, const float* p, const BuiltinContext&, float* ro) noexcept override {
        const uint32_t f = factor_of(p[kOversampling]);
        if (f != factor_) { // a new rate: start clean
            factor_ = f;
            reset();
        }
        const double rate = sr_ * f;
        const float threshold = std::clamp(p[kThreshold], -60.0f, 0.0f);
        const float ceiling_db = std::clamp(p[kCeiling], -60.0f, 0.0f);
        const float c_log2 = ceiling_db * dsp::kLog2_10_over_20;
        const float ceiling = std::exp2(c_log2);
        const float push = dsp::db_to_gain(ceiling_db - threshold);
        const int style = std::clamp(static_cast<int>(std::lround(p[kStyle])), 0, 3);
        const float share = kSlowShare[style];
        const float release = std::clamp(p[kRelease], 0.01f, 5000.0f) * kReleaseScale[style];
        const bool auto_release = p[kAutoRelease] >= 0.5f;
        const float a_att = dsp::pole(std::clamp(p[kAttack], 0.0f, 1000.0f), rate);
        const float a_rel = dsp::pole(auto_release ? 0.35f * release : release, rate);
        const float a_rel_long = dsp::pole(2.5f * release, rate);
        const float a_att_long = dsp::pole(4.0f * std::clamp(p[kAttack], 0.0f, 1000.0f) + 20.0f, rate);
        const float a_fast_rel = dsp::pole(std::clamp(0.2f * release, 1.0f, 50.0f), rate);
        const uint32_t la = std::clamp(static_cast<uint32_t>(std::lround(dsp::ms_to_samples(std::clamp(p[kLookahead], 0.0f, kMaxLookaheadMs), rate))), 1u,
                                       la_base_ * f);
        const uint32_t max_la = la_base_ * f;
        const uint32_t window = la + 1;
        const float t_link = std::clamp(p[kTransientLink], 0.0f, 100.0f) * 0.01f;
        const float r_link = std::clamp(p[kReleaseLink], 0.0f, 100.0f) * 0.01f;
        const bool ms = p[kMsMode] >= 0.5f;
        const bool true_peak = p[kTruePeak] >= 0.5f && f < 4;
        const uint32_t g = tp_delay(f);
        const int bits = kWordLength[std::clamp(static_cast<int>(std::lround(p[kDitherBits])), 0, 3)];
        const int shape = std::clamp(static_cast<int>(std::lround(p[kDitherShape])), 0, 3);
        const bool bypass = p[kBypass] >= 0.5f;
        const bool unity = p[kUnityMonitor] >= 0.5f;
        const bool delta = p[kDeltaMonitor] >= 0.5f;
        const uint32_t lat = latency_of(f);
        if (!primed_) {
            push_.snap(push);
            primed_ = true;
        }

        float in_peak = 0.0f, out_peak = 0.0f, gr_min = 0.0f;
        for (uint32_t i = 0; i < n; ++i) {
            const float xl = l[i], xr = r[i];
            in_peak = std::max(in_peak, std::max(std::fabs(xl), std::fabs(xr)));
            const float pg = push_.next(push);
            float ul[dsp::Oversampler::kMaxFactor], ur[dsp::Oversampler::kMaxFactor];
            os_[0].up(xl * pg, ul, f);
            os_[1].up(xr * pg, ur, f);
            for (uint32_t j = 0; j < f; ++j) {
                const float L = ul[j], R = ur[j];
                const float a = ms ? 0.5f * (L + R) : L;
                const float b = ms ? 0.5f * (L - R) : R;
                // Detection (true-peak estimates lag by g; sample peaks are delayed to match).
                float pa, pb, pl, pr;
                if (true_peak) {
                    pl = tp_lr_[0].next(L);
                    pr = tp_lr_[1].next(R);
                    pa = ms ? tp_[0].next(a) : pl;
                    pb = ms ? tp_[1].next(b) : pr;
                } else {
                    const float da = det_[0].process(a, g), db = det_[1].process(b, g);
                    pa = std::fabs(da);
                    pb = std::fabs(db);
                    pl = ms ? std::fabs(da + db) : pa;
                    pr = ms ? std::fabs(da - db) : pb;
                }
                const float need_link = need(std::max(pl, pr), ceiling, c_log2);
                const float own[2] = {need(pa, ceiling, c_log2), need(pb, ceiling, c_log2)};
                float gain[2];
                for (int c = 0; c < 2; ++c) {
                    // Slow stage: a share of the need, attack / release (auto: two paths).
                    const float ts = share * (r_link * need_link + (1.0f - r_link) * own[c]);
                    float s = slow_[c] = ts + (ts < slow_[c] ? a_att : a_rel) * (slow_[c] - ts);
                    if (auto_release) {
                        slow_long_[c] = ts + (ts < slow_long_[c] ? a_att_long : a_rel_long) * (slow_long_[c] - ts);
                        s = std::min(s, slow_long_[c]);
                    }
                    // Fast stage: what is left, held over the window, released, faded in.
                    const float tf = std::min(0.0f, t_link * need_link + (1.0f - t_link) * own[c] - s);
                    const float fade = fast_[c].push(tf, window, a_fast_rel);
                    gain[c] = slow_delay_[c].process(s, max_la) + fast_delay_[c].process(fade, max_la - la);
                }
                gr_min = std::min(gr_min, std::min(gain[0], gain[1]));
                const float da = audio_[0].process(a, max_la + g);
                const float db = audio_[1].process(b, max_la + g);
                const float ga = gain[0] < 0.0f ? dsp::fast_exp2(gain[0]) : 1.0f;
                const float gb = gain[1] < 0.0f ? dsp::fast_exp2(gain[1]) : 1.0f;
                const float oa = da * ga, ob = db * gb;
                ul[j] = ms ? oa + ob : oa;
                ur[j] = ms ? oa - ob : ob;
            }
            float yl = os_[0].down(ul, f), yr = os_[1].down(ur, f);
            {   // guard
                const float gl = guard_tp_[0].next(yl), gr = guard_tp_[1].next(yr);
                const float pk = p[kTruePeak] >= 0.5f ? std::max(gl, gr)
                                                       : std::max(std::fabs(guard_det_[0].process(yl, dsp::TruePeak4x::kDelay)),
                                                                  std::fabs(guard_det_[1].process(yr, dsp::TruePeak4x::kDelay)));
                const float gg = guard_.push(need(pk, ceiling, c_log2), kGuardWindow, guard_rel_);
                const float k = gg < 0.0f ? dsp::fast_exp2(gg) : 1.0f;
                gr_min = std::min(gr_min, gg);
                yl = std::clamp(guard_audio_[0].process(yl, kGuardDelay) * k, -ceiling, ceiling);
                yr = std::clamp(guard_audio_[1].process(yr, kGuardDelay) * k, -ceiling, ceiling);
            }
            const float dl = dry_[0].process(xl, lat), dr = dry_[1].process(xr, lat);
            if (unity) {
                yl /= pg;
                yr /= pg;
            } else if (delta) {
                yl = dl * pg - yl;
                yr = dr * pg - yr;
            }
            if (bits > 0) {
                yl = dither(yl, 0, bits, shape);
                yr = dither(yr, 1, bits, shape);
            }
            if (bypass) {
                yl = dl;
                yr = dr;
            }
            l[i] = yl;
            r[i] = yr;
            out_peak = std::max(out_peak, std::max(std::fabs(yl), std::fabs(yr)));
        }
        for (int c = 0; c < 2; ++c) {
            if (slow_[c] > -1e-7f) slow_[c] = 0.0f;
            if (slow_long_[c] > -1e-7f) slow_long_[c] = 0.0f;
        }
        ro[kInDb] = dsp::gain_to_db(in_peak);
        ro[kOutDb] = dsp::gain_to_db(out_peak);
        ro[kGrDb] = gr_min * dsp::kDbPerLog2;
    }

private:
    uint32_t latency_of(uint32_t f) const noexcept { return dsp::Oversampler::latency(f) + la_base_ + tp_delay(f) / f + kGuardDelay; }

    // log2 gain (<= 0) that brings `peak` down to the ceiling.
    static float need(float peak, float ceiling, float c_log2) noexcept {
        return peak > ceiling ? c_log2 - dsp::fast_log2(peak) : 0.0f;
    }

    float dither(float x, int c, int bits, int shape) noexcept {
        const float q = std::exp2(static_cast<float>(1 - bits)); // one LSB of a full-scale +-1 word
        float fb = 0.0f;
        switch (shape) {
        case 1: fb = 0.5f * err_[c][0]; break;
        case 2: fb = err_[c][0]; break;
        case 3: fb = 2.0f * err_[c][0] - err_[c][1]; break;
        default: break;
        }
        const float v = x - fb;
        const float tpdf = (noise_.uniform() + noise_.uniform()) * q;
        const float y = std::clamp(std::nearbyint((v + tpdf) / q) * q, -1.0f, 1.0f - q);
        err_[c][1] = err_[c][0];
        err_[c][0] = std::clamp(y - v, -4.0f * q, 4.0f * q);
        return y;
    }

    double sr_ = 48000.0;
    uint32_t la_base_ = 240, factor_ = 0;
    dsp::Oversampler os_[2];
    dsp::TruePeak4x tp_[2], tp_lr_[2];
    dsp::FixedDelay audio_[2], det_[2], slow_delay_[2], fast_delay_[2], dry_[2];
    dsp::LookaheadGain fast_[2], guard_;
    dsp::FixedDelay guard_audio_[2], guard_det_[2];
    dsp::TruePeak4x guard_tp_[2];
    float guard_rel_ = 0.0f;
    dsp::Smoother push_;
    dsp::Noise noise_;
    float slow_[2] = {}, slow_long_[2] = {};
    float err_[2][2] = {};
    bool primed_ = false;
};

std::unique_ptr<BuiltinProcessor> create_mastering_limiter() { return std::make_unique<MasteringLimiter>(); }

} // namespace

extern const BuiltinDescriptor kMasteringLimiterDescriptor;
constexpr float kIdleReadouts[] = {-120.0f, -120.0f, 0.0f};
// Rust definition defaults, in parameter order.
constexpr float kDefaults[] = {0.f, -1.f, 150.f, 1.f, 0.f, 2.f, 10.f, 75.f, 100.f, 0.f, 2.f, 1.f, 0.f, 2.f, 0.f, 0.f, 0.f};
const BuiltinDescriptor kMasteringLimiterDescriptor{"mastering-limiter", kParamIds, kReadoutIds, &create_mastering_limiter, kIdleReadouts, kDefaults};

} // namespace saine
