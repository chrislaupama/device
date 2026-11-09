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

// Per-call context of BuiltinProcessor::process().
struct BuiltinContext {
    // Sidechain input (post-fader output of the source track), or null.
    const float* sc_l = nullptr;
    const float* sc_r = nullptr;
    const RtTransport* transport = nullptr;
    // Frames from transport->position to the first frame of this call (the
    // instance splits blocks at automation events).
    uint32_t offset = 0;
    // MIDI events of this call (instruments), offsets relative to l/r[0],
    // sorted by offset.
    const MidiEvent* events = nullptr;
    uint32_t event_count = 0;
};

class BuiltinProcessor {
public:
    virtual ~BuiltinProcessor() = default;
    // Non-RT. Called once before any processing; may allocate.
    virtual void prepare(double sample_rate, uint32_t max_block) = 0;
    // RT. Clears internal state (envelopes, delay lines) without allocating.
    virtual void reset() noexcept = 0;
    // RT. Processes l/r in place (an instrument gets silence in l/r and
    // writes its output there; its notes arrive in ctx.events). `params` holds the current plain values
    // (registry order), constant over this call: the instance splits blocks
    // at automation events. Smoothing of audible steps is the processor's job.
    // Write readouts (if any) into `readouts`.
    virtual void process(float* l, float* r, uint32_t frames, const float* params, const BuiltinContext& ctx,
                         float* readouts) noexcept = 0;
    // Latency in frames (reported for delay compensation).
    virtual uint32_t latency() const noexcept { return 0; }

    // ---- modulation sources (LFO, chain head; see se_mod_route)
    // Number of outputs (0 = not a source).
    virtual uint32_t mod_outputs() const noexcept { return 0; }
    // RT. Writes (value 0..1, gain) per output into out[2 * o], out[2 * o + 1]
    // for the moment ctx.transport->position, then advances the source's
    // clock by `frames`. Called once per SE_MOD_GRID_FRAMES of every block
    // the source is enabled, before the track's audio is processed.
    virtual void modulate(const float* params, uint32_t frames, const BuiltinContext& ctx, float* out) noexcept {
        (void)params;
        (void)frames;
        (void)ctx;
        (void)out;
    }
};

// One modulated parameter of a built-in target for the next process() call:
// per grid point (SE_MOD_GRID_FRAMES apart from the block start) the
// additive offset and the absolute normalized value (NaN = none).
struct BuiltinMod {
    uint32_t index = 0;
    bool log = false;
    float min = 0.0f, max = 1.0f, skew = 1.0f, step = 0.0f;
    const float* off = nullptr;
    const float* abs = nullptr;
};

// Normalisation of saine_core::builtin::ParamDef (to_normalized /
// from_normalized + snap), used to apply modulation in the 0..1 domain.
float mod_to_norm(const BuiltinMod& m, float v) noexcept;
float mod_from_norm(const BuiltinMod& m, float n) noexcept;

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
    // A modulation source (LFO, chain head): it runs every block it is
    // enabled, with or without routes, so its clock and readouts stay live.
    bool mod_source = false;
    // An instrument: it takes the track's instrument slot and plays the
    // track's notes (BuiltinContext::events) instead of processing audio.
    bool instrument = false;
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
// The same for v2 blobs: also the modulation routes (se_mod_route records,
// unaligned) and their count (0 for v1 blobs).
const char* parse_builtin_state(const uint8_t* data, std::size_t len, const uint8_t*& values, uint32_t& count,
                                uint64_t& sidechain, const uint8_t*& routes, uint32_t& route_count) noexcept;

// Adapter from a BuiltinProcessor to the plugin host.
class BuiltinInstance final : public PluginInstance {
public:
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
    // RT: sidechain buffers for the next run() (cleared after it).
    void set_sidechain(const float* l, const float* r) noexcept {
        sc_l_ = l;
        sc_r_ = r;
    }
    // RT: modulated parameters for the next run() (cleared after it).
    void set_modulation(const BuiltinMod* mods, uint32_t count) noexcept {
        mods_ = mods;
        mod_count_ = count;
    }

    // ---- modulation sources
    static constexpr uint32_t kGrid = SE_MOD_GRID_FRAMES;
    uint32_t mod_outputs() const noexcept { return mod_outputs_; }
    // RT: evaluates the source for a block of `frames` (automation events
    // applied at their grid point); results stay readable until the next call.
    void run_modulation(uint32_t frames, const RtTransport& transport, const ParamEvent* params,
                        uint32_t param_count) noexcept;
    // (value, gain) of output `o` at grid point `k` of the last evaluated block.
    const float* mod_point(uint32_t k, uint32_t o) const noexcept { return mod_out_.data() + 2 * (std::size_t{k} * mod_outputs_ + o); }

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
    std::vector<MidiEvent> midi_scratch_; // MIDI events of one automation segment
    const float* sc_l_ = nullptr;
    const float* sc_r_ = nullptr;
    const BuiltinMod* mods_ = nullptr;
    uint32_t mod_count_ = 0;
    std::vector<float> modded_;  // params with modulation applied (RT scratch)
    uint32_t mod_outputs_ = 0;
    std::vector<float> mod_out_; // [point][output][value, gain]
    std::vector<float> mod_params_;
};

std::shared_ptr<PluginInstance> create_builtin_instance(const PluginSpec& spec, double sample_rate, uint32_t max_block,
                                                        std::string& err);

} // namespace saine
