// Engine: owns assets, the published render graph, transport, meters, stats
// and the platform device. See engine/README.md for the threading model.
#pragma once

#include "asset.hpp"
#include "platform/audio_device.hpp"
#include "processor.hpp"
#include "render_graph.hpp"
#include "saine_engine.h"
#include "spsc_queue.hpp"
#include "wav.hpp"

#include <atomic>
#include <cstdint>
#include <memory>
#include <mutex>
#include <string>
#include <unordered_map>
#include <vector>

namespace saine {

struct TransportCommand {
    enum class Type : uint8_t { Play, Pause, Stop, Seek };
    Type type = Type::Play;
    int64_t frame = 0;
};

class Engine final : private AudioDevice::Callback {
public:
    static constexpr uint32_t kMaxBlockLimit = 16384;
    static constexpr std::size_t kCommandQueueSize = 256;

    static std::unique_ptr<Engine> create(const se_config& config, std::string& err);
    ~Engine();
    Engine(const Engine&) = delete;
    Engine& operator=(const Engine&) = delete;

    // ---- device
    int32_t device_start(std::string& err);
    int32_t device_stop();
    bool device_running() const noexcept;
    double sample_rate() const noexcept { return sample_rate_; }
    uint32_t max_block() const noexcept { return max_block_; }

    // ---- assets
    int32_t asset_load_file(const char* path, se_asset_info* out, std::string& err);
    int32_t asset_release(uint64_t id);
    int32_t asset_get_info(uint64_t id, se_asset_info* out) const;
    int32_t asset_compute_peaks(uint64_t id, uint64_t start_frame, uint32_t frames_per_peak, float* out_minmax,
                                std::size_t capacity, std::size_t* out_count) const;
    // Registers already-decoded audio (resampling to the engine rate if needed).
    int32_t asset_add_decoded(DecodedAudio&& audio, se_asset_info* out, std::string& err);

    // ---- session
    int32_t publish(const se_session_desc& desc, std::string& err);
    // Frees retired graphs the audio thread can no longer reach. Non-RT.
    void collect_garbage();
    std::size_t retired_graph_count() const;

    // ---- transport
    int32_t push_command(const TransportCommand& cmd);
    void transport_get(se_transport_info* out) const noexcept;

    // ---- meters / stats
    int32_t meters_read(se_meter* out, std::size_t capacity, std::size_t* out_count);
    void stats_get(se_stats* out) const noexcept;
    void stats_reset() noexcept;

    // ---- offline
    int32_t render_offline(float* out_interleaved, uint32_t frames, std::string& err);
    int32_t export_wav(const char* path, int64_t start_frame, int64_t end_frame, std::string& err);

private:
    Engine(double sample_rate, uint32_t max_block, uint32_t channels);

    // AudioDevice::Callback (device RT thread).
    void render(float* const* channels, uint32_t num_channels, uint32_t frames) noexcept override;

    // The single RT entry point used by the device callback and
    // render_offline. `sink(l, r, offset, n)` receives each rendered chunk.
    template <class Sink>
    void process(uint32_t frames, Sink&& sink) noexcept;

    void reclaim_locked(); // requires control_mutex_

    // Who currently owns the live render path (RT role).
    enum : int { kOwnerNone = 0, kOwnerDevice = 1, kOwnerOffline = 2 };

    const double sample_rate_;
    const uint32_t max_block_;
    const uint32_t channels_;

    // -- assets (non-RT)
    mutable std::mutex assets_mutex_;
    std::unordered_map<uint64_t, AssetRef> assets_;
    uint64_t next_asset_id_ = 1;

    // -- graph publication
    mutable std::mutex control_mutex_; // guards everything below up to live_graph_
    std::shared_ptr<RenderGraph> current_;                 // owner of the published graph
    std::vector<std::shared_ptr<RenderGraph>> retired_;    // waiting for the RT side to let go
    uint64_t next_seq_ = 1;
    std::atomic<RenderGraph*> live_graph_{nullptr};        // what the RT side loads
    // RT-side "hazard" marker: kIdle between blocks, kBusy while acquiring,
    // otherwise the seq of the graph in use. See reclaim_locked().
    static constexpr uint64_t kIdle = 0;
    static constexpr uint64_t kBusy = ~uint64_t{0};
    std::atomic<uint64_t> rt_active_seq_{kIdle};

    // -- device
    std::mutex device_mutex_;
    std::unique_ptr<AudioDevice> device_;
    std::atomic<int> rt_owner_{kOwnerNone};

    // -- transport: commands in, state out
    std::mutex command_mutex_; // serialises producers (SPSC producer side)
    SpscQueue<TransportCommand, kCommandQueueSize> commands_;
    int64_t rt_position_ = 0; // RT-owned
    bool rt_playing_ = false; // RT-owned
    std::atomic<int64_t> pub_position_{0};
    std::atomic<bool> pub_playing_{false};

    // -- meters (master; track meters live in the graph)
    std::atomic<float> master_peak_l_{0.0f};
    std::atomic<float> master_peak_r_{0.0f};

    // -- stats (written by RT only, except reset)
    std::atomic<uint64_t> stat_callbacks_{0};
    std::atomic<uint64_t> stat_overloads_{0};
    std::atomic<uint64_t> stat_last_ns_{0};
    std::atomic<uint64_t> stat_max_ns_{0};
    std::atomic<uint64_t> stat_total_ns_{0};
    std::atomic<uint64_t> stat_deadline_ns_{0};

    // -- RT scratch (used by whoever holds rt_owner_)
    RenderContext live_ctx_;
};

} // namespace saine
