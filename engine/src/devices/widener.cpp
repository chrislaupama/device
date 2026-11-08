// Stereo Widener: mid/side width with a low cut that keeps the bass centred.
// Parameters in the order of saine_core::builtin::widener (checked by a Rust
// test).
//
//   M = (L + R) / 2, S = (L - R) / 2
//   M' = M * (1 - x + x * mid)
//   S' = S * (1 - x) + x * width * side * HP(S)     HP: one pole at Low Cut
//   L = M' + S', R = M' - S'                         x = Mix
// The same response the panel draws (saine_core widener_response). No latency.
#include "builtin.hpp"
#include "dsp.hpp"

#include <algorithm>
#include <cmath>

namespace saine {

namespace {

enum Param : uint32_t { kWidth, kMid, kSide, kLowCut, kMix, kParamCount };

constexpr const char* kParamIds[kParamCount] = {"width2", "midgain", "sidegain", "lowcut", "mix"};

class Widener final : public BuiltinProcessor {
public:
    void prepare(double sample_rate, uint32_t) override {
        sr_ = sample_rate;
        for (auto* s : {&width_, &mid_, &side_, &pole_, &mix_}) s->set_time(20.0f, sr_);
    }

    void reset() noexcept override {
        x1_ = y1_ = 0.0f;
        primed_ = false;
    }

    void process(float* l, float* r, uint32_t n, const float* p, const BuiltinContext&, float*) noexcept override {
        const float width = std::clamp(p[kWidth], 0.0f, 400.0f) * 0.01f;
        const float mid = dsp::db_to_gain(std::clamp(p[kMid], -24.0f, 24.0f));
        const float side = dsp::db_to_gain(std::clamp(p[kSide], -24.0f, 24.0f));
        const float fc = std::clamp(p[kLowCut], 10.0f, 2000.0f);
        const float pole = static_cast<float>(std::exp(-6.283185307179586 * static_cast<double>(fc) / sr_));
        const float mix = std::clamp(p[kMix], 0.0f, 100.0f) * 0.01f;
        if (!primed_) {
            width_.snap(width);
            mid_.snap(mid);
            side_.snap(side);
            pole_.snap(pole);
            mix_.snap(mix);
            primed_ = true;
        }
        float x1 = x1_, y1 = y1_;
        for (uint32_t i = 0; i < n; ++i) {
            const float w = width_.next(width), mg = mid_.next(mid), sg = side_.next(side), a = pole_.next(pole), x = mix_.next(mix);
            const float m = 0.5f * (l[i] + r[i]);
            const float s = 0.5f * (l[i] - r[i]);
            const float hp = a * (y1 + s - x1);
            x1 = s;
            y1 = hp;
            const float mo = m * (1.0f - x + x * mg);
            const float so = s * (1.0f - x) + x * w * sg * hp;
            l[i] = mo + so;
            r[i] = mo - so;
        }
        x1_ = x1;
        y1_ = std::fabs(y1) < 1e-20f ? 0.0f : y1;
    }

private:
    double sr_ = 48000.0;
    dsp::Smoother width_, mid_, side_, pole_, mix_;
    float x1_ = 0.0f, y1_ = 0.0f;
    bool primed_ = false;
};

std::unique_ptr<BuiltinProcessor> create_widener() { return std::make_unique<Widener>(); }

} // namespace

extern const BuiltinDescriptor kWidenerDescriptor;
const BuiltinDescriptor kWidenerDescriptor{"widener", kParamIds, {}, &create_widener, {}};

} // namespace saine
