// Shaper: seven tempo-synced curve lanes. Parameters in the order of
// saine_core::builtin::shaper (checked by a Rust test): 11 globals, then per
// lane (Volume, Time, Width, Pan, Filter, Crush, Pitch) on, rate, depth,
// band, split, points and 16 x (x, y, bend).
//
// Curves: points (x, y) with an exponential bend per segment, evaluated the
// same way as the panel draws them (saine_core::builtin::shaper::segment).
// Depth scales the curve around the lane's rest value: v = rest + depth *
// (y - rest), so Depth 0 is neutral for every lane.
//
// Cycle position: Trigger = Sync follows the song position (free-running
// from where the transport stopped while it is stopped); Trigger = Audio
// restarts every lane when the peak level of the input (or the sidechain,
// when one is picked) rises through Threshold, runs one cycle and holds the
// curve's end value until the next trigger.
//
// Lanes, in processing order:
//   Time   reads the lane's input v cycles in the past (Hermite interpolation
//          from a ring buffer; a jump in the curve crossfades over 5 ms).
//   Pitch  granular pitch shifter, +-12 semitones at full depth: grains of
//          24 ms (Hann, 50 % overlap) read the input at the pitch ratio; each
//          new grain starts where the input best matches the grain it takes
//          over from (normalised cross-correlation within +-8 ms), so
//          tones keep their exact pitch and 0 semitones is the plain signal.
//          It adds 20 ms of latency, reported for delay compensation while
//          the lane is on.
//   Filter state-variable filter (low-, band- or high-pass, Resonance as Q),
//          cutoff = Low * (High / Low)^v.
//   Crush  bit depth from 16 down to Bits and sample-rate division from 1 up
//          to Decimate as v goes from 0 to 1.
//   Width  side gain 2v (0 = mono, 0.5 = unchanged, 1 = twice as wide).
//   Pan    constant-power balance, 2v - 1 from left to right.
//   Volume gain v.
// Band: a lane set to Lows or Highs works on that side of a Linkwitz-Riley
// (24 dB/oct) split at its Split frequency; the other side passes (summed
// the two sides are flat in level).
// Output: dry (delayed to match) / wet Mix, then Output gain.
#include "builtin.hpp"
#include "dsp.hpp"

#include <algorithm>
#include <array>
#include <cmath>
#include <cstring>
#include <memory>
#include <vector>

namespace saine {

namespace {

constexpr int kLanes = 7;
constexpr int kMaxPoints = 16;
constexpr int kGlobals = 11;
constexpr int kStride = 54;
enum Global : uint32_t {
    kTrigger,
    kThreshold,
    kMix,
    kOutput,
    kSidechain,
    kFilterType,
    kFilterResonance,
    kFilterLow,
    kFilterHigh,
    kCrushBits,
    kCrushDecimate
};
enum Lane : int { kVolume, kTime, kWidth, kPan, kFilter, kCrush, kPitch };
enum LaneParam : int { kOn, kRate, kDepth, kBand, kSplit, kPoints, kPoint0 };
constexpr float kRest[kLanes] = {1.0f, 0.0f, 0.5f, 0.5f, 1.0f, 0.0f, 0.5f};
// Processing order (see the header comment).
constexpr int kOrder[kLanes] = {kTime, kPitch, kFilter, kCrush, kWidth, kPan, kVolume};
constexpr double kDivisionQuarters[] = {32.0, 16.0, 8.0, 4.0, 3.0, 2.0, 4.0 / 3.0, 1.5, 1.0, 2.0 / 3.0,
                                        0.75, 0.5, 1.0 / 3.0, 0.375, 0.25, 1.0 / 6.0, 0.125, 0.09375, 0.0625, 1.0 / 24.0};
constexpr int kDivisions = static_cast<int>(sizeof kDivisionQuarters / sizeof kDivisionQuarters[0]);

constexpr uint32_t kChunk = 16;         // control rate of the curve values (samples)
constexpr double kPitchHopMs = 12.0;    // pitch shifter: grain = 2 hops
constexpr double kPitchSearchMs = 8.0;  // pitch shifter: splice search range (+-)
constexpr double kTimeBufferSeconds = 20.0;
constexpr float kXfadeMs = 5.0f;
constexpr double kPi = 3.14159265358979323846;

// Parameter ids in definition order.
constexpr uint32_t kParamCount = kGlobals + kLanes * kStride; // 389
constexpr const char* kParamIds[kParamCount] = {
    "trigger", "threshold", "mix", "output", "sidechain", "filter_type", "filter_resonance", "filter_low",
    "filter_high", "crush_bits", "crush_decimate", "vol_on", "vol_rate", "vol_depth", "vol_band", "vol_split",
    "vol_points", "vol_p1_x", "vol_p1_y", "vol_p1_bend", "vol_p2_x", "vol_p2_y", "vol_p2_bend", "vol_p3_x", "vol_p3_y",
    "vol_p3_bend", "vol_p4_x", "vol_p4_y", "vol_p4_bend", "vol_p5_x", "vol_p5_y", "vol_p5_bend", "vol_p6_x", "vol_p6_y",
    "vol_p6_bend", "vol_p7_x", "vol_p7_y", "vol_p7_bend", "vol_p8_x", "vol_p8_y", "vol_p8_bend", "vol_p9_x", "vol_p9_y",
    "vol_p9_bend", "vol_p10_x", "vol_p10_y", "vol_p10_bend", "vol_p11_x", "vol_p11_y", "vol_p11_bend", "vol_p12_x",
    "vol_p12_y", "vol_p12_bend", "vol_p13_x", "vol_p13_y", "vol_p13_bend", "vol_p14_x", "vol_p14_y", "vol_p14_bend",
    "vol_p15_x", "vol_p15_y", "vol_p15_bend", "vol_p16_x", "vol_p16_y", "vol_p16_bend", "time_on", "time_rate",
    "time_depth", "time_band", "time_split", "time_points", "time_p1_x", "time_p1_y", "time_p1_bend", "time_p2_x",
    "time_p2_y", "time_p2_bend", "time_p3_x", "time_p3_y", "time_p3_bend", "time_p4_x", "time_p4_y", "time_p4_bend",
    "time_p5_x", "time_p5_y", "time_p5_bend", "time_p6_x", "time_p6_y", "time_p6_bend", "time_p7_x", "time_p7_y",
    "time_p7_bend", "time_p8_x", "time_p8_y", "time_p8_bend", "time_p9_x", "time_p9_y", "time_p9_bend", "time_p10_x",
    "time_p10_y", "time_p10_bend", "time_p11_x", "time_p11_y", "time_p11_bend", "time_p12_x", "time_p12_y",
    "time_p12_bend", "time_p13_x", "time_p13_y", "time_p13_bend", "time_p14_x", "time_p14_y", "time_p14_bend",
    "time_p15_x", "time_p15_y", "time_p15_bend", "time_p16_x", "time_p16_y", "time_p16_bend", "width_on", "width_rate",
    "width_depth", "width_band", "width_split", "width_points", "width_p1_x", "width_p1_y", "width_p1_bend",
    "width_p2_x", "width_p2_y", "width_p2_bend", "width_p3_x", "width_p3_y", "width_p3_bend", "width_p4_x",
    "width_p4_y", "width_p4_bend", "width_p5_x", "width_p5_y", "width_p5_bend", "width_p6_x", "width_p6_y",
    "width_p6_bend", "width_p7_x", "width_p7_y", "width_p7_bend", "width_p8_x", "width_p8_y", "width_p8_bend",
    "width_p9_x", "width_p9_y", "width_p9_bend", "width_p10_x", "width_p10_y", "width_p10_bend", "width_p11_x",
    "width_p11_y", "width_p11_bend", "width_p12_x", "width_p12_y", "width_p12_bend", "width_p13_x", "width_p13_y",
    "width_p13_bend", "width_p14_x", "width_p14_y", "width_p14_bend", "width_p15_x", "width_p15_y", "width_p15_bend",
    "width_p16_x", "width_p16_y", "width_p16_bend", "pan_on", "pan_rate", "pan_depth", "pan_band", "pan_split",
    "pan_points", "pan_p1_x", "pan_p1_y", "pan_p1_bend", "pan_p2_x", "pan_p2_y", "pan_p2_bend", "pan_p3_x", "pan_p3_y",
    "pan_p3_bend", "pan_p4_x", "pan_p4_y", "pan_p4_bend", "pan_p5_x", "pan_p5_y", "pan_p5_bend", "pan_p6_x", "pan_p6_y",
    "pan_p6_bend", "pan_p7_x", "pan_p7_y", "pan_p7_bend", "pan_p8_x", "pan_p8_y", "pan_p8_bend", "pan_p9_x", "pan_p9_y",
    "pan_p9_bend", "pan_p10_x", "pan_p10_y", "pan_p10_bend", "pan_p11_x", "pan_p11_y", "pan_p11_bend", "pan_p12_x",
    "pan_p12_y", "pan_p12_bend", "pan_p13_x", "pan_p13_y", "pan_p13_bend", "pan_p14_x", "pan_p14_y", "pan_p14_bend",
    "pan_p15_x", "pan_p15_y", "pan_p15_bend", "pan_p16_x", "pan_p16_y", "pan_p16_bend", "filter_on", "filter_rate",
    "filter_depth", "filter_band", "filter_split", "filter_points", "filter_p1_x", "filter_p1_y", "filter_p1_bend",
    "filter_p2_x", "filter_p2_y", "filter_p2_bend", "filter_p3_x", "filter_p3_y", "filter_p3_bend", "filter_p4_x",
    "filter_p4_y", "filter_p4_bend", "filter_p5_x", "filter_p5_y", "filter_p5_bend", "filter_p6_x", "filter_p6_y",
    "filter_p6_bend", "filter_p7_x", "filter_p7_y", "filter_p7_bend", "filter_p8_x", "filter_p8_y", "filter_p8_bend",
    "filter_p9_x", "filter_p9_y", "filter_p9_bend", "filter_p10_x", "filter_p10_y", "filter_p10_bend", "filter_p11_x",
    "filter_p11_y", "filter_p11_bend", "filter_p12_x", "filter_p12_y", "filter_p12_bend", "filter_p13_x",
    "filter_p13_y", "filter_p13_bend", "filter_p14_x", "filter_p14_y", "filter_p14_bend", "filter_p15_x",
    "filter_p15_y", "filter_p15_bend", "filter_p16_x", "filter_p16_y", "filter_p16_bend", "crush_on", "crush_rate",
    "crush_depth", "crush_band", "crush_split", "crush_points", "crush_p1_x", "crush_p1_y", "crush_p1_bend",
    "crush_p2_x", "crush_p2_y", "crush_p2_bend", "crush_p3_x", "crush_p3_y", "crush_p3_bend", "crush_p4_x",
    "crush_p4_y", "crush_p4_bend", "crush_p5_x", "crush_p5_y", "crush_p5_bend", "crush_p6_x", "crush_p6_y",
    "crush_p6_bend", "crush_p7_x", "crush_p7_y", "crush_p7_bend", "crush_p8_x", "crush_p8_y", "crush_p8_bend",
    "crush_p9_x", "crush_p9_y", "crush_p9_bend", "crush_p10_x", "crush_p10_y", "crush_p10_bend", "crush_p11_x",
    "crush_p11_y", "crush_p11_bend", "crush_p12_x", "crush_p12_y", "crush_p12_bend", "crush_p13_x", "crush_p13_y",
    "crush_p13_bend", "crush_p14_x", "crush_p14_y", "crush_p14_bend", "crush_p15_x", "crush_p15_y", "crush_p15_bend",
    "crush_p16_x", "crush_p16_y", "crush_p16_bend", "pitch_on", "pitch_rate", "pitch_depth", "pitch_band",
    "pitch_split", "pitch_points", "pitch_p1_x", "pitch_p1_y", "pitch_p1_bend", "pitch_p2_x", "pitch_p2_y",
    "pitch_p2_bend", "pitch_p3_x", "pitch_p3_y", "pitch_p3_bend", "pitch_p4_x", "pitch_p4_y", "pitch_p4_bend",
    "pitch_p5_x", "pitch_p5_y", "pitch_p5_bend", "pitch_p6_x", "pitch_p6_y", "pitch_p6_bend", "pitch_p7_x",
    "pitch_p7_y", "pitch_p7_bend", "pitch_p8_x", "pitch_p8_y", "pitch_p8_bend", "pitch_p9_x", "pitch_p9_y",
    "pitch_p9_bend", "pitch_p10_x", "pitch_p10_y", "pitch_p10_bend", "pitch_p11_x", "pitch_p11_y", "pitch_p11_bend",
    "pitch_p12_x", "pitch_p12_y", "pitch_p12_bend", "pitch_p13_x", "pitch_p13_y", "pitch_p13_bend", "pitch_p14_x",
    "pitch_p14_y", "pitch_p14_bend", "pitch_p15_x", "pitch_p15_y", "pitch_p15_bend", "pitch_p16_x", "pitch_p16_y",
    "pitch_p16_bend",
};
static_assert(sizeof kParamIds / sizeof kParamIds[0] == 389);

// One lane's shape: points with precomputed segment constants.
struct Curve {
    int n = 2;
    float x[kMaxPoints]{};
    float y[kMaxPoints]{};
    float k[kMaxPoints]{};   // signed bend exponent of the segment starting at the point
    float inv[kMaxPoints]{}; // 1 / (1 - e^-k), or 0 for a straight segment
    float span_inv[kMaxPoints]{};

    void load(const float* lp, int lane) noexcept {
        n = std::clamp(static_cast<int>(std::lround(lp[kPoints])), 2, kMaxPoints);
        float prev = 0.0f;
        for (int i = 0; i < n; ++i) {
            const float* q = lp + kPoint0 + 3 * i;
            float xi = i == 0 ? 0.0f : i == n - 1 ? 1.0f : std::clamp(q[0], prev, 1.0f);
            if (!(xi >= prev)) xi = prev;
            prev = xi;
            x[i] = xi;
            const float yp = lane == kPitch ? (q[1] + 12.0f) / 24.0f : q[1] * 0.01f;
            y[i] = std::clamp(yp, 0.0f, 1.0f);
            k[i] = std::clamp(q[2] * 0.01f, -1.0f, 1.0f) * 12.0f;
        }
        for (int i = 0; i + 1 < n; ++i) {
            if (y[i + 1] < y[i]) k[i] = -k[i];
            inv[i] = std::fabs(k[i]) < 1e-4f ? 0.0f : 1.0f / (1.0f - std::exp(-k[i]));
            const float span = x[i + 1] - x[i];
            span_inv[i] = span > 1e-6f ? 1.0f / span : 0.0f;
        }
    }

    float eval(float px) const noexcept {
        if (!(px > 0.0f)) return y[0];
        if (px >= 1.0f) return y[n - 1];
        int i = 0;
        while (i + 2 < n && px > x[i + 1]) ++i;
        if (span_inv[i] == 0.0f) return y[i + 1];
        const float t = std::clamp((px - x[i]) * span_inv[i], 0.0f, 1.0f);
        const float f = inv[i] == 0.0f ? t : (1.0f - std::exp(-k[i] * t)) * inv[i];
        return y[i] + (y[i + 1] - y[i]) * f;
    }
};

// Topology-preserving state-variable filter (one channel).
struct Svf {
    float s1 = 0.0f, s2 = 0.0f;
    // Low-, band- and high-pass outputs for g = tan(pi fc / fs), k = 1 / Q.
    void run(float x, float g, float k, float& lp, float& bp, float& hp) noexcept {
        const float a1 = 1.0f / (1.0f + g * (g + k));
        const float v3 = x - s2;
        const float v1 = a1 * (s1 + g * v3);
        const float v2 = s2 + g * v1;
        s1 = 2.0f * v1 - s1;
        s2 = 2.0f * v2 - s2;
        lp = v2;
        bp = v1;
        hp = x - k * v1 - v2;
    }
    void clear() noexcept { s1 = s2 = 0.0f; }
    void flush() noexcept {
        if (std::fabs(s1) < 1e-20f) s1 = 0.0f;
        if (std::fabs(s2) < 1e-20f) s2 = 0.0f;
    }
};

// Linkwitz-Riley 4th-order split (two Butterworth SVF stages per side).
struct Crossover {
    Svf lp[2], hp[2];
    void split(float x, float g, float& lo, float& hi) noexcept {
        constexpr float k = 1.41421356f;
        float a, b, c;
        lp[0].run(x, g, k, a, b, c);
        lp[1].run(a, g, k, lo, b, c);
        float d, e, f;
        hp[0].run(x, g, k, d, e, f);
        hp[1].run(f, g, k, d, e, hi);
    }
    void clear() noexcept {
        for (Svf& s : lp) s.clear();
        for (Svf& s : hp) s.clear();
    }
    void flush() noexcept {
        for (Svf& s : lp) s.flush();
        for (Svf& s : hp) s.flush();
    }
};

// Stereo ring buffer with a power-of-two length.
struct Ring {
    std::vector<float> l, r;
    uint32_t mask = 0;
    uint32_t w = 0;
    void allocate(uint32_t frames) {
        uint32_t size = 1;
        while (size < frames) size <<= 1;
        l.assign(size, 0.0f);
        r.assign(size, 0.0f);
        mask = size - 1;
        w = 0;
    }
    void clear() noexcept {
        std::fill(l.begin(), l.end(), 0.0f);
        std::fill(r.begin(), r.end(), 0.0f);
        w = 0;
    }
    void push(float a, float b) noexcept {
        w = (w + 1) & mask;
        l[w] = a;
        r[w] = b;
    }
    // Hermite read d samples back (d >= 1; the newest sample is d = 0).
    static float hermite(const std::vector<float>& buf, uint32_t mask, uint32_t w, double d) noexcept {
        const double pos = static_cast<double>(w) - d;
        const double fl = std::floor(pos);
        const auto f = static_cast<float>(pos - fl);
        const auto i = static_cast<uint32_t>(static_cast<int64_t>(fl)) & mask;
        const float xm1 = buf[(i - 1) & mask], x0 = buf[i], x1 = buf[(i + 1) & mask], x2 = buf[(i + 2) & mask];
        const float c1 = 0.5f * (x1 - xm1);
        const float c2 = xm1 - 2.5f * x0 + 2.0f * x1 - 0.5f * x2;
        const float c3 = 0.5f * (x2 - xm1) + 1.5f * (x0 - x1);
        return ((c3 * f + c2) * f + c1) * f + x0;
    }
    float tap(const std::vector<float>& buf, uint32_t d) const noexcept { return buf[(w - d) & mask]; }
};

class Shaper final : public BuiltinProcessor {
public:
    void prepare(double sample_rate, uint32_t) override {
        sr_ = sample_rate;
        hop_ = std::max<uint32_t>(16, static_cast<uint32_t>(std::lround(kPitchHopMs * 0.001 * sr_)));
        search_ = std::max<uint32_t>(8, static_cast<uint32_t>(std::lround(kPitchSearchMs * 0.001 * sr_)));
        pitch_latency_ = hop_ + search_ + 2;
        time_buf_.allocate(static_cast<uint32_t>(std::ceil(kTimeBufferSeconds * sr_)) + 8);
        pitch_buf_.allocate(pitch_latency_ + 3 * hop_ + 2 * search_ + 64);
        pitch_mono_.assign(pitch_buf_.l.size(), 0.0f);
        dry_delay_.allocate(pitch_latency_ + 2);
        rest_delay_.allocate(pitch_latency_ + 2);
        xfade_samples_ = std::max(1.0f, kXfadeMs * 0.001f * static_cast<float>(sr_));
        smooth_a_ = dsp::pole(0.6f, sr_);
        mix_.set_time(20.0f, sr_);
        out_.set_time(20.0f, sr_);
        det_release_ = dsp::pole(30.0f, sr_);
        holdoff_ = static_cast<uint32_t>(0.03 * sr_);
    }

    void reset() noexcept override {
        time_buf_.clear();
        pitch_buf_.clear();
        dry_delay_.clear();
        rest_delay_.clear();
        for (Crossover& c : xo_l_) c.clear();
        for (Crossover& c : xo_r_) c.clear();
        filt_l_.clear();
        filt_r_.clear();
        for (float& v : smooth_) v = 0.0f;
        for (double& ph : audio_phase_) ph = 1.0;
        free_beats_ = 0.0;
        prev_delay_ = 0.0;
        xf_delay_ = 0.0;
        xf_left_ = 0.0f;
        std::fill(pitch_mono_.begin(), pitch_mono_.end(), 0.0f);
        for (Grain& g : grains_) g = Grain{};
        hop_count_ = 0;
        pitch_widx_ = 0;
        hold_l_ = hold_r_ = 0.0f;
        hold_phase_ = 1.0f;
        det_ = 0.0f;
        armed_ = true;
        since_trigger_ = holdoff_;
        filt_g_ = -1.0f;
        primed_ = false;
    }

    uint32_t latency() const noexcept override { return pitch_on_.load(std::memory_order_relaxed) ? pitch_latency_ : 0; }
    uint32_t latency_for(const float* params) const noexcept override {
        return params[kGlobals + kStride * kPitch + kOn] >= 0.5f ? pitch_latency_ : 0;
    }

    void process(float* l, float* r, uint32_t n, const float* p, const BuiltinContext& ctx, float*) noexcept override {
        // ---- parameters
        bool on[kLanes];
        float depth[kLanes];
        double quarters[kLanes];
        int band[kLanes];
        float split_g[kLanes];
        for (int lane = 0; lane < kLanes; ++lane) {
            const float* lp = p + kGlobals + kStride * lane;
            on[lane] = lp[kOn] >= 0.5f;
            depth[lane] = std::clamp(lp[kDepth], 0.0f, 100.0f) * 0.01f;
            quarters[lane] = kDivisionQuarters[std::clamp(static_cast<int>(std::lround(lp[kRate])), 0, kDivisions - 1)];
            band[lane] = std::clamp(static_cast<int>(std::lround(lp[kBand])), 0, 2);
            split_g[lane] = coef(std::clamp(lp[kSplit], 20.0f, 20000.0f));
            if (on[lane]) curve_[lane].load(lp, lane);
        }
        pitch_on_.store(on[kPitch], std::memory_order_relaxed);
        const uint32_t latency = on[kPitch] ? pitch_latency_ : 0;
        const bool audio_trigger = p[kTrigger] >= 0.5f;
        const float threshold = dsp::db_to_gain(std::clamp(p[kThreshold], -96.0f, 0.0f));
        const bool use_sc = p[kSidechain] >= 0.5f && ctx.sc_l != nullptr;
        const float mix = std::clamp(p[kMix], 0.0f, 100.0f) * 0.01f;
        const float out_gain = dsp::db_to_gain(std::clamp(p[kOutput], -48.0f, 24.0f));
        const int ftype = std::clamp(static_cast<int>(std::lround(p[kFilterType])), 0, 2);
        const float fk = 1.0f / std::clamp(p[kFilterResonance], 0.5f, 12.0f);
        const float flow = std::clamp(p[kFilterLow], 20.0f, 20000.0f);
        const float fhigh = std::clamp(p[kFilterHigh], 20.0f, 20000.0f);
        const float bits = std::clamp(p[kCrushBits], 1.0f, 16.0f);
        const float decim = std::clamp(p[kCrushDecimate], 1.0f, 32.0f);
        if (!primed_) {
            mix_.snap(mix);
            out_.snap(out_gain);
            for (int lane = 0; lane < kLanes; ++lane) smooth_[lane] = kRest[lane];
            primed_ = true;
        }

        const RtTransport* tr = ctx.transport;
        const double tempo = tr != nullptr ? std::clamp(tr->tempo_bpm, 1.0, 999.0) : 120.0;
        const bool playing = tr != nullptr && tr->playing;
        const double beats_per_sample = tempo / 60.0 / sr_;
        // ctx.offset: frames from the transport position to this segment's first frame.
        const double now = playing ? tr->beats() + static_cast<double>(ctx.offset) * beats_per_sample : free_beats_;
        // The output is heard latency samples late: run the curves for that time.
        const double beat0 = now - static_cast<double>(latency) * beats_per_sample;
        free_beats_ = now + static_cast<double>(n) * beats_per_sample;
        const double samples_per_quarter = 60.0 / tempo * sr_;
        const Shared sh{ftype, fk, flow, fhigh, bits, decim, samples_per_quarter};

        for (uint32_t c0 = 0; c0 < n; c0 += kChunk) {
            const uint32_t m = std::min(kChunk, n - c0);
            float* cl = l + c0;
            float* cr = r + c0;
            // Dry copy (delayed by the reported latency) for Mix.
            float dry_l[kChunk], dry_r[kChunk];
            for (uint32_t i = 0; i < m; ++i) {
                dry_delay_.push(cl[i], cr[i]);
                dry_l[i] = dry_delay_.tap(dry_delay_.l, latency);
                dry_r[i] = dry_delay_.tap(dry_delay_.r, latency);
            }

            // Audio trigger: peak detector on the input or the sidechain.
            if (audio_trigger) {
                const float* sl = use_sc ? ctx.sc_l + c0 : cl;
                const float* sr = use_sc ? ctx.sc_r + c0 : cr;
                bool fire = false;
                for (uint32_t i = 0; i < m; ++i) {
                    const float x = std::max(std::fabs(sl[i]), std::fabs(sr[i]));
                    det_ = x > det_ ? x : x + det_release_ * (det_ - x);
                    if (since_trigger_ < holdoff_) ++since_trigger_;
                    if (armed_ && det_ >= threshold && since_trigger_ >= holdoff_) {
                        fire = true;
                        armed_ = false;
                        since_trigger_ = 0;
                    } else if (!armed_ && det_ < threshold * 0.7079f) { // re-arm 3 dB below
                        armed_ = true;
                    }
                }
                if (det_ < 1e-20f) det_ = 0.0f;
                if (fire)
                    for (double& ph : audio_phase_) ph = 0.0;
            } else {
                for (double& ph : audio_phase_) ph = 1.0;
            }

            // Curve values at the chunk's start and end per lane.
            const double b_start = beat0 + static_cast<double>(c0) * beats_per_sample;
            const double b_end = b_start + static_cast<double>(m) * beats_per_sample;
            float v0[kLanes]{}, v1[kLanes]{};
            for (int lane = 0; lane < kLanes; ++lane) {
                if (!on[lane]) continue;
                double ph0, ph1;
                if (audio_trigger) {
                    ph0 = audio_phase_[lane];
                    ph1 = std::min(1.0, ph0 + static_cast<double>(m) * beats_per_sample / quarters[lane]);
                    audio_phase_[lane] = ph1;
                } else {
                    ph0 = cycle(b_start, quarters[lane]);
                    ph1 = cycle(b_end, quarters[lane]);
                    if (ph1 < ph0) ph1 = 1.0; // the cycle wraps inside the chunk: ramp to its end
                }
                const float rest = kRest[lane];
                v0[lane] = rest + depth[lane] * (curve_[lane].eval(static_cast<float>(ph0)) - rest);
                v1[lane] = rest + depth[lane] * (curve_[lane].eval(static_cast<float>(ph1)) - rest);
                if (lane == kTime) {
                    time_ph0_ = ph0;
                    time_dph_ = (ph1 - ph0) / static_cast<double>(m);
                    time_wrap_ = audio_trigger ? -1.0 : cycle(b_start, quarters[lane]);
                    time_rate_ = beats_per_sample / quarters[lane];
                }
            }

            for (int lane : kOrder) {
                if (!on[lane]) {
                    if (lane == kTime) feed_time(cl, cr, m);
                    if (lane == kPitch) feed_pitch(cl, cr, m);
                    continue;
                }
                if (band[lane] == 0) {
                    run_lane(lane, cl, cr, m, v0[lane], v1[lane], depth[lane], quarters[lane], sh);
                    continue;
                }
                float lo_l[kChunk], lo_r[kChunk], hi_l[kChunk], hi_r[kChunk];
                for (uint32_t i = 0; i < m; ++i) {
                    xo_l_[lane].split(cl[i], split_g[lane], lo_l[i], hi_l[i]);
                    xo_r_[lane].split(cr[i], split_g[lane], lo_r[i], hi_r[i]);
                }
                xo_l_[lane].flush();
                xo_r_[lane].flush();
                float* wl = band[lane] == 1 ? lo_l : hi_l;
                float* wr = band[lane] == 1 ? lo_r : hi_r;
                float* pl = band[lane] == 1 ? hi_l : lo_l;
                float* pr = band[lane] == 1 ? hi_r : lo_r;
                run_lane(lane, wl, wr, m, v0[lane], v1[lane], depth[lane], quarters[lane], sh);
                if (lane == kPitch) { // the passing side waits for the shifter
                    for (uint32_t i = 0; i < m; ++i) {
                        rest_delay_.push(pl[i], pr[i]);
                        pl[i] = rest_delay_.tap(rest_delay_.l, pitch_latency_);
                        pr[i] = rest_delay_.tap(rest_delay_.r, pitch_latency_);
                    }
                }
                for (uint32_t i = 0; i < m; ++i) {
                    cl[i] = wl[i] + pl[i];
                    cr[i] = wr[i] + pr[i];
                }
            }

            // Mix and output.
            for (uint32_t i = 0; i < m; ++i) {
                const float mx = mix_.next(mix);
                const float g = out_.next(out_gain);
                cl[i] = (dry_l[i] + mx * (cl[i] - dry_l[i])) * g;
                cr[i] = (dry_r[i] + mx * (cr[i] - dry_r[i])) * g;
            }
        }
        filt_l_.flush();
        filt_r_.flush();
    }

private:
    struct Shared {
        int ftype;
        float fk, flow, fhigh, bits, decim;
        double samples_per_quarter;
    };

    static double cycle(double beats, double quarters) noexcept {
        const double c = beats / quarters;
        return c - std::floor(c);
    }
    float coef(float hz) const noexcept {
        const double fc = std::min(static_cast<double>(hz), 0.45 * sr_);
        return static_cast<float>(std::tan(kPi * fc / sr_));
    }

    void feed_time(const float* l, const float* r, uint32_t m) noexcept {
        for (uint32_t i = 0; i < m; ++i) time_buf_.push(l[i], r[i]);
        prev_delay_ = 0.0;
        xf_left_ = 0.0f;
    }
    void feed_pitch(const float* l, const float* r, uint32_t m) noexcept {
        for (uint32_t i = 0; i < m; ++i) push_pitch(l[i], r[i]);
        for (Grain& g : grains_) g.active = false;
        hop_count_ = 0;
    }
    void push_pitch(float a, float b) noexcept {
        pitch_buf_.push(a, b);
        pitch_mono_[pitch_buf_.w] = a + b;
        ++pitch_widx_;
    }

    // Start offset (samples, added to the grain's ideal start delay) whose
    // input best continues the grain reading at `from` (absolute position).
    int splice(double from, double ideal_pos, double ratio) const noexcept {
        const uint32_t mask = pitch_buf_.mask;
        const float* x = pitch_mono_.data();
        const auto span = static_cast<int>(ratio * hop_);
        const auto ref = static_cast<int64_t>(std::llround(from));
        const auto base = static_cast<int64_t>(std::llround(ideal_pos));
        const int s = static_cast<int>(search_);
        // The newest usable sample: a candidate must not read past "now - 2".
        const int64_t newest = pitch_widx_ - 2;
        auto score = [&](int o, int step) {
            const int64_t c0 = base + o;
            if (c0 + span > newest) return -2.0f;
            float xy = 0.0f, yy = 1e-12f;
            for (int j = 0; j < span; j += step) {
                const float a = x[static_cast<uint64_t>(ref + j) & mask];
                const float b = x[static_cast<uint64_t>(c0 + j) & mask];
                xy += a * b;
                yy += b * b;
            }
            return xy / std::sqrt(yy);
        };
        // Prefer the ideal start (o = 0) unless another offset is clearly better,
        // so a steady pitch ratio of 1 continues the grain exactly.
        const auto better = [](float v, float than) { return v > than + 1e-4f * std::fabs(than) + 1e-9f; };
        int best = 0;
        float best_score = score(0, 8);
        for (int o = -s; o <= s; o += 4) {
            const float v = score(o, 8);
            if (better(v, best_score)) {
                best_score = v;
                best = o;
            }
        }
        if (best == 0) return 0;
        const int coarse = best;
        float fine = score(coarse, 2);
        for (int o = coarse - 3; o <= coarse + 3; ++o) {
            if (o < -s || o > s || o == coarse) continue;
            const float v = score(o, 2);
            if (better(v, fine)) {
                fine = v;
                best = o;
            }
        }
        return best;
    }

    void run_lane(int lane, float* l, float* r, uint32_t m, float v0, float v1, float depth, double quarters, const Shared& sh) noexcept {
        const float dv = (v1 - v0) / static_cast<float>(m);
        switch (lane) {
        case kVolume: {
            float s = smooth_[lane];
            for (uint32_t i = 0; i < m; ++i) {
                const float t = v0 + dv * static_cast<float>(i + 1);
                s = t + smooth_a_ * (s - t);
                l[i] *= s;
                r[i] *= s;
            }
            smooth_[lane] = s;
            break;
        }
        case kWidth: {
            float s = smooth_[lane];
            for (uint32_t i = 0; i < m; ++i) {
                const float t = v0 + dv * static_cast<float>(i + 1);
                s = t + smooth_a_ * (s - t);
                const float mid = 0.5f * (l[i] + r[i]);
                const float side = (l[i] - r[i]) * s; // 0.5 * (l - r) * 2v
                l[i] = mid + side;
                r[i] = mid - side;
            }
            smooth_[lane] = s;
            break;
        }
        case kPan: {
            float s = smooth_[lane];
            for (uint32_t i = 0; i < m; ++i) {
                const float t = v0 + dv * static_cast<float>(i + 1);
                s = t + smooth_a_ * (s - t);
                const float a = static_cast<float>(0.5 * kPi) * std::clamp(s, 0.0f, 1.0f);
                l[i] *= std::min(1.0f, 1.41421356f * std::cos(a));
                r[i] *= std::min(1.0f, 1.41421356f * std::sin(a));
            }
            smooth_[lane] = s;
            break;
        }
        case kFilter: {
            const float g_end = coef(sh.flow * std::pow(sh.fhigh / sh.flow, std::clamp(v1, 0.0f, 1.0f)));
            if (filt_g_ < 0.0f) filt_g_ = coef(sh.flow * std::pow(sh.fhigh / sh.flow, std::clamp(v0, 0.0f, 1.0f)));
            const float dg = (g_end - filt_g_) / static_cast<float>(m);
            // Depth fades the filter in (as Crush does): at 0 the lane is neutral
            // whatever the type, since a high-pass at the rest cutoff is not.
            const float wet = std::min(1.0f, depth * 10.0f);
            float g = filt_g_;
            for (uint32_t i = 0; i < m; ++i) {
                g += dg;
                float a, b, c, d, e, f;
                filt_l_.run(l[i], g, sh.fk, a, b, c);
                filt_r_.run(r[i], g, sh.fk, d, e, f);
                const float fl = sh.ftype == 0 ? a : sh.ftype == 1 ? sh.fk * b : c;
                const float fr = sh.ftype == 0 ? d : sh.ftype == 1 ? sh.fk * e : f;
                l[i] += wet * (fl - l[i]);
                r[i] += wet * (fr - r[i]);
            }
            filt_g_ = g_end;
            break;
        }
        case kCrush: {
            const float log_decim = std::log2(sh.decim);
            for (uint32_t i = 0; i < m; ++i) {
                const float a = std::clamp(v0 + dv * static_cast<float>(i + 1), 0.0f, 1.0f);
                if (a < 1e-4f) {
                    hold_phase_ = 1.0f;
                    continue;
                }
                const float levels = std::exp2(15.0f + a * (sh.bits - 16.0f));
                hold_phase_ += std::exp2(-a * log_decim); // new samples per input sample
                if (hold_phase_ >= 1.0f) {
                    hold_phase_ -= std::floor(hold_phase_);
                    hold_l_ = std::round(l[i] * levels) / levels;
                    hold_r_ = std::round(r[i] * levels) / levels;
                }
                const float wet = std::min(1.0f, a * 10.0f);
                l[i] += wet * (hold_l_ - l[i]);
                r[i] += wet * (hold_r_ - r[i]);
            }
            break;
        }
        case kTime: {
            const double max_delay = std::min(static_cast<double>(time_buf_.mask) - 4.0, kTimeBufferSeconds * sr_);
            const double cycle_samples = quarters * sh.samples_per_quarter;
            double ph = time_ph0_;
            double sync_ph = time_wrap_;
            for (uint32_t i = 0; i < m; ++i) {
                time_buf_.push(l[i], r[i]);
                if (sync_ph >= 0.0) { // sample-exact cycle position (wraps)
                    sync_ph += time_rate_;
                    if (sync_ph >= 1.0) sync_ph -= 1.0;
                    ph = sync_ph;
                } else {
                    ph = std::min(1.0, ph + time_dph_);
                }
                const float y = curve_[kTime].eval(static_cast<float>(ph));
                const double delay = std::clamp(static_cast<double>(depth * y) * cycle_samples, 0.0, max_delay);
                if (std::fabs(delay - prev_delay_) > 64.0) { // a jump in the curve: crossfade from the old read head
                    xf_delay_ = prev_delay_;
                    xf_left_ = xfade_samples_;
                }
                prev_delay_ = delay;
                float ol, orr;
                read_time(delay, ol, orr);
                if (xf_left_ > 0.0f) {
                    float pl, pr;
                    read_time(xf_delay_, pl, pr); // the old head keeps playing forward
                    const float w = xf_left_ / xfade_samples_;
                    ol += w * (pl - ol);
                    orr += w * (pr - orr);
                    xf_left_ -= 1.0f;
                }
                l[i] = ol;
                r[i] = orr;
            }
            break;
        }
        case kPitch: {
            const float st0 = (v0 - 0.5f) * 24.0f, st1 = (v1 - 0.5f) * 24.0f;
            const float dst = (st1 - st0) / static_cast<float>(m);
            const uint32_t mask = pitch_buf_.mask;
            const double grain = 2.0 * hop_;
            const double lat = pitch_latency_;
            for (uint32_t i = 0; i < m; ++i) {
                push_pitch(l[i], r[i]);
                const double ratio = std::exp2(static_cast<double>(st0 + dst * static_cast<float>(i + 1)) / 12.0);
                const auto now = static_cast<double>(pitch_widx_);
                if (hop_count_ == 0) {
                    // A new grain takes over from the one half-way through (if any).
                    Grain* old = nullptr;
                    for (Grain& g : grains_)
                        if (g.active && (old == nullptr || g.age > old->age)) old = &g;
                    if (old == nullptr) { // starting: a grain already at full weight on the plain delay
                        grains_[0] = Grain{now - lat, hop_, true};
                        old = &grains_[0];
                    }
                    Grain* slot = old == &grains_[0] ? &grains_[1] : &grains_[0];
                    // Ideal start: the grain's middle sits at the reported latency.
                    const double ideal = now - (lat + (ratio - 1.0) * hop_);
                    const int o = old != nullptr ? splice(old->pos, ideal, ratio) : 0;
                    *slot = Grain{ideal + o, 0, true};
                }
                hop_count_ = hop_count_ + 1 >= hop_ ? 0 : hop_count_ + 1;
                float yl = 0.0f, yr = 0.0f;
                for (Grain& g : grains_) {
                    if (!g.active) continue;
                    const double w = 0.5 - 0.5 * std::cos(2.0 * kPi * static_cast<double>(g.age) / grain);
                    const double d = std::max(2.0, now - g.pos);
                    yl += static_cast<float>(w) * Ring::hermite(pitch_buf_.l, mask, pitch_buf_.w, d);
                    yr += static_cast<float>(w) * Ring::hermite(pitch_buf_.r, mask, pitch_buf_.w, d);
                    g.pos = std::min(g.pos + ratio, now + 1.0 - 2.0);
                    if (++g.age >= 2 * hop_) g.active = false;
                }
                l[i] = yl;
                r[i] = yr;
            }
            break;
        }
        default: break;
        }
    }

    void read_time(double delay, float& l, float& r) const noexcept {
        if (delay < 1.0) { // linear between the two newest samples near "now"
            const auto f = static_cast<float>(delay);
            l = time_buf_.tap(time_buf_.l, 0) + f * (time_buf_.tap(time_buf_.l, 1) - time_buf_.tap(time_buf_.l, 0));
            r = time_buf_.tap(time_buf_.r, 0) + f * (time_buf_.tap(time_buf_.r, 1) - time_buf_.tap(time_buf_.r, 0));
            return;
        }
        l = Ring::hermite(time_buf_.l, time_buf_.mask, time_buf_.w, delay);
        r = Ring::hermite(time_buf_.r, time_buf_.mask, time_buf_.w, delay);
    }

    double sr_ = 48000.0;
    uint32_t pitch_latency_ = 962;
    std::atomic<bool> pitch_on_{false};
    Curve curve_[kLanes];
    float smooth_[kLanes]{};
    double audio_phase_[kLanes]{};
    Crossover xo_l_[kLanes], xo_r_[kLanes];
    Ring time_buf_, pitch_buf_, dry_delay_, rest_delay_;
    Svf filt_l_, filt_r_;
    float filt_g_ = -1.0f;
    float smooth_a_ = 0.0f;
    dsp::Smoother mix_, out_;
    double free_beats_ = 0.0;
    // Time lane
    double time_ph0_ = 0.0, time_dph_ = 0.0, time_wrap_ = -1.0, time_rate_ = 0.0;
    double prev_delay_ = 0.0, xf_delay_ = 0.0;
    float xf_left_ = 0.0f, xfade_samples_ = 240.0f;
    // Pitch lane
    struct Grain {
        double pos = 0.0; // absolute input position read next
        uint32_t age = 0;
        bool active = false;
    };
    Grain grains_[2];
    std::vector<float> pitch_mono_; // l + r, for the splice search
    int64_t pitch_widx_ = 0;        // absolute index of the newest pitch_buf_ sample
    uint32_t hop_ = 576, search_ = 384, hop_count_ = 0;
    // Crush lane
    float hold_l_ = 0.0f, hold_r_ = 0.0f, hold_phase_ = 1.0f;
    // Audio trigger
    float det_ = 0.0f, det_release_ = 0.0f;
    bool armed_ = true;
    uint32_t since_trigger_ = 0, holdoff_ = 1440;
    bool primed_ = false;
};

std::unique_ptr<BuiltinProcessor> create_shaper() { return std::make_unique<Shaper>(); }

} // namespace

// The definition's defaults (saine_core::builtin::shaper), in parameter order:
// the globals, then per lane on, rate, depth, band, split, points and 16
// points (unused points parked at x = 1 on the lane's rest height).
const std::array<float, kParamCount> kDefaults = [] {
    struct LaneDefault {
        float on, rate;
        int points;
        float x[5], y[5], bend[5];
        float rest_y;
    };
    constexpr LaneDefault lanes[kLanes] = {
        {1, 8, 3, {0, .5f, 1}, {0, 100, 100}, {55, 0, 0}, 100},
        {0, 8, 4, {0, .5f, .5f, 1}, {0, 0, 50, 50}, {0, 0, 0, 0}, 50},
        {0, 5, 3, {0, .5f, 1}, {50, 100, 50}, {45, 45, 0}, 50},
        {0, 5, 5, {0, .25f, .5f, .75f, 1}, {50, 100, 50, 0, 50}, {50, 50, -50, -50, 0}, 50},
        {0, 3, 2, {0, 1}, {100, 15}, {40, 0}, 15},
        {0, 8, 2, {0, 1}, {100, 0}, {-50, 0}, 0},
        {0, 8, 2, {0, 1}, {0, -12}, {50, 0}, -12},
    };
    std::array<float, kParamCount> v{0.f, -24.f, 100.f, 0.f, 0.f, 0.f, 1.2973f, 150.f, 12000.f, 6.f, 8.f};
    std::size_t i = kGlobals;
    for (const LaneDefault& l : lanes) {
        for (float x : {l.on, l.rate, 100.f, 0.f, 150.f, static_cast<float>(l.points)}) v[i++] = x;
        for (int k = 0; k < 16; ++k) {
            const bool used = k < l.points;
            v[i++] = used ? l.x[k] : 1.f;
            v[i++] = used ? l.y[k] : l.rest_y;
            v[i++] = used ? l.bend[k] : 0.f;
        }
    }
    return v;
}();

extern const BuiltinDescriptor kShaperDescriptor;
const BuiltinDescriptor kShaperDescriptor{"shaper", kParamIds, {}, &create_shaper, {}, kDefaults};

} // namespace saine
