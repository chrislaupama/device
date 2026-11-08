#include "engine.hpp"

#include "resampler.hpp"
#include "rt_util.hpp"
#include "wav.hpp"

#include <algorithm>
#include <cmath>
#include <cstring>
#include <limits>
#include <chrono>
#include <numbers>
#include <thread>

namespace saine {

namespace {

// ~15 ms exponentially decaying sine click. Phase starts half a sample in so
// the very first sample is non-zero (the onset frame is the beat frame); a
// short raised-cosine fade makes the last sample exactly reach zero.
std::vector<float> make_click(double sample_rate, double freq, std::size_t len) {
    std::vector<float> v(len);
    const double tau = 0.003 * sample_rate; // decay time constant (frames)
    const std::size_t fade = std::max<std::size_t>(1, static_cast<std::size_t>(0.002 * sample_rate));
    for (std::size_t i = 0; i < len; ++i) {
        const double t = static_cast<double>(i);
        double env = std::exp(-t / tau);
        const std::size_t left = len - 1 - i;
        if (left < fade) env *= 0.5 - 0.5 * std::cos(std::numbers::pi * static_cast<double>(left) / static_cast<double>(fade));
        v[i] = static_cast<float>(std::sin(2.0 * std::numbers::pi * freq * (t + 0.5) / sample_rate) * env);
    }
    return v;
}

int64_t floor_mod(int64_t a, int64_t m) noexcept {
    const int64_t r = a % m;
    return r < 0 ? r + m : r;
}

} // namespace

// ============================================================ lifecycle

Engine::Engine(double sample_rate, uint32_t max_block, uint32_t channels)
    : sample_rate_(sample_rate), max_block_(max_block), channels_(channels), live_ctx_(max_block) {
    // Preallocated here so neither se_metronome_set nor the callback allocates.
    const auto len = static_cast<std::size_t>(std::llround(0.015 * sample_rate));
    click_down_ = make_click(sample_rate, 1200.0, len);
    click_beat_ = make_click(sample_rate, 800.0, len);
    click_len_ = static_cast<uint32_t>(len);
    peek_bucket_frames_ = std::max<uint32_t>(1, static_cast<uint32_t>(std::llround(0.010 * sample_rate)));
    in_scratch_.assign(static_cast<std::size_t>(kMaxInjectedInputChannels) * max_block, 0.0f);
    device_factory_ = [] { return create_platform_audio_device(); };
}

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
    {
        // Nothing renders any more; finalize a recording still in progress.
        std::lock_guard lock(rec_mutex_);
        rec_live_.store(nullptr, std::memory_order_seq_cst);
        if (recorder_) recorder_->finish(last_takes_);
        recorder_.reset();
    }
    // Nothing can be on the RT path any more (contract: no calls race with
    // destroy). Members are destroyed normally on this (non-RT) thread.
}

// ============================================================ device

int32_t Engine::device_start(std::string& err) {
    std::lock_guard lock(device_mutex_);
    const int conflict = acquire_rt_owner(kOwnerDevice);
    if (conflict != kOwnerNone) {
        err = conflict == kOwnerDevice ? "device already running" : "offline render in progress";
        return SE_ERR_STATE;
    }
    if (!device_) device_ = device_factory_();
    AudioDevice::Config cfg = device_setup_; // defaults: default output, no input
    cfg.sample_rate = sample_rate_;
    cfg.channels = channels_;
    cfg.preferred_block = max_block_;
    const int32_t rc = device_->start(cfg, this, err);
    if (rc != SE_OK) {
        device_input_open_.store(false);
        release_rt_owner();
        return rc;
    }
    refresh_device_latency();
    input_error_ = device_->status().input_error;
    return SE_OK;
}

// Requires device_mutex_ and a started device.
void Engine::refresh_device_latency() {
    const AudioDevice::Status st = device_->status();
    const double ratio = st.device_rate > 0.0 ? sample_rate_ / st.device_rate : 1.0;
    const int64_t lat =
        st.input_open ? std::llround(static_cast<double>(st.input_latency + st.output_latency) * ratio) : 0;
    device_rec_latency_.store(lat, std::memory_order_relaxed);
    device_input_open_.store(st.input_open, std::memory_order_release);
}

int32_t Engine::device_configure(const AudioDevice::Config& setup, std::string& err) {
    std::lock_guard lock(device_mutex_);
    if (!device_) device_ = device_factory_();
    AudioDevice::Config cfg = setup;
    cfg.sample_rate = sample_rate_;
    cfg.channels = channels_;
    cfg.preferred_block = max_block_;
    int32_t rc = device_->validate(cfg, err); // opens nothing
    if (rc != SE_OK) return rc;
    device_setup_ = cfg;
    input_error_.clear();
    if (rt_owner_.load() != kOwnerDevice) return SE_OK; // applied by the next se_device_start
    // Restart in place: we keep the RT ownership across stop/start, so no
    // offline render can sneak in; queued commands wait for the new stream.
    device_->stop();
    rc = device_->start(cfg, this, err);
    if (rc != SE_OK) {
        device_input_open_.store(false);
        release_rt_owner();
        return rc;
    }
    refresh_device_latency();
    const AudioDevice::Status st = device_->status();
    input_error_ = st.input_error;
    if (cfg.input_enabled && !st.input_open) {
        err = "output restarted, but the input could not be opened: " + st.input_error;
        return SE_ERR_DEVICE;
    }
    return SE_OK;
}

namespace {
void copy_str(char* dst, std::size_t cap, const std::string& src) {
    const std::size_t n = std::min(cap - 1, src.size());
    std::memcpy(dst, src.data(), n);
    dst[n] = '\0';
}
} // namespace

void Engine::device_status(se_device_status* out) {
    std::lock_guard lock(device_mutex_);
    *out = se_device_status{};
    out->sample_rate = sample_rate_;
    copy_str(out->input_error, sizeof out->input_error, input_error_);
    if (!device_ || rt_owner_.load() != kOwnerDevice) return;
    const AudioDevice::Status st = device_->status();
    const double ratio = st.device_rate > 0.0 ? sample_rate_ / st.device_rate : 1.0;
    out->running = 1;
    out->input_open = st.input_open ? 1 : 0;
    copy_str(out->output_name, sizeof out->output_name, st.output_name);
    if (st.input_open) copy_str(out->input_name, sizeof out->input_name, st.input_name);
    if (st.device_rate > 0.0) out->sample_rate = st.device_rate;
    out->buffer_size = st.buffer_size;
    out->input_channels = st.input_channels;
    out->output_channels = st.output_channels;
    out->input_latency_frames = static_cast<uint32_t>(std::llround(st.input_latency * ratio));
    out->output_latency_frames = static_cast<uint32_t>(std::llround(st.output_latency * ratio));
}

void Engine::set_device_factory(DeviceFactory f) {
    std::lock_guard lock(device_mutex_);
    device_factory_ = std::move(f);
    if (rt_owner_.load() != kOwnerDevice) device_.reset();
}

int32_t Engine::device_stop() {
    std::lock_guard lock(device_mutex_);
    if (rt_owner_.load() != kOwnerDevice) return SE_OK;
    if (device_) device_->stop(); // returns once the callback can no longer run
    device_input_open_.store(false);
    // Commands queued but not yet consumed by a callback are applied here.
    release_rt_owner();
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
    return install_graph(std::move(built));
}

int32_t Engine::publish(const se_session_desc_v2& desc, std::string& err) {
    std::unique_ptr<RenderGraph> built;
    const AssetLookup lookup = [this](uint64_t id) -> AssetRef {
        std::lock_guard lock(assets_mutex_);
        auto it = assets_.find(id);
        return it == assets_.end() ? nullptr : it->second;
    };
    const int32_t rc = build_render_graph(desc, lookup, built, err);
    if (rc != SE_OK) return rc;
    return install_graph(std::move(built));
}

int32_t Engine::publish(const se_session_desc_v3& desc, std::string& err) {
    std::unique_ptr<RenderGraph> built;
    const AssetLookup lookup = [this](uint64_t id) -> AssetRef {
        std::lock_guard lock(assets_mutex_);
        auto it = assets_.find(id);
        return it == assets_.end() ? nullptr : it->second;
    };
    const int32_t rc = build_render_graph(desc, lookup, built, err);
    if (rc != SE_OK) return rc;
    return install_graph(std::move(built));
}

int32_t Engine::publish(const se_session_desc_v4& desc, std::string& err) {
    std::unique_ptr<RenderGraph> built;
    const AssetLookup lookup = [this](uint64_t id) -> AssetRef {
        std::lock_guard lock(assets_mutex_);
        auto it = assets_.find(id);
        return it == assets_.end() ? nullptr : it->second;
    };
    const int32_t rc = build_render_graph(desc, lookup, built, err);
    if (rc != SE_OK) return rc;
    return install_graph(std::move(built));
}

// Synth states outlive graph swaps: an instrument track takes over the Synth
// of the same track_id in the currently published graph (the RT side keeps
// mutating that object; we only copy the reference). New instrument tracks
// get a fresh Synth, allocated here. A Synth whose track disappeared stays
// owned by the retired graph(s) and is freed when they are reclaimed, which
// happens only on non-RT threads once the RT side can no longer reach them.
int32_t Engine::install_graph(std::unique_ptr<RenderGraph> built) {
    std::shared_ptr<RenderGraph> next(std::move(built));
    std::lock_guard lock(control_mutex_);
    std::unordered_map<uint64_t, std::shared_ptr<Synth>> previous;
    if (current_)
        for (std::size_t i = 0; i < current_->tracks.size(); ++i)
            if (current_->synth_refs[i]) previous.emplace(current_->tracks[i].track_id, current_->synth_refs[i]);
    for (std::size_t i = 0; i < next->tracks.size(); ++i) {
        if (!next->tracks[i].instrument) continue;
        std::shared_ptr<Synth> synth;
        if (auto it = previous.find(next->tracks[i].track_id); it != previous.end()) {
            synth = std::move(it->second);
            previous.erase(it); // a duplicated track_id must not share one Synth
        } else {
            synth = std::make_shared<Synth>(sample_rate_);
        }
        next->synths[i] = synth.get();
        next->synth_refs[i] = std::move(synth);
    }

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

// ---- RT ownership and synchronous (offline) command application
//
// rt_owner_ says who may touch rt_* state and pop the queue: the device
// callback (kOwnerDevice), an se_render_offline call (kOwnerOffline), or a
// non-RT thread applying commands inline (kOwnerControl). Ownership changes
// only through seq_cst RMW/stores, which also order the plain rt_* fields
// between successive owners.
//
// Every command is first pushed onto the SPSC queue, then the producer tries
// to claim the path (None -> Control). If it succeeds it drains the queue
// itself (so the command applies before push_command returns) and releases.
// If the device or an offline render owns the path, the command stays queued
// and that owner applies it at its next block start or when it releases.
//
// Lost-wakeup freedom: an owner releasing the path does
//     owner.store(None); fence; if (!queue.empty()) claim + drain
// and a producer does
//     queue.push(cmd);   fence; claim (CAS None -> Control) + drain
// Both fences are seq_cst, so (store-buffering argument) either the
// producer's CAS observes None and drains, or the releaser's empty() check
// observes the push and drains. A command can therefore never be stranded in
// the queue with nobody owning the path. kOwnerControl is held only for the
// few loads/stores of a drain; other claimants yield until it is gone.
int Engine::acquire_rt_owner(int who) noexcept {
    for (;;) {
        int expected = kOwnerNone;
        if (rt_owner_.compare_exchange_strong(expected, who, std::memory_order_seq_cst)) return kOwnerNone;
        if (expected != kOwnerControl) return expected;
        std::this_thread::yield();
    }
}

void Engine::release_rt_owner() noexcept {
    for (;;) {
        rt_owner_.store(kOwnerNone, std::memory_order_seq_cst);
        std::atomic_thread_fence(std::memory_order_seq_cst);
        if (commands_.empty()) return;
        int expected = kOwnerNone;
        // If somebody else claimed the path in the meantime, they drain.
        if (!rt_owner_.compare_exchange_strong(expected, kOwnerControl, std::memory_order_seq_cst)) return;
        drain_commands();
        publish_transport();
    }
}

int32_t Engine::push_command(const TransportCommand& cmd) {
    std::lock_guard lock(command_mutex_); // producers stay serialised (SPSC)
    if (!commands_.push(cmd)) return SE_ERR_STATE;
    std::atomic_thread_fence(std::memory_order_seq_cst);
    if (acquire_rt_owner(kOwnerControl) == kOwnerNone) {
        drain_commands();
        publish_transport();
        release_rt_owner();
    }
    return SE_OK;
}

void Engine::apply_command(const TransportCommand& cmd) noexcept {
    switch (cmd.type) {
    case TransportCommand::Type::Play: rt_playing_ = true; break;
    case TransportCommand::Type::Pause:
        rt_playing_ = false;
        rt_voice_count_ = 0;
        ++rt_synth_gen_;
        break;
    case TransportCommand::Type::Stop:
        rt_playing_ = false;
        rt_position_ = 0;
        rt_voice_count_ = 0;
        ++rt_synth_gen_;
        break;
    case TransportCommand::Type::Seek: // click tails ring on; synth voices fade
        rt_position_ = cmd.frame;
        ++rt_synth_gen_;
        break;
    case TransportCommand::Type::SetLoop:
        rt_loop_enabled_ = cmd.enabled;
        if (cmd.enabled) {
            rt_loop_start_ = cmd.frame;
            rt_loop_end_ = cmd.frame2;
        }
        break;
    case TransportCommand::Type::SetMetronome:
        rt_metro_enabled_ = cmd.enabled;
        if (cmd.enabled) {
            rt_metro_gain_ = cmd.gain;
            rt_metro_fpb_ = cmd.frames_per_beat;
            rt_metro_bpb_ = cmd.beats_per_bar;
        } else {
            rt_voice_count_ = 0;
        }
        break;
    }
}

void Engine::drain_commands() noexcept {
    TransportCommand cmd;
    while (commands_.pop(cmd)) apply_command(cmd);
}

void Engine::publish_transport() noexcept {
    pub_position_.store(rt_position_, std::memory_order_release);
    pub_playing_.store(rt_playing_, std::memory_order_release);
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

int32_t Engine::meters_peek(se_meter* out, std::size_t capacity, std::size_t* out_count) const {
    auto window_max = [](const std::atomic<float>* ring) {
        float m = 0.0f;
        for (uint32_t i = 0; i < kPeekBuckets; ++i) m = std::max(m, ring[i].load(std::memory_order_relaxed));
        return m;
    };
    std::size_t n = 0;
    if (capacity > 0) {
        out[0].track_id = 0;
        out[0].peak_l = window_max(master_win_l_);
        out[0].peak_r = window_max(master_win_r_);
        n = 1;
    }
    std::lock_guard lock(control_mutex_); // keeps current_ alive while reading
    if (current_) {
        for (std::size_t i = 0; i < current_->meter_count && n < capacity; ++i, ++n) {
            const TrackMeter& m = current_->meters[i];
            out[n].track_id = m.track_id;
            out[n].peak_l = window_max(m.win_l);
            out[n].peak_r = window_max(m.win_r);
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
void Engine::process(uint32_t frames, const RtInput& input, int64_t rec_latency, Sink&& sink) noexcept {
    const uint64_t t0 = now_ns();
    ScopedFlushDenormals ftz;

    // 1. Transport commands, applied at block start.
    drain_commands();
    if (!rt_playing_ || !rt_metro_enabled_) rt_voice_count_ = 0;

    // Recording session for this block (see record_stop() for the handshake).
    rec_in_use_.store(true, std::memory_order_seq_cst);
    Recorder* const rec = rec_live_.load(std::memory_order_seq_cst);
    const bool muted = input_muted_.load(std::memory_order_relaxed);
    const uint32_t in_count =
        std::min(input.channels, input.planar != nullptr ? kMaxDeviceInputChannels : kMaxInjectedInputChannels);
    const float* in_ptrs[kMaxDeviceInputChannels];

    // 2. Acquire the published graph (see reclaim_locked()).
    rt_active_seq_.store(kBusy, std::memory_order_seq_cst);
    const RenderGraph* graph = live_graph_.load(std::memory_order_seq_cst);
    if (graph != nullptr) rt_active_seq_.store(graph->seq, std::memory_order_seq_cst);

    // 3. Render in segments of at most max_block frames, additionally split
    //    at the loop end so the wrap is sample-accurate. render_block() is
    //    stateless (clips are addressed by timeline frame), so the segment
    //    after a wrap renders the loop start with correct clip offsets.
    float peak_l = 0.0f, peak_r = 0.0f;
    for (uint32_t done = 0; done < frames;) {
        uint32_t n = std::min(frames - done, max_block_);
        const bool looping = rt_playing_ && rt_loop_enabled_ && rt_position_ < rt_loop_end_;
        if (looping) n = static_cast<uint32_t>(std::min<int64_t>(n, rt_loop_end_ - rt_position_));
        const uint32_t slot = advance_peek_window(n, graph);
        // Input channels for this segment: device buffers (offset), injected
        // input deinterleaved into scratch, or zeros when input is muted.
        for (uint32_t c = 0; c < in_count; ++c) {
            if (muted) {
                in_ptrs[c] = live_ctx_.zeros();
            } else if (input.planar != nullptr) {
                in_ptrs[c] = input.planar[c] + done;
            } else {
                float* d = in_scratch_.data() + static_cast<std::size_t>(c) * max_block_;
                const float* s = input.interleaved + static_cast<std::size_t>(done) * input.channels + c;
                for (uint32_t i = 0; i < n; ++i) d[i] = s[static_cast<std::size_t>(i) * input.channels];
                in_ptrs[c] = d;
            }
        }
        const InputView iv{in_ptrs, in_count};
        const InstrumentInputs inst{graph != nullptr ? graph->synths.data() : nullptr, rt_synth_gen_};
        const BlockPeaks p = render_block(graph, rt_position_, rt_playing_, n, live_ctx_, true, slot, inst, iv);
        if (rec != nullptr && rt_playing_) rec->capture(in_ptrs, in_count, live_ctx_.zeros(), rt_position_, rec_latency, n);
        peak_l = std::max(peak_l, p.l);
        peak_r = std::max(peak_r, p.r);
        atomic_store_max(master_win_l_[slot], p.l);
        atomic_store_max(master_win_r_[slot], p.r);
        // Metronome: after master gain and after metering, never in export.
        if (rt_playing_ && rt_metro_enabled_)
            mix_metronome(live_ctx_.master_l(), live_ctx_.master_r(), rt_position_, n);
        sink(live_ctx_.master_l(), live_ctx_.master_r(), done, n);
        if (rt_playing_) {
            rt_position_ += n;
            if (looping && rt_position_ == rt_loop_end_) {
                rt_position_ = rt_loop_start_;
                if (rec != nullptr) rec->on_loop_wrap(rt_loop_start_); // takes continue linearly
                ++rt_synth_gen_; // sounding notes fade; notes at loop start retrigger
            }
        }
        done += n;
    }

    // 4. Release the graph, publish state.
    rt_active_seq_.store(kIdle, std::memory_order_seq_cst);
    rec_in_use_.store(false, std::memory_order_release);
    publish_transport();
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

// Moves the se_meters_peek window forward by `frames` rendered frames and
// returns the bucket the segment's peak belongs to (the one it ends in).
// Buckets entered are zeroed for the master and the live graph's tracks.
uint32_t Engine::advance_peek_window(uint32_t frames, const RenderGraph* graph) noexcept {
    const uint64_t total = rt_peek_fill_ + frames;
    uint64_t steps = total / peek_bucket_frames_;
    rt_peek_fill_ = total % peek_bucket_frames_;
    if (steps > kPeekBuckets) steps = kPeekBuckets;
    for (uint64_t s = 0; s < steps; ++s) {
        rt_peek_slot_ = (rt_peek_slot_ + 1) % kPeekBuckets;
        const uint32_t k = rt_peek_slot_;
        master_win_l_[k].store(0.0f, std::memory_order_relaxed);
        master_win_r_[k].store(0.0f, std::memory_order_relaxed);
        if (graph != nullptr)
            for (std::size_t t = 0; t < graph->meter_count; ++t) {
                graph->meters[t].win_l[k].store(0.0f, std::memory_order_relaxed);
                graph->meters[t].win_r[k].store(0.0f, std::memory_order_relaxed);
            }
    }
    return rt_peek_slot_;
}

// Mixes `buf` from its start into l/r at `offset`; keeps a voice for the
// tail if it does not fit in this segment (stealing the oldest if full).
void Engine::start_click(const float* buf, uint32_t offset, float* l, float* r, uint32_t frames) noexcept {
    const uint32_t m = std::min(click_len_, frames - offset);
    kernel_add_scaled(l + offset, buf, rt_metro_gain_, m);
    kernel_add_scaled(r + offset, buf, rt_metro_gain_, m);
    if (m >= click_len_) return;
    std::size_t slot = rt_voice_count_;
    if (slot == kMaxClickVoices) {
        slot = 0;
        for (std::size_t i = 1; i < rt_voice_count_; ++i)
            if (rt_voices_[i].pos > rt_voices_[slot].pos) slot = i;
    } else {
        ++rt_voice_count_;
    }
    rt_voices_[slot] = {buf, m};
}

// Adds metronome clicks for timeline frames [position, position + frames).
// Voices carry tails across segment boundaries in playback time (so a click
// rings out over a block edge or a loop wrap); onsets are taken from the
// timeline beat grid, so only beats the playhead actually crosses sound.
void Engine::mix_metronome(float* l, float* r, int64_t position, uint32_t frames) noexcept {
    const float g = rt_metro_gain_;
    for (std::size_t i = 0; i < rt_voice_count_;) {
        ClickVoice& v = rt_voices_[i];
        const uint32_t m = std::min(click_len_ - v.pos, frames);
        kernel_add_scaled(l, v.buf + v.pos, g, m);
        kernel_add_scaled(r, v.buf + v.pos, g, m);
        v.pos += m;
        if (v.pos >= click_len_)
            rt_voices_[i] = rt_voices_[--rt_voice_count_];
        else
            ++i;
    }
    const double fpb = rt_metro_fpb_;
    const auto beat_frame = [fpb](int64_t k) { return std::llround(static_cast<double>(k) * fpb); };
    const int64_t end = position + frames;
    int64_t k = static_cast<int64_t>(std::floor(static_cast<double>(position) / fpb)) - 1;
    while (beat_frame(k) < position) ++k;
    for (int64_t f = beat_frame(k); f < end; f = beat_frame(++k)) {
        const bool downbeat = floor_mod(k, rt_metro_bpb_) == 0;
        start_click(downbeat ? click_down_.data() : click_beat_.data(), static_cast<uint32_t>(f - position), l, r,
                    frames);
    }
}

void Engine::render(float* const* channels, uint32_t num_channels, const float* const* in, uint32_t num_in,
                    uint32_t frames) noexcept {
    const RtInput input{in, nullptr, in != nullptr ? num_in : 0};
    process(frames, input, device_rec_latency_.load(std::memory_order_relaxed),
            [&](const float* l, const float* r, uint32_t offset, uint32_t n) {
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
    return render_offline_impl(RtInput{}, out_interleaved, frames, err);
}

int32_t Engine::render_offline_with_input(const float* input, uint32_t input_channels, float* out_interleaved,
                                          uint32_t frames, std::string& err) {
    if (input_channels == 0 || input_channels > kMaxInjectedInputChannels) {
        err = "input_channels must be 1.." + std::to_string(kMaxInjectedInputChannels);
        return SE_ERR_INVALID_ARG;
    }
    return render_offline_impl(RtInput{nullptr, input, input_channels}, out_interleaved, frames, err);
}

int32_t Engine::render_offline_impl(const RtInput& input, float* out_interleaved, uint32_t frames, std::string& err) {
    if (frames == 0) return SE_OK;
    const int conflict = acquire_rt_owner(kOwnerOffline);
    if (conflict != kOwnerNone) {
        err = conflict == kOwnerDevice ? "cannot render offline while the device is running"
                                       : "another offline render is in progress";
        return SE_ERR_STATE;
    }
    // Injected input is not delayed by a device, so takes get 0 compensation.
    process(frames, input, 0, [out_interleaved](const float* l, const float* r, uint32_t offset, uint32_t n) {
        float* dst = out_interleaved + static_cast<std::size_t>(offset) * 2;
        for (uint32_t i = 0; i < n; ++i) {
            dst[2 * i] = l[i];
            dst[2 * i + 1] = r[i];
        }
    });
    release_rt_owner(); // also applies commands queued during the render
    return SE_OK;
}

// ============================================================ recording

int32_t Engine::record_start(const char* dir, std::string& err) {
    std::lock_guard lock(rec_mutex_);
    if (recorder_) {
        err = "already recording";
        return SE_ERR_STATE;
    }
    std::vector<RecordTrackSpec> specs;
    {
        std::lock_guard g(control_mutex_);
        if (current_)
            for (const TrackNode& t : current_->tracks)
                if (t.arm && t.input_mode != SE_INPUT_NONE)
                    specs.push_back({t.track_id, t.input_channel, t.input_mode == SE_INPUT_STEREO ? 2u : 1u});
    }
    if (specs.empty()) {
        err = "no armed track has an input";
        return SE_ERR_STATE;
    }
    if (device_running() && !device_input_open_.load(std::memory_order_acquire)) {
        err = "the audio device has no input open (configure an input device first)";
        return SE_ERR_STATE;
    }
    if (sample_rate_ != std::floor(sample_rate_)) {
        err = "engine sample rate is not representable in a WAV header";
        return SE_ERR_UNSUPPORTED_FORMAT;
    }
    const std::size_t ring = record_ring_frames_ != 0 ? record_ring_frames_
                                                      : static_cast<std::size_t>(2.0 * sample_rate_);
    auto rec = Recorder::create(dir, std::move(specs), sample_rate_, ring, err);
    if (!rec) return SE_ERR_IO;
    last_takes_.clear();
    last_stats_ = {};
    recorder_ = std::move(rec);
    rec_live_.store(recorder_.get(), std::memory_order_seq_cst); // the next block captures
    TransportCommand play;
    play.type = TransportCommand::Type::Play; // no-op if already playing
    push_command(play);
    return SE_OK;
}

// Stop handshake with process(): we clear rec_live_ and then wait until the
// RT side is not inside a block that loaded the old pointer. Both sides use
// seq_cst (RT: in_use = true; load ptr / us: store null; load in_use), so
// either the RT block sees null or we see in_use == true and wait for it.
// Only this non-RT thread waits; the audio thread never does.
int32_t Engine::record_stop(std::size_t* out_count, std::string& err) {
    std::lock_guard lock(rec_mutex_);
    if (!recorder_) {
        err = "not recording";
        return SE_ERR_STATE;
    }
    rec_live_.store(nullptr, std::memory_order_seq_cst);
    while (rec_in_use_.load(std::memory_order_seq_cst)) std::this_thread::sleep_for(std::chrono::microseconds(200));
    recorder_->finish(last_takes_);
    last_stats_ = recorder_->stats();
    recorder_.reset();
    if (out_count != nullptr) *out_count = last_takes_.size();
    return SE_OK;
}

int32_t Engine::record_get_take(std::size_t index, se_recorded_take* out) const {
    std::lock_guard lock(rec_mutex_);
    if (index >= last_takes_.size()) return SE_ERR_NOT_FOUND;
    const RecordedTakeInfo& t = last_takes_[index];
    *out = se_recorded_take{};
    out->track_id = t.track_id;
    out->start_frame = t.start_frame;
    out->frames = t.frames;
    out->channels = t.channels;
    out->sample_rate = t.sample_rate;
    copy_str(out->path, sizeof out->path, t.path);
    return SE_OK;
}

void Engine::record_state(bool& active, std::vector<LiveTakeInfo>& takes) const {
    std::lock_guard lock(rec_mutex_);
    active = recorder_ != nullptr;
    takes.clear();
    if (recorder_) recorder_->live_takes(takes);
}

std::size_t Engine::record_peaks(uint64_t track_id, uint32_t frames_per_peak, float* out, std::size_t capacity) const {
    std::lock_guard lock(rec_mutex_);
    return recorder_ ? recorder_->peaks(track_id, frames_per_peak, out, capacity) : 0;
}

RecordStats Engine::record_stats() const {
    std::lock_guard lock(rec_mutex_);
    return recorder_ ? recorder_->stats() : last_stats_;
}

void Engine::set_record_writer_stalled(bool stalled) {
    std::lock_guard lock(rec_mutex_);
    if (recorder_) recorder_->set_writer_stalled(stalled);
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
    // Private synth state: export never touches the live voices.
    std::vector<std::unique_ptr<Synth>> synths;
    std::vector<Synth*> synth_ptrs;
    if (graph) {
        synth_ptrs.assign(graph->tracks.size(), nullptr);
        for (std::size_t i = 0; i < graph->tracks.size(); ++i)
            if (graph->tracks[i].instrument) {
                synths.push_back(std::make_unique<Synth>(sample_rate_));
                synth_ptrs[i] = synths.back().get();
            }
    }
    const InstrumentInputs inst{synth_ptrs.empty() ? nullptr : synth_ptrs.data(), 0};
    ScopedFlushDenormals ftz;
    for (int64_t pos = start_frame; pos < end_frame;) {
        const auto n = static_cast<uint32_t>(std::min<int64_t>(end_frame - pos, max_block_));
        render_block(graph.get(), pos, true, n, ctx, false, 0, inst);
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
