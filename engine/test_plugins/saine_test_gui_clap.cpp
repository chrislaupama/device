// SaineTestGui.clap ("studio.saine.test.gui"): original test plugin for the
// Windows editor host (src/platform/plugin_editor_win.cpp). Built on Windows
// only.
//  - Audio: stereo pass-through.
//  - clap.gui, CLAP_WINDOW_API_WIN32, embedded only: set_parent creates a
//    child "STATIC" window of the host window; resizable (min 100 x 60).
//  - show() asks the host to resize the editor to 400 x 250 once
//    (clap_host_gui.request_resize), so the test can check the host's
//    resize path.
#include <clap/clap.h>

#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <windows.h>

#include <algorithm>
#include <cstring>
#include <new>

namespace {

constexpr const char* kId = "studio.saine.test.gui";
const char* const kFeatures[] = {CLAP_PLUGIN_FEATURE_AUDIO_EFFECT, CLAP_PLUGIN_FEATURE_UTILITY, nullptr};
const clap_plugin_descriptor_t kDescriptor = {
    CLAP_VERSION_INIT, kId, "Saine Test GUI", "Saine", "", "", "", "1.0.0", "Pass-through with a Win32 editor",
    kFeatures};

struct Plugin {
    clap_plugin_t clap{};
    const clap_host_t* host = nullptr;
    HWND child = nullptr;
    uint32_t width = 320, height = 200;
    bool created = false;
    bool resized_once = false;
};

Plugin* self(const clap_plugin_t* p) { return static_cast<Plugin*>(p->plugin_data); }

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

// ---- gui
bool gui_is_api_supported(const clap_plugin_t*, const char* api, bool floating) {
    return !floating && std::strcmp(api, CLAP_WINDOW_API_WIN32) == 0;
}
bool gui_get_preferred_api(const clap_plugin_t*, const char** api, bool* floating) {
    *api = CLAP_WINDOW_API_WIN32;
    *floating = false;
    return true;
}
bool gui_create(const clap_plugin_t* p, const char* api, bool floating) {
    if (!gui_is_api_supported(p, api, floating)) return false;
    self(p)->created = true;
    return true;
}
void gui_destroy(const clap_plugin_t* p) {
    Plugin* s = self(p);
    if (s->child != nullptr) DestroyWindow(s->child);
    s->child = nullptr;
    s->created = false;
}
bool gui_set_scale(const clap_plugin_t*, double) { return false; }
bool gui_get_size(const clap_plugin_t* p, uint32_t* w, uint32_t* h) {
    *w = self(p)->width;
    *h = self(p)->height;
    return true;
}
bool gui_can_resize(const clap_plugin_t*) { return true; }
bool gui_get_resize_hints(const clap_plugin_t*, clap_gui_resize_hints_t* hints) {
    hints->can_resize_horizontally = true;
    hints->can_resize_vertically = true;
    hints->preserve_aspect_ratio = false;
    hints->aspect_ratio_width = 0;
    hints->aspect_ratio_height = 0;
    return true;
}
bool gui_adjust_size(const clap_plugin_t*, uint32_t* w, uint32_t* h) {
    *w = std::max<uint32_t>(*w, 100);
    *h = std::max<uint32_t>(*h, 60);
    return true;
}
bool gui_set_size(const clap_plugin_t* p, uint32_t w, uint32_t h) {
    Plugin* s = self(p);
    s->width = w;
    s->height = h;
    if (s->child != nullptr) MoveWindow(s->child, 0, 0, static_cast<int>(w), static_cast<int>(h), TRUE);
    return true;
}
bool gui_set_parent(const clap_plugin_t* p, const clap_window_t* window) {
    Plugin* s = self(p);
    if (!s->created || window == nullptr || std::strcmp(window->api, CLAP_WINDOW_API_WIN32) != 0) return false;
    s->child = CreateWindowExW(0, L"STATIC", L"Saine Test GUI", WS_CHILD | WS_VISIBLE, 0, 0, static_cast<int>(s->width),
                               static_cast<int>(s->height), static_cast<HWND>(window->win32), nullptr,
                               GetModuleHandleW(nullptr), nullptr);
    return s->child != nullptr;
}
bool gui_set_transient(const clap_plugin_t*, const clap_window_t*) { return false; }
void gui_suggest_title(const clap_plugin_t*, const char*) {}
bool gui_show(const clap_plugin_t* p) {
    Plugin* s = self(p);
    if (s->child != nullptr) ShowWindow(s->child, SW_SHOW);
    if (!s->resized_once && s->host != nullptr) {
        s->resized_once = true;
        const auto* hg = static_cast<const clap_host_gui_t*>(s->host->get_extension(s->host, CLAP_EXT_GUI));
        if (hg != nullptr && hg->request_resize(s->host, 400, 250)) gui_set_size(p, 400, 250);
    }
    return true;
}
bool gui_hide(const clap_plugin_t* p) {
    if (self(p)->child != nullptr) ShowWindow(self(p)->child, SW_HIDE);
    return true;
}
const clap_plugin_gui_t kGui = {gui_is_api_supported, gui_get_preferred_api, gui_create,   gui_destroy,
                                gui_set_scale,        gui_get_size,          gui_can_resize, gui_get_resize_hints,
                                gui_adjust_size,      gui_set_size,          gui_set_parent, gui_set_transient,
                                gui_suggest_title,    gui_show,              gui_hide};

// ---- lifecycle
bool init(const clap_plugin_t*) { return true; }
void destroy(const clap_plugin_t* p) {
    gui_destroy(p);
    delete self(p);
}
bool activate(const clap_plugin_t*, double, uint32_t, uint32_t) { return true; }
void deactivate(const clap_plugin_t*) {}
bool start_processing(const clap_plugin_t*) { return true; }
void stop_processing(const clap_plugin_t*) {}
void reset(const clap_plugin_t*) {}

clap_process_status process(const clap_plugin_t*, const clap_process_t* pr) {
    for (int c = 0; c < 2; ++c) {
        const float* in = pr->audio_inputs[0].data32[c];
        float* out = pr->audio_outputs[0].data32[c];
        if (out != in) std::memmove(out, in, sizeof(float) * pr->frames_count);
    }
    return CLAP_PROCESS_CONTINUE;
}

const void* get_extension(const clap_plugin_t*, const char* id) {
    if (std::strcmp(id, CLAP_EXT_AUDIO_PORTS) == 0) return &kAudioPorts;
    if (std::strcmp(id, CLAP_EXT_GUI) == 0) return &kGui;
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
