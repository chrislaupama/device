// Character Compressor: six circuit models on one feed-forward core.
// Parameters in the order of dstudio_core::builtin::character_compressor
// (checked by a Rust test).
//
// Signal flow per sample (base rate unless noted):
//   level rider   optional slow gain that moves the long-term level (tilted,
//                 band-weighted detector) towards the threshold
//   detector      internal (input after the rider) or external sidechain,
//                 optional M/S, SC high-pass and SC EQ; peak (Clean, Bus VCA,
//                 FET) or RMS (Vari-Mu 5 ms, Opto 10 ms, RMS VCA 20 ms) level
//   stereo link   per-channel levels pulled towards the louder one (dB)
//   static curve  the model's law (same as dstudio_core character_transfer),
//                 limited by Range
//   ballistics    gain reduction smoothed in dB: attack / release / hold
//                 (Clean, Vari-Mu, Bus VCA, FET), Adapt = program-dependent
//                 release (Bus VCA: auto release); Opto and RMS VCA have
//                 their own fixed, program-dependent timing
//   audio         delayed by the lookahead, gain applied, then the model's
//                 saturation (Drive) at 2x / 4x, makeup (+ auto gain), mix
//                 against the equally delayed dry signal (0..200 %)
// Latency = lookahead + oversampling latency (non-Clean models).
#include "builtin.hpp"
#include "dsp.hpp"
#include "dynamics_kit.hpp"

#include <algorithm>
#include <array>
#include <atomic>
#include <cmath>

namespace dstudio {

namespace {

enum Param : uint32_t {
    kModel,
    kThreshold,
    kRatio,
    kKnee,
    kAttack,
    kRelease,
    kAdapt,
    kHold,
    kRange,
    kDrive,
    kMix,
    kMakeup,
    kAutoGain,
    kLookahead,
    kOversampling,
    kStereoLink,
    kMsMode,
    kScSource,
    kScHpfOn,
    kScHpfFreq,
    kScEqFreq,
    kScEqGain,
    kScEqQ,
    kScListen,
    kBypass,
    kLevelEnabled,
    kLevelAmount,
    kLevelResponse,
    kLevelTilt,
    kLevelBands, // 4 x (enabled, freq, q, weight)
    kDeltaMonitor = kLevelBands + 16,
    kParamCount
};
enum Readout : uint32_t { kInDb, kOutDb, kGrDb, kLevelDb, kReadoutCount };
enum Model : int { kClean, kVariMu, kBusVca, kFet, kOpto, kRmsVca };

constexpr const char* kParamIds[kParamCount] = {
    "model",          "threshold",      "ratio",          "knee",          "attack",         "release",
    "adapt",          "hold",           "range",          "drive",         "mix",            "makeup",
    "auto_gain",      "lookahead",      "oversampling",   "stereo_link",   "ms_mode",        "sc_source",
    "sc_hpf_on",      "sc_hpf_freq",    "sc_eq_freq",     "sc_eq_gain",    "sc_eq_q",        "sc_listen",
    "bypass",         "level_enabled",  "level_amount",   "level_response", "level_tilt",    "level1_enabled",
    "level1_freq",    "level1_q",       "level1_weight",  "level2_enabled", "level2_freq",   "level2_q",
    "level2_weight",  "level3_enabled", "level3_freq",    "level3_q",      "level3_weight",  "level4_enabled",
    "level4_freq",    "level4_q",       "level4_weight",  "delta_monitor"};
constexpr const char* kReadoutIds[kReadoutCount] = {"in_db", "out_db", "gr_db", "level_db"};

constexpr float kMaxLookaheadMs = 20.0f;

// ---------------------------------------------------------------- static curves
// The laws of dstudio_core::builtin::character_transfer (kept in step by
// test_character_compressor.cpp, which checks values computed in Rust).

inline float slope_of(float ratio) noexcept { return ratio >= 29.5f ? -1.0f : 1.0f / std::max(ratio, 1.0f) - 1.0f; }

inline float soft(float over, float k, float s) noexcept {
    if (k > 0.0f && std::fabs(over) <= 0.5f * k) {
        const float t = over + 0.5f * k;
        return s * t * t / (2.0f * k);
    }
    return over > 0.0f ? s * over : 0.0f;
}

inline float vari_mu_knee(float ratio) noexcept { return 4.0f + 16.0f * (10.0f - std::min(ratio, 10.0f)) / 9.0f; }

struct Curve {
    static constexpr int kSteps = 256;
    int model = -1;
    float thr = 0.0f, ratio = 0.0f, knee = 0.0f, range = 0.0f;
    // Vari-Mu: integrated gain from the knee start to +20 dB.
    float vm_start = 0.0f, vm_h = 1.0f, vm_tail = 0.0f;
    std::array<float, kSteps + 1> vm{};

    bool same(int m, float t, float r, float k, float rg) const noexcept {
        return m == model && t == thr && r == ratio && k == knee && rg == range;
    }

    void build(int m, float t, float r, float k, float rg) noexcept {
        model = m, thr = t, ratio = r, knee = k, range = rg;
        if (m != kVariMu) return;
        const float kw = vari_mu_knee(r);
        const auto local = [&](float o) {
            const float n = std::clamp((o + 0.5f * kw) / 20.0f, 0.0f, 1.0f);
            const float eff = std::max(1.0f + (std::max(r, 1.0f) - 1.0f) * std::pow(n, 0.7f), 1.0f + 19.0f * n * n * n);
            const float w = std::clamp((o + 0.5f * kw) / kw, 0.0f, 1.0f);
            return w * (1.0f / eff - 1.0f);
        };
        vm_start = -0.5f * kw;
        vm_h = (20.0f - vm_start) / static_cast<float>(kSteps);
        vm[0] = 0.0f;
        for (int i = 0; i < kSteps; ++i) {
            const float o0 = vm_start + vm_h * static_cast<float>(i);
            vm[static_cast<std::size_t>(i) + 1] = vm[static_cast<std::size_t>(i)] + 0.5f * (local(o0) + local(o0 + vm_h)) * vm_h;
        }
        vm_tail = local(20.0f);
    }

    float vari_mu(float over) const noexcept {
        if (over <= vm_start) return 0.0f;
        const float p = (over - vm_start) / vm_h;
        if (p >= static_cast<float>(kSteps)) return vm[kSteps] + vm_tail * (over - 20.0f);
        const auto i = static_cast<std::size_t>(p);
        const float f = p - static_cast<float>(i);
        return vm[i] + f * (vm[i + 1] - vm[i]);
    }

    // Gain change (dB, <= 0) for a level x (dB).
    float eval(float x) const noexcept {
        const float over = x - thr;
        float g;
        switch (model) {
        case kClean: g = soft(over, knee, slope_of(ratio)); break;
        case kVariMu: g = vari_mu(over); break;
        case kBusVca: g = soft(over + (ratio < 10.0f ? 3.0f * (10.0f - ratio) / 8.0f : 0.0f), 4.0f, slope_of(ratio)); break;
        case kFet:
            g = ratio < 25.0f ? soft(over, 4.0f, slope_of(ratio))
                              : soft(over, 12.0f + 8.0f * std::clamp(over / 4.0f, 0.0f, 1.0f), slope_of(ratio));
            break;
        case kOpto: g = ratio < 5.5f ? soft(over, 10.0f, slope_of(3.0f)) : soft(over, 6.0f, slope_of(8.0f)); break;
        default: g = soft(over, 10.0f + 10.0f * std::clamp((ratio - 1.0f) / 9.0f, 0.0f, 1.0f), slope_of(ratio)); break;
        }
        return std::min(0.0f, std::max(g, -std::max(range, 0.0f)));
    }
};

// Saturation of each model: an asymmetric soft clipper with unity
// small-signal gain, y = (T(k (x + b)) - T(k b)) / (k T'(k b)).
struct Saturator {
    float k = 0.0f, bias = 0.0f, offset = 0.0f, norm = 1.0f;
    bool active = false;
    void set(int model, float drive) noexcept {
        static constexpr float kK[] = {0.0f, 0.6f, 0.35f, 0.9f, 0.5f, 0.3f};
        static constexpr float kBias[] = {0.0f, 0.15f, 0.0f, 0.1f, 0.2f, 0.0f};
        k = kK[model] * std::clamp(drive, 0.0f, 2.0f);
        bias = kBias[model];
        active = k > 1e-3f;
        if (!active) return;
        offset = dsp::soft_tanh(k * bias);
        // Slope of the rational tanh at k * bias (numerically).
        const float e = 1e-3f;
        const float slope = (dsp::soft_tanh(k * bias + e) - dsp::soft_tanh(k * bias - e)) / (2.0f * e);
        norm = 1.0f / (k * std::max(slope, 1e-3f));
    }
    float operator()(float x) const noexcept { return (dsp::soft_tanh(k * (x + bias)) - offset) * norm; }
};

// Writes x and returns the sample written n calls ago (n = 0: x itself).
inline float delayed(dsp::DelayLine& line, float x, uint32_t n) noexcept {
    line.push(x);
    return line.at(n);
}

class CharacterCompressor final : public BuiltinProcessor {
public:
    void prepare(double sample_rate, uint32_t max_block) override {
        sr_ = sample_rate;
        os_.prepare();
        const auto max_la = static_cast<uint32_t>(std::ceil(static_cast<double>(kMaxLookaheadMs) * 1e-3 * sample_rate));
        for (int c = 0; c < 2; ++c) {
            la_delay_[c].prepare(max_la);
            dry_delay_[c].prepare(max_la + dsp::Oversampler::latency(4));
        }
        wet_l_.assign(max_block, 0.0f);
        wet_r_.assign(max_block, 0.0f);
        pre_l_.assign(max_block, 0.0f);
        pre_r_.assign(max_block, 0.0f);
        makeup_.set_time(20.0f, sr_);
        mix_.set_time(20.0f, sr_);
        level_gain_.set_time(50.0f, sr_);
        rms_[0] = dsp::pole(5.0f, sr_);
        rms_[1] = dsp::pole(10.0f, sr_);
        rms_[2] = dsp::pole(20.0f, sr_);
        sus_pole_ = dsp::pole(800.0f, sr_);
        opto_charge_ = dsp::pole(1500.0f, sr_);
        opto_discharge_ = dsp::pole(6000.0f, sr_);
        short_pole_ = dsp::pole(100.0f, sr_);
        dc_pole_ = static_cast<float>(std::exp(-2.0 * 3.14159265358979323846 * 8.0 / sample_rate));
        fixed_.att_opto = dsp::pole(10.0f, sr_);
        fixed_.rel_opto_fast = dsp::pole(60.0f, sr_);
        fixed_.rel_opto_slow = dsp::pole(2500.0f, sr_);
        fixed_.att_rms = dsp::pole(2.0f, sr_);
        fixed_.rel_rms = dsp::pole(90.0f, sr_);
        fixed_.rel_auto_fast = dsp::pole(100.0f, sr_);
        fixed_.rel_auto_slow = dsp::pole(1200.0f, sr_);
    }

    void reset() noexcept override {
        os_.reset();
        for (int c = 0; c < 2; ++c) {
            la_delay_[c].reset();
            dry_delay_[c].reset();
            hpf_[c].reset();
            eq_[c].reset();
            gr_[c] = 0.0f;
            ms_[c] = 0.0f;
            hold_[c] = 0;
            dc_x_[c] = dc_y_[c] = 0.0f;
        }
        for (int c = 0; c < 2; ++c) {
            tilt_[c].reset();
            for (auto& w : weight_[c]) w.reset();
        }
        sus_ = 0.0f;
        opto_mem_ = 0.0f;
        lt_power_ = -1.0f;
        st_power_ = 0.0f;
        primed_ = false;
    }

    uint32_t latency() const noexcept override { return latency_.load(std::memory_order_relaxed); }
    uint32_t latency_for(const float* p) const noexcept override { return latency_of(p); }

    // Lookahead + oversampler latency (non-Clean models).
    uint32_t latency_of(const float* p) const noexcept {
        const int model = std::clamp(static_cast<int>(std::lround(p[kModel])), 0, 5);
        const auto la = static_cast<uint32_t>(std::lround(static_cast<double>(std::clamp(p[kLookahead], 0.0f, kMaxLookaheadMs)) * 1e-3 * sr_));
        return la + (model != kClean ? dsp::Oversampler::latency(p[kOversampling] >= 0.5f ? 4 : 2) : 0u);
    }

    void process(float* l, float* r, uint32_t n, const float* p, const BuiltinContext& ctx, float* ro) noexcept override {
        const int model = std::clamp(static_cast<int>(std::lround(p[kModel])), 0, 5);
        const float thr = std::clamp(p[kThreshold], -96.0f, 24.0f);
        const float ratio = std::clamp(p[kRatio], 1.0f, 100.0f);
        const float knee = std::clamp(p[kKnee], 0.0f, 48.0f);
        const float range = std::clamp(p[kRange], 0.0f, 96.0f);
        if (!curve_.same(model, thr, ratio, knee, range)) curve_.build(model, thr, ratio, knee, range);
        const bool timed = model <= kFet; // attack / release / hold / adapt apply
        const float att_ms = std::clamp(p[kAttack], 0.01f, 5000.0f);
        const float rel_ms = std::clamp(p[kRelease], 1.0f, 20000.0f);
        const bool adapt = timed && p[kAdapt] >= 0.5f;
        const auto hold_samples = timed ? static_cast<uint32_t>(std::clamp(p[kHold], 0.0f, 2000.0f) * 1e-3f * static_cast<float>(sr_)) : 0u;
        const float a_att = model == kOpto ? fixed_.att_opto : model == kRmsVca ? fixed_.att_rms : dsp::pole(att_ms, sr_);
        const float a_rel = dsp::pole(rel_ms, sr_);
        const float a_rel_slow = dsp::pole(4.0f * rel_ms, sr_);
        const float link = std::clamp(p[kStereoLink], 0.0f, 100.0f) * 0.01f;
        const bool ms_mode = p[kMsMode] >= 0.5f;
        const bool external = p[kScSource] >= 0.5f && ctx.sc_l != nullptr;
        const bool listen = p[kScListen] >= 0.5f;
        const bool delta = p[kDeltaMonitor] >= 0.5f;
        const bool bypass = p[kBypass] >= 0.5f;
        const float mix = std::clamp(p[kMix], 0.0f, 200.0f) * 0.01f;
        const float auto_db = p[kAutoGain] >= 0.5f ? std::min(24.0f, -0.5f * curve_.eval(0.0f)) : 0.0f;
        const float makeup = dsp::db_to_gain(std::clamp(p[kMakeup], -48.0f, 48.0f) + auto_db);
        const int factor = p[kOversampling] >= 0.5f ? 4 : 2;
        sat_.set(model, p[kDrive]);
        const bool os_path = model != kClean;
        const auto la = static_cast<uint32_t>(std::lround(static_cast<double>(std::clamp(p[kLookahead], 0.0f, kMaxLookaheadMs)) * 1e-3 * sr_));
        const uint32_t os_lat = os_path ? dsp::Oversampler::latency(factor) : 0u;
        latency_.store(la + os_lat, std::memory_order_relaxed);

        // Detector filters (coefficients only change with their parameters).
        const bool hpf_on = p[kScHpfOn] >= 0.5f;
        const float eq_gain = std::clamp(p[kScEqGain], -24.0f, 24.0f);
        if (hpf_on && p[kScHpfFreq] != hpf_f_) {
            hpf_f_ = p[kScHpfFreq];
            hpf_[0].c = hpf_[1].c = dsp::design_biquad(dsp::BiquadKind::Highpass, std::clamp(hpf_f_, 10.0f, 2000.0f), 0.7071f, 0.0f, sr_);
        }
        const bool eq_on = std::fabs(eq_gain) > 0.05f;
        // A stage switched back on starts from silence, not from its old tail.
        if (hpf_on && !hpf_was_) hpf_[0].reset(), hpf_[1].reset();
        if (eq_on && !eq_was_) eq_[0].reset(), eq_[1].reset();
        hpf_was_ = hpf_on, eq_was_ = eq_on;
        if (eq_on && (p[kScEqFreq] != eq_f_ || eq_gain != eq_g_ || p[kScEqQ] != eq_q_)) {
            eq_f_ = p[kScEqFreq], eq_g_ = eq_gain, eq_q_ = p[kScEqQ];
            eq_[0].c = eq_[1].c = dsp::design_biquad(dsp::BiquadKind::Peak, std::clamp(eq_f_, 20.0f, 20000.0f), std::clamp(eq_q_, 0.1f, 10.0f), eq_g_, sr_);
        }

        // Level rider.
        const bool level_on = p[kLevelEnabled] >= 0.5f;
        const float level_amount = std::clamp(p[kLevelAmount], 0.0f, 100.0f) * 0.01f;
        const float lt_pole = dsp::pole(std::clamp(p[kLevelResponse], 0.1f, 60.0f) * 1000.0f, sr_);
        const float tilt = std::clamp(p[kLevelTilt], -12.0f, 12.0f);
        if (level_on) update_weighting(p, tilt);
        if (!level_on && level_was_) {
            lt_power_ = -1.0f;
            st_power_ = 0.0f;
            for (int c = 0; c < 2; ++c) {
                tilt_[c].reset();
                for (auto& w : weight_[c]) w.reset();
            }
        }
        level_was_ = level_on;
        const float level_max = std::min(range, 12.0f);

        if (!primed_) {
            makeup_.snap(makeup);
            mix_.snap(mix);
            level_gain_.snap(0.0f);
            primed_ = true;
        }
        const float sc_detect_rms = model == kVariMu ? rms_[0] : model == kOpto ? rms_[1] : model == kRmsVca ? rms_[2] : 0.0f;

        float in_peak = 0.0f, gr_min = 0.0f, level_ext = 0.0f;
        for (uint32_t i = 0; i < n; ++i) {
            const float xl = l[i], xr = r[i];
            in_peak = std::max(in_peak, std::max(std::fabs(xl), std::fabs(xr)));

            // Long-term level rider (on the input).
            float lev_db = 0.0f;
            if (level_on) {
                // Weighted power per channel, then averaged (a mono sum would cancel wide audio).
                float pw = 0.0f;
                const float x[2] = {xl, xr};
                for (int c = 0; c < 2; ++c) {
                    float w = tilt_[c].process(x[c]) * tilt_gain_;
                    for (int b = 0; b < 4; ++b)
                        if (weight_on_[b]) w = weight_[c][b].process(w);
                    pw += 0.5f * w * w;
                }
                st_power_ = pw + short_pole_ * (st_power_ - pw);
                if (lt_power_ < 0.0f) lt_power_ = dsp::db_to_gain(thr) * dsp::db_to_gain(thr);
                if (st_power_ > 1e-5f) lt_power_ = pw + lt_pole * (lt_power_ - pw);
                const float lt_db = 3.0103f * dsp::fast_log2(lt_power_);
                lev_db = std::clamp(level_amount * (thr - lt_db), -level_max, level_max);
            }
            lev_db = level_gain_.next(lev_db);
            if (std::fabs(lev_db) > std::fabs(level_ext)) level_ext = lev_db;
            const float lev = lev_db == 0.0f ? 1.0f : dsp::fast_db_to_gain(lev_db);

            // Main and detector signals (M/S if asked).
            float a = xl * lev, b = xr * lev;
            float da = a, db = b;
            if (external) {
                da = ctx.sc_l[i];
                db = ctx.sc_r[i];
            }
            if (ms_mode) {
                const float m = 0.5f * (a + b), s = 0.5f * (a - b);
                a = m, b = s;
                const float dm = 0.5f * (da + db), dsd = 0.5f * (da - db);
                da = dm, db = dsd;
            }
            if (hpf_on) {
                da = hpf_[0].process(da);
                db = hpf_[1].process(db);
            }
            if (eq_on) {
                da = eq_[0].process(da);
                db = eq_[1].process(db);
            }

            // Levels (dB) per channel, stereo link.
            float lv[2];
            const float d[2] = {da, db};
            for (int c = 0; c < 2; ++c) {
                if (sc_detect_rms > 0.0f) {
                    const float x2 = d[c] * d[c];
                    ms_[c] = x2 + sc_detect_rms * (ms_[c] - x2);
                    lv[c] = 3.0103f * dsp::fast_log2(ms_[c] + 1e-24f);
                } else {
                    lv[c] = dsp::fast_gain_to_db(std::fabs(d[c]) + 1e-12f);
                }
            }
            const float lmax = std::max(lv[0], lv[1]);
            lv[0] += link * (lmax - lv[0]);
            lv[1] += link * (lmax - lv[1]);

            // Ballistics.
            const float depth = std::min(1.0f, -std::min(gr_[0], gr_[1]) * (1.0f / 12.0f));
            sus_ = depth + sus_pole_ * (sus_ - depth);
            if (model == kOpto) {
                const float target = std::min(1.0f, -std::min(gr_[0], gr_[1]) * 0.1f);
                opto_mem_ = target + (target > opto_mem_ ? opto_charge_ : opto_discharge_) * (opto_mem_ - target);
            }
            float rel;
            if (model == kOpto) {
                rel = fixed_.rel_opto_fast + opto_mem_ * (fixed_.rel_opto_slow - fixed_.rel_opto_fast);
            } else if (model == kRmsVca) {
                rel = fixed_.rel_rms;
            } else if (adapt && model == kBusVca) {
                rel = fixed_.rel_auto_fast + sus_ * (fixed_.rel_auto_slow - fixed_.rel_auto_fast);
            } else if (adapt) {
                rel = a_rel + sus_ * (a_rel_slow - a_rel);
            } else {
                rel = a_rel;
            }
            float g[2];
            for (int c = 0; c < 2; ++c) {
                const float target = curve_.eval(lv[c]);
                if (target < gr_[c]) {
                    gr_[c] = target + a_att * (gr_[c] - target);
                    hold_[c] = hold_samples;
                } else if (hold_[c] > 0) {
                    --hold_[c];
                } else {
                    gr_[c] = target + rel * (gr_[c] - target);
                }
                gr_min = std::min(gr_min, gr_[c]);
                g[c] = gr_[c] > -1e-4f ? 1.0f : dsp::fast_db_to_gain(gr_[c]);
            }

            if (listen) {
                float ol = da, orr = db;
                if (ms_mode) {
                    ol = da + db;
                    orr = da - db;
                }
                // The audio paths below keep running, so turning Listen off resumes current audio.
                l[i] = ol;
                r[i] = orr;
            }
            // Delayed audio with the gain applied (still M/S if asked).
            const float ya = delayed(la_delay_[0], a, la) * g[0];
            const float yb = delayed(la_delay_[1], b, la) * g[1];
            float wl = ya, wr = yb;
            if (ms_mode) {
                wl = ya + yb;
                wr = ya - yb;
            }
            wet_l_[i] = wl;
            wet_r_[i] = wr;
            // Dry, aligned with the wet path below.
            pre_l_[i] = delayed(dry_delay_[0], xl, la + os_lat);
            pre_r_[i] = delayed(dry_delay_[1], xr, la + os_lat);
        }

        // Saturation, oversampled. Non-Clean models always run this one path (Drive 0 passes
        // through), so Drive changes never switch between paths with different state.
        if (os_path) {
            const Saturator s = sat_;
            const bool sat = s.active;
            os_.process(wet_l_.data(), wet_r_.data(), n, factor, [&s, sat](float& a, float& b) {
                if (sat) {
                    a = s(a);
                    b = s(b);
                }
            });
            // Asymmetric models make DC: remove it below the audio band. Without Drive the
            // blocker only follows the signal, so it starts clean when Drive comes back.
            for (uint32_t i = 0; i < n; ++i) {
                for (int c = 0; c < 2; ++c) {
                    float& x = c == 0 ? wet_l_[i] : wet_r_[i];
                    const float y = sat ? x - dc_x_[c] + dc_pole_ * dc_y_[c] : x;
                    dc_x_[c] = x;
                    dc_y_[c] = y;
                    if (sat) x = y;
                }
            }
        }

        if (listen) {
            finish(l, r, n, in_peak, gr_min, level_ext, ro);
            return;
        }
        for (uint32_t i = 0; i < n; ++i) {
            const float dl = pre_l_[i], dr = pre_r_[i];
            float yl, yr;
            if (bypass) {
                yl = dl, yr = dr;
            } else if (delta) {
                yl = dl - wet_l_[i];
                yr = dr - wet_r_[i];
            } else {
                const float mk = makeup_.next(makeup);
                const float m = mix_.next(mix);
                yl = dl + m * (wet_l_[i] * mk - dl);
                yr = dr + m * (wet_r_[i] * mk - dr);
            }
            l[i] = yl;
            r[i] = yr;
        }
        finish(l, r, n, in_peak, gr_min, level_ext, ro);
    }

private:
    void update_weighting(const float* p, float tilt) noexcept {
        if (tilt != tilt_db_) {
            tilt_db_ = tilt;
            tilt_[0].c = tilt_[1].c = dsp::design_biquad(dsp::BiquadKind::HighShelf, 1000.0f, 0.5f, tilt, sr_);
            tilt_gain_ = dsp::db_to_gain(-0.5f * tilt);
        }
        for (int b = 0; b < 4; ++b) {
            const float* q = p + kLevelBands + 4 * b;
            const bool on = q[0] >= 0.5f && q[3] < -0.05f;
            if (on && !weight_on_[b]) weight_[0][b].reset(), weight_[1][b].reset();
            weight_on_[b] = on;
            if (weight_on_[b] && (q[1] != wf_[b] || q[2] != wq_[b] || q[3] != ww_[b])) {
                wf_[b] = q[1], wq_[b] = q[2], ww_[b] = q[3];
                weight_[0][b].c = weight_[1][b].c = dsp::design_biquad(dsp::BiquadKind::Peak, std::clamp(wf_[b], 20.0f, 20000.0f), std::clamp(wq_[b], 0.1f, 12.0f),
                                                  std::clamp(ww_[b], -48.0f, 0.0f), sr_);
            }
        }
    }

    void finish(const float* l, const float* r, uint32_t n, float in_peak, float gr_min, float level_db, float* ro) noexcept {
        for (int c = 0; c < 2; ++c) {
            if (ms_[c] < 1e-20f) ms_[c] = 0.0f;
            if (gr_[c] > -1e-6f) gr_[c] = 0.0f;
            hpf_[c].flush();
            eq_[c].flush();
            if (std::fabs(dc_y_[c]) < 1e-20f) dc_y_[c] = 0.0f;
        }
        for (int c = 0; c < 2; ++c) {
            tilt_[c].flush();
            for (auto& w : weight_[c]) w.flush();
        }
        if (st_power_ < 1e-20f) st_power_ = 0.0f;
        ro[kInDb] = dsp::gain_to_db(in_peak);
        ro[kOutDb] = dsp::gain_to_db(dsp::stereo_peak(l, r, n));
        ro[kGrDb] = gr_min;
        ro[kLevelDb] = level_db;
    }

    double sr_ = 48000.0;
    Curve curve_;
    Saturator sat_;
    dsp::Oversampler os_;
    dsp::DelayLine la_delay_[2], dry_delay_[2];
    std::vector<float> wet_l_, wet_r_, pre_l_, pre_r_;
    dsp::Biquad hpf_[2], eq_[2], tilt_[2], weight_[2][4];
    bool hpf_was_ = false, eq_was_ = false, level_was_ = false;
    float hpf_f_ = -1.0f, eq_f_ = -1.0f, eq_g_ = 0.0f, eq_q_ = -1.0f, tilt_db_ = 1e9f, tilt_gain_ = 1.0f;
    float wf_[4] = {-1, -1, -1, -1}, wq_[4] = {-1, -1, -1, -1}, ww_[4] = {1, 1, 1, 1};
    bool weight_on_[4] = {false, false, false, false};
    dsp::Smoother makeup_, mix_, level_gain_;
    float rms_[3] = {0, 0, 0};
    float sus_pole_ = 0.0f, opto_charge_ = 0.0f, opto_discharge_ = 0.0f, short_pole_ = 0.0f, dc_pole_ = 0.0f;
    struct {
        float att_opto, rel_opto_fast, rel_opto_slow, att_rms, rel_rms, rel_auto_fast, rel_auto_slow;
    } fixed_{};
    float gr_[2] = {0, 0}, ms_[2] = {0, 0}, dc_x_[2] = {0, 0}, dc_y_[2] = {0, 0};
    uint32_t hold_[2] = {0, 0};
    float sus_ = 0.0f, opto_mem_ = 0.0f, lt_power_ = -1.0f, st_power_ = 0.0f;
    bool primed_ = false;
    std::atomic<uint32_t> latency_{0};
};

std::unique_ptr<BuiltinProcessor> create_character_compressor() { return std::make_unique<CharacterCompressor>(); }

} // namespace

extern const BuiltinDescriptor kCharacterCompressorDescriptor;
constexpr float kIdleReadouts[] = {-120.0f, -120.0f, 0.0f, 0.0f}; // silent in / out, no reduction, no rider gain
// Defaults of dstudio_core::builtin::character_compressor, registry order.
constexpr float kDefaults[kParamCount] = {
    0.f, -18.f, 4.f,  6.f,   10.f,   200.f, 0.f, 0.f, 24.f, 1.f, 100.f, 0.f, 0.f, 0.f, 0.f, 100.f, //
    0.f, 0.f,   0.f,  80.f,  1000.f, 0.f,   1.f, 0.f, 0.f,  0.f, 50.f,  8.f, 0.f,                   //
    0.f, 150.f, 1.f,  0.f,   0.f,    600.f, 1.f, 0.f, 0.f,  2500.f, 1.f, 0.f, 0.f, 8000.f, 1.f, 0.f, //
    0.f};
const BuiltinDescriptor kCharacterCompressorDescriptor{"character-compressor", kParamIds, kReadoutIds, &create_character_compressor,
                                                       kIdleReadouts, kDefaults};

} // namespace dstudio
