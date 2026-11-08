// Compressor: feed-forward, stereo-linked, soft knee, four detector
// characters (Clean, Opto, Punch, Smooth), make-up gain, dry/wet mix,
// output gain and an external sidechain. Parameters in the order of
// saine_core::builtin::compressor (checked by a Rust test).
//
// Signal flow per sample:
//   detector input  x = max(|L|, |R|) (or the sidechain, scaled by its gain)
//   level           Clean / Punch: peak; Opto / Smooth: RMS (10 / 25 ms)
//   static curve    soft knee of width W around threshold T, slope 1/R above
//   gain reduction  smoothed in dB: attack when reduction grows, release
//                   when it shrinks (character-dependent times, see below)
//   output          (dry + mix * (dry * g(gr + makeup) - dry)) * g(output)
#include "builtin.hpp"
#include "dsp.hpp"

#include <algorithm>
#include <cmath>

namespace saine {

namespace {

enum Param : uint32_t {
    kThreshold,
    kRatio,
    kAttack,
    kRelease,
    kKnee,
    kMakeup,
    kMix,
    kStyle,
    kSidechainEnable,
    kOutput,
    kSidechainGain,
    kParamCount
};
enum Readout : uint32_t { kInDb, kOutDb, kGrDb, kReadoutCount };
enum Style : int { kClean, kOpto, kPunch, kSmooth };

constexpr const char* kParamIds[kParamCount] = {"threshold", "ratio", "attack", "release", "knee", "makeup_gain",
                                                "mix", "style", "sidechainEnable", "output_gain", "sidechain_gain"};
constexpr const char* kReadoutIds[kReadoutCount] = {"in_db", "out_db", "gr_db"};

// Static curve: gain reduction in dB (<= 0) for a level in dB.
inline float reduction(float x, float t, float slope, float knee) noexcept {
    const float over = x - t;
    if (knee > 0.0f && 2.0f * over > -knee && 2.0f * over < knee) {
        const float k = over + 0.5f * knee;
        return slope * k * k / (2.0f * knee);
    }
    return over > 0.0f ? slope * over : 0.0f;
}

class Compressor final : public BuiltinProcessor {
public:
    void prepare(double sample_rate, uint32_t) override {
        sr_ = sample_rate;
        gain_smooth_ms_ = 20.0f;
        makeup_.set_time(gain_smooth_ms_, sr_);
        mix_.set_time(gain_smooth_ms_, sr_);
        output_.set_time(gain_smooth_ms_, sr_);
        rms_fast_ = dsp::pole(10.0f, sr_);
        rms_slow_ = dsp::pole(25.0f, sr_);
    }

    void reset() noexcept override {
        gr_ = 0.0f;
        ms_ = 0.0f;
        primed_ = false;
    }

    void process(float* l, float* r, uint32_t n, const float* p, const BuiltinContext& ctx, float* ro) noexcept override {
        const float threshold = std::clamp(p[kThreshold], -96.0f, 24.0f);
        const float ratio = std::clamp(p[kRatio], 1.0f, 100.0f);
        const float attack = std::clamp(p[kAttack], 0.01f, 5000.0f);
        const float release = std::clamp(p[kRelease], 1.0f, 10000.0f);
        float knee = std::clamp(p[kKnee], 0.0f, 48.0f);
        const float makeup = dsp::db_to_gain(std::clamp(p[kMakeup], -48.0f, 48.0f));
        const float mix = std::clamp(p[kMix], 0.0f, 100.0f) * 0.01f;
        const float output = dsp::db_to_gain(std::clamp(p[kOutput], -48.0f, 48.0f));
        const int style = std::clamp(static_cast<int>(std::lround(p[kStyle])), 0, 3);
        const bool use_sc = p[kSidechainEnable] >= 0.5f && ctx.sc_l != nullptr;
        const float sc_gain = dsp::db_to_gain(std::clamp(p[kSidechainGain], -48.0f, 48.0f));

        // Character: detector, time scaling, knee.
        float att_ms = attack, rel_ms = release, rel_slow_ms = release;
        float rms_a = 0.0f; // 0 = peak detector
        switch (style) {
        case kOpto: // smooth onset, release slows down the more it compressed
            att_ms = std::max(attack, 5.0f);
            rel_ms = 0.5f * release;
            rel_slow_ms = 2.0f * release;
            rms_a = rms_fast_;
            break;
        case kPunch: // lets the transient through, recovers fast
            att_ms = 1.6f * attack;
            rel_ms = rel_slow_ms = 0.6f * release;
            break;
        case kSmooth: // slow, gentle, wide knee
            att_ms = 2.0f * attack;
            rel_ms = rel_slow_ms = 1.6f * release;
            knee += 6.0f;
            rms_a = rms_slow_;
            break;
        default: break;
        }
        const float a_att = dsp::pole(att_ms, sr_);
        const float a_rel = dsp::pole(rel_ms, sr_);
        const float a_rel_slow = dsp::pole(rel_slow_ms, sr_);
        const float slope = 1.0f / ratio - 1.0f;
        if (!primed_) { // first block: no fade-in of the static gains
            makeup_.snap(makeup);
            mix_.snap(mix);
            output_.snap(output);
            primed_ = true;
        }

        float in_peak = 0.0f, out_peak = 0.0f, gr_min = 0.0f;
        float gr = gr_, ms = ms_;
        const float* scl = use_sc ? ctx.sc_l : l;
        const float* scr = use_sc ? ctx.sc_r : r;
        const float det_gain = use_sc ? sc_gain : 1.0f;
        for (uint32_t i = 0; i < n; ++i) {
            const float dl = l[i], dr = r[i];
            in_peak = std::max(in_peak, std::max(std::fabs(dl), std::fabs(dr)));
            const float x = std::max(std::fabs(scl[i]), std::fabs(scr[i])) * det_gain;
            float level_db;
            if (rms_a > 0.0f) {
                ms = x * x + rms_a * (ms - x * x);
                level_db = dsp::power_to_db(ms);
            } else {
                level_db = dsp::gain_to_db(x);
            }
            const float target = reduction(level_db, threshold, slope, knee);
            float a;
            if (target < gr) {
                a = a_att;
            } else if (style == kOpto) { // program dependent: deeper reduction -> slower release
                const float w = std::min(1.0f, -gr * (1.0f / 18.0f));
                a = a_rel + w * (a_rel_slow - a_rel);
            } else {
                a = a_rel;
            }
            gr = target + a * (gr - target);
            gr_min = std::min(gr_min, gr);
            const float g = dsp::db_to_gain(gr) * makeup_.next(makeup);
            const float m = mix_.next(mix);
            const float o = output_.next(output);
            const float yl = (dl + m * (dl * g - dl)) * o;
            const float yr = (dr + m * (dr * g - dr)) * o;
            l[i] = yl;
            r[i] = yr;
            out_peak = std::max(out_peak, std::max(std::fabs(yl), std::fabs(yr)));
        }
        if (ms < 1e-20f) ms = 0.0f; // no denormals in the RMS state
        if (gr > -1e-6f) gr = 0.0f;
        gr_ = gr;
        ms_ = ms;
        ro[kInDb] = dsp::gain_to_db(in_peak);
        ro[kOutDb] = dsp::gain_to_db(out_peak);
        ro[kGrDb] = gr_min;
    }

private:
    double sr_ = 48000.0;
    float gain_smooth_ms_ = 20.0f;
    float rms_fast_ = 0.0f, rms_slow_ = 0.0f;
    float gr_ = 0.0f; // smoothed gain reduction, dB (<= 0)
    float ms_ = 0.0f; // RMS detector state (mean square)
    bool primed_ = false;
    dsp::Smoother makeup_, mix_, output_;
};

std::unique_ptr<BuiltinProcessor> create_compressor() { return std::make_unique<Compressor>(); }

} // namespace

extern const BuiltinDescriptor kCompressorDescriptor;
constexpr float kIdleReadouts[] = {-120.0f, -120.0f, 0.0f}; // silent in / out, no reduction
const BuiltinDescriptor kCompressorDescriptor{"compressor", kParamIds, kReadoutIds, &create_compressor, kIdleReadouts};

} // namespace saine
