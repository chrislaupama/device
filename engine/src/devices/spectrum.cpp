// Spectrum: real-time frequency analyser; audio passes through unchanged.
// Parameters in the order of dstudio_core::builtin::spectrum (checked by a
// Rust test).
//
// Every hop (~21 ms at 48 kHz, at most half the FFT) the last FFT Size
// samples of the chosen channel (Stereo = mean power of L and R, L, R, Mid,
// Side, or Mid+Side as two traces) are windowed (Window), transformed with
// the shared FFT (spectral.hpp) and reduced to kBands log-spaced bands from
// 20 Hz to 20 kHz: band i covers 20 Hz * 1000^(i/N) .. 20 Hz * 1000^((i+1)/N)
// and takes the strongest bin inside it, or, below the FFT's resolution,
// the power interpolated at its centre. Levels are dBFS (a full-scale sine
// reads 0 dB whatever the window), tilted by Slope dB per octave around
// 1 kHz so pink noise reads flat. Decay sets how fast a band falls (0 % =
// instantly, 100 % = 3 dB/s); Peak Hold keeps each band's maximum for 2 s
// (then it falls at 24 dB/s) or until reset (Infinite).
//
// Analysis frame (AnalysisChannel): kBands band levels, kBands peak-hold
// levels (-120 when Peak Hold is Off), and in Mid+Side kBands Side levels.
// The panel applies Averaging and Smoothing and draws within Ceiling and
// Range. Display Mode, Color Scheme and History Depth are display settings
// that the kernel does not read.
#include "builtin.hpp"
#include "dsp.hpp"
#include "spectral.hpp"

#include <algorithm>
#include <array>
#include <cmath>
#include <vector>

namespace dstudio {

namespace {

enum Param : uint32_t {
    kFftSize,
    kWindow,
    kChannel,
    kSmoothing,
    kSlope,
    kCeiling,
    kRange,
    kPeakHold,
    kAveraging,
    kDecay,
    kDisplayMode,
    kColorScheme,
    kHistoryDepth,
    kParamCount
};
constexpr const char* kParamIds[kParamCount] = {"fft_size", "window", "channel_mode", "smoothing", "slope",
                                                "db_ceiling", "db_range", "peak_hold", "averaging", "decay",
                                                "display_mode", "color_scheme", "history_depth"};

enum Channel : int { kStereo, kLeft, kRight, kMid, kSide, kMidSide };

constexpr uint32_t kBands = 256;
constexpr uint32_t kSizes = 6;
constexpr uint32_t kMinSize = 512;
constexpr uint32_t kMaxSize = 16384;
constexpr float kFloorDb = -120.0f;

class Spectrum final : public BuiltinProcessor {
public:
    void prepare(double sample_rate, uint32_t) override {
        sr_ = sample_rate > 0.0 ? sample_rate : 48000.0;
        ring_l_.assign(kMaxSize, 0.0f);
        ring_r_.assign(kMaxSize, 0.0f);
        frame_.assign(kMaxSize, 0.0f);
        win_.assign(kMaxSize, 0.0f);
        for (int c = 0; c < 2; ++c) {
            re_[c].assign(kMaxSize / 2 + 1, 0.0f);
            im_[c].assign(kMaxSize / 2 + 1, 0.0f);
            pow_[c].assign(kMaxSize / 2 + 1, 0.0f);
        }
        uint32_t hop = 256;
        while (hop * 2 <= sr_ / 45.0) hop <<= 1;
        hop_ = hop;
        for (uint32_t s = 0; s < kSizes; ++s) {
            const uint32_t n = kMinSize << s;
            fft_[s].prepare(n);
            // Band -> bin ranges (b0 > b1: interpolate at the centre bin position).
            for (uint32_t i = 0; i < kBands; ++i) {
                const double lo = 20.0 * std::pow(1000.0, static_cast<double>(i) / kBands) * n / sr_;
                const double hi = 20.0 * std::pow(1000.0, static_cast<double>(i + 1) / kBands) * n / sr_;
                const double mid = 20.0 * std::pow(1000.0, (i + 0.5) / kBands) * n / sr_;
                Band& b = bands_[s][i];
                b.b0 = static_cast<uint32_t>(std::ceil(lo));
                b.b1 = static_cast<uint32_t>(std::min(std::floor(hi), n / 2.0));
                b.at = static_cast<float>(std::min(mid, n / 2.0 - 1.0));
                b.tilt_oct = static_cast<float>(std::log2(20.0 * std::pow(1000.0, (i + 0.5) / kBands) / 1000.0));
            }
        }
    }

    void reset() noexcept override {
        std::fill(ring_l_.begin(), ring_l_.end(), 0.0f);
        std::fill(ring_r_.begin(), ring_r_.end(), 0.0f);
        pos_ = 0;
        since_ = 0;
        shown_.fill(kFloorDb);
        shown2_.fill(kFloorDb);
        peak_.fill(kFloorDb);
        peak_t_.fill(0.0f);
        win_size_ = 0;
        win_kind_ = -1;
        last_size_ = last_hold_ = -1;
    }

    uint32_t analysis_capacity() const noexcept override { return 3 * kBands; }

    void process(float* l, float* r, uint32_t n, const float* p, const BuiltinContext& ctx, float*) noexcept override {
        const int size_i = std::clamp(static_cast<int>(std::lround(p[kFftSize])), 0, static_cast<int>(kSizes) - 1);
        const int wkind = std::clamp(static_cast<int>(std::lround(p[kWindow])), 0, 5);
        const int chan = std::clamp(static_cast<int>(std::lround(p[kChannel])), 0, 5);
        const float slope = std::clamp(p[kSlope], -12.0f, 12.0f);
        const int hold = std::clamp(static_cast<int>(std::lround(p[kPeakHold])), 0, 2);
        const float decay = std::clamp(p[kDecay], 0.0f, 100.0f) * 0.01f;
        const uint32_t mask = kMaxSize - 1;
        // Analyse at least every half frame, so small FFT sizes leave no input unexamined.
        cur_hop_ = std::min(hop_, (kMinSize << size_i) / 2);
        uint32_t i = 0;
        while (i < n) {
            const uint32_t m = since_ >= cur_hop_ ? 0 : std::min(n - i, cur_hop_ - since_);
            for (uint32_t k = 0; k < m; ++k) {
                ring_l_[(pos_ + k) & mask] = l[i + k];
                ring_r_[(pos_ + k) & mask] = r[i + k];
            }
            pos_ = (pos_ + m) & mask;
            since_ += m;
            i += m;
            if (since_ >= cur_hop_) {
                since_ = 0;
                analyse(static_cast<uint32_t>(size_i), wkind, chan, slope, hold, decay, ctx.analysis);
            }
        }
    }

private:
    struct Band {
        uint32_t b0 = 0, b1 = 0;
        float at = 0.0f, tilt_oct = 0.0f;
    };

    // Power spectrum of one channel source into pow_[slot].
    void spectrum(uint32_t si, int source, int slot, float norm) noexcept {
        const uint32_t n = kMinSize << si, mask = kMaxSize - 1;
        const uint32_t start = (pos_ - n) & mask;
        float* const f = frame_.data();
        const float* const w = win_.data();
        for (uint32_t k = 0; k < n; ++k) {
            const uint32_t j = (start + k) & mask;
            const float a = ring_l_[j], b = ring_r_[j];
            const float x = source == kLeft ? a : source == kRight ? b : source == kSide ? 0.5f * (a - b) : 0.5f * (a + b);
            f[k] = x * w[k];
        }
        float* const re = re_[slot].data();
        float* const im = im_[slot].data();
        fft_[si].forward(f, re, im);
        float* const pw = pow_[slot].data();
        for (uint32_t k = 0; k <= n / 2; ++k) pw[k] = (re[k] * re[k] + im[k] * im[k]) * norm;
    }

    void bands(uint32_t si, const float* pw, float slope, float fall, std::array<float, kBands>& shown, float* out) noexcept {
        for (uint32_t b = 0; b < kBands; ++b) {
            const Band& d = bands_[si][b];
            float pv;
            if (d.b0 <= d.b1) {
                pv = pw[d.b0];
                for (uint32_t k = d.b0 + 1; k <= d.b1; ++k) pv = std::max(pv, pw[k]);
            } else {
                const auto k = static_cast<uint32_t>(d.at);
                const float t = d.at - static_cast<float>(k);
                pv = pw[k] + t * (pw[k + 1] - pw[k]);
            }
            const float db = std::max(kFloorDb, dsp::power_to_db(pv) + slope * d.tilt_oct);
            shown[b] = fall <= 0.0f ? db : std::max(db, std::max(kFloorDb, shown[b] - fall));
            out[b] = shown[b];
        }
    }

    void analyse(uint32_t si, int wkind, int chan, float slope, int hold, float decay, AnalysisChannel* out) noexcept {
        const uint32_t n = kMinSize << si;
        if (win_size_ != n || win_kind_ != wkind) {
            gain_ = dsp::fill_window(static_cast<dsp::Window>(wkind), n, win_.data());
            win_size_ = n;
            win_kind_ = wkind;
        }
        if (last_size_ != static_cast<int>(si) || last_hold_ != hold) { // a new resolution or mode starts a new peak
            peak_.fill(kFloorDb);
            peak_t_.fill(0.0f);
            last_size_ = static_cast<int>(si);
            last_hold_ = hold;
        }
        // Amplitude 2|X| / (n * coherent gain), squared.
        const float a = 2.0f / (static_cast<float>(n) * gain_);
        const float norm = a * a;
        const float dt = static_cast<float>(cur_hop_ / sr_);
        const float fall = decay <= 0.0f ? 0.0f : 3.0f * std::pow(40.0f, 1.0f - decay) * dt;
        if (out == nullptr) return;
        float* const o = out->begin();
        switch (chan) {
        case kStereo: {
            spectrum(si, kLeft, 0, norm);
            spectrum(si, kRight, 1, norm);
            float* const p0 = pow_[0].data();
            const float* const p1 = pow_[1].data();
            for (uint32_t k = 0; k <= n / 2; ++k) p0[k] = 0.5f * (p0[k] + p1[k]);
            bands(si, p0, slope, fall, shown_, o);
            break;
        }
        case kMidSide:
            spectrum(si, kMid, 0, norm);
            spectrum(si, kSide, 1, norm);
            bands(si, pow_[0].data(), slope, fall, shown_, o);
            bands(si, pow_[1].data(), slope, fall, shown2_, o + 2 * kBands);
            break;
        default:
            spectrum(si, chan, 0, norm);
            bands(si, pow_[0].data(), slope, fall, shown_, o);
            break;
        }
        for (uint32_t b = 0; b < kBands; ++b) {
            const float v = o[b];
            if (hold == 0) {
                peak_[b] = kFloorDb;
            } else if (v >= peak_[b]) {
                peak_[b] = v;
                peak_t_[b] = 2.0f;
            } else if (hold == 1) {
                peak_t_[b] -= dt;
                if (peak_t_[b] <= 0.0f) peak_[b] = std::max(v, peak_[b] - 24.0f * dt);
            }
            o[kBands + b] = peak_[b];
        }
        out->publish(chan == kMidSide ? 3 * kBands : 2 * kBands);
    }

    double sr_ = 48000.0;
    uint32_t hop_ = 1024, cur_hop_ = 1024, since_ = 0, pos_ = 0;
    std::vector<float> ring_l_, ring_r_, frame_, win_;
    std::vector<float> re_[2], im_[2], pow_[2];
    dsp::RealFft fft_[kSizes];
    Band bands_[kSizes][kBands];
    std::array<float, kBands> shown_{}, shown2_{}, peak_{}, peak_t_{};
    uint32_t win_size_ = 0;
    int win_kind_ = -1, last_size_ = -1, last_hold_ = -1;
    float gain_ = 0.5f;
};

std::unique_ptr<BuiltinProcessor> create_spectrum() { return std::make_unique<Spectrum>(); }

} // namespace

extern const BuiltinDescriptor kSpectrumDescriptor;
constexpr float kDefaults[] = {3.0f, 0.0f, 0.0f, 30.0f, 4.5f, 0.0f, 90.0f, 1.0f, 30.0f, 50.0f, 1.0f, 0.0f, 3.0f};
const BuiltinDescriptor kSpectrumDescriptor{"spectrum", kParamIds, {}, &create_spectrum, {}, kDefaults};

} // namespace dstudio
