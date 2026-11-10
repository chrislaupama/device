// De-Hummer: a cascade of up to 8 depth-limited notches at the hum
// fundamental and its multiples. Parameters in the order of
// dstudio_core::builtin::dehummer (checked by a Rust test).
//
// Stage k (frequency k * f, Q = notch width): y = x - a_k * BP_k(x), with
// BP_k a TPT state-variable band-pass (0 dB at the centre), so the stage is
// (1 - a_k) + a_k * Notch_k and its deepest cut is exactly Depth
// (a = 1 - 10^(Depth / 20)). This is the response the panel draws. Each
// harmonic's a_k glides to 0 when Harmonics drops below k (no clicks), and
// stages above 0.45 * the sample rate are skipped. Mix blends with the dry.
// Readouts: the input spectrum bands.
#include "builtin.hpp"
#include "dsp.hpp"
#include "spectral.hpp"

#include <algorithm>
#include <cmath>

namespace dstudio {

namespace {

enum Param : uint32_t { kFrequency, kHarmonics, kWidth, kDepth, kMix, kParamCount };
constexpr const char* kParamIds[kParamCount] = {"frequency", "num_harmonics", "notch_width", "depth", "mix"};
constexpr float kDefaults[kParamCount] = {60.0f, 4.0f, 5.0f, -30.0f, 100.0f};
constexpr uint32_t kMaxHarmonics = 8;
constexpr uint32_t kCoeffStride = 16;

struct IdleReadouts {
    float v[dsp::kSpectrumBands];
    constexpr IdleReadouts() : v{} {
        for (float& x : v) x = -120.0f;
    }
};
constexpr IdleReadouts kIdle{};

class DeHummer final : public BuiltinProcessor {
public:
    void prepare(double sample_rate, uint32_t) override {
        sr_ = sample_rate;
        freq_.set_time(30.0f, sr_);
        q_.set_time(30.0f, sr_);
        mix_.set_time(20.0f, sr_);
        for (auto& a : amount_) a.set_time(30.0f, sr_);
        spectrum_.prepare(sr_);
    }

    void reset() noexcept override {
        for (auto& ch : svf_)
            for (auto& f : ch) f.reset();
        primed_ = false;
        countdown_ = 0;
        spectrum_.reset();
    }

    void process(float* l, float* r, uint32_t n, const float* p, const BuiltinContext&, float* ro) noexcept override {
        const float freq = std::clamp(p[kFrequency], 20.0f, 200.0f);
        const float hf = std::clamp(p[kHarmonics], 1.0f, static_cast<float>(kMaxHarmonics));
        const auto harmonics = static_cast<uint32_t>(std::lround(hf));
        const float q = std::clamp(p[kWidth], 0.1f, 50.0f);
        const float a = 1.0f - dsp::db_to_gain(std::clamp(p[kDepth], -120.0f, 0.0f));
        const float mix = std::clamp(p[kMix], 0.0f, 100.0f) * 0.01f;
        float target[kMaxHarmonics];
        for (uint32_t k = 0; k < kMaxHarmonics; ++k) {
            const bool audible = static_cast<double>(freq) * (k + 1) < 0.45 * sr_;
            target[k] = k < harmonics && audible ? a : 0.0f;
        }
        if (!primed_) {
            freq_.snap(freq);
            q_.snap(q);
            mix_.snap(mix);
            for (uint32_t k = 0; k < kMaxHarmonics; ++k) amount_[k].snap(target[k]);
            primed_ = true;
            countdown_ = 0;
        }
        spectrum_.push(l, r, n);

        // Stages that are off and settled cost nothing.
        uint32_t active = 0;
        for (uint32_t k = 0; k < kMaxHarmonics; ++k)
            if (target[k] > 0.0f || amount_[k].y > 1e-6f) active = k + 1;
        if (active > active_) countdown_ = 0; // new stages need coefficients now
        active_ = active;

        for (uint32_t i = 0; i < n; ++i) {
            const float f = freq_.next(freq);
            const float qq = q_.next(q);
            if (countdown_ == 0) {
                for (uint32_t k = 0; k < active; ++k) c_[k] = dsp::SvfCoeffs::bandpass(f * static_cast<float>(k + 1), qq, sr_);
                countdown_ = kCoeffStride;
            }
            --countdown_;
            const float xl = l[i], xr = r[i];
            float yl = xl, yr = xr;
            for (uint32_t k = 0; k < active; ++k) {
                const float ak = amount_[k].next(target[k]);
                yl -= ak * svf_[0][k].process(yl, c_[k]);
                yr -= ak * svf_[1][k].process(yr, c_[k]);
            }
            const float m = mix_.next(mix);
            l[i] = xl + m * (yl - xl);
            r[i] = xr + m * (yr - xr);
        }
        for (uint32_t k = active; k < kMaxHarmonics; ++k) {
            amount_[k].snap(0.0f);
            svf_[0][k].reset();
            svf_[1][k].reset();
        }
        for (auto& ch : svf_)
            for (auto& f : ch) f.flush_denormals();
        spectrum_.write(ro);
    }

private:
    double sr_ = 48000.0;
    uint32_t countdown_ = 0, active_ = 0;
    bool primed_ = false;
    dsp::Smoother freq_, q_, mix_, amount_[kMaxHarmonics];
    dsp::SvfCoeffs c_[kMaxHarmonics];
    dsp::Svf svf_[2][kMaxHarmonics];
    dsp::SpectrumTap spectrum_;
};

std::unique_ptr<BuiltinProcessor> create() { return std::make_unique<DeHummer>(); }

} // namespace

extern const BuiltinDescriptor kDeHummerDescriptor;
const BuiltinDescriptor kDeHummerDescriptor{"dehummer", kParamIds, dsp::kSpectrumIds, &create, kIdle.v, kDefaults};

} // namespace dstudio
