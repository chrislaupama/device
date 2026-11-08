// Character EQ: twelve bands with seven shapes (Bell, Low Shelf, High Shelf,
// High Pass, Low Pass, Notch, Dual Low), per-band slope, proportional Q,
// overshoot, drive and dynamics; ten banks that voice the bands; low-end
// Warmth; an oversampled Valve stage; a four-band resonance Tamer; Auto Gain;
// Output; a band Listen (solo); and a Bypass that keeps the latency.
// Parameters in the order of saine_core::builtin::character_eq (checked by
// a Rust test).
//
// Signal flow (stereo, linked detectors), in 16-sample control periods:
//   bands   serial TPT sections (eq_sections.hpp: the same design as the
//           panel's curve), voiced by the bank. A band with Drive saturates
//           only what it adds, out = x + sat(band(x) - x), antialiased with a
//           first-order antiderivative (ADAA). A dynamic band moves its gain by
//           up to Dyn Range while the level in its region (band-pass of the
//           input) is above the threshold: Dyn Thresh, or with Dyn Auto a slow
//           average of that level; Dyn Attack / Release time the reaction.
//   tamer   up to four dynamic cuts: each compares a narrow band (Width) with
//           the rest of its neighbourhood and cuts what pokes out above a flat
//           spectrum, scaled by Tame Amount, at most 6 dB + |Weight|.
//   warmth  the lows below Warmth Tail (6 or 12 dB/oct, complementary split)
//           go through a gentle asymmetric saturator; 0% is transparent.
//   valve   2x or 4x oversampled (linear-phase halfband FIRs) asymmetric
//           triode-style curve; Valve Trim drives it, Valve Mix blends it.
//   output  Output trim and Auto Gain (minus the curve's mean dB over the
//           audio band, the same figure the panel adds to its curve).
// The oversampler's delay is applied to every path and reported as latency,
// with 2x padded to the 4x delay, so delay compensation never changes.
#include "builtin.hpp"
#include "dsp.hpp"
#include "eq_sections.hpp"

#include <algorithm>
#include <cmath>
#include <cstring>

namespace saine {

namespace {

constexpr int kBands = 12;
constexpr int kTames = 4;
constexpr uint32_t kControl = 16;

enum BandOff : uint32_t { kOn, kType, kFreq, kGain, kQ, kSlope, kPropQ, kOvershoot, kLowAtten };
enum Param : uint32_t {
    kBank = 108,
    kWarmth,
    kWarmthSlope,
    kWarmthTail,
    kValveOn,
    kValveTrim,
    kValveMix,
    kOversampling,
    kOutput,
    kBypass,
    kAutoGain,
    kDyn0 = 119, // 6 per band: on, range, auto, thresh, attack, release
    kTameOn = 191,
    kTameAmount,
    kTame0 = 193, // 4 per tamer band: on, freq, width, weight
    kDrive0 = 209,
    kListen = 221,
    kParamCount = 222
};

#define SAINE_CEQ_BAND(n)                                                                                              \
    "band" #n "_enabled", "band" #n "_type", "band" #n "_freq", "band" #n "_gain", "band" #n "_q", "band" #n "_slope", \
        "band" #n "_propq", "band" #n "_overshoot", "band" #n "_low_atten"
#define SAINE_CEQ_DYN(n)                                                                                               \
    "band" #n "_dyn", "band" #n "_dyn_range", "band" #n "_dyn_auto", "band" #n "_dyn_thresh", "band" #n "_dyn_attack", \
        "band" #n "_dyn_release"
#define SAINE_CEQ_TAME(n) "tame" #n "_enabled", "tame" #n "_freq", "tame" #n "_q", "tame" #n "_weight"

constexpr const char* kParamIds[kParamCount] = {
    SAINE_CEQ_BAND(1), SAINE_CEQ_BAND(2), SAINE_CEQ_BAND(3), SAINE_CEQ_BAND(4), SAINE_CEQ_BAND(5), SAINE_CEQ_BAND(6),
    SAINE_CEQ_BAND(7), SAINE_CEQ_BAND(8), SAINE_CEQ_BAND(9), SAINE_CEQ_BAND(10), SAINE_CEQ_BAND(11), SAINE_CEQ_BAND(12),
    "bank", "warmth_amount", "warmth_slope", "warmth_tail", "valve_enabled", "valve_trim", "valve_mix", "oversampling",
    "output_trim", "bypass", "auto_gain",
    SAINE_CEQ_DYN(1), SAINE_CEQ_DYN(2), SAINE_CEQ_DYN(3), SAINE_CEQ_DYN(4), SAINE_CEQ_DYN(5), SAINE_CEQ_DYN(6),
    SAINE_CEQ_DYN(7), SAINE_CEQ_DYN(8), SAINE_CEQ_DYN(9), SAINE_CEQ_DYN(10), SAINE_CEQ_DYN(11), SAINE_CEQ_DYN(12),
    "tame_enabled", "tame_amount", SAINE_CEQ_TAME(1), SAINE_CEQ_TAME(2), SAINE_CEQ_TAME(3), SAINE_CEQ_TAME(4),
    "band1_drive", "band2_drive", "band3_drive", "band4_drive", "band5_drive", "band6_drive", "band7_drive",
    "band8_drive", "band9_drive", "band10_drive", "band11_drive", "band12_drive", "listen_band"};

#undef SAINE_CEQ_BAND
#undef SAINE_CEQ_DYN
#undef SAINE_CEQ_TAME

// Readouts: each band's dynamic gain offset (dB), then each tamer band's cut (dB, <= 0).
constexpr const char* kReadoutIds[kBands + kTames] = {"dyn1", "dyn2",  "dyn3",  "dyn4",  "dyn5",  "dyn6",
                                                       "dyn7", "dyn8",  "dyn9",  "dyn10", "dyn11", "dyn12",
                                                       "tame1", "tame2", "tame3", "tame4"};

constexpr eq::Shape kShapes[7] = {eq::Shape::Bell,    eq::Shape::LowShelf, eq::Shape::HighShelf, eq::Shape::HighPass,
                                  eq::Shape::LowPass, eq::Shape::Notch,    eq::Shape::DualLow};
constexpr float kSlopes[6] = {6.0f, 12.0f, 18.0f, 24.0f, 36.0f, 48.0f};

// Bank voicings, mirroring saine_core::builtin::eq::BANKS: Q scale,
// proportional Q on bells, added shelf / pass overshoot, drive asymmetry.
struct Voicing {
    float q_scale;
    bool prop_q;
    float overshoot;
    float asymmetry;
};
constexpr Voicing kBanks[10] = {
    {1.0f, false, 0.0f, 0.15f},  {0.8f, false, 0.3f, 0.6f}, {0.75f, true, 0.15f, 0.35f}, {1.0f, true, 0.05f, 0.1f},
    {1.15f, false, 0.0f, 0.05f}, {0.9f, true, 0.0f, 0.45f}, {1.0f, false, 0.2f, 0.45f},  {1.4f, true, 0.1f, 0.3f},
    {0.85f, false, 0.0f, 0.2f},  {0.7f, false, 0.1f, 0.5f},
};

inline int choice(float v, int count) noexcept { return std::clamp(static_cast<int>(std::lround(v)), 0, count - 1); }

// tanh to ~1e-6 (continued-fraction rational, clamped): the valve runs it
// four times per sample per channel at 4x, where std::tanh dominates.
inline float fast_tanh(float x) noexcept {
    const float c = std::clamp(x, -5.0f, 5.0f);
    const float x2 = c * c;
    const float num = c * (135135.0f + x2 * (17325.0f + x2 * (378.0f + x2)));
    const float den = 135135.0f + x2 * (62370.0f + x2 * (3150.0f + 28.0f * x2));
    return std::clamp(num / den, -1.0f, 1.0f);
}

inline float logcosh(float x) noexcept {
    const float a = std::fabs(x);
    return a + std::log1p(std::exp(-2.0f * a)) - 0.69314718f;
}

// Asymmetric tanh, f(u) = tanh(u + b) - tanh(b), with a first-order
// antiderivative antialiaser (F = logcosh(u + b) - tanh(b) u).
struct Adaa {
    float prev[2] = {0.0f, 0.0f};
    void clear() noexcept { prev[0] = prev[1] = 0.0f; }
    float run(int ch, float u, float b, float tb) noexcept {
        const float u0 = prev[ch];
        prev[ch] = u;
        const float d = u - u0;
        if (std::fabs(d) < 1e-3f) return std::tanh(0.5f * (u + u0) + b) - tb;
        return (logcosh(u + b) - logcosh(u0 + b)) / d - tb;
    }
};

// Linear-phase halfband FIR (Kaiser-windowed sinc, cutoff a quarter of the
// high rate) for 2x up- and down-sampling. Fixed storage: no allocation.
struct Halfband {
    static constexpr int kMaxTaps = 48;
    float h[kMaxTaps] = {};
    int n = 0, c = 0;
    int nz[kMaxTaps] = {}; // indices of the non-zero taps
    int nnz = 0;
    float ub[2][kMaxTaps] = {};     // base-rate history (doubled ring)
    float db[2][2 * kMaxTaps] = {}; // high-rate history (doubled ring)
    int up_pos[2] = {0, 0}, dn_pos[2] = {0, 0};

    void design(int taps) noexcept {
        n = std::min(taps, kMaxTaps);
        c = (n - 1) / 2;
        auto i0 = [](double x) {
            double s = 1.0, t = 1.0;
            for (int k = 1; k < 40; ++k) {
                t *= (x / (2.0 * k)) * (x / (2.0 * k));
                s += t;
            }
            return s;
        };
        const double beta = 9.0;
        double sum = 0.0;
        double tmp[kMaxTaps];
        for (int i = 0; i < n; ++i) {
            const int m = i - c;
            double v = m == 0 ? 0.5 : (m % 2 == 0 ? 0.0 : std::sin(0.5 * eq::kPi * m) / (eq::kPi * m));
            const double r = static_cast<double>(m) / (c + 1);
            v *= i0(beta * std::sqrt(std::max(0.0, 1.0 - r * r))) / i0(beta);
            tmp[i] = v;
            sum += v;
        }
        nnz = 0;
        for (int i = 0; i < n; ++i) {
            h[i] = static_cast<float>(tmp[i] / sum);
            if (tmp[i] != 0.0) nz[nnz++] = i;
        }
        clear();
    }
    void clear() noexcept {
        std::memset(ub, 0, sizeof ub);
        std::memset(db, 0, sizeof db);
        up_pos[0] = up_pos[1] = dn_pos[0] = dn_pos[1] = 0;
    }
    // One base sample -> the two high-rate samples y0 (even), y1 (odd).
    void up(int ch, float x, float& y0, float& y1) noexcept {
        const int m = (n + 1) / 2;
        int& p = up_pos[ch];
        p = (p == 0 ? m : p) - 1;
        float* b = ub[ch];
        b[p] = x;
        if (p + m < kMaxTaps) b[p + m] = x;
        float a0 = 0.0f, a1 = 0.0f;
        for (int k = 0; 2 * k < n; ++k) a0 += h[2 * k] * b[p + k];
        for (int k = 0; 2 * k + 1 < n; ++k) a1 += h[2 * k + 1] * b[p + k];
        y0 = 2.0f * a0;
        y1 = 2.0f * a1;
    }
    void push(int ch, float u) noexcept {
        int& p = dn_pos[ch];
        p = (p == 0 ? n : p) - 1;
        db[ch][p] = u;
        db[ch][p + n] = u;
    }
    float out(int ch) const noexcept {
        const float* b = db[ch] + dn_pos[ch];
        float a = 0.0f;
        for (int j = 0; j < nnz; ++j) a += h[nz[j]] * b[nz[j]];
        return a;
    }
    // Two high-rate samples -> one base sample (aligned with up()).
    float down(int ch, float u0, float u1) noexcept {
        push(ch, u0);
        const float y = out(ch);
        push(ch, u1);
        return y;
    }
};

constexpr int kStage1Taps = 47; // delay 23 base samples (up + down)
constexpr int kStage2Taps = 17; // delay 8 samples at 2x = 4 base samples
constexpr uint32_t kLatency = 27;
constexpr int kPad2x = 4; // 2x mode is padded to the 4x delay

class CharacterEq final : public BuiltinProcessor {
public:
    void prepare(double sample_rate, uint32_t) override {
        fs_ = sample_rate;
        glide_ = static_cast<float>(std::exp(-static_cast<double>(kControl) / (0.02 * fs_)));
        stage1_.design(kStage1Taps);
        stage2_.design(kStage2Taps);
        output_.set_time(20.0f, fs_);
        auto_.set_time(60.0f, fs_);
        wet_.set_time(15.0f, fs_);
        dc_a_ = static_cast<float>(std::exp(-2.0 * eq::kPi * 8.0 / fs_));
        tame_att_ = dsp::pole(5.0f, fs_);
        tame_rel_ = dsp::pole(80.0f, fs_);
        dyn_avg_ = static_cast<float>(std::exp(-static_cast<double>(kControl) / (1.5 * fs_)));
        reset();
    }

    void reset() noexcept override {
        for (auto& b : bands_) b.reset();
        for (auto& a : adaa_) a.clear();
        for (auto& d : det_) d = Detector{};
        for (auto& t : tames_) t = Tamer{};
        warm_st_.clear();
        listen_st_.clear();
        stage1_.clear();
        stage2_.clear();
        std::memset(delay_, 0, sizeof delay_);
        std::memset(pad_, 0, sizeof pad_);
        dc_x_[0] = dc_x_[1] = dc_y_[0] = dc_y_[1] = 0.0f;
        delay_pos_ = pad_pos_ = 0;
        valve_live_ = false;
        primed_ = false;
        std::memset(last_bands_, 0xff, sizeof last_bands_);
    }

    uint32_t latency() const noexcept override { return kLatency; }

    void process(float* l, float* r, uint32_t n, const float* p, const BuiltinContext&, float* ro) noexcept override {
        const Voicing& v = kBanks[choice(p[kBank], 10)];
        const float out_gain = dsp::db_to_gain(std::clamp(p[kOutput], -48.0f, 48.0f));
        const bool bypass = p[kBypass] >= 0.5f;
        const int listen = choice(p[kListen], kBands + 1) - 1;

        // Band targets (voiced by the bank).
        for (int b = 0; b < kBands; ++b) {
            const float* bp = p + 9 * b;
            eq::BandRunner& br = bands_[b];
            eq::Band t;
            t.shape = kShapes[choice(bp[kType], 7)];
            t.freq = std::clamp(bp[kFreq], 10.0f, 24000.0f);
            t.gain_db = std::clamp(bp[kGain], -24.0f, 24.0f);
            t.q = std::clamp(bp[kQ], 0.05f, 24.0f) * v.q_scale;
            t.slope_db = kSlopes[choice(bp[kSlope], 6)];
            t.prop_q = bp[kPropQ] >= 0.5f || (v.prop_q && t.shape == eq::Shape::Bell);
            t.overshoot = std::clamp(bp[kOvershoot], 0.0f, 100.0f) * 0.01f;
            if (t.shape == eq::Shape::LowShelf || t.shape == eq::Shape::HighShelf || t.shape == eq::Shape::HighPass ||
                t.shape == eq::Shape::LowPass)
                t.overshoot = std::min(1.0f, t.overshoot + v.overshoot);
            t.low_atten_db = std::clamp(bp[kLowAtten], 0.0f, 18.0f);
            br.target = t;
            br.on = bp[kOn] >= 0.5f;
            const float* dp = p + kDyn0 + 6 * b;
            Detector& d = det_[b];
            d.on = br.on && dp[0] >= 0.5f;
            d.range = std::clamp(dp[1], -24.0f, 24.0f);
            d.automatic = dp[2] >= 0.5f;
            d.thresh = std::clamp(dp[3], -96.0f, 0.0f);
            d.att = dsp::pole(std::clamp(dp[4], 0.05f, 5000.0f), fs_);
            d.rel = dsp::pole(std::clamp(dp[5], 1.0f, 20000.0f), fs_);
            drive_[b] = std::clamp(p[kDrive0 + b], 0.0f, 100.0f) * 0.01f;
        }
        asym_ = 0.5f * v.asymmetry;

        // Auto gain: recomputed only when a band setting changed.
        const float auto_target = p[kAutoGain] >= 0.5f ? auto_gain_db(p) : 0.0f;
        const float auto_gain = dsp::db_to_gain(auto_target);

        // Tamer.
        const bool tame_on = p[kTameOn] >= 0.5f;
        const float tame_amount = std::clamp(p[kTameAmount], 0.0f, 100.0f) * 0.01f;
        for (int t = 0; t < kTames; ++t) {
            const float* tp = p + kTame0 + 4 * t;
            Tamer& tm = tames_[t];
            tm.on = tame_on && tp[0] >= 0.5f;
            tm.freq = std::clamp(tp[1], 10.0f, 24000.0f);
            tm.q = std::clamp(tp[2], 0.1f, 24.0f);
            tm.max_cut = 6.0f + std::fabs(std::clamp(tp[3], -48.0f, 0.0f));
        }

        // Warmth.
        const float warmth = std::clamp(p[kWarmth], 0.0f, 100.0f) * 0.01f;
        if (warmth > 0.0f) {
            const double tail = static_cast<double>(std::clamp(p[kWarmthTail], 20.0f, 4000.0f));
            warm_c_ = p[kWarmthSlope] >= 0.5f ? eq::two(eq::prewarp(tail, fs_), 1.41421356, 0.0, 0.0, 1.0)
                                              : eq::one(eq::prewarp(tail, fs_), 0.0, 1.0);
            warm_c_.finish();
        }

        // Valve.
        const bool four = p[kOversampling] >= 0.5f;
        const float drive_v = std::clamp(p[kValveTrim], 0.0f, 4.0f) * 1.5f;
        const float wet_target = bypass ? 0.0f : (p[kValveOn] >= 0.5f ? std::clamp(p[kValveMix], 0.0f, 100.0f) * 0.01f : 0.0f);

        if (listen >= 0) {
            const eq::Band& b = bands_[listen].primed ? bands_[listen].cur : bands_[listen].target;
            listen_c_ = eq::band_pass(b.freq, std::clamp(b.q, 0.5f, 8.0f), fs_);
            listen_c_.finish();
        }

        if (!primed_) {
            output_.snap(out_gain);
            auto_.snap(auto_gain);
            wet_.snap(wet_target);
            primed_ = true;
        }

        float dyn_peak[kBands] = {};
        float tame_peak[kTames] = {};
        for (uint32_t pos = 0; pos < n; pos += kControl) {
            const uint32_t len = std::min(kControl, n - pos);
            float* cl = l + pos;
            float* cr = r + pos;
            float in_l[kControl], in_r[kControl];
            std::memcpy(in_l, cl, 4 * len);
            std::memcpy(in_r, cr, 4 * len);

            if (!bypass) {
                // Dynamic bands: detect on the input, then move the band gains.
                // Undriven bands run as one cascade; a driven band flushes it.
                eq::Cascade chain;
                for (int b = 0; b < kBands; ++b) {
                    eq::BandRunner& br = bands_[b];
                    br.glide(glide_);
                    Detector& d = det_[b];
                    float offset = 0.0f;
                    if (d.on) {
                        d.coef = eq::band_pass(br.cur.freq, std::clamp(br.cur.q, 0.3f, 8.0f), fs_);
                        d.coef.finish();
                        float env = d.env;
                        for (uint32_t i = 0; i < len; ++i) {
                            const float s = std::max(std::fabs(eq::tick(d.coef, d.st, 0, in_l[i])), std::fabs(eq::tick(d.coef, d.st, 1, in_r[i])));
                            env = s + (s > env ? d.att : d.rel) * (env - s);
                        }
                        d.env = env < 1e-12f ? 0.0f : env;
                        const float lvl = dsp::gain_to_db(1.41421356f * d.env);
                        if (lvl > -80.0f) d.avg = lvl + dyn_avg_ * (d.avg - lvl);
                        const float thr = d.automatic ? d.avg : d.thresh;
                        const float amount = std::clamp((lvl - thr) / (d.automatic ? 6.0f : 12.0f), 0.0f, 1.0f);
                        offset = d.range * amount;
                        eq::sanitize(d.st);
                    } else {
                        d.env = 0.0f;
                    }
                    if (br.dirty || std::fabs(offset - d.applied) > 0.02f) {
                        br.redesign(fs_, offset);
                        d.applied = offset;
                    }
                    if (std::fabs(offset) > std::fabs(dyn_peak[b])) dyn_peak[b] = offset;

                    if (br.d.n == 0) continue;
                    const float dr = drive_[b];
                    if (dr <= 0.0f) {
                        br.add_to(chain);
                        continue;
                    }
                    chain.run(cl, cr, len);
                    chain.clear();
                    // Drive: saturate only what the band adds.
                    float xl[kControl], xr[kControl];
                    std::memcpy(xl, cl, 4 * len);
                    std::memcpy(xr, cr, 4 * len);
                    br.run_block(cl, cr, len);
                    const float k = 1.0f + 7.0f * dr * dr;
                    const float tb = std::tanh(asym_);
                    const float norm = 1.0f / (k * (1.0f - tb * tb));
                    for (uint32_t i = 0; i < len; ++i) {
                        cl[i] = xl[i] + norm * adaa_[b].run(0, k * (cl[i] - xl[i]), asym_, tb);
                        cr[i] = xr[i] + norm * adaa_[b].run(1, k * (cr[i] - xr[i]), asym_, tb);
                    }
                }
                chain.run(cl, cr, len);

                // Tamer: detect on the EQ'd signal, cut what pokes out.
                for (int t = 0; t < kTames; ++t) {
                    Tamer& tm = tames_[t];
                    float target = 0.0f;
                    if (tm.on) {
                        const eq::Coef nb = finished(eq::band_pass(tm.freq, tm.q, fs_));
                        const eq::Coef wb = finished(eq::band_pass(tm.freq, std::max(0.1f, tm.q * 0.25f), fs_));
                        float pn = tm.pn, pw = tm.pw;
                        for (uint32_t i = 0; i < len; ++i) {
                            const float x = 0.5f * (cl[i] + cr[i]);
                            const float a = eq::tick(nb, tm.st, 0, x);
                            const float b = eq::tick(wb, tm.st, 1, x);
                            const float a2 = a * a, b2 = b * b;
                            pn = a2 + (a2 > pn ? tame_att_ : tame_rel_) * (pn - a2);
                            pw = b2 + (b2 > pw ? tame_att_ : tame_rel_) * (pw - b2);
                        }
                        tm.pn = pn < 1e-20f ? 0.0f : pn;
                        tm.pw = pw < 1e-20f ? 0.0f : pw;
                        eq::sanitize(tm.st);
                        if (tm.pw > 1e-10f) {
                            // Narrow band against the rest of the neighbourhood (a flat
                            // spectrum puts a third of the narrow band's power there: -4.8 dB).
                            const float rest = std::max(tm.pw - tm.pn, 1e-3f * tm.pw);
                            const float excess = 10.0f * std::log10(std::max(tm.pn, 1e-20f) / rest) + 4.77f;
                            target = -std::clamp((excess - 3.0f) * tame_amount, 0.0f, tm.max_cut);
                        }
                    }
                    tm.cut = target + 0.5f * (tm.cut - target);
                    if (std::fabs(tm.cut) < 0.01f && target == 0.0f) tm.cut = 0.0f;
                    if (tm.cut < -0.01f) {
                        if (std::fabs(tm.cut - tm.applied) > 0.02f || tm.freq != tm.applied_freq || tm.q != tm.applied_q) {
                            tm.c = finished(eq::bell(static_cast<double>(tm.freq), static_cast<double>(tm.cut), static_cast<double>(tm.q), fs_));
                            tm.applied = tm.cut;
                            tm.applied_freq = tm.freq;
                            tm.applied_q = tm.q;
                        }
                        eq::run(tm.c, tm.cst, cl, cr, len);
                        eq::sanitize(tm.cst);
                    } else {
                        tm.cst.clear();
                        tm.applied = 0.0f;
                    }
                    tame_peak[t] = std::min(tame_peak[t], tm.cut);
                }

                // Warmth: saturate the lows only.
                if (warmth > 0.0f) {
                    const float k = 1.0f + 3.0f * warmth;
                    const float b = 0.2f, tb = std::tanh(b);
                    const float norm = 1.0f / (k * (1.0f - tb * tb));
                    for (uint32_t i = 0; i < len; ++i) {
                        const float lo_l = eq::tick(warm_c_, warm_st_, 0, cl[i]);
                        const float lo_r = eq::tick(warm_c_, warm_st_, 1, cr[i]);
                        cl[i] += warmth * (norm * (std::tanh(k * lo_l + b) - tb) - lo_l);
                        cr[i] += warmth * (norm * (std::tanh(k * lo_r + b) - tb) - lo_r);
                    }
                    eq::sanitize(warm_st_);
                } else {
                    warm_st_.clear();
                }

                if (listen >= 0) {
                    std::memcpy(cl, in_l, 4 * len);
                    std::memcpy(cr, in_r, 4 * len);
                    eq::run(listen_c_, listen_st_, cl, cr, len);
                    eq::sanitize(listen_st_);
                }
            } else {
                std::memcpy(cl, in_l, 4 * len);
                std::memcpy(cr, in_r, 4 * len);
            }

            // Latency-aligned dry path and the oversampled valve.
            for (uint32_t i = 0; i < len; ++i) {
                const float w = wet_.next(wet_target);
                const float dry_l = delayed(0, cl[i]);
                const float dry_r = delayed(1, cr[i]);
                if (w > 1e-5f || wet_target > 0.0f) {
                    if (!valve_live_) {
                        stage1_.clear();
                        stage2_.clear();
                        std::memset(pad_, 0, sizeof pad_);
                        dc_x_[0] = dc_x_[1] = dc_y_[0] = dc_y_[1] = 0.0f;
                        valve_live_ = true;
                    }
                    const float vl = valve(0, cl[i], four, drive_v);
                    const float vr = valve(1, cr[i], four, drive_v);
                    advance_pad();
                    cl[i] = dry_l + w * (vl - dry_l);
                    cr[i] = dry_r + w * (vr - dry_r);
                } else {
                    valve_live_ = false;
                    cl[i] = dry_l;
                    cr[i] = dry_r;
                }
                advance_delay();
                const float g = bypass ? 1.0f : output_.next(out_gain) * auto_.next(auto_gain);
                cl[i] *= g;
                cr[i] *= g;
            }
        }
        for (auto& b : bands_) b.sanitize_state();
        for (int b = 0; b < kBands; ++b) ro[b] = dyn_peak[b];
        for (int t = 0; t < kTames; ++t) ro[kBands + t] = tame_peak[t];
    }

private:
    struct Detector {
        bool on = false, automatic = true;
        float range = 0.0f, thresh = -30.0f, att = 0.0f, rel = 0.0f;
        float env = 0.0f, avg = -40.0f, applied = 0.0f;
        eq::Coef coef{};
        eq::State st{};
    };
    struct Tamer {
        bool on = false;
        float freq = 1000.0f, q = 1.0f, max_cut = 6.0f;
        float pn = 0.0f, pw = 0.0f, cut = 0.0f, applied = 0.0f, applied_freq = 0.0f, applied_q = 0.0f;
        eq::State st{};
        eq::Coef c{};
        eq::State cst{};
    };

    static eq::Coef finished(eq::Coef c) noexcept {
        c.finish();
        return c;
    }

    float delayed(int ch, float x) noexcept {
        float* d = delay_[ch];
        const float y = d[delay_pos_];
        d[delay_pos_] = x;
        return y;
    }
    void advance_delay() noexcept { delay_pos_ = (delay_pos_ + 1) % kLatency; }
    void advance_pad() noexcept { pad_pos_ = (pad_pos_ + 1) % kPad2x; }

    // Triode-style asymmetric curve with unity small-signal gain.
    static float tube(float u, float drive) noexcept {
        constexpr float b = 0.25f;
        constexpr float tb = 0.24491866f;   // tanh(b)
        constexpr float slope = 0.94001485f; // 1 - tanh(b)^2
        if (drive < 1e-3f) return u;
        return (fast_tanh(drive * u + b) - tb) / (drive * slope);
    }

    float valve(int ch, float x, bool four, float drive) noexcept {
        float y;
        if (!four) {
            float a, b;
            stage1_.up(ch, x, a, b);
            y = stage1_.down(ch, tube(a, drive), tube(b, drive));
        } else {
            float a, b, a0, a1, b0, b1;
            stage1_.up(ch, x, a, b);
            stage2_.up(ch, a, a0, a1);
            stage2_.up(ch, b, b0, b1);
            const float ya = stage2_.down(ch, tube(a0, drive), tube(a1, drive));
            const float yb = stage2_.down(ch, tube(b0, drive), tube(b1, drive));
            y = stage1_.down(ch, ya, yb);
        }
        if (!four) { // pad the 2x path to the 4x delay
            float& slot = pad_[ch][pad_pos_];
            const float o = slot;
            slot = y;
            y = o;
        }
        // DC blocker (the asymmetric curve adds an offset).
        const float o = y - dc_x_[ch] + dc_a_ * dc_y_[ch];
        dc_x_[ch] = y;
        dc_y_[ch] = std::fabs(o) < 1e-20f ? 0.0f : o;
        return o;
    }

    // Minus the mean dB of the static curve over 32 log-spaced points (20 Hz..20 kHz).
    float auto_gain_db(const float* p) noexcept {
        float key[9 * kBands + 1];
        std::memcpy(key, p, sizeof(float) * 9 * kBands);
        key[9 * kBands] = p[kBank];
        if (std::memcmp(key, last_bands_, sizeof key) == 0) return last_auto_;
        std::memcpy(last_bands_, key, sizeof key);
        double sum = 0.0;
        for (int i = 0; i < 32; ++i) {
            const double f = 20.0 * std::pow(1000.0, i / 31.0);
            double m2 = 1.0;
            for (int b = 0; b < kBands; ++b) {
                if (!bands_[b].on) continue;
                const eq::Design d = eq::design(bands_[b].target, fs_);
                for (int s = 0; s < d.n; ++s) m2 *= eq::mag2(d.s[s], f, fs_);
            }
            sum += 10.0 * std::log10(std::max(m2, 1e-24));
        }
        last_auto_ = static_cast<float>(std::clamp(-sum / 32.0, -24.0, 24.0));
        return last_auto_;
    }

    double fs_ = 48000.0;
    float glide_ = 0.0f;
    eq::BandRunner bands_[kBands];
    Adaa adaa_[kBands];
    float drive_[kBands] = {};
    float asym_ = 0.0f;
    Detector det_[kBands];
    float dyn_avg_ = 0.0f;
    Tamer tames_[kTames];
    float tame_att_ = 0.0f, tame_rel_ = 0.0f;
    eq::Coef warm_c_{};
    eq::State warm_st_{};
    eq::Coef listen_c_{};
    eq::State listen_st_{};
    Halfband stage1_, stage2_;
    float delay_[2][kLatency] = {};
    uint32_t delay_pos_ = 0;
    float pad_[2][kPad2x] = {};
    int pad_pos_ = 0;
    float dc_x_[2] = {}, dc_y_[2] = {}, dc_a_ = 0.0f;
    bool valve_live_ = false;
    dsp::Smoother output_, auto_, wet_;
    float last_bands_[9 * kBands + 1] = {};
    float last_auto_ = 0.0f;
    bool primed_ = false;
};

std::unique_ptr<BuiltinProcessor> create_character_eq() { return std::make_unique<CharacterEq>(); }

} // namespace

extern const BuiltinDescriptor kCharacterEqDescriptor;
const BuiltinDescriptor kCharacterEqDescriptor{"character-eq", kParamIds, kReadoutIds, &create_character_eq, {}};

} // namespace saine
