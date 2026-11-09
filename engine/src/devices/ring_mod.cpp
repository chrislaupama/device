// Ring Mod: multiplies the signal by a carrier (Sine, Triangle, Square, Saw,
// smoothed Noise, stepped Random) with rectify, bias (towards amplitude
// modulation), left/right detune (Spread), dry/wet mix and output gain.
// Parameters in the order of saine_core::builtin::ring_mod (checked by a
// Rust test).
//
// Carrier per channel: frequency f * 2^(-+spread / 12) (spread 0..1 = up to
// a semitone each way), wave w(phase) in -1..1, then
//   rectify r >= 0: (1 - r) w + r |w|;  r < 0: (1 + r) w - |r| |w|
//   bias b:         (w + b) / (1 + b)   (b = 1 keeps the dry note: AM)
//   wet             x * carrier
//   out             ((1 - mix) x + mix wet) * output
// Audio-rate products create sum frequencies above the input band, and the
// Square / Saw / Random waves have hard edges, so the product runs at 2x
// the sample rate (linear-phase half-band FIR up and down, 23 samples of
// reported latency; the dry path is delayed to match) and the waves are
// band-limited: polyBLEP at steps, polyBLAMP at the triangle's corners.
// Noise is random values at 16 per carrier cycle, low-passed twice,
// normalised, soft-clipped and Catmull-Rom interpolated.
#include "builtin.hpp"
#include "dsp.hpp"

#include <algorithm>
#include <cmath>

namespace saine {

namespace {

enum Param : uint32_t { kFrequency, kBias, kRectify, kSpread, kModulator, kMix, kOutput, kParamCount };
enum Readout : uint32_t { kCarrierPhase, kReadoutCount };
enum Wave : int { kSine, kTriangle, kSquare, kSaw, kNoise, kRandom };

constexpr const char* kParamIds[kParamCount] = {"frequency", "bias", "rectify", "spread", "modulator", "mix", "output"};
constexpr const char* kReadoutIds[kReadoutCount] = {"carrier_phase"};
constexpr float kDefaults[kParamCount] = {440.0f, 0.0f, 0.0f, 0.0f, 0.0f, 100.0f, 0.0f};
constexpr uint32_t kLatency = dsp::Oversampler2x::kLatency;
constexpr int kNoiseSteps = 16;

// Band-limiting residuals around a discontinuity at tau = 0 (tau in samples).
// Unit step (0 -> 1):
inline float blep(float tau) noexcept {
    if (tau >= 1.0f || tau <= -1.0f) return 0.0f;
    if (tau < 0.0f) return 0.5f * (tau + 1.0f) * (tau + 1.0f);
    return -0.5f * (1.0f - tau) * (1.0f - tau);
}
// Unit slope change (ramp max(0, tau)):
inline float blamp(float tau) noexcept {
    if (tau >= 1.0f || tau <= -1.0f) return 0.0f;
    const float u = 1.0f - std::fabs(tau);
    return u * u * u * (1.0f / 6.0f);
}
// Distance in samples from phase \`ph\` to an event at phase \`at\` (wrapped).
inline float tau_of(float ph, float at, float dt) noexcept {
    float d = ph - at;
    d -= std::floor(d + 0.5f);
    return d / dt;
}

struct Carrier {
    double phase = 0.0;
    dsp::Rng rng;
    // Random: the value before the last cycle start, the current and the
    // next held values. Noise: last four filtered steps.
    float prev = 0.0f, held = 0.0f, next = 0.0f;
    float pts[4]{};
    float lp1 = 0.0f, lp2 = 0.0f, step_pos = 0.0f;

    void seed(uint32_t s) noexcept {
        rng.seed(s);
        phase = 0.0;
        prev = held = rng.next();
        next = rng.next();
        lp1 = lp2 = step_pos = 0.0f;
        for (float& p : pts) p = 0.0f;
    }
};

class RingMod final : public BuiltinProcessor {
public:
    void prepare(double sample_rate, uint32_t) override {
        sr2_ = 2.0 * sample_rate;
        for (auto* s : {&bias_, &rect_, &mix_, &out_, &spread_}) s->set_time(20.0f, sample_rate);
        logf_.set_time(20.0f, sample_rate);
        for (auto& d : dry_) d.prepare(kLatency + 4);
        // Noise normalisation: RMS of uniform noise through the two one-poles.
        noise_k_ = 1.0f - std::exp(-dsp::kTwoPi / static_cast<float>(kNoiseSteps));
        dsp::Rng rng;
        rng.seed(7);
        double acc = 0.0;
        float a = 0.0f, b = 0.0f;
        for (int i = 0; i < 20000; ++i) {
            a += noise_k_ * (rng.next() - a);
            b += noise_k_ * (a - b);
            if (i >= 2000) acc += static_cast<double>(b) * static_cast<double>(b);
        }
        noise_scale_ = 1.33f / static_cast<float>(std::sqrt(acc / 18000.0));
    }

    void reset() noexcept override {
        for (auto& o : os_) o.reset();
        for (auto& d : dry_) d.reset();
        car_[0].seed(1380535879u);
        car_[1].seed(1297040417u);
        primed_ = false;
    }

    uint32_t latency() const noexcept override { return kLatency; }

    void process(float* l, float* r, uint32_t n, const float* p, const BuiltinContext&, float* ro) noexcept override {
        const float logf = std::log2(std::clamp(p[kFrequency], 0.1f, 22000.0f));
        const float bias = std::clamp(p[kBias], 0.0f, 100.0f) * 0.01f;
        const float rect = std::clamp(p[kRectify], -100.0f, 100.0f) * 0.01f;
        const float spread = std::clamp(p[kSpread], 0.0f, 100.0f) * (0.01f / 12.0f);
        const float mix = std::clamp(p[kMix], 0.0f, 100.0f) * 0.01f;
        const float out = dsp::db_to_gain(std::clamp(p[kOutput], -48.0f, 48.0f));
        const int wave = std::clamp(static_cast<int>(std::lround(p[kModulator])), 0, 5);
        if (!primed_) {
            logf_.snap(logf);
            bias_.snap(bias);
            rect_.snap(rect);
            spread_.snap(spread);
            mix_.snap(mix);
            out_.snap(out);
            primed_ = true;
        }
        float* io[2] = {l, r};
        const float nyq = 0.45f;
        for (uint32_t i = 0; i < n; ++i) {
            const float lf = logf_.next(logf), b = bias_.next(bias), rc = rect_.next(rect), sp = spread_.next(spread);
            const float m = mix_.next(mix), g = out_.next(out);
            for (int c = 0; c < 2; ++c) {
                const float x = io[c][i];
                const float hz = std::exp2(lf + (c == 0 ? -sp : sp));
                const float dt = std::min(static_cast<float>(static_cast<double>(hz) / sr2_), nyq);
                float u0, u1;
                os_[c].up(x, u0, u1);
                const float y0 = u0 * shape(car_[c], wave, dt, rc, b);
                const float y1 = u1 * shape(car_[c], wave, dt, rc, b);
                const float wet = os_[c].down(y0, y1);
                dry_[c].push(x);
                const float xd = dry_[c].at(kLatency);
                io[c][i] = ((1.0f - m) * xd + m * wet) * g;
            }
        }
        ro[kCarrierPhase] = static_cast<float>(car_[0].phase);
    }

private:
    // One 2x-rate carrier sample (advances the phase).
    float shape(Carrier& k, int wave, float dt, float rect, float bias) noexcept {
        const float ph = static_cast<float>(k.phase);
        float w;
        switch (wave) {
        case kTriangle:
            w = dsp::tri(ph) - 8.0f * dt * blamp(tau_of(ph, 0.25f, dt)) + 8.0f * dt * blamp(tau_of(ph, 0.75f, dt));
            break;
        case kSquare: w = (ph < 0.5f ? 1.0f : -1.0f) + 2.0f * blep(tau_of(ph, 0.0f, dt)) - 2.0f * blep(tau_of(ph, 0.5f, dt)); break;
        case kSaw: w = 2.0f * ph - 1.0f - 2.0f * blep(tau_of(ph, 0.0f, dt)); break;
        case kRandom: {
            // Steps at cycle starts: approaching one the jump is held -> next,
            // just after one it was prev -> held.
            const float t = tau_of(ph, 0.0f, dt);
            w = k.held + (t < 0.0f ? k.next - k.held : k.held - k.prev) * blep(t);
            break;
        }
        case kNoise: {
            const float f = k.step_pos;
            const float p0 = k.pts[0], p1 = k.pts[1], p2 = k.pts[2], p3 = k.pts[3];
            w = p1 + 0.5f * f * (p2 - p0 + f * (2.0f * p0 - 5.0f * p1 + 4.0f * p2 - p3 + f * (3.0f * (p1 - p2) + p3 - p0)));
            k.step_pos += dt * static_cast<float>(kNoiseSteps);
            for (int guard = 0; k.step_pos >= 1.0f && guard < 8; ++guard) {
                k.step_pos -= 1.0f;
                k.lp1 += noise_k_ * (k.rng.next() - k.lp1);
                k.lp2 += noise_k_ * (k.lp1 - k.lp2);
                k.pts[0] = k.pts[1];
                k.pts[1] = k.pts[2];
                k.pts[2] = k.pts[3];
                k.pts[3] = std::tanh(k.lp2 * noise_scale_);
            }
            k.step_pos = std::min(k.step_pos, 0.999f);
            break;
        }
        default: w = dsp::sin2pi(ph); break;
        }
        k.phase += static_cast<double>(dt);
        if (k.phase >= 1.0) {
            k.phase -= 1.0;
            if (wave == kRandom) {
                k.prev = k.held;
                k.held = k.next;
                k.next = k.rng.next();
            }
        }
        const float a = std::fabs(w);
        w = rect >= 0.0f ? w + rect * (a - w) : w + rect * (a + w); // r < 0: (1 + r) w + r |w|
        return (w + bias) / (1.0f + bias);
    }

    double sr2_ = 96000.0;
    dsp::Oversampler2x os_[2];
    dsp::DelayLine dry_[2];
    Carrier car_[2];
    dsp::Smoother logf_, bias_, rect_, spread_, mix_, out_;
    float noise_k_ = 0.3f, noise_scale_ = 1.0f;
    bool primed_ = false;
};

std::unique_ptr<BuiltinProcessor> create() { return std::make_unique<RingMod>(); }

} // namespace

extern const BuiltinDescriptor kRingModDescriptor;
const BuiltinDescriptor kRingModDescriptor{"ring-mod", kParamIds, kReadoutIds, &create, {}, kDefaults};

} // namespace saine
