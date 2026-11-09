// Tuner: shows the note being played and how far off it is; audio passes
// through unchanged unless Mute Output is on. Parameters in the order of
// saine_core::builtin::tuner (checked by a Rust test).
//
// Detection: the shared PitchDetector (pitch_detect.hpp) on the mid signal,
// 30 Hz .. 1.5 kHz, an estimate every 5 ms. The reading is the median of
// the last five estimates (in pitch), smoothed with a 40 ms one-pole.
// Chromatic names the nearest semitone; a string tuning names the nearest
// string (0.3 semitone hysteresis), so the cents are measured against it.
// In tune = within 3 cents (left again past 5). After the note stops the
// last reading is held for 1.2 s with the holding flag set.
//
// Readouts: voiced, target MIDI note, cents (-50..50), in tune, string index
// (-1 = none), holding.
#include "builtin.hpp"
#include "dsp.hpp"
#include "pitch_detect.hpp"

#include <algorithm>
#include <cmath>

namespace saine {

namespace {

enum Param : uint32_t { kReference, kTuning, kMute, kParamCount };
enum Readout : uint32_t { kVoiced, kNote, kCents, kInTune, kString, kHolding, kReadoutCount };

constexpr const char* kParamIds[kParamCount] = {"reference", "tuning", "mute_output"};
constexpr const char* kReadoutIds[kReadoutCount] = {"voiced", "note", "cents", "in_tune", "string", "holding"};

// Open strings (MIDI) per tuning, low to high; Chromatic (0) has none.
constexpr int kStrings[5][6] = {
    {0, 0, 0, 0, 0, 0},
    {40, 45, 50, 55, 59, 64}, // Standard
    {38, 45, 50, 55, 59, 64}, // Drop D
    {38, 45, 50, 55, 57, 62}, // DADGAD
    {40, 45, 50, 55, 59, 64}, // Sweetened: Standard with the offsets below
};
// Sweetened tuning offsets in cents (our own temperament: the G and B
// strings sit slightly flat so open major chords beat less).
constexpr float kSweetened[6] = {0.0f, 0.0f, 0.0f, -1.0f, -2.0f, 0.0f};

constexpr float kHoldSeconds = 1.2f;

class Tuner final : public BuiltinProcessor {
public:
    void prepare(double sample_rate, uint32_t) override {
        sr_ = sample_rate > 0.0 ? sample_rate : 48000.0;
        det_.prepare(sr_, 30.0f, 1500.0f, 5.0f);
        hop_s_ = static_cast<float>(det_.hop() / sr_);
        smooth_a_ = std::exp(-hop_s_ / 0.04f);
        fade_step_ = static_cast<float>(1.0 / (0.005 * sr_)); // 5 ms linear mute fade
    }

    void reset() noexcept override {
        det_.reset();
        hist_n_ = 0;
        shown_ = false;
        holding_ = false;
        hold_t_ = 0.0f;
        note_ = 0;
        string_ = -1;
        cents_ = 0.0f;
        in_tune_ = false;
        primed_ = false;
    }

    void process(float* l, float* r, uint32_t n, const float* p, const BuiltinContext&, float* ro) noexcept override {
        const float ref = std::lround(p[kReference]) == 1 ? 432.0f : 440.0f;
        const int tuning = std::clamp(static_cast<int>(std::lround(p[kTuning])), 0, 4);
        const float gain = p[kMute] >= 0.5f ? 0.0f : 1.0f;
        if (!primed_) {
            gain_ = gain;
            primed_ = true;
        }
        float mono[64];
        for (uint32_t off = 0; off < n; off += 64) {
            const uint32_t m = std::min<uint32_t>(64, n - off);
            for (uint32_t i = 0; i < m; ++i) mono[i] = 0.5f * (l[off + i] + r[off + i]);
            if (det_.push(mono, m) > 0) update(ref, tuning);
        }
        if (gain_ != gain || gain == 0.0f) {
            for (uint32_t i = 0; i < n; ++i) {
                gain_ = gain > gain_ ? std::min(gain, gain_ + fade_step_) : std::max(gain, gain_ - fade_step_);
                l[i] *= gain_;
                r[i] *= gain_;
            }
        }
        ro[kVoiced] = shown_ ? 1.0f : 0.0f;
        ro[kNote] = shown_ ? static_cast<float>(note_) : 0.0f;
        ro[kCents] = shown_ ? std::clamp(cents_, -50.0f, 50.0f) : 0.0f;
        ro[kInTune] = shown_ && in_tune_ ? 1.0f : 0.0f;
        ro[kString] = shown_ ? static_cast<float>(string_) : -1.0f;
        ro[kHolding] = shown_ && holding_ ? 1.0f : 0.0f;
    }

private:
    void update(float ref, int tuning) noexcept {
        const dsp::PitchEstimate& e = det_.estimate();
        if (!e.voiced) {
            hist_n_ = 0;
            if (shown_) {
                hold_t_ += hop_s_;
                holding_ = true;
                if (hold_t_ >= kHoldSeconds) shown_ = holding_ = false;
            }
            return;
        }
        // Median of the last five pitches.
        const float m = 69.0f + 12.0f * std::log2(e.hz / ref);
        for (int i = 4; i > 0; --i) hist_[i] = hist_[i - 1];
        hist_[0] = m;
        hist_n_ = std::min(hist_n_ + 1, 5);
        float s[5];
        std::copy(hist_, hist_ + hist_n_, s);
        std::sort(s, s + hist_n_);
        const float mf = s[hist_n_ / 2];

        int note = note_, str = string_;
        float pitch = static_cast<float>(note);
        if (tuning == 0) {
            str = -1;
            if (!shown_ || holding_ || std::fabs(mf - static_cast<float>(note_)) > 0.6f) note = static_cast<int>(std::lround(mf));
            pitch = static_cast<float>(note);
        } else {
            auto at = [&](int i) { return static_cast<float>(kStrings[tuning][i]) + (tuning == 4 ? kSweetened[i] * 0.01f : 0.0f); };
            int best = 0;
            for (int i = 1; i < 6; ++i)
                if (std::fabs(mf - at(i)) < std::fabs(mf - at(best))) best = i;
            if (!shown_ || holding_ || string_ < 0 || std::fabs(mf - at(string_)) - std::fabs(mf - at(best)) > 0.3f) str = best;
            note = kStrings[tuning][str];
            pitch = at(str);
        }
        const float c = (mf - pitch) * 100.0f;
        const bool snap = !shown_ || holding_ || note != note_ || str != string_;
        cents_ = snap ? c : c + smooth_a_ * (cents_ - c);
        note_ = note;
        string_ = str;
        const float a = std::fabs(cents_);
        in_tune_ = in_tune_ && !snap ? a <= 5.0f : a <= 3.0f;
        shown_ = true;
        holding_ = false;
        hold_t_ = 0.0f;
    }

    double sr_ = 48000.0;
    dsp::PitchDetector det_;
    float gain_ = 1.0f, fade_step_ = 0.004f;
    float hop_s_ = 0.005f, smooth_a_ = 0.88f;
    float hist_[5] = {0, 0, 0, 0, 0};
    int hist_n_ = 0;
    bool shown_ = false, holding_ = false, in_tune_ = false, primed_ = false;
    float hold_t_ = 0.0f, cents_ = 0.0f;
    int note_ = 0, string_ = -1;
};

std::unique_ptr<BuiltinProcessor> create_tuner() { return std::make_unique<Tuner>(); }

} // namespace

extern const BuiltinDescriptor kTunerDescriptor;
constexpr float kDefaults[] = {0.0f, 0.0f, 0.0f};
constexpr float kIdleReadouts[] = {0.0f, 0.0f, 0.0f, 0.0f, -1.0f, 0.0f};
const BuiltinDescriptor kTunerDescriptor{"tuner", kParamIds, kReadoutIds, &create_tuner, kIdleReadouts, kDefaults, false, true};

} // namespace saine
