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

#include <array>
#include <atomic>
#include <cstdint>
#include <memory>
#include <mutex>
#include <string>
#include <unordered_map>
#include <vector>

namespace saine {

// Everything that changes RT-owned playback state goes through this one
// trivially copyable command (queued while the device runs, applied inline
// otherwise; see Engine::push_command()).
struct TransportCommand {
    enum class Type : uint8_t { Play, Pause, Stop, Seek, SetLoop, SetMetronome };
    Type type = Type::Play;
    bool enabled = false;   // SetLoop, SetMetronome
    float gain = 0.0f;      // SetMetronome
    uint32_t beats_per_bar = 0; // SetMetronome
    int64_t frame = 0;      // Seek target, SetLoop start
    int64_t frame2 = 0;     // SetLoop end
    double frames_per_beat = 0.0; // SetMetronome
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
    int32_t publish(const se_session_desc_v2& desc, std::string& err);
    int32_t publish(const se_session_desc_v3& desc, std::string& err);
    // Frees retired graphs the audio thread can no longer reach. Non-RT.
    void collect_garbage();
    std::size_t retired_graph_count() const;

    // ---- transport
    // Validated by the caller (c_api.cpp). Applied immediately when nobody
    // owns the RT path, otherwise queued for the next block.
    int32_t push_command(const TransportCommand& cmd);
    void transport_get(se_transport_info* out) const noexcept;

    // ---- meters / stats
    int32_t meters_read(se_meter* out, std::size_t capacity, std::size_t* out_count);
    int32_t meters_peek(se_meter* out, std::size_t capacity, std::size_t* out_count) const;
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
    // Assigns synth states (reusing the previous graph's by track_id) and
    // publishes the graph to the RT side.
    int32_t install_graph(std::unique_ptr<RenderGraph> built);

    // Who currently owns the live render path (RT role). Exactly one owner
    // may touch the rt_* state and pop the command queue. kOwnerControl is a
    // short-lived claim taken by a non-RT thread to apply queued commands
    // synchronously when no device or offline render owns the path.
    enum : int { kOwnerNone = 0, kOwnerDevice = 1, kOwnerOffline = 2, kOwnerControl = 3 };

    // CAS rt_owner_ None -> `who`, waiting out transient kOwnerControl
    // claims. Returns the conflicting owner (Device/Offline) or kOwnerNone
    // on success.
    int acquire_rt_owner(int who) noexcept;
    // Store None, then apply any commands that were queued while we owned the
    // path (Dekker-style handshake with push_command, see engine.cpp).
    void release_rt_owner() noexcept;

    // ---- RT-owner-only helpers (caller holds rt_owner_)
    void apply_command(const TransportCommand& cmd) noexcept;
    void drain_commands() noexcept;
    void publish_transport() noexcept;
    uint32_t advance_peek_window(uint32_t frames, const RenderGraph* graph) noexcept;
    void mix_metronome(float* l, float* r, int64_t position, uint32_t frames) noexcept;
    void start_click(const float* buf, uint32_t offset, float* l, float* r, uint32_t frames) noexcept;

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
    bool rt_loop_enabled_ = false;  // RT-owned
    int64_t rt_loop_start_ = 0;     // RT-owned
    int64_t rt_loop_end_ = 0;       // RT-owned
    // Bumped on every playback jump (seek, stop, pause, loop wrap); synths
    // fade their voices when it changes. RT-owned.
    uint64_t rt_synth_gen_ = 0;
    std::atomic<int64_t> pub_position_{0};
    std::atomic<bool> pub_playing_{false};

    // -- metronome. Click shapes are computed in the constructor; the RT
    // side keeps a few voices so a click's tail continues across blocks,
    // loop wraps and seeks.
    struct ClickVoice {
        const float* buf = nullptr;
        uint32_t pos = 0; // next sample index into buf
    };
    static constexpr std::size_t kMaxClickVoices = 8;
    std::vector<float> click_down_; // downbeat (1.2 kHz)
    std::vector<float> click_beat_; // other beats (800 Hz)
    uint32_t click_len_ = 0;
    bool rt_metro_enabled_ = false;     // RT-owned
    float rt_metro_gain_ = 0.0f;        // RT-owned
    double rt_metro_fpb_ = 0.0;         // RT-owned
    uint32_t rt_metro_bpb_ = 1;         // RT-owned
    std::array<ClickVoice, kMaxClickVoices> rt_voices_{}; // RT-owned
    std::size_t rt_voice_count_ = 0;    // RT-owned

    // -- meters (master; track meters live in the graph)
    std::atomic<float> master_peak_l_{0.0f};
    std::atomic<float> master_peak_r_{0.0f};
    // se_meters_peek window: kPeekBuckets buckets of peek_bucket_frames_.
    std::atomic<float> master_win_l_[kPeekBuckets] = {};
    std::atomic<float> master_win_r_[kPeekBuckets] = {};
    uint32_t peek_bucket_frames_ = 1;
    uint32_t rt_peek_slot_ = 0;   // RT-owned
    uint64_t rt_peek_fill_ = 0;   // RT-owned: frames in the current bucket

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
