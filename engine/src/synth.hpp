// Built-in polyphonic instrument ("Saine basic synth"): one Synth per
// instrument track. Original, deliberately plain design:
//
//   voice = 2 oscillators (each an equal blend of a polyBLEP saw and a
//           polyBLEP square), detuned +/- kDetuneCents, summed
//         -> 12 dB/oct state-variable low-pass (TPT form, Q = 0.707),
//            cutoff key-tracked at 8 x the note frequency (500 Hz..12 kHz,
//            below 0.45 x sample rate)
//         -> ADSR (linear 5 ms attack; exponential decay over 200 ms to
//            sustain 0.6; exponential release falling 60 dB in 250 ms)
//         -> velocity (linear, v / 127) x kVoiceGain
//
// Real-time rules: every voice is preallocated inside the object; render()
// does no allocation, locking or I/O. A Synth is mutated only by the thread
// that renders with it (the RT owner for live playback, or an export job
// for its private copy).
//
// Polyphony: kMaxNotes note voices. When a 17th note starts, the oldest
// note voice (by note-on order) is stolen: it moves to a fade-out state
// (kFadeSeconds linear fade) in its slot, and the new note takes a free
// slot. There are kSlots = kMaxNotes + 8 slots so fading voices don't
// block new notes; if all slots are busy, the quietest fading voice is
// replaced (the only case where a step can occur, and only at a level that
// has already been fading).
//
// Event model (see render()): note-on at a note's `start` frame, note-off
// at its `end` frame, both sample-accurate (the segment is split at every
// event). Voices remember (start, pitch, end) of their note; when the graph
// is republished they are re-matched by (start, pitch) so a sounding note
// keeps playing with its new end, and a voice whose note disappeared is
// released. A change of `transport_gen` (seek / stop / pause / loop wrap)
// or a timeline discontinuity fades out every sounding voice (kFadeSeconds);
// notes in progress at the new position are not retriggered.
#pragma once

#include <cstdint>

namespace saine {

struct NoteEvent {
    int64_t start = 0; // note-on frame
    int64_t end = 0;   // note-off frame (> start)
    uint8_t pitch = 0;
    uint8_t velocity = 0;
};

class Synth {
public:
    static constexpr uint32_t kMaxNotes = 16;
    static constexpr uint32_t kSlots = kMaxNotes + 8;
    static constexpr float kAttackSeconds = 0.005f;
    static constexpr float kDecaySeconds = 0.200f;
    static constexpr float kSustain = 0.6f;
    static constexpr float kReleaseSeconds = 0.250f; // time to fall 60 dB
    static constexpr float kFadeSeconds = 0.005f;    // steal / seek / stop fade
    static constexpr float kDetuneCents = 5.0f;
    static constexpr float kVoiceGain = 0.25f;

    explicit Synth(double sample_rate) noexcept;

    // Renders timeline frames [position, position + frames) into `out`
    // (mono). `notes` are this track's notes sorted by start. When
    // `playing` is false no events are processed (only tails ring out).
    // Returns false (and leaves `out` untouched) if the synth is silent for
    // the whole segment; otherwise overwrites out[0..frames).
    bool render(const NoteEvent* notes, uint32_t note_count, uint64_t graph_seq, uint64_t transport_gen,
                int64_t position, uint32_t frames, bool playing, float* out) noexcept;

    // Introspection (tests): occupied slots, and voices that count against
    // the polyphony limit (held or releasing, not fading).
    uint32_t active_slots() const noexcept;
    uint32_t note_voices() const noexcept;

private:
    enum class Stage : uint8_t { Free, Attack, Decay, Release };

    struct Voice {
        Stage stage = Stage::Free;
        bool held = false;   // note-off not yet applied
        bool fading = false; // stolen / transport fade: not a note voice any more
        uint8_t pitch = 0;
        int64_t note_start = 0;
        int64_t note_end = 0;
        uint64_t order = 0; // note-on sequence number (steal oldest)
        float phase1 = 0.0f, phase2 = 0.0f;
        float inc1 = 0.0f, inc2 = 0.0f;
        float level = 0.0f; // envelope
        float fade = 1.0f;  // fade-out multiplier (1 unless fading)
        float amp = 0.0f;   // velocity x kVoiceGain
        // SVF state and coefficients
        float ic1 = 0.0f, ic2 = 0.0f;
        float a1 = 0.0f, a2 = 0.0f, a3 = 0.0f;
    };

    void note_on(const NoteEvent& n) noexcept;
    void release(Voice& v) noexcept;
    void fade_out(Voice& v) noexcept;
    void fade_all() noexcept;
    void rematch(const NoteEvent* notes, uint32_t note_count) noexcept;
    void render_voices(float* out, uint32_t frames) noexcept;
    void render_voice(Voice& v, float* out, uint32_t frames) noexcept;

    float sample_rate_;
    float attack_inc_;
    float decay_coef_;
    float release_coef_;
    float fade_step_;
    float detune_up_, detune_down_;
    float max_inc_;

    Voice voices_[kSlots];
    uint32_t active_ = 0; // occupied slots (fast silence check)
    uint64_t next_order_ = 1;
    uint64_t seen_seq_ = 0;
    uint64_t seen_gen_ = 0;
    int64_t expected_pos_ = 0;
    bool have_pos_ = false;
};

} // namespace saine
