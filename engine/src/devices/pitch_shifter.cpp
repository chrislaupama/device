// Pitch Shifter: semitones + cents through the shared phase vocoder
// (pitch_shift.hpp), optional formant preservation, dry/wet mix. The dry
// path is delayed by the vocoder latency so the mix never combs, and the
// whole device reports that latency for delay compensation. At exactly zero
// shift with formants off the wet path is the delayed input (bit exact).
// Parameters in the order of saine_core::builtin::pitch_shifter.
#include "builtin.hpp"
#include "dsp.hpp"
#include "dsp_delay.hpp"
#include "pitch_shift.hpp"

#include <algorithm>
#include <cmath>

namespace saine {

namespace {

enum Param : uint32_t { kSemitones, kCents, kFormant, kMix, kParamCount };
constexpr const char* kParamIds[kParamCount] = {"semitones", "cents", "formant_preserve", "mix"};
enum Readout : uint32_t { kShiftSt, kInDb, kReadoutCount };
constexpr const char* kReadoutIds[kReadoutCount] = {"shift_st", "in_db"};

uint32_t fft_size_for(double sample_rate) {
    uint32_t n = 2048;
    while (static_cast<double>(n) < sample_rate * 2048.0 / 48000.0 - 1.0) n <<= 1;
    return n;
}

class PitchShifterDevice final : public BuiltinProcessor {
public:
    void prepare(double sample_rate, uint32_t max_block) override {
        sr_ = sample_rate;
        const uint32_t n = fft_size_for(sample_rate);
        for (auto& s : shift_) s.prepare(n);
        for (auto& d : dry_) d.allocate(n + max_block + 8);
        wet_l_.assign(max_block, 0.0f);
        wet_r_.assign(max_block, 0.0f);
        mix_.set_time(20.0f, sr_);
    }

    void reset() noexcept override {
        for (auto& s : shift_) s.reset();
        for (auto& d : dry_) d.clear();
        primed_ = false;
    }

    uint32_t latency() const noexcept override { return shift_[0].latency(); }

    void process(float* l, float* r, uint32_t n, const float* p, const BuiltinContext&, float* ro) noexcept override {
        const float st = std::clamp(p[kSemitones], -24.0f, 24.0f);
        const float ct = std::clamp(p[kCents], -100.0f, 100.0f);
        const bool formant = p[kFormant] >= 0.5f;
        const float mix = std::clamp(p[kMix], 0.0f, 100.0f) * 0.01f;
        const float shift = st + ct * 0.01f;
        const bool unity = std::fabs(shift) < 1e-4f && !formant;
        const float ratio = std::exp2(shift / 12.0f);
        if (!primed_) {
            mix_.snap(mix);
            primed_ = true;
        }
        const uint32_t lat = latency();
        float in_peak = 0.0f;
        // Blocks larger than the scratch never happen (max_block), but stay safe.
        for (uint32_t off = 0; off < n;) {
            const auto m = std::min<uint32_t>(n - off, static_cast<uint32_t>(wet_l_.size()));
            float* cl = l + off;
            float* cr = r + off;
            in_peak = std::max(in_peak, dsp::stereo_peak(cl, cr, m));
            for (int c = 0; c < 2; ++c) {
                shift_[c].set_ratio(ratio);
                shift_[c].set_formant(formant);
                shift_[c].process(c == 0 ? cl : cr, c == 0 ? wet_l_.data() : wet_r_.data(), m);
            }
            for (uint32_t i = 0; i < m; ++i) {
                dry_[0].push(cl[i]);
                dry_[1].push(cr[i]);
                const float dl = dry_[0].at(lat + 1), dr = dry_[1].at(lat + 1);
                const float wl = unity ? dl : wet_l_[i], wr = unity ? dr : wet_r_[i];
                const float g = mix_.next(mix);
                cl[i] = dl + g * (wl - dl);
                cr[i] = dr + g * (wr - dr);
            }
            off += m;
        }
        ro[kShiftSt] = shift;
        ro[kInDb] = dsp::gain_to_db(in_peak);
    }

private:
    double sr_ = 48000.0;
    dsp::PitchShifter shift_[2];
    dsp::DelayLine dry_[2];
    std::vector<float> wet_l_, wet_r_;
    dsp::Smoother mix_;
    bool primed_ = false;
};

std::unique_ptr<BuiltinProcessor> create() { return std::make_unique<PitchShifterDevice>(); }

} // namespace

extern const BuiltinDescriptor kPitchShifterDescriptor;
constexpr float kIdle[] = {0.0f, -120.0f};
const BuiltinDescriptor kPitchShifterDescriptor{"pitch-shifter", kParamIds, kReadoutIds, &create, kIdle};

} // namespace saine
