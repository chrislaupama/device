// Plugin instances (Audio Unit, VST3, CLAP) behind one small interface.
//
// Threading (see engine/README.md "Plugins"):
//  - Creation, state save, latency queries, editors and destruction run on
//    the host's main thread (the thread that publishes sessions).
//  - process() runs on whoever owns the RT path (device callback or an
//    offline render / export holding the RT ownership). It never allocates,
//    locks or does I/O on our side; the plugin's own code is out of our
//    control.
//  - set_parameter() is a test/host hook: main thread, delivered to the
//    processor through a lock-free queue at the next process() call.
#pragma once

#include "../spsc_queue.hpp"
#include "../synth.hpp" // NoteEvent

#include <atomic>
#include <cstdint>
#include <cstring>
#include <memory>
#include <mutex>
#include <string>
#include <vector>

namespace saine {

enum class PluginFormat : uint8_t { AudioUnit = 0, Vst3 = 1, Clap = 2 };

const char* plugin_format_name(PluginFormat f) noexcept;

// Everything needed to (re)create an instance. `state` is only consulted at
// creation.
struct PluginSpec {
    uint64_t device_id = 0;
    PluginFormat format = PluginFormat::AudioUnit;
    std::string plugin_id;
    std::string path;
    std::string name;  // display name
    std::string track; // track display name ("" if unknown)
    bool instrument = false; // used as a track's instrument plugin
    std::vector<uint8_t> state;

    // Same plugin in the same role (an instance can be reused).
    bool same_plugin(const PluginSpec& o) const noexcept {
        return format == o.format && plugin_id == o.plugin_id && path == o.path && instrument == o.instrument;
    }
};

// A short MIDI channel message at a sample offset inside the block.
struct MidiEvent {
    uint32_t offset = 0;
    uint8_t status = 0;
    uint8_t data1 = 0;
    uint8_t data2 = 0;
    uint8_t _pad = 0;
};

// Transport as seen by plugins for one processed segment.
struct RtTransport {
    int64_t position = 0;     // timeline frame of the segment's first sample
    bool playing = false;
    double sample_rate = 48000.0;
    double tempo_bpm = 120.0; // from the metronome grid (se_metronome_set), else 120
    // Quarter notes since frame 0 at `position` (constant tempo from 0).
    double beats() const noexcept { return static_cast<double>(position) / sample_rate * tempo_bpm / 60.0; }
};

// Converts a track's note list (timeline frames) into sample-accurate MIDI
// note on/off events, one segment at a time. Mirrors Synth's event model:
// note-on at `start`, note-off at `end`; a change of `transport_gen` or a
// timeline discontinuity sends note-offs for every sounding note (+ CC 123);
// a republish (new graph seq) re-matches sounding notes by (start, pitch),
// releasing the ones that disappeared. RT-safe; mutated only by the RT owner.
class NoteFeeder {
public:
    static constexpr uint32_t kMaxSounding = 128;
    // Writes events sorted by offset (note-offs first at equal offsets);
    // returns the count (events beyond `cap` are dropped).
    uint32_t build(const NoteEvent* notes, uint32_t note_count, uint64_t graph_seq, uint64_t transport_gen,
                   int64_t position, uint32_t frames, bool playing, MidiEvent* out, uint32_t cap) noexcept;
    uint32_t sounding() const noexcept { return count_; }

private:
    struct Sounding {
        int64_t start = 0;
        int64_t end = 0;
        uint8_t pitch = 0;
    };
    Sounding sounding_[kMaxSounding];
    uint32_t count_ = 0;
    uint64_t seen_seq_ = ~uint64_t{0};
    uint64_t seen_gen_ = 0;
    int64_t expected_pos_ = 0;
    bool have_pos_ = false;
};

// An open editor window (platform layer). Main thread only.
class PluginEditor {
public:
    virtual ~PluginEditor() = default;
    virtual bool is_open() const noexcept = 0; // false once the user closed the window
    virtual void close() noexcept = 0;         // idempotent
    virtual void bring_to_front() noexcept = 0;
};

// Parameter change from the main thread to the processor.
struct ParamChange {
    uint32_t id = 0;   // format parameter id (AU parameter id, VST3 ParamID, CLAP clap_id)
    double value = 0;  // VST3: normalized 0..1; AU/CLAP: plain value
};

class PluginInstance {
public:
    static constexpr uint32_t kMaxEvents = 512; // per processed segment

    virtual ~PluginInstance() = default;
    PluginInstance(const PluginInstance&) = delete;
    PluginInstance& operator=(const PluginInstance&) = delete;

    const PluginSpec& spec() const noexcept { return spec_; }

    // RT entry point used by render_block: process() unless the instance is
    // suspended for a restart (then effects pass audio through and
    // instruments are silent). `in_process_` / `suspended_` form a Dekker
    // pair with suspend(): either this call sees the suspension or
    // suspend() waits until the call has returned.
    void run(float* l, float* r, uint32_t frames, const MidiEvent* events, uint32_t event_count,
             const RtTransport& transport) noexcept {
        in_process_.store(true, std::memory_order_seq_cst);
        if (!suspended_.load(std::memory_order_seq_cst)) {
            process(l, r, frames, events, event_count, transport);
        } else if (spec_.instrument) {
            std::memset(l, 0, sizeof(float) * frames);
            std::memset(r, 0, sizeof(float) * frames);
        }
        in_process_.store(false, std::memory_order_release);
    }

    // RT. Effects process l/r in place. Instruments overwrite l/r with
    // their output; `events` are this segment's note events.
    virtual void process(float* l, float* r, uint32_t frames, const MidiEvent* events, uint32_t event_count,
                         const RtTransport& transport) noexcept = 0;

    // ---- restarts and latency (main thread)
    // Set from any thread by the format host when the plugin asks to be
    // restarted (CLAP request_restart, VST3 restartComponent(kLatencyChanged)).
    void request_restart() noexcept { restart_requested_.store(true, std::memory_order_release); }
    bool take_restart_request() noexcept { return restart_requested_.exchange(false, std::memory_order_acq_rel); }
    // Keeps the RT side out of process() until resume(); waits (main thread
    // only) for a process() call in flight to return.
    void suspend() noexcept;
    void resume() noexcept { suspended_.store(false, std::memory_order_seq_cst); }
    // Deactivate + re-activate (format specific), called between suspend()
    // and resume(). Default: nothing to do.
    virtual void restart() noexcept {}
    // Latency as last read with refresh_latency() (what delay compensation
    // uses; guarded by the PluginHost mutex).
    uint32_t cached_latency() const noexcept { return cached_latency_; }
    // Re-reads latency(); true if it changed.
    bool refresh_latency() {
        const uint32_t l = latency();
        const bool changed = l != cached_latency_;
        cached_latency_ = l;
        return changed;
    }

    // Main thread.
    virtual bool save_state(std::vector<uint8_t>& out, std::string& err) = 0;
    virtual uint32_t latency() = 0;
    virtual std::unique_ptr<PluginEditor> open_editor(const std::string& title, std::string& err) = 0;
    // Called on the main thread right before destruction (the RT side can
    // no longer reach the instance).
    virtual void shutdown() noexcept {}
    // CLAP request_callback etc.
    virtual void on_main_thread() noexcept {}

    // Main thread: queue a parameter change for the processor (and update
    // the controller side where the format has one). False if the queue is full.
    bool set_parameter(uint32_t id, double value) noexcept;

    // RT: the instrument slot's note feeder.
    NoteFeeder& feeder() noexcept { return feeder_; }
    MidiEvent* event_scratch() noexcept { return events_; }

protected:
    explicit PluginInstance(PluginSpec spec) : spec_(std::move(spec)) {}
    // Main thread hook for set_parameter (controller side). Default: nothing.
    virtual void on_parameter_set(const ParamChange&) noexcept {}
    // RT: next queued parameter change.
    bool pop_param(ParamChange& c) noexcept { return params_.pop(c); }
    bool push_param(const ParamChange& c) noexcept;

    PluginSpec spec_;

private:
    std::atomic<bool> in_process_{false};
    std::atomic<bool> suspended_{false};
    std::atomic<bool> restart_requested_{false};
    uint32_t cached_latency_ = 0;
    std::mutex param_mutex_; // serialises producers (SPSC producer side)
    SpscQueue<ParamChange, 256> params_;
    NoteFeeder feeder_;
    MidiEvent events_[kMaxEvents];
};

// Creates and prepares an instance (main thread): stereo in/out at
// `sample_rate` / `max_block`, `spec.state` applied. Null + `err` on failure.
std::shared_ptr<PluginInstance> create_plugin_instance(const PluginSpec& spec, double sample_rate, uint32_t max_block,
                                                       std::string& err);

// Format factories (each returns null + err on failure or when the format is
// not available on this platform).
std::shared_ptr<PluginInstance> create_au_instance(const PluginSpec& spec, double sample_rate, uint32_t max_block,
                                                   std::string& err);
std::shared_ptr<PluginInstance> create_vst3_instance(const PluginSpec& spec, double sample_rate, uint32_t max_block,
                                                     std::string& err);
std::shared_ptr<PluginInstance> create_clap_instance(const PluginSpec& spec, double sample_rate, uint32_t max_block,
                                                     std::string& err);

// True on the platform's main thread (macOS: pthread_main_np). Other
// platforms: always true (not enforced).
bool on_platform_main_thread() noexcept;

} // namespace saine
