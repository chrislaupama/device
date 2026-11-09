// Grains: a live granular processor. Parameters in the order of
// saine_core::builtin::grains (checked by a Rust test).
//
//   input (+ grain feedback) -> 20 s stereo capture buffer (stops while frozen)
//   scheduler: Cloud = scattered onsets at Density per second (exponential
//              gaps); Rhythm = one grain per Rate step on the transport grid,
//              fired with Chance; Freeze in Rhythm waits for the next step
//   each grain: reaches back Position (+ random Spray), plays Size (Cloud) or
//              Gate x step (Rhythm) at Pitch + a random Spread snapped to the
//              Scale, reversed with probability Reverse, panned within
//              Width, enveloped by Shape (hard -> Hann -> soft bell), with
//              slow random Drift of position and pitch
//   grain bus -> Character (clean / tape wobble + saturation / 12-bit
//              26 kHz sampler) -> Blur (allpass diffusion) -> Tone (tilt)
//              -> Space (small reverb) -> Output gain -> Mix with the dry.
//
// Up-pitched grains read faster than the write head; their start is pushed
// back by the distance they travel so they never cross it. Voices come from
// a fixed pool of 64 (the oldest is stolen). The scale of Key / Key Chord /
// Pentatonic is relative to the grain (no project key reaches the engine).
#include "builtin.hpp"
#include "dsp.hpp"
#include "dsp_delay.hpp"

#include <algorithm>
#include <array>
#include <cmath>

namespace saine {

namespace {

enum Param : uint32_t {
    kMode, kFreeze, kPosition, kPositionNote, kSize, kGate, kDensity, kRate, kChance, kSpray, kPitch, kSpread, kScale, kReverse,
    kShape, kWidth, kDrift, kFeedback, kBlur, kTone, kSpace, kCharacter, kMix, kMixLock, kOutput, kParamCount
};
constexpr const char* kParamIds[kParamCount] = {"mode", "freeze", "position", "position_note", "size", "gate", "density", "rate",
                                                "chance", "spray", "pitch", "spread", "scale", "reverse", "shape", "width",
                                                "drift", "feedback", "blur", "tone", "space", "character", "mix", "mix_lock", "output"};
constexpr uint32_t kShown = 8; // grains reported to the card
// Readouts: input peak (dB), active grain count, then per shown grain:
// distance back from now (s), pitch (st), envelope level (0..1).
constexpr const char* kReadoutIds[2 + 3 * kShown] = {"in_db", "grains", "g0_t", "g0_st", "g0_lv", "g1_t", "g1_st", "g1_lv",
                                                     "g2_t", "g2_st", "g2_lv", "g3_t", "g3_st", "g3_lv", "g4_t", "g4_st", "g4_lv",
                                                     "g5_t", "g5_st", "g5_lv", "g6_t", "g6_st", "g6_lv", "g7_t", "g7_st", "g7_lv"};
constexpr float kPositionBeats[10] = {0.0f, 0.125f, 0.25f, 0.5f, 0.75f, 1.0f, 2.0f, 4.0f, 8.0f, 16.0f};
constexpr float kRateBeats[12] = {2.0f, 1.5f, 1.0f, 2.0f / 3.0f, 0.75f, 0.5f, 1.0f / 3.0f, 0.375f, 0.25f, 1.0f / 6.0f, 0.125f, 0.0625f};
constexpr float kBufferSeconds = 20.0f;
constexpr int kVoices = 64;
constexpr int kWindow = 1024;

struct Voice {
    bool active = false;
    double pos = 0.0;   // read position, samples behind the write head at spawn (absolute index)
    float inc = 1.0f;   // read increment (negative = reversed)
    float ratio = 1.0f;
    float phase = 0.0f; // 0..1 through the grain
    float dphase = 0.0f;
    float gl = 1.0f, gr = 1.0f;
    float st = 0.0f;
    uint32_t born = 0;
};

// Allowed semitone offsets of a scale within one octave.
const std::array<int, 12>* scale_steps(int scale, int& count) {
    static constexpr std::array<int, 12> kChromatic = {0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11};
    static constexpr std::array<int, 12> kOct = {0};
    static constexpr std::array<int, 12> kOct5 = {0, 7};
    static constexpr std::array<int, 12> kKey = {0, 2, 4, 5, 7, 9, 11};
    static constexpr std::array<int, 12> kChord = {0, 4, 7};
    static constexpr std::array<int, 12> kPenta = {0, 2, 4, 7, 9};
    switch (scale) {
    case 1: count = 12; return &kChromatic;
    case 2: count = 1; return &kOct;
    case 3: count = 2; return &kOct5;
    case 4: count = 7; return &kKey;
    case 5: count = 3; return &kChord;
    case 6: count = 5; return &kPenta;
    default: count = 0; return nullptr;
    }
}

float snap_to_scale(float st, float limit, int scale) {
    int count = 0;
    const auto* steps = scale_steps(scale, count);
    if (steps == nullptr) return st;
    float best = 0.0f, best_d = 1e9f;
    for (int oct = -3; oct <= 3; ++oct)
        for (int k = 0; k < count; ++k) {
            const float c = static_cast<float>(12 * oct + (*steps)[static_cast<std::size_t>(k)]);
            if (std::fabs(c) > limit + 1e-3f) continue;
            const float d = std::fabs(c - st);
            if (d < best_d) {
                best_d = d;
                best = c;
            }
        }
    return best;
}

class Grains final : public BuiltinProcessor {
public:
    void prepare(double sample_rate, uint32_t) override {
        sr_ = sample_rate;
        len_ = 1;
        while (static_cast<double>(len_) < static_cast<double>(kBufferSeconds) * sr_) len_ <<= 1;
        mask_ = len_ - 1;
        buf_[0].assign(len_, 0.0f);
        buf_[1].assign(len_, 0.0f);
        for (auto& side : blur_)
            for (auto& a : side) a.allocate(static_cast<std::size_t>(0.06 * sr_));
        for (auto& d : space_) d.prepare(static_cast<std::size_t>(0.12 * sr_));
        tone_lo_[0].set(dsp::Biquad::kLowShelf, 700.0f, 0.6f, 0.0f, sr_);
        reset();
    }

    void reset() noexcept override {
        std::fill(buf_[0].begin(), buf_[0].end(), 0.0f);
        std::fill(buf_[1].begin(), buf_[1].end(), 0.0f);
        w_ = 0;
        for (auto& v : voices_) v = Voice{};
        next_onset_ = 0.0;
        clock_beats_ = 0.0;
        last_step_ = -1;
        frozen_ = false;
        for (auto& side : blur_)
            for (auto& a : side) a.clear();
        for (auto& d : space_) d.reset();
        for (float& s : space_lp_) s = 0.0f;
        for (int c = 0; c < 2; ++c) {
            tone_lo_[c].reset();
            tone_hi_[c].reset();
            char_lp_[c].reset();
            hold_[c] = 0.0f;
        }
        hold_phase_ = 0.0f;
        wob_ = 0.0f;
        drift_pos_ = drift_st_ = 0.0f;
        fb_l_ = fb_r_ = 0.0f;
        rng_.seed(0xa5a5f00du);
        shape_key_ = -1.0f;
        tone_key_ = -1000.0f;
        clock_ = 0;
        primed_ = false;
    }

    void process(float* l, float* r, uint32_t n, const float* p, const BuiltinContext& ctx, float* ro) noexcept override {
        const auto choice = [&](uint32_t i, int hi) { return std::clamp(static_cast<int>(std::lround(p[i])), 0, hi); };
        const bool rhythm = choice(kMode, 1) == 1;
        const bool freeze = p[kFreeze] >= 0.5f;
        const double bpm = std::clamp(ctx.transport != nullptr ? ctx.transport->tempo_bpm : 120.0, 20.0, 999.0);
        const double beat_s = 60.0 / bpm;
        const float step_s = static_cast<float>(static_cast<double>(kRateBeats[choice(kRate, 11)]) * beat_s);
        const float pos_s = rhythm ? static_cast<float>(static_cast<double>(kPositionBeats[choice(kPositionNote, 9)]) * beat_s) : std::clamp(p[kPosition], 0.0f, 8000.0f) * 0.001f;
        const float spray_ms = std::clamp(p[kSpray], 0.0f, 2000.0f);
        const float spray_s = rhythm ? (spray_ms > 0.5f ? std::max(step_s, spray_ms * 0.001f) : 0.0f) : spray_ms * 0.001f;
        const float grain_s = rhythm ? step_s * std::clamp(p[kGate], 5.0f, 400.0f) * 0.01f : std::clamp(p[kSize], 5.0f, 2000.0f) * 0.001f;
        const float density = std::clamp(p[kDensity], 0.5f, 200.0f);
        const float chance = std::clamp(p[kChance], 0.0f, 100.0f) * 0.01f;
        const float pitch = std::clamp(p[kPitch], -24.0f, 24.0f);
        const float spread = std::clamp(p[kSpread], 0.0f, 24.0f);
        const int scale = choice(kScale, 6);
        const float reverse = std::clamp(p[kReverse], 0.0f, 100.0f) * 0.01f;
        const float width = std::clamp(p[kWidth], 0.0f, 100.0f) * 0.01f;
        const float drift = std::clamp(p[kDrift], 0.0f, 100.0f) * 0.01f;
        const float feedback = std::clamp(p[kFeedback], 0.0f, 95.0f) * 0.01f;
        const float blur = std::clamp(p[kBlur], 0.0f, 100.0f) * 0.01f;
        const float tone = std::clamp(p[kTone], -100.0f, 100.0f) * 0.01f;
        const float space = std::clamp(p[kSpace], 0.0f, 100.0f) * 0.01f;
        const int character = choice(kCharacter, 2);
        const float mix = std::clamp(p[kMix], 0.0f, 100.0f) * 0.01f;
        const float out_gain = dsp::db_to_gain(std::clamp(p[kOutput], -24.0f, 12.0f));
        update_window(std::clamp(p[kShape], 0.0f, 100.0f) * 0.01f);
        if (tone != tone_key_) {
            tone_key_ = tone;
            for (int c = 0; c < 2; ++c) {
                tone_lo_[c].set(dsp::Biquad::kLowShelf, 650.0f, 0.6f, -9.0f * tone, sr_);
                tone_hi_[c].set(dsp::Biquad::kHighShelf, 2200.0f, 0.6f, 9.0f * tone, sr_);
            }
        }
        for (auto& f : char_lp_) f.set_hz(character == 1 ? 7000.0f : 11000.0f, sr_);
        // Overlap normalisation: expected simultaneous grains.
        const float overlap = rhythm ? std::max(1.0f, grain_s / std::max(step_s, 1e-3f)) : std::max(1.0f, density * grain_s);
        const float grain_gain = 1.0f / std::sqrt(overlap);
        if (!primed_) {
            mix_ = mix;
            primed_ = true;
        }
        const double block_beats = ctx.transport != nullptr && ctx.transport->playing ? ctx.transport->beats() : clock_beats_;
        const double beats_per_sample = 1.0 / (beat_s * sr_);
        const auto step_beats = static_cast<double>(kRateBeats[choice(kRate, 11)]);

        float in_peak = 0.0f;
        for (uint32_t i = 0; i < n; ++i) {
            const float xl = l[i], xr = r[i];
            in_peak = std::max(in_peak, std::max(std::fabs(xl), std::fabs(xr)));
            ++clock_;
            // Freeze (Rhythm: on the next step).
            const double beat_now = block_beats + static_cast<double>(i) * beats_per_sample;
            const auto step = static_cast<int64_t>(std::floor(beat_now / step_beats + 1e-9));
            const bool new_step = step != last_step_;
            if (freeze != frozen_) {
                if (!rhythm || !freeze || new_step) frozen_ = freeze;
            }
            if (!frozen_) {
                buf_[0][w_] = dsp::flush(xl + feedback * fb_l_);
                buf_[1][w_] = dsp::flush(xr + feedback * fb_r_);
                w_ = (w_ + 1) & mask_;
            }
            // Drift: slow random walk.
            if ((clock_ & 255u) == 0) {
                drift_pos_ = std::clamp(drift_pos_ + rng_.bi() * 0.05f, -1.0f, 1.0f);
                drift_st_ = std::clamp(drift_st_ + rng_.bi() * 0.05f, -1.0f, 1.0f);
            }
            // Scheduling.
            bool fire = false;
            if (rhythm) {
                if (new_step) {
                    last_step_ = step;
                    fire = rng_.uni() < chance;
                }
            } else {
                next_onset_ -= 1.0;
                if (next_onset_ <= 0.0) {
                    fire = true;
                    const float u = std::max(rng_.uni(), 1e-4f);
                    next_onset_ += static_cast<double>(std::clamp(-std::log(u), 0.2f, 4.0f)) * sr_ / static_cast<double>(density);
                }
            }
            if (fire) spawn(pos_s, spray_s, grain_s, pitch, spread, scale, reverse, width, drift);

            // Play voices.
            float gl = 0.0f, gr = 0.0f;
            const float wob = character == 1 ? 1.0f + 0.003f * dsp::fast_sin(wob_ += 0.9f / static_cast<float>(sr_)) : 1.0f;
            if (wob_ >= 1.0f) wob_ -= 1.0f;
            for (Voice& v : voices_) {
                if (!v.active) continue;
                const float env = window_[static_cast<std::size_t>(std::min(v.phase, 0.9999f) * (kWindow - 1))];
                const float sl = read(0, v.pos), sr = read(1, v.pos);
                gl += sl * env * v.gl;
                gr += sr * env * v.gr;
                v.pos += static_cast<double>(v.inc * wob);
                v.phase += v.dphase;
                if (v.phase >= 1.0f) v.active = false;
            }
            gl *= grain_gain;
            gr *= grain_gain;
            // Character.
            if (character == 1) {
                gl = dsp::soft_clip(char_lp_[0].lp(gl) * 1.4f) / 1.4f;
                gr = dsp::soft_clip(char_lp_[1].lp(gr) * 1.4f) / 1.4f;
            } else if (character == 2) {
                hold_phase_ += 26040.0f / static_cast<float>(sr_);
                if (hold_phase_ >= 1.0f) {
                    hold_phase_ -= 1.0f;
                    hold_[0] = std::round(gl * 2048.0f) / 2048.0f;
                    hold_[1] = std::round(gr * 2048.0f) / 2048.0f;
                }
                gl = char_lp_[0].lp(hold_[0]);
                gr = char_lp_[1].lp(hold_[1]);
            }
            // Blur: diffusion.
            if (blur > 0.0f) {
                constexpr float kMs[4] = {7.1f, 11.3f, 17.9f, 29.3f};
                float bl = gl, br = gr;
                for (int k = 0; k < 4; ++k) {
                    const float d = kMs[k] * 0.001f * static_cast<float>(sr_) * (0.5f + blur);
                    bl = blur_[0][k].process(bl, d, 0.65f * blur);
                    br = blur_[1][k].process(br, d * 1.09f, 0.65f * blur);
                }
                gl += blur * (bl - gl);
                gr += blur * (br - gr);
            }
            gl = tone_hi_[0].process(tone_lo_[0].process(gl));
            gr = tone_hi_[1].process(tone_lo_[1].process(gr));
            fb_l_ = dsp::soft_clip(gl);
            fb_r_ = dsp::soft_clip(gr);
            // Space: 4-line feedback network.
            if (space > 0.0f) {
                constexpr float kMs[4] = {37.3f, 47.9f, 61.1f, 73.7f};
                float v[4];
                for (int k = 0; k < 4; ++k) {
                    float y = space_[k].read_linear(kMs[k] * 0.001f * static_cast<float>(sr_) * (0.6f + 0.6f * space) - 1.0f);
                    space_lp_[k] = y + 0.35f * (space_lp_[k] - y);
                    v[k] = space_lp_[k] * (0.72f + 0.2f * space);
                }
                const float a = v[0] + v[1], b = v[0] - v[1], c = v[2] + v[3], d = v[2] - v[3];
                const float h[4] = {0.5f * (a + c), 0.5f * (b + d), 0.5f * (a - c), 0.5f * (b - d)};
                space_[0].push(dsp::flush(h[0] + gl * 0.5f));
                space_[1].push(dsp::flush(h[1] + gr * 0.5f));
                space_[2].push(dsp::flush(h[2] + gl * 0.5f));
                space_[3].push(dsp::flush(h[3] + gr * 0.5f));
                gl += space * (v[0] + v[2]) * 0.6f;
                gr += space * (v[1] + v[3]) * 0.6f;
            }
            gl *= out_gain;
            gr *= out_gain;
            mix_ += (mix - mix_) * 0.001f;
            l[i] = xl + mix_ * (gl - xl);
            r[i] = xr + mix_ * (gr - xr);
        }
        clock_beats_ = block_beats + static_cast<double>(n) * beats_per_sample;
        for (int c = 0; c < 2; ++c) {
            tone_lo_[c].flush_state();
            tone_hi_[c].flush_state();
            char_lp_[c].y = dsp::flush(char_lp_[c].y);
        }
        for (float& s : space_lp_) s = dsp::flush(s);

        // Readouts: the newest grains.
        ro[0] = dsp::gain_to_db(in_peak);
        int active = 0;
        for (const Voice& v : voices_) active += v.active ? 1 : 0;
        ro[1] = static_cast<float>(active);
        uint32_t k = 0;
        for (int s = 0; s < kVoices && k < kShown; ++s) {
            const Voice& v = voices_[static_cast<std::size_t>((next_voice_ - 1 - s + 2 * kVoices) % kVoices)];
            if (!v.active) continue;
            const double back = static_cast<double>((w_ - static_cast<uint32_t>(static_cast<int64_t>(std::floor(v.pos)) & mask_)) & mask_);
            ro[2 + 3 * k] = static_cast<float>(back / sr_);
            ro[3 + 3 * k] = v.st;
            ro[4 + 3 * k] = window_[static_cast<std::size_t>(std::min(v.phase, 0.9999f) * (kWindow - 1))];
            ++k;
        }
        for (; k < kShown; ++k) ro[2 + 3 * k] = ro[3 + 3 * k] = ro[4 + 3 * k] = 0.0f;
    }

private:
    float read(int c, double pos) const noexcept {
        const double f = std::floor(pos);
        const auto i = static_cast<uint32_t>(static_cast<int64_t>(f)) & mask_;
        const auto t = static_cast<float>(pos - f);
        const std::vector<float>& b = buf_[c];
        return dsp::hermite(b[(i - 1) & mask_], b[i], b[(i + 1) & mask_], b[(i + 2) & mask_], t);
    }

    void update_window(float shape) noexcept {
        if (shape == shape_key_) return;
        shape_key_ = shape;
        // 0: nearly rectangular (2% fades), 0.5: Hann, 1: narrow bell (Hann^3).
        const float taper = std::clamp(shape * 2.0f, 0.02f, 1.0f);
        const float power = shape > 0.5f ? 1.0f + 4.0f * (shape - 0.5f) : 1.0f;
        for (int i = 0; i < kWindow; ++i) {
            const float x = static_cast<float>(i) / (kWindow - 1);
            const float edge = std::min(x, 1.0f - x) / (0.5f * taper);
            float w = edge >= 1.0f ? 1.0f : 0.5f - 0.5f * std::cos(dsp::kPi * edge);
            if (power != 1.0f) w = std::pow(w, power);
            window_[static_cast<std::size_t>(i)] = w;
        }
    }

    void spawn(float pos_s, float spray_s, float grain_s, float pitch, float spread, int scale, float reverse, float width, float drift) noexcept {
        Voice& v = voices_[static_cast<std::size_t>(next_voice_)];
        next_voice_ = (next_voice_ + 1) % kVoices;
        float st = pitch + drift * 0.5f * drift_st_;
        if (spread > 0.0f) st += snap_to_scale(rng_.bi() * spread, spread, scale);
        st = std::clamp(st, -48.0f, 48.0f);
        const float ratio = std::exp2(st / 12.0f);
        // A grain reads len * ratio samples; keep that within half the capture so a start exists
        // that neither overruns the write head nor reaches audio older than the buffer.
        const float max_read = 0.5f * static_cast<float>(len_) - 0.002f * static_cast<float>(sr_) - 8.0f;
        const float len = std::max(std::min(grain_s * static_cast<float>(sr_), max_read / ratio), 16.0f);
        const bool rev = rng_.uni() < reverse;
        float back = (pos_s + rng_.uni() * spray_s + drift * 0.5f * std::fabs(drift_pos_)) * static_cast<float>(sr_);
        // Forward grains must not overrun the write head: it moves at 1x, or not at all while frozen.
        const float travel = len * std::max(0.0f, ratio - (frozen_ ? 0.0f : 1.0f));
        back = std::max(back, travel + 0.002f * static_cast<float>(sr_));
        if (rev) back = std::max(back, len * ratio + 4.0f); // reversed grains read back from their start
        back = std::min(back, static_cast<float>(len_) - len * ratio - 8.0f);
        v.active = true;
        v.ratio = ratio;
        v.st = st;
        v.inc = rev ? -ratio : ratio;
        v.pos = static_cast<double>(w_) - static_cast<double>(back) + static_cast<double>(len_);
        v.phase = 0.0f;
        v.dphase = 1.0f / len;
        const float pan = std::clamp(rng_.bi() * width, -1.0f, 1.0f);
        const float a = (pan + 1.0f) * 0.25f * dsp::kPi;
        v.gl = std::cos(a) * 1.41421356f;
        v.gr = std::sin(a) * 1.41421356f;
    }

    double sr_ = 48000.0;
    std::vector<float> buf_[2];
    uint32_t len_ = 1, mask_ = 0, w_ = 0;
    std::array<Voice, kVoices> voices_{};
    int next_voice_ = 0;
    std::array<float, kWindow> window_{};
    float shape_key_ = -1.0f, tone_key_ = -1000.0f;
    double next_onset_ = 0.0, clock_beats_ = 0.0;
    int64_t last_step_ = -1;
    bool frozen_ = false;
    dsp::Allpass blur_[2][4];
    dsp::DelayLine space_[4];
    float space_lp_[4] = {};
    dsp::Biquad tone_lo_[2], tone_hi_[2];
    dsp::ExpOnePole char_lp_[2];
    float hold_[2] = {}, hold_phase_ = 0.0f, wob_ = 0.0f;
    float drift_pos_ = 0.0f, drift_st_ = 0.0f, fb_l_ = 0.0f, fb_r_ = 0.0f, mix_ = 0.0f;
    uint32_t clock_ = 0;
    dsp::UnitRng rng_;
    bool primed_ = false;
};

std::unique_ptr<BuiltinProcessor> create() { return std::make_unique<Grains>(); }

} // namespace

extern const BuiltinDescriptor kGrainsDescriptor;
constexpr float kIdle[2 + 3 * kShown] = {-120.0f};
constexpr float kDefaults[kParamCount] = {0.f, 0.f, 250.f, 3.f, 120.f, 100.f, 24.f, 8.f, 100.f, 40.f, 0.f, 0.f, 4.f,
                                          0.f, 50.f, 50.f, 0.f, 0.f, 0.f, 0.f, 0.f, 0.f, 50.f, 0.f, 0.f};
const BuiltinDescriptor kGrainsDescriptor{"grains", kParamIds, kReadoutIds, &create, kIdle, kDefaults};

} // namespace saine
