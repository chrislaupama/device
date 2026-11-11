// Glue Compressor: a bus compressor in the style of a VCA console bus
// compressor, with stepped ratio / attack / release, Auto release, a
// reduction range, detector high-pass, external sidechain, Color
// saturation, makeup, mix and output. Parameters in the order of
// dstudio_core::builtin::glue_compressor (checked by a Rust test).
//
//   detector   own signal or sidechain, high-passed (SC HPF, 12 dB/oct),
//              max(|L|, |R|) in dB (stereo linked)
//   curve      soft knee 4 dB wide, slope 1/ratio; the threshold sits
//              3 * (10 - ratio) / 8 dB lower for the gentler ratios (as the
//              panel draws it); reduction limited to Range
//   ballistics in dB: attack while reduction grows, release while it shrinks;
//              Auto = two release paths (fast 0.1 s, slow 1.2 s with a slow
//              attack) and the deeper one wins, so short peaks recover fast
//              and sustained squeezes slowly
//   color      after the gain cell, at 2x oversampling: blends in an
//              asymmetric tanh stage (even and odd harmonics) driven harder as
//              Color rises (up to +6 dB at 100 %), small-signal gain
//              compensated; 0 % is a straight line
//   output     (dry + mix * (wet * makeup - dry)) * output; the dry path is
//              delayed to match the oversampler (fixed latency, 32 samples)
#include "builtin.hpp"
#include "dsp.hpp"
#include "dsp_dynamics.hpp"

#include <algorithm>
#include <cmath>

namespace dstudio {

namespace {

enum Param : uint32_t { kThreshold, kRatio, kAttack, kRelease, kMakeup, kMix, kColor, kRange, kScHpf, kSidechain, kOutput, kParamCount };
enum Readout : uint32_t { kInDb, kOutDb, kGrDb, kReadoutCount };

constexpr const char* kParamIds[kParamCount] = {"threshold", "ratio", "attack", "release", "makeup", "mix", "color", "range", "sc_hpf", "sidechain", "output_gain"};
constexpr const char* kReadoutIds[kReadoutCount] = {"in_db", "out_db", "gr_db"};

constexpr float kRatios[] = {2.0f, 4.0f, 10.0f};
constexpr float kAttackMs[] = {0.1f, 0.3f, 1.0f, 3.0f, 10.0f, 30.0f};
constexpr float kReleaseMs[] = {100.0f, 300.0f, 600.0f, 1200.0f}; // index 4 = Auto
constexpr float kScHpfHz[] = {0.0f, 60.0f, 90.0f, 150.0f, 300.0f};
constexpr float kKneeDb = 4.0f;
constexpr uint32_t kLatency = dsp::Oversampler::latency(2);

template <std::size_t N>
int pick(float v) noexcept {
    return std::clamp(static_cast<int>(std::lround(v)), 0, static_cast<int>(N) - 1);
}

// x + c * (sat(x) - x), sat(x) = (tanh(d x + a) - tanh(a)) / (d (1 - tanh(a)^2)):
// unity gain for small signals, even harmonics from the bias a, a straight
// line at c = 0.
struct Color {
    float amount = 0.0f, drive = 1.0f, bias = 0.0f, tb = 0.0f, norm = 1.0f;
    void set(float c) noexcept {
        amount = c;
        drive = 1.0f + c;
        bias = 0.2f * c;
        tb = dsp::fast_tanh(bias);
        norm = 1.0f / (drive * (1.0f - tb * tb));
    }
    float operator()(float x) const noexcept { return x + amount * ((dsp::fast_tanh(drive * x + bias) - tb) * norm - x); }
};

class GlueCompressor final : public BuiltinProcessor {
public:
    void prepare(double sample_rate, uint32_t) override {
        sr_ = sample_rate;
        for (auto* s : {&makeup_, &mix_, &output_}) s->set_time(20.0f, sr_);
        a_color_ = dsp::pole(20.0f, sr_);
        for (auto& d : dry_) d.prepare(kLatency);
        a_fast_rel_ = dsp::pole(100.0f, sr_);
        a_slow_rel_ = dsp::pole(1200.0f, sr_);
        a_slow_att_ = dsp::pole(150.0f, sr_);
        hpf_index_ = -1;
        reset();
    }

    void reset() noexcept override {
        for (auto& f : hpf_) f.reset();
        for (auto& o : os_) o.reset();
        for (auto& d : dry_) d.reset();
        gr_ = gr_slow_ = 0.0f;
        primed_ = false;
    }

    uint32_t latency() const noexcept override { return kLatency; }

    void process(float* l, float* r, uint32_t n, const float* p, const BuiltinContext& ctx, float* ro) noexcept override {
        const float ratio = kRatios[pick<3>(p[kRatio])];
        const float threshold = std::clamp(p[kThreshold], -96.0f, 24.0f) - 3.0f * (10.0f - ratio) / 8.0f;
        const float slope = 1.0f / ratio - 1.0f;
        const float range = -std::clamp(p[kRange], 0.0f, 96.0f);
        const float a_att = dsp::pole(kAttackMs[pick<6>(p[kAttack])], sr_);
        const int rel = pick<5>(p[kRelease]);
        const bool auto_release = rel == 4;
        const float a_rel = auto_release ? a_fast_rel_ : dsp::pole(kReleaseMs[rel], sr_);
        const int hpf = pick<5>(p[kScHpf]);
        if (hpf != hpf_index_) {
            hpf_index_ = hpf;
            for (auto& f : hpf_) {
                if (hpf == 0) f.pass(); else f.highpass(static_cast<double>(kScHpfHz[hpf]), sr_);
                f.reset();
            }
        }
        const float makeup = dsp::db_to_gain(std::clamp(p[kMakeup], -48.0f, 48.0f));
        const float mix = std::clamp(p[kMix], 0.0f, 100.0f) * 0.01f;
        const float color = std::clamp(p[kColor], 0.0f, 100.0f) * 0.01f;
        const float output = dsp::db_to_gain(std::clamp(p[kOutput], -48.0f, 48.0f));
        const bool use_sc = p[kSidechain] >= 0.5f && ctx.sc_l != nullptr;
        if (!primed_) {
            makeup_.snap(makeup);
            mix_.snap(mix);
            output_.snap(output);
            color_now_ = color;
            primed_ = true;
        }
        // Color moves once per block, smoothed across blocks (the curve has no zipper at this rate).
        color_now_ = color + std::pow(a_color_, static_cast<float>(n)) * (color_now_ - color);
        Color shape;
        shape.set(color_now_);

        const float* scl = use_sc ? ctx.sc_l : l;
        const float* scr = use_sc ? ctx.sc_r : r;
        float in_peak = 0.0f, out_peak = 0.0f, gr_min = 0.0f;
        float gr = gr_, gr_slow = gr_slow_;
        for (uint32_t i = 0; i < n; ++i) {
            const float dl = l[i], dr = r[i];
            in_peak = std::max(in_peak, std::max(std::fabs(dl), std::fabs(dr)));
            const float x = std::max(std::fabs(hpf_[0].tick(scl[i])), std::fabs(hpf_[1].tick(scr[i])));
            const float over = dsp::fast_gain_to_db(x) - threshold;
            float target = 0.0f;
            if (2.0f * over > -kKneeDb) {
                const float k = over + 0.5f * kKneeDb;
                target = 2.0f * over < kKneeDb ? slope * k * k / (2.0f * kKneeDb) : slope * over;
                target = std::max(target, range);
            }
            gr = target + (target < gr ? a_att : a_rel) * (gr - target);
            float g_db = gr;
            if (auto_release) {
                gr_slow = target + (target < gr_slow ? a_slow_att_ : a_slow_rel_) * (gr_slow - target);
                g_db = std::min(gr, gr_slow);
            }
            gr_min = std::min(gr_min, g_db);
            const float g = g_db < 0.0f ? dsp::fast_db_to_gain(g_db) : 1.0f;
            float up_l[2], up_r[2];
            os_[0].up(dl * g, up_l, 2);
            os_[1].up(dr * g, up_r, 2);
            for (int k = 0; k < 2; ++k) {
                up_l[k] = shape(up_l[k]);
                up_r[k] = shape(up_r[k]);
            }
            const float wl = os_[0].down(up_l, 2), wr = os_[1].down(up_r, 2);
            const float xl = dry_[0].process(dl, kLatency), xr = dry_[1].process(dr, kLatency);
            const float mk = makeup_.next(makeup), m = mix_.next(mix), o = output_.next(output);
            const float yl = (xl + m * (wl * mk - xl)) * o;
            const float yr = (xr + m * (wr * mk - xr)) * o;
            l[i] = yl;
            r[i] = yr;
            out_peak = std::max(out_peak, std::max(std::fabs(yl), std::fabs(yr)));
        }
        for (auto& f : hpf_) f.flush_denormals();
        if (gr > -1e-6f) gr = 0.0f;
        if (gr_slow > -1e-6f) gr_slow = 0.0f;
        gr_ = gr;
        gr_slow_ = gr_slow;
        ro[kInDb] = dsp::gain_to_db(in_peak);
        ro[kOutDb] = dsp::gain_to_db(out_peak);
        ro[kGrDb] = gr_min;
    }

private:
    double sr_ = 48000.0;
    dsp::Biquad hpf_[2];
    int hpf_index_ = -1;
    dsp::Oversampler os_[2];
    dsp::FixedDelay dry_[2];
    dsp::Smoother makeup_, mix_, output_;
    float a_color_ = 0.0f, color_now_ = 0.0f;
    float a_fast_rel_ = 0.0f, a_slow_rel_ = 0.0f, a_slow_att_ = 0.0f;
    float gr_ = 0.0f, gr_slow_ = 0.0f;
    bool primed_ = false;
};

std::unique_ptr<BuiltinProcessor> create_glue() { return std::make_unique<GlueCompressor>(); }

} // namespace

extern const BuiltinDescriptor kGlueCompressorDescriptor;
constexpr float kIdleReadouts[] = {-120.0f, -120.0f, 0.0f};
// Rust definition defaults, in parameter order.
constexpr float kDefaults[] = {-15.f, 1.f, 5.f, 4.f, 0.f, 100.f, 50.f, 24.f, 0.f, 0.f, 0.f};
const BuiltinDescriptor kGlueCompressorDescriptor{"glue-compressor", kParamIds, kReadoutIds, &create_glue, kIdleReadouts, kDefaults};

} // namespace dstudio
