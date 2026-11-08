#include "engine.hpp"

#include "resampler.hpp"
#include "rt_util.hpp"
#include "wav.hpp"

#include <algorithm>
#include <cmath>
#include <cstring>
#include <limits>
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
    if (!device_) device_ = create_platform_audio_device();
    AudioDevice::Config cfg;
    cfg.sample_rate = sample_rate_;
    cfg.channels = channels_;
    cfg.preferred_block = max_block_;
    const int32_t rc = device_->start(cfg, this, err);
    if (rc != SE_OK) {
        release_rt_owner();
        return rc;
    }
    return SE_OK;
}

int32_t Engine::device_stop() {
    std::lock_guard lock(device_mutex_);
    if (rt_owner_.load() != kOwnerDevice) return SE_OK;
    if (device_) device_->stop(); // returns once the callback can no longer run
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
        break;
    case TransportCommand::Type::Stop:
        rt_playing_ = false;
        rt_position_ = 0;
        rt_voice_count_ = 0;
        break;
    case TransportCommand::Type::Seek: rt_position_ = cmd.frame; break; // click tails ring on
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
void Engine::process(uint32_t frames, Sink&& sink) noexcept {
    const uint64_t t0 = now_ns();
    ScopedFlushDenormals ftz;

    // 1. Transport commands, applied at block start.
    drain_commands();
    if (!rt_playing_ || !rt_metro_enabled_) rt_voice_count_ = 0;

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
        const BlockPeaks p = render_block(graph, rt_position_, rt_playing_, n, live_ctx_, true, slot);
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
            if (looping && rt_position_ == rt_loop_end_) rt_position_ = rt_loop_start_;
        }
        done += n;
    }

    // 4. Release the graph, publish state.
    rt_active_seq_.store(kIdle, std::memory_order_seq_cst);
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
    const int conflict = acquire_rt_owner(kOwnerOffline);
    if (conflict != kOwnerNone) {
        err = conflict == kOwnerDevice ? "cannot render offline while the device is running"
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
    release_rt_owner(); // also applies commands queued during the render
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
