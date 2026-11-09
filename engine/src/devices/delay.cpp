// Delay: stereo delay with Tape, Digital and BBD characters, tempo sync,
// linked or independent sides, ping-pong above 100% width, wow modulation,
// grit and a six-band EQ inside the feedback loop (it shapes the repeats,
// never the dry path). Parameters in the order of saine_core::builtin::delay.
//
// Routing (the reference's echo model): with ping-pong amount a = (width -
// 100%) / 100% (0 below 100%), the left line takes the input and the right
// line (1 - a) of it; every repeat feeds its own side by (1 - a) and the
// other side by a. Below 100% the repeats are narrowed (M/S).
//
// Time changes: Digital crossfades between the old and the new tap (no pitch
// change); Tape and BBD glide the read head (pitch bends like a real machine).
// BBD darkens with longer times (clock-dependent anti-alias filter).
#include "builtin.hpp"
#include "dsp.hpp"
#include "dsp_delay.hpp"
#include "spectral.hpp"

#include <algorithm>
#include <cmath>

namespace saine {

namespace {

enum Param : uint32_t {
    kBpm, kDivision, kFeedback, kGrit, kModSpeed, kModDepth, kWidth, kMix, kMode,
    kEq0, // 6 bands x (freq, gain, q, enabled)
    kSync = kEq0 + 24, kDry, kTimeMs, kLink, kDivisionR, kTimeMsR, kParamCount
};
constexpr const char* kParamIds[kParamCount] = {
    "time", "timeDivision", "feedback", "grit", "modSpeed", "modDepth", "stereoWidth", "mix", "mode",
    "eq0Freq", "eq0Gain", "eq0Q", "eq0Enabled", "eq1Freq", "eq1Gain", "eq1Q", "eq1Enabled",
    "eq2Freq", "eq2Gain", "eq2Q", "eq2Enabled", "eq3Freq", "eq3Gain", "eq3Q", "eq3Enabled",
    "eq4Freq", "eq4Gain", "eq4Q", "eq4Enabled", "eq5Freq", "eq5Gain", "eq5Q", "eq5Enabled",
    "sync", "dry", "timeMs", "link", "timeDivisionR", "timeMsR"};
constexpr uint32_t kBands = dsp::kSpectrumBands;
// Readouts: wet peak (dB), current delay of each side (ms), then the wet spectrum bands (dB, dsp::SpectrumTap).
struct ReadoutIds {
    const char* ids[3 + kBands];
    constexpr ReadoutIds() : ids{"wet_db", "delay_l_ms", "delay_r_ms"} {
        for (uint32_t i = 0; i < kBands; ++i) ids[3 + i] = dsp::kSpectrumIds[i];
    }
};
constexpr ReadoutIds kReadouts{};
enum Mode : int { kTape, kDigital, kBbd };
constexpr float kDivisionBeats[15] = {8.0f, 4.0f, 3.0f, 2.0f, 4.0f / 3.0f, 1.5f, 1.0f, 2.0f / 3.0f, 0.75f, 0.5f, 1.0f / 3.0f, 0.375f, 0.25f, 1.0f / 6.0f, 0.125f};
constexpr float kMaxSeconds = 12.0f;
constexpr float kMaxModMs = 8.0f;
constexpr float kXfadeMs = 60.0f;

class Delay final : public BuiltinProcessor {
public:
    void prepare(double sample_rate, uint32_t max_block) override {
        sr_ = sample_rate;
        for (auto& d : line_) d.prepare(static_cast<std::size_t>(static_cast<double>(kMaxSeconds + 0.05f + 2.0f * kMaxModMs * 0.001f) * sr_) + 16);
        tap_.prepare(sample_rate);
        wet_.assign(max_block, 0.0f);
        mix_.set_time(20.0f, sr_);
        fb_.set_time(20.0f, sr_);
        width_.set_time(20.0f, sr_);
        glide_ = dsp::pole(120.0f, sr_);
        reset();
    }

    void reset() noexcept override {
        for (auto& d : line_) d.reset();
        for (int c = 0; c < 2; ++c) {
            for (auto& b : eq_[c]) b.reset();
            tone_[c].reset();
            aa_[c][0].reset();
            aa_[c][1].reset();
            cur_[c] = target_[c] = old_[c] = -1.0f;
            xf_[c] = 1.0f;
            fbs_[c] = 0.0f;
        }
        wow_phase_ = 0.0f;
        drift_ = drift_target_ = 0.0f;
        drift_count_ = 0;
        rng_.seed(0x5eed1234u);
        tap_.reset();
        for (auto& v : eq_key_) v = -1.0f;
        primed_ = false;
    }

    void process(float* l, float* r, uint32_t n, const float* p, const BuiltinContext& ctx, float* ro) noexcept override {
        const int mode = std::clamp(static_cast<int>(std::lround(p[kMode])), 0, 2);
        const bool sync = p[kSync] >= 0.5f;
        const bool link = p[kLink] >= 0.5f;
        const double bpm = std::clamp(ctx.transport != nullptr ? ctx.transport->tempo_bpm : static_cast<double>(p[kBpm]), 20.0, 999.0);
        const auto beats = [](float idx) { return kDivisionBeats[std::clamp(static_cast<int>(std::lround(idx)), 0, 14)]; };
        float t_l = sync ? static_cast<float>(static_cast<double>(beats(p[kDivision])) * 60.0 / bpm) : std::clamp(p[kTimeMs], 1.0f, 12000.0f) * 0.001f;
        float t_r = link ? t_l : (sync ? static_cast<float>(static_cast<double>(beats(p[kDivisionR])) * 60.0 / bpm) : std::clamp(p[kTimeMsR], 1.0f, 12000.0f) * 0.001f);
        t_l = std::min(t_l, kMaxSeconds);
        t_r = std::min(t_r, kMaxSeconds);
        const float fb = std::clamp(p[kFeedback], 0.0f, 100.0f) * 0.01f;
        const float grit = std::clamp(p[kGrit], 0.0f, 100.0f) * 0.01f;
        const float mod_hz = std::clamp(p[kModSpeed], 0.005f, 20.0f);
        const float mod_depth = std::clamp(p[kModDepth], 0.0f, 100.0f) * 0.01f;
        const float width = std::clamp(p[kWidth], 0.0f, 200.0f) * 0.01f;
        const float mix = std::clamp(p[kMix], 0.0f, 100.0f) * 0.01f;
        const bool dry_on = p[kDry] >= 0.5f;
        update_eq(p);

        const float target_samples[2] = {t_l * static_cast<float>(sr_), t_r * static_cast<float>(sr_)};
        if (!primed_) {
            mix_.snap(mix);
            fb_.snap(fb);
            width_.snap(std::min(width, 1.0f));
            for (int c = 0; c < 2; ++c) cur_[c] = target_[c] = old_[c] = target_samples[c];
            primed_ = true;
        }
        for (int c = 0; c < 2; ++c) {
            if (mode == kDigital) {
                // New time: crossfade from the tap in use to the new one.
                if (std::fabs(target_samples[c] - target_[c]) > 0.5f) {
                    old_[c] = xf_[c] >= 0.5f ? target_[c] : old_[c];
                    target_[c] = target_samples[c];
                    cur_[c] = target_[c];
                    xf_[c] = 0.0f;
                }
            } else {
                target_[c] = target_samples[c];
                xf_[c] = 1.0f;
            }
        }
        // Character constants.
        const float pong = std::max(0.0f, width - 1.0f);
        const float hiss = mode == kTape ? grit * grit * 0.0015f : (mode == kBbd ? grit * 0.002f : 0.0f);
        const float drive = 1.0f + (mode == kDigital ? 0.0f : 5.0f * grit);
        const float bits = 16.0f - 10.0f * grit; // Digital grit: word length reduction
        const float quant = std::exp2(bits - 1.0f);
        if (mode == kTape) {
            for (auto& t : tone_) t.set_hz(16000.0f - 12500.0f * grit, sr_);
        } else if (mode == kBbd) {
            // Clock-limited bandwidth: ~4096 stages, darker for longer times.
            const float avg = 0.5f * (t_l + t_r);
            const float fc = std::clamp(4096.0f / (4.0f * std::max(avg, 0.001f)) * 0.7f, 1500.0f, 11000.0f) * (1.0f - 0.45f * grit);
            for (int c = 0; c < 2; ++c) {
                aa_[c][0].set(dsp::Biquad::kLowpass, fc, 0.54f, 0.0f, sr_);
                aa_[c][1].set(dsp::Biquad::kLowpass, fc, 1.31f, 0.0f, sr_);
            }
        }
        const float mod_amp = mod_depth * kMaxModMs * 0.001f * static_cast<float>(sr_) * 0.5f;
        const float wow_inc = mod_hz / static_cast<float>(sr_);
        const float xf_inc = 1000.0f / (kXfadeMs * static_cast<float>(sr_));
        const auto cap = static_cast<uint32_t>(wet_.size());

        float wet_peak = 0.0f;
        for (uint32_t i = 0; i < n; ++i) {
            const float xl = l[i], xr = r[i];
            // Modulation: sine wow (+ slow random drift on tape).
            wow_phase_ += wow_inc;
            if (wow_phase_ >= 1.0f) wow_phase_ -= 1.0f;
            float mod = mod_amp * (1.0f + dsp::fast_sin(wow_phase_));
            if (mode == kTape) {
                if (drift_count_-- <= 0) {
                    drift_target_ = rng_.uni();
                    drift_count_ = static_cast<int32_t>(0.25 * sr_);
                }
                drift_ += (drift_target_ - drift_) * 0.00005f;
                mod += mod_amp * 0.6f * drift_;
            }
            float y[2];
            for (int c = 0; c < 2; ++c) {
                if (mode != kDigital) cur_[c] = target_[c] + glide_ * (cur_[c] - target_[c]);
                const float d = cur_[c] + mod + 2.0f;
                float v = line_[c].read(d - 1.0f);
                if (xf_[c] < 1.0f) {
                    const float o = line_[c].read(old_[c] + mod + 1.0f);
                    v = o + xf_[c] * (v - o);
                    xf_[c] = std::min(1.0f, xf_[c] + xf_inc);
                }
                y[c] = v;
            }
            // Feedback path: cross matrix, EQ, character, soft limit.
            const float g = fb_.next(fb);
            float f[2] = {y[0] * (1.0f - pong) + y[1] * pong, y[1] * (1.0f - pong) + y[0] * pong};
            float in[2] = {xl * (1.0f - pong) + 0.5f * (xl + xr) * pong, xr * (1.0f - pong)};
            for (int c = 0; c < 2; ++c) {
                float s = in[c] + g * f[c];
                for (int b = 0; b < 6; ++b)
                    if (eq_on_[b]) s = eq_[c][b].process(s);
                switch (mode) {
                case kTape:
                    s = tone_[c].lp(s);
                    s += grit * (dsp::soft_clip(s * drive) / drive - s) + hiss * rng_.bi();
                    break;
                case kBbd:
                    s = aa_[c][1].process(aa_[c][0].process(s));
                    s += grit * (dsp::soft_clip(s * drive) / drive - s) + hiss * rng_.bi();
                    break;
                default:
                    if (grit > 0.0f) s = std::round(s * quant) / quant;
                    break;
                }
                // Keep runaway feedback bounded (soft above ~0 dBFS).
                s = dsp::soft_limit(s);
                line_[c].push(dsp::flush(s));
            }
            // Output: narrow below 100%.
            const float w = width_.next(std::min(width, 1.0f));
            const float mid = 0.5f * (y[0] + y[1]), side = 0.5f * (y[0] - y[1]) * w;
            const float wl = mid + side, wr = mid - side;
            wet_peak = std::max(wet_peak, std::max(std::fabs(wl), std::fabs(wr)));
            if (i < cap) wet_[i] = mid;
            const float m = mix_.next(mix);
            const float dry = dry_on ? 1.0f - m : 0.0f;
            l[i] = xl * dry + wl * m;
            r[i] = xr * dry + wr * m;
        }
        for (int c = 0; c < 2; ++c) {
            for (auto& b : eq_[c]) b.flush_state();
            tone_[c].y = dsp::flush(tone_[c].y);
            aa_[c][0].flush_state();
            aa_[c][1].flush_state();
        }
        tap_.push(wet_.data(), wet_.data(), std::min(n, cap));
        ro[0] = dsp::gain_to_db(wet_peak);
        ro[1] = cur_[0] / static_cast<float>(sr_) * 1000.0f;
        ro[2] = cur_[1] / static_cast<float>(sr_) * 1000.0f;
        tap_.write(ro + 3);
    }

private:
    void update_eq(const float* p) noexcept {
        static constexpr dsp::Biquad::Type kTypes[6] = {dsp::Biquad::kHighpass, dsp::Biquad::kLowShelf, dsp::Biquad::kPeak,
                                                        dsp::Biquad::kPeak, dsp::Biquad::kHighShelf, dsp::Biquad::kLowpass};
        for (int b = 0; b < 6; ++b) {
            const float* q = p + kEq0 + 4 * b;
            eq_on_[b] = q[3] >= 0.5f;
            const float key = q[0] * 1.0001f + q[1] * 7.0f + q[2] * 131.0f;
            if (key == eq_key_[b]) continue;
            eq_key_[b] = key;
            const float f = std::clamp(q[0], 20.0f, 20000.0f);
            const float gain = std::clamp(q[1], -30.0f, 15.0f);
            const float qq = std::clamp(q[2], 0.1f, 20.0f);
            for (int c = 0; c < 2; ++c) eq_[c][b].set(kTypes[b], f, qq, gain, sr_);
        }
    }

    double sr_ = 48000.0;
    dsp::DelayLine line_[2];
    dsp::Biquad eq_[2][6];
    bool eq_on_[6] = {};
    float eq_key_[6] = {};
    dsp::ExpOnePole tone_[2];
    dsp::Biquad aa_[2][2];
    float cur_[2] = {}, target_[2] = {}, old_[2] = {}, xf_[2] = {1.0f, 1.0f}, fbs_[2] = {};
    float glide_ = 0.0f;
    float wow_phase_ = 0.0f, drift_ = 0.0f, drift_target_ = 0.0f;
    int32_t drift_count_ = 0;
    dsp::UnitRng rng_;
    dsp::SpectrumTap tap_;
    std::vector<float> wet_;
    dsp::Smoother mix_, fb_, width_;
    bool primed_ = false;
};

std::unique_ptr<BuiltinProcessor> create() { return std::make_unique<Delay>(); }

} // namespace

extern const BuiltinDescriptor kDelayDescriptor;
struct IdleReadouts {
    float v[3 + kBands];
    constexpr IdleReadouts() : v{-120.0f, 0.0f, 0.0f} {
        for (uint32_t i = 3; i < 3 + kBands; ++i) v[i] = -120.0f;
    }
};
constexpr IdleReadouts kIdle{};
constexpr float kDefaults[kParamCount] = {120.f, 8.f, 50.f, 0.f, 0.0692f, 0.f, 100.f, 50.f, 0.f,
                                          20.f, 0.f, 0.71f, 0.f, 150.f, 0.f, 1.f, 1.f, 500.f, 0.f, 1.f, 1.f,
                                          3000.f, 0.f, 1.f, 1.f, 8000.f, 0.f, 1.f, 1.f, 20000.f, 0.f, 0.71f, 0.f,
                                          1.f, 1.f, 500.f, 1.f, 8.f, 500.f};
const BuiltinDescriptor kDelayDescriptor{"delay", kParamIds, kReadouts.ids, &create, kIdle.v, kDefaults};

} // namespace saine
