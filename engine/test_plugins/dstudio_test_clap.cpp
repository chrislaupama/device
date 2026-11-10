// DStudio test CLAP plugins (original, minimal; used by the engine tests only).
//
//  DSTUDIO_TEST_CLAP_GAIN: "studio.dstudio.test.gain"  stereo effect, one
//      parameter "Gain" (id 0, linear 0..1, default 0.5), applied
//      sample-accurately at each CLAP_EVENT_PARAM_VALUE's time. State = the
//      gain as a little-endian float32.
//      Editor-gesture test hooks (hidden, unlisted parameters): a
//      PARAM_VALUE event for id 1000 with value v acts like a knob drag in
//      the editor during process(): the gain becomes v and the plugin
//      outputs GESTURE_BEGIN(0), PARAM_VALUE(0, v), GESTURE_END(0) at that
//      event's time. Id 1001 does the same outside process(): the plugin
//      asks for a main-thread callback, requests a parameter flush from
//      there, and outputs the gesture from params.flush() (or the next
//      process(), whichever the host calls first).
//  DSTUDIO_TEST_CLAP_SINE: "studio.dstudio.test.sine"  instrument: 16 voices of
//      a cosine at the note's equal-tempered pitch (A4 = 440 Hz), amplitude
//      0.25 * velocity, starting at full amplitude on the note-on sample and
//      stopping at the note-off sample. State = 1 placeholder byte.
//
// Neither allocates in process() (the engine's RT test relies on it).
#include <clap/clap.h>

#include <atomic>
#include <cmath>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <new>

namespace {

#if defined(DSTUDIO_TEST_CLAP_GAIN)
constexpr const char* kId = "studio.dstudio.test.gain";
constexpr const char* kName = "DStudio Test Gain";
const char* const kFeatures[] = {CLAP_PLUGIN_FEATURE_AUDIO_EFFECT, CLAP_PLUGIN_FEATURE_UTILITY, nullptr};
constexpr bool kInstrument = false;
#elif defined(DSTUDIO_TEST_CLAP_SINE)
constexpr const char* kId = "studio.dstudio.test.sine";
constexpr const char* kName = "DStudio Test Sine";
const char* const kFeatures[] = {CLAP_PLUGIN_FEATURE_INSTRUMENT, CLAP_PLUGIN_FEATURE_SYNTHESIZER, nullptr};
constexpr bool kInstrument = true;
#else
#error define DSTUDIO_TEST_CLAP_GAIN or DSTUDIO_TEST_CLAP_SINE
#endif

const clap_plugin_descriptor_t kDescriptor = {
    CLAP_VERSION_INIT, kId, kName, "DStudio", "", "", "", "1.0.0", "Device Studio test plugin", kFeatures};

struct Voice {
    bool on = false;
    int16_t key = 0;
    double phase = 0.0; // radians
    double inc = 0.0;
    float amp = 0.0f;
};

struct Plugin {
    clap_plugin_t clap{};
    const clap_host_t* host = nullptr;
    std::atomic<float> gain{0.5f};
    // Gesture hooks (ids 1000/1001).
    std::atomic<int> pending{0}; // 0 none, 1 callback requested, 2 flush requested
    std::atomic<float> pending_value{0.0f};
    double rate = 48000.0;
    Voice voices[16];
};

Plugin* self(const clap_plugin_t* p) { return static_cast<Plugin*>(p->plugin_data); }

// ---- params (gain only)
uint32_t params_count(const clap_plugin_t*) { return kInstrument ? 0 : 1; }
bool params_info(const clap_plugin_t*, uint32_t index, clap_param_info_t* info) {
    if (kInstrument || index != 0) return false;
    std::memset(info, 0, sizeof *info);
    info->id = 0;
    info->flags = CLAP_PARAM_IS_AUTOMATABLE;
    std::strncpy(info->name, "Gain", sizeof info->name - 1);
    info->min_value = 0.0;
    info->max_value = 1.0;
    info->default_value = 0.5;
    return true;
}
bool params_value(const clap_plugin_t* p, clap_id id, double* v) {
    if (kInstrument || id != 0) return false;
    *v = self(p)->gain.load();
    return true;
}
bool params_to_text(const clap_plugin_t*, clap_id, double v, char* out, uint32_t cap) {
    return std::snprintf(out, cap, "%.3f", v) > 0;
}
bool params_from_text(const clap_plugin_t*, clap_id, const char* text, double* v) {
    *v = std::atof(text);
    return true;
}
constexpr clap_id kGestureNow = 1000;
constexpr clap_id kGestureFlush = 1001;

void emit_gesture(const clap_output_events_t* out, uint32_t time, double value) {
    if (out == nullptr) return;
    clap_event_param_gesture_t g{};
    g.header = {sizeof g, time, CLAP_CORE_EVENT_SPACE_ID, CLAP_EVENT_PARAM_GESTURE_BEGIN, 0};
    g.param_id = 0;
    out->try_push(out, &g.header);
    clap_event_param_value_t v{};
    v.header = {sizeof v, time, CLAP_CORE_EVENT_SPACE_ID, CLAP_EVENT_PARAM_VALUE, 0};
    v.param_id = 0;
    v.note_id = -1;
    v.port_index = -1;
    v.channel = -1;
    v.key = -1;
    v.value = value;
    out->try_push(out, &v.header);
    g.header.type = CLAP_EVENT_PARAM_GESTURE_END;
    out->try_push(out, &g.header);
}

// One input event (PARAM_VALUE only).
void apply_param_event(Plugin* s, const clap_event_header_t* h, const clap_output_events_t* out) {
    if (h->space_id != CLAP_CORE_EVENT_SPACE_ID || h->type != CLAP_EVENT_PARAM_VALUE) return;
    const auto* e = reinterpret_cast<const clap_event_param_value_t*>(h);
    if (e->param_id == 0) {
        s->gain.store(static_cast<float>(e->value));
    } else if (e->param_id == kGestureNow) {
        s->gain.store(static_cast<float>(e->value));
        emit_gesture(out, h->time, e->value);
    } else if (e->param_id == kGestureFlush) {
        s->pending_value.store(static_cast<float>(e->value));
        s->pending.store(1);
        s->host->request_callback(s->host);
    }
}

void emit_pending(Plugin* s, const clap_output_events_t* out) {
    if (s->pending.load() != 2) return;
    s->pending.store(0);
    const float v = s->pending_value.load();
    s->gain.store(v);
    emit_gesture(out, 0, v);
}

void params_flush(const clap_plugin_t* p, const clap_input_events_t* in, const clap_output_events_t* out) {
    Plugin* s = self(p);
    const uint32_t n = in->size(in);
    for (uint32_t i = 0; i < n; ++i) apply_param_event(s, in->get(in, i), out);
    emit_pending(s, out);
}
const clap_plugin_params_t kParams = {params_count, params_info, params_value, params_to_text, params_from_text,
                                      params_flush};

// ---- state
bool state_save(const clap_plugin_t* p, const clap_ostream_t* s) {
    if (kInstrument) {
        const uint8_t b = 1;
        return s->write(s, &b, 1) == 1;
    }
    const float g = self(p)->gain.load();
    uint8_t b[4];
    std::memcpy(b, &g, 4); // little-endian hosts only (arm64/x86-64)
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
    if (kInstrument) return got >= 1;
    if (got != 4) return false;
    float g = 0.0f;
    std::memcpy(&g, b, 4);
    if (!std::isfinite(g) || g < 0.0f || g > 1.0f) return false;
    self(p)->gain.store(g);
    return true;
}
const clap_plugin_state_t kState = {state_save, state_load};

// ---- ports
uint32_t audio_ports_count(const clap_plugin_t*, bool is_input) { return is_input && kInstrument ? 0 : 1; }
bool audio_ports_get(const clap_plugin_t*, uint32_t index, bool is_input, clap_audio_port_info_t* info) {
    if (index != 0 || (is_input && kInstrument)) return false;
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

uint32_t note_ports_count(const clap_plugin_t*, bool is_input) { return is_input && kInstrument ? 1 : 0; }
bool note_ports_get(const clap_plugin_t*, uint32_t index, bool is_input, clap_note_port_info_t* info) {
    if (!kInstrument || !is_input || index != 0) return false;
    std::memset(info, 0, sizeof *info);
    info->id = 0;
    info->supported_dialects = CLAP_NOTE_DIALECT_CLAP | CLAP_NOTE_DIALECT_MIDI;
    info->preferred_dialect = CLAP_NOTE_DIALECT_CLAP;
    std::strncpy(info->name, "Notes", sizeof info->name - 1);
    return true;
}
const clap_plugin_note_ports_t kNotePorts = {note_ports_count, note_ports_get};

// ---- lifecycle
bool init(const clap_plugin_t*) { return true; }
void destroy(const clap_plugin_t* p) { delete self(p); }
bool activate(const clap_plugin_t* p, double rate, uint32_t, uint32_t) {
    self(p)->rate = rate;
    return true;
}
void deactivate(const clap_plugin_t*) {}
bool start_processing(const clap_plugin_t*) { return true; }
void stop_processing(const clap_plugin_t*) {}
void reset(const clap_plugin_t* p) {
    for (Voice& v : self(p)->voices) v.on = false;
}

void note_on(Plugin* s, int16_t key, double velocity) {
    Voice* slot = nullptr;
    for (Voice& v : s->voices)
        if (!v.on) {
            slot = &v;
            break;
        }
    if (slot == nullptr) slot = &s->voices[0];
    slot->on = true;
    slot->key = key;
    slot->phase = 0.0; // cosine: full amplitude on the first sample
    slot->inc = 2.0 * 3.14159265358979323846 * 440.0 * std::pow(2.0, (key - 69) / 12.0) / s->rate;
    slot->amp = static_cast<float>(0.25 * velocity);
}
void note_off(Plugin* s, int16_t key) {
    for (Voice& v : s->voices)
        if (v.on && (key < 0 || v.key == key)) v.on = false;
}

void render_voices(Plugin* s, float* l, float* r, uint32_t from, uint32_t to) {
    for (Voice& v : s->voices) {
        if (!v.on) continue;
        for (uint32_t i = from; i < to; ++i) {
            const float x = v.amp * static_cast<float>(std::cos(v.phase));
            l[i] += x;
            r[i] += x;
            v.phase += v.inc;
        }
    }
}

clap_process_status process(const clap_plugin_t* p, const clap_process_t* pr) {
    Plugin* s = self(p);
    const uint32_t n = pr->frames_count;
    float* ol = pr->audio_outputs[0].data32[0];
    float* orr = pr->audio_outputs[0].data32[1];
    if (!kInstrument) {
        // Sample-accurate: each event applies from its time on.
        emit_pending(s, pr->out_events);
        const float* il = pr->audio_inputs[0].data32[0];
        const float* ir = pr->audio_inputs[0].data32[1];
        const clap_input_events_t* in = pr->in_events;
        const uint32_t ne = in->size(in);
        uint32_t pos = 0;
        for (uint32_t e = 0; e <= ne; ++e) {
            const clap_event_header_t* h = e < ne ? in->get(in, e) : nullptr;
            const uint32_t t = h == nullptr ? n : (h->time < n ? h->time : n);
            const float g = s->gain.load();
            for (uint32_t i = pos; i < t; ++i) {
                ol[i] = il[i] * g;
                orr[i] = ir[i] * g;
            }
            pos = t;
            if (h != nullptr) apply_param_event(s, h, pr->out_events);
        }
        return CLAP_PROCESS_CONTINUE;
    }
    std::memset(ol, 0, sizeof(float) * n);
    std::memset(orr, 0, sizeof(float) * n);
    const clap_input_events_t* in = pr->in_events;
    const uint32_t ne = in->size(in);
    uint32_t pos = 0;
    for (uint32_t e = 0; e < ne; ++e) {
        const clap_event_header_t* h = in->get(in, e);
        const uint32_t t = h->time < n ? h->time : n;
        render_voices(s, ol, orr, pos, t);
        pos = t;
        if (h->space_id != CLAP_CORE_EVENT_SPACE_ID) continue;
        if (h->type == CLAP_EVENT_NOTE_ON) {
            const auto* ev = reinterpret_cast<const clap_event_note_t*>(h);
            note_on(s, ev->key, ev->velocity);
        } else if (h->type == CLAP_EVENT_NOTE_OFF || h->type == CLAP_EVENT_NOTE_CHOKE) {
            note_off(s, reinterpret_cast<const clap_event_note_t*>(h)->key);
        } else if (h->type == CLAP_EVENT_MIDI) {
            const auto* ev = reinterpret_cast<const clap_event_midi_t*>(h);
            const uint8_t st = ev->data[0] & 0xF0;
            if (st == 0x90 && ev->data[2] > 0) note_on(s, ev->data[1], ev->data[2] / 127.0);
            else if (st == 0x80 || st == 0x90) note_off(s, ev->data[1]);
            else if (st == 0xB0 && ev->data[1] == 123) note_off(s, -1);
        }
    }
    render_voices(s, ol, orr, pos, n);
    return CLAP_PROCESS_CONTINUE;
}

const void* get_extension(const clap_plugin_t*, const char* id) {
    if (std::strcmp(id, CLAP_EXT_AUDIO_PORTS) == 0) return &kAudioPorts;
    if (std::strcmp(id, CLAP_EXT_STATE) == 0) return &kState;
    if (!kInstrument && std::strcmp(id, CLAP_EXT_PARAMS) == 0) return &kParams;
    if (kInstrument && std::strcmp(id, CLAP_EXT_NOTE_PORTS) == 0) return &kNotePorts;
    return nullptr;
}
void on_main_thread(const clap_plugin_t* p) {
    Plugin* s = self(p);
    int expected = 1;
    if (!s->pending.compare_exchange_strong(expected, 2)) return;
    const auto* hp = static_cast<const clap_host_params_t*>(s->host->get_extension(s->host, CLAP_EXT_PARAMS));
    if (hp != nullptr) hp->request_flush(s->host); // from the main thread, as the spec asks
}

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
