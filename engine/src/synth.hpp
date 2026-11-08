// Built-in polyphonic instrument ("Saine synth"): one Synth per instrument
// track, driven by se_synth_params (see saine_engine.h and README.md). An
// original subtractive design, it does not model any commercial instrument:
//
//   voice = up to 4 unison stacks of (osc1 + osc2), osc2 -> osc1 linear FM,
//           + sine sub one octave below osc1, + white noise
//         -> multimode TPT state-variable filter (LP / HP / BP, 12 dB/oct)
//            cutoff = cutoff_hz * 2^(key_track * (pitch - 60) / 12
//                                    + 6 * env_amount * filter_env
//                                    + lfo * lfo_to_cutoff_oct)
//         -> amp ADSR x velocity x tremolo x output x kVoiceGain
//
// Oscillators: polyBLEP saw / pulse, polyBLAMP triangle, polynomial sine.
// `shape` is pulse width (50% -> 5%) for square, a saw <-> triangle morph for
// saw and triangle, and a sine wavefolder for sine.
//
// Control rate: every voice recomputes pitch, filter coefficients, LFO,
// filter envelope, glide and the smoothed parameters every kControlFrames
// samples of its own lifetime (a per-voice grid starting at its note-on), so
// the output does not depend on how the timeline is split into blocks.
// Gains are ramped linearly per sample between control ticks.
//
// Smoothing: continuous parameters arriving with a republish are approached
// per voice with a one-pole (time constant kSmoothSeconds) at control rate;
// a voice that starts uses the target values directly. Discrete parameters
// (waveforms, filter mode, octave/semitone, unison count, LFO wave) switch
// at the next control tick.
//
// Real-time rules: every voice is preallocated inside the object; render()
// does no allocation, locking or I/O. A Synth is mutated only by the thread
// that renders with it (the RT owner for live playback, or an export job for
// its private copy).
//
// Polyphony: `polyphony` (1..16) note voices. A note beyond the limit steals
// the oldest note voice (note-on order), which fades out over kFadeSeconds in
// its slot while the new note takes a free slot (kSlots = 16 + 8). With
// polyphony 1 the synth is monophonic and legato (see note_on()).
//
// Event model (see render()): note-on at a note's `start` frame, note-off at
// its `end` frame, both sample-accurate. Voices remember (start, pitch, end)
// of their note; when the graph is republished they are re-matched by
// (start, pitch). A change of `transport_gen` or a timeline discontinuity
// fades out every sounding voice; notes in progress at the new position are
// not retriggered.
#pragma once

#include "saine_engine.h"

#include <cstdint>

namespace saine {

struct NoteEvent {
    int64_t start = 0; // note-on frame
    int64_t end = 0;   // note-off frame (> start)
    uint8_t pitch = 0;
    uint8_t velocity = 0;
};

// Defaults (= saine_core::instrument::SynthParams::default()).
se_synth_params default_synth_params() noexcept;
// nullptr if valid, else a static description of the first problem.
const char* validate_synth_params(const se_synth_params& p) noexcept;

class Synth {
public:
    static constexpr uint32_t kMaxNotes = 16;
    static constexpr uint32_t kSlots = kMaxNotes + 8;
    static constexpr uint32_t kMaxUnison = 4;
    static constexpr uint32_t kControlFrames = 16;
    static constexpr float kFadeSeconds = 0.005f;    // steal / seek / stop fade
    static constexpr float kSmoothSeconds = 0.010f;  // parameter smoothing time constant
    static constexpr float kVoiceGain = 0.25f;
    static constexpr float kMaxQ = 25.0f;            // resonance 1
    static constexpr float kFmIndex = 0.9f;          // cross_fm 1: +-90% frequency deviation

    explicit Synth(double sample_rate) noexcept;

    // Renders timeline frames [position, position + frames) into `out`
    // (mono). `notes` are this track's notes sorted by start; `params` the
    // track's synth parameters (nullptr = defaults), compared with the
    // current ones on every call (automation changes them per segment). When `playing` is false no events are processed
    // (only tails ring out). Returns false (and leaves `out` untouched) if
    // the synth is silent for the whole segment; otherwise overwrites
    // out[0..frames).
    bool render(const NoteEvent* notes, uint32_t note_count, const se_synth_params* params, uint64_t graph_seq,
                uint64_t transport_gen, int64_t position, uint32_t frames, bool playing, float* out) noexcept;

    // Introspection (tests): occupied slots, voices that count against the
    // polyphony limit (held or releasing, not fading), and the limit.
    uint32_t active_slots() const noexcept;
    uint32_t note_voices() const noexcept;
    uint32_t polyphony() const noexcept { return poly_; }

private:
    enum class Stage : uint8_t { Free, Attack, Decay, Release };

    // Parameters approached per voice by the one-pole smoother.
    enum Smoothed : uint32_t {
        kShape1, kShape2, kFine1, kFine2, kLevel1, kLevel2, kCrossFm, kSub, kNoise,
        kCutoff,   // log2(Hz)
        kResonance, kKeyTrack, kEnvAmount, kLfoPitch, kLfoCutoff, kLfoAmp, kDetune, kOutput,
        kNumSmoothed
    };

    struct EnvCoefs {
        float attack_inc = 0.0f;  // linear attack step (per sample)
        float decay_coef = 0.0f;  // remaining distance multiplier (per sample)
        float sustain = 0.0f;
        float release_coef = 0.0f;
    };

    // Everything derived from se_synth_params (recomputed when it changes).
    struct Targets {
        uint8_t wave1 = 0, wave2 = 0, mode = 0, lfo_wave = 0;
        float coarse1 = 0.0f, coarse2 = 0.0f; // octave * 12 + semitones
        uint32_t unison = 1;
        float glide = 0.0f; // seconds
        float smoothed[kNumSmoothed] = {};
        EnvCoefs amp_env, filter_env;    // per sample
        EnvCoefs filter_env_tick;        // per control tick
        float lfo_inc_tick = 0.0f;       // LFO phase increment per control tick
    };

    enum Gain : uint32_t { kGain1, kGain2, kGainSub, kGainNoise, kGainFm, kGainOut, kNumGains };

    struct OscShape {
        uint8_t wave = 0;
        float morph = 0.0f;     // saw <-> triangle
        float pw = 0.5f;        // pulse width
        float dc = 0.0f;        // pulse DC offset removed
        float fold = 0.0f;      // sine fold gain (0 = pure sine)
        float fold_norm = 1.0f;
    };

    struct Voice {
        Stage stage = Stage::Free;
        Stage fstage = Stage::Free; // filter envelope
        bool held = false;   // note-off not yet applied
        bool fading = false; // stolen / transport fade: not a note voice any more
        uint8_t pitch = 0;
        int64_t note_start = 0;
        int64_t note_end = 0;
        uint64_t order = 0; // note-on sequence number (steal oldest)
        float amp = 0.0f;   // velocity x kVoiceGain
        float level = 0.0f; // amp envelope
        float flevel = 0.0f; // filter envelope
        float fade = 1.0f;  // fade-out multiplier (1 unless fading)
        // Pitch in semitones (MIDI note numbers), glide per control tick.
        float pitch_cur = 0.0f, pitch_target = 0.0f, glide_step = 0.0f;
        // Oscillators (per unison stack) and sub.
        float ph1[kMaxUnison] = {}, ph2[kMaxUnison] = {};
        float inc1[kMaxUnison] = {}, inc2[kMaxUnison] = {};
        float ph_sub = 0.0f, inc_sub = 0.0f;
        uint32_t unison = 1;
        OscShape sh1, sh2;
        uint32_t rng = 1;
        // SVF state and coefficients.
        float ic1 = 0.0f, ic2 = 0.0f;
        float a1 = 0.0f, a2 = 0.0f, a3 = 0.0f, k = 0.0f;
        uint8_t mode = 0;
        // LFO (per voice, phase 0 at note-on).
        float lfo_phase = 0.0f, lfo_out = 0.0f;
        // Control grid: samples rendered since the last control tick
        // (kControlFrames = tick due); force_tick runs one at the next sample.
        uint32_t tick_pos = 0;
        bool force_tick = false;
        bool smoothing = false;  // smoothed parameters not yet at their targets
        uint64_t params_gen = 0; // Synth::params_gen_ seen at the last tick
        float sm[kNumSmoothed] = {};
        // Caches: skip transcendental math while the inputs do not change.
        float last_pitch = -1e9f, last_log2fc = -1e9f, last_res = -1.0f;
        float detune_ratio[kMaxUnison] = {1.0f, 1.0f, 1.0f, 1.0f}; // per unison stack
        // Gains ramped per sample across a tick: value at tick offset o is
        // g0[i] + gd[i] * o (so it does not depend on how blocks are split).
        float g0[kNumGains] = {}, gd[kNumGains] = {};
    };

    void apply_params(const se_synth_params& p) noexcept;
    void note_on(const NoteEvent& n) noexcept;
    void start_voice(Voice& v, const NoteEvent& n) noexcept;
    void set_pitch_target(Voice& v, float pitch) noexcept;
    void release(Voice& v) noexcept;
    void fade_out(Voice& v) noexcept;
    void fade_all() noexcept;
    void rematch(const NoteEvent* notes, uint32_t note_count) noexcept;
    void control_tick(Voice& v, bool init) noexcept;
    void update_pitch(Voice& v, float pitch) noexcept;
    void update_filter(Voice& v, float log2fc) noexcept;
    void render_voices(float* out, uint32_t frames) noexcept;
    void render_voice(Voice& v, float* out, uint32_t frames) noexcept;
    bool run_samples(Voice& v, float* out, uint32_t n) noexcept; // true: voice finished

    float sample_rate_;
    float fade_step_;
    float smooth_coef_; // per control tick
    float lfo_coef_;    // LFO output de-click one-pole, per control tick
    float max_inc_;

    se_synth_params params_{};
    Targets tg_;
    uint32_t poly_ = kMaxNotes;
    uint64_t params_gen_ = 1; // bumped whenever the parameters change

    Voice voices_[kSlots];
    uint32_t active_ = 0; // occupied slots (fast silence check)
    uint64_t next_order_ = 1;
    uint64_t seen_seq_ = ~uint64_t{0};
    uint64_t seen_gen_ = 0;
    int64_t expected_pos_ = 0;
    bool have_pos_ = false;
    float last_pitch_ = 0.0f; // glide source for the next note
    bool have_last_pitch_ = false;
};

} // namespace saine
