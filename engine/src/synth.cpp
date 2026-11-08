#include "synth.hpp"

#include <algorithm>
#include <cmath>
#include <cstring>
#include <numbers>

namespace saine {

namespace {

// Envelope level below which a releasing voice is freed (-90 dB).
constexpr float kOffLevel = 3.1623e-5f;
// Both oscillators start here: the blended waveform is non-zero at 0.25, so
// the very first sample of a note (its note-on frame) is already non-zero.
constexpr float kStartPhase = 0.25f;

// 2-sample polynomial band-limited step residual (polyBLEP).
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

// Equal blend of a band-limited saw and a band-limited square, phase t in [0, 1).
inline float osc(float t, float dt) noexcept {
    const float saw = 2.0f * t - 1.0f - blep(t, dt);
    float t2 = t + 0.5f;
    if (t2 >= 1.0f) t2 -= 1.0f;
    const float square = (t < 0.5f ? 1.0f : -1.0f) + blep(t, dt) - blep(t2, dt);
    return 0.5f * (saw + square);
}

} // namespace

Synth::Synth(double sample_rate) noexcept : sample_rate_(static_cast<float>(sample_rate)) {
    const double sr = sample_rate;
    attack_inc_ = static_cast<float>(1.0 / std::max(1.0, static_cast<double>(kAttackSeconds) * sr));
    // Exponential segments: the remaining distance shrinks by 60 dB over the
    // nominal time (decay: to sustain; release: to zero).
    decay_coef_ = static_cast<float>(std::exp(std::log(1e-3) / (static_cast<double>(kDecaySeconds) * sr)));
    release_coef_ = static_cast<float>(std::exp(std::log(1e-3) / (static_cast<double>(kReleaseSeconds) * sr)));
    fade_step_ = static_cast<float>(1.0 / std::max(1.0, static_cast<double>(kFadeSeconds) * sr));
    detune_up_ = static_cast<float>(std::exp2(static_cast<double>(kDetuneCents) / 1200.0));
    detune_down_ = static_cast<float>(std::exp2(-static_cast<double>(kDetuneCents) / 1200.0));
    max_inc_ = 0.45f; // keep oscillators below Nyquist for extreme pitch / low rates
}

uint32_t Synth::active_slots() const noexcept { return active_; }

uint32_t Synth::note_voices() const noexcept {
    uint32_t n = 0;
    for (const Voice& v : voices_)
        if (v.stage != Stage::Free && !v.fading) ++n;
    return n;
}

void Synth::release(Voice& v) noexcept {
    v.held = false;
    if (v.stage != Stage::Free) v.stage = Stage::Release;
}

void Synth::fade_out(Voice& v) noexcept {
    v.held = false;
    v.fading = true;
}

void Synth::fade_all() noexcept {
    if (active_ == 0) return;
    for (Voice& v : voices_)
        if (v.stage != Stage::Free) fade_out(v);
}

void Synth::note_on(const NoteEvent& n) noexcept {
    // Polyphony limit: fade out the oldest note voice.
    if (note_voices() >= kMaxNotes) {
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

    const double sr = static_cast<double>(sample_rate_);
    const double freq = 440.0 * std::exp2((static_cast<double>(n.pitch) - 69.0) / 12.0);
    const auto inc = static_cast<float>(freq / sr);
    // Key-tracked low-pass cutoff.
    const double fc = std::min(std::clamp(freq * 8.0, 500.0, 12000.0), 0.45 * sr);
    const double g = std::tan(std::numbers::pi * fc / sr);
    const double k = std::numbers::sqrt2; // 1/Q, Q = 0.707
    const double a1 = 1.0 / (1.0 + g * (g + k));

    Voice& v = *slot;
    v = Voice{};
    v.stage = Stage::Attack;
    v.held = true;
    v.pitch = n.pitch;
    v.note_start = n.start;
    v.note_end = n.end;
    v.order = next_order_++;
    v.phase1 = kStartPhase;
    v.phase2 = kStartPhase;
    v.inc1 = std::min(inc * detune_up_, max_inc_);
    v.inc2 = std::min(inc * detune_down_, max_inc_);
    v.amp = kVoiceGain * static_cast<float>(n.velocity) / 127.0f;
    v.a1 = static_cast<float>(a1);
    v.a2 = static_cast<float>(g * a1);
    v.a3 = static_cast<float>(g * g * a1);
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

void Synth::render_voice(Voice& v, float* out, uint32_t frames) noexcept {
    float p1 = v.phase1, p2 = v.phase2, level = v.level, fade = v.fade, ic1 = v.ic1, ic2 = v.ic2;
    const float i1 = v.inc1, i2 = v.inc2, a1 = v.a1, a2 = v.a2, a3 = v.a3, amp = v.amp;
    Stage stage = v.stage;
    const bool fading = v.fading;
    bool done = false;
    for (uint32_t i = 0; i < frames; ++i) {
        const float x = 0.5f * (osc(p1, i1) + osc(p2, i2));
        p1 += i1;
        if (p1 >= 1.0f) p1 -= 1.0f;
        p2 += i2;
        if (p2 >= 1.0f) p2 -= 1.0f;

        // TPT state-variable filter, low-pass output.
        const float v3 = x - ic2;
        const float v1 = a1 * ic1 + a2 * v3;
        const float v2 = ic2 + a2 * ic1 + a3 * v3;
        ic1 = 2.0f * v1 - ic1;
        ic2 = 2.0f * v2 - ic2;

        switch (stage) {
        case Stage::Attack:
            level += attack_inc_;
            if (level >= 1.0f) {
                level = 1.0f;
                stage = Stage::Decay;
            }
            break;
        case Stage::Decay: level = kSustain + (level - kSustain) * decay_coef_; break;
        case Stage::Release: level *= release_coef_; break;
        case Stage::Free: break;
        }

        out[i] += v2 * level * amp * fade;

        if (fading) {
            fade -= fade_step_;
            if (fade <= 0.0f) {
                done = true;
                break;
            }
        }
        if (stage == Stage::Release && level < kOffLevel) {
            done = true;
            break;
        }
    }
    if (done) {
        v.stage = Stage::Free;
        v.fading = false;
        v.held = false;
        --active_;
        return;
    }
    v.phase1 = p1;
    v.phase2 = p2;
    v.level = level;
    v.fade = fade;
    v.ic1 = ic1;
    v.ic2 = ic2;
    v.stage = stage;
}

void Synth::render_voices(float* out, uint32_t frames) noexcept {
    if (frames == 0) return;
    for (Voice& v : voices_)
        if (v.stage != Stage::Free) render_voice(v, out, frames);
}

bool Synth::render(const NoteEvent* notes, uint32_t note_count, uint64_t graph_seq, uint64_t transport_gen,
                   int64_t position, uint32_t frames, bool playing, float* out) noexcept {
    // Transport jump (seek/stop/pause/loop wrap) or timeline discontinuity:
    // fade out everything that sounds. Notes in progress at the new position
    // are not retriggered.
    if (seen_gen_ != transport_gen || (playing && have_pos_ && position != expected_pos_)) fade_all();
    seen_gen_ = transport_gen;
    if (seen_seq_ != graph_seq) {
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
