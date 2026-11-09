// Built-in devices: processors that ship with the engine (docs/devices.md).
//
// A built-in device has three parts on the engine side:
//  - a BuiltinProcessor subclass: the DSP kernel. prepare() may allocate;
//    reset() and process() run on the RT path and must not allocate, lock,
//    block or do I/O.
//  - a BuiltinDescriptor: registry id (the same id as the Rust definition in
//    saine_core::builtin), parameter ids in definition order, readout ids, and
//    a factory. Registered in kBuiltinRegistry (builtin.cpp).
//  - nothing else: BuiltinInstance adapts every processor to the plugin host
//    (PluginInstance), so chains, bypass, delay compensation, automation
//    lanes and instance lifetime work exactly as for third-party plugins.
//
// Parameter values are plain values in the parameter's unit, already
// validated and snapped by the application core. Processors still clamp to
// physically safe ranges (the C ABI can be called directly).
#pragma once

#include "../plugins/plugin_instance.hpp"
#include "saine_engine.h"

#include <atomic>
#include <cstdint>
#include <memory>
#include <span>
#include <string_view>
#include <vector>

namespace saine {

// Bulk analysis output of a built-in (spectra, traces): a lock-free triple
// buffer of float frames. The audio thread fills begin() and publish()es a
// frame; the main thread copies the newest complete frame with read().
// One writer (the processor) and one reader (the main thread).
class AnalysisChannel {
public:
    explicit AnalysisChannel(uint32_t capacity);
    uint32_t capacity() const noexcept { return capacity_; }
    // RT: the frame being written (capacity() floats).
    float* begin() noexcept { return buf_.get() + std::size_t{back_} * capacity_; }
    // RT: completes the frame begun with begin(); count floats are valid.
    void publish(uint32_t count) noexcept;
    // Main thread: copies the newest frame (up to cap floats) and returns
    // its float count; seq = its publish number (0 = nothing published yet).
    uint32_t read(float* out, uint32_t cap, uint64_t& seq) noexcept;

private:
    static constexpr uint32_t kFresh = 4;
    uint32_t capacity_;
    std::unique_ptr<float[]> buf_;
    uint32_t count_[3] = {0, 0, 0};
    uint64_t seq_[3] = {0, 0, 0};
    uint64_t next_seq_ = 0;
    uint32_t back_ = 0, front_ = 1;
    std::atomic<uint32_t> middle_{2};
};

// Per-call context of BuiltinProcessor::process().
struct BuiltinContext {
    // Sidechain input (post-fader output of the source track), or null.
    const float* sc_l = nullptr;
    const float* sc_r = nullptr;
    const RtTransport* transport = nullptr;
    // Frames from transport->position to the first frame of this call (the
    // instance splits blocks at automation events).
    uint32_t offset = 0;
    // Analysis output, when the processor asked for one (analysis_capacity()).
    AnalysisChannel* analysis = nullptr;
};

class BuiltinProcessor {
public:
    virtual ~BuiltinProcessor() = default;
    // Non-RT. Called once before any processing; may allocate.
    virtual void prepare(double sample_rate, uint32_t max_block) = 0;
    // RT. Clears internal state (envelopes, delay lines) without allocating.
    virtual void reset() noexcept = 0;
    // RT. Processes l/r in place. `params` holds the current plain values
    // (registry order), constant over this call: the instance splits blocks
    // at automation events. Smoothing of audible steps is the processor's job.
    // Write readouts (if any) into `readouts`.
    virtual void process(float* l, float* r, uint32_t frames, const float* params, const BuiltinContext& ctx,
                         float* readouts) noexcept = 0;
    // Latency in frames (reported for delay compensation).
    virtual uint32_t latency() const noexcept { return 0; }
    // Floats per analysis frame (spectra, traces) the processor publishes
    // through BuiltinContext::analysis; 0 = none. Read once after prepare().
    virtual uint32_t analysis_capacity() const noexcept { return 0; }
};

struct BuiltinDescriptor {
    const char* id;
    std::span<const char* const> params;
    std::span<const char* const> readouts;
    std::unique_ptr<BuiltinProcessor> (*create)();
    // Readout values before the first processed block (empty = zeros), e.g.
    // silence for level meters. When automation splits a block, each readout
    // keeps the segment value farthest from its idle value.
    std::span<const float> idle_readouts;
    // Default plain values in parameter order (the Rust definition's
    // defaults), for the generic tests and the benchmark; empty = unknown.
    std::span<const float> defaults;
    // Readouts are one consistent state (a detected note, its cents and
    // string), not meters: when automation splits a block the last
    // segment's values are kept together.
    bool state_readouts = false;
};

std::span<const BuiltinDescriptor> builtin_registry() noexcept;
const BuiltinDescriptor* find_builtin(std::string_view id) noexcept;
// The ABI view of the registry (static storage).
const se_builtin_desc* builtin_registry_c(std::size_t& count) noexcept;

// Parses an se_builtin_state blob. Returns an error text or null. On
// success `values` / `count` point into `data` (may be unaligned: read with
// memcpy) and `sidechain` is the source track id (0 = none).
const char* parse_builtin_state(const uint8_t* data, std::size_t len, const uint8_t*& values, uint32_t& count,
                                uint64_t& sidechain) noexcept;

// Adapter from a BuiltinProcessor to the plugin host.
class BuiltinInstance final : public PluginInstance {
public:
    static constexpr uint32_t kMaxParams = 256;
    static constexpr uint32_t kMaxReadouts = 32;

    BuiltinInstance(PluginSpec spec, const BuiltinDescriptor& desc, std::unique_ptr<BuiltinProcessor> proc,
                    double sample_rate, uint32_t max_block);

    BuiltinInstance* as_builtin() noexcept override { return this; }
    const BuiltinDescriptor& descriptor() const noexcept { return desc_; }
    uint32_t param_count() const noexcept { return static_cast<uint32_t>(desc_.params.size()); }

    // Main thread: new values (lock-free stores, picked up at the next block).
    void set_value(uint32_t index, float value) noexcept;
    void update_state(const uint8_t* data, std::size_t len) noexcept override;
    float value(uint32_t index) const noexcept;
    // Main thread: latest readouts.
    uint32_t readouts(float* out, uint32_t capacity) const noexcept;
    // Main thread: newest analysis frame (see AnalysisChannel::read); 0 when
    // the processor publishes none.
    uint32_t analysis(float* out, uint32_t capacity, uint64_t& seq) noexcept;
    // RT: sidechain buffers for the next run() (cleared after it).
    void set_sidechain(const float* l, const float* r) noexcept {
        sc_l_ = l;
        sc_r_ = r;
    }

    void process(float* l, float* r, uint32_t frames, const MidiEvent* events, uint32_t event_count,
                 const RtTransport& transport, const ParamEvent* params, uint32_t param_count) noexcept override;
    bool save_state(std::vector<uint8_t>& out, std::string& err) override;
    uint32_t latency() override { return proc_->latency(); }
    std::unique_ptr<PluginEditor> open_editor(const std::string& title, std::string& err) override;
    void params(std::vector<ParamInfo>& out) override;
    bool param_range(uint32_t id, double& min, double& max) override;

    // Test hook: the wrapped processor.
    BuiltinProcessor& processor() noexcept { return *proc_; }

private:
    const BuiltinDescriptor& desc_;
    std::unique_ptr<BuiltinProcessor> proc_;
    std::unique_ptr<std::atomic<float>[]> target_; // written by the main thread
    std::vector<float> current_;                   // RT copy for this block
    std::unique_ptr<std::atomic<float>[]> readout_atomic_;
    std::vector<float> readout_scratch_;
    std::vector<float> readout_segment_; // later automation segments, merged into readout_scratch_
    std::unique_ptr<AnalysisChannel> analysis_;
    const float* sc_l_ = nullptr;
    const float* sc_r_ = nullptr;
};

std::shared_ptr<PluginInstance> create_builtin_instance(const PluginSpec& spec, double sample_rate, uint32_t max_block,
                                                        std::string& err);

} // namespace saine
