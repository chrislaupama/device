// Decoded, engine-rate audio asset. Immutable once registered.
//
// Lifetime: assets are owned by std::shared_ptr. The asset registry holds one
// reference; every RenderGraph that uses the asset holds another. The audio
// thread only ever sees raw sample pointers reached through a published graph,
// and graphs are destroyed on non-RT threads only, so the last reference (and
// therefore the sample memory) is always released off the audio thread.
#pragma once

#include <atomic>
#include <cstdint>
#include <functional>
#include <memory>
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

    // Number of Asset objects currently alive (diagnostics / tests).
    static int64_t live_count() noexcept { return live_count_.load(std::memory_order_relaxed); }

private:
    static inline std::atomic<int64_t> live_count_{0};
};

using AssetRef = std::shared_ptr<const Asset>;
// Resolves an engine asset id (null when unknown). Main thread.
using AssetLookup = std::function<AssetRef(uint64_t asset_id)>;

} // namespace saine
