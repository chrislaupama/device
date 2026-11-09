// De-Clipper: rebuilds clipped peaks, then holds the result under a
// true-peak ceiling. Parameters in the order of saine_core::builtin::declipper
// (checked by a Rust test).
//
// 1. Detection, per channel: a run of consecutive samples at or above
//    Threshold (same polarity) is a clipped stretch.
// 2. Rebuild: once the run has ended, the stretch is replaced by a cubic
//    Hermite arc between the last clean sample before it and the first one
//    after it, with the slopes of the clean signal on both sides. Clipped
//    samples are lower bounds, so the arc never dips under them, and it is
//    capped 12 dB above the threshold. Runs longer than the look-ahead
//    window (64 samples at 48 kHz) are left alone.
// 3. Mix: blend of the rebuilt and the original signal.
// 4. Ceiling: a look-ahead limiter (stereo linked) with a true-peak detector
//    (polyphase windowed-sinc interpolation at 2x / 4x / 8x, the
//    Oversampling choice): instant attack spread over a 0.67 ms ramp by a
//    sliding minimum + moving average, 50 ms release. Rebuilt peaks that
//    would pass the ceiling come out quieter.
// Latency: window + 8 + ramp samples (104 at 48 kHz) whatever the factor, so
// delay compensation does not jump when Oversampling changes.
#include "builtin.hpp"
#include "dsp.hpp"
#include "spectral.hpp"

#include <algorithm>
#include <cmath>
#include <cstdint>
#include <vector>

namespace saine {

namespace {

enum Param : uint32_t { kThreshold, kCeiling, kOversampling, kMix, kParamCount };
constexpr const char* kParamIds[kParamCount] = {"threshold", "ceiling", "oversampling", "mix"};
constexpr float kDefaults[kParamCount] = {-1.0f, -0.3f, 1.0f, 100.0f};

constexpr int kTaps = 16;        // interpolation taps: 7 samples before, 8 after
constexpr int kTapsBefore = 7;   // so a point between s and s + 1 needs s + 8
constexpr int kMaxPhases = 7;    // 8x: 7 points between two samples
constexpr float kMaxRebuild = 3.98107f; // +12 dB over the threshold

class DeClipper final : public BuiltinProcessor {
public:
    void prepare(double sample_rate, uint32_t) override {
        sr_ = sample_rate;
        const auto scale = static_cast<int64_t>(std::max(1.0, std::round(sample_rate / 48000.0)));
        window_ = 64 * scale;
        ramp_ = 32 * scale;
        const auto ring = dsp::pow2_at_least(static_cast<uint32_t>(window_ + 8));
        mask_ = ring - 1;
        const auto mring = dsp::pow2_at_least(static_cast<uint32_t>(ramp_ + kTaps + 2));
        mmask_ = mring - 1;
        for (int c = 0; c < 2; ++c) {
            buf_[c].assign(ring, 0.0f);
            orig_[c].assign(ring, 0.0f);
            mixed_[c].assign(mring, 0.0f);
        }
        hist_.assign(static_cast<std::size_t>(ramp_ + 1), 1.0);
        qidx_.assign(static_cast<std::size_t>(ramp_ + 2), 0);
        qval_.assign(static_cast<std::size_t>(ramp_ + 2), 1.0f);
        // Interpolation kernels: Blackman-windowed sinc over +-8 samples, unity DC gain per phase.
        const int factors[3] = {2, 4, 8};
        for (int f = 0; f < 3; ++f) {
            for (int ph = 1; ph < factors[f]; ++ph) {
                const double frac = static_cast<double>(ph) / factors[f];
                double sum = 0.0;
                double c[kTaps];
                for (int t = 0; t < kTaps; ++t) {
                    const double x = frac - (t - kTapsBefore);
                    const double s = std::fabs(x) < 1e-9 ? 1.0 : std::sin(dsp::kPiD * x) / (dsp::kPiD * x);
                    const double w = 0.42 + 0.5 * std::cos(dsp::kPiD * x / 8.0) + 0.08 * std::cos(2.0 * dsp::kPiD * x / 8.0);
                    c[t] = s * w;
                    sum += c[t];
                }
                for (int t = 0; t < kTaps; ++t) kernel_[f][ph - 1][t] = static_cast<float>(c[t] / sum);
            }
        }
        rel_ = dsp::pole(50.0f, sr_);
        mix_.set_time(20.0f, sr_);
        reset();
    }

    void reset() noexcept override {
        for (int c = 0; c < 2; ++c) {
            std::fill(buf_[c].begin(), buf_[c].end(), 0.0f);
            std::fill(orig_[c].begin(), orig_[c].end(), 0.0f);
            std::fill(mixed_[c].begin(), mixed_[c].end(), 0.0f);
            run_[c] = Run{};
        }
        std::fill(hist_.begin(), hist_.end(), 1.0);
        sum_ = static_cast<double>(ramp_ + 1);
        qhead_ = qtail_ = 0;
        rel_state_ = 1.0f;
        n_ = 0;
        primed_ = false;
    }

    uint32_t latency() const noexcept override { return static_cast<uint32_t>(window_ + 8 + ramp_); }

    void process(float* l, float* r, uint32_t n, const float* p, const BuiltinContext&, float* ro) noexcept override {
        (void)ro;
        const float threshold = dsp::db_to_gain(std::clamp(p[kThreshold], -24.0f, 0.0f));
        const float ceiling = dsp::db_to_gain(std::clamp(p[kCeiling], -24.0f, 0.0f));
        const int factor = std::clamp(static_cast<int>(std::lround(p[kOversampling])), 0, 2);
        const int phases = (2 << factor) - 1;
        const float mix = std::clamp(p[kMix], 0.0f, 100.0f) * 0.01f;
        if (!primed_) {
            mix_.snap(mix);
            primed_ = true;
        }
        const float(*kern)[kTaps] = kernel_[factor];
        float* io[2] = {l, r};
        for (uint32_t i = 0; i < n; ++i, ++n_) {
            const float m = mix_.next(mix);
            float peak = 0.0f;
            for (int c = 0; c < 2; ++c) {
                const float x = io[c][i];
                orig_[c][n_ & mask_] = x;
                buf_[c][n_ & mask_] = x;
                detect(c, x, threshold);
                const int64_t o = n_ - window_;
                const float z = buf_[c][o & mask_];
                const float xd = orig_[c][o & mask_];
                float* mr = mixed_[c].data();
                mr[n_ & mmask_] = xd + m * (z - xd);
                // True peak of the mixed signal at s = n - 8 (between s and s + 1).
                const int64_t s = n_ - 8;
                float pk = std::fabs(mr[s & mmask_]);
                float tap[kTaps];
                for (int t = 0; t < kTaps; ++t) tap[t] = mr[(s - kTapsBefore + t) & mmask_];
                for (int ph = 0; ph < phases; ++ph) {
                    float v = 0.0f;
                    for (int t = 0; t < kTaps; ++t) v += tap[t] * kern[ph][t];
                    pk = std::max(pk, std::fabs(v));
                }
                peak = std::max(peak, pk);
            }
            // Gain needed for sample s, release, then sliding minimum + moving average over the ramp.
            const float need = peak > ceiling ? ceiling / peak : 1.0f;
            rel_state_ = need < rel_state_ ? need : need + rel_ * (rel_state_ - need);
            const float h = push_min(n_, rel_state_);
            const std::size_t slot = static_cast<std::size_t>(n_ % (ramp_ + 1));
            sum_ += static_cast<double>(h) - hist_[slot];
            hist_[slot] = static_cast<double>(h);
            const float g = std::min(1.0f, static_cast<float>(sum_ / static_cast<double>(ramp_ + 1)));
            for (int c = 0; c < 2; ++c) {
                const float y = mixed_[c][(n_ - 8 - ramp_) & mmask_] * g;
                io[c][i] = std::clamp(y, -ceiling, ceiling);
            }
        }
        if ((n_ & 0xffff) < n) sum_ = recount(); // keep the running sum exact
    }

private:
    struct Run {
        int64_t start = -1, pend_s = -1, pend_e = -1;
        float sign = 0.0f, pend_sign = 0.0f;
    };

    void detect(int c, float x, float threshold) noexcept {
        Run& st = run_[c];
        if (st.pend_e >= 0 && n_ == st.pend_e + 1) {
            rebuild(c, st.pend_s, st.pend_e, st.pend_sign, threshold);
            st.pend_e = -1;
        }
        const bool clipped = std::fabs(x) >= threshold;
        const float sign = x >= 0.0f ? 1.0f : -1.0f;
        if (st.start >= 0 && !(clipped && sign == st.sign)) {
            // The run [start, n) has ended; rebuild it next sample if it still fits the window.
            if (n_ - st.start <= window_ - 4 && st.pend_e < 0) {
                st.pend_s = st.start;
                st.pend_e = n_;
                st.pend_sign = st.sign;
            }
            st.start = -1;
        }
        if (st.start < 0 && clipped) {
            st.start = n_;
            st.sign = sign;
        }
    }

    // Replace samples [s, e) with a Hermite arc from s - 1 to e.
    void rebuild(int c, int64_t s, int64_t e, float sign, float threshold) noexcept {
        float* b = buf_[c].data();
        const float xa = b[(s - 1) & mask_], xb = b[e & mask_];
        const float ma = xa - b[(s - 2) & mask_];
        const float mb = b[(e + 1) & mask_] - xb;
        const auto len = static_cast<float>(e - (s - 1));
        const float cap = threshold * kMaxRebuild;
        for (int64_t i = s; i < e; ++i) {
            const float t = static_cast<float>(i - (s - 1)) / len;
            const float t2 = t * t, t3 = t2 * t;
            const float y = (2.0f * t3 - 3.0f * t2 + 1.0f) * xa + (t3 - 2.0f * t2 + t) * len * ma +
                            (-2.0f * t3 + 3.0f * t2) * xb + (t3 - t2) * len * mb;
            const float lower = std::fabs(b[i & mask_]);
            b[i & mask_] = sign * std::min(cap, std::max(lower, sign * y));
        }
    }

    // Sliding minimum over the last ramp + 1 values (monotonic queue).
    float push_min(int64_t idx, float v) noexcept {
        const auto cap = static_cast<uint32_t>(qidx_.size());
        while (qtail_ != qhead_ && qval_[(qtail_ + cap - 1) % cap] >= v) qtail_ = (qtail_ + cap - 1) % cap;
        qidx_[qtail_] = idx;
        qval_[qtail_] = v;
        qtail_ = (qtail_ + 1) % cap;
        while (qidx_[qhead_] <= idx - (ramp_ + 1)) qhead_ = (qhead_ + 1) % cap;
        return qval_[qhead_];
    }

    double recount() const noexcept {
        double s = 0.0;
        for (double v : hist_) s += v;
        return s;
    }

    double sr_ = 48000.0;
    int64_t window_ = 64, ramp_ = 32, n_ = 0;
    int64_t mask_ = 127, mmask_ = 63;
    std::vector<float> buf_[2], orig_[2], mixed_[2];
    Run run_[2];
    float kernel_[3][kMaxPhases][kTaps] = {};
    std::vector<double> hist_;
    std::vector<int64_t> qidx_;
    std::vector<float> qval_;
    uint32_t qhead_ = 0, qtail_ = 0;
    double sum_ = 0.0;
    float rel_ = 0.0f, rel_state_ = 1.0f;
    bool primed_ = false;
    dsp::Smoother mix_;
};

std::unique_ptr<BuiltinProcessor> create() { return std::make_unique<DeClipper>(); }

} // namespace

extern const BuiltinDescriptor kDeClipperDescriptor;
const BuiltinDescriptor kDeClipperDescriptor{"declipper", kParamIds, {}, &create, {}, kDefaults};

} // namespace saine
