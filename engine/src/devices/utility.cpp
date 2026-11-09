// Utility: channel tools. Parameters in the order of
// saine_core::builtin::utility (checked by a Rust test).
//
// Per frame:
//   DC filter      5 Hz one-pole high-pass on both channels (switch crossfades)
//   channel matrix Stereo / Left / Right / Swap, then polarity per output side
//   width          M/S: S *= width (Mono: 0)
//   bass mono      S high-passed (Linkwitz-Riley 24 dB/oct) at the Bass Mono
//                  frequency: the lows of the side signal are removed, the
//                  mid is untouched, so nothing sums out of phase
//   audition       outputs only the mid below the frequency (LR 24 dB/oct low-pass)
//   balance        linear: the far side fades to silence at 50L / 50R
//   gain, mute
// Every switch and matrix entry is smoothed (10 ms), so toggles never click.
// No latency.
#include "builtin.hpp"
#include "dsp.hpp"

#include <algorithm>
#include <cmath>

namespace saine {

namespace {

enum Param : uint32_t {
    kGain,
    kBalance,
    kWidth,
    kChannelMode,
    kPhaseL,
    kPhaseR,
    kMono,
    kBassMono,
    kBassFreq,
    kAudition,
    kMute,
    kDcFilter,
    kParamCount
};

constexpr const char* kParamIds[kParamCount] = {"gain", "balance", "width", "channel_mode", "phase_l", "phase_r",
                                                "mono", "bass_mono", "bass_mono_freq", "bass_mono_audition", "mute", "dc_filter"};

enum Smooth : int { kLL, kLR, kRL, kRR, kWide, kBass, kAud, kGainL, kGainR, kDc, kSmoothCount };

class Utility final : public BuiltinProcessor {
public:
    void prepare(double sample_rate, uint32_t) override {
        sr_ = sample_rate;
        for (auto& s : sm_) s.set_time(10.0f, sr_);
        for (auto& b : dc_) b.set(5.0f, sr_);
    }

    void reset() noexcept override {
        for (auto& b : dc_) b.reset();
        for (auto& f : hp_) f.reset();
        for (auto& f : lp_) f.reset();
        primed_ = false;
    }

    void process(float* l, float* r, uint32_t n, const float* p, const BuiltinContext&, float*) noexcept override {
        auto on = [&](uint32_t i) { return p[i] >= 0.5f ? 1.0f : 0.0f; };
        const int mode = std::clamp(static_cast<int>(std::lround(p[kChannelMode])), 0, 3);
        // Output L = ll * L + lr * R, output R = rl * L + rr * R.
        float ll = 1, lr = 0, rl = 0, rr = 1;
        switch (mode) {
        case 1: rl = 1, rr = 0; break;         // Left on both sides
        case 2: ll = 0, lr = 1; break;         // Right on both sides
        case 3: ll = 0, lr = 1, rl = 1, rr = 0; break; // Swap
        default: break;
        }
        const float pl = on(kPhaseL) > 0 ? -1.0f : 1.0f, pr = on(kPhaseR) > 0 ? -1.0f : 1.0f;
        const float width = on(kMono) > 0 ? 0.0f : std::clamp(p[kWidth], 0.0f, 400.0f) * 0.01f;
        const float bal = std::clamp(p[kBalance], -50.0f, 50.0f) / 50.0f;
        const float gain = on(kMute) > 0 ? 0.0f : dsp::db_to_gain(std::clamp(p[kGain], -70.0f, 35.0f));
        const float target[kSmoothCount] = {ll * pl, lr * pl, rl * pr, rr * pr, width, on(kBassMono), on(kAudition),
                                            gain * std::min(1.0f, 1.0f - bal), gain * std::min(1.0f, 1.0f + bal), on(kDcFilter)};
        const float freq = std::clamp(p[kBassFreq], 20.0f, 2000.0f);
        if (!primed_) {
            for (int i = 0; i < kSmoothCount; ++i) sm_[i].snap(target[i]);
            hp_c_ = dsp::SvfCoeffs::highpass(freq, 0.7071f, sr_);
            lp_c_ = dsp::SvfCoeffs::lowpass(freq, 0.7071f, sr_);
            freq_ = freq;
            primed_ = true;
        }
        const dsp::SvfCoeffs hp0 = hp_c_, lp0 = lp_c_;
        if (freq != freq_) {
            hp_c_ = dsp::SvfCoeffs::highpass(freq, 0.7071f, sr_);
            lp_c_ = dsp::SvfCoeffs::lowpass(freq, 0.7071f, sr_);
            freq_ = freq;
        }
        const float inv_n = 1.0f / static_cast<float>(std::max<uint32_t>(1, n));
        for (uint32_t i = 0; i < n; ++i) {
            float s[kSmoothCount];
            for (int k = 0; k < kSmoothCount; ++k) s[k] = sm_[k].next(target[k]);
            const float t = static_cast<float>(i + 1) * inv_n;
            const dsp::SvfCoeffs hc = dsp::SvfCoeffs::lerp(hp0, hp_c_, t), lc = dsp::SvfCoeffs::lerp(lp0, lp_c_, t);
            float xl = l[i], xr = r[i];
            const float fl = dc_[0].process(xl), fr = dc_[1].process(xr);
            xl += s[kDc] * (fl - xl);
            xr += s[kDc] * (fr - xr);
            const float a = s[kLL] * xl + s[kLR] * xr;
            const float b = s[kRL] * xl + s[kRR] * xr;
            const float m = 0.5f * (a + b);
            float side = 0.5f * (a - b) * s[kWide];
            const float side_hp = hp_[1].process(hp_[0].process(side, hc), hc);
            side += s[kBass] * (side_hp - side);
            const float m_lo = lp_[1].process(lp_[0].process(m, lc), lc);
            float yl = m + side, yr = m - side;
            yl += s[kAud] * (m_lo - yl);
            yr += s[kAud] * (m_lo - yr);
            l[i] = yl * s[kGainL];
            r[i] = yr * s[kGainR];
        }
        // Land exactly on the targets (a muted output becomes true silence).
        for (int k = 0; k < kSmoothCount; ++k)
            if (std::fabs(sm_[k].y - target[k]) < 1e-7f) sm_[k].snap(target[k]);
        for (auto& b : dc_) b.flush_denormals();
        for (auto& f : hp_) f.flush_denormals();
        for (auto& f : lp_) f.flush_denormals();
    }

private:
    double sr_ = 48000.0;
    dsp::Smoother sm_[kSmoothCount];
    dsp::DcBlocker dc_[2];
    dsp::Svf hp_[2], lp_[2]; // two cascaded Butterworth sections = Linkwitz-Riley 4th order
    dsp::SvfCoeffs hp_c_, lp_c_;
    float freq_ = 120.0f;
    bool primed_ = false;
};

std::unique_ptr<BuiltinProcessor> create_utility() { return std::make_unique<Utility>(); }

} // namespace

extern const BuiltinDescriptor kUtilityDescriptor;
constexpr float kDefaults[] = {0.f, 0.f, 100.f, 0.f, 0.f, 0.f, 0.f, 0.f, 120.f, 0.f, 0.f, 0.f};
const BuiltinDescriptor kUtilityDescriptor{"utility", kParamIds, {}, &create_utility, {}, kDefaults};

} // namespace saine
