#include "engine.hpp"

#include "resampler.hpp"
#include "rt_util.hpp"
#include "wav.hpp"

#include <algorithm>
#include <cmath>
#include <cstring>
#include <limits>

namespace saine {

// ============================================================ lifecycle

Engine::Engine(double sample_rate, uint32_t max_block, uint32_t channels)
    : sample_rate_(sample_rate), max_block_(max_block), channels_(channels), live_ctx_(max_block) {}

std::unique_ptr<Engine> Engine::create(const se_config& config, std::string& err) {
    if (config.abi_version != SE_ABI_VERSION) {
        err = "ABI version mismatch: header " + std::to_string(SE_ABI_VERSION) + ", caller " +
              std::to_string(config.abi_version);
        return nullptr;
    }
    double rate = config.sample_rate;
    if (rate <= 0.0) {
        // 0 = "use the device's rate". Fixed for the engine's lifetime because
        // assets are resampled to it on import.
        rate = query_default_output_sample_rate();
        if (rate <= 0.0) rate = 48000.0;
    }
    if (!std::isfinite(rate) || rate < 8000.0 || rate > 768000.0) {
        err = "unsupported sample rate";
        return nullptr;
    }
    if (config.max_block_size == 0 || config.max_block_size > kMaxBlockLimit) {
        err = "max_block_size must be in 1.." + std::to_string(kMaxBlockLimit);
        return nullptr;
    }
    if (config.output_channels != 2) {
        err = "only output_channels == 2 is supported in this milestone";
        return nullptr;
    }
    return std::unique_ptr<Engine>(new Engine(rate, config.max_block_size, config.output_channels));
}

Engine::~Engine() {
    device_stop();
    // Nothing can be on the RT path any more (contract: no calls race with
    // destroy). Members are destroyed normally on this (non-RT) thread.
}

// ============================================================ device

int32_t Engine::device_start(std::string& err) {
    std::lock_guard lock(device_mutex_);
    int expected = kOwnerNone;
    if (!rt_owner_.compare_exchange_strong(expected, kOwnerDevice)) {
        err = expected == kOwnerDevice ? "device already running" : "offline render in progress";
        return SE_ERR_STATE;
    }
    if (!device_) device_ = create_platform_audio_device();
    AudioDevice::Config cfg;
    cfg.sample_rate = sample_rate_;
    cfg.channels = channels_;
    cfg.preferred_block = max_block_;
    const int32_t rc = device_->start(cfg, this, err);
    if (rc != SE_OK) {
        rt_owner_.store(kOwnerNone);
        return rc;
    }
    return SE_OK;
}

int32_t Engine::device_stop() {
    std::lock_guard lock(device_mutex_);
    if (rt_owner_.load() != kOwnerDevice) return SE_OK;
    if (device_) device_->stop(); // returns once the callback can no longer run
    rt_owner_.store(kOwnerNone);
    return SE_OK;
}

bool Engine::device_running() const noexcept {
    return rt_owner_.load(std::memory_order_acquire) == kOwnerDevice;
}

// ============================================================ assets

int32_t Engine::asset_add_decoded(DecodedAudio&& audio, se_asset_info* out, std::string& err) {
    if (audio.channels < 1 || audio.channels > 2) {
        err = "asset must be mono or stereo";
        return SE_ERR_UNSUPPORTED_FORMAT;
    }
    auto asset = std::make_shared<Asset>();
    asset->channels = audio.channels;
    asset->source_sample_rate = audio.sample_rate;
    if (audio.sample_rate == sample_rate_) {
        for (uint32_t c = 0; c < audio.channels; ++c) asset->planar[c] = std::move(audio.planar[c]);
        asset->frames = audio.frames;
    } else {
        const SincResampler rs(audio.sample_rate, sample_rate_);
        for (uint32_t c = 0; c < audio.channels; ++c) {
            rs.process(audio.planar[c].data(), audio.frames, asset->planar[c]);
            audio.planar[c] = {};
        }
        asset->frames = asset->planar[0].size();
    }

    std::lock_guard lock(assets_mutex_);
    asset->id = next_asset_id_++;
    if (out != nullptr) {
        out->asset_id = asset->id;
        out->frames = asset->frames;
        out->channels = asset->channels;
        out->source_sample_rate = asset->source_sample_rate;
    }
    assets_.emplace(asset->id, std::move(asset));
    return SE_OK;
}

int32_t Engine::asset_load_file(const char* path, se_asset_info* out, std::string& err) {
    DecodedAudio audio;
    const int32_t rc = decode_wav_file(path, audio, err);
    if (rc != SE_OK) return rc;
    return asset_add_decoded(std::move(audio), out, err);
}

int32_t Engine::asset_release(uint64_t id) {
    AssetRef victim; // dropped outside the lock
    {
        std::lock_guard lock(assets_mutex_);
        auto it = assets_.find(id);
        if (it == assets_.end()) return SE_ERR_NOT_FOUND;
        victim = std::move(it->second);
        assets_.erase(it);
    }
    // Published/retired graphs may still hold the asset; it is freed when the
    // last of them is reclaimed (always on a non-RT thread).
    collect_garbage();
    return SE_OK;
}

int32_t Engine::asset_get_info(uint64_t id, se_asset_info* out) const {
    std::lock_guard lock(assets_mutex_);
    auto it = assets_.find(id);
    if (it == assets_.end()) return SE_ERR_NOT_FOUND;
    const Asset& a = *it->second;
    out->asset_id = a.id;
    out->frames = a.frames;
    out->channels = a.channels;
    out->source_sample_rate = a.source_sample_rate;
    return SE_OK;
}

int32_t Engine::asset_compute_peaks(uint64_t id, uint64_t start_frame, uint32_t frames_per_peak, float* out_minmax,
                                    std::size_t capacity, std::size_t* out_count) const {
    AssetRef asset;
    {
        std::lock_guard lock(assets_mutex_);
        auto it = assets_.find(id);
        if (it == assets_.end()) return SE_ERR_NOT_FOUND;
        asset = it->second;
    }
    std::size_t count = 0;
    const float* l = asset->channel(0);
    const float* r = asset->channel(1);
    const bool stereo = asset->channels == 2;
    for (uint64_t f = start_frame; count < capacity && f < asset->frames; f += frames_per_peak) {
        const uint64_t end = std::min<uint64_t>(asset->frames, f + frames_per_peak);
        float lo = std::numeric_limits<float>::infinity();
        float hi = -std::numeric_limits<float>::infinity();
        for (uint64_t i = f; i < end; ++i) {
            const float v = stereo ? 0.5f * (l[i] + r[i]) : l[i];
            lo = std::min(lo, v);
            hi = std::max(hi, v);
        }
        out_minmax[2 * count] = lo;
        out_minmax[2 * count + 1] = hi;
        ++count;
    }
    if (out_count != nullptr) *out_count = count;
    return SE_OK;
}

// ============================================================ session

int32_t Engine::publish(const se_session_desc& desc, std::string& err) {
    // Build entirely off the RT thread. Asset lookups take assets_mutex_.
    std::unique_ptr<RenderGraph> built;
    const AssetLookup lookup = [this](uint64_t id) -> AssetRef {
        std::lock_guard lock(assets_mutex_);
        auto it = assets_.find(id);
        return it == assets_.end() ? nullptr : it->second;
    };
    const int32_t rc = build_render_graph(desc, lookup, built, err);
    if (rc != SE_OK) return rc;

    std::shared_ptr<RenderGraph> next(std::move(built));
    std::lock_guard lock(control_mutex_);
    next->seq = next_seq_++;
    live_graph_.store(next.get(), std::memory_order_seq_cst);
    if (current_) retired_.push_back(std::move(current_));
    current_ = std::move(next);
    reclaim_locked();
    return SE_OK;
}

// Reclamation protocol (single RT reader, wait-free on the RT side):
//   RT block start:  rt_active_seq_ = kBusy;  g = live_graph_;  rt_active_seq_ = g->seq
//   RT block end:    rt_active_seq_ = kIdle
// A retired graph R (already swapped out of live_graph_) may be freed when
//   active == kIdle            -> RT holds nothing; the next load sees the new graph
//   active != kBusy && active > R.seq -> RT holds a newer graph and can never
//                                 reach R again (it only ever loads live_graph_)
// All five operations are seq_cst, so if RT loaded R before the swap, its
// kBusy store precedes our read of rt_active_seq_ and we observe kBusy, R.seq
// (or older), or kIdle only after RT finished with R.
void Engine::reclaim_locked() {
    if (retired_.empty()) return;
    const uint64_t active = rt_active_seq_.load(std::memory_order_seq_cst);
    if (active == kBusy) return;
    std::erase_if(retired_, [active](const std::shared_ptr<RenderGraph>& g) {
        return active == kIdle || active > g->seq;
    });
}

void Engine::collect_garbage() {
    std::lock_guard lock(control_mutex_);
    reclaim_locked();
}

std::size_t Engine::retired_graph_count() const {
    std::lock_guard lock(control_mutex_);
    return retired_.size();
}

// ============================================================ transport

int32_t Engine::push_command(const TransportCommand& cmd) {
    std::lock_guard lock(command_mutex_);
    return commands_.push(cmd) ? SE_OK : SE_ERR_STATE;
}

void Engine::transport_get(se_transport_info* out) const noexcept {
    *out = se_transport_info{};
    out->position = pub_position_.load(std::memory_order_acquire);
    out->playing = pub_playing_.load(std::memory_order_acquire) ? 1 : 0;
}

// ============================================================ meters / stats

int32_t Engine::meters_read(se_meter* out, std::size_t capacity, std::size_t* out_count) {
    std::size_t n = 0;
    if (capacity > 0) {
        out[0].track_id = 0;
        out[0].peak_l = master_peak_l_.exchange(0.0f, std::memory_order_relaxed);
        out[0].peak_r = master_peak_r_.exchange(0.0f, std::memory_order_relaxed);
        n = 1;
    }
    std::lock_guard lock(control_mutex_); // keeps current_ alive while reading
    if (current_) {
        for (std::size_t i = 0; i < current_->meter_count && n < capacity; ++i, ++n) {
            TrackMeter& m = current_->meters[i];
            out[n].track_id = m.track_id;
            out[n].peak_l = m.peak_l.exchange(0.0f, std::memory_order_relaxed);
            out[n].peak_r = m.peak_r.exchange(0.0f, std::memory_order_relaxed);
        }
    }
    if (out_count != nullptr) *out_count = n;
    return SE_OK;
}

void Engine::stats_get(se_stats* out) const noexcept {
    const uint64_t calls = stat_callbacks_.load(std::memory_order_relaxed);
    out->callbacks = calls;
    out->xruns_or_overloads = stat_overloads_.load(std::memory_order_relaxed);
    out->last_callback_us = static_cast<double>(stat_last_ns_.load(std::memory_order_relaxed)) / 1000.0;
    out->max_callback_us = static_cast<double>(stat_max_ns_.load(std::memory_order_relaxed)) / 1000.0;
    out->avg_callback_us =
        calls ? static_cast<double>(stat_total_ns_.load(std::memory_order_relaxed)) / 1000.0 / static_cast<double>(calls)
              : 0.0;
    out->block_deadline_us = static_cast<double>(stat_deadline_ns_.load(std::memory_order_relaxed)) / 1000.0;
}

void Engine::stats_reset() noexcept {
    stat_callbacks_.store(0, std::memory_order_relaxed);
    stat_overloads_.store(0, std::memory_order_relaxed);
    stat_last_ns_.store(0, std::memory_order_relaxed);
    stat_max_ns_.store(0, std::memory_order_relaxed);
    stat_total_ns_.store(0, std::memory_order_relaxed);
    stat_deadline_ns_.store(0, std::memory_order_relaxed);
}

// ============================================================ RT process path

template <class Sink>
void Engine::process(uint32_t frames, Sink&& sink) noexcept {
    const uint64_t t0 = now_ns();
    ScopedFlushDenormals ftz;

    // 1. Transport commands, applied at block start.
    TransportCommand cmd;
    while (commands_.pop(cmd)) {
        switch (cmd.type) {
        case TransportCommand::Type::Play: rt_playing_ = true; break;
        case TransportCommand::Type::Pause: rt_playing_ = false; break;
        case TransportCommand::Type::Stop:
            rt_playing_ = false;
            rt_position_ = 0;
            break;
        case TransportCommand::Type::Seek: rt_position_ = cmd.frame; break;
        }
    }

    // 2. Acquire the published graph (see reclaim_locked()).
    rt_active_seq_.store(kBusy, std::memory_order_seq_cst);
    const RenderGraph* graph = live_graph_.load(std::memory_order_seq_cst);
    if (graph != nullptr) rt_active_seq_.store(graph->seq, std::memory_order_seq_cst);

    // 3. Render in chunks of at most max_block frames.
    float peak_l = 0.0f, peak_r = 0.0f;
    for (uint32_t done = 0; done < frames;) {
        const uint32_t n = std::min(frames - done, max_block_);
        const BlockPeaks p = render_block(graph, rt_position_, rt_playing_, n, live_ctx_, true);
        peak_l = std::max(peak_l, p.l);
        peak_r = std::max(peak_r, p.r);
        sink(live_ctx_.master_l(), live_ctx_.master_r(), done, n);
        if (rt_playing_) rt_position_ += n;
        done += n;
    }

    // 4. Release the graph, publish state.
    rt_active_seq_.store(kIdle, std::memory_order_seq_cst);
    pub_position_.store(rt_position_, std::memory_order_release);
    pub_playing_.store(rt_playing_, std::memory_order_release);
    atomic_store_max(master_peak_l_, peak_l);
    atomic_store_max(master_peak_r_, peak_r);

    // 5. Timing stats.
    const uint64_t elapsed = now_ns() - t0;
    const auto deadline = static_cast<uint64_t>(static_cast<double>(frames) * 1e9 / sample_rate_);
    stat_callbacks_.fetch_add(1, std::memory_order_relaxed);
    stat_last_ns_.store(elapsed, std::memory_order_relaxed);
    stat_total_ns_.fetch_add(elapsed, std::memory_order_relaxed);
    stat_deadline_ns_.store(deadline, std::memory_order_relaxed);
    if (elapsed > stat_max_ns_.load(std::memory_order_relaxed)) stat_max_ns_.store(elapsed, std::memory_order_relaxed);
    if (elapsed > deadline) stat_overloads_.fetch_add(1, std::memory_order_relaxed);
}

void Engine::render(float* const* channels, uint32_t num_channels, uint32_t frames) noexcept {
    process(frames, [&](const float* l, const float* r, uint32_t offset, uint32_t n) {
        if (num_channels == 0) return;
        if (num_channels == 1) {
            float* dst = channels[0] + offset;
            for (uint32_t i = 0; i < n; ++i) dst[i] = 0.5f * (l[i] + r[i]);
            return;
        }
        std::memcpy(channels[0] + offset, l, sizeof(float) * n);
        std::memcpy(channels[1] + offset, r, sizeof(float) * n);
        for (uint32_t c = 2; c < num_channels; ++c) std::memset(channels[c] + offset, 0, sizeof(float) * n);
    });
}

int32_t Engine::render_offline(float* out_interleaved, uint32_t frames, std::string& err) {
    if (frames == 0) return SE_OK;
    int expected = kOwnerNone;
    if (!rt_owner_.compare_exchange_strong(expected, kOwnerOffline)) {
        err = expected == kOwnerDevice ? "cannot render offline while the device is running"
                                       : "another offline render is in progress";
        return SE_ERR_STATE;
    }
    process(frames, [out_interleaved](const float* l, const float* r, uint32_t offset, uint32_t n) {
        float* dst = out_interleaved + static_cast<std::size_t>(offset) * 2;
        for (uint32_t i = 0; i < n; ++i) {
            dst[2 * i] = l[i];
            dst[2 * i + 1] = r[i];
        }
    });
    rt_owner_.store(kOwnerNone, std::memory_order_release);
    return SE_OK;
}

// Export renders with a private context and transport so it never disturbs
// live playback, meters or stats. It shares render_block() with the RT path.
int32_t Engine::export_wav(const char* path, int64_t start_frame, int64_t end_frame, std::string& err) {
    if (end_frame <= start_frame) {
        err = "export range is empty";
        return SE_ERR_INVALID_ARG;
    }
    std::shared_ptr<RenderGraph> graph;
    {
        std::lock_guard lock(control_mutex_);
        graph = current_; // keeps the graph (and its assets) alive for the export
    }
    if (sample_rate_ != std::floor(sample_rate_) || sample_rate_ > 4294967295.0) {
        err = "engine sample rate is not representable in a WAV header";
        return SE_ERR_UNSUPPORTED_FORMAT;
    }

    WavFloatWriter writer;
    int32_t rc = writer.open(path, 2, static_cast<uint32_t>(sample_rate_), err);
    if (rc != SE_OK) return rc;

    RenderContext ctx(max_block_);
    std::vector<float> interleaved(static_cast<std::size_t>(max_block_) * 2);
    ScopedFlushDenormals ftz;
    for (int64_t pos = start_frame; pos < end_frame;) {
        const auto n = static_cast<uint32_t>(std::min<int64_t>(end_frame - pos, max_block_));
        render_block(graph.get(), pos, true, n, ctx, false);
        const float* l = ctx.master_l();
        const float* r = ctx.master_r();
        for (uint32_t i = 0; i < n; ++i) {
            interleaved[2 * i] = l[i];
            interleaved[2 * i + 1] = r[i];
        }
        rc = writer.write(interleaved.data(), n, err);
        if (rc != SE_OK) return rc;
        pos += n;
    }
    return writer.finish(err);
}

} // namespace saine
