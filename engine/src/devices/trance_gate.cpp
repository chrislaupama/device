// Trance Gate: a tempo-synced step gate. Parameters in the order of
// saine_core::builtin::trance_gate (checked by a Rust test).
//
// Timing: a step lasts the Rate's note value; step k of the timeline starts
// at k steps, every odd step delayed by swing x half a step. The pattern
// index of a step is k modulo the active pattern's length, so the gate
// follows the song position while the transport plays (and free-runs from
// where it stopped while it doesn't, so live input can still be gated).
//
// Envelope: every step that is on retriggers an attack from the current
// level (linear rise to full in Attack), then a linear fall to Sustain in
// Decay (rate (1 - S) / Decay), then holds; a step that is off releases
// linearly to silence in Release. Changes of pattern, length or rate take
// effect at the next step boundary.
//
// Output: (dry + mix * (dry * env - dry)); Mix is smoothed.
#include "builtin.hpp"
#include "dsp.hpp"

#include <algorithm>
#include <cmath>
#include <cstdint>
#include <vector>

namespace saine {

namespace {

enum Param : uint32_t { kPattern, kRate, kAttack, kDecay, kSustain, kRelease, kSwing, kMix, kPatternBase, kParamCount = 32 };

constexpr const char* kParamIds[kParamCount] = {
    "pattern",  "rate",  "attack",   "decay",    "sustain",  "release", "swing",    "mix",      "p1_steps", "p1_lo", "p1_hi",
    "p2_steps", "p2_lo", "p2_hi",    "p3_steps", "p3_lo",    "p3_hi",   "p4_steps", "p4_lo",    "p4_hi",    "p5_steps", "p5_lo",
    "p5_hi",    "p6_steps", "p6_lo", "p6_hi",    "p7_steps", "p7_lo",   "p7_hi",    "p8_steps", "p8_lo",    "p8_hi"};

// Quarter notes per step, in the order of the Rate choices (slowest first).
constexpr double kStepQuarters[] = {1.5, 1.0, 2.0 / 3.0, 0.75, 0.5, 1.0 / 3.0, 0.375, 0.25, 1.0 / 6.0, 0.125, 0.0625};
constexpr int kRates = static_cast<int>(sizeof kStepQuarters / sizeof kStepQuarters[0]);

enum Stage : int { kIdle, kAttackStage, kDecayStage, kSustainStage, kReleaseStage };

class TranceGate final : public BuiltinProcessor {
public:
    void prepare(double sample_rate, uint32_t max_block) override {
        sr_ = sample_rate;
        env_.assign(std::max<uint32_t>(1, max_block), 0.0f);
        mix_.set_time(20.0f, sr_);
    }

    void reset() noexcept override {
        level_ = 0.0f;
        stage_ = kIdle;
        release_rate_ = 0.0f;
        last_step_ = INT64_MIN;
        free_beats_ = 0.0;
        primed_ = false;
    }

    void process(float* l, float* r, uint32_t n, const float* p, const BuiltinContext& ctx, float*) noexcept override {
        const int pattern = std::clamp(static_cast<int>(std::lround(p[kPattern])), 0, 7);
        const int rate = std::clamp(static_cast<int>(std::lround(p[kRate])), 0, kRates - 1);
        const float ms_per_sample = static_cast<float>(1000.0 / sr_);
        const float attack_step = ms_per_sample / std::clamp(p[kAttack], 0.5f, 250.0f);
        const float sustain = std::clamp(p[kSustain], 0.0f, 100.0f) * 0.01f;
        const float decay_step = (1.0f - sustain) * ms_per_sample / std::clamp(p[kDecay], 1.0f, 1000.0f);
        const float release_ms = std::max(0.5f, std::clamp(p[kRelease], 1.0f, 500.0f));
        const double swing = std::clamp(static_cast<double>(p[kSwing]), 0.0, 100.0) * 0.005; // in steps (half a step at 100 %)
        const float mix = std::clamp(p[kMix], 0.0f, 100.0f) * 0.01f;
        const float* pat = p + kPatternBase + 3 * pattern;
        const auto steps = static_cast<int64_t>(std::clamp(static_cast<int>(std::lround(pat[0])), 1, 32));
        const auto bit = [](float v) { return static_cast<uint32_t>(std::clamp(v, 0.0f, 65535.0f)); };
        const uint32_t mask = bit(pat[1]) | (bit(pat[2]) << 16);

        const RtTransport* tr = ctx.transport;
        const double tempo = tr != nullptr ? std::clamp(tr->tempo_bpm, 1.0, 999.0) : 120.0;
        const bool playing = tr != nullptr && tr->playing;
        // ctx.offset: frames from the transport position to this segment's first frame.
        const double beat0 = playing ? tr->beats() + static_cast<double>(ctx.offset) * tempo / 60.0 / sr_ : free_beats_;
        const double quarters = kStepQuarters[rate];
        const double du = tempo / 60.0 / sr_ / quarters; // steps per sample
        if (!primed_) {
            mix_.snap(mix);
            primed_ = true;
        }

        const double u0 = beat0 / quarters;
        float level = level_;
        float* const env = env_.data();
        for (uint32_t i = 0; i < n; ++i) {
            const double u = u0 + static_cast<double>(i) * du;
            const double k = std::floor(u);
            const auto ki = static_cast<int64_t>(k);
            // Odd steps start late by the swing: until then the previous step continues.
            const int64_t step = ((ki & 1) != 0 && u - k < swing) ? ki - 1 : ki;
            if (step != last_step_) {
                last_step_ = step;
                const int64_t idx = ((step % steps) + steps) % steps;
                if (((mask >> idx) & 1u) != 0) {
                    stage_ = kAttackStage;
                } else if (stage_ != kIdle && stage_ != kReleaseStage) {
                    stage_ = kReleaseStage;
                    release_rate_ = level * ms_per_sample / release_ms;
                }
            }
            switch (stage_) {
            case kAttackStage:
                level += attack_step;
                if (level >= 1.0f) {
                    level = 1.0f;
                    stage_ = kDecayStage;
                }
                break;
            case kDecayStage:
                level -= decay_step;
                if (level <= sustain) {
                    level = sustain;
                    stage_ = kSustainStage;
                }
                break;
            case kSustainStage: level = sustain; break;
            case kReleaseStage:
                level -= release_rate_;
                if (level <= 0.0f) {
                    level = 0.0f;
                    stage_ = kIdle;
                }
                break;
            default: level = 0.0f; break;
            }
            env[i] = level;
        }
        level_ = level;
        free_beats_ = playing ? beat0 + static_cast<double>(n) * du * quarters : free_beats_ + static_cast<double>(n) * du * quarters;

        if (mix_.settled(mix)) {
            const float m = mix;
            for (uint32_t i = 0; i < n; ++i) {
                const float g = 1.0f + m * (env[i] - 1.0f);
                l[i] *= g;
                r[i] *= g;
            }
        } else {
            for (uint32_t i = 0; i < n; ++i) {
                const float g = 1.0f + mix_.next(mix) * (env[i] - 1.0f);
                l[i] *= g;
                r[i] *= g;
            }
        }
    }

private:
    double sr_ = 48000.0;
    std::vector<float> env_;
    dsp::Smoother mix_;
    float level_ = 0.0f;
    float release_rate_ = 0.0f;
    int stage_ = kIdle;
    int64_t last_step_ = INT64_MIN;
    double free_beats_ = 0.0;
    bool primed_ = false;
};

std::unique_ptr<BuiltinProcessor> create_trance_gate() { return std::make_unique<TranceGate>(); }

} // namespace

// The definition's defaults (saine_core::builtin::trance_gate), in parameter order.
constexpr float kDefaults[] = {0.f,  7.f,  3.f,     160.f, 60.f, 40.f,    0.f,  100.f, 16.f, 23387.f, 0.f,  16.f, 21845.f, 0.f,  16.f, 46967.f,
                               0.f,  16.f, 19983.f, 0.f,   12.f, 2779.f,  0.f,  32.f,  44397.f, 30125.f, 16.f, 18761.f, 0.f,  16.f, 56669.f, 0.f};
static_assert(sizeof kDefaults / sizeof kDefaults[0] == sizeof kParamIds / sizeof kParamIds[0]);

extern const BuiltinDescriptor kTranceGateDescriptor;
const BuiltinDescriptor kTranceGateDescriptor{"trance-gate", kParamIds, {}, &create_trance_gate, {}, kDefaults};

} // namespace saine
