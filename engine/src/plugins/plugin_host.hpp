// Registry of live plugin instances, keyed by device id (non-RT).
//
// Lifetime: Engine::publish(v5) holds mutex() while it resolves every device
// of the new session through use() (creating instances for ids seen for the
// first time), installs the graph, and calls end_publish(seq): instances the
// new graph does not use are retired (their editor closes now) and destroyed
// by collect() on a later main-thread call, once the audio thread can no
// longer reach a graph that references them (see Engine::reclaim_locked()
// for the hazard protocol; `rt_active` is Engine::rt_active_seq_).
#pragma once

#include "../asset.hpp"
#include "plugin_instance.hpp"
#include "saine_engine.h"

#include <atomic>
#include <cstdint>
#include <memory>
#include <mutex>
#include <string>
#include <unordered_map>
#include <utility>
#include <vector>

namespace saine {

class PluginHost {
public:
    static constexpr uint64_t kIdle = 0;
    static constexpr uint64_t kBusy = ~uint64_t{0};

    PluginHost(double sample_rate, uint32_t max_block);
    ~PluginHost();
    PluginHost(const PluginHost&) = delete;
    PluginHost& operator=(const PluginHost&) = delete;

    std::mutex& mutex() noexcept { return mutex_; }
    // Asset registry access for built-in devices with sample slots (set
    // once by the engine before the first publish).
    void set_asset_lookup(AssetLookup lookup) { assets_ = std::move(lookup); }

    // ---- publish (caller holds mutex())
    void begin_publish();
    // Instance for one device of the session being published, or null
    // (load failed: pass-through / silent instrument).
    PluginInstance* use(const se_device_desc& d, bool instrument, const char* track_name);
    // The graph with `seq` is live: retire everything it does not use.
    void end_publish(uint64_t seq);

    // ---- main-thread housekeeping (caller holds mutex())
    void collect(uint64_t rt_active);
    void idle(uint64_t rt_active);
    // Delay compensation: restarts instances that asked for it (suspended
    // meanwhile) and re-reads every live instance's latency into its cache.
    // True if any cached latency changed.
    bool service_latencies();
    // Engine destruction (RT path stopped): destroy everything now.
    void shutdown_all();

    // ---- queries (caller holds mutex())
    bool is_live(uint64_t device_id) const;
    void load_errors(std::vector<std::pair<uint64_t, std::string>>& out) const;
    std::size_t save_states();
    bool saved_state(std::size_t index, uint64_t& id, const uint8_t*& data, std::size_t& len) const;
    int32_t editor_open(uint64_t device_id, std::string& err);
    void editor_close(uint64_t device_id);
    bool editor_is_open(uint64_t device_id);
    bool latency(uint64_t device_id, uint32_t& frames);
    PluginInstance* instance(uint64_t device_id) const;

    // ---- editor parameter changes (lock-free, see ParamBus)
    ParamBus& bus() noexcept { return bus_; }
    // CLAP params.request_flush pending on some live instance.
    bool flush_pending() const;
    // Runs those flushes (caller holds mutex() and the engine's RT path).
    void flush_params();

    // Test hook: instances created so far.
    uint64_t instantiations() const noexcept { return instantiations_.load(); }

private:
    struct Entry {
        PluginSpec spec; // without state
        std::shared_ptr<PluginInstance> inst;
        std::string error;
        bool used = false;
    };
    struct Retired {
        std::shared_ptr<PluginInstance> inst;
        uint64_t seq; // first graph seq that does not reference it
    };

    void retire(uint64_t device_id, Entry& e, uint64_t seq);
    static void destroy(std::shared_ptr<PluginInstance>& inst) noexcept;

    const double sample_rate_;
    const uint32_t max_block_;
    AssetLookup assets_;
    ParamBus bus_; // outlives every instance (declared before them)
    mutable std::mutex mutex_;
    std::unordered_map<uint64_t, Entry> entries_;
    std::vector<Retired> retired_;
    std::vector<std::shared_ptr<PluginInstance>> pending_; // replaced during this publish, seq not known yet
    std::unordered_map<uint64_t, std::unique_ptr<PluginEditor>> editors_;
    std::vector<std::pair<uint64_t, std::vector<uint8_t>>> saved_;
    std::atomic<uint64_t> instantiations_{0};
};

} // namespace saine
