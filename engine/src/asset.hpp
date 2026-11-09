// Decoded, engine-rate audio asset. Immutable once registered.
//
// Lifetime: assets are owned by std::shared_ptr. The asset registry holds one
// reference; every RenderGraph that uses the asset holds another. The audio
// thread only ever sees raw sample pointers reached through a published graph,
// and graphs are destroyed on non-RT threads only, so the last reference (and
// therefore the sample memory) is always released off the audio thread.
#pragma once

#include <atomic>
#include <cmath>
#include <cstdint>
#include <functional>
#include <memory>
#include <mutex>
#include <vector>

namespace saine {

struct Asset {
    Asset() noexcept { live_count_.fetch_add(1, std::memory_order_relaxed); }
    ~Asset() { live_count_.fetch_sub(1, std::memory_order_relaxed); }
    Asset(const Asset&) = delete;
    Asset& operator=(const Asset&) = delete;

    uint64_t id = 0;
    uint32_t channels = 0; // 1 or 2
    uint64_t frames = 0;   // at engine sample rate
    double source_sample_rate = 0.0;
    std::vector<float> planar[2]; // planar[1] empty for mono

    const float* channel(uint32_t c) const noexcept {
        return (c < channels ? planar[c] : planar[0]).data();
    }

    // Octave-down copies for fast reads (samplers pitching far up): level
    // L (0-based) is the audio low-passed and decimated by 2^(L+1); frame n
    // of a level sits at frame n * 2^(L+1) of the original. Built on first
    // use, off the audio thread.
    static constexpr int kOctaves = 4;
    struct Octave {
        uint64_t frames = 0;
        std::vector<float> planar[2]; // planar[1] empty for mono
        const float* channel(uint32_t c) const noexcept { return (c < 2 && !planar[c].empty() ? planar[c] : planar[0]).data(); }
    };
    const Octave* octaves() const {
        std::call_once(octaves_once_, [this] { build_octaves(); });
        return octaves_;
    }

    // Number of Asset objects currently alive (diagnostics / tests).
    static int64_t live_count() noexcept { return live_count_.load(std::memory_order_relaxed); }

private:
    // Halfband low-pass (Kaiser-windowed sinc at a quarter of the rate),
    // centred, so decimated frames stay aligned with the original.
    void build_octaves() const {
        constexpr int kHalf = 31;
        constexpr double kBeta = 8.0;
        double h[2 * kHalf + 1];
        const auto i0 = [](double x) {
            double s = 1.0, t = 1.0;
            for (int k = 1; k < 32; ++k) {
                t *= (x / (2.0 * k)) * (x / (2.0 * k));
                s += t;
            }
            return s;
        };
        double sum = 0.0;
        for (int j = -kHalf; j <= kHalf; ++j) {
            const double sinc = j == 0 ? 0.5 : std::sin(3.14159265358979323846 * 0.5 * j) / (3.14159265358979323846 * j);
            const double w = static_cast<double>(j) / (kHalf + 1);
            h[j + kHalf] = sinc * i0(kBeta * std::sqrt(1.0 - w * w)) / i0(kBeta);
            sum += h[j + kHalf];
        }
        for (double& v : h) v /= sum;
        const std::vector<float>* src[2] = {&planar[0], &planar[1]};
        uint64_t n = frames;
        for (int L = 0; L < kOctaves; ++L) {
            Octave& o = octaves_[L];
            o.frames = (n + 1) / 2;
            for (uint32_t c = 0; c < channels && c < 2; ++c) {
                const std::vector<float>& x = *src[c];
                std::vector<float>& y = o.planar[c];
                y.assign(o.frames, 0.0f);
                const auto len = static_cast<int64_t>(n);
                for (uint64_t m = 0; m < o.frames; ++m) {
                    const int64_t at = static_cast<int64_t>(2 * m);
                    double acc = 0.0;
                    for (int j = -kHalf; j <= kHalf; ++j) {
                        const int64_t i = at + j;
                        if (i >= 0 && i < len) acc += h[j + kHalf] * static_cast<double>(x[static_cast<std::size_t>(i)]);
                    }
                    y[m] = static_cast<float>(acc);
                }
                src[c] = &o.planar[c];
            }
            n = o.frames;
        }
    }

    mutable std::once_flag octaves_once_;
    mutable Octave octaves_[kOctaves];
    static inline std::atomic<int64_t> live_count_{0};
};

using AssetRef = std::shared_ptr<const Asset>;
// Resolves an engine asset id (null when unknown). Main thread.
using AssetLookup = std::function<AssetRef(uint64_t asset_id)>;

} // namespace saine
