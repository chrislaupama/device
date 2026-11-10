// Audio clip warping (se_session_publish_v7, engine/README.md "Clip warping").
//
// A warped clip carries a warp map (timeline frame -> source frame, piecewise
// linear) and a mode. Varispeed and slicing read the asset directly with the
// sampler's band-limited interpolation; the stretch modes run a WarpVoice, a
// signalsmith-stretch phase vocoder that follows the map block by block.
//
// Ownership: WarpVoices are created on the main thread when a graph is
// installed (Engine::install_graph), one per (track, voice slot), and carried
// over to the next graph like synths. A slot serves warped clips of its track
// that never overlap, so the audio thread only checks whether the voice is
// still aligned with the clip and frame it renders, and restarts it (without
// allocating) when it is not.
#pragma once

#include "asset.hpp"
#include "saine_engine.h"

#include <condition_variable>
#include <cstdint>
#include <deque>
#include <memory>
#include <mutex>
#include <thread>
#include <unordered_map>
#include <vector>

namespace saine {

struct RenderGraph;
struct ClipNode;

// From `frame` on (until the next segment), source = source + (t - frame) * slope.
// The first segment also covers t < frame, the last one every t after it.
struct WarpSegment {
    int64_t frame = 0;
    double source = 0.0;
    double slope = 1.0;
};

// One slice (SE_WARP_SLICE / SE_WARP_AUTO), placed on the timeline.
struct WarpSlice {
    int64_t start = 0;   // timeline frame where the map reaches `source`
    double source = 0.0; // first source frame
    double end = 0.0;    // source frame where it runs out (next slice, or the asset end)
    bool percussive = false;
};

// A warped clip rendered ahead of time (WarpRenderer): its timeline frames
// from the clip start, at unity gain. right == left for mono sources.
struct WarpRender {
    uint64_t key = 0;
    int64_t frames = 0;
    std::vector<float> left;
    std::vector<float> right;
    std::size_t bytes() const { return (left.capacity() + right.capacity()) * sizeof(float); }
};

struct WarpClip {
    uint64_t clip_id = 0;
    uint64_t key = 0;      // identifies the audio a render of this clip holds (WarpRenderer)
    int64_t start = 0;     // timeline frame where the clip starts
    int64_t length = 0;    // timeline frames of the clip
    AssetRef asset;        // keep-alive for background renders
    uint8_t mode = SE_WARP_STRETCH;
    uint8_t channels = 2;  // of the asset; voices are configured per channel count
    float transpose = 0.0f;
    double rate = 1.0;     // slice playback rate, 2^(transpose / 12)
    uint32_t seg_begin = 0; // RenderGraph::warp_segments
    uint32_t seg_count = 0;
    uint32_t slice_begin = 0; // RenderGraph::warp_slices
    uint32_t slice_count = 0;
    int32_t voice = -1;    // index into RenderGraph::warp_voice_keys (stretch and auto modes), -1 = none
    // Set when a render of this exact clip is ready: the audio thread plays it
    // instead of stretching live.
    std::shared_ptr<const WarpRender> rendered;
};

// Identity of a voice slot: the track, the slot on that track and the
// channel count it is configured for (Engine::install_graph matches these).
struct WarpVoiceKey {
    uint64_t track_id = 0;
    uint32_t slot = 0;
    uint32_t channels = 2;
};

inline bool warp_needs_voice(uint8_t mode) noexcept {
    return mode == SE_WARP_STRETCH || mode == SE_WARP_STRETCH_FORMANT || mode == SE_WARP_AUTO;
}

// Validation of one description (see se_session_publish_v7); null when valid.
const char* validate_warp(const se_clip_warp_desc& d) noexcept;

// Appends the clip's map and slices to `g` and returns its index in
// g.warps. Non-RT. `timeline_start` and `length` are the clip's; the
// asset bounds the last slice and is kept alive by the warp.
int32_t add_warp(RenderGraph& g, const se_clip_warp_desc& d, const AssetRef& asset, int64_t timeline_start,
                 int64_t length);

// Gives the warped clips of each track their voice slots (clips that overlap
// get different slots) and fills g.warp_voice_keys. Call once the clips of
// every track are sorted. Non-RT.
void assign_warp_voices(RenderGraph& g);

class WarpVoice {
public:
    // Non-RT: allocates and configures the stretcher.
    WarpVoice(double sample_rate, uint32_t channels, uint32_t max_block);
    ~WarpVoice();
    WarpVoice(const WarpVoice&) = delete;
    WarpVoice& operator=(const WarpVoice&) = delete;

    uint32_t channels() const noexcept;
    // Input and output latency of the stretcher in frames (diagnostics, tests).
    int input_latency() const noexcept;
    int output_latency() const noexcept;

    struct Impl;
    Impl& impl() noexcept { return *impl_; }

private:
    std::unique_ptr<Impl> impl_;
};

// RT: adds the warped clip's timeline frames [from, to) into dst_l / dst_r
// (dst[0] is frame `from`), scaled by the clip gain. `voice` is the clip's
// stretch voice; when it is null the stretch modes fall back to varispeed.
// to - from must not exceed the voice's max_block.
void render_warped_clip(const RenderGraph& g, const ClipNode& clip, WarpVoice* voice, int64_t from, int64_t to,
                        float* dst_l, float* dst_r) noexcept;

// Renders the stretch-mode clips of published graphs ahead of time on a
// background thread, so playback costs a copy instead of a phase vocoder.
// Renders are keyed by everything that shapes the audio (asset, mode,
// transpose, map and slices relative to the clip start, length), so moving a
// clip or changing its gain reuses them. Until a render is ready the clip
// stretches live. Main thread: resolve() and evict(); the worker only renders.
class WarpRenderer {
public:
    WarpRenderer(double sample_rate, uint32_t max_block);
    ~WarpRenderer();
    WarpRenderer(const WarpRenderer&) = delete;
    WarpRenderer& operator=(const WarpRenderer&) = delete;

    // Attaches finished renders to g's clips and queues the missing ones.
    void resolve(RenderGraph& g);
    // True once since a render finished after the last call (then resolve
    // a fresh copy of the current graph).
    bool take_finished();
    // Drops the oldest renders `current` does not use until at most
    // kKeepUnused of them, holding at most kUnusedBytes, remain.
    void evict(const RenderGraph& current);
    // Renders queued or running.
    std::size_t pending() const;
    // Off: resolve() attaches and queues nothing (tests of live stretching).
    void set_enabled(bool enabled);

    static constexpr std::size_t kKeepUnused = 32;
    static constexpr std::size_t kUnusedBytes = std::size_t{512} << 20;
    static constexpr int64_t kMaxFrames = int64_t{48000} * 600; // longer clips always stretch live

private:
    struct Job;
    void run();

    double sample_rate_;
    uint32_t max_block_;
    mutable std::mutex mutex_;
    std::condition_variable cv_;
    std::deque<std::shared_ptr<Job>> queue_;
    std::unordered_map<uint64_t, std::shared_ptr<const WarpRender>> done_;
    std::unordered_map<uint64_t, uint64_t> last_used_; // key -> resolve generation
    uint64_t generation_ = 0;
    uint64_t in_flight_ = 0; // key being rendered, 0 = none
    bool finished_ = false;
    bool enabled_ = true;
    bool stop_ = false;
    std::thread thread_;
};

// Onset detection for se_asset_detect_onsets. Non-RT.
void detect_onsets(const Asset& asset, double sample_rate, std::vector<se_onset>& out);

} // namespace saine
