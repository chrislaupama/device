// De-Noiser: a linked-stereo spectral gate. Parameters in the order of
// dstudio_core::builtin::denoiser (checked by a Rust test).
//
// STFT: 1024-point frames at 48 kHz (the length follows the sample rate so
// the resolution in Hz stays put), hop N/4, square-root Hann analysis and
// synthesis windows (perfect reconstruction at unity gain). Per frame:
//   level     per-bin power, the louder of L / R, in dB where a full-scale
//             sine reads 0 dB (the scale of the panel's spectrum and
//             threshold axis)
//   Smoothing a forward-backward one-pole across bins averages each bin's
//             power with its neighbours before the decision (0 % = none):
//             noise bins decide together, which tames musical noise, and the
//             skirts of a tone open with it
//   decision  the smoothed level, lightly smoothed over time as well, below
//             Threshold gets Reduction dB of cut; the cut fades in over the
//             6 dB below the threshold (smoothstep), no hard edge
//   timing    per-bin attack (gain rising: the band opens) and release
//             (gain falling: the band closes)
//   mix       the mask is pulled toward unity: 1 - mix * (1 - g), so the dry
//             path is the same STFT and stays phase aligned
// Latency: N samples (1024 at 48 kHz), reported for delay compensation.
// Readouts: applied reduction (dB, <= 0, power weighted over the spectrum),
// then the input spectrum bands.
#include "builtin.hpp"
#include "dsp.hpp"
#include "spectral.hpp"

#include <algorithm>
#include <cmath>
#include <cstring>
#include <vector>

namespace dstudio {

namespace {

enum Param : uint32_t { kThreshold, kReduction, kSmoothing, kAttack, kRelease, kMix, kParamCount };
constexpr const char* kParamIds[kParamCount] = {"threshold", "reduction", "frequency_smoothing", "attack", "release", "mix"};
constexpr float kDefaults[kParamCount] = {-40.0f, -20.0f, 50.0f, 5.0f, 50.0f, 100.0f};

constexpr uint32_t kReadoutCount = 1 + dsp::kSpectrumBands;
struct ReadoutIds {
    const char* ids[kReadoutCount];
    constexpr ReadoutIds() : ids{} {
        ids[0] = "reduction_db";
        for (uint32_t i = 0; i < dsp::kSpectrumBands; ++i) ids[1 + i] = dsp::kSpectrumIds[i];
    }
};
constexpr ReadoutIds kReadouts{};
struct IdleReadouts {
    float v[kReadoutCount];
    constexpr IdleReadouts() : v{} {
        for (uint32_t i = 1; i < kReadoutCount; ++i) v[i] = -120.0f;
    }
};
constexpr IdleReadouts kIdle{};

constexpr float kKneeDb = 6.0f;
constexpr float kLevelSmooth = 0.5f; // per-frame weight of the newest power

class DeNoiser final : public BuiltinProcessor {
public:
    void prepare(double sample_rate, uint32_t) override {
        sr_ = sample_rate;
        const uint32_t scale = dsp::pow2_at_least(static_cast<uint32_t>(std::max(1.0, std::round(sample_rate / 48000.0))));
        n_ = 1024 * scale;
        hop_ = n_ / 4;
        lat_ = n_ - hop_;
        fft_.prepare(n_);
        bins_ = fft_.bins();
        win_.resize(n_);
        double wsum = 0.0;
        for (uint32_t i = 0; i < n_; ++i) {
            win_[i] = static_cast<float>(std::sqrt(0.5 - 0.5 * std::cos(2.0 * dsp::kPiD * i / n_)));
            wsum += static_cast<double>(win_[i]);
        }
        // Sum of analysis * synthesis windows at one point over all overlapping frames.
        double ola = 0.0;
        for (uint32_t k = 0; k < n_ / hop_; ++k) {
            const float w = win_[(k * hop_ + hop_ / 2) % n_];
            ola += static_cast<double>(w) * static_cast<double>(w);
        }
        ola_gain_ = static_cast<float>(1.0 / ola);
        const double ref = wsum / 2.0;
        level_norm_ = static_cast<float>(1.0 / (ref * ref));
        for (int c = 0; c < 2; ++c) {
            in_[c].assign(n_, 0.0f);
            out_[c].assign(hop_, 0.0f);
            acc_[c].assign(n_, 0.0f);
            re_[c].assign(bins_, 0.0f);
            im_[c].assign(bins_, 0.0f);
        }
        frame_.assign(n_, 0.0f);
        level_.assign(bins_, 0.0f);
        target_.assign(bins_, 0.0f);
        gain_db_.assign(bins_, 0.0f);
        spectrum_.prepare(sr_);
        reset();
    }

    void reset() noexcept override {
        for (int c = 0; c < 2; ++c) {
            std::fill(in_[c].begin(), in_[c].end(), 0.0f);
            std::fill(out_[c].begin(), out_[c].end(), 0.0f);
            std::fill(acc_[c].begin(), acc_[c].end(), 0.0f);
        }
        std::fill(level_.begin(), level_.end(), 0.0f);
        std::fill(gain_db_.begin(), gain_db_.end(), 0.0f);
        rover_ = lat_;
        reduction_db_ = 0.0f;
        mix_ = -1.0f;
        spectrum_.reset();
    }

    uint32_t latency() const noexcept override { return n_; }

    void process(float* l, float* r, uint32_t n, const float* p, const BuiltinContext&, float* ro) noexcept override {
        threshold_ = std::clamp(p[kThreshold], -140.0f, 24.0f);
        reduction_ = std::clamp(p[kReduction], -120.0f, 0.0f);
        smoothing_ = std::clamp(p[kSmoothing], 0.0f, 100.0f) * 0.01f;
        const float frame_ms = 1000.0f * static_cast<float>(hop_ / sr_);
        att_ = std::exp(-frame_ms / std::clamp(p[kAttack], 0.01f, 5000.0f));
        rel_ = std::exp(-frame_ms / std::clamp(p[kRelease], 0.1f, 50000.0f));
        mix_target_ = std::clamp(p[kMix], 0.0f, 100.0f) * 0.01f;
        if (mix_ < 0.0f) mix_ = mix_target_;
        spectrum_.push(l, r, n);

        float* in0 = in_[0].data();
        float* in1 = in_[1].data();
        uint32_t i = 0;
        while (i < n) {
            // Run up to the next frame boundary without a per-sample branch on it.
            const uint32_t run = std::min(n - i, n_ - rover_);
            const uint32_t o = rover_ - lat_;
            std::memcpy(in0 + rover_, l + i, 4 * run);
            std::memcpy(in1 + rover_, r + i, 4 * run);
            std::memcpy(l + i, out_[0].data() + o, 4 * run);
            std::memcpy(r + i, out_[1].data() + o, 4 * run);
            rover_ += run;
            i += run;
            if (rover_ == n_) {
                rover_ = lat_;
                frame();
            }
        }
        ro[0] = reduction_db_;
        spectrum_.write(ro + 1);
    }

private:
    void frame() noexcept {
        for (int c = 0; c < 2; ++c) {
            const float* x = in_[c].data();
            for (uint32_t k = 0; k < n_; ++k) frame_[k] = x[k] * win_[k];
            fft_.forward(frame_.data(), re_[c].data(), im_[c].data());
        }
        const float* r0 = re_[0].data();
        const float* i0 = im_[0].data();
        const float* r1 = re_[1].data();
        const float* i1 = im_[1].data();
        float* level = level_.data();
        float* target = target_.data();
        for (uint32_t k = 0; k < bins_; ++k)
            target[k] = std::max(r0[k] * r0[k] + i0[k] * i0[k], r1[k] * r1[k] + i1[k] * i1[k]) * level_norm_;
        if (smoothing_ > 0.0f) {
            const float a = 0.92f * smoothing_;
            for (uint32_t k = 1; k < bins_; ++k) target[k] += a * (target[k - 1] - target[k]);
            for (uint32_t k = bins_ - 1; k-- > 0;) target[k] += a * (target[k + 1] - target[k]);
        }
        for (uint32_t k = 0; k < bins_; ++k) {
            level[k] += kLevelSmooth * (target[k] - level[k]);
            const float ldb = 10.0f * std::log10(level[k] + 1e-20f);
            float s = std::clamp((threshold_ - ldb) * (1.0f / kKneeDb), 0.0f, 1.0f);
            s = s * s * (3.0f - 2.0f * s);
            target[k] = reduction_ * s;
        }
        mix_ += 0.5f * (mix_target_ - mix_);
        if (std::fabs(mix_ - mix_target_) < 1e-4f) mix_ = mix_target_;
        double wsum = 0.0, psum = 0.0;
        float* gdb = gain_db_.data();
        for (uint32_t k = 0; k < bins_; ++k) {
            const float t = target[k];
            gdb[k] = t + (t > gdb[k] ? att_ : rel_) * (gdb[k] - t);
            const float g = 1.0f - mix_ * (1.0f - dsp::db_to_gain(gdb[k]));
            const float pw = r0[k] * r0[k] + i0[k] * i0[k] + r1[k] * r1[k] + i1[k] * i1[k];
            psum += static_cast<double>(pw);
            wsum += static_cast<double>(pw * g * g);
            target[k] = g;
        }
        reduction_db_ = psum > 1e-12 ? std::min(0.0f, static_cast<float>(10.0 * std::log10(wsum / psum + 1e-30))) : 0.0f;
        for (int c = 0; c < 2; ++c) {
            float* re = re_[c].data();
            float* im = im_[c].data();
            for (uint32_t k = 0; k < bins_; ++k) {
                re[k] *= target[k];
                im[k] *= target[k];
            }
            fft_.inverse(re, im, frame_.data());
            float* acc = acc_[c].data();
            for (uint32_t k = 0; k < n_; ++k) acc[k] += frame_[k] * win_[k] * ola_gain_;
            std::memcpy(out_[c].data(), acc, 4 * hop_);
            std::memmove(acc, acc + hop_, 4 * (n_ - hop_));
            std::fill(acc + (n_ - hop_), acc + n_, 0.0f);
            std::memmove(in_[c].data(), in_[c].data() + hop_, 4 * lat_);
        }
        for (uint32_t k = 0; k < bins_; ++k)
            if (level[k] < 1e-30f) level[k] = 0.0f;
    }

    double sr_ = 48000.0;
    uint32_t n_ = 1024, hop_ = 256, lat_ = 768, bins_ = 513, rover_ = 768;
    float ola_gain_ = 0.5f, level_norm_ = 1.0f;
    float threshold_ = -40.0f, reduction_ = -20.0f, smoothing_ = 0.5f, att_ = 0.0f, rel_ = 0.0f;
    float mix_ = -1.0f, mix_target_ = 1.0f, reduction_db_ = 0.0f;
    dsp::RealFft fft_;
    std::vector<float> win_, frame_, level_, target_, gain_db_;
    std::vector<float> in_[2], out_[2], acc_[2], re_[2], im_[2];
    dsp::SpectrumTap spectrum_;
};

std::unique_ptr<BuiltinProcessor> create() { return std::make_unique<DeNoiser>(); }

} // namespace

extern const BuiltinDescriptor kDeNoiserDescriptor;
const BuiltinDescriptor kDeNoiserDescriptor{"denoiser", kParamIds, kReadouts.ids, &create, kIdle.v, kDefaults};

} // namespace dstudio
