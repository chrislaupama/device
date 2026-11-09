// LFO (handoffs/device-specs/lfo.md): a modulation source. Audio passes
// through; modulate() gives the wave position (0..1) and Depth for the
// engine to apply to the LFO's routes (se_mod_route).
//
// Timing: Free runs a phase accumulator at Rate (Hz) from the moment the
// device is created. Sync follows the project tempo: while the transport
// plays the cycle position is the song position in quarter notes divided by
// the division length, so it lines up with the bar; while stopped it keeps
// counting at the tempo from where it was. Phase is added before the shape.
// Readouts: the last wave position, the depth and the position inside the
// cycle 0..1 (for the panel's live dot).
#include "builtin.hpp"
#include "dsp.hpp"
#include "lfo.hpp"

#include <algorithm>
#include <cmath>

namespace saine {
namespace {

constexpr const char* kParams[] = {"shape", "sync", "rate", "rateSync", "depth", "phase"};
constexpr const char* kReadouts[] = {"position", "depth", "cycle"};
constexpr float kIdle[] = {0.5f, 0.0f, 0.0f};
constexpr float kDefaults[] = {0.0f, 1.0f, 1.0f, 8.0f, 100.0f, 0.0f};

class Lfo final : public BuiltinProcessor {
public:
    void prepare(double sample_rate, uint32_t) override { rate_ = sample_rate > 0.0 ? sample_rate : 48000.0; }
    void reset() noexcept override {
        free_ = 0.0;
        beats_ = 0.0;
        was_playing_ = false;
    }

    void process(float*, float*, uint32_t, const float*, const BuiltinContext&, float* readouts) noexcept override {
        readouts[0] = last_pos_;
        readouts[1] = last_depth_;
        readouts[2] = last_cycle_;
    }

    uint32_t mod_outputs() const noexcept override { return 1; }

    void modulate(const float* p, uint32_t frames, const BuiltinContext& ctx, float* out) noexcept override {
        const auto shape = static_cast<uint32_t>(std::clamp(std::lround(p[0]), 0l, static_cast<long>(lfo::kShapeCount) - 1));
        const bool sync = p[1] >= 0.5f;
        const double hz = std::clamp(static_cast<double>(p[2]), 0.01, 40.0);
        const auto div = static_cast<std::size_t>(std::clamp(std::lround(p[3]), 0l, 19l));
        const float depth = std::clamp(p[4] / 100.0f, 0.0f, 1.0f);
        const double phase = std::clamp(static_cast<double>(p[5]) / 100.0, 0.0, 1.0);
        const RtTransport* t = ctx.transport;
        const double tempo = t != nullptr && t->tempo_bpm > 0.0 ? t->tempo_bpm : 120.0;
        const bool playing = t != nullptr && t->playing;
        if (playing) beats_ = t->beats(); // the song position, also right after play starts
        was_playing_ = playing;
        const double x = (sync ? beats_ / static_cast<double>(dsp::kDivisionQuarters[div]) : free_) + phase;
        const float pos = 0.5f * (lfo::wave(shape, x) + 1.0f);
        out[0] = pos;
        out[1] = depth;
        last_pos_ = pos;
        last_depth_ = depth;
        last_cycle_ = static_cast<float>(x - std::floor(x));
        // Advance both clocks so switching Sync keeps running smoothly; wrap
        // the free phase at 4096 cycles (the random shapes repeat there).
        free_ = std::fmod(free_ + hz * frames / rate_, 4096.0);
        beats_ += frames * tempo / 60.0 / rate_;
    }

private:
    double rate_ = 48000.0;
    double free_ = 0.0;
    double beats_ = 0.0;
    bool was_playing_ = false;
    float last_pos_ = 0.5f;
    float last_depth_ = 0.0f;
    float last_cycle_ = 0.0f;
};

std::unique_ptr<BuiltinProcessor> create() { return std::make_unique<Lfo>(); }

} // namespace

extern const BuiltinDescriptor kLfoDescriptor;
const BuiltinDescriptor kLfoDescriptor{"lfo", kParams, kReadouts, &create, kIdle, kDefaults};

} // namespace saine
