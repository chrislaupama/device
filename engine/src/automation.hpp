// Parameter automation (docs/adr/0005-automation.md): lane evaluation, the
// per-sample ramp fill used for volume/pan, the timeline history used to
// keep lanes aligned with delay-compensated audio, and the synth parameter
// mapping. Everything here is RT-safe (no allocation, no locks).
#pragma once

#include "saine_engine.h"

#include <algorithm>
#include <cmath>
#include <cstdint>
#include <cstring>

namespace saine {

// Same layout as se_auto_point (copied with memcpy).
struct AutoPoint {
    int64_t frame = 0;
    float value = 0.0f;
    float curve = 0.0f;
};
static_assert(sizeof(AutoPoint) == sizeof(se_auto_point));

struct LaneView {
    const AutoPoint* p = nullptr;
    uint32_t n = 0; // > 0
    bool log = false;
};

// ---- the evaluation contract (saine_core::automation, bit-for-bit) --------
// Float contraction (FMA) would change rounding versus the Rust reference.
#if defined(__clang__)
#pragma clang fp contract(off)
#endif

inline float auto_shape(float t, float c) noexcept {
    t = std::clamp(t, 0.0f, 1.0f);
    c = std::clamp(c, -1.0f, 1.0f);
    if (std::fabs(c) < 1e-6f) return t;
    if (c > 0.0f) return std::pow(t, 1.0f + 3.0f * c);
    return 1.0f - std::pow(1.0f - t, 1.0f - 3.0f * c);
}

inline float auto_lerp(float a, float b, float s, bool log) noexcept {
    if (log && a > 0.0f && b > 0.0f) {
        const float la = std::log(a);
        const float lb = std::log(b);
        return std::exp(la + (lb - la) * s);
    }
    return a + (b - a) * s;
}

// Value of the segment a -> b (a.frame < b.frame) at a.frame <= t <= b.frame.
inline float auto_segment(const AutoPoint& a, const AutoPoint& b, int64_t t, bool log) noexcept {
    const float s = auto_shape(static_cast<float>(t - a.frame) / static_cast<float>(b.frame - a.frame), a.curve);
    return auto_lerp(a.value, b.value, s, log);
}

// Index of the first point with frame > t.
inline uint32_t auto_upper(const LaneView& l, int64_t t) noexcept {
    const AutoPoint* it =
        std::upper_bound(l.p, l.p + l.n, t, [](int64_t v, const AutoPoint& q) { return v < q.frame; });
    return static_cast<uint32_t>(it - l.p);
}

inline float auto_eval(const LaneView& l, int64_t t) noexcept {
    const AutoPoint* p = l.p;
    if (t <= p[0].frame) return p[0].value;
    const uint32_t i = auto_upper(l, t);
    if (i >= l.n) return p[l.n - 1].value;
    const AutoPoint& a = p[i - 1];
    const AutoPoint& b = p[i];
    if (b.frame == a.frame) return b.value;
    return auto_segment(a, b, t, l.log);
}

#if defined(__clang__)
#pragma clang fp contract(on)
#endif

// ---- ramp fill ---------------------------------------------------------------
inline constexpr uint32_t kAutoRampFrames = 32;

// dst[k] = map(value at frame t0 + k) for k < n, as linear ramps of the
// mapped value over runs of at most kAutoRampFrames samples that never cross
// a point (exact at each run start; a run ending on a point ends on the left
// limit, so vertical steps stay steps). Constant stretches are filled flat;
// a run whose mapped ends are zero and non-zero (leaving / entering
// silence) is evaluated per sample.
template <class Map>
void auto_fill(const LaneView& l, int64_t t0, uint32_t n, Map&& map, float* dst) noexcept {
    const AutoPoint* p = l.p;
    uint32_t i = 0;
    const auto flat = [&](uint32_t to, float v) {
        const float g = map(v);
        for (; i < to; ++i) dst[i] = g;
    };
    // Hold before (and at) the first point.
    if (t0 <= p[0].frame) {
        const int64_t run = p[0].frame - t0 + 1;
        flat(static_cast<uint32_t>(std::min<int64_t>(run, n)), p[0].value);
        if (i >= n) return;
    }
    uint32_t idx = auto_upper(l, t0 + i);
    while (i < n) {
        const int64_t t = t0 + i;
        while (idx < l.n && p[idx].frame <= t) ++idx;
        if (idx >= l.n) {
            flat(n, p[l.n - 1].value);
            return;
        }
        const AutoPoint& a = p[idx - 1];
        const AutoPoint& b = p[idx];
        const uint32_t seg_left = static_cast<uint32_t>(std::min<int64_t>(b.frame - t, n - i));
        if (a.value == b.value) { // flat segment (any curve)
            flat(i + seg_left, a.value);
            continue;
        }
        const uint32_t len = std::min(seg_left, kAutoRampFrames);
        const float g0 = map(auto_segment(a, b, t, l.log));
        const float g1 = map(auto_segment(a, b, t + len, l.log));
        if ((g0 == 0.0f) != (g1 == 0.0f)) {
            // Crossing a hard zero of the map (volume: -60 dB is silence, the
            // next step up is -60 dB = 0.001): exact per sample, no smear.
            for (uint32_t k = 0; k < len; ++k) dst[i + k] = map(auto_segment(a, b, t + k, l.log));
        } else {
            const float step = (g1 - g0) / static_cast<float>(len);
            for (uint32_t k = 0; k < len; ++k) dst[i + k] = g0 + step * static_cast<float>(k);
        }
        i += len;
    }
}

// dB -> linear gain; <= -60 dB is silence.
inline float auto_db_to_gain(float db) noexcept {
    if (!(db > -60.0f)) return 0.0f;
    return std::pow(10.0f, db * 0.05f);
}

// ---- timeline history --------------------------------------------------------
// Which timeline frame each recently rendered output sample was read at
// (playback segments, newest last). Lets a stage that processes audio d
// samples late (delay compensation) evaluate its lanes at the frame that
// audio came from, across loop wraps and seeks. RT-owned.
class TimeHistory {
public:
    static constexpr uint32_t kCap = 64;

    void reset() noexcept { count_ = 0; }

    // After rendering n samples starting at `tl` (moving: playing, so the
    // frame advances per sample; else every sample is at `tl`).
    void append(int64_t tl, uint32_t n, bool moving) noexcept {
        if (n == 0) return;
        if (count_ > 0) {
            Seg& s = at(0);
            const bool contiguous = s.moving == moving && (moving ? s.tl + s.len == tl : s.tl == tl);
            if (contiguous) {
                s.len += n;
                return;
            }
        }
        head_ = (head_ + 1) % kCap;
        ring_[head_] = Seg{tl, n, moving};
        if (count_ < kCap) ++count_;
    }

    // Calls f(i0, i1, tl0, moving) for consecutive pieces covering output
    // offsets [0, n) of the block being rendered (starting at `position`,
    // moving = playing) seen `delay` samples late: sample i maps to the
    // frame rendered `delay` samples before it; within a piece the frame is
    // tl0 + (i - i0) (moving) or tl0. Without history (or beyond it) the
    // oldest known segment is extrapolated.
    template <class F>
    void pieces(int64_t position, bool moving, uint32_t n, uint32_t delay, F&& f) const noexcept {
        if (n == 0) return;
        if (delay == 0 || count_ == 0) {
            f(0u, n, moving ? position - static_cast<int64_t>(delay) : position, moving);
            return;
        }
        uint32_t i = 0;
        int64_t j = -static_cast<int64_t>(delay); // sample index relative to the block start
        int64_t total = 0;
        for (uint32_t k = 0; k < count_; ++k) total += at(k).len;
        if (-j > total) { // older than the history: extrapolate the oldest segment
            const Seg& o = at(count_ - 1);
            const int64_t before = -j - total;
            const auto m = static_cast<uint32_t>(std::min<int64_t>(before, n));
            f(0u, m, o.moving ? o.tl - before : o.tl, o.moving);
            i = m;
            j += m;
        }
        while (i < n && j < 0) {
            uint32_t k = 0;
            int64_t end = 0;
            int64_t start = -at(0).len;
            while (j < start) {
                end = start;
                start -= at(++k).len;
            }
            const Seg& s = at(k);
            const auto m = static_cast<uint32_t>(std::min<int64_t>(n - i, end - j));
            f(i, i + m, s.moving ? s.tl + (j - start) : s.tl, s.moving);
            i += m;
            j += m;
        }
        if (i < n) f(i, n, moving ? position + j : position, moving);
    }

private:
    struct Seg {
        int64_t tl = 0;
        int64_t len = 0;
        bool moving = false;
    };
    // k = 0: newest.
    Seg& at(uint32_t k) noexcept { return ring_[(head_ + kCap - k) % kCap]; }
    const Seg& at(uint32_t k) const noexcept { return ring_[(head_ + kCap - k) % kCap]; }

    Seg ring_[kCap];
    uint32_t head_ = 0;
    uint32_t count_ = 0;
};

// Linear mapping used when no history is given (export: linear playback).
template <class F>
void linear_pieces(int64_t position, bool moving, uint32_t n, uint32_t delay, F&& f) noexcept {
    if (n > 0) f(0u, n, moving ? position - static_cast<int64_t>(delay) : position, moving);
}

// ---- built-in synth parameters ------------------------------------------------
inline float* synth_param_field(se_synth_params& p, uint32_t index) noexcept {
    switch (index) {
    case SE_SYNTH_P_OSC1_LEVEL: return &p.osc1_level;
    case SE_SYNTH_P_OSC1_SHAPE: return &p.osc1_shape;
    case SE_SYNTH_P_OSC1_FINE: return &p.osc1_fine_cents;
    case SE_SYNTH_P_OSC2_LEVEL: return &p.osc2_level;
    case SE_SYNTH_P_OSC2_SHAPE: return &p.osc2_shape;
    case SE_SYNTH_P_OSC2_FINE: return &p.osc2_fine_cents;
    case SE_SYNTH_P_CROSS_FM: return &p.cross_fm;
    case SE_SYNTH_P_SUB_LEVEL: return &p.sub_level;
    case SE_SYNTH_P_NOISE_LEVEL: return &p.noise_level;
    case SE_SYNTH_P_FILTER_CUTOFF: return &p.filter_cutoff_hz;
    case SE_SYNTH_P_FILTER_RESONANCE: return &p.filter_resonance;
    case SE_SYNTH_P_FILTER_KEY_TRACK: return &p.filter_key_track;
    case SE_SYNTH_P_FILTER_ENV_AMOUNT: return &p.filter_env_amount;
    case SE_SYNTH_P_LFO_RATE: return &p.lfo_rate_hz;
    case SE_SYNTH_P_LFO_TO_PITCH: return &p.lfo_to_pitch_cents;
    case SE_SYNTH_P_LFO_TO_CUTOFF: return &p.lfo_to_cutoff_oct;
    case SE_SYNTH_P_LFO_TO_AMP: return &p.lfo_to_amp;
    case SE_SYNTH_P_GLIDE: return &p.glide;
    case SE_SYNTH_P_UNISON_DETUNE: return &p.unison_detune_cents;
    case SE_SYNTH_P_OUTPUT: return &p.output;
    default: return nullptr;
    }
}

// (min, max) of a synth parameter (= SynthParam::range in saine_core).
inline void synth_param_range(uint32_t index, float& lo, float& hi) noexcept {
    lo = 0.0f;
    hi = 1.0f;
    switch (index) {
    case SE_SYNTH_P_OSC1_FINE:
    case SE_SYNTH_P_OSC2_FINE: lo = -100.0f, hi = 100.0f; break;
    case SE_SYNTH_P_FILTER_CUTOFF: lo = 20.0f, hi = 20000.0f; break;
    case SE_SYNTH_P_FILTER_ENV_AMOUNT: lo = -1.0f, hi = 1.0f; break;
    case SE_SYNTH_P_LFO_RATE: lo = 0.01f, hi = 20.0f; break;
    case SE_SYNTH_P_LFO_TO_PITCH: hi = 1200.0f; break;
    case SE_SYNTH_P_LFO_TO_CUTOFF: hi = 6.0f; break;
    case SE_SYNTH_P_GLIDE: hi = 2.0f; break;
    case SE_SYNTH_P_UNISON_DETUNE: hi = 50.0f; break;
    case SE_SYNTH_P_OUTPUT: hi = 2.0f; break;
    default: break;
    }
}

} // namespace saine
