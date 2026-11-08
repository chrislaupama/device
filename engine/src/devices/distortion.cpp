// Distortion: a 4x oversampled waveshaper with four curves (Overdrive, Fuzz,
// Tube, Hard Clip), bias, a tone low-pass on the distorted signal, wet gain,
// dry/wet mix and output gain. Parameters in the order of
// saine_core::builtin::distortion (checked by a Rust test).
//
// Signal flow (per channel):
//   wet  = 4x[ f(x * g + o) - f(o) ]       g = 1 + 10 * drive, o = 0.5 * bias
//          (f with first-order antiderivative anti-aliasing on top of 4x)
//   wet  = DC block (10 Hz) -> 12 dB/oct low-pass at Tone -> * Gain
//   out  = (dry * (1 - mix) + wet * mix) * Output
// The curves are the ones the panel draws (saine_core distortion_transfer).
// The dry path is delayed by the oversampler latency so Mix never combs; the
// latency (40 frames) is reported for delay compensation.
#include "builtin.hpp"
#include "dsp.hpp"
#include "oversample.hpp"

#include <algorithm>
#include <cmath>

namespace saine {

namespace {

enum Param : uint32_t { kDrive, kTone, kMix, kGain, kMode, kBias, kOutput, kParamCount };
enum Mode : int { kOverdrive, kFuzz, kTube, kHardClip };

constexpr const char* kParamIds[kParamCount] = {"drive", "tone", "mix", "output", "mode", "bias", "output_gain"};
constexpr float kTwoOverPi = 0.636619772f;

inline float shape(int mode, float x, float asym) noexcept {
    switch (mode) {
    case kFuzz: return x >= 0.0f ? dsp::fast_tanh(2.0f * x) : std::atan(3.0f * x) * kTwoOverPi;
    case kTube: return x / (1.0f + std::fabs(x)) - asym * x * x;
    case kHardClip: return std::clamp(x, -1.0f, 1.0f);
    default: return dsp::fast_tanh(x);
    }
}

// Antiderivative of shape() (double: the ADAA difference quotient cancels).
inline double shape_integral(int mode, double x, double asym) noexcept {
    const double a = std::fabs(x);
    switch (mode) {
    case kFuzz:
        if (x >= 0.0) return 0.5 * (2.0 * a + std::log1p(std::exp(-4.0 * a)) - 0.6931471805599453);
        return 0.6366197723675814 * (x * std::atan(3.0 * x) - std::log1p(9.0 * x * x) / 6.0);
    case kTube: return a - std::log1p(a) - asym * x * x * x / 3.0;
    case kHardClip: return a <= 1.0 ? 0.5 * x * x : a - 0.5;
    default: return a + std::log1p(std::exp(-2.0 * a)) - 0.6931471805599453; // log cosh
    }
}

// First-order antiderivative anti-aliasing (ADAA) of one curve on one channel:
// y = (F(x) - F(x1)) / (x - x1), the curve at the midpoint when x ~ x1. On
// top of the 4x oversampling it pushes the aliases of hard curves down by
// another ~20 dB; it delays the wet signal by half a 4x frame (1/8 frame).
struct Adaa {
    double x1 = 0.0, f1 = 0.0;
    int mode = -1;
    double asym = 0.0;
    float process(int m, float x, float a) noexcept {
        const double xd = static_cast<double>(x), ad = static_cast<double>(a);
        if (m != mode || ad != asym) { // curve changed: restart from the previous input
            mode = m;
            asym = ad;
            f1 = shape_integral(m, x1, ad);
        }
        const double f = shape_integral(m, xd, ad);
        const double dx = xd - x1;
        float y;
        if (std::fabs(dx) > 1e-4) {
            y = static_cast<float>((f - f1) / dx);
        } else {
            y = shape(m, static_cast<float>(0.5 * (xd + x1)), a);
        }
        x1 = xd;
        f1 = f;
        return y;
    }
    void reset() noexcept {
        x1 = f1 = 0.0;
        mode = -1;
    }
};


class Distortion final : public BuiltinProcessor {
public:
    void prepare(double sample_rate, uint32_t max_block) override {
        sr_ = sample_rate;
        block_ = std::max<uint32_t>(1, max_block);
        os_.prepare(block_);
        dry_buf_.assign(2 * static_cast<std::size_t>(block_), 0.0f);
        for (auto& d : dry_) d.prepare(dsp::Oversampler4x::kLatency);
        for (auto* s : {&drive_, &bias_, &gain_, &mix_, &out_}) s->set_time(20.0f, sr_);
        tone_.set_time(30.0f, sr_);
        for (auto& b : dc_) b.set(10.0f, sr_);
        fade_step_ = 1.0f / static_cast<float>(0.02 * sr_);
    }

    void reset() noexcept override {
        os_.reset();
        for (auto& d : dry_) d.reset();
        for (auto& b : dc_) b.reset();
        for (auto& f : lp_) f.reset();
        for (auto& a : adaa_) a.reset();
        for (auto& a : adaa_prev_) a.reset();
        primed_ = false;
        fade_ = 1.0f;
    }

    uint32_t latency() const noexcept override { return dsp::Oversampler4x::kLatency; }

    void process(float* l, float* r, uint32_t n, const float* p, const BuiltinContext&, float*) noexcept override {
        for (uint32_t done = 0; done < n;) {
            const uint32_t m = std::min(block_, n - done);
            run(l + done, r + done, m, p);
            done += m;
        }
    }

private:
    void run(float* l, float* r, uint32_t n, const float* p) noexcept {
        const float drive = std::clamp(p[kDrive], 0.0f, 100.0f) * 0.01f;
        const float tone = std::log2(std::clamp(p[kTone], 20.0f, 20000.0f));
        const float mix = std::clamp(p[kMix], 0.0f, 100.0f) * 0.01f;
        const float gain = dsp::db_to_gain(std::clamp(p[kGain], -48.0f, 24.0f));
        const int mode = std::clamp(static_cast<int>(std::lround(p[kMode])), 0, 3);
        const float bias = std::clamp(p[kBias], 0.0f, 100.0f) * 0.01f;
        const float out = dsp::db_to_gain(std::clamp(p[kOutput], -48.0f, 24.0f));
        if (!primed_) {
            drive_.snap(drive);
            bias_.snap(bias);
            gain_.snap(gain);
            mix_.snap(mix);
            out_.snap(out);
            tone_.snap(tone);
            lp_coeffs_ = dsp::SvfCoeffs::lowpass(std::exp2(tone), 0.7071f, sr_);
            mode_ = prev_mode_ = mode;
            primed_ = true;
        }
        if (mode != mode_) { // crossfade the curves over 20 ms
            prev_mode_ = mode_;
            mode_ = mode;
            fade_ = 0.0f;
            for (int c = 0; c < 2; ++c) adaa_prev_[c] = adaa_[c];
        }

        // Dry path, aligned with the oversampler latency.
        float* dl = scratch_dry_l();
        float* dr = scratch_dry_r();
        for (uint32_t i = 0; i < n; ++i) {
            dl[i] = dry_[0].process(l[i]);
            dr[i] = dry_[1].process(r[i]);
        }

        // Shaper at 4x. Parameters are smoothed at the base rate and held
        // over the four oversampled frames of a base frame.
        os_.up(l, r, n);
        float* hl = os_.left();
        float* hr = os_.right();
        for (uint32_t i = 0; i < n; ++i) {
            const float g = 1.0f + 10.0f * drive_.next(drive);
            const float b = bias_.next(bias);
            const float o = 0.5f * b;
            const float asym = 0.5f * dsp::fast_tanh(2.0f * b);
            const float f0 = shape(mode_, o, asym);
            if (fade_ >= 1.0f) {
                for (int k = 0; k < 4; ++k) {
                    hl[4 * i + k] = adaa_[0].process(mode_, hl[4 * i + k] * g + o, asym) - f0;
                    hr[4 * i + k] = adaa_[1].process(mode_, hr[4 * i + k] * g + o, asym) - f0;
                }
            } else {
                const float t = fade_;
                const float p0 = shape(prev_mode_, o, asym);
                for (int k = 0; k < 4; ++k) {
                    const float xl = hl[4 * i + k] * g + o, xr = hr[4 * i + k] * g + o;
                    hl[4 * i + k] = (1.0f - t) * (adaa_prev_[0].process(prev_mode_, xl, asym) - p0) + t * (adaa_[0].process(mode_, xl, asym) - f0);
                    hr[4 * i + k] = (1.0f - t) * (adaa_prev_[1].process(prev_mode_, xr, asym) - p0) + t * (adaa_[1].process(mode_, xr, asym) - f0);
                }
                fade_ = std::min(1.0f, fade_ + fade_step_);
            }
        }
        os_.down(l, r, n);

        // Tone low-pass (coefficients interpolated across the block), DC block, gains, mix.
        const dsp::SvfCoeffs c0 = lp_coeffs_;
        float tone_now = tone_.y;
        for (uint32_t i = 0; i < n; ++i) tone_now = tone_.next(tone);
        const dsp::SvfCoeffs c1 = dsp::SvfCoeffs::lowpass(std::exp2(tone_now), 0.7071f, sr_);
        lp_coeffs_ = c1;
        const float inv_n = 1.0f / static_cast<float>(n);
        for (uint32_t i = 0; i < n; ++i) {
            const dsp::SvfCoeffs c = dsp::SvfCoeffs::lerp(c0, c1, static_cast<float>(i + 1) * inv_n);
            const float wg = gain_.next(gain);
            const float mx = mix_.next(mix);
            const float og = out_.next(out);
            const float wl = lp_[0].process(dc_[0].process(l[i]), c) * wg;
            const float wr = lp_[1].process(dc_[1].process(r[i]), c) * wg;
            l[i] = (dl[i] + mx * (wl - dl[i])) * og;
            r[i] = (dr[i] + mx * (wr - dr[i])) * og;
        }
        for (auto& b : dc_) b.flush_denormals();
        for (auto& f : lp_) f.flush_denormals();
    }

    float* scratch_dry_l() noexcept { return dry_buf_.data(); }
    float* scratch_dry_r() noexcept { return dry_buf_.data() + block_; }

    double sr_ = 48000.0;
    uint32_t block_ = 1;
    dsp::Oversampler4x os_;
    dsp::DelayLine dry_[2];
    std::vector<float> dry_buf_;
    dsp::Smoother drive_, bias_, gain_, mix_, out_, tone_;
    dsp::DcBlocker dc_[2];
    dsp::Svf lp_[2];
    Adaa adaa_[2], adaa_prev_[2];
    dsp::SvfCoeffs lp_coeffs_;
    int mode_ = 0, prev_mode_ = 0;
    float fade_ = 1.0f, fade_step_ = 0.001f;
    bool primed_ = false;
};

std::unique_ptr<BuiltinProcessor> create_distortion() { return std::make_unique<Distortion>(); }

} // namespace

extern const BuiltinDescriptor kDistortionDescriptor;
const BuiltinDescriptor kDistortionDescriptor{"distortion", kParamIds, {}, &create_distortion, {}};

} // namespace saine
