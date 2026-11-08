#include "synth.hpp"

#include <algorithm>
#include <cmath>
#include <cstring>
#include <numbers>

namespace saine {

namespace {

// Envelope level below which a releasing voice is freed (-90 dB).
constexpr float kOffLevel = 3.1623e-5f;
// Starting phase of unison stack k (osc1; osc2 starts half a cycle later).
// Stack 0 starts at 0.25 (osc2 at 0.75), where every waveform is non-zero
// (saw +-0.5, square +-1, triangle and sine +-1), so the very first sample
// of a note (its note-on frame) is non-zero.
// Further stacks start at spread-out phases so they do not sum coherently.
constexpr float kStartPhase[Synth::kMaxUnison] = {0.25f, 0.61f, 0.83f, 0.07f};
constexpr float kPi = std::numbers::pi_v<float>;
constexpr float kButterworthQ = 0.70710678f;

// 2-sample polynomial band-limited step residual (polyBLEP), for a step of
// +2 at phase 0. t in [0, 1), dt = phase increment.
inline float blep(float t, float dt) noexcept {
    if (t < dt) {
        t /= dt;
        return t + t - t * t - 1.0f;
    }
    if (t > 1.0f - dt) {
        t = (t - 1.0f) / dt;
        return t * t + t + t + 1.0f;
    }
    return 0.0f;
}

// Integrated polyBLEP (polyBLAMP) residual for a slope change at phase 0.
inline float blamp(float t, float dt) noexcept {
    if (t < dt) {
        t = t / dt - 1.0f;
        return -(1.0f / 3.0f) * t * t * t;
    }
    if (t > 1.0f - dt) {
        t = (t - 1.0f) / dt + 1.0f;
        return (1.0f / 3.0f) * t * t * t;
    }
    return 0.0f;
}

inline float wrap01(float t) noexcept { return t >= 1.0f ? t - 1.0f : (t < 0.0f ? t + 1.0f : t); }

// sin(2 pi p) for p in [0, 1): odd degree-9 polynomial on a quarter period
// (max error ~4e-6, harmonics below -100 dB).
inline float sin_turns(float p) noexcept {
    float x = p - 0.5f; // sin(2 pi p) = -sin(2 pi x), x in [-0.5, 0.5)
    if (x > 0.25f)
        x = 0.5f - x;
    else if (x < -0.25f)
        x = -0.5f - x;
    const float z = 2.0f * kPi * x;
    const float z2 = z * z;
    const float s =
        z * (1.0f + z2 * (-1.0f / 6.0f + z2 * (1.0f / 120.0f + z2 * (-1.0f / 5040.0f + z2 * (1.0f / 362880.0f)))));
    return -s;
}

inline float sin_turns_any(float a) noexcept { return sin_turns(a - std::floor(a)); }

// Band-limited saw: falling ramp 1 -> -1, rising edge at phase 0. Falling
// (not rising) so its fundamental is in phase with the sine-aligned
// triangle and the saw <-> triangle morph never cancels the fundamental.
inline float saw(float t, float dt) noexcept { return 1.0f - 2.0f * t + blep(t, dt); }

// Band-limited triangle aligned with the sine (peak at 0.25, trough at 0.75).
inline float tri(float t, float dt) noexcept {
    const float naive = t < 0.25f ? 4.0f * t : (t < 0.75f ? 2.0f - 4.0f * t : 4.0f * t - 4.0f);
    return naive + 4.0f * dt * (blamp(wrap01(t - 0.75f), dt) - blamp(wrap01(t - 0.25f), dt));
}

inline float white(uint32_t& r) noexcept {
    r ^= r << 13;
    r ^= r >> 17;
    r ^= r << 5;
    return static_cast<float>(static_cast<int32_t>(r)) * (1.0f / 2147483648.0f);
}

inline float lfo_value(uint8_t wave, float p) noexcept {
    switch (wave) {
    case SE_WAVE_SQUARE: return p < 0.5f ? 1.0f : -1.0f;
    case SE_WAVE_TRIANGLE: return p < 0.25f ? 4.0f * p : (p < 0.75f ? 2.0f - 4.0f * p : 4.0f * p - 4.0f);
    case SE_WAVE_SAW: return p < 0.5f ? 2.0f * p : 2.0f * p - 2.0f; // rising, 0 at phase 0
    default: return sin_turns(p);
    }
}

float env_coef(float seconds, float sr, float frames) noexcept {
    // The remaining distance shrinks by 60 dB over `seconds`.
    const double n = std::max(1.0, static_cast<double>(seconds) * static_cast<double>(sr));
    return static_cast<float>(std::exp(std::log(1e-3) * static_cast<double>(frames) / n));
}

} // namespace

se_synth_params default_synth_params() noexcept {
    se_synth_params p{};
    p.osc1_wave = SE_WAVE_SAW;
    p.osc1_fine_cents = -5.0f;
    p.osc1_level = 0.6f;
    p.osc2_wave = SE_WAVE_SQUARE;
    p.osc2_fine_cents = 5.0f;
    p.osc2_level = 0.5f;
    p.filter_mode = SE_FILTER_LOWPASS;
    p.filter_cutoff_hz = 2093.0f;
    p.filter_resonance = 0.1f;
    p.filter_key_track = 1.0f;
    p.filter_env_amount = 0.0f;
    p.amp_attack = 0.005f;
    p.amp_decay = 0.2f;
    p.amp_sustain = 0.6f;
    p.amp_release = 0.25f;
    p.filter_attack = 0.005f;
    p.filter_decay = 0.3f;
    p.filter_sustain = 0.0f;
    p.filter_release = 0.3f;
    p.lfo_wave = SE_WAVE_SINE;
    p.lfo_rate_hz = 5.0f;
    p.polyphony = 16;
    p.unison = 1;
    p.unison_detune_cents = 12.0f;
    p.output = 1.0f;
    return p;
}

const char* validate_synth_params(const se_synth_params& p) noexcept {
    const auto in = [](float v, float lo, float hi) { return std::isfinite(v) && v >= lo && v <= hi; };
    if (p.osc1_wave > SE_WAVE_SINE || p.osc2_wave > SE_WAVE_SINE || p.lfo_wave > SE_WAVE_SINE)
        return "synth waveform must be 0..3";
    if (p.filter_mode > SE_FILTER_BANDPASS) return "synth filter_mode must be 0..2";
    if (p.osc1_octave < -3 || p.osc1_octave > 3 || p.osc2_octave < -3 || p.osc2_octave > 3)
        return "synth octave must be -3..3";
    if (p.osc1_semitones < -12 || p.osc1_semitones > 12 || p.osc2_semitones < -12 || p.osc2_semitones > 12)
        return "synth semitones must be -12..12";
    if (p.polyphony < 1 || p.polyphony > Synth::kMaxNotes) return "synth polyphony must be 1..16";
    if (p.unison < 1 || p.unison > Synth::kMaxUnison) return "synth unison must be 1..4";
    if (!in(p.osc1_shape, 0, 1) || !in(p.osc2_shape, 0, 1)) return "synth osc shape must be 0..1";
    if (!in(p.osc1_fine_cents, -100, 100) || !in(p.osc2_fine_cents, -100, 100))
        return "synth osc fine_cents must be -100..100";
    if (!in(p.osc1_level, 0, 1) || !in(p.osc2_level, 0, 1)) return "synth osc level must be 0..1";
    if (!in(p.cross_fm, 0, 1) || !in(p.sub_level, 0, 1) || !in(p.noise_level, 0, 1))
        return "synth cross_fm / sub_level / noise_level must be 0..1";
    if (!in(p.filter_cutoff_hz, 20, 20000)) return "synth filter_cutoff_hz must be 20..20000";
    if (!in(p.filter_resonance, 0, 1) || !in(p.filter_key_track, 0, 1))
        return "synth filter resonance / key_track must be 0..1";
    if (!in(p.filter_env_amount, -1, 1)) return "synth filter_env_amount must be -1..1";
    if (!in(p.amp_attack, 0.001f, 10) || !in(p.amp_decay, 0.001f, 10) || !in(p.amp_sustain, 0, 1) ||
        !in(p.amp_release, 0.001f, 20) || !in(p.filter_attack, 0.001f, 10) || !in(p.filter_decay, 0.001f, 10) ||
        !in(p.filter_sustain, 0, 1) || !in(p.filter_release, 0.001f, 20))
        return "synth envelope out of range (attack/decay 0.001..10 s, sustain 0..1, release 0.001..20 s)";
    if (!in(p.lfo_rate_hz, 0.01f, 20)) return "synth lfo_rate_hz must be 0.01..20";
    if (!in(p.lfo_to_pitch_cents, 0, 1200) || !in(p.lfo_to_cutoff_oct, 0, 6) || !in(p.lfo_to_amp, 0, 1))
        return "synth lfo depth out of range (pitch 0..1200 cents, cutoff 0..6 oct, amp 0..1)";
    if (!in(p.glide, 0, 2)) return "synth glide must be 0..2 s";
    if (!in(p.unison_detune_cents, 0, 50)) return "synth unison_detune_cents must be 0..50";
    if (!in(p.output, 0, 2)) return "synth output must be 0..2";
    return nullptr;
}

Synth::Synth(double sample_rate) noexcept : sample_rate_(static_cast<float>(sample_rate)) {
    const double sr = sample_rate;
    const double tick = static_cast<double>(kControlFrames);
    fade_step_ = static_cast<float>(1.0 / std::max(1.0, static_cast<double>(kFadeSeconds) * sr));
    smooth_coef_ = static_cast<float>(1.0 - std::exp(-tick / (static_cast<double>(kSmoothSeconds) * sr)));
    lfo_coef_ = static_cast<float>(1.0 - std::exp(-tick / (0.001 * sr))); // ~1 ms de-click
    max_inc_ = 0.45f; // keep oscillators below Nyquist for extreme pitch / low rates
    apply_params(default_synth_params());
}

uint32_t Synth::active_slots() const noexcept { return active_; }

uint32_t Synth::note_voices() const noexcept {
    uint32_t n = 0;
    for (const Voice& v : voices_)
        if (v.stage != Stage::Free && !v.fading) ++n;
    return n;
}

void Synth::apply_params(const se_synth_params& p) noexcept {
    if (std::memcmp(&p, &params_, sizeof p) == 0) return;
    params_ = p;
    ++params_gen_;
    const float sr = sample_rate_;
    const auto tick = static_cast<float>(kControlFrames);
    Targets t;
    t.wave1 = p.osc1_wave;
    t.wave2 = p.osc2_wave;
    t.mode = p.filter_mode;
    t.lfo_wave = p.lfo_wave;
    t.coarse1 = static_cast<float>(p.osc1_octave * 12 + p.osc1_semitones);
    t.coarse2 = static_cast<float>(p.osc2_octave * 12 + p.osc2_semitones);
    t.unison = std::clamp<uint32_t>(p.unison, 1, kMaxUnison);
    t.glide = p.glide;
    float* s = t.smoothed;
    s[kShape1] = p.osc1_shape;
    s[kShape2] = p.osc2_shape;
    s[kFine1] = p.osc1_fine_cents;
    s[kFine2] = p.osc2_fine_cents;
    s[kLevel1] = p.osc1_level;
    s[kLevel2] = p.osc2_level;
    s[kCrossFm] = p.cross_fm;
    s[kSub] = p.sub_level;
    s[kNoise] = p.noise_level;
    s[kCutoff] = std::log2(std::max(p.filter_cutoff_hz, 1.0f));
    s[kResonance] = p.filter_resonance;
    s[kKeyTrack] = p.filter_key_track;
    s[kEnvAmount] = p.filter_env_amount;
    s[kLfoPitch] = p.lfo_to_pitch_cents;
    s[kLfoCutoff] = p.lfo_to_cutoff_oct;
    s[kLfoAmp] = p.lfo_to_amp;
    s[kDetune] = p.unison_detune_cents;
    s[kOutput] = p.output;
    const auto env = [&](float a, float d, float sus, float r, float frames) {
        EnvCoefs e;
        e.attack_inc = frames / std::max(1.0f, a * sr);
        e.decay_coef = env_coef(d, sr, frames);
        e.sustain = sus;
        e.release_coef = env_coef(r, sr, frames);
        return e;
    };
    t.amp_env = env(p.amp_attack, p.amp_decay, p.amp_sustain, p.amp_release, 1.0f);
    t.filter_env = env(p.filter_attack, p.filter_decay, p.filter_sustain, p.filter_release, 1.0f);
    t.filter_env_tick = env(p.filter_attack, p.filter_decay, p.filter_sustain, p.filter_release, tick);
    t.lfo_inc_tick = p.lfo_rate_hz * tick / sr;
    tg_ = t;

    // A lower polyphony limit releases the oldest held voices (normal
    // release, no cut).
    poly_ = std::clamp<uint32_t>(p.polyphony, 1, kMaxNotes);
    uint32_t held = 0;
    for (const Voice& v : voices_)
        if (v.stage != Stage::Free && !v.fading && v.held) ++held;
    while (held > poly_) {
        Voice* oldest = nullptr;
        for (Voice& v : voices_)
            if (v.stage != Stage::Free && !v.fading && v.held && (oldest == nullptr || v.order < oldest->order))
                oldest = &v;
        if (oldest == nullptr) break;
        release(*oldest);
        --held;
    }
}

void Synth::release(Voice& v) noexcept {
    v.held = false;
    if (v.stage != Stage::Free) {
        v.stage = Stage::Release;
        v.fstage = Stage::Release;
    }
}

void Synth::fade_out(Voice& v) noexcept {
    v.held = false;
    v.fading = true;
}

void Synth::fade_all() noexcept {
    have_last_pitch_ = false; // no glide across transport jumps
    if (active_ == 0) return;
    for (Voice& v : voices_)
        if (v.stage != Stage::Free) fade_out(v);
}

void Synth::set_pitch_target(Voice& v, float pitch) noexcept {
    v.pitch_target = pitch;
    if (tg_.glide > 0.0f) {
        const float ticks = std::max(1.0f, tg_.glide * sample_rate_ / static_cast<float>(kControlFrames));
        v.glide_step = std::fabs(pitch - v.pitch_cur) / ticks;
    } else {
        v.pitch_cur = pitch;
        v.glide_step = 0.0f;
    }
}

void Synth::start_voice(Voice& v, const NoteEvent& n) noexcept {
    const auto pitch = static_cast<float>(n.pitch);
    const float from = tg_.glide > 0.0f && have_last_pitch_ ? last_pitch_ : pitch;
    v = Voice{};
    v.stage = Stage::Attack;
    v.fstage = Stage::Attack;
    v.held = true;
    v.pitch = n.pitch;
    v.note_start = n.start;
    v.note_end = n.end;
    v.order = next_order_++;
    v.amp = kVoiceGain * static_cast<float>(n.velocity) / 127.0f;
    v.pitch_cur = from;
    set_pitch_target(v, pitch);
    // osc2 starts half a cycle later: with equal pitch its square's falling
    // edge meets the saw's rising edge, so the default patch's edges
    // partially cancel instead of stacking.
    for (uint32_t k = 0; k < kMaxUnison; ++k) {
        v.ph1[k] = kStartPhase[k];
        v.ph2[k] = wrap01(kStartPhase[k] + 0.5f);
    }
    // Deterministic noise seed per note (export == live playback).
    const auto h = static_cast<uint64_t>(n.start) * 0x9E3779B97F4A7C15ull + n.pitch * 0x85EBCA6Bu;
    v.rng = static_cast<uint32_t>(h ^ (h >> 32)) | 1u;
    std::memcpy(v.sm, tg_.smoothed, sizeof v.sm);
    v.unison = tg_.unison;
    control_tick(v, true);
}

// Note-on. Polyphonic: a new voice (stealing the oldest note voice when at
// the limit). Monophonic (polyphony 1), last-note priority:
//  * a held voice is taken over legato: no envelope retrigger, the pitch
//    glides (or jumps when glide = 0) to the new note, velocity unchanged;
//  * a releasing voice is retriggered: both envelopes restart their attack
//    from their current level (no click), pitch glides from where it is;
//  * otherwise a fresh voice starts.
// With glide > 0, every fresh voice (mono or poly) glides from the pitch of
// the previous note-on, unless the transport jumped in between.
void Synth::note_on(const NoteEvent& n) noexcept {
    const auto pitch = static_cast<float>(n.pitch);
    if (poly_ == 1) {
        Voice* cur = nullptr;
        for (Voice& v : voices_)
            if (v.stage != Stage::Free && !v.fading && (cur == nullptr || v.order > cur->order)) cur = &v;
        if (cur != nullptr) {
            if (!cur->held) {
                cur->stage = Stage::Attack;
                cur->fstage = Stage::Attack;
                cur->amp = kVoiceGain * static_cast<float>(n.velocity) / 127.0f;
            }
            cur->held = true;
            cur->pitch = n.pitch;
            cur->note_start = n.start;
            cur->note_end = n.end;
            cur->order = next_order_++;
            set_pitch_target(*cur, pitch);
            cur->force_tick = true; // new pitch / gain from this sample on
            last_pitch_ = pitch;
            have_last_pitch_ = true;
            return;
        }
    } else if (note_voices() >= poly_) {
        Voice* oldest = nullptr;
        for (Voice& v : voices_)
            if (v.stage != Stage::Free && !v.fading && (oldest == nullptr || v.order < oldest->order)) oldest = &v;
        if (oldest != nullptr) fade_out(*oldest);
    }
    Voice* slot = nullptr;
    for (Voice& v : voices_)
        if (v.stage == Stage::Free) {
            slot = &v;
            break;
        }
    if (slot == nullptr) {
        // Every slot busy: replace the quietest fading voice.
        for (Voice& v : voices_)
            if (v.fading && (slot == nullptr || v.level * v.fade < slot->level * slot->fade)) slot = &v;
        if (slot == nullptr) slot = &voices_[0]; // unreachable (kSlots > kMaxNotes)
    } else {
        ++active_;
    }
    start_voice(*slot, n);
    last_pitch_ = pitch;
    have_last_pitch_ = true;
}

// Voices are matched to the new note list by (start, pitch); a held voice
// keeps sounding with the note's (possibly changed) end, a held voice whose
// note no longer exists is released.
void Synth::rematch(const NoteEvent* notes, uint32_t note_count) noexcept {
    if (active_ == 0) return;
    const NoteEvent* const end = notes + note_count;
    for (Voice& v : voices_) {
        if (v.stage == Stage::Free || !v.held) continue;
        const NoteEvent* it = std::lower_bound(
            notes, end, v.note_start, [](const NoteEvent& n, int64_t s) { return n.start < s; });
        bool found = false;
        for (; it != end && it->start == v.note_start; ++it)
            if (it->pitch == v.pitch) {
                v.note_end = it->end;
                found = true;
                break;
            }
        if (!found) release(v);
    }
}

void Synth::update_pitch(Voice& v, float pitch) noexcept {
    const Targets& t = tg_;
    const float* s = v.sm;
    const float sr = sample_rate_;
    const float base1 = 440.0f / sr * std::exp2((pitch + t.coarse1 + s[kFine1] * 0.01f - 69.0f) * (1.0f / 12.0f));
    const float base2 = 440.0f / sr * std::exp2((pitch + t.coarse2 + s[kFine2] * 0.01f - 69.0f) * (1.0f / 12.0f));
    const uint32_t un = v.unison;
    for (uint32_t k = 0; k < un; ++k) {
        const float r = v.detune_ratio[k];
        v.inc1[k] = std::min(base1 * r, max_inc_);
        v.inc2[k] = std::min(base2 * r, max_inc_);
    }
    v.inc_sub = std::min(0.5f * base1, max_inc_);
}

void Synth::update_filter(Voice& v, float log2fc) noexcept {
    const float sr = sample_rate_;
    const float fc = std::clamp(std::exp2(log2fc), 10.0f, 0.45f * sr);
    const float g = std::tan(kPi * fc / sr);
    if (v.sm[kResonance] != v.last_res) {
        v.last_res = v.sm[kResonance];
        const float q = kButterworthQ *
                        std::exp2(std::clamp(v.last_res, 0.0f, 1.0f) * std::log2(kMaxQ / kButterworthQ));
        v.k = 1.0f / q;
    }
    const float k = v.k;
    const float a1 = 1.0f / (1.0f + g * (g + k));
    v.a1 = a1;
    v.a2 = g * a1;
    v.a3 = g * g * a1;
}

// Control-rate update. Non-init ticks first advance the tick-rate state
// (smoothing, LFO phase, filter envelope, glide) by one tick, then every
// tick derives the coefficients used for the next kControlFrames samples.
// Oscillator increments and filter coefficients (the transcendental math)
// are only recomputed when their inputs changed.
void Synth::control_tick(Voice& v, bool init) noexcept {
    const Targets& t = tg_;
    bool dirty = init || v.params_gen != params_gen_;
    v.params_gen = params_gen_;
    if (!init) {
        if (v.smoothing || dirty) {
            bool moving = false;
            for (uint32_t i = 0; i < kNumSmoothed; ++i) {
                const float target = t.smoothed[i];
                float x = v.sm[i] + smooth_coef_ * (target - v.sm[i]);
                if (std::fabs(target - x) <= 1e-6f * std::max(1.0f, std::fabs(target)))
                    x = target; // converged: snap, so the caches can kick in
                else
                    moving = true;
                v.sm[i] = x;
            }
            v.smoothing = moving;
            dirty = true;
        }
        v.lfo_phase += t.lfo_inc_tick;
        if (v.lfo_phase >= 1.0f) v.lfo_phase -= std::floor(v.lfo_phase);
        const EnvCoefs& fe = t.filter_env_tick;
        switch (v.fstage) {
        case Stage::Attack:
            v.flevel += fe.attack_inc;
            if (v.flevel >= 1.0f) {
                v.flevel = 1.0f;
                v.fstage = Stage::Decay;
            }
            break;
        case Stage::Decay: v.flevel = fe.sustain + (v.flevel - fe.sustain) * fe.decay_coef; break;
        case Stage::Release: v.flevel *= fe.release_coef; break;
        case Stage::Free: break;
        }
        if (v.pitch_cur != v.pitch_target) {
            if (v.glide_step <= 0.0f || std::fabs(v.pitch_target - v.pitch_cur) <= v.glide_step)
                v.pitch_cur = v.pitch_target;
            else
                v.pitch_cur += v.pitch_cur < v.pitch_target ? v.glide_step : -v.glide_step;
        }
    }
    const float raw = lfo_value(t.lfo_wave, v.lfo_phase);
    v.lfo_out = init ? raw : v.lfo_out + lfo_coef_ * (raw - v.lfo_out);
    const float lfo = v.lfo_out;
    const float* s = v.sm;

    if (dirty) {
        v.unison = t.unison;
        v.mode = t.mode;
        const auto shape = [](uint8_t wave, float sh) {
            OscShape o;
            o.wave = wave;
            sh = std::clamp(sh, 0.0f, 1.0f);
            if (wave == SE_WAVE_SQUARE) {
                o.pw = 0.5f - 0.45f * sh;
                o.dc = 2.0f * o.pw - 1.0f;
            } else if (wave == SE_WAVE_SINE) {
                o.fold = 5.0f * sh;
                o.fold_norm =
                    o.fold < 1.0f ? (o.fold > 1e-4f ? 1.0f / sin_turns_any(0.25f * o.fold) : 1.0f) : 1.0f;
                if (o.fold <= 1e-4f) o.fold = 0.0f;
            } else {
                o.morph = sh;
            }
            return o;
        };
        v.sh1 = shape(t.wave1, s[kShape1]);
        v.sh2 = shape(t.wave2, s[kShape2]);
        // Unison stacks spread evenly over +-detune / 2.
        for (uint32_t k = 0; k < v.unison; ++k) {
            float r = 1.0f;
            if (v.unison > 1) {
                const float cents =
                    s[kDetune] * (static_cast<float>(k) / static_cast<float>(v.unison - 1) - 0.5f);
                r = std::exp2(cents * (1.0f / 1200.0f));
            }
            v.detune_ratio[k] = r;
        }
    }

    const float pitch = v.pitch_cur + lfo * s[kLfoPitch] * 0.01f;
    if (dirty || pitch != v.last_pitch) {
        update_pitch(v, pitch);
        v.last_pitch = pitch;
    }
    const float log2fc = s[kCutoff] + s[kKeyTrack] * (v.pitch_cur - 60.0f) * (1.0f / 12.0f) +
                         6.0f * s[kEnvAmount] * v.flevel + lfo * s[kLfoCutoff];
    if (dirty || log2fc != v.last_log2fc) {
        update_filter(v, log2fc);
        v.last_log2fc = log2fc;
    }

    // Gains, ramped linearly from where they are to the new values over the
    // next tick.
    const uint32_t un = v.unison;
    const float u = un == 1 ? 1.0f : 1.0f / std::sqrt(static_cast<float>(un));
    const float trem = 1.0f - s[kLfoAmp] * 0.5f * (1.0f - lfo);
    float target[kNumGains];
    target[kGain1] = s[kLevel1] * u;
    target[kGain2] = s[kLevel2] * u;
    target[kGainSub] = s[kSub];
    target[kGainNoise] = s[kNoise];
    target[kGainFm] = s[kCrossFm] * kFmIndex;
    target[kGainOut] = v.amp * s[kOutput] * trem;
    constexpr float inv = 1.0f / static_cast<float>(kControlFrames);
    const auto pos = static_cast<float>(v.tick_pos);
    for (uint32_t i = 0; i < kNumGains; ++i) {
        if (init) {
            v.g0[i] = target[i];
            v.gd[i] = 0.0f;
        } else {
            const float cur = v.g0[i] + v.gd[i] * pos;
            v.g0[i] = cur;
            v.gd[i] = (target[i] - cur) * inv;
        }
    }
    v.tick_pos = 0;
    v.force_tick = false;
}

namespace {

// Oscillator block generators: the waveform switch is outside the sample
// loop. `dt` holds the per-sample phase increment (constant without FM).
template <class Wave>
inline void gen(Wave w, float& phase, const float* dt, uint32_t n, float* o) noexcept {
    float p = phase;
    for (uint32_t i = 0; i < n; ++i) {
        o[i] = w(p, dt[i]);
        p += dt[i];
        if (p >= 1.0f) p -= 1.0f;
    }
    phase = p;
}

template <class Shape>
inline void gen_osc(const Shape& s, float& phase, const float* dt, uint32_t n, float* o) noexcept {
    switch (s.wave) {
    case SE_WAVE_SAW:
        if (s.morph > 0.0f) {
            const float m = s.morph;
            gen([m](float t, float d) { const float y = saw(t, d); return y + m * (tri(t, d) - y); }, phase, dt, n, o);
        } else {
            gen([](float t, float d) { return saw(t, d); }, phase, dt, n, o);
        }
        break;
    case SE_WAVE_TRIANGLE:
        if (s.morph > 0.0f) {
            const float m = s.morph;
            gen([m](float t, float d) { const float y = tri(t, d); return y + m * (saw(t, d) - y); }, phase, dt, n, o);
        } else {
            gen([](float t, float d) { return tri(t, d); }, phase, dt, n, o);
        }
        break;
    case SE_WAVE_SQUARE: {
        const float pw = s.pw, dc = s.dc;
        gen(
            [pw, dc](float t, float d) {
                float t2 = t - pw;
                if (t2 < 0.0f) t2 += 1.0f;
                return (t < pw ? 1.0f : -1.0f) + blep(t, d) - blep(t2, d) - dc;
            },
            phase, dt, n, o);
        break;
    }
    default:
        if (s.fold > 0.0f) {
            const float f = 0.25f * s.fold, norm = s.fold_norm;
            gen([f, norm](float t, float) { return sin_turns_any(f * sin_turns(t)) * norm; }, phase, dt, n, o);
        } else {
            gen([](float t, float) { return sin_turns(t); }, phase, dt, n, o);
        }
        break;
    }
}

} // namespace

// Renders n <= kControlFrames - tick_pos samples (never crosses a tick).
bool Synth::run_samples(Voice& v, float* out, uint32_t n) noexcept {
    float x[kControlFrames], o1[kControlFrames], o2[kControlFrames], dt[kControlFrames];
    const auto off = static_cast<float>(v.tick_pos);
    const float* g0 = v.g0;
    const float* gd = v.gd;
    const auto ramp = [&](uint32_t g, uint32_t i) { return g0[g] + gd[g] * (off + static_cast<float>(i)); };
    const bool sub_on = g0[kGainSub] != 0.0f || gd[kGainSub] != 0.0f;
    const bool noise_on = g0[kGainNoise] != 0.0f || gd[kGainNoise] != 0.0f;
    const bool fm_on = g0[kGainFm] != 0.0f || gd[kGainFm] != 0.0f;
    const bool osc2_on = fm_on || g0[kGain2] != 0.0f || gd[kGain2] != 0.0f;
    const bool osc1_on = g0[kGain1] != 0.0f || gd[kGain1] != 0.0f;

    for (uint32_t i = 0; i < n; ++i) x[i] = 0.0f;
    for (uint32_t k = 0; k < v.unison; ++k) {
        if (osc2_on) {
            for (uint32_t i = 0; i < n; ++i) dt[i] = v.inc2[k];
            gen_osc(v.sh2, v.ph2[k], dt, n, o2);
        }
        if (osc1_on) {
            const float inc = v.inc1[k];
            if (fm_on) {
                const float lo = 0.02f * inc, hi = max_inc_;
                for (uint32_t i = 0; i < n; ++i) dt[i] = std::clamp(inc * (1.0f + ramp(kGainFm, i) * o2[i]), lo, hi);
            } else {
                for (uint32_t i = 0; i < n; ++i) dt[i] = inc;
            }
            gen_osc(v.sh1, v.ph1[k], dt, n, o1);
            for (uint32_t i = 0; i < n; ++i) x[i] += ramp(kGain1, i) * o1[i];
        }
        // (A silent oscillator's phase is frozen, not advanced.)
        if (osc2_on && (g0[kGain2] != 0.0f || gd[kGain2] != 0.0f))
            for (uint32_t i = 0; i < n; ++i) x[i] += ramp(kGain2, i) * o2[i];
    }
    if (sub_on) {
        float p = v.ph_sub;
        const float inc = v.inc_sub;
        for (uint32_t i = 0; i < n; ++i) {
            x[i] += ramp(kGainSub, i) * sin_turns(p);
            p += inc;
            if (p >= 1.0f) p -= 1.0f;
        }
        v.ph_sub = p;
    }
    if (noise_on) {
        uint32_t r = v.rng;
        for (uint32_t i = 0; i < n; ++i) x[i] += ramp(kGainNoise, i) * white(r);
        v.rng = r;
    }

    // Filter, amp envelope, output.
    float level = v.level, fade = v.fade, ic1 = v.ic1, ic2 = v.ic2;
    const float a1 = v.a1, a2 = v.a2, a3 = v.a3, kk = v.k;
    const uint8_t mode = v.mode;
    const EnvCoefs& env = tg_.amp_env;
    Stage stage = v.stage;
    const bool fading = v.fading;
    for (uint32_t i = 0; i < n; ++i) {
        const float in = x[i];
        const float v3 = in - ic2;
        const float v1 = a1 * ic1 + a2 * v3;
        const float v2 = ic2 + a2 * ic1 + a3 * v3;
        ic1 = 2.0f * v1 - ic1;
        ic2 = 2.0f * v2 - ic2;
        const float y = mode == SE_FILTER_LOWPASS ? v2 : (mode == SE_FILTER_HIGHPASS ? in - kk * v1 - v2 : kk * v1);

        switch (stage) {
        case Stage::Attack:
            level += env.attack_inc;
            if (level >= 1.0f) {
                level = 1.0f;
                stage = Stage::Decay;
            }
            break;
        case Stage::Decay: level = env.sustain + (level - env.sustain) * env.decay_coef; break;
        case Stage::Release: level *= env.release_coef; break;
        case Stage::Free: break;
        }

        out[i] += y * level * ramp(kGainOut, i) * fade;

        if (fading) {
            fade -= fade_step_;
            if (fade <= 0.0f) return true;
        }
        if (level < kOffLevel && (stage == Stage::Release || (stage == Stage::Decay && env.sustain == 0.0f)))
            return true;
    }
    v.level = level;
    v.fade = fade;
    v.ic1 = ic1;
    v.ic2 = ic2;
    v.stage = stage;
    return false;
}

void Synth::render_voice(Voice& v, float* out, uint32_t frames) noexcept {
    uint32_t i = 0;
    while (i < frames) {
        if (v.tick_pos >= kControlFrames || v.force_tick) control_tick(v, false);
        const uint32_t n = std::min(frames - i, kControlFrames - v.tick_pos);
        if (run_samples(v, out + i, n)) {
            v.stage = Stage::Free;
            v.fstage = Stage::Free;
            v.fading = false;
            v.held = false;
            --active_;
            return;
        }
        v.tick_pos += n;
        i += n;
    }
}

void Synth::render_voices(float* out, uint32_t frames) noexcept {
    if (frames == 0) return;
    for (Voice& v : voices_)
        if (v.stage != Stage::Free) render_voice(v, out, frames);
}

bool Synth::render(const NoteEvent* notes, uint32_t note_count, const se_synth_params* params, uint64_t graph_seq,
                   uint64_t transport_gen, int64_t position, uint32_t frames, bool playing, float* out) noexcept {
    // Transport jump (seek/stop/pause/loop wrap) or timeline discontinuity:
    // fade out everything that sounds. Notes in progress at the new position
    // are not retriggered.
    if (seen_gen_ != transport_gen || (playing && have_pos_ && position != expected_pos_)) fade_all();
    seen_gen_ = transport_gen;
    if (seen_seq_ != graph_seq) {
        if (params != nullptr)
            apply_params(*params);
        else
            apply_params(default_synth_params());
        rematch(notes, note_count);
        seen_seq_ = graph_seq;
    }

    if (!playing) {
        if (active_ == 0) return false;
        std::memset(out, 0, sizeof(float) * frames);
        render_voices(out, frames);
        return true;
    }

    have_pos_ = true;
    expected_pos_ = position + frames;
    const int64_t end = position + frames;
    const NoteEvent* const notes_end = notes + note_count;
    const NoteEvent* on =
        std::lower_bound(notes, notes_end, position, [](const NoteEvent& n, int64_t s) { return n.start < s; });
    if (active_ == 0 && (on == notes_end || on->start >= end)) return false;

    std::memset(out, 0, sizeof(float) * frames);
    // Split the segment at every note-on / note-off frame.
    for (int64_t cur = position; cur < end;) {
        int64_t next = end;
        for (Voice& v : voices_) {
            if (v.stage == Stage::Free || !v.held) continue;
            if (v.note_end <= cur)
                release(v);
            else
                next = std::min(next, v.note_end);
        }
        for (; on != notes_end && on->start == cur; ++on) {
            note_on(*on);
            next = std::min(next, on->end);
        }
        if (on != notes_end) next = std::min(next, on->start);
        render_voices(out + (cur - position), static_cast<uint32_t>(next - cur));
        cur = next;
    }
    return true;
}

} // namespace saine
