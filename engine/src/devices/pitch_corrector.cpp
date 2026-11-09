// Pitch Corrector: automatic pitch correction of a monophonic voice or
// instrument to a key and scale. Parameters in the order of
// saine_core::builtin::pitch_corrector (checked by a Rust test). When Key
// Follow is Project, the application sends the project's key and scale as
// Key / Scale, so the kernel only reads those two.
//
// Detection: the shared PitchDetector (pitch_detect.hpp) on the mid signal,
// an estimate every 3 ms that describes the audio about lag() frames ago.
//
// Correction law, per estimate (m = detected MIDI pitch):
//   centre     slow note centre (120 ms glide; re-seeded on a jump > 0.75 st)
//   target     nearest scale note to the centre (0.12 st hysteresis)
//   desired    Speed * (target - (H * centre + (1 - H) * m)), H = Humanize:
//              at Humanize 0 every deviation is flattened (hard tune), at
//              100 % only the note centre moves and vibrato / drift stay
//   glide      the applied shift follows desired with a one-pole of
//              Retune Speed ms, lengthened by up to 3x on sustained notes as
//              Humanize rises; unvoiced input glides back to no shift
//
// Shifter: a pitch-synchronous two-head delay line. Each head's delay sweeps
// at (1 - ratio) samples per sample through a span D = two detected periods
// around the fixed latency C, and the heads (half a span apart, so exactly
// one period) cross-fade with sin^2 / cos^2 gains; a head jumps by D (two
// whole periods) only at zero gain, so periodic input splices seamlessly.
// With no shift the heads settle on the centre: the output is the input
// delayed by exactly C (bit transparent apart from the delay). Reads use
// 4-point Hermite interpolation. C = the detector lag, so the shift lines up
// with the audio it was measured on; reported as latency, and the dry path
// of Mix is delayed by the same amount.
#include "builtin.hpp"
#include "dsp.hpp"
#include "pitch_detect.hpp"

#include <algorithm>
#include <cmath>
#include <vector>

namespace saine {

namespace {

enum Param : uint32_t { kSpeed, kKey, kScale, kRetune, kHumanize, kMix, kKeyFollow, kParamCount };
enum Readout : uint32_t { kVoiced, kPitch, kShift, kTarget, kReadoutCount };

constexpr const char* kParamIds[kParamCount] = {"speed", "key", "scale", "retune_speed_ms", "humanize", "mix", "key_follow"};
constexpr const char* kReadoutIds[kReadoutCount] = {"voiced", "pitch", "shift", "target"};

// Scale masks over pitch classes relative to the key (bit i = semitone i).
constexpr uint32_t kScaleMask[3] = {0xFFF, 0xAB5 /* major 0 2 4 5 7 9 11 */, 0x5AD /* natural minor 0 2 3 5 7 8 10 */};

constexpr uint32_t kChunk = 32;
constexpr float kPi = 3.14159265f;

inline float hermite(const float* buf, uint32_t mask, uint32_t w, float delay) noexcept {
    const float pos = static_cast<float>(w) - delay;
    const float fl = std::floor(pos);
    const float t = pos - fl;
    const auto i = static_cast<uint32_t>(static_cast<int64_t>(fl));
    const float y0 = buf[(i - 1) & mask], y1 = buf[i & mask], y2 = buf[(i + 1) & mask], y3 = buf[(i + 2) & mask];
    const float c1 = 0.5f * (y2 - y0);
    const float c2 = y0 - 2.5f * y1 + 2.0f * y2 - 0.5f * y3;
    const float c3 = 0.5f * (y3 - y0) + 1.5f * (y1 - y2);
    return ((c3 * t + c2) * t + c1) * t + y1;
}

class PitchCorrector final : public BuiltinProcessor {
public:
    void prepare(double sample_rate, uint32_t) override {
        sr_ = sample_rate > 0.0 ? sample_rate : 48000.0;
        det_.prepare(sr_, 55.0f, 1400.0f, 3.0f);
        c_ = static_cast<float>(det_.lag());
        uint32_t size = 64;
        while (size < 2 * det_.lag() + 16) size <<= 1;
        mask_ = size - 1;
        bl_.assign(size, 0.0f);
        br_.assign(size, 0.0f);
        d_max_ = 2.0f * c_ - 4.0f;
        d_min_ = static_cast<float>(sr_ / 600.0);
        hop_s_ = static_cast<float>(det_.hop() / sr_);
        centre_a_ = std::exp(-hop_s_ / 0.12f);
        d_a_ = dsp::pole(8.0f, sr_);
        mix_.set_time(20.0f, sr_);
    }

    void reset() noexcept override {
        det_.reset();
        std::fill(bl_.begin(), bl_.end(), 0.0f);
        std::fill(br_.begin(), br_.end(), 0.0f);
        w_ = 0;
        phase_ = 0.5f;
        span_ = span_target_ = std::min(d_max_, static_cast<float>(sr_ / 50.0));
        shift_ = desired_ = 0.0f;
        tracking_ = false;
        held_ = 0.0f;
        centre_ = pitch_ = 0.0f;
        target_ = -1;
        glide_ms_ = 30.0f;
        primed_ = false;
    }

    uint32_t latency() const noexcept override { return static_cast<uint32_t>(c_); }

    void process(float* l, float* r, uint32_t n, const float* p, const BuiltinContext&, float* ro) noexcept override {
        const float speed = std::clamp(p[kSpeed], 0.0f, 100.0f) * 0.01f;
        const int key = std::clamp(static_cast<int>(std::lround(p[kKey])), 0, 11);
        const uint32_t mask = kScaleMask[std::clamp(static_cast<int>(std::lround(p[kScale])), 0, 2)];
        const float retune = std::clamp(p[kRetune], 0.0f, 2000.0f);
        const float human = std::clamp(p[kHumanize], 0.0f, 100.0f) * 0.01f;
        const float mix = std::clamp(p[kMix], 0.0f, 100.0f) * 0.01f;
        if (!primed_) {
            mix_.snap(mix);
            primed_ = true;
        }
        float mono[kChunk];
        for (uint32_t off = 0; off < n; off += kChunk) {
            const uint32_t m = std::min(kChunk, n - off);
            float* const xl = l + off;
            float* const xr = r + off;
            for (uint32_t i = 0; i < m; ++i) mono[i] = 0.5f * (xl[i] + xr[i]);
            if (det_.push(mono, m) > 0) update(speed, key, mask, retune, human);
            const float a = dsp::pole(glide_ms_, sr_);
            for (uint32_t i = 0; i < m; ++i) {
                bl_[w_] = xl[i];
                br_[w_] = xr[i];
                shift_ = desired_ + a * (shift_ - desired_);
                span_ = span_target_ + d_a_ * (span_ - span_target_);
                const float ratio = std::exp2(shift_ * (1.0f / 12.0f));
                float dphi = (ratio - 1.0f) / span_;
                if (std::fabs(ratio - 1.0f) < 1e-4f) { // settle the heads on the centre (<= 3 cents of drift)
                    const float rate = 0.0017f / span_;
                    const float to = phase_ >= 0.25f && phase_ < 0.75f ? 0.5f : (phase_ < 0.5f ? 0.0f : 1.0f);
                    dphi += std::clamp(to - phase_, -rate, rate);
                }
                phase_ += dphi;
                if (phase_ >= 1.0f) phase_ -= 1.0f;
                if (phase_ < 0.0f) phase_ += 1.0f;
                const float pb = phase_ >= 0.5f ? phase_ - 0.5f : phase_ + 0.5f;
                const float da = c_ - span_ * (phase_ - 0.5f);
                const float db = c_ - span_ * (pb - 0.5f);
                const float ga = 0.5f - 0.5f * std::cos(2.0f * kPi * phase_);
                const float gb = 1.0f - ga;
                const float wl = ga * hermite(bl_.data(), mask_, w_, da) + gb * hermite(bl_.data(), mask_, w_, db);
                const float wr = ga * hermite(br_.data(), mask_, w_, da) + gb * hermite(br_.data(), mask_, w_, db);
                const uint32_t dry = (w_ - static_cast<uint32_t>(c_)) & mask_;
                const float k = mix_.next(mix);
                xl[i] = bl_[dry] + k * (wl - bl_[dry]);
                xr[i] = br_[dry] + k * (wr - br_[dry]);
                w_ = (w_ + 1) & mask_;
            }
        }
        if (std::fabs(shift_) < 1e-7f) shift_ = 0.0f;
        const bool voiced = tracking_;
        ro[kVoiced] = voiced ? 1.0f : 0.0f;
        ro[kPitch] = voiced ? pitch_ : 0.0f;
        ro[kShift] = shift_;
        ro[kTarget] = voiced && target_ >= 0 ? static_cast<float>(target_) : 0.0f;
    }

private:
    // Nearest note to x (MIDI) whose pitch class is in the scale.
    static int nearest(float x, int key, uint32_t mask) noexcept {
        const int base = static_cast<int>(std::lround(x));
        int best = base;
        float dist = 1e9f;
        for (int s = base - 6; s <= base + 6; ++s) {
            if (((mask >> (((s - key) % 12 + 12) % 12)) & 1u) == 0) continue;
            const float d = std::fabs(x - static_cast<float>(s));
            if (d < dist) {
                dist = d;
                best = s;
            }
        }
        return best;
    }

    void update(float speed, int key, uint32_t mask, float retune, float human) noexcept {
        const dsp::PitchEstimate& e = det_.estimate();
        if (!e.voiced) {
            tracking_ = false;
            held_ = 0.0f;
            desired_ = 0.0f;
            glide_ms_ = std::max(retune, 30.0f);
            return;
        }
        const float m = 69.0f + 12.0f * std::log2(e.hz / 440.0f);
        span_target_ = std::clamp(2.0f * static_cast<float>(sr_) / e.hz, d_min_, d_max_);
        if (!tracking_ || std::fabs(m - centre_) > 0.75f) {
            centre_ = m;
            held_ = 0.0f;
            target_ = nearest(m, key, mask);
            tracking_ = true;
        } else {
            centre_ = m + centre_a_ * (centre_ - m);
            held_ += hop_s_;
        }
        pitch_ = m;
        const int cand = nearest(centre_, key, mask);
        if (target_ < 0 || ((mask >> (((target_ - key) % 12 + 12) % 12)) & 1u) == 0 ||
            std::fabs(centre_ - static_cast<float>(target_)) - std::fabs(centre_ - static_cast<float>(cand)) > 0.12f)
            target_ = cand;
        const float sung = human * centre_ + (1.0f - human) * m;
        desired_ = std::clamp(speed * (static_cast<float>(target_) - sung), -2.0f, 2.0f);
        glide_ms_ = retune * (1.0f + 2.0f * human * std::min(1.0f, held_ / 0.25f));
    }

    double sr_ = 48000.0;
    dsp::PitchDetector det_;
    std::vector<float> bl_, br_;
    uint32_t mask_ = 0, w_ = 0;
    float c_ = 1024.0f, d_min_ = 80.0f, d_max_ = 2044.0f;
    float phase_ = 0.5f, span_ = 960.0f, span_target_ = 960.0f, d_a_ = 0.0f;
    float shift_ = 0.0f, desired_ = 0.0f, glide_ms_ = 30.0f;
    bool tracking_ = false, primed_ = false;
    float held_ = 0.0f, centre_ = 0.0f, pitch_ = 0.0f, hop_s_ = 0.0025f, centre_a_ = 0.98f;
    int target_ = -1;
    dsp::Smoother mix_;
};

std::unique_ptr<BuiltinProcessor> create_pitch_corrector() { return std::make_unique<PitchCorrector>(); }

} // namespace

extern const BuiltinDescriptor kPitchCorrectorDescriptor;
const BuiltinDescriptor kPitchCorrectorDescriptor{"pitch-corrector", kParamIds, kReadoutIds, &create_pitch_corrector, {}, true};

} // namespace saine
