// CLAP hosting: clap_entry -> factory -> create_plugin -> init -> activate.
//
// Threads: the host "main thread" is the thread that created the instance
// (the publishing thread); clap_host_thread_check reports it. The "audio
// thread" is whoever is inside process(), plus shutdown()'s single
// stop_processing call, made after the RT side can no longer reach the
// instance (CLAP allows the audio-thread role to move between threads as
// long as calls never overlap).
#include "plugin_instance.hpp"

#include "plugin_editor.hpp"
#include "plugin_modules.hpp"

#include <clap/clap.h>

#include <algorithm>
#include <atomic>
#include <cstring>
#include <thread>

#if defined(__APPLE__)
#include <dispatch/dispatch.h>
#endif

namespace saine {

namespace {

thread_local bool t_clap_audio_thread = false;

struct AudioThreadScope {
    AudioThreadScope() noexcept { t_clap_audio_thread = true; }
    ~AudioThreadScope() { t_clap_audio_thread = false; }
};

union ClapEventStorage {
    clap_event_header_t header;
    clap_event_note_t note;
    clap_event_midi_t midi;
    clap_event_param_value_t param;
};

class ClapInstance final : public PluginInstance, public std::enable_shared_from_this<ClapInstance> {
public:
    static constexpr uint32_t kMaxParamEvents = 64;

    ClapInstance(PluginSpec spec, double rate, uint32_t max_block)
        : PluginInstance(std::move(spec)), rate_(rate), max_block_(max_block), in_l_(max_block), in_r_(max_block),
          out_l_(max_block), out_r_(max_block), main_thread_(std::this_thread::get_id()) {
        host_.clap_version = CLAP_VERSION;
        host_.host_data = this;
        host_.name = "Saine Studio";
        host_.vendor = "Saine";
        host_.url = "";
        host_.version = "0.1.0";
        host_.get_extension = &ClapInstance::host_get_extension;
        host_.request_restart = [](const clap_host* h) { self_of(h)->request_restart(); };
        host_.request_process = [](const clap_host*) {};
        host_.request_callback = &ClapInstance::host_request_callback;
    }

    ~ClapInstance() override { shutdown(); }

    bool init(std::string& err) {
        mod_ = load_clap_module(spec_.path, err);
        if (!mod_) return false;
        const auto* factory =
            static_cast<const clap_plugin_factory_t*>(mod_->entry->get_factory(CLAP_PLUGIN_FACTORY_ID));
        if (factory == nullptr) {
            err = "no CLAP plugin factory in " + spec_.path;
            return false;
        }
        plugin_ = factory->create_plugin(factory, &host_, spec_.plugin_id.c_str());
        if (plugin_ == nullptr) {
            err = "CLAP plugin '" + spec_.plugin_id + "' not found in " + spec_.path;
            return false;
        }
        if (!plugin_->init(plugin_)) {
            err = "CLAP plugin init failed";
            plugin_->destroy(plugin_);
            plugin_ = nullptr;
            return false;
        }
        const auto ext = [&](const char* id) { return plugin_->get_extension(plugin_, id); };
        audio_ports_ = static_cast<const clap_plugin_audio_ports_t*>(ext(CLAP_EXT_AUDIO_PORTS));
        note_ports_ = static_cast<const clap_plugin_note_ports_t*>(ext(CLAP_EXT_NOTE_PORTS));
        state_ = static_cast<const clap_plugin_state_t*>(ext(CLAP_EXT_STATE));
        latency_ = static_cast<const clap_plugin_latency_t*>(ext(CLAP_EXT_LATENCY));
        gui_ = static_cast<const clap_plugin_gui_t*>(ext(CLAP_EXT_GUI));

        // Audio ports: main input (effects) and main output, stereo or mono.
        in_channels_ = port_channels(true);
        out_channels_ = port_channels(false);
        if (out_channels_ == 0 || out_channels_ > 2) {
            err = "unsupported CLAP output layout (need a mono or stereo main output)";
            return false;
        }
        if (!spec_.instrument && (in_channels_ == 0 || in_channels_ > 2)) {
            err = "unsupported CLAP input layout (an effect needs a mono or stereo main input)";
            return false;
        }
        if (in_channels_ > 2) in_channels_ = 0;
        if (note_ports_ != nullptr && note_ports_->count(plugin_, true) > 0) {
            clap_note_port_info_t info{};
            if (note_ports_->get(plugin_, 0, true, &info)) {
                use_midi_ = (info.supported_dialects & CLAP_NOTE_DIALECT_CLAP) == 0 &&
                            (info.supported_dialects & CLAP_NOTE_DIALECT_MIDI) != 0;
                note_port_ = true;
            }
        }

        if (!spec_.state.empty()) {
            if (state_ == nullptr) {
                err = "saved state given but the plugin has no clap.state extension";
                return false;
            }
            if (!load_state(spec_.state)) {
                err = "the plugin rejected the saved state";
                return false;
            }
        }
        spec_.state.clear();
        spec_.state.shrink_to_fit();

        if (!plugin_->activate(plugin_, rate_, 1, max_block_)) {
            err = "CLAP plugin activate failed";
            return false;
        }
        activated_ = true;

        in_ptrs_[0] = in_l_.data();
        in_ptrs_[1] = in_r_.data();
        out_ptrs_[0] = out_l_.data();
        out_ptrs_[1] = out_r_.data();
        in_buf_ = clap_audio_buffer_t{in_ptrs_, nullptr, in_channels_, 0, 0};
        out_buf_ = clap_audio_buffer_t{out_ptrs_, nullptr, out_channels_, 0, 0};
        in_events_.ctx = this;
        in_events_.size = [](const clap_input_events* l) -> uint32_t {
            return static_cast<const ClapInstance*>(l->ctx)->event_count_;
        };
        in_events_.get = [](const clap_input_events* l, uint32_t i) -> const clap_event_header_t* {
            auto* self = static_cast<const ClapInstance*>(l->ctx);
            return i < self->event_count_ ? &self->event_store_[i].header : nullptr;
        };
        out_events_.ctx = this;
        out_events_.try_push = [](const clap_output_events*, const clap_event_header_t*) { return true; };
        return true;
    }

    void process(float* l, float* r, uint32_t frames, const MidiEvent* events, uint32_t event_count,
                 const RtTransport& t) noexcept override {
        AudioThreadScope audio;
        if (!activated_ || failed_) return pass(l, r, frames);
        if (!processing_) {
            if (!plugin_->start_processing(plugin_)) {
                failed_ = true;
                return pass(l, r, frames);
            }
            processing_ = true;
        }
        // Events: parameter changes at 0, then notes (already sorted).
        event_count_ = 0;
        ParamChange pc;
        while (event_count_ < kMaxParamEvents && pop_param(pc)) {
            clap_event_param_value_t& e = event_store_[event_count_++].param;
            e = clap_event_param_value_t{};
            e.header = {sizeof e, 0, CLAP_CORE_EVENT_SPACE_ID, CLAP_EVENT_PARAM_VALUE, 0};
            e.param_id = pc.id;
            e.note_id = -1;
            e.port_index = -1;
            e.channel = -1;
            e.key = -1;
            e.value = pc.value;
        }
        for (uint32_t i = 0; i < event_count && note_port_; ++i) {
            const MidiEvent& m = events[i];
            const uint8_t kind = m.status & 0xF0;
            if (use_midi_) {
                clap_event_midi_t& e = event_store_[event_count_++].midi;
                e.header = {sizeof e, m.offset, CLAP_CORE_EVENT_SPACE_ID, CLAP_EVENT_MIDI, 0};
                e.port_index = 0;
                e.data[0] = m.status;
                e.data[1] = m.data1;
                e.data[2] = m.data2;
            } else if (kind == 0x90 || kind == 0x80) {
                clap_event_note_t& e = event_store_[event_count_++].note;
                e.header = {sizeof e, m.offset, CLAP_CORE_EVENT_SPACE_ID,
                            static_cast<uint16_t>(kind == 0x90 ? CLAP_EVENT_NOTE_ON : CLAP_EVENT_NOTE_OFF), 0};
                e.note_id = -1;
                e.port_index = 0;
                e.channel = 0;
                e.key = m.data1;
                e.velocity = kind == 0x90 ? m.data2 / 127.0 : 0.0;
            }
        }
        transport_ = clap_event_transport_t{};
        transport_.header = {sizeof transport_, 0, CLAP_CORE_EVENT_SPACE_ID, CLAP_EVENT_TRANSPORT, 0};
        transport_.flags = CLAP_TRANSPORT_HAS_TEMPO | CLAP_TRANSPORT_HAS_BEATS_TIMELINE |
                           CLAP_TRANSPORT_HAS_SECONDS_TIMELINE | CLAP_TRANSPORT_HAS_TIME_SIGNATURE |
                           (t.playing ? CLAP_TRANSPORT_IS_PLAYING : 0u);
        transport_.tempo = t.tempo_bpm;
        transport_.song_pos_beats = static_cast<clap_beattime>(t.beats() * static_cast<double>(CLAP_BEATTIME_FACTOR));
        transport_.song_pos_seconds = static_cast<clap_sectime>(static_cast<double>(t.position) / t.sample_rate *
                                                                static_cast<double>(CLAP_SECTIME_FACTOR));
        transport_.tsig_num = 4;
        transport_.tsig_denom = 4;

        if (in_channels_ == 2) {
            std::memcpy(in_l_.data(), l, sizeof(float) * frames);
            std::memcpy(in_r_.data(), r, sizeof(float) * frames);
        } else if (in_channels_ == 1) {
            for (uint32_t i = 0; i < frames; ++i) in_l_[i] = 0.5f * (l[i] + r[i]);
        }
        clap_process_t p{};
        p.steady_time = steady_time_;
        p.frames_count = frames;
        p.transport = &transport_;
        p.audio_inputs = in_channels_ > 0 ? &in_buf_ : nullptr;
        p.audio_inputs_count = in_channels_ > 0 ? 1 : 0;
        p.audio_outputs = &out_buf_;
        p.audio_outputs_count = 1;
        p.in_events = &in_events_;
        p.out_events = &out_events_;
        const clap_process_status st = plugin_->process(plugin_, &p);
        steady_time_ += frames;
        if (st == CLAP_PROCESS_ERROR) return pass(l, r, frames);
        std::memcpy(l, out_l_.data(), sizeof(float) * frames);
        std::memcpy(r, out_channels_ > 1 ? out_r_.data() : out_l_.data(), sizeof(float) * frames);
    }

    bool save_state(std::vector<uint8_t>& out, std::string& err) override {
        if (state_ == nullptr) {
            err = "plugin has no clap.state extension";
            return false;
        }
        out.clear();
        clap_ostream_t os{&out, [](const clap_ostream* s, const void* buf, uint64_t size) -> int64_t {
                              auto* v = static_cast<std::vector<uint8_t>*>(s->ctx);
                              const auto* b = static_cast<const uint8_t*>(buf);
                              v->insert(v->end(), b, b + size);
                              return static_cast<int64_t>(size);
                          }};
        if (!state_->save(plugin_, &os)) {
            err = "clap.state save failed";
            return false;
        }
        return true;
    }

    uint32_t latency() override { return latency_ != nullptr && activated_ ? latency_->get(plugin_) : 0; }

    std::unique_ptr<PluginEditor> open_editor(const std::string& title, std::string& err) override {
        if (gui_ == nullptr) {
            err = "the plugin has no editor (no clap.gui extension)";
            return nullptr;
        }
        std::weak_ptr<ClapInstance> weak = weak_from_this();
        return create_clap_editor(plugin_, gui_, title, [weak](ClapEditorHost* h) {
            if (auto self = weak.lock()) self->editor_host_ = h;
        }, err);
    }

    void shutdown() noexcept override {
        if (plugin_ == nullptr) return;
        if (processing_) {
            AudioThreadScope audio; // the RT side is gone; we take the audio role once
            plugin_->stop_processing(plugin_);
            processing_ = false;
        }
        if (activated_) plugin_->deactivate(plugin_);
        activated_ = false;
        plugin_->destroy(plugin_);
        plugin_ = nullptr;
        mod_.reset();
    }

    // Latency change protocol: the plugin called request_restart (e.g. after
    // a parameter changed its latency). Called by the PluginHost on the main
    // thread while the instance is suspended (the RT side cannot be inside
    // process()): stop_processing (taking the audio-thread role once, as in
    // shutdown()), deactivate, activate. process() calls start_processing
    // again on the next block. A failed activate leaves the instance passing
    // audio through (effects) / silent (instruments).
    void restart() noexcept override {
        if (plugin_ == nullptr) return;
        if (processing_) {
            AudioThreadScope audio;
            plugin_->stop_processing(plugin_);
            processing_ = false;
        }
        if (activated_) plugin_->deactivate(plugin_);
        activated_ = plugin_->activate(plugin_, rate_, 1, max_block_);
    }

    void on_main_thread() noexcept override {
        if (plugin_ != nullptr && callback_requested_.exchange(false)) plugin_->on_main_thread(plugin_);
    }

private:
    // Failure path: effects pass audio through, instruments are silent.
    void pass(float* l, float* r, uint32_t frames) noexcept {
        if (!spec_.instrument) return;
        std::memset(l, 0, sizeof(float) * frames);
        std::memset(r, 0, sizeof(float) * frames);
    }

    uint32_t port_channels(bool input) {
        if (audio_ports_ == nullptr || audio_ports_->count(plugin_, input) == 0) return 0;
        clap_audio_port_info_t info{};
        if (!audio_ports_->get(plugin_, 0, input, &info)) return 0;
        return info.channel_count;
    }

    bool load_state(const std::vector<uint8_t>& bytes) {
        struct Reader {
            const std::vector<uint8_t>* v;
            std::size_t pos;
        } reader{&bytes, 0};
        clap_istream_t is{&reader, [](const clap_istream* s, void* buf, uint64_t size) -> int64_t {
                              auto* rd = static_cast<Reader*>(s->ctx);
                              const std::size_t n = std::min<std::size_t>(size, rd->v->size() - rd->pos);
                              std::memcpy(buf, rd->v->data() + rd->pos, n);
                              rd->pos += n;
                              return static_cast<int64_t>(n);
                          }};
        return state_->load(plugin_, &is);
    }

    static ClapInstance* self_of(const clap_host* h) { return static_cast<ClapInstance*>(h->host_data); }

    static const void* host_get_extension(const clap_host* h, const char* id) {
        static const clap_host_thread_check_t thread_check{
            [](const clap_host* host) { return std::this_thread::get_id() == self_of(host)->main_thread_; },
            [](const clap_host*) { return t_clap_audio_thread; }};
        static const clap_host_log_t log{[](const clap_host*, clap_log_severity, const char*) {}};
        static const clap_host_latency_t latency{[](const clap_host*) {}};
        static const clap_host_state_t state{[](const clap_host*) {}};
        static const clap_host_params_t params{[](const clap_host*, clap_param_rescan_flags) {},
                                               [](const clap_host*, clap_id, clap_param_clear_flags) {},
                                               [](const clap_host*) {}};
        static const clap_host_gui_t gui{
            [](const clap_host*) {},
            [](const clap_host* host, uint32_t w, uint32_t hgt) {
                auto* self = self_of(host);
                return self->editor_host_ != nullptr && self->editor_host_->request_resize(w, hgt);
            },
            [](const clap_host*) { return false; },
            [](const clap_host*) { return false; },
            [](const clap_host* host, bool) {
                auto* self = self_of(host);
                if (self->editor_host_ != nullptr) self->editor_host_->plugin_closed();
            }};
        const std::string_view s(id);
        if (s == CLAP_EXT_THREAD_CHECK) return &thread_check;
        if (s == CLAP_EXT_LOG) return &log;
        if (s == CLAP_EXT_LATENCY) return &latency;
        if (s == CLAP_EXT_STATE) return &state;
        if (s == CLAP_EXT_PARAMS) return &params;
        if (s == CLAP_EXT_GUI) return &gui;
        (void)h;
        return nullptr;
    }

    static void host_request_callback(const clap_host* h) {
        auto* self = self_of(h);
        self->callback_requested_.store(true);
#if defined(__APPLE__)
        // Run on the main queue (the app's run loop); the instance may be
        // gone by then, hence the weak reference.
        std::weak_ptr<ClapInstance> weak;
        try {
            weak = self->weak_from_this();
        } catch (...) {
        }
        if (weak.expired()) return; // during init: picked up by se_plugin_idle
        auto* ctx = new std::weak_ptr<ClapInstance>(std::move(weak));
        dispatch_async_f(dispatch_get_main_queue(), ctx, [](void* c) {
            auto* w = static_cast<std::weak_ptr<ClapInstance>*>(c);
            if (auto inst = w->lock()) inst->on_main_thread();
            delete w;
        });
#endif
    }

    double rate_;
    uint32_t max_block_;
    std::shared_ptr<ClapModule> mod_;
    clap_host_t host_{};
    const clap_plugin_t* plugin_ = nullptr;
    const clap_plugin_audio_ports_t* audio_ports_ = nullptr;
    const clap_plugin_note_ports_t* note_ports_ = nullptr;
    const clap_plugin_state_t* state_ = nullptr;
    const clap_plugin_latency_t* latency_ = nullptr;
    const clap_plugin_gui_t* gui_ = nullptr;
    uint32_t in_channels_ = 0, out_channels_ = 0;
    bool use_midi_ = false;
    bool note_port_ = false;
    bool activated_ = false;
    std::vector<float> in_l_, in_r_, out_l_, out_r_;
    float* in_ptrs_[2] = {};
    float* out_ptrs_[2] = {};
    clap_audio_buffer_t in_buf_{}, out_buf_{};
    clap_input_events_t in_events_{};
    clap_output_events_t out_events_{};
    std::thread::id main_thread_;
    std::atomic<bool> callback_requested_{false};
    ClapEditorHost* editor_host_ = nullptr; // main thread
    // RT-owned
    bool processing_ = false;
    bool failed_ = false;
    int64_t steady_time_ = 0;
    clap_event_transport_t transport_{};
    uint32_t event_count_ = 0;
    ClapEventStorage event_store_[kMaxEvents + kMaxParamEvents];
};

} // namespace

std::shared_ptr<PluginInstance> create_clap_instance(const PluginSpec& spec, double sample_rate, uint32_t max_block,
                                                     std::string& err) {
    if (spec.path.empty()) {
        err = "CLAP device has no bundle path";
        return nullptr;
    }
    auto inst = std::make_shared<ClapInstance>(spec, sample_rate, max_block);
    if (!inst->init(err)) return nullptr;
    return inst;
}

} // namespace saine
