// DStudioTestGui.clap ("studio.dstudio.test.gui"): original test plugin for the
// editor hosts on Windows (src/platform/plugin_editor_win.cpp) and Linux
// (src/platform/plugin_editor_x11.cpp). Built on Windows, and on Linux with
// X11.
//  - Audio: stereo pass-through.
//  - clap.gui, embedded only (CLAP_WINDOW_API_WIN32 / CLAP_WINDOW_API_X11):
//    set_parent creates a child window of the host window; resizable (min
//    100 x 60).
//  - show() asks the host to resize the editor to 400 x 250 once
//    (clap_host_gui.request_resize), so the test can check the host's
//    resize path.
//  - Linux, like a real plugin: the GUI opens its own X connection and asks
//    the host to watch its fd (clap.posix-fd-support) and to run a 10 ms
//    timer (clap.timer-support). The child window's name reports the calls
//    ("DStudio Test GUI timer=N fd=M"), which the host test reads.
#include <clap/clap.h>

#if defined(_WIN32)
#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <windows.h>
constexpr const char* kApi = CLAP_WINDOW_API_WIN32;
#else
#include <X11/Xlib.h>
#include <X11/Xutil.h>

#include <cstdio>
#include <string>
constexpr const char* kApi = CLAP_WINDOW_API_X11;
#pragma GCC diagnostic ignored "-Wold-style-cast" // Xlib macros
#endif

#include <algorithm>
#include <cstring>
#include <new>

namespace {

constexpr const char* kId = "studio.dstudio.test.gui";
// Not listed by the factory: an instance that registers a host timer in
// init() and then fails it (the host must drop the timer).
constexpr const char* kInitFailsId = "studio.dstudio.test.gui.init-fails";
const char* const kFeatures[] = {CLAP_PLUGIN_FEATURE_AUDIO_EFFECT, CLAP_PLUGIN_FEATURE_UTILITY, nullptr};
const clap_plugin_descriptor_t kDescriptor = {
    CLAP_VERSION_INIT, kId, "DStudio Test GUI", "DStudio", "", "", "", "1.0.0", "Pass-through with an embedded editor",
    kFeatures};

// Per-instance state: the host, the editor's child window and its size.
struct Plugin {
    clap_plugin_t clap{};
    const clap_host_t* host = nullptr;
    bool init_fails = false;
#if defined(_WIN32)
    HWND child = nullptr;
#else
    Display* dpy = nullptr; // the GUI's own X connection
    Window child = 0;
    clap_id timer = CLAP_INVALID_ID;
    int fd = -1;
    unsigned timer_calls = 0, fd_calls = 0;
#endif
    uint32_t width = 320, height = 200;
    bool created = false;
    bool resized_once = false;
};

// The Plugin behind a clap_plugin.
Plugin* self(const clap_plugin_t* p) { return static_cast<Plugin*>(p->plugin_data); }

// ---- ports
// One main audio port in each direction.
uint32_t audio_ports_count(const clap_plugin_t*, bool) { return 1; }
// Describes the stereo main input (id 0) or output (id 1).
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
// Only embedded windows of the platform's API are supported.
bool gui_is_api_supported(const clap_plugin_t*, const char* api, bool floating) {
    return !floating && std::strcmp(api, kApi) == 0;
}
// Prefers an embedded window.
bool gui_get_preferred_api(const clap_plugin_t*, const char** api, bool* floating) {
    *api = kApi;
    *floating = false;
    return true;
}
// Marks the GUI created (the window itself is made in set_parent).
bool gui_create(const clap_plugin_t* p, const char* api, bool floating) {
    if (!gui_is_api_supported(p, api, floating)) return false;
    self(p)->created = true;
    return true;
}
#if !defined(_WIN32)
// The host's timer / fd extensions (nullptr if it has none).
const clap_host_timer_support_t* host_timers(const Plugin* s) {
    return static_cast<const clap_host_timer_support_t*>(s->host->get_extension(s->host, CLAP_EXT_TIMER_SUPPORT));
}
const clap_host_posix_fd_support_t* host_fds(const Plugin* s) {
    return static_cast<const clap_host_posix_fd_support_t*>(s->host->get_extension(s->host, CLAP_EXT_POSIX_FD_SUPPORT));
}
// Names the child window after the callback counts (the host test reads it).
void report(Plugin* s) {
    if (s->dpy == nullptr || s->child == 0) return;
    const std::string name =
        "DStudio Test GUI timer=" + std::to_string(s->timer_calls) + " fd=" + std::to_string(s->fd_calls);
    XStoreName(s->dpy, s->child, name.c_str());
    XFlush(s->dpy);
}
#endif

// Destroys the child window, if any (Linux: also the timer, the fd and the
// X connection).
void gui_destroy(const clap_plugin_t* p) {
    Plugin* s = self(p);
#if defined(_WIN32)
    if (s->child != nullptr) DestroyWindow(s->child);
    s->child = nullptr;
#else
    if (s->timer != CLAP_INVALID_ID) {
        if (const auto* t = host_timers(s)) t->unregister_timer(s->host, s->timer);
        s->timer = CLAP_INVALID_ID;
    }
    if (s->fd >= 0) {
        if (const auto* f = host_fds(s)) f->unregister_fd(s->host, s->fd);
        s->fd = -1;
    }
    if (s->dpy != nullptr) {
        if (s->child != 0) XDestroyWindow(s->dpy, s->child);
        XCloseDisplay(s->dpy);
    }
    s->dpy = nullptr;
    s->child = 0;
#endif
    s->created = false;
}
// No scaling support.
bool gui_set_scale(const clap_plugin_t*, double) { return false; }
// Current editor size.
bool gui_get_size(const clap_plugin_t* p, uint32_t* w, uint32_t* h) {
    *w = self(p)->width;
    *h = self(p)->height;
    return true;
}
// The editor is resizable.
bool gui_can_resize(const clap_plugin_t*) { return true; }
// Freely resizable in both directions, no fixed aspect ratio.
bool gui_get_resize_hints(const clap_plugin_t*, clap_gui_resize_hints_t* hints) {
    hints->can_resize_horizontally = true;
    hints->can_resize_vertically = true;
    hints->preserve_aspect_ratio = false;
    hints->aspect_ratio_width = 0;
    hints->aspect_ratio_height = 0;
    return true;
}
// Enforces the minimum size of 100 x 60.
bool gui_adjust_size(const clap_plugin_t*, uint32_t* w, uint32_t* h) {
    *w = std::max<uint32_t>(*w, 100);
    *h = std::max<uint32_t>(*h, 60);
    return true;
}
// Stores the size and resizes the child window to match.
bool gui_set_size(const clap_plugin_t* p, uint32_t w, uint32_t h) {
    Plugin* s = self(p);
    s->width = w;
    s->height = h;
#if defined(_WIN32)
    if (s->child != nullptr) MoveWindow(s->child, 0, 0, static_cast<int>(w), static_cast<int>(h), TRUE);
#else
    if (s->dpy != nullptr && s->child != 0) {
        XResizeWindow(s->dpy, s->child, w, h);
        XFlush(s->dpy);
    }
#endif
    return true;
}
// Creates the editor as a child window of the host's window (Windows: a
// "STATIC" window; Linux: an X window on the GUI's own connection, whose fd
// and a 10 ms timer are registered with the host).
bool gui_set_parent(const clap_plugin_t* p, const clap_window_t* window) {
    Plugin* s = self(p);
    if (!s->created || window == nullptr || std::strcmp(window->api, kApi) != 0) return false;
#if defined(_WIN32)
    s->child = CreateWindowExW(0, L"STATIC", L"DStudio Test GUI", WS_CHILD | WS_VISIBLE, 0, 0, static_cast<int>(s->width),
                               static_cast<int>(s->height), static_cast<HWND>(window->win32), nullptr,
                               GetModuleHandleW(nullptr), nullptr);
    return s->child != nullptr;
#else
    s->dpy = XOpenDisplay(nullptr);
    if (s->dpy == nullptr) return false;
    s->child = XCreateSimpleWindow(s->dpy, static_cast<Window>(window->x11), 0, 0, s->width, s->height, 0,
                                   BlackPixel(s->dpy, DefaultScreen(s->dpy)), WhitePixel(s->dpy, DefaultScreen(s->dpy)));
    if (s->child == 0) return false;
    XSelectInput(s->dpy, s->child, ExposureMask | StructureNotifyMask);
    XMapWindow(s->dpy, s->child);
    XFlush(s->dpy);
    if (const auto* f = host_fds(s); f != nullptr && f->register_fd(s->host, ConnectionNumber(s->dpy), CLAP_POSIX_FD_READ))
        s->fd = ConnectionNumber(s->dpy);
    if (const auto* t = host_timers(s); t == nullptr || !t->register_timer(s->host, 10, &s->timer))
        s->timer = CLAP_INVALID_ID;
    report(s);
    return true;
#endif
}
// Floating windows are not supported.
bool gui_set_transient(const clap_plugin_t*, const clap_window_t*) { return false; }
// Ignored (embedded only).
void gui_suggest_title(const clap_plugin_t*, const char*) {}
// Shows the child window; the first time, asks the host to resize the
// editor to 400 x 250 and applies it if the host agrees.
bool gui_show(const clap_plugin_t* p) {
    Plugin* s = self(p);
#if defined(_WIN32)
    if (s->child != nullptr) ShowWindow(s->child, SW_SHOW);
#else
    if (s->dpy != nullptr && s->child != 0) {
        XMapWindow(s->dpy, s->child);
        XFlush(s->dpy);
    }
#endif
    if (!s->resized_once && s->host != nullptr) {
        s->resized_once = true;
        const auto* hg = static_cast<const clap_host_gui_t*>(s->host->get_extension(s->host, CLAP_EXT_GUI));
        if (hg != nullptr && hg->request_resize(s->host, 400, 250)) gui_set_size(p, 400, 250);
    }
    return true;
}
// Hides the child window.
bool gui_hide(const clap_plugin_t* p) {
#if defined(_WIN32)
    if (self(p)->child != nullptr) ShowWindow(self(p)->child, SW_HIDE);
#else
    Plugin* s = self(p);
    if (s->dpy != nullptr && s->child != 0) {
        XUnmapWindow(s->dpy, s->child);
        XFlush(s->dpy);
    }
#endif
    return true;
}
const clap_plugin_gui_t kGui = {gui_is_api_supported, gui_get_preferred_api, gui_create,   gui_destroy,
                                gui_set_scale,        gui_get_size,          gui_can_resize, gui_get_resize_hints,
                                gui_adjust_size,      gui_set_size,          gui_set_parent, gui_set_transient,
                                gui_suggest_title,    gui_show,              gui_hide};

// ---- lifecycle
// Nothing to initialise, except for the kInitFailsId instance (see there).
bool init(const clap_plugin_t* p) {
    Plugin* s = self(p);
    if (!s->init_fails) return true;
#if !defined(_WIN32)
    // Left registered on purpose (not s->timer, which destroy() unregisters).
    // Fails only once the timer is in: if the host refused it, the load
    // succeeds and the test (which expects a load error) notices.
    clap_id leaked = CLAP_INVALID_ID;
    const auto* t = host_timers(s);
    if (t == nullptr || !t->register_timer(s->host, 10, &leaked)) return true;
#endif
    return false;
}
// Destroys the GUI and frees the instance.
void destroy(const clap_plugin_t* p) {
    gui_destroy(p);
    delete self(p);
}
// No processing state to set up or tear down.
bool activate(const clap_plugin_t*, double, uint32_t, uint32_t) { return true; }
void deactivate(const clap_plugin_t*) {}
bool start_processing(const clap_plugin_t*) { return true; }
void stop_processing(const clap_plugin_t*) {}
void reset(const clap_plugin_t*) {}

// Stereo pass-through.
clap_process_status process(const clap_plugin_t*, const clap_process_t* pr) {
    for (int c = 0; c < 2; ++c) {
        const float* in = pr->audio_inputs[0].data32[c];
        float* out = pr->audio_outputs[0].data32[c];
        if (out != in) std::memmove(out, in, sizeof(float) * pr->frames_count);
    }
    return CLAP_PROCESS_CONTINUE;
}

#if !defined(_WIN32)
// Host timer: counts the call.
void on_timer(const clap_plugin_t* p, clap_id) {
    Plugin* s = self(p);
    ++s->timer_calls;
    report(s);
}
const clap_plugin_timer_support_t kTimer = {on_timer};
// The X connection is readable: drain its events and count the call.
void on_fd(const clap_plugin_t* p, int, clap_posix_fd_flags_t) {
    Plugin* s = self(p);
    if (s->dpy == nullptr) return;
    while (XPending(s->dpy) > 0) {
        XEvent ev;
        XNextEvent(s->dpy, &ev);
    }
    ++s->fd_calls;
}
const clap_plugin_posix_fd_support_t kFd = {on_fd};
#endif

// Supports clap.audio-ports and clap.gui (Linux: also timer and fd support).
const void* get_extension(const clap_plugin_t*, const char* id) {
    if (std::strcmp(id, CLAP_EXT_AUDIO_PORTS) == 0) return &kAudioPorts;
    if (std::strcmp(id, CLAP_EXT_GUI) == 0) return &kGui;
#if !defined(_WIN32)
    if (std::strcmp(id, CLAP_EXT_TIMER_SUPPORT) == 0) return &kTimer;
    if (std::strcmp(id, CLAP_EXT_POSIX_FD_SUPPORT) == 0) return &kFd;
#endif
    return nullptr;
}
// No main-thread callbacks are requested.
void on_main_thread(const clap_plugin_t*) {}

// Creates an instance for kId; nullptr for any other id.
const clap_plugin_t* create(const clap_plugin_factory_t*, const clap_host_t* host, const char* id) {
    const bool init_fails = std::strcmp(id, kInitFailsId) == 0;
    if (std::strcmp(id, kId) != 0 && !init_fails) return nullptr;
    auto* s = new (std::nothrow) Plugin();
    if (s == nullptr) return nullptr;
    s->host = host;
    s->init_fails = init_fails;
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

// One plugin in the factory.
uint32_t factory_count(const clap_plugin_factory_t*) { return 1; }
// The descriptor of plugin 0.
const clap_plugin_descriptor_t* factory_descriptor(const clap_plugin_factory_t*, uint32_t i) {
    return i == 0 ? &kDescriptor : nullptr;
}
const clap_plugin_factory_t kFactory = {factory_count, factory_descriptor, create};

// Module init/deinit: nothing to do.
bool entry_init(const char*) { return true; }
void entry_deinit() {}
// The plugin factory; nullptr for any other factory id.
const void* entry_get_factory(const char* id) {
    return std::strcmp(id, CLAP_PLUGIN_FACTORY_ID) == 0 ? &kFactory : nullptr;
}

} // namespace

extern "C" CLAP_EXPORT const clap_plugin_entry_t clap_entry = {CLAP_VERSION_INIT, entry_init, entry_deinit,
                                                                entry_get_factory};
