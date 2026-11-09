// Auto Filter: a resonant low-pass / high-pass / band-pass / notch (12 or
// 24 dB/oct) with input drive, an envelope follower and an LFO that move the
// cutoff, a dry/wet mix, and a DJ mode where one position sweeps a low-pass
// (left of centre) or a high-pass (right). Parameters in the order of
// saine_core::builtin::auto_filter (checked by a Rust test).
//
// The filter stages are those of saine_core::builtin::auto_filter::
// filter_stages (the panel's curve): TPT state-variable sections, a
// resonance-to-Q map (0.5 .. ~40), a second fixed-Q stage for 24 dB/oct and
// a level correction that keeps resonant peaks in check. Cutoff modulation:
//   cutoff = frequency * 2^((env * Env Amount + lfo * LFO Amount) / 12)
// where env (0..1) follows the input level (-40..0 dBFS) with Env Attack /
// Release and lfo (-1..1) is the LFO (free in Hz or synced to the tempo; the
// right channel runs LFO Stereo degrees ahead). Coefficients update every 8
// samples per channel. Drive saturates the input with an antialiased (ADAA)
// tanh. No latency.
#include "builtin.hpp"
#include "dsp.hpp"
#include "eq_sections.hpp"

#include <algorithm>
#include <cmath>

namespace saine {

namespace {

enum Param : uint32_t {
    kType,
    kSlope,
    kFreq,
    kReso,
    kDrive,
    kEnvAmount,
    kEnvAttack,
    kEnvRelease,
    kLfoAmount,
    kLfoRate,
    kLfoRateSync,
    kLfoShape,
    kLfoSync,
    kLfoStereo,
    kMix,
    kMode,
    kDjPosition,
    kParamCount
};
enum Readout : uint32_t { kCutoff, kReadoutCount };

constexpr const char* kParamIds[kParamCount] = {"filter_type", "slope",      "frequency",     "resonance",
                                                "drive",       "env_amount", "env_attack",    "env_release",
                                                "lfo_amount",  "lfo_rate_free", "lfo_rate_sync", "lfo_shape",
                                                "lfo_sync",    "lfo_stereo", "mix",           "mode",
                                                "dj_position"};
constexpr const char* kReadoutIds[kReadoutCount] = {"cutoff_hz"};

// Quarter notes per LFO cycle of each sync division (1/1 .. 1/32).
constexpr double kQuarters[14] = {4.0,       2.0,  3.0,   4.0 / 3.0, 1.0,  1.5,   2.0 / 3.0,
                                  0.5,       0.75, 1.0 / 3.0, 0.25, 0.375, 1.0 / 6.0, 0.125};

constexpr uint32_t kControl = 8;
constexpr double kSecondStageQ = 0.54; // saine_core auto_filter::SECOND_STAGE_Q
constexpr float kDjDeadZone = 0.06f;

inline int choice(float v, int count) noexcept { return std::clamp(static_cast<int>(std::lround(v)), 0, count - 1); }

inline double resonance_q(double reso) noexcept {
    const double t = std::clamp(reso / 1.25, 0.0, 1.0);
    return 0.5 * std::exp(t * (6.0 - 1.6 * t));
}
inline double resonance_gain(double q) noexcept { return std::pow((1.0 + (q / 2.0) * (q / 2.0)) / 1.0625, -0.1); }

inline float smoothstep(float x) noexcept {
    x = std::clamp(x, 0.0f, 1.0f);
    return x * x * (3.0f - 2.0f * x);
}

// DJ position -> (kind 0 LP / 1 HP, cutoff, resonance scale, wet).
inline void dj_map(float pos, int& kind, float& fc, float& scale, float& wet) noexcept {
    const float a = std::fabs(pos);
    if (a < kDjDeadZone) {
        kind = 0;
        fc = 20000.0f;
        scale = 0.0f;
        wet = 0.0f;
        return;
    }
    const float n = std::clamp((a - kDjDeadZone) / (1.0f - kDjDeadZone), 0.0f, 1.0f);
    scale = n * n;
    if (pos < 0.0f) {
        kind = 0;
        fc = 20000.0f * std::pow(0.001f, n);
        wet = smoothstep(std::log2(20000.0f / fc) * 3.0f);
    } else {
        kind = 1;
        fc = 20.0f * std::pow(1000.0f, n);
        wet = smoothstep(std::log2(fc / 20.0f) * 3.0f);
    }
}

inline uint32_t hash(uint32_t x) noexcept {
    x ^= x >> 16;
    x *= 0x7feb352dU;
    x ^= x >> 15;
    x *= 0x846ca68bU;
    x ^= x >> 16;
    return x;
}
inline float hashed(int64_t i) noexcept { return static_cast<float>(hash(static_cast<uint32_t>(i) * 2654435761U + 12345U) >> 8) / 8388608.0f - 1.0f; }

// LFO value (-1..1) at phase (cycles; the integer part counts cycles).
inline float lfo_value(int shape, double phase) noexcept {
    const double cyc = std::floor(phase);
    const float f = static_cast<float>(phase - cyc);
    switch (shape) {
    case 0: return std::sin(6.28318530718f * f);
    case 1: return 4.0f * std::fabs(f - 0.5f) - 1.0f;
    case 2: return 2.0f * f - 1.0f;
    case 3: return f < 0.5f ? 1.0f : -1.0f;
    case 4: return hashed(static_cast<int64_t>(cyc));
    default: {
        const float a = hashed(static_cast<int64_t>(cyc)), b = hashed(static_cast<int64_t>(cyc) + 1);
        const float w = 0.5f - 0.5f * std::cos(3.14159265f * f);
        return a + w * (b - a);
    }
    }
}

inline float logcosh(float x) noexcept {
    const float a = std::fabs(x);
    return a + std::log1p(std::exp(-2.0f * a)) - 0.69314718f;
}

class AutoFilter final : public BuiltinProcessor {
public:
    void prepare(double sample_rate, uint32_t) override {
        fs_ = sample_rate;
        const double ctl = static_cast<double>(kControl);
        slew_ = static_cast<float>(std::exp(-ctl / (0.004 * fs_)));    // cutoff glide (~4 ms)
        param_ = static_cast<float>(std::exp(-ctl / (0.02 * fs_)));    // knob glide (~20 ms)
        mix_.set_time(20.0f, fs_);
        reset();
    }

    void reset() noexcept override {
        for (auto& s : st_) s.clear();
        env_ = 0.0f;
        phase_ = 0.0;
        prev_[0] = prev_[1] = 0.0f;
        primed_ = false;
    }

    void process(float* l, float* r, uint32_t n, const float* p, const BuiltinContext& ctx, float* ro) noexcept override {
        const int type = choice(p[kType], 4);
        const bool slope24 = p[kSlope] >= 0.5f;
        const float freq = std::clamp(p[kFreq], 10.0f, 24000.0f);
        const float reso = std::clamp(p[kReso], 0.0f, 1.25f);
        const float drive_db = std::clamp(p[kDrive], 0.0f, 48.0f);
        const float env_amt = std::clamp(p[kEnvAmount], -96.0f, 96.0f);
        const float a_att = dsp::pole(std::clamp(p[kEnvAttack], 0.01f, 5000.0f), fs_);
        const float a_rel = dsp::pole(std::clamp(p[kEnvRelease], 0.1f, 20000.0f), fs_);
        const float lfo_amt = std::clamp(p[kLfoAmount], 0.0f, 96.0f);
        const int shape = choice(p[kLfoShape], 6);
        const bool sync = p[kLfoSync] >= 0.5f;
        const double stereo = static_cast<double>(std::clamp(p[kLfoStereo], 0.0f, 360.0f)) / 360.0;
        const float mix = std::clamp(p[kMix], 0.0f, 100.0f) * 0.01f;
        const bool dj = p[kMode] >= 0.5f;
        const float dj_pos = std::clamp(p[kDjPosition], -1.0f, 1.0f);

        const RtTransport* tr = ctx.transport;
        const double tempo = tr != nullptr && tr->tempo_bpm > 0.0 ? tr->tempo_bpm : 120.0;
        double rate;
        if (sync) {
            const double q = kQuarters[choice(p[kLfoRateSync], 14)];
            rate = tempo / 60.0 / q;
            if (tr != nullptr && tr->playing) { // lock to the bar grid at this segment's first frame
                const double beats = static_cast<double>(tr->position + ctx.offset) / tr->sample_rate * tempo / 60.0;
                phase_ = beats / q;
            }
        } else {
            rate = std::clamp(static_cast<double>(p[kLfoRate]), 0.001, 100.0);
        }
        const double dphase = rate / fs_;

        const float log_freq = std::log2(freq);
        if (!primed_) {
            log_freq_ = log_freq;
            reso_ = reso;
            dj_pos_ = dj_pos;
            mix_.snap(mix);
            cut_[0] = cut_[1] = log_freq;
            primed_ = true;
        }

        const float g_drive = dsp::db_to_gain(drive_db);
        const float drive_mix = std::clamp(drive_db / 3.0f, 0.0f, 1.0f);
        const float inv_drive = 1.0f / g_drive;
        const float top = static_cast<float>(std::log2(std::min(20000.0, 0.49 * fs_)));
        constexpr float bottom = 4.321928f; // log2(20)

        float cutoff_sum = 0.0f;
        uint32_t cutoff_n = 0;
        for (uint32_t pos = 0; pos < n; pos += kControl) {
            const uint32_t len = std::min(kControl, n - pos);
            // Parameter glides (once per control period).
            log_freq_ = log_freq + param_ * (log_freq_ - log_freq);
            reso_ = reso + param_ * (reso_ - reso);
            dj_pos_ = dj_pos + param_ * (dj_pos_ - dj_pos);

            // Envelope (linked) over this period, from the dry input.
            float env = env_;
            for (uint32_t i = 0; i < len; ++i) {
                const float x = std::max(std::fabs(l[pos + i]), std::fabs(r[pos + i]));
                env = x + (x > env ? a_att : a_rel) * (env - x);
            }
            env_ = env < 1e-9f ? 0.0f : env;
            const float e = std::clamp((dsp::gain_to_db(env_) + 40.0f) / 40.0f, 0.0f, 1.0f);

            int kind = type;
            float wet = mix, rscale = 1.0f, base = log_freq_;
            if (dj) {
                float fc;
                dj_map(dj_pos_, kind, fc, rscale, wet);
                wet *= mix;
                base = std::log2(fc);
            }
            const double q = resonance_q(static_cast<double>(reso_ * rscale));
            const float level = static_cast<float>(resonance_gain(q));

            eq::Coef c[2][2];
            for (int ch = 0; ch < 2; ++ch) {
                const float lfo = lfo_amt > 0.0f ? lfo_value(shape, phase_ + (ch == 1 ? stereo : 0.0)) : 0.0f;
                float target = base;
                if (!dj) target += (e * env_amt + lfo * lfo_amt) / 12.0f;
                target = std::clamp(target, bottom, top);
                cut_[ch] = target + slew_ * (cut_[ch] - target);
                const double g = eq::prewarp(std::exp2(static_cast<double>(cut_[ch])), fs_);
                c[ch][0] = stage(kind, g, q);
                c[ch][1] = stage(kind, g, kSecondStageQ);
            }
            phase_ += dphase * len;
            if (phase_ > 1e6) phase_ -= std::floor(phase_);
            cutoff_sum += std::exp2(cut_[0]);
            ++cutoff_n;

            for (uint32_t i = 0; i < len; ++i) {
                const float m = mix_.next(wet);
                float* io[2] = {l + pos + i, r + pos + i};
                for (int ch = 0; ch < 2; ++ch) {
                    const float dry = *io[ch];
                    float x = dry;
                    if (drive_mix > 0.0f) x = dry + drive_mix * (saturate(ch, g_drive * dry) * inv_drive - dry);
                    else prev_[ch] = 0.0f;
                    float y = eq::tick(c[ch][0], st_[2 * ch], ch, x);
                    if (slope24) y = eq::tick(c[ch][1], st_[2 * ch + 1], ch, y);
                    *io[ch] = dry + m * (level * y - dry);
                }
            }
        }
        for (auto& s : st_) eq::sanitize(s);
        for (float& u : prev_)
            if (!std::isfinite(u)) u = 0.0f;
        ro[kCutoff] = cutoff_n > 0 ? cutoff_sum / static_cast<float>(cutoff_n) : 0.0f;
    }

private:
    static eq::Coef stage(int kind, double g, double q) noexcept {
        const double k = 1.0 / q;
        eq::Coef c;
        switch (kind) {
        case 0: c = eq::two(g, k, 0.0, 0.0, 1.0); break;
        case 1: c = eq::two(g, k, 1.0, -k, -1.0); break;
        case 2: c = eq::two(g, k, 0.0, k, 0.0); break;
        default: c = eq::two(g, k, 1.0, -k, 0.0); break;
        }
        c.finish();
        return c;
    }

    // tanh with a first-order antiderivative antialiaser.
    float saturate(int ch, float u) noexcept {
        const float u0 = prev_[ch];
        prev_[ch] = u;
        const float d = u - u0;
        if (std::fabs(d) < 1e-3f) return std::tanh(0.5f * (u + u0));
        return (logcosh(u) - logcosh(u0)) / d;
    }

    double fs_ = 48000.0;
    float slew_ = 0.0f, param_ = 0.0f;
    eq::State st_[4]{}; // per channel: stage 1, stage 2 (each uses its channel slot)
    float env_ = 0.0f;
    double phase_ = 0.0;
    float log_freq_ = 0.0f, reso_ = 0.0f, dj_pos_ = 0.0f;
    float cut_[2] = {0.0f, 0.0f};
    float prev_[2] = {0.0f, 0.0f};
    dsp::Smoother mix_;
    bool primed_ = false;
};

std::unique_ptr<BuiltinProcessor> create_auto_filter() { return std::make_unique<AutoFilter>(); }

} // namespace

constexpr float kDefaults[kParamCount] = {0.0f, 0.0f, 20000.0f, 0.0f, 0.0f, 0.0f, 1.0f, 100.0f, 0.0f, 1.0f, 4.0f, 0.0f, 0.0f, 0.0f, 100.0f, 0.0f, 0.0f};
extern const BuiltinDescriptor kAutoFilterDescriptor;
const BuiltinDescriptor kAutoFilterDescriptor{"auto-filter", kParamIds, kReadoutIds, &create_auto_filter, {}, kDefaults};

} // namespace saine
