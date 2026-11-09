// De-Esser: split-band dynamic cut. Parameters in the order of
// saine_core::builtin::deesser (checked by a Rust test).
//
//   band      b = k * v1 of a TPT state-variable band-pass at Frequency with
//             Q = Bandwidth (0 dB at the centre), per channel
//   detector  stereo-linked peak of the band, 0.3 ms attack / 60 ms release
//   gain      soft knee (6 dB) 3:1 above Threshold, softly capped at Reduction
//   output    x - mix * (1 - g) * b: only the band is turned down, and with
//             g = 1 the sum is exactly the input (notch + band = input)
//   Listen    plays the band alone (crossfaded over 10 ms)
//
// This is the curve the panel draws: H = 1 - mix * (1 - g) * BP.
// Readouts: live reduction (dB, <= 0), then the input spectrum bands.
#include "builtin.hpp"
#include "dsp.hpp"
#include "spectral.hpp"
#include "svf.hpp"

#include <algorithm>
#include <cmath>

namespace saine {

namespace {

enum Param : uint32_t { kFrequency, kBandwidth, kThreshold, kReduction, kListen, kMix, kParamCount };
constexpr const char* kParamIds[kParamCount] = {"frequency", "bandwidth", "threshold", "reduction", "listen", "mix"};
constexpr float kDefaults[kParamCount] = {7000.0f, 1.0f, -36.0f, 8.0f, 0.0f, 100.0f};

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

constexpr uint32_t kCoeffStride = 16; // samples between filter coefficient updates
constexpr float kKneeDb = 6.0f;
constexpr float kSlope = 1.0f - 1.0f / 3.0f;

// Gain reduction (dB, >= 0) for a band level over the threshold, softly capped at cap.
inline float reduction_db(float over, float cap) noexcept {
    if (cap <= 0.0f || over <= -0.5f * kKneeDb) return 0.0f;
    float gr;
    if (over < 0.5f * kKneeDb) {
        const float k = over + 0.5f * kKneeDb;
        gr = kSlope * k * k / (2.0f * kKneeDb);
    } else {
        gr = kSlope * over;
    }
    // Smooth minimum of gr and cap (3 dB corner).
    constexpr float kCorner = 3.0f;
    return std::max(0.0f, 0.5f * (gr + cap - std::sqrt((gr - cap) * (gr - cap) + kCorner * kCorner)));
}

class DeEsser final : public BuiltinProcessor {
public:
    void prepare(double sample_rate, uint32_t) override {
        sr_ = sample_rate;
        att_ = dsp::pole(0.3f, sr_);
        rel_ = dsp::pole(60.0f, sr_);
        freq_.set_time(20.0f, sr_);
        q_.set_time(20.0f, sr_);
        mix_.set_time(20.0f, sr_);
        thr_.set_time(20.0f, sr_);
        cap_.set_time(20.0f, sr_);
        listen_.set_time(10.0f, sr_);
        spectrum_.prepare(sr_);
    }

    void reset() noexcept override {
        for (auto& f : svf_) f.reset();
        env_ = 0.0f;
        primed_ = false;
        countdown_ = 0;
        spectrum_.reset();
    }

    void process(float* l, float* r, uint32_t n, const float* p, const BuiltinContext&, float* ro) noexcept override {
        const float freq = std::clamp(p[kFrequency], 20.0f, 20000.0f);
        const float q = std::clamp(p[kBandwidth], 0.05f, 10.0f);
        const float threshold = std::clamp(p[kThreshold], -80.0f, 24.0f);
        const float cap = std::clamp(p[kReduction], 0.0f, 60.0f);
        const float listen = p[kListen] >= 0.5f ? 1.0f : 0.0f;
        const float mix = std::clamp(p[kMix], 0.0f, 100.0f) * 0.01f;
        const float lf = std::log2(freq);
        if (!primed_) {
            freq_.snap(lf);
            q_.snap(q);
            mix_.snap(mix);
            thr_.snap(threshold);
            cap_.snap(cap);
            listen_.snap(listen);
            primed_ = true;
        }
        spectrum_.push(l, r, n);

        float env = env_, gr_max = 0.0f;
        for (uint32_t i = 0; i < n; ++i) {
            const float f = freq_.next(lf);
            const float qq = q_.next(q);
            if (countdown_ == 0) {
                c_ = dsp::SvfCoeffs::band(std::exp2(f), qq, sr_);
                countdown_ = kCoeffStride;
            }
            --countdown_;
            const float xl = l[i], xr = r[i];
            const float bl = svf_[0].band(xl, c_);
            const float br = svf_[1].band(xr, c_);
            const float d = std::max(std::fabs(bl), std::fabs(br));
            env = d + (d > env ? att_ : rel_) * (env - d);
            const float gr = reduction_db(dsp::gain_to_db(env) - thr_.next(threshold), cap_.next(cap));
            gr_max = std::max(gr_max, gr);
            const float cut = mix_.next(mix) * (1.0f - dsp::db_to_gain(-gr));
            const float ls = listen_.next(listen);
            const float yl = xl - cut * bl;
            const float yr = xr - cut * br;
            l[i] = yl + ls * (bl - yl);
            r[i] = yr + ls * (br - yr);
        }
        if (env < 1e-20f) env = 0.0f;
        env_ = env;
        for (auto& f : svf_) f.flush();
        ro[0] = -gr_max;
        spectrum_.write(ro + 1);
    }

private:
    double sr_ = 48000.0;
    float att_ = 0.0f, rel_ = 0.0f, env_ = 0.0f;
    uint32_t countdown_ = 0;
    bool primed_ = false;
    dsp::Smoother freq_, q_, mix_, thr_, cap_, listen_;
    dsp::SvfCoeffs c_;
    dsp::Svf svf_[2];
    dsp::SpectrumTap spectrum_;
};

std::unique_ptr<BuiltinProcessor> create() { return std::make_unique<DeEsser>(); }

} // namespace

extern const BuiltinDescriptor kDeEsserDescriptor;
const BuiltinDescriptor kDeEsserDescriptor{"deesser", kParamIds, kReadouts.ids, &create, kIdle.v, kDefaults};

} // namespace saine
