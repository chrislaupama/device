// Soft Clipper: 4x oversampled clipping to a ceiling with three curves
// (Smooth, Warm, Hard), adjustable softness, input gain with optional gain
// match, dry/wet mix and output gain. Parameters in the order of
// saine_core::builtin::soft_clipper (checked by a Rust test).
//
//   wet = c * clip(x * a / c) * m       a = Input, c = Ceiling, m = 1/a with Gain Match
//   out = (dry * (1 - mix) + wet * mix) * Output
// clip() is the curve the panel draws (saine_core clip_curve):
//   Smooth  u / (1 + |u|^k)^(1/k),  k = 24 * (1/16)^softness
//   Warm    the same, but the negative half uses k' = max(k/2, 1.2) (even
//           harmonics); the DC this adds is tracked on the even part alone
//           (5 Hz) and subtracted, so no high-pass touches the clipped peaks
//   Hard    linear up to 1 - w, a quadratic knee to 1 + w, w = 0.01 + 0.49 * softness
// Latency 40 frames (oversampler), the dry path is aligned to it.
#include "builtin.hpp"
#include "dsp.hpp"
#include "oversample.hpp"

#include <algorithm>
#include <cmath>
#include <vector>

namespace saine {

namespace {

enum Param : uint32_t { kInput, kCeiling, kSoftness, kShape, kGainMatch, kMix, kOutput, kParamCount };
enum Shape : int { kSmooth, kWarm, kHard };

constexpr const char* kParamIds[kParamCount] = {"input", "ceiling", "softness", "shape", "gain_match", "mix", "output_gain"};

// a / (1 + a^k)^(1/k) for a >= 0.
inline float sat(float a, float k, float inv_k) noexcept {
    if (a < 1e-6f) return a;
    const float ak = dsp::fast_exp2(k * dsp::fast_log2(a));
    return a * dsp::fast_exp2(-inv_k * dsp::fast_log2(1.0f + ak));
}

inline float hard(float u, float w) noexcept {
    const float a = std::fabs(u);
    float y;
    if (a <= 1.0f - w) {
        y = a;
    } else if (a >= 1.0f + w) {
        y = 1.0f;
    } else {
        const float d = a - (1.0f - w);
        y = a - d * d / (4.0f * w);
    }
    return std::copysign(y, u);
}

struct Curve {
    int shape;
    float k, inv_k, kn, inv_kn, w;
    // Clipped value; for Warm also the even part (for its DC removal).
    float operator()(float u, float& even) const noexcept {
        even = 0.0f;
        switch (shape) {
        case kHard: return hard(u, w);
        case kWarm: {
            const float a = std::fabs(u);
            const float pos = sat(a, k, inv_k), neg = sat(a, kn, inv_kn);
            even = 0.5f * (pos - neg);
            return u >= 0.0f ? pos : -neg;
        }
        default: return std::copysign(sat(std::fabs(u), k, inv_k), u);
        }
    }
};

class SoftClipper final : public BuiltinProcessor {
public:
    void prepare(double sample_rate, uint32_t max_block) override {
        sr_ = sample_rate;
        block_ = std::max<uint32_t>(1, max_block);
        os_.prepare(block_);
        dry_buf_.assign(2 * static_cast<std::size_t>(block_), 0.0f);
        for (auto& d : dry_) d.prepare(dsp::Oversampler4x::kLatency);
        for (auto* s : {&in_, &ceil_, &soft_, &match_, &mix_, &out_}) s->set_time(20.0f, sr_);
        fade_step_ = 1.0f / static_cast<float>(0.02 * sr_);
        // DC tracker of the Warm even part: ~5 Hz one-pole at the 4x rate.
        dc_a_ = static_cast<float>(1.0 - std::exp(-6.283185307179586 * 5.0 / (4.0 * sr_)));
    }

    void reset() noexcept override {
        os_.reset();
        for (auto& d : dry_) d.reset();
        dc_[0] = dc_[1] = 0.0f;
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
    Curve curve(int shape, float s) const noexcept {
        Curve c;
        c.shape = shape;
        c.k = 24.0f * dsp::fast_exp2(-4.0f * s); // 24 * (1.5 / 24)^s
        c.inv_k = 1.0f / c.k;
        c.kn = std::max(0.5f * c.k, 1.2f);
        c.inv_kn = 1.0f / c.kn;
        c.w = 0.01f + 0.49f * s;
        return c;
    }

    void run(float* l, float* r, uint32_t n, const float* p) noexcept {
        const float a = dsp::db_to_gain(std::clamp(p[kInput], -48.0f, 48.0f));
        const float c = dsp::db_to_gain(std::clamp(p[kCeiling], -48.0f, 0.0f));
        const float s = std::clamp(p[kSoftness], 0.0f, 100.0f) * 0.01f;
        const int shape = std::clamp(static_cast<int>(std::lround(p[kShape])), 0, 2);
        const float match = p[kGainMatch] >= 0.5f ? 1.0f : 0.0f;
        const float mix = std::clamp(p[kMix], 0.0f, 100.0f) * 0.01f;
        const float out = dsp::db_to_gain(std::clamp(p[kOutput], -48.0f, 24.0f));
        if (!primed_) {
            in_.snap(a);
            ceil_.snap(c);
            soft_.snap(s);
            match_.snap(match);
            mix_.snap(mix);
            out_.snap(out);
            shape_ = prev_shape_ = shape;
            primed_ = true;
        }
        if (shape != shape_) {
            prev_shape_ = shape_;
            shape_ = shape;
            fade_ = 0.0f;
        }

        float* dl = dry_buf_.data();
        float* dr = dry_buf_.data() + block_;
        for (uint32_t i = 0; i < n; ++i) {
            dl[i] = dry_[0].process(l[i]);
            dr[i] = dry_[1].process(r[i]);
        }

        os_.up(l, r, n);
        float* hl = os_.left();
        float* hr = os_.right();
        float dc0 = dc_[0], dc1 = dc_[1];
        for (uint32_t i = 0; i < n; ++i) {
            const float ai = in_.next(a);
            const float ci = ceil_.next(c);
            const float si = soft_.next(s);
            const float mi = match_.next(match);
            const float pre = ai / ci;
            // Gain match blends 1 -> 1/a smoothly when toggled.
            const float post = ci * (1.0f + mi * (1.0f / ai - 1.0f));
            const Curve cur = curve(shape_, si);
            const Curve old = curve(prev_shape_, si);
            const float t = fade_;
            for (int k = 0; k < 4; ++k) {
                float* xs[2] = {hl + 4 * i + k, hr + 4 * i + k};
                float* dcs[2] = {&dc0, &dc1};
                for (int ch = 0; ch < 2; ++ch) {
                    const float u = *xs[ch] * pre;
                    float even = 0.0f, even_old = 0.0f;
                    float y = cur(u, even);
                    if (t < 1.0f) {
                        const float yo = old(u, even_old);
                        y = yo + t * (y - yo);
                        even = even_old + t * (even - even_old);
                    }
                    float& dc = *dcs[ch];
                    dc += dc_a_ * (even - dc);
                    *xs[ch] = (y - dc) * post;
                }
            }
            if (t < 1.0f) fade_ = std::min(1.0f, fade_ + fade_step_);
        }
        dc_[0] = std::fabs(dc0) < 1e-20f ? 0.0f : dc0;
        dc_[1] = std::fabs(dc1) < 1e-20f ? 0.0f : dc1;
        os_.down(l, r, n);

        for (uint32_t i = 0; i < n; ++i) {
            const float mx = mix_.next(mix);
            const float og = out_.next(out);
            l[i] = (dl[i] + mx * (l[i] - dl[i])) * og;
            r[i] = (dr[i] + mx * (r[i] - dr[i])) * og;
        }
    }

    double sr_ = 48000.0;
    uint32_t block_ = 1;
    dsp::Oversampler4x os_;
    dsp::DelayLine dry_[2];
    std::vector<float> dry_buf_;
    dsp::Smoother in_, ceil_, soft_, match_, mix_, out_;
    float dc_[2] = {0.0f, 0.0f};
    float dc_a_ = 0.0f;
    int shape_ = 0, prev_shape_ = 0;
    float fade_ = 1.0f, fade_step_ = 0.001f;
    bool primed_ = false;
};

std::unique_ptr<BuiltinProcessor> create_soft_clipper() { return std::make_unique<SoftClipper>(); }

} // namespace

extern const BuiltinDescriptor kSoftClipperDescriptor;
const BuiltinDescriptor kSoftClipperDescriptor{"soft-clipper", kParamIds, {}, &create_soft_clipper, {}};

} // namespace saine
