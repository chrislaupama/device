#include "resampler.hpp"

#include <algorithm>
#include <cmath>
#include <numbers>

namespace saine {
namespace {

constexpr int kPhases = 512;          // table resolution per input sample
constexpr double kZeroCrossings = 32; // kernel half-width in cutoff-scaled zero crossings
constexpr double kKaiserBeta = 9.0;
constexpr double kRolloff = 0.94; // passband edge as fraction of the lower Nyquist

double bessel_i0(double x) noexcept {
    // Power series; converges quickly for the beta values we use.
    double sum = 1.0, term = 1.0;
    const double q = x * x / 4.0;
    for (int k = 1; k < 64; ++k) {
        term *= q / (static_cast<double>(k) * static_cast<double>(k));
        sum += term;
        if (term < sum * 1e-17) break;
    }
    return sum;
}

} // namespace

uint64_t resampled_length(uint64_t input_frames, double src_rate, double dst_rate) noexcept {
    if (src_rate <= 0.0 || dst_rate <= 0.0) return 0;
    return static_cast<uint64_t>(std::llround(static_cast<double>(input_frames) * dst_rate / src_rate));
}

SincResampler::SincResampler(double src_rate, double dst_rate) : src_rate_(src_rate), dst_rate_(dst_rate) {
    const double ratio = dst_rate / src_rate;
    cutoff_ = 0.5 * std::min(1.0, ratio) * kRolloff;
    // Zero crossings of sinc(2*fc*x) are spaced 1/(2*fc) input samples apart.
    half_width_ = kZeroCrossings / (2.0 * cutoff_);

    const auto n = static_cast<std::size_t>(std::ceil(half_width_ * kPhases)) + 2;
    table_.resize(n);
    const double i0_beta = bessel_i0(kKaiserBeta);
    for (std::size_t k = 0; k < n; ++k) {
        const double x = static_cast<double>(k) / kPhases;
        double h = 0.0;
        if (x < half_width_) {
            const double arg = 2.0 * cutoff_ * x;
            const double sinc = (arg == 0.0) ? 1.0 : std::sin(std::numbers::pi * arg) / (std::numbers::pi * arg);
            const double r = x / half_width_;
            const double w = bessel_i0(kKaiserBeta * std::sqrt(std::max(0.0, 1.0 - r * r))) / i0_beta;
            h = 2.0 * cutoff_ * sinc * w; // unit DC gain
        }
        table_[k] = static_cast<float>(h);
    }
}

void SincResampler::process(const float* in, uint64_t in_frames, std::vector<float>& out) const {
    const uint64_t out_frames = resampled_length(in_frames, src_rate_, dst_rate_);
    out.assign(out_frames, 0.0f);
    if (in_frames == 0) return;

    const double step = src_rate_ / dst_rate_;
    const auto last = static_cast<int64_t>(in_frames) - 1;
    const std::size_t table_last = table_.size() - 2;

    for (uint64_t j = 0; j < out_frames; ++j) {
        const double t = static_cast<double>(j) * step; // exact per-sample, no drift
        const auto i0 = std::max<int64_t>(0, static_cast<int64_t>(std::ceil(t - half_width_)));
        const auto i1 = std::min<int64_t>(last, static_cast<int64_t>(std::floor(t + half_width_)));
        double acc = 0.0;
        for (int64_t i = i0; i <= i1; ++i) {
            const double pos = std::fabs(t - static_cast<double>(i)) * kPhases;
            const auto k = static_cast<std::size_t>(pos);
            if (k > table_last) continue;
            const double frac = pos - static_cast<double>(k);
            const double h0 = static_cast<double>(table_[k]);
            const double h = h0 + (static_cast<double>(table_[k + 1]) - h0) * frac;
            acc += h * static_cast<double>(in[i]);
        }
        out[j] = static_cast<float>(acc);
    }
}

} // namespace saine
