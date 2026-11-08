// Immutable render graph built from a session snapshot on a non-RT thread.
//
// Everything the audio thread needs is resolved at build time: mute/solo is
// collapsed into `audible`, pan law and track gain are folded into two
// per-channel gains, clips are clamped to their asset and sorted by start,
// and raw sample pointers are cached. The graph also owns AssetRefs so the
// sample memory outlives every block that can still read it.
//
// Concurrency model: a graph is never mutated after publication except for
// its meter atomics. Retired graphs are destroyed on non-RT threads only.
//
// Future work (multithreaded processing): TrackNodes are independent of each
// other until the master sum, so a worker pool can render disjoint track
// ranges into per-worker scratch buffers and the RT thread then sums them.
// The graph is already a flat, read-only array, which is what a lock-free
// work-stealing scheduler wants. Buses/sends will turn this into a DAG with a
// precomputed topological order; this milestone only has track -> master.
#pragma once

#include "asset.hpp"
#include "saine_engine.h"

#include <atomic>
#include <cstdint>
#include <functional>
#include <memory>
#include <string>
#include <vector>

namespace saine {

struct ClipNode {
    const float* src_l = nullptr; // asset channel 0
    const float* src_r = nullptr; // asset channel 1, or channel 0 for mono assets
    int64_t timeline_start = 0;   // first frame on the timeline
    int64_t timeline_end = 0;     // one past the last frame (clamped to asset end)
    int64_t source_offset = 0;    // asset frame played at timeline_start
    float gain = 1.0f;
};

struct TrackNode {
    uint64_t track_id = 0;
    float gain_l = 0.0f; // track gain * constant-power pan (left)
    float gain_r = 0.0f; // track gain * constant-power pan (right)
    bool audible = false;
    uint32_t clip_begin = 0; // index into RenderGraph::clips
    uint32_t clip_count = 0;
};

// Number of buckets in the se_meters_peek sliding window (each bucket covers
// ~10 ms of rendered frames, see Engine::advance_peek_window()).
inline constexpr uint32_t kPeekBuckets = 5;

struct TrackMeter {
    uint64_t track_id = 0;
    std::atomic<float> peak_l{0.0f}; // peak since last se_meters_read (reset by the reader)
    std::atomic<float> peak_r{0.0f};
    std::atomic<float> win_l[kPeekBuckets] = {}; // se_meters_peek ring (zeroed by the RT side)
    std::atomic<float> win_r[kPeekBuckets] = {};
};

struct RenderGraph {
    uint64_t seq = 0; // publication sequence number, assigned when published
    float master_gain = 1.0f;
    std::vector<TrackNode> tracks;
    std::vector<ClipNode> clips;
    std::vector<AssetRef> assets; // keep-alive references
    std::unique_ptr<TrackMeter[]> meters; // one per track, same order
    std::size_t meter_count = 0;
};

// Constant-power pan law: theta = (pan + 1) * pi / 4, L = cos, R = sin.
// Centre is -3 dB on both sides (L^2 + R^2 = 1 for any pan). Applied as a
// balance control to stereo sources (left gain on the left channel).
void constant_power_pan(float pan, float& left, float& right) noexcept;

using AssetLookup = std::function<AssetRef(uint64_t asset_id)>;

// Validates and copies the description. Returns an se_result code.
int32_t build_render_graph(const se_session_desc& desc, const AssetLookup& lookup,
                           std::unique_ptr<RenderGraph>& out, std::string& err);

} // namespace saine
