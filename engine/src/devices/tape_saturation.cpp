// Tape Saturation: a tape-machine colour stage. Parameters in the order of
// dstudio_core::builtin::tape_saturation (checked by a Rust test).
//
// Per channel:
//   pre-emphasis   high shelf +E dB (the machine's record EQ), so highs
//                  saturate earlier, undone after the tape stage
//   tape comp.     slow RMS leveller (30 ms / 300 ms) before the tape,
//                  up to 4:1 above -20 dBFS with Compression
//   tape stage     4x oversampled: s = sat(x * g) + k s^2 (the mode's curve,
//                  as drawn by the panel), then a soft ceiling 1 - 0.5 * comp
//   de-emphasis    the inverse shelf, a 10 Hz DC block (the even term)
//   machine        head bump (bell) and high-shelf softening scaled by Warmth,
//                  a bandwidth low-pass per mode
//   hiss           shaped white noise at the Noise level (-60 dB = off)
//   out            (dry * (1 - mix) + wet * mix) * Output
// Modes: Reel-to-Reel (g = 1 + 5d, clean, 20 kHz), Cassette (g = 1 + 8d,
// darker, 14 kHz), VHS (g = 1 + 6d, soft knee curve, 9 kHz).
// Latency 40 frames (oversampler), the dry path is aligned to it.
#include "builtin.hpp"
#include "dsp.hpp"
#include "oversample.hpp"

#include <algorithm>
#include <cmath>
#include <vector>

namespace dstudio {

namespace {

enum Param : uint32_t { kDrive, kWarmth, kCompression, kNoise, kMix, kMode, kOutput, kParamCount };
enum Mode : int { kReel, kCassette, kVhs };

constexpr const char* kParamIds[kParamCount] = {"drive", "warmth", "compression", "noise", "mix", "mode", "output_gain"};
constexpr float kTwoOverPi = 0.636619772f;

struct Voice {
    float drive_gain;   // g = 1 + drive_gain * d
    float even;         // k of the even-order term
    float emphasis_db;  // record pre-emphasis
    float emphasis_hz;
    float bump_hz;      // head bump centre
    float bump_db;      // at Warmth 100%
    float soft_hz;      // high shelf corner
    float soft_db;      // at Warmth 100% (negative)
    float bandwidth_hz; // machine low-pass
    float hiss_hz;      // hiss high-pass corner
};

constexpr Voice kVoices[3] = {
    {5.0f, 0.05f, 4.0f, 6000.0f, 90.0f, 4.5f, 9000.0f, -6.0f, 20000.0f, 300.0f},
    {8.0f, 0.05f, 6.0f, 4000.0f, 70.0f, 5.0f, 6000.0f, -8.0f, 14000.0f, 600.0f},
    {6.0f, 0.03f, 5.0f, 3000.0f, 110.0f, 3.5f, 4000.0f, -9.0f, 9000.0f, 900.0f},
};

inline float tape_curve(int mode, float t) noexcept {
    switch (mode) {
    case kCassette: return 0.9f * dsp::fast_tanh(t) + 0.1f * std::atan(2.0f * t) * kTwoOverPi;
    case kVhs: return t / (1.0f + std::fabs(t));
    default: return dsp::fast_tanh(t);
    }
}

// Soft ceiling: linear to 0.85 C, a quadratic knee that flattens at C.
inline float ceiling(float s, float c) noexcept {
    const float a = std::fabs(s), w = 0.15f * c, lo = c - w;
    float y;
    if (a <= lo) {
        y = a;
    } else if (a >= c + w) {
        y = c;
    } else {
        const float d = a - lo;
        y = a - d * d / (4.0f * w);
    }
    return std::copysign(y, s);
}

class TapeSaturation final : public BuiltinProcessor {
public:
    void prepare(double sample_rate, uint32_t max_block) override {
        sr_ = sample_rate;
        block_ = std::max<uint32_t>(1, max_block);
        os_.prepare(block_);
        dry_buf_.assign(2 * static_cast<std::size_t>(block_), 0.0f);
        for (auto& d : dry_) d.prepare(dsp::Oversampler4x::kLatency);
        for (auto* s : {&drive_, &warmth_, &comp_, &noise_, &mix_, &out_}) s->set_time(20.0f, sr_);
        for (auto& b : dc_) b.set(10.0f, sr_);
        rms_a_ = dsp::pole(30.0f, sr_);
        rel_a_ = dsp::pole(300.0f, sr_);
        att_a_ = dsp::pole(30.0f, sr_);
        fade_step_ = 1.0f / static_cast<float>(0.03 * sr_);
        hiss_[0].s = 0x1234567u;
        hiss_[1].s = 0x89abcdefu;
    }

    void reset() noexcept override {
        os_.reset();
        for (auto& d : dry_) d.reset();
        for (auto& b : dc_) b.reset();
        for (auto* f : {&pre_[0], &pre_[1], &post_[0], &post_[1], &bump_[0], &bump_[1], &soft_[0], &soft_[1], &bw_[0], &bw_[1],
                        &hp_[0], &hp_[1], &hlp_[0], &hlp_[1]})
            f->reset();
        ms_ = 0.0f;
        gr_ = 1.0f;
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
    struct Eq {
        dsp::SvfCoeffs pre, post, bump, soft, bw, hp, hlp;
    };
    Eq eq_for(int mode, float warmth) const noexcept {
        const Voice& v = kVoices[mode];
        Eq e;
        e.pre = dsp::SvfCoeffs::highshelf(v.emphasis_hz, 0.7071f, v.emphasis_db, sr_);
        e.post = dsp::SvfCoeffs::highshelf(v.emphasis_hz, 0.7071f, -v.emphasis_db, sr_);
        e.bump = dsp::SvfCoeffs::bell(v.bump_hz, 0.9f, v.bump_db * warmth, sr_);
        e.soft = dsp::SvfCoeffs::highshelf(v.soft_hz, 0.6f, v.soft_db * warmth, sr_);
        e.bw = dsp::SvfCoeffs::lowpass(v.bandwidth_hz, 0.7071f, sr_);
        e.hp = dsp::SvfCoeffs::highpass(v.hiss_hz, 0.7071f, sr_);
        e.hlp = dsp::SvfCoeffs::lowpass(v.bandwidth_hz * 0.8f, 0.7071f, sr_);
        return e;
    }

    void run(float* l, float* r, uint32_t n, const float* p) noexcept {
        const float drive = std::clamp(p[kDrive], 0.0f, 100.0f) * 0.01f;
        const float warmth = std::clamp(p[kWarmth], 0.0f, 100.0f) * 0.01f;
        const float comp = std::clamp(p[kCompression], 0.0f, 100.0f) * 0.01f;
        const float noise_db = std::clamp(p[kNoise], -60.0f, 0.0f);
        const float noise = noise_db <= -59.999f ? 0.0f : dsp::db_to_gain(noise_db);
        const float mix = std::clamp(p[kMix], 0.0f, 100.0f) * 0.01f;
        const int mode = std::clamp(static_cast<int>(std::lround(p[kMode])), 0, 2);
        const float out = dsp::db_to_gain(std::clamp(p[kOutput], -48.0f, 24.0f));
        if (!primed_) {
            drive_.snap(drive);
            warmth_.snap(warmth);
            comp_.snap(comp);
            noise_.snap(noise);
            mix_.snap(mix);
            out_.snap(out);
            mode_ = prev_mode_ = mode;
            eq_ = eq_for(mode, warmth);
            primed_ = true;
        }
        if (mode != mode_) {
            prev_mode_ = mode_;
            mode_ = mode;
            fade_ = 0.0f;
        }
        // Filters follow the mode and the (block-smoothed) warmth; the curve
        // crossfades between modes.
        float w_end = warmth_.y;
        for (uint32_t i = 0; i < n; ++i) w_end = warmth_.next(warmth);
        const Eq e0 = eq_;
        const Eq e1 = eq_for(mode_, w_end);
        eq_ = e1;
        const float inv_n = 1.0f / static_cast<float>(n);

        float* dl = dry_buf_.data();
        float* dr = dry_buf_.data() + block_;
        // Pre-emphasis and the slow leveller at the base rate.
        float ms = ms_, gr = gr_;
        for (uint32_t i = 0; i < n; ++i) {
            const float t = static_cast<float>(i + 1) * inv_n;
            const dsp::SvfCoeffs pre = dsp::SvfCoeffs::lerp(e0.pre, e1.pre, t);
            dry_[0].push(l[i]);
            dl[i] = dry_[0].at(dsp::Oversampler4x::kLatency);
            dry_[1].push(r[i]);
            dr[i] = dry_[1].at(dsp::Oversampler4x::kLatency);
            const float cmp = comp_.next(comp);
            const float xl = pre_[0].process(l[i], pre), xr = pre_[1].process(r[i], pre);
            const float x2 = 0.5f * (xl * xl + xr * xr);
            ms = x2 + rms_a_ * (ms - x2);
            const float level_db = dsp::power_to_db(ms);
            const float over = std::max(0.0f, level_db + 20.0f);
            const float slope = 0.75f * cmp; // up to 4:1
            const float target = dsp::db_to_gain(-slope * over);
            gr = target + (target < gr ? att_a_ : rel_a_) * (gr - target);
            l[i] = xl * gr;
            r[i] = xr * gr;
        }
        ms_ = ms < 1e-20f ? 0.0f : ms;
        gr_ = gr;

        // Tape stage at 4x.
        os_.up(l, r, n);
        float* hl = os_.left();
        float* hr = os_.right();
        for (uint32_t i = 0; i < n; ++i) {
            const float d = drive_.next(drive);
            const float c = 1.0f - 0.5f * comp_.y;
            const Voice& v = kVoices[mode_];
            const Voice& vo = kVoices[prev_mode_];
            const float g = 1.0f + v.drive_gain * d, go = 1.0f + vo.drive_gain * d;
            const float t = fade_;
            for (int k = 0; k < 4; ++k) {
                for (float* x : {hl + 4 * i + k, hr + 4 * i + k}) {
                    float s = tape_curve(mode_, *x * g);
                    s += v.even * s * s;
                    if (t < 1.0f) {
                        float so = tape_curve(prev_mode_, *x * go);
                        so += vo.even * so * so;
                        s = so + t * (s - so);
                    }
                    *x = ceiling(s, c);
                }
            }
            if (t < 1.0f) fade_ = std::min(1.0f, fade_ + fade_step_);
        }
        os_.down(l, r, n);

        for (uint32_t i = 0; i < n; ++i) {
            const float t = static_cast<float>(i + 1) * inv_n;
            const dsp::SvfCoeffs post = dsp::SvfCoeffs::lerp(e0.post, e1.post, t);
            const dsp::SvfCoeffs bump = dsp::SvfCoeffs::lerp(e0.bump, e1.bump, t);
            const dsp::SvfCoeffs soft = dsp::SvfCoeffs::lerp(e0.soft, e1.soft, t);
            const dsp::SvfCoeffs bw = dsp::SvfCoeffs::lerp(e0.bw, e1.bw, t);
            const dsp::SvfCoeffs hp = dsp::SvfCoeffs::lerp(e0.hp, e1.hp, t);
            const dsp::SvfCoeffs hlp = dsp::SvfCoeffs::lerp(e0.hlp, e1.hlp, t);
            const float nz = noise_.next(noise);
            const float mx = mix_.next(mix);
            const float og = out_.next(out);
            float* io[2] = {l + i, r + i};
            const float* dry[2] = {dl + i, dr + i};
            for (int ch = 0; ch < 2; ++ch) {
                float y = dc_[ch].process(post_[ch].process(*io[ch], post));
                y = bw_[ch].process(soft_[ch].process(bump_[ch].process(y, bump), soft), bw);
                if (nz > 0.0f) y += nz * hlp_[ch].process(hp_[ch].process(hiss_[ch].next() * 1.7320508f, hp), hlp);
                *io[ch] = (*dry[ch] + mx * (y - *dry[ch])) * og;
            }
        }
        for (auto& b : dc_) b.flush_denormals();
        for (auto* f : {&pre_[0], &pre_[1], &post_[0], &post_[1], &bump_[0], &bump_[1], &soft_[0], &soft_[1], &bw_[0], &bw_[1],
                        &hp_[0], &hp_[1], &hlp_[0], &hlp_[1]})
            f->flush_denormals();
    }

    double sr_ = 48000.0;
    uint32_t block_ = 1;
    dsp::Oversampler4x os_;
    dsp::DelayLine dry_[2];
    std::vector<float> dry_buf_;
    dsp::Smoother drive_, warmth_, comp_, noise_, mix_, out_;
    dsp::DcBlocker dc_[2];
    dsp::Svf pre_[2], post_[2], bump_[2], soft_[2], bw_[2], hp_[2], hlp_[2];
    dsp::Noise hiss_[2];
    Eq eq_;
    float ms_ = 0.0f, gr_ = 1.0f, rms_a_ = 0.0f, att_a_ = 0.0f, rel_a_ = 0.0f;
    int mode_ = 0, prev_mode_ = 0;
    float fade_ = 1.0f, fade_step_ = 0.001f;
    bool primed_ = false;
};

std::unique_ptr<BuiltinProcessor> create_tape_saturation() { return std::make_unique<TapeSaturation>(); }

} // namespace

extern const BuiltinDescriptor kTapeSaturationDescriptor;
constexpr float kDefaults[] = {30.f, 40.f, 30.f, -60.f, 100.f, 0.f, 0.f};
const BuiltinDescriptor kTapeSaturationDescriptor{"tapesaturation", kParamIds, {}, &create_tape_saturation, {}, kDefaults};

} // namespace dstudio
