// Reverb: six algorithms on one engine. Parameters in the order of
// saine_core::builtin::reverb (checked by a Rust test).
//
//   input -> pre-delay line (also feeds the early-reflection taps)
//         -> input diffusion (4 allpasses per side; longer and denser in Haze)
//         -> 16-line feedback delay network, Hadamard mixing, per-line
//            modulated reads and two-band absorption filters set from the
//            reference's T60 law: T60(f) = Decay / (1 + damping * (b(f) - b(1 kHz)) * 0.85)
//         -> wet = late tail + early reflections (arrival times from the
//            reference's tail model), width (M/S), low / high cut (1st order,
//            the reference's filter law), mix with the dry signal.
//
// Mode specifics: Room / Hall / Plate differ in line lengths, early
// reflections, diffusion and modulation; Plate and Haze add an allpass in
// every feedback line (density); Spring runs two dispersive spring tanks
// (stretched allpass chirps in a feedback loop) over a quiet diffuse bed;
// Shimmer feeds the tail through two pitch shifters (the Pitch Shifter's
// phase vocoder) back into the network; Haze smears the onset with long
// diffusion set by Diffusion.
//
// When the input has been silent for longer than the tail can ring, the
// network is cleared once and skipped (dry pass-through) until sound returns.
#include "builtin.hpp"
#include "dsp.hpp"
#include "dsp_delay.hpp"
#include "pitch_shift.hpp"
#include "spectral.hpp"

#include <algorithm>
#include <array>
#include <cmath>

namespace saine {

namespace {

enum Param : uint32_t {
    kSize, kDecay, kDamping, kPredelay, kMix, kWidth, kMode, kHighCut, kLowCut, kDry, kModDepth, kModRate,
    kShift1, kShift2, kShimmer, kDiffusion, kParamCount
};
constexpr const char* kParamIds[kParamCount] = {"size", "decay", "damping", "predelay", "mix", "width", "mode2", "highcut2",
                                                "lowcut2", "dry", "moddepth", "modrate", "shift1", "shift2", "shimmer", "diffusion"};
// Readouts: wet peak (dB), then the wet spectrum bands (dB, dsp::SpectrumTap).
constexpr uint32_t kBands = dsp::kSpectrumBands;
struct ReadoutIds {
    const char* ids[1 + kBands];
    constexpr ReadoutIds() : ids{"tail_db"} {
        for (uint32_t i = 0; i < kBands; ++i) ids[1 + i] = dsp::kSpectrumIds[i];
    }
};
constexpr ReadoutIds kReadouts{};
enum Mode : int { kRoom, kHall, kPlate, kSpring, kShimmerMode, kHaze };

constexpr int kLines = 16;
// Base line lengths (ms) at scale 1: spread over ~1:3.2, mutually detuned.
constexpr float kLineMs[kLines] = {31.7f, 37.1f, 41.3f, 43.9f, 47.9f, 53.3f, 59.1f, 61.7f,
                                   67.3f, 71.9f, 73.7f, 79.3f, 83.9f, 89.1f, 97.3f, 101.9f};
constexpr float kLoopApMs[kLines] = {1.3f, 2.1f, 1.7f, 2.9f, 1.1f, 2.5f, 3.3f, 1.9f, 2.3f, 1.5f, 3.1f, 2.7f, 1.2f, 3.7f, 2.2f, 1.6f};
constexpr float kDiffMs[4] = {4.77f, 3.59f, 12.73f, 9.31f};

struct ModeSpec {
    float scale_min, scale_max; // line-length scale over Size 0..1
    float er_level;
    float diffusion;            // input allpass gain
    float loop_ap;              // in-loop allpass gain (0 = none)
    float mod_ms;               // modulation depth at Mod Depth 100%
    float mod_hz;               // modulation rate at Mod Rate 1x
    float first_s, spacing_s;   // early-reflection model (reference tail model constants)
    float gain;                 // wet level trim
};
constexpr ModeSpec kModes[6] = {
    {0.12f, 0.50f, 0.65f, 0.62f, 0.00f, 0.18f, 0.60f, 0.060f, 0.0060f, 0.95f}, // Room
    {0.35f, 1.00f, 0.42f, 0.66f, 0.00f, 0.45f, 0.45f, 0.045f, 0.0120f, 0.85f}, // Hall
    {0.15f, 0.48f, 0.12f, 0.75f, 0.55f, 0.30f, 0.80f, 0.005f, 0.0030f, 0.90f}, // Plate
    {0.20f, 0.50f, 0.00f, 0.50f, 0.00f, 0.40f, 0.50f, 0.040f, 0.0220f, 0.85f}, // Spring
    {0.35f, 1.00f, 0.25f, 0.66f, 0.00f, 0.60f, 0.35f, 0.000f, 0.0100f, 0.80f}, // Shimmer
    {0.40f, 1.20f, 0.10f, 0.60f, 0.45f, 1.20f, 0.25f, 0.000f, 0.0015f, 0.80f}, // Haze
};
constexpr float kMaxScale = 1.2f;

// b(f) of the reference T60 law.
inline float law_b(float f) noexcept { return 0.5f * std::log2(1.0f + (f / 250.0f) * (f / 250.0f)); }

// 16-point fast Walsh-Hadamard transform, orthonormal (scale 1/4).
inline void hadamard16(float* x) noexcept {
    for (int len = 1; len < kLines; len <<= 1)
        for (int i = 0; i < kLines; i += len << 1)
            for (int j = i; j < i + len; ++j) {
                const float a = x[j], b = x[j + len];
                x[j] = a + b;
                x[j + len] = a - b;
            }
    for (int i = 0; i < kLines; ++i) x[i] *= 0.25f;
}

// Stretched first-order allpass (a + z^-M) / (1 + a z^-M): one stage of a spring's dispersion.
struct StretchedAp {
    static constexpr int kM = 2;
    float xs[kM] = {}, ys[kM] = {};
    int i = 0;
    float process(float x, float a) noexcept {
        const float y = a * x + xs[i] - a * ys[i];
        xs[i] = x;
        ys[i] = dsp::flush(y);
        i = (i + 1) % kM;
        return y;
    }
    void reset() noexcept {
        for (int k = 0; k < kM; ++k) xs[k] = ys[k] = 0.0f;
    }
};

class Reverb final : public BuiltinProcessor {
public:
    void prepare(double sample_rate, uint32_t max_block) override {
        sr_ = sample_rate;
        const auto ms = [&](float v) { return static_cast<std::size_t>(static_cast<double>(v) * 0.001 * sr_) + 8; };
        for (auto& d : pre_) d.prepare(ms(1000.0f));
        for (int i = 0; i < kLines; ++i) {
            line_[i].prepare(ms(kLineMs[i] * kMaxScale + 8.0f));
            loop_ap_[i].allocate(ms(kLoopApMs[i] * 2.0f));
        }
        for (auto& side : diff_)
            for (int k = 0; k < 4; ++k) side[k].allocate(ms(kDiffMs[k] * 6.0f + 2.0f));
        for (auto& s : spring_) s.prepare(ms(90.0f));
        for (auto& s : shifter_) s.prepare(sample_rate > 60000.0 ? 2048 : 1024);
        wet_mono_.assign(max_block, 0.0f);
        tap_.prepare(sample_rate);
        mix_.set_time(25.0f, sr_);
        width_.set_time(25.0f, sr_);
        pre_ms_.set_time(60.0f, sr_);
        reset();
    }

    void reset() noexcept override {
        for (auto& d : pre_) d.reset();
        for (int i = 0; i < kLines; ++i) {
            line_[i].reset();
            loop_ap_[i].clear();
            damp_[i] = 0.0f;
            lfo_[i] = static_cast<float>(i) / kLines;
        }
        for (auto& side : diff_)
            for (auto& a : side) a.clear();
        for (auto& s : spring_) s.reset();
        for (auto& side : disp_)
            for (auto& a : side) a.reset();
        for (auto& f : spring_lp_) f.reset();
        for (auto& s : shifter_) s.reset();
        shim_fb_ = 0.0f;
        for (auto& f : lc_) f.reset();
        for (auto& f : hc_) f.reset();
        tap_.reset();
        quiet_ = 0;
        sleeping_ = false;
        primed_ = false;
    }

    void process(float* l, float* r, uint32_t n, const float* p, const BuiltinContext&, float* ro) noexcept override {
        const int mode = std::clamp(static_cast<int>(std::lround(p[kMode])), 0, 5);
        const ModeSpec& ms = kModes[mode];
        const float size = std::clamp(p[kSize], 0.0f, 100.0f) * 0.01f;
        const float decay = std::clamp(p[kDecay], 0.05f, 20.0f);
        const float damping = std::clamp(p[kDamping], 0.0f, 100.0f) * 0.01f;
        const float pre_ms = std::clamp(p[kPredelay], 0.0f, 100.0f);
        const float mix = std::clamp(p[kMix], 0.0f, 100.0f) * 0.01f;
        const float width = std::clamp(p[kWidth], 0.0f, 100.0f) * 0.01f;
        const float hc = std::clamp(p[kHighCut], 20.0f, 20000.0f);
        const float lc = std::clamp(p[kLowCut], 20.0f, 20000.0f);
        const bool dry_on = p[kDry] >= 0.5f;
        const float mod_depth = std::clamp(p[kModDepth], 0.0f, 200.0f) * 0.01f;
        const float mod_rate = std::clamp(p[kModRate], 0.1f, 8.0f);
        const float shimmer = std::clamp(p[kShimmer], 0.0f, 100.0f) * 0.01f;
        const float diffusion = std::clamp(p[kDiffusion], 0.0f, 100.0f) * 0.01f;
        const int s1 = std::clamp(static_cast<int>(std::lround(p[kShift1])), 0, 9);
        const int s2 = std::clamp(static_cast<int>(std::lround(p[kShift2])), 0, 10);
        constexpr float kShiftSt[10] = {-12.0f, -7.0f, -5.0f, -0.1f, 0.1f, 5.0f, 7.0f, 12.0f, 19.0f, 24.0f};

        if (!primed_) {
            mix_.snap(mix);
            width_.snap(width);
            pre_ms_.snap(pre_ms);
            primed_ = true;
        }

        // Silence gate: skip the network once the tail has surely died out.
        const float in_peak = dsp::stereo_peak(l, r, n);
        const double ring_s = 1.5 * static_cast<double>(decay) * (mode == kShimmerMode ? 1.0 + 3.0 * static_cast<double>(shimmer) : 1.0) + 0.6;
        if (in_peak > 1e-6f) {
            quiet_ = 0;
            sleeping_ = false;
        } else if (!sleeping_) {
            quiet_ += n;
            if (static_cast<double>(quiet_) > ring_s * sr_) {
                clear_tail();
                sleeping_ = true;
            }
        }
        if (sleeping_) {
            const float m = mix_.next(mix); // keep the smoother moving
            if (!dry_on) {
                for (uint32_t i = 0; i < n; ++i) l[i] = r[i] = 0.0f;
            } else {
                for (uint32_t i = 0; i < n; ++i) {
                    l[i] *= 1.0f - m;
                    r[i] *= 1.0f - m;
                }
            }
            const auto m_cap = std::min<uint32_t>(n, static_cast<uint32_t>(wet_mono_.size()));
            std::fill(wet_mono_.begin(), wet_mono_.begin() + m_cap, 0.0f);
            tap_.push(wet_mono_.data(), wet_mono_.data(), m_cap);
            ro[0] = -120.0f;
            tap_.write(ro + 1);
            return;
        }

        // Per-block coefficients.
        const float scale = ms.scale_min + (ms.scale_max - ms.scale_min) * size;
        const float hf_factor = 1.0f + damping * std::max(0.0f, law_b(static_cast<float>(0.5 * sr_)) - law_b(1000.0f)) * 0.85f;
        float len[kLines], g_lo[kLines], pole[kLines];
        for (int i = 0; i < kLines; ++i) {
            len[i] = kLineMs[i] * scale * 0.001f * static_cast<float>(sr_);
            const float seconds = len[i] / static_cast<float>(sr_);
            g_lo[i] = std::pow(10.0f, -3.0f * seconds / decay);
            const float g_hi = std::pow(10.0f, -3.0f * seconds * hf_factor / decay);
            const float ratio = g_hi / g_lo[i];
            pole[i] = (1.0f - ratio) / (1.0f + ratio);
        }
        const float mod_samples = ms.mod_ms * mod_depth * 0.001f * static_cast<float>(sr_);
        const float lfo_inc = ms.mod_hz * mod_rate / static_cast<float>(sr_);
        const float loop_ap_g = mode == kHaze ? 0.35f + 0.35f * diffusion : ms.loop_ap;
        const float diff_g = mode == kHaze ? 0.45f + 0.3f * diffusion : ms.diffusion;
        const float diff_len_scale = mode == kHaze ? 1.0f + 5.0f * diffusion : (0.6f + 0.6f * size);
        // Early reflections: 8 arrivals per side (reference tail model).
        float er_t[8], er_g[8];
        for (int i = 0; i < 8; ++i) {
            const float t = ms.first_s * (0.6f + size) / 1.1f + ms.spacing_s * std::pow(static_cast<float>(i + 1), 1.1f) * (0.4f + 1.2f * size);
            er_t[i] = t * static_cast<float>(sr_);
            er_g[i] = ms.er_level * std::pow(0.82f, static_cast<float>(i)) * ((i & 1) ? -0.8f : 1.0f) * std::pow(10.0f, -3.0f * t / decay);
        }
        // Spring tanks.
        const float spring_len = (28.0f + 38.0f * size) * 0.001f * static_cast<float>(sr_);
        const float spring_g = std::pow(10.0f, -3.0f * (spring_len / static_cast<float>(sr_)) / decay);
        const float spring_disp = -0.55f - 0.15f * size;
        spring_lp_[0].set_hz(4200.0f - 2600.0f * damping, sr_);
        spring_lp_[1].a = spring_lp_[0].a;
        // Output filters (the reference's first-order law); Low Cut at 20 Hz is off.
        const bool lc_on = lc > 20.02f;
        for (auto& f : lc_) f.set_hz(lc, sr_);
        for (auto& f : hc_) f.set_hz(hc, sr_);
        // Shimmer voices: the tail is shifted sample by sample and fed back one sample later,
        // so the feedback delay is the shifter latency whatever the block split.
        const bool shim_on = mode == kShimmerMode && shimmer > 0.0f;
        if (shim_on) {
            shifter_[0].set_ratio(std::exp2(kShiftSt[s1] / 12.0f));
            shifter_[1].set_ratio(s2 > 0 ? std::exp2(kShiftSt[s2 - 1] / 12.0f) : 1.0f);
        }
        const float shim_gain = shimmer * 0.55f;
        const float out_gain = ms.gain * 0.7f;
        const uint32_t cap = static_cast<uint32_t>(wet_mono_.size());

        float wet_peak = 0.0f;
        for (uint32_t i = 0; i < n; ++i) {
            const float xl = l[i], xr = r[i];
            const float pd = std::max(1.0f, pre_ms_.next(pre_ms) * 0.001f * static_cast<float>(sr_));
            pre_[0].push(xl);
            pre_[1].push(xr);
            float il = pre_[0].read(pd + 1.0f), ir = pre_[1].read(pd + 1.0f);
            // Early reflections from the pre-delayed input.
            float erl = 0.0f, err = 0.0f;
            if (ms.er_level > 0.0f) {
                for (int k = 0; k < 8; ++k) {
                    erl += er_g[k] * pre_[k & 1].read_linear(pd + 1.0f + er_t[k]);
                    err += er_g[k] * pre_[(k + 1) & 1].read_linear(pd + 1.0f + er_t[k] * 1.07f);
                }
            }
            if (shim_on) {
                const float s = shim_fb_ * shim_gain;
                il += s;
                ir += s;
            }
            // Input diffusion.
            for (int k = 0; k < 4; ++k) {
                const float d = kDiffMs[k] * diff_len_scale * 0.001f * static_cast<float>(sr_);
                il = diff_[0][k].process(il, d, diff_g);
                ir = diff_[1][k].process(ir, d * 1.031f, diff_g);
            }
            float tl = 0.0f, tr = 0.0f;
            if (mode == kSpring) {
                // Two dispersive tanks over a quiet diffuse bed.
                for (int c = 0; c < 2; ++c) {
                    const float len_c = spring_len * (c == 0 ? 1.0f : 1.137f);
                    float y = spring_[c].read_linear(len_c - 1.0f);
                    y = spring_lp_[c].lp(y);
                    float x = (c == 0 ? il : ir) + dsp::soft_clip(y * spring_g * 0.98f);
                    for (auto& ap : disp_[c]) x = ap.process(x, spring_disp);
                    spring_[c].push(dsp::flush(x));
                    (c == 0 ? tl : tr) += y * 0.9f;
                }
            }
            // Late network.
            float v[kLines];
            for (int k = 0; k < kLines; ++k) {
                lfo_[k] += lfo_inc * (1.0f + 0.07f * static_cast<float>(k));
                if (lfo_[k] >= 1.0f) lfo_[k] -= 1.0f;
                const float d = len[k] + mod_samples * (1.0f + dsp::fast_sin(lfo_[k]));
                float y = line_[k].read_linear(d + 1.0f);
                damp_[k] = y + pole[k] * (damp_[k] - y); // two-band absorption
                y = damp_[k] * g_lo[k];
                v[k] = y;
            }
            const float bed = mode == kSpring ? 0.3f : 1.0f;
            float late_l = 0.0f, late_r = 0.0f;
            for (int k = 0; k < kLines; k += 2) {
                late_l += v[k] - v[k + 1] * 0.5f;
                late_r += v[k + 1] - v[k] * 0.5f;
            }
            hadamard16(v);
            for (int k = 0; k < kLines; ++k) {
                float x = v[k] + ((k & 1) ? ir : il) * (((k >> 1) & 1) ? -0.35f : 0.35f);
                if (loop_ap_g > 0.0f) x = loop_ap_[k].process(x, kLoopApMs[k] * 0.001f * static_cast<float>(sr_), loop_ap_g);
                line_[k].push(dsp::flush(x));
            }
            tl += bed * late_l * 0.35f + erl;
            tr += bed * late_r * 0.35f + err;
            tl *= out_gain;
            tr *= out_gain;
            const float mono = 0.5f * (tl + tr);
            if (i < cap) wet_mono_[i] = mono;
            if (shim_on) {
                float a = 0.0f, b = 0.0f;
                shifter_[0].process(&mono, &a, 1);
                if (s2 > 0) shifter_[1].process(&mono, &b, 1);
                shim_fb_ = dsp::soft_clip((s2 > 0 ? 0.5f * (a + b) : a) * 1.6f) * 0.625f;
            }
            // Width, filters, mix.
            const float wm = 0.5f * (tl + tr), wsd = 0.5f * (tl - tr) * width_.next(width);
            tl = wm + wsd;
            tr = wm - wsd;
            if (lc_on) {
                tl = lc_[0].hp(tl);
                tr = lc_[1].hp(tr);
            }
            tl = hc_[0].lp(tl);
            tr = hc_[1].lp(tr);
            wet_peak = std::max(wet_peak, std::max(std::fabs(tl), std::fabs(tr)));
            const float m = mix_.next(mix);
            const float dry = dry_on ? 1.0f - m : 0.0f;
            l[i] = xl * dry + tl * m;
            r[i] = xr * dry + tr * m;
        }
        for (auto& d : damp_) d = dsp::flush(d);
        for (auto& f : lc_) f.y = dsp::flush(f.y);
        for (auto& f : hc_) f.y = dsp::flush(f.y);
        for (auto& f : spring_lp_) f.y = dsp::flush(f.y);

        const uint32_t m_cap = std::min(n, cap);
        if (!shim_on && shim_was_on_) {
            shim_fb_ = 0.0f;
            for (auto& s : shifter_) s.reset();
        }
        shim_was_on_ = shim_on;
        tap_.push(wet_mono_.data(), wet_mono_.data(), m_cap);
        ro[0] = dsp::gain_to_db(wet_peak);
        tap_.write(ro + 1);
    }

private:
    void clear_tail() noexcept {
        for (int i = 0; i < kLines; ++i) {
            line_[i].reset();
            loop_ap_[i].clear();
            damp_[i] = 0.0f;
        }
        for (auto& side : diff_)
            for (auto& a : side) a.clear();
        for (auto& s : spring_) s.reset();
        for (auto& side : disp_)
            for (auto& a : side) a.reset();
        for (auto& s : shifter_) s.reset();
        shim_fb_ = 0.0f;
    }

    double sr_ = 48000.0;
    dsp::DelayLine pre_[2];
    dsp::DelayLine line_[kLines];
    dsp::Allpass loop_ap_[kLines];
    dsp::Allpass diff_[2][4];
    float damp_[kLines] = {};
    float lfo_[kLines] = {};
    dsp::DelayLine spring_[2];
    std::array<StretchedAp, 12> disp_[2];
    dsp::ExpOnePole spring_lp_[2];
    dsp::PitchShifter shifter_[2];
    std::vector<float> wet_mono_;
    float shim_fb_ = 0.0f;
    bool shim_was_on_ = false;
    dsp::ExpOnePole lc_[2], hc_[2];
    dsp::SpectrumTap tap_;
    dsp::Smoother mix_, width_, pre_ms_;
    uint64_t quiet_ = 0;
    bool sleeping_ = false;
    bool primed_ = false;
};

std::unique_ptr<BuiltinProcessor> create() { return std::make_unique<Reverb>(); }

} // namespace

extern const BuiltinDescriptor kReverbDescriptor;
struct IdleReadouts {
    float v[1 + kBands];
    constexpr IdleReadouts() : v{-120.0f} {
        for (uint32_t i = 1; i < 1 + kBands; ++i) v[i] = -120.0f;
    }
};
constexpr IdleReadouts kIdle{};
constexpr float kDefaults[kParamCount] = {50.f, 2.f, 50.f, 10.f, 30.f, 100.f, 1.f, 12000.f, 20.f, 1.f, 20.f, 1.f, 7.f, 10.f, 60.f, 50.f};
const BuiltinDescriptor kReverbDescriptor{"reverb", kParamIds, kReadouts.ids, &create, kIdle.v, kDefaults};

} // namespace saine
