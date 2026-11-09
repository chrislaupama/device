// Parametric EQ sections shared by the EQ kernels (Channel EQ, Character EQ)
// and the Auto Filter. The design mirrors saine_core::builtin::eq (Rust), so
// the curve the panel draws is the response the engine applies:
//
//  - every section is a trapezoidal (TPT, "zero-delay feedback") state
//    variable filter. A two-pole section outputs m0 * x + m1 * bp + m2 * lp,
//    a one-pole section m0 * hp + m1 * lp; g = tan(pi fc / fs) is the
//    prewarped corner, k = 1 / Q the damping. TPT sections stay stable and
//    quiet while their coefficients move, so bands can be swept per control
//    period without zipper noise or blow-ups;
//  - a band is up to kMaxSections sections (Butterworth cascades for steep
//    pass filters, split shelves for steep shelves, plus an overshoot bell).
//
// Everything here is RT safe: fixed-size arrays, no allocation.
#pragma once

#include <algorithm>
#include <cmath>
#include <cstdint>

namespace saine::eq {

constexpr double kPi = 3.14159265358979323846;
constexpr double kMaxCorner = 0.49; // highest corner as a fraction of fs
constexpr int kMaxSections = 4;

enum class Shape : uint8_t { Bell, LowShelf, HighShelf, Notch, HighPass, LowPass, DualLow };

inline bool has_gain(Shape s) noexcept {
    return s == Shape::Bell || s == Shape::LowShelf || s == Shape::HighShelf || s == Shape::DualLow;
}

struct Band {
    Shape shape = Shape::Bell;
    float freq = 1000.0f;
    float gain_db = 0.0f;
    float q = 0.7071f;
    float slope_db = 12.0f;
    bool prop_q = false;
    float overshoot = 0.0f; // 0..1
    float low_atten_db = 0.0f;
};

// Two-pole: out = m[0] x + m[1] bp + m[2] lp. One-pole: out = m[0] hp + m[1] lp.
struct Coef {
    bool two = true;
    float g = 0.0f;
    float k = 1.0f;
    float m[3] = {1.0f, 0.0f, 0.0f};
    // Derived TPT gains.
    float a1 = 0.0f, a2 = 0.0f, a3 = 0.0f;
    void finish() noexcept {
        if (two) {
            a1 = 1.0f / (1.0f + g * (g + k));
            a2 = g * a1;
            a3 = g * a2;
        } else {
            a1 = g / (1.0f + g);
        }
    }
};

struct Design {
    Coef s[kMaxSections];
    int n = 0;
    void push(const Coef& c) noexcept {
        if (n < kMaxSections) {
            s[n] = c;
            s[n].finish();
            ++n;
        }
    }
};

inline double prewarp(double freq, double fs) noexcept {
    return std::tan(kPi * std::clamp(freq / fs, 1e-6, kMaxCorner));
}

inline Coef two(double g, double k, double m0, double m1, double m2) noexcept {
    Coef c;
    c.two = true;
    c.g = static_cast<float>(g);
    c.k = static_cast<float>(k);
    c.m[0] = static_cast<float>(m0);
    c.m[1] = static_cast<float>(m1);
    c.m[2] = static_cast<float>(m2);
    return c;
}

inline Coef one(double g, double m0, double m1) noexcept {
    Coef c;
    c.two = false;
    c.g = static_cast<float>(g);
    c.m[0] = static_cast<float>(m0);
    c.m[1] = static_cast<float>(m1);
    return c;
}

inline Coef bell(double f, double gain_db, double q, double fs) noexcept {
    const double a = std::pow(10.0, gain_db / 40.0);
    const double k = 1.0 / (q * a);
    return two(prewarp(f, fs), k, 1.0, k * (a * a - 1.0), 0.0);
}
inline Coef low_shelf2(double f, double gain_db, double q, double fs) noexcept {
    const double a = std::pow(10.0, gain_db / 40.0);
    const double k = 1.0 / q;
    return two(prewarp(f, fs) / std::sqrt(a), k, 1.0, k * (a - 1.0), a * a - 1.0);
}
inline Coef high_shelf2(double f, double gain_db, double q, double fs) noexcept {
    const double a = std::pow(10.0, gain_db / 40.0);
    const double k = 1.0 / q;
    return two(prewarp(f, fs) * std::sqrt(a), k, a * a, k * (1.0 - a) * a, 1.0 - a * a);
}
inline Coef low_shelf1(double f, double gain_db, double fs) noexcept {
    const double gg = std::pow(10.0, gain_db / 20.0);
    return one(prewarp(f, fs) / std::sqrt(gg), 1.0, gg);
}
inline Coef high_shelf1(double f, double gain_db, double fs) noexcept {
    const double gg = std::pow(10.0, gain_db / 20.0);
    return one(prewarp(f, fs) * std::sqrt(gg), gg, 1.0);
}
// Band-pass with unity gain at the centre (listen / solo, detectors).
inline Coef band_pass(float f, float q, double fs) noexcept {
    const double k = 1.0 / static_cast<double>(std::max(0.05f, q));
    return two(prewarp(static_cast<double>(f), fs), k, 0.0, k, 0.0);
}

constexpr double kOvershootRatio = 1.6;
constexpr double kOvershootQ = 1.2;
constexpr double kDualLowCutRatio = 0.7;
constexpr double kDualLowCutQ = 0.5;

// Butterworth section Qs (highest first) of an order 1..8 filter.
struct ButterQs {
    double q[4];
    int n2;
    bool first;
};
inline ButterQs butterworth(int order) noexcept {
    ButterQs b{};
    const int n = std::clamp(order, 1, 8);
    b.n2 = n / 2;
    b.first = (n % 2) == 1;
    for (int i = 1; i <= b.n2; ++i) {
        const double angle = (n % 2 == 0) ? (2 * i - 1) * kPi / (2 * n) : i * kPi / n;
        b.q[i - 1] = 1.0 / (2.0 * std::cos(angle));
    }
    std::sort(b.q, b.q + b.n2, [](double x, double y) { return x > y; });
    return b;
}

// Second- and first-order section counts of a shelf (6, 12, 18, 24 dB/oct).
inline void shelf_split(float slope_db, int& n2, int& n1) noexcept {
    const long u = std::lround(slope_db / 6.0f);
    if (u <= 1) { n2 = 0; n1 = 1; }
    else if (u == 2) { n2 = 1; n1 = 0; }
    else if (u == 3) { n2 = 1; n1 = 1; }
    else { n2 = 2; n1 = 0; }
}

inline double bell_q(const Band& b) noexcept {
    const double q = static_cast<double>(std::max(0.01f, b.q));
    return b.prop_q ? q * std::exp2(static_cast<double>(std::fabs(b.gain_db)) / 12.0) : q;
}

// The sections of one band (an empty design is a flat band).
inline Design design(const Band& b, double fs) noexcept {
    Design d;
    const double f = static_cast<double>(std::max(1.0f, b.freq));
    const double g = static_cast<double>(b.gain_db);
    const double q = static_cast<double>(std::max(0.01f, b.q));
    const double ov = static_cast<double>(std::clamp(b.overshoot, 0.0f, 1.0f));
    switch (b.shape) {
    case Shape::Bell:
        if (g != 0.0) d.push(bell(f, g, bell_q(b), fs));
        break;
    case Shape::Notch: d.push(two(prewarp(f, fs), 1.0 / q, 1.0, -1.0 / q, 0.0)); break;
    case Shape::LowShelf:
    case Shape::HighShelf: {
        if (g == 0.0) break;
        const bool low = b.shape == Shape::LowShelf;
        int n2 = 0, n1 = 0;
        shelf_split(b.slope_db, n2, n1);
        const double units = 2 * n2 + n1;
        for (int i = 0; i < n2; ++i) d.push(low ? low_shelf2(f, g * 2.0 / units, q, fs) : high_shelf2(f, g * 2.0 / units, q, fs));
        for (int i = 0; i < n1; ++i) d.push(low ? low_shelf1(f, g / units, fs) : high_shelf1(f, g / units, fs));
        if (ov > 0.0) d.push(bell(low ? f * kOvershootRatio : f / kOvershootRatio, -0.5 * ov * g, kOvershootQ, fs));
        break;
    }
    case Shape::HighPass:
    case Shape::LowPass: {
        const bool hp = b.shape == Shape::HighPass;
        const int order = static_cast<int>(std::clamp(std::lround(b.slope_db / 6.0f), 1L, 8L));
        const ButterQs bq = butterworth(order);
        const double w = prewarp(f, fs);
        for (int i = 0; i < bq.n2; ++i) {
            const double qq = i == 0 ? bq.q[i] * (q / 0.70710678118654752) * (1.0 + 2.0 * ov) : bq.q[i];
            const double k = 1.0 / qq;
            d.push(hp ? two(w, k, 1.0, -k, -1.0) : two(w, k, 0.0, 0.0, 1.0));
        }
        if (bq.first) d.push(hp ? one(w, 1.0, 0.0) : one(w, 0.0, 1.0));
        break;
    }
    case Shape::DualLow:
        if (g != 0.0) d.push(low_shelf2(f, g, q, fs));
        if (b.low_atten_db > 0.0f) d.push(low_shelf2(f * kDualLowCutRatio, -static_cast<double>(b.low_atten_db), kDualLowCutQ, fs));
        break;
    }
    return d;
}

// Magnitude squared of one section at freq (auto gain, tests).
inline double mag2(const Coef& c, double freq, double fs) noexcept {
    const double o = prewarp(freq, fs) / std::max(1e-12, static_cast<double>(c.g));
    double nr, ni, dr, di;
    const double k = static_cast<double>(c.k), m0 = static_cast<double>(c.m[0]), m1 = static_cast<double>(c.m[1]),
                 m2 = static_cast<double>(c.m[2]);
    if (c.two) {
        dr = 1.0 - o * o;
        di = k * o;
        nr = m0 * dr + m2;
        ni = m0 * di + m1 * o;
    } else {
        dr = 1.0;
        di = o;
        nr = m1;
        ni = m0 * o;
    }
    const double den = dr * dr + di * di;
    return den <= 1e-300 ? 0.0 : (nr * nr + ni * ni) / den;
}

// Stereo state of one section.
struct State {
    float s1[2] = {0.0f, 0.0f};
    float s2[2] = {0.0f, 0.0f};
    void clear() noexcept { s1[0] = s1[1] = s2[0] = s2[1] = 0.0f; }
};

// One sample of one channel (detectors, per-sample paths).
inline float tick(const Coef& c, State& st, int ch, float x) noexcept {
    if (c.two) {
        const float ic1 = st.s1[ch], ic2 = st.s2[ch];
        const float v3 = x - ic2;
        const float v1 = c.a1 * ic1 + c.a2 * v3;
        const float v2 = ic2 + c.a2 * ic1 + c.a3 * v3;
        st.s1[ch] = 2.0f * v1 - ic1;
        st.s2[ch] = 2.0f * v2 - ic2;
        return c.m[0] * x + c.m[1] * v1 + c.m[2] * v2;
    }
    const float v = (x - st.s1[ch]) * c.a1;
    const float lp = v + st.s1[ch];
    st.s1[ch] = lp + v;
    return c.m[0] * (x - lp) + c.m[1] * lp;
}

// One section over a stereo block: both channels advance in the same loop
// iteration (two independent dependency chains the CPU overlaps, and the
// compiler can pack into one vector register).
inline void run(const Coef& c, State& st, float* l, float* r, uint32_t n) noexcept {
    if (c.two) {
        const float a1 = c.a1, a2 = c.a2, a3 = c.a3, m0 = c.m[0], m1 = c.m[1], m2 = c.m[2];
        float p1 = st.s1[0], p2 = st.s2[0], q1 = st.s1[1], q2 = st.s2[1];
        for (uint32_t i = 0; i < n; ++i) {
            const float xl = l[i], xr = r[i];
            const float v3l = xl - p2, v3r = xr - q2;
            const float v1l = a1 * p1 + a2 * v3l, v1r = a1 * q1 + a2 * v3r;
            const float v2l = p2 + a2 * p1 + a3 * v3l, v2r = q2 + a2 * q1 + a3 * v3r;
            p1 = 2.0f * v1l - p1;
            q1 = 2.0f * v1r - q1;
            p2 = 2.0f * v2l - p2;
            q2 = 2.0f * v2r - q2;
            l[i] = m0 * xl + m1 * v1l + m2 * v2l;
            r[i] = m0 * xr + m1 * v1r + m2 * v2r;
        }
        st.s1[0] = p1;
        st.s2[0] = p2;
        st.s1[1] = q1;
        st.s2[1] = q2;
        return;
    }
    const float G = c.a1, m0 = c.m[0], m1 = c.m[1];
    float sl = st.s1[0], sr = st.s1[1];
    for (uint32_t i = 0; i < n; ++i) {
        const float vl = (l[i] - sl) * G, vr = (r[i] - sr) * G;
        const float lpl = vl + sl, lpr = vr + sr;
        sl = lpl + vl;
        sr = lpr + vr;
        l[i] = m0 * (l[i] - lpl) + m1 * lpl;
        r[i] = m0 * (r[i] - lpr) + m1 * lpr;
    }
    st.s1[0] = sl;
    st.s1[1] = sr;
}

// A cascade of sections over a stereo block, sample-major: within one sample
// the sections depend on each other, but section j at sample n + 1 does not
// wait for section j + 1 at sample n, so the CPU overlaps the cascade
// instead of waiting on one section's feedback loop at a time (about 2.5x
// faster than running the sections one block at a time).
struct Cascade {
    static constexpr int kMax = 64;
    const Coef* c[kMax];
    State* s[kMax];
    int n = 0;
    void clear() noexcept { n = 0; }
    void add(const Coef& coef, State& st) noexcept {
        if (n < kMax) {
            c[n] = &coef;
            s[n] = &st;
            ++n;
        }
    }
    void run(float* l, float* r, uint32_t frames) const noexcept {
        if (n == 0) return;
        if (n == 1) {
            saine::eq::run(*c[0], *s[0], l, r, frames);
            return;
        }
        for (uint32_t i = 0; i < frames; ++i) {
            float xl = l[i], xr = r[i];
            for (int j = 0; j < n; ++j) {
                const Coef& k = *c[j];
                State& st = *s[j];
                if (k.two) {
                    const float p1 = st.s1[0], p2 = st.s2[0], q1 = st.s1[1], q2 = st.s2[1];
                    const float v3l = xl - p2, v3r = xr - q2;
                    const float v1l = k.a1 * p1 + k.a2 * v3l, v1r = k.a1 * q1 + k.a2 * v3r;
                    const float v2l = p2 + k.a2 * p1 + k.a3 * v3l, v2r = q2 + k.a2 * q1 + k.a3 * v3r;
                    st.s1[0] = 2.0f * v1l - p1;
                    st.s1[1] = 2.0f * v1r - q1;
                    st.s2[0] = 2.0f * v2l - p2;
                    st.s2[1] = 2.0f * v2r - q2;
                    xl = k.m[0] * xl + k.m[1] * v1l + k.m[2] * v2l;
                    xr = k.m[0] * xr + k.m[1] * v1r + k.m[2] * v2r;
                } else {
                    const float sl = st.s1[0], sr = st.s1[1];
                    const float vl = (xl - sl) * k.a1, vr = (xr - sr) * k.a1;
                    const float lpl = vl + sl, lpr = vr + sr;
                    st.s1[0] = lpl + vl;
                    st.s1[1] = lpr + vr;
                    xl = k.m[0] * (xl - lpl) + k.m[1] * lpl;
                    xr = k.m[0] * (xr - lpr) + k.m[1] * lpr;
                }
            }
            l[i] = xl;
            r[i] = xr;
        }
    }
};

// Flush denormals and recover from non-finite state (hostile input).
inline void sanitize(State& st) noexcept {
    for (int ch = 0; ch < 2; ++ch) {
        float& a = st.s1[ch];
        float& b = st.s2[ch];
        if (!std::isfinite(a) || !std::isfinite(b) || std::fabs(a) > 1e9f || std::fabs(b) > 1e9f) a = b = 0.0f;
        if (std::fabs(a) < 1e-20f) a = 0.0f;
        if (std::fabs(b) < 1e-20f) b = 0.0f;
    }
}

// A band in an EQ: target and smoothed settings, its sections and their
// state. Frequency and Q glide in the log domain, gain in dB, so sweeps
// sound even across the range. Switching a band on or off crossfades its
// effect (level 0..1) instead of clicking.
struct BandRunner {
    Band target{};
    Band cur{};
    bool on = false;
    float level = 0.0f;
    Design d{};
    State st[kMaxSections]{};
    bool primed = false;
    bool dirty = true;
    // Shape or slope changed: the sections' saved state belongs to another design.
    bool retopo = false;

    void reset() noexcept {
        for (State& s : st) s.clear();
        primed = false;
        dirty = true;
    }

    // One control step toward the target (one-pole coefficient a per step).
    // Returns true while the band is still moving.
    bool glide(float a) noexcept {
        if (!primed) {
            cur = target;
            level = on ? 1.0f : 0.0f;
            primed = true;
            dirty = true;
            return false;
        }
        if (cur.shape != target.shape || cur.slope_db != target.slope_db || cur.prop_q != target.prop_q) {
            retopo |= cur.shape != target.shape || cur.slope_db != target.slope_db;
            cur.shape = target.shape;
            cur.slope_db = target.slope_db;
            cur.prop_q = target.prop_q;
            dirty = true;
        }
        bool moving = false;
        auto step_log = [&](float& v, float t) {
            if (v == t) return;
            const float lv = std::log(std::max(v, 1e-6f)), lt = std::log(std::max(t, 1e-6f));
            const float nl = lt + a * (lv - lt);
            v = std::fabs(nl - lt) < 1e-5f ? t : std::exp(nl);
            moving = true;
        };
        auto step_lin = [&](float& v, float t, float eps) {
            if (v == t) return;
            const float nv = t + a * (v - t);
            v = std::fabs(nv - t) < eps ? t : nv;
            moving = true;
        };
        step_log(cur.freq, target.freq);
        step_log(cur.q, target.q);
        step_lin(cur.gain_db, target.gain_db, 1e-4f);
        step_lin(cur.overshoot, target.overshoot, 1e-5f);
        step_lin(cur.low_atten_db, target.low_atten_db, 1e-4f);
        step_lin(level, on ? 1.0f : 0.0f, 1e-4f);
        dirty |= moving;
        return moving;
    }

    // Sections from cur (plus a dynamic gain offset), scaled by the on/off level.
    void redesign(double fs, float extra_gain_db = 0.0f) noexcept {
        Band b = cur;
        b.gain_db += extra_gain_db;
        const int old_n = d.n;
        if (level <= 0.0f) {
            d.n = 0;
        } else if (level >= 1.0f) {
            d = design(b, fs);
        } else if (has_gain(b.shape)) {
            b.gain_db *= level;
            b.low_atten_db *= level;
            d = design(b, fs);
        } else {
            // Pass / notch: blend each section toward a flat one.
            d = design(b, fs);
            for (int i = 0; i < d.n; ++i) {
                Coef& c = d.s[i];
                c.m[0] = level * c.m[0] + (1.0f - level);
                if (c.two) {
                    c.m[1] *= level;
                    c.m[2] *= level;
                } else {
                    c.m[1] = level * c.m[1] + (1.0f - level);
                }
            }
        }
        for (int i = d.n; i < old_n; ++i) st[i].clear();
        if (retopo) {
            for (int i = 0; i < d.n; ++i) st[i].clear();
            retopo = false;
        }
        dirty = false;
    }

    void run_block(float* l, float* r, uint32_t n) noexcept {
        for (int i = 0; i < d.n; ++i) run(d.s[i], st[i], l, r, n);
    }

    void add_to(Cascade& c) noexcept {
        for (int i = 0; i < d.n; ++i) c.add(d.s[i], st[i]);
    }

    void sanitize_state() noexcept {
        for (int i = 0; i < d.n; ++i) sanitize(st[i]);
    }
};

} // namespace saine::eq
