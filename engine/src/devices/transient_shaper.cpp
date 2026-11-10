// Transient Shaper: level-independent attack / sustain shaping from the
// difference of envelope followers, with a parallel mix, output gain and an
// optional soft clipper at 0 dBFS. Parameters in the order of
// saine_core::builtin::transient_shaper (checked by a Rust test).
//
//   detector  max(|L|, |R|), floored at a level set by Sensitivity
//   envelope  e: 0.3 ms rise, 40 ms fall (smooth enough for low notes)
//   attack    e minus a copy of e that rises over Attack Time and falls with
//             it at once: positive only at the start of a hit, for about
//             Attack Time (a dB difference, so it does not depend on level)
//   sustain   a copy of e that rises with it at once and falls over Sustain
//             Time (Full = 10 s), minus e: grows through the tail of a hit
//   gain      Sensitivity scales both: attack% * 0.5 * diff + sustain% * 0.6 * diff,
//             limited to -30..+24 dB, smoothed over 0.5 ms
//   output    (dry + mix * (shaped - dry)) * output, then (Clip on) a soft
//             clipper (linear to -1.4 dBFS, bending into 0 dBFS) at 2x
//             oversampling. The oversampler always runs, so latency is a
//             fixed 32 samples and toggling Clip never clicks.
//   readouts  output peak, pre-clip peak and applied gain (largest
//             magnitude), each held for 1/30 s so a poll never misses a hit.
//   scope     the same three per column, 750 columns a second (64 samples at
//             48 kHz), as the reference app's gain scope reads them. Each analysis frame
//             is the column count so far (two floats: count >> 20 and
//             count & 0xFFFFF) and then the newest 256 columns, oldest first,
//             3 floats each; the UI merges frames by that count.
#include "builtin.hpp"
#include "dsp.hpp"
#include "dsp_dynamics.hpp"

#include <algorithm>
#include <cmath>

namespace saine {

namespace {

enum Param : uint32_t { kAttack, kSustain, kSensitivity, kOutput, kMix, kAttackTime, kSustainTime, kClip, kParamCount };
enum Readout : uint32_t { kOutPeak, kPrePeak, kGainDb, kReadoutCount };

constexpr const char* kParamIds[kParamCount] = {"attack", "sustain", "sensitivity", "output_gain", "mix", "attack_time", "sustain_time", "clip"};
constexpr const char* kReadoutIds[kReadoutCount] = {"out_peak", "pre_peak", "gain_db"};
constexpr uint32_t kLatency = dsp::Oversampler::latency(2);
constexpr float kClipKnee = 0.85f; // -1.4 dBFS
constexpr double kScopeRate = 750.0;   // columns per second
constexpr uint32_t kScopeWindow = 256; // columns per analysis frame (341 ms)

class TransientShaper final : public BuiltinProcessor {
public:
    void prepare(double sample_rate, uint32_t) override {
        sr_ = sample_rate;
        rise_ = dsp::pole(0.3f, sr_);
        fall_ = dsp::pole(40.0f, sr_);
        gain_smooth_ = dsp::pole(0.5f, sr_);
        hold_ = static_cast<uint32_t>(sr_ / 30.0);
        col_len_ = sr_ / kScopeRate;
        for (auto* s : {&mix_, &output_}) s->set_time(20.0f, sr_);
        reset();
    }

    void reset() noexcept override {
        env_ = slow_ = hold_env_ = 0.0f;
        gain_db_ = 0.0f;
        for (auto& o : os_) o.reset();
        out_hold_.reset();
        pre_hold_.reset();
        gain_hold_.reset();
        primed_ = false;
        col_out_ = col_pre_ = col_gain_ = 0.0f;
        col_frac_ = 0.0;
        next_column();
    }

    uint32_t latency() const noexcept override { return kLatency; }
    uint32_t analysis_capacity() const noexcept override { return 2 + 3 * kScopeWindow; }

    void process(float* l, float* r, uint32_t n, const float* p, const BuiltinContext& ctx, float* ro) noexcept override {
        const float attack = std::clamp(p[kAttack], -100.0f, 100.0f) * 0.01f;
        const float sustain = std::clamp(p[kSustain], -100.0f, 100.0f) * 0.01f;
        const float sens = std::clamp(p[kSensitivity], 0.0f, 100.0f) * 0.01f;
        const float k = std::exp2((sens - 0.5f) * 2.4f);
        const float floor = dsp::db_to_gain(-50.0f - 40.0f * sens);
        const float a_att_slow = dsp::pole(std::clamp(p[kAttackTime], 0.5f, 1000.0f), sr_);
        const float sus_ms = p[kSustainTime] >= 1999.0f ? 10000.0f : std::clamp(p[kSustainTime], 1.0f, 10000.0f);
        const float fall_sus = dsp::pole(sus_ms, sr_);
        const float ka = 0.5f * k * attack, ks = 0.6f * k * sustain;
        const float mix = std::clamp(p[kMix], 0.0f, 100.0f) * 0.01f;
        const float output = dsp::db_to_gain(std::clamp(p[kOutput], -48.0f, 48.0f));
        const bool clip = p[kClip] >= 0.5f;
        if (!primed_) {
            mix_.snap(mix);
            output_.snap(output);
            env_ = slow_ = hold_env_ = floor; // start at rest: no attack burst on the first sample
            primed_ = true;
        }
        const bool shaping = ka != 0.0f || ks != 0.0f;

        float out_peak = 0.0f, pre_peak = 0.0f, gain_max = 0.0f;
        float env = env_, slow = slow_, tail = hold_env_, gdb = gain_db_;
        for (uint32_t i = 0; i < n; ++i) {
            const float xl = l[i], xr = r[i];
            const float x = std::max(floor, std::max(std::fabs(xl), std::fabs(xr)));
            env = x + (x > env ? rise_ : fall_) * (env - x);
            slow = env < slow ? env : env + a_att_slow * (slow - env); // <= env
            tail = env > tail ? env : env + fall_sus * (tail - env);   // >= env
            float target = 0.0f;
            if (shaping) {
                const float att_diff = std::min(dsp::kDbPerLog2 * dsp::fast_log2(env / slow), 24.0f);
                const float sus_diff = std::min(dsp::kDbPerLog2 * dsp::fast_log2(tail / env), 30.0f);
                target = std::clamp(ka * att_diff + ks * sus_diff, -30.0f, 24.0f);
            }
            gdb = target + gain_smooth_ * (gdb - target);
            if (std::fabs(gdb) > std::fabs(gain_max)) gain_max = gdb;
            const float g = gdb != 0.0f ? dsp::fast_db_to_gain(gdb) : 1.0f;
            const float m = mix_.next(mix), o = output_.next(output);
            const float wl = (xl + m * (xl * g - xl)) * o;
            const float wr = (xr + m * (xr * g - xr)) * o;
            const float pre = std::max(std::fabs(wl), std::fabs(wr));
            pre_peak = std::max(pre_peak, pre);
            float ul[2], ur[2];
            os_[0].up(wl, ul, 2);
            os_[1].up(wr, ur, 2);
            if (clip) {
                for (int j = 0; j < 2; ++j) {
                    ul[j] = dsp::soft_clip(ul[j], kClipKnee, 1.0f);
                    ur[j] = dsp::soft_clip(ur[j], kClipKnee, 1.0f);
                }
            }
            const float yl = os_[0].down(ul, 2), yr = os_[1].down(ur, 2);
            l[i] = yl;
            r[i] = yr;
            const float out = std::max(std::fabs(yl), std::fabs(yr));
            out_peak = std::max(out_peak, out);
            col_out_ = std::max(col_out_, out);
            col_pre_ = std::max(col_pre_, pre);
            if (std::fabs(gdb) > std::fabs(col_gain_)) col_gain_ = gdb;
            if (--col_left_ == 0) end_column();
        }
        if (std::fabs(gdb) < 1e-6f) gdb = 0.0f;
        env_ = env, slow_ = slow, hold_env_ = tail, gain_db_ = gdb;
        ro[kOutPeak] = out_hold_.update(out_peak, n, hold_);
        ro[kPrePeak] = pre_hold_.update(pre_peak, n, hold_);
        ro[kGainDb] = gain_hold_.update(gain_max, n, hold_);
        if (ctx.analysis != nullptr && total_ != published_) publish(*ctx.analysis);
    }

private:
    // Starts the next scope column: sr / 750 samples, the fraction carried over.
    void next_column() noexcept {
        const double len = col_len_ + col_frac_;
        col_left_ = std::max<uint32_t>(1, static_cast<uint32_t>(len));
        col_frac_ = len - col_left_;
    }

    void end_column() noexcept {
        float* c = cols_ + 3 * (total_ % kScopeWindow);
        c[0] = col_out_, c[1] = col_pre_, c[2] = std::fabs(col_gain_) < 1e-6f ? 0.0f : col_gain_;
        ++total_;
        col_out_ = col_pre_ = col_gain_ = 0.0f;
        next_column();
    }

    void publish(AnalysisChannel& a) noexcept {
        const uint64_t n = std::min<uint64_t>(total_, kScopeWindow);
        float* f = a.begin();
        f[0] = static_cast<float>(total_ >> 20);
        f[1] = static_cast<float>(total_ & 0xFFFFFu);
        for (uint64_t k = 0; k < n; ++k) {
            const float* c = cols_ + 3 * ((total_ - n + k) % kScopeWindow);
            std::copy(c, c + 3, f + 2 + 3 * k);
        }
        a.publish(static_cast<uint32_t>(2 + 3 * n));
        published_ = total_;
    }

    double sr_ = 48000.0;
    float rise_ = 0.0f, fall_ = 0.0f, gain_smooth_ = 0.0f;
    float env_ = 0.0f, slow_ = 0.0f, hold_env_ = 0.0f, gain_db_ = 0.0f;
    uint32_t hold_ = 1600;
    dsp::Oversampler os_[2];
    dsp::Smoother mix_, output_;
    dsp::HeldPeak out_hold_, pre_hold_;
    dsp::HeldSigned gain_hold_;
    bool primed_ = false;
    double col_len_ = 64.0, col_frac_ = 0.0;
    uint32_t col_left_ = 64;
    float col_out_ = 0.0f, col_pre_ = 0.0f, col_gain_ = 0.0f;
    float cols_[3 * kScopeWindow] = {};
    uint64_t total_ = 0, published_ = 0;
};

std::unique_ptr<BuiltinProcessor> create_transient_shaper() { return std::make_unique<TransientShaper>(); }

} // namespace

extern const BuiltinDescriptor kTransientShaperDescriptor;
constexpr float kIdleReadouts[] = {0.0f, 0.0f, 0.0f};
// Rust definition defaults, in parameter order.
constexpr float kDefaults[] = {0.f, 0.f, 50.f, 0.f, 100.f, 30.f, 2000.f, 0.f};
const BuiltinDescriptor kTransientShaperDescriptor{"transient-shaper", kParamIds, kReadoutIds, &create_transient_shaper, kIdleReadouts, kDefaults};

} // namespace saine
