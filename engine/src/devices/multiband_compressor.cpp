// Multiband Compressor: three bands (Linkwitz-Riley 24 dB/oct crossovers),
// each compressed downwards above its threshold and upwards below its floor.
// Parameters in the order of saine_core::builtin::multiband_compressor
// (checked by a Rust test).
//
// Signal flow per sample:
//   input gain -> split: low = LR4 low-pass(f1) then a 2nd-order all-pass at
//   f2 (phase match), mid = LR4 high-pass(f1) -> LR4 low-pass(f2), high =
//   LR4 high-pass(f1) -> LR4 high-pass(f2); the three unprocessed bands sum
//   to an all-pass of the input, used as the dry signal so Mix never combs.
//   per band: band input gain, stereo-linked peak envelope (attack / release
//   x Time), level in dB, the band law (same as saine_core multiband_gain:
//   downward above the threshold, upward below the floor, 0.5 dB knees,
//   limited to -40..+36 dB, x Depth; upward fades out for levels under
//   -80 dB so silence is not lifted), band makeup.
//   sum of bands -> mix with the dry sum -> output gain.
// The crossovers run as four-lane biquads and the band detectors / gain laws
// as one four-lane pass (three bands + one idle lane). No latency.
#include "builtin.hpp"
#include "dsp.hpp"
#include "dynamics_kit.hpp"

#include <algorithm>
#include <cmath>

namespace saine {

namespace {

using dsp::F4;

constexpr uint32_t kBandParams = 5, kBandUpParams = 3;
enum Param : uint32_t {
    kLowCrossover,
    kHighCrossover,
    kBand0 = 2, // per band b: threshold, ratio, attack, release, makeup at kBand0 + 5 b
    kMix = 17,
    kOutput,
    kBandUp0 = 19, // per band b: input, up threshold, up ratio at kBandUp0 + 3 b
    kInput = 28,
    kTime,
    kUpward,
    kDownward,
    kDepth,
    kParamCount
};
enum BandParam : uint32_t { kThr, kRatio, kAttack, kRelease, kMakeup };
enum BandUpParam : uint32_t { kBandInput, kFloor, kLift };
enum Readout : uint32_t { kInDb, kOutDb, kLevel0, kGain0 = kLevel0 + 3, kReadoutCount = kGain0 + 3 };

constexpr const char* kParamIds[kParamCount] = {
    "low_crossover", "high_crossover", "low_threshold",   "low_ratio",         "low_attack",    "low_release",       "low_makeup",
    "mid_threshold", "mid_ratio",      "mid_attack",      "mid_release",       "mid_makeup",    "high_threshold",    "high_ratio",
    "high_attack",   "high_release",   "high_makeup",     "mix",               "output_gain",   "low_input",         "low_up_threshold",
    "low_up_ratio",  "mid_input",      "mid_up_threshold", "mid_up_ratio",     "high_input",    "high_up_threshold", "high_up_ratio",
    "input_gain",    "time",           "upward",          "downward",          "depth"};
constexpr const char* kReadoutIds[kReadoutCount] = {"in_db",       "out_db",       "low_level_db", "mid_level_db",
                                                    "high_level_db", "low_gain_db", "mid_gain_db",  "high_gain_db"};
constexpr float kLevelFloorDb = -100.0f;

// One-sided ramp with a 0.5 dB soft knee (four lanes).
inline F4 ramp(F4 o) noexcept {
    F4 y;
    for (int i = 0; i < 4; ++i) {
        const float x = o[i];
        const float t = x + 0.25f;
        y[i] = x <= -0.25f ? 0.0f : x >= 0.25f ? x : t * t;
    }
    return y;
}

class MultibandCompressor final : public BuiltinProcessor {
public:
    void prepare(double sample_rate, uint32_t) override {
        sr_ = sample_rate;
        input_.set_time(20.0f, sr_);
        output_.set_time(20.0f, sr_);
        mix_.set_time(20.0f, sr_);
        law_pole_ = dsp::pole(20.0f, sr_);
        f1_ = f2_ = -1.0f;
    }

    void reset() noexcept override {
        split_a_.reset();
        split_b_.reset();
        split_c_.reset();
        split_d_.reset();
        split_e_.reset();
        env_ = F4{};
        primed_ = false;
    }

    void process(float* l, float* r, uint32_t n, const float* p, const BuiltinContext&, float* ro) noexcept override {
        update_crossovers(std::clamp(p[kLowCrossover], 10.0f, 2000.0f), std::clamp(p[kHighCrossover], 500.0f, 22000.0f));
        const float time = std::clamp(p[kTime], 1.0f, 2000.0f) * 0.01f;
        const float downward = std::clamp(p[kDownward], 0.0f, 400.0f) * 0.01f;
        const float upward = std::clamp(p[kUpward], 0.0f, 400.0f) * 0.01f;
        const float depth = std::clamp(p[kDepth], 0.0f, 100.0f) * 0.01f;
        F4 thr{}, floor{}, down{}, up{}, att{}, rel{}, makeup{}, band_in{};
        for (int b = 0; b < 3; ++b) {
            const float* q = p + kBand0 + kBandParams * static_cast<uint32_t>(b);
            const float* u = p + kBandUp0 + kBandUpParams * static_cast<uint32_t>(b);
            const float ratio = std::clamp(q[kRatio], 1.0f, 100.0f);
            const float lift = std::clamp(u[kLift], 1.0f, 100.0f);
            thr[b] = std::clamp(q[kThr], -120.0f, 24.0f);
            floor[b] = std::clamp(u[kFloor], -120.0f, 24.0f);
            down[b] = std::min(1.0f, (1.0f - 1.0f / ratio) * downward) * depth;
            up[b] = (1.0f - 1.0f / lift) * upward * depth;
            att[b] = dsp::pole(std::clamp(q[kAttack], 0.01f, 5000.0f) * time, sr_);
            rel[b] = dsp::pole(std::clamp(q[kRelease], 1.0f, 20000.0f) * time, sr_);
            makeup[b] = dsp::db_to_gain(std::clamp(q[kMakeup], -48.0f, 48.0f));
            band_in[b] = dsp::db_to_gain(std::clamp(u[kBandInput], -48.0f, 48.0f));
        }
        const float in_gain = dsp::db_to_gain(std::clamp(p[kInput], -48.0f, 48.0f));
        const float out_gain = dsp::db_to_gain(std::clamp(p[kOutput], -48.0f, 48.0f));
        const float mix = std::clamp(p[kMix], 0.0f, 100.0f) * 0.01f;
        if (!primed_) {
            input_.snap(in_gain);
            output_.snap(out_gain);
            mix_.snap(mix);
            down_s_ = down, up_s_ = up, makeup_s_ = makeup, band_in_s_ = band_in;
            depth_s_ = depth;
            primed_ = true;
        }
        // Band law amounts, depth and per-band gains glide (20 ms) so edits and automation never step.
        const F4 kp = F4::all(law_pole_);
        F4 down_s = down_s_, up_s = up_s_, makeup_s = makeup_s_, band_in_s = band_in_s_;
        float depth_s = depth_s_;
        float in_peak = 0.0f;
        F4 level_max = F4::all(kLevelFloorDb), gain_ext{};
        F4 env = env_;
        for (uint32_t i = 0; i < n; ++i) {
            const float xl = l[i], xr = r[i];
            in_peak = std::max(in_peak, std::max(std::fabs(xl), std::fabs(xr)));
            const float gi = input_.next(in_gain);
            down_s = down + kp * (down_s - down);
            up_s = up + kp * (up_s - up);
            makeup_s = makeup + kp * (makeup_s - makeup);
            band_in_s = band_in + kp * (band_in_s - band_in);
            depth_s = depth + law_pole_ * (depth_s - depth);
            // Crossovers: lanes [L, R, L, R]. The input gain applies after the split so the
            // dry sum (an all-pass of the input) never carries gain changes.
            const F4 a = split_a_.process(F4{{xl, xr, xl, xr}}); // LP1a | HP1a
            const F4 lohi = split_b_.process(a);                 // LP1b | HP1b -> low L R, high L R
            const F4 c = split_c_.process(lohi);                 // AP2 (low) | LP2a (high)
            const F4 dd = split_d_.process(F4{{c[2], c[3], lohi[2], lohi[3]}}); // LP2b | HP2a
            const F4 e = split_e_.process(F4{{dd[2], dd[3], 0.0f, 0.0f}});        // HP2b
            const F4 bl{{c[0], dd[0], e[0], 0.0f}}; // low, mid, high (left)
            const F4 br{{c[1], dd[1], e[1], 0.0f}};
            const F4 gin = band_in_s * F4::all(gi);
            const F4 sl = bl * gin, sr = br * gin;
            // Envelopes: stereo-linked peak, attack / release in the linear domain.
            const F4 x = max4(F4{{std::fabs(sl[0]), std::fabs(sl[1]), std::fabs(sl[2]), 0.0f}},
                              F4{{std::fabs(sr[0]), std::fabs(sr[1]), std::fabs(sr[2]), 0.0f}});
            F4 coef;
            for (int b = 0; b < 4; ++b) coef[b] = x[b] > env[b] ? att[b] : rel[b];
            env = x + coef * (env - x);
            F4 lvl;
            for (int b = 0; b < 4; ++b) lvl[b] = dsp::fast_gain_to_db(env[b] + 1e-12f);
            // Band law: downward above the threshold, upward below the floor.
            F4 g = up_s * ramp(floor - lvl) - down_s * ramp(lvl - thr);
            g = min4(max4(g, F4::all(-40.0f * depth_s)), F4::all(36.0f * depth_s));
            for (int b = 0; b < 4; ++b) {
                if (g[b] > 0.0f) g[b] *= std::clamp((lvl[b] - kLevelFloorDb) * 0.05f, 0.0f, 1.0f);
                if (std::fabs(g[b]) > std::fabs(gain_ext[b])) gain_ext[b] = g[b];
            }
            level_max = max4(level_max, lvl);
            F4 lin;
            for (int b = 0; b < 4; ++b) lin[b] = dsp::fast_db_to_gain(g[b]);
            const F4 k = lin * makeup_s;
            const F4 wl = sl * k, wr = sr * k;
            const float wet_l = wl[0] + wl[1] + wl[2], wet_r = wr[0] + wr[1] + wr[2];
            const float dry_l = bl[0] + bl[1] + bl[2], dry_r = br[0] + br[1] + br[2];
            const float m = mix_.next(mix), o = output_.next(out_gain);
            l[i] = (dry_l + m * (wet_l - dry_l)) * o;
            r[i] = (dry_r + m * (wet_r - dry_r)) * o;
        }
        for (int b = 0; b < 4; ++b)
            if (env[b] < 1e-20f) env[b] = 0.0f;
        env_ = env;
        down_s_ = down_s, up_s_ = up_s, makeup_s_ = makeup_s, band_in_s_ = band_in_s;
        depth_s_ = depth_s;
        split_a_.flush();
        split_b_.flush();
        split_c_.flush();
        split_d_.flush();
        split_e_.flush();
        ro[kInDb] = dsp::gain_to_db(in_peak);
        ro[kOutDb] = dsp::gain_to_db(dsp::stereo_peak(l, r, n));
        for (uint32_t b = 0; b < 3; ++b) {
            ro[kLevel0 + b] = std::max(kLevelFloorDb, level_max[static_cast<int>(b)]);
            ro[kGain0 + b] = gain_ext[static_cast<int>(b)];
        }
    }

private:
    void update_crossovers(float f1, float f2) noexcept {
        if (f1 == f1_ && f2 == f2_) return;
        f1_ = f1, f2_ = f2;
        constexpr float q = 0.70710678f;
        const auto lp1 = dsp::design_biquad(dsp::BiquadKind::Lowpass, f1, q, 0.0f, sr_);
        const auto hp1 = dsp::design_biquad(dsp::BiquadKind::Highpass, f1, q, 0.0f, sr_);
        const auto lp2 = dsp::design_biquad(dsp::BiquadKind::Lowpass, f2, q, 0.0f, sr_);
        const auto hp2 = dsp::design_biquad(dsp::BiquadKind::Highpass, f2, q, 0.0f, sr_);
        const auto ap2 = dsp::design_biquad(dsp::BiquadKind::Allpass, f2, q, 0.0f, sr_);
        for (int c = 0; c < 2; ++c) {
            split_a_.set(c, lp1);
            split_a_.set(c + 2, hp1);
            split_b_.set(c, lp1);
            split_b_.set(c + 2, hp1);
            split_c_.set(c, ap2);
            split_c_.set(c + 2, lp2);
            split_d_.set(c, lp2);
            split_d_.set(c + 2, hp2);
            split_e_.set(c, hp2);
            split_e_.set(c + 2, dsp::BiquadCoefs{});
        }
    }

    double sr_ = 48000.0;
    float f1_ = -1.0f, f2_ = -1.0f;
    dsp::Biquad4 split_a_, split_b_, split_c_, split_d_, split_e_;
    F4 env_{};
    F4 down_s_{}, up_s_{}, makeup_s_{}, band_in_s_{};
    float depth_s_ = 0.0f, law_pole_ = 0.0f;
    dsp::Smoother input_, output_, mix_;
    bool primed_ = false;
};

std::unique_ptr<BuiltinProcessor> create_multiband_compressor() { return std::make_unique<MultibandCompressor>(); }

} // namespace

extern const BuiltinDescriptor kMultibandCompressorDescriptor;
constexpr float kIdleReadouts[] = {-120.0f, -120.0f, -100.0f, -100.0f, -100.0f, 0.0f, 0.0f, 0.0f};
// Defaults of saine_core::builtin::multiband_compressor, registry order.
constexpr float kDefaults[kParamCount] = {120.f, 2500.f, -18.f, 2.5f, 30.f, 200.f, 0.f, -20.f, 2.f, 15.f, 120.f, 0.f, -22.f, 2.f, 5.f, 80.f, 0.f,
                                          100.f, 0.f,  0.f,   -60.f, 1.f, 0.f, -60.f, 1.f, 0.f, -60.f, 1.f, 0.f, 100.f, 100.f, 100.f, 100.f};
const BuiltinDescriptor kMultibandCompressorDescriptor{"multiband-compressor", kParamIds, kReadoutIds, &create_multiband_compressor,
                                                       kIdleReadouts, kDefaults};

} // namespace saine
