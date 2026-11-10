// DStudioTestLatency.clap ("studio.dstudio.test.latency"): original test plugin
// for plugin delay compensation. A stereo effect that delays its input by
// exactly N frames and reports latency N (clap.latency).
//
//  - State: N as a little-endian uint32 (default N = 0).
//  - Parameter 0 "Latency" (plain value = frames, 0..kMaxLatency). A new
//    value is not applied while active: the plugin stores it and calls
//    host->request_restart(); the host deactivates and re-activates the
//    plugin on its main thread, activate() applies the pending N (and
//    allocates the delay buffer there) and calls clap_host_latency.changed().
//    This is the CLAP latency-change protocol, used by the engine tests
//    through the parameter test hook.
//  - reset() clears the delay buffer. process() never allocates.
#include <clap/clap.h>

#include <algorithm>
#include <atomic>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <new>
#include <vector>

namespace {

constexpr const char* kId = "studio.dstudio.test.latency";
constexpr uint32_t kMaxLatency = 1u << 20;
const char* const kFeatures[] = {CLAP_PLUGIN_FEATURE_AUDIO_EFFECT, CLAP_PLUGIN_FEATURE_UTILITY, nullptr};
const clap_plugin_descriptor_t kDescriptor = {
    CLAP_VERSION_INIT, kId, "DStudio Test Latency", "DStudio", "", "", "", "1.0.0", "Delays by N frames, reports N",
    kFeatures};

struct Plugin {
    clap_plugin_t clap{};
    const clap_host_t* host = nullptr;
    std::atomic<uint32_t> requested{0}; // pending N (parameter / state)
    uint32_t latency = 0;               // active N
    bool active = false;
    std::vector<float> buf[2];          // N frames per channel (ring)
    uint32_t pos = 0;
};

Plugin* self(const clap_plugin_t* p) { return static_cast<Plugin*>(p->plugin_data); }

void request_change(Plugin* s, uint32_t n) {
    if (n > kMaxLatency) n = kMaxLatency;
    s->requested.store(n);
    if (s->active && n != s->latency && s->host != nullptr && s->host->request_restart != nullptr)
        s->host->request_restart(s->host);
}

// ---- params
uint32_t params_count(const clap_plugin_t*) { return 1; }
bool params_info(const clap_plugin_t*, uint32_t index, clap_param_info_t* info) {
    if (index != 0) return false;
    std::memset(info, 0, sizeof *info);
    info->id = 0;
    info->flags = CLAP_PARAM_IS_STEPPED;
    std::strncpy(info->name, "Latency", sizeof info->name - 1);
    info->min_value = 0.0;
    info->max_value = kMaxLatency;
    info->default_value = 0.0;
    return true;
}
bool params_value(const clap_plugin_t* p, clap_id id, double* v) {
    if (id != 0) return false;
    *v = self(p)->requested.load();
    return true;
}
bool params_to_text(const clap_plugin_t*, clap_id, double v, char* out, uint32_t cap) {
    return std::snprintf(out, cap, "%.0f", v) > 0;
}
bool params_from_text(const clap_plugin_t*, clap_id, const char* text, double* v) {
    *v = std::atof(text);
    return true;
}
void apply_param_events(Plugin* s, const clap_input_events_t* in) {
    const uint32_t n = in->size(in);
    for (uint32_t i = 0; i < n; ++i) {
        const clap_event_header_t* h = in->get(in, i);
        if (h->space_id == CLAP_CORE_EVENT_SPACE_ID && h->type == CLAP_EVENT_PARAM_VALUE) {
            const auto* e = reinterpret_cast<const clap_event_param_value_t*>(h);
            if (e->param_id == 0 && e->value >= 0.0) request_change(s, static_cast<uint32_t>(e->value + 0.5));
        }
    }
}
void params_flush(const clap_plugin_t* p, const clap_input_events_t* in, const clap_output_events_t*) {
    apply_param_events(self(p), in);
}
const clap_plugin_params_t kParams = {params_count, params_info, params_value, params_to_text, params_from_text,
                                      params_flush};

// ---- latency
uint32_t latency_get(const clap_plugin_t* p) { return self(p)->latency; }
const clap_plugin_latency_t kLatency = {latency_get};

// ---- state
bool state_save(const clap_plugin_t* p, const clap_ostream_t* s) {
    const uint32_t n = self(p)->requested.load();
    uint8_t b[4];
    for (int i = 0; i < 4; ++i) b[i] = static_cast<uint8_t>(n >> (8 * i));
    return s->write(s, b, 4) == 4;
}
bool state_load(const clap_plugin_t* p, const clap_istream_t* s) {
    uint8_t b[4] = {};
    int64_t got = 0;
    while (got < 4) {
        const int64_t r = s->read(s, b + got, static_cast<uint64_t>(4 - got));
        if (r <= 0) break;
        got += r;
    }
    if (got != 4) return false;
    uint32_t n = 0;
    for (int i = 0; i < 4; ++i) n |= static_cast<uint32_t>(b[i]) << (8 * i);
    if (n > kMaxLatency) return false;
    request_change(self(p), n);
    return true;
}
const clap_plugin_state_t kState = {state_save, state_load};

// ---- ports
uint32_t audio_ports_count(const clap_plugin_t*, bool) { return 1; }
bool audio_ports_get(const clap_plugin_t*, uint32_t index, bool is_input, clap_audio_port_info_t* info) {
    if (index != 0) return false;
    std::memset(info, 0, sizeof *info);
    info->id = is_input ? 0 : 1;
    std::strncpy(info->name, is_input ? "In" : "Out", sizeof info->name - 1);
    info->flags = CLAP_AUDIO_PORT_IS_MAIN;
    info->channel_count = 2;
    info->port_type = CLAP_PORT_STEREO;
    info->in_place_pair = CLAP_INVALID_ID;
    return true;
}
const clap_plugin_audio_ports_t kAudioPorts = {audio_ports_count, audio_ports_get};

// ---- lifecycle
bool init(const clap_plugin_t*) { return true; }
void destroy(const clap_plugin_t* p) { delete self(p); }
bool activate(const clap_plugin_t* p, double, uint32_t, uint32_t) {
    Plugin* s = self(p);
    const uint32_t before = s->latency;
    s->latency = s->requested.load();
    for (auto& b : s->buf) b.assign(s->latency, 0.0f); // main thread: allocation allowed
    s->pos = 0;
    s->active = true;
    if (before != s->latency && s->host != nullptr) {
        const auto* hl = static_cast<const clap_host_latency_t*>(s->host->get_extension(s->host, CLAP_EXT_LATENCY));
        if (hl != nullptr && hl->changed != nullptr) hl->changed(s->host);
    }
    return true;
}
void deactivate(const clap_plugin_t* p) { self(p)->active = false; }
bool start_processing(const clap_plugin_t*) { return true; }
void stop_processing(const clap_plugin_t*) {}
void reset(const clap_plugin_t* p) {
    for (auto& b : self(p)->buf) std::fill(b.begin(), b.end(), 0.0f);
}

clap_process_status process(const clap_plugin_t* p, const clap_process_t* pr) {
    Plugin* s = self(p);
    apply_param_events(s, pr->in_events);
    const uint32_t n = pr->frames_count;
    const uint32_t lat = s->latency;
    for (int c = 0; c < 2; ++c) {
        const float* in = pr->audio_inputs[0].data32[c];
        float* out = pr->audio_outputs[0].data32[c];
        if (lat == 0) {
            if (out != in) std::memmove(out, in, sizeof(float) * n);
            continue;
        }
        float* ring = s->buf[c].data();
        uint32_t pos = s->pos;
        for (uint32_t i = 0; i < n; ++i) {
            const float x = in[i];
            out[i] = ring[pos];
            ring[pos] = x;
            if (++pos == lat) pos = 0;
        }
    }
    if (lat != 0) s->pos = static_cast<uint32_t>((s->pos + static_cast<uint64_t>(n)) % lat);
    return CLAP_PROCESS_CONTINUE;
}

const void* get_extension(const clap_plugin_t*, const char* id) {
    if (std::strcmp(id, CLAP_EXT_AUDIO_PORTS) == 0) return &kAudioPorts;
    if (std::strcmp(id, CLAP_EXT_STATE) == 0) return &kState;
    if (std::strcmp(id, CLAP_EXT_PARAMS) == 0) return &kParams;
    if (std::strcmp(id, CLAP_EXT_LATENCY) == 0) return &kLatency;
    return nullptr;
}
void on_main_thread(const clap_plugin_t*) {}

const clap_plugin_t* create(const clap_plugin_factory_t*, const clap_host_t* host, const char* id) {
    if (std::strcmp(id, kId) != 0) return nullptr;
    auto* s = new (std::nothrow) Plugin();
    if (s == nullptr) return nullptr;
    s->host = host;
    s->clap.desc = &kDescriptor;
    s->clap.plugin_data = s;
    s->clap.init = init;
    s->clap.destroy = destroy;
    s->clap.activate = activate;
    s->clap.deactivate = deactivate;
    s->clap.start_processing = start_processing;
    s->clap.stop_processing = stop_processing;
    s->clap.reset = reset;
    s->clap.process = process;
    s->clap.get_extension = get_extension;
    s->clap.on_main_thread = on_main_thread;
    return &s->clap;
}

uint32_t factory_count(const clap_plugin_factory_t*) { return 1; }
const clap_plugin_descriptor_t* factory_descriptor(const clap_plugin_factory_t*, uint32_t i) {
    return i == 0 ? &kDescriptor : nullptr;
}
const clap_plugin_factory_t kFactory = {factory_count, factory_descriptor, create};

bool entry_init(const char*) { return true; }
void entry_deinit() {}
const void* entry_get_factory(const char* id) {
    return std::strcmp(id, CLAP_PLUGIN_FACTORY_ID) == 0 ? &kFactory : nullptr;
}

} // namespace

extern "C" CLAP_EXPORT const clap_plugin_entry_t clap_entry = {CLAP_VERSION_INIT, entry_init, entry_deinit,
                                                                entry_get_factory};
