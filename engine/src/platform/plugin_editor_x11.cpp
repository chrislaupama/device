// Plugin editor windows on Linux (X11): one top-level window per editor
// whose client area hosts the plugin's child window. MAIN THREAD ONLY (the
// thread that calls se_plugin_editor_* and se_plugin_idle).
//
//  - VST3: IPlugView attached with kPlatformTypeX11EmbedWindowID. The
//          IPlugFrame also implements Linux::IRunLoop, which plugins need
//          for their fds (usually their own X connection) and timers.
//  - CLAP: clap.gui embedded (CLAP_WINDOW_API_X11); fds and timers go
//          through clap.posix-fd-support / clap.timer-support, which the
//          instance provides (clap_instance.cpp).
// Both run on the plugin run loop (run_loop.hpp), which platform_plugin_idle()
// dispatches together with this file's own X events (close box, user
// resize). The app calls se_plugin_idle from its UI timer.
//
// The host keeps its own X connection, separate from the UI toolkit's. A
// Wayland session needs XWayland for plugin editors (every Linux plugin GUI
// is X11). X errors on this connection (a plugin destroying its child window
// first, say) are logged and ignored; errors on other connections go to the
// handler that was installed before ours.
//
// Not handled yet: XEmbed focus negotiation, HiDPI scaling (the window is
// created at the plugin's pixel size).
#if defined(__linux__) && defined(DSTUDIO_HAVE_X11)

#include "plugins/plugin_editor.hpp"
#include "run_loop.hpp"

#include <clap/clap.h>
#include <pluginterfaces/base/funknownimpl.h>
#include <pluginterfaces/gui/iplugview.h>

// Xlib last: it defines macros (None, Bool, Status, Success, ...) that clash
// with other headers.
#include <X11/Xatom.h>
#include <X11/Xlib.h>
#include <X11/Xutil.h>

#include <dlfcn.h>

#include <algorithm>
#include <cstdio>
#include <cstring>
#include <string>
#include <vector>

#pragma GCC diagnostic ignored "-Wold-style-cast" // Xlib macros (DefaultRootWindow, ...)

namespace dstudio {
namespace {

// libX11 is loaded with dlopen when the first editor window opens, not
// linked: linking it made every process that uses the engine load libX11,
// libxcb and their dependencies at start-up (benchmarks, the headless tests,
// a Wayland-only system without libX11). The Xlib calls below go through
// these pointers; the macros after the struct keep the call sites plain Xlib.
#define DSTUDIO_XLIB_FUNCTIONS(F) \
    F(XAllocSizeHints) \
    F(XChangeProperty) \
    F(XCreateWindow) \
    F(XDestroyWindow) \
    F(XFlush) \
    F(XFree) \
    F(XGetErrorText) \
    F(XInternAtom) \
    F(XMapRaised) \
    F(XNextEvent) \
    F(XOpenDisplay) \
    F(XPending) \
    F(XResizeWindow) \
    F(XSendEvent) \
    F(XSetClassHint) \
    F(XSetErrorHandler) \
    F(XSetWMNormalHints) \
    F(XSetWMProtocols) \
    F(XStoreName) \
    F(XSync)

struct XlibFunctions {
#define DSTUDIO_XLIB_MEMBER(name) decltype(&::name) name = nullptr;
    DSTUDIO_XLIB_FUNCTIONS(DSTUDIO_XLIB_MEMBER)
#undef DSTUDIO_XLIB_MEMBER
};

XlibFunctions g_xlib;

// Loads libX11 once; false (with err set) when it isn't installed.
bool load_xlib(std::string& err) {
    static bool loaded = false;
    if (loaded) return true;
    void* lib = dlopen("libX11.so.6", RTLD_NOW | RTLD_LOCAL);
    if (lib == nullptr) {
        err = "plugin editor windows need libX11 (libX11.so.6 was not found)";
        return false;
    }
    XlibFunctions f;
#define DSTUDIO_XLIB_LOAD(name) \
    f.name = reinterpret_cast<decltype(f.name)>(dlsym(lib, #name)); \
    if (f.name == nullptr) { \
        err = "libX11 has no " #name; \
        return false; \
    }
    DSTUDIO_XLIB_FUNCTIONS(DSTUDIO_XLIB_LOAD)
#undef DSTUDIO_XLIB_LOAD
    g_xlib = f;
    loaded = true; // the library stays loaded for the rest of the process
    return true;
}

#define XAllocSizeHints g_xlib.XAllocSizeHints
#define XChangeProperty g_xlib.XChangeProperty
#define XCreateWindow g_xlib.XCreateWindow
#define XDestroyWindow g_xlib.XDestroyWindow
#define XFlush g_xlib.XFlush
#define XFree g_xlib.XFree
#define XGetErrorText g_xlib.XGetErrorText
#define XInternAtom g_xlib.XInternAtom
#define XMapRaised g_xlib.XMapRaised
#define XNextEvent g_xlib.XNextEvent
#define XOpenDisplay g_xlib.XOpenDisplay
#define XPending g_xlib.XPending
#define XResizeWindow g_xlib.XResizeWindow
#define XSendEvent g_xlib.XSendEvent
#define XSetClassHint g_xlib.XSetClassHint
#define XSetErrorHandler g_xlib.XSetErrorHandler
#define XSetWMNormalHints g_xlib.XSetWMNormalHints
#define XSetWMProtocols g_xlib.XSetWMProtocols
#define XStoreName g_xlib.XStoreName
#define XSync g_xlib.XSync

constexpr const char* kWindowClass = "DeviceStudioPluginEditor";

class X11Editor;

// The host's X connection and the editors living on it.
struct XHost {
    Display* dpy = nullptr;
    Atom wm_protocols = 0, wm_delete = 0, net_wm_name = 0, utf8 = 0;
    XErrorHandler previous = nullptr;
    std::vector<X11Editor*> editors;
};

XHost& xhost() {
    static XHost h;
    return h;
}

// Ignores (logs) errors on our connection; passes the rest on.
int on_x_error(Display* d, XErrorEvent* e) {
    XHost& h = xhost();
    if (d == h.dpy) {
        char text[128] = {};
        XGetErrorText(d, e->error_code, text, sizeof text);
        std::fprintf(stderr, "dstudio: X error in a plugin editor window: %s (request %d)\n", text, e->request_code);
        return 0;
    }
    return h.previous != nullptr ? h.previous(d, e) : 0;
}

// Opens the host's X connection on first use.
Display* display(std::string& err) {
    XHost& h = xhost();
    if (h.dpy != nullptr) return h.dpy;
    if (!load_xlib(err)) return nullptr;
    h.dpy = XOpenDisplay(nullptr);
    if (h.dpy == nullptr) {
        err = "cannot open the X11 display for the plugin editor (DISPLAY is not set; on Wayland, plugin editors need "
              "XWayland)";
        return nullptr;
    }
    h.wm_protocols = XInternAtom(h.dpy, "WM_PROTOCOLS", False);
    h.wm_delete = XInternAtom(h.dpy, "WM_DELETE_WINDOW", False);
    h.net_wm_name = XInternAtom(h.dpy, "_NET_WM_NAME", False);
    h.utf8 = XInternAtom(h.dpy, "UTF8_STRING", False);
    h.previous = XSetErrorHandler(&on_x_error);
    return h.dpy;
}

// Base of the X11 editor windows: owns the top-level window and routes its
// close and resize events to the plugin-specific subclass.
class X11Editor : public PluginEditor {
public:
    ~X11Editor() override { close(); }

    // False once the window has been closed by the user or by close().
    bool is_open() const noexcept override { return open_; }

    // Tears the plugin view down, then destroys the window. Idempotent.
    void close() noexcept override {
        if (!open_) return;
        open_ = false;
        teardown(); // while the window (the plugin's parent) still exists
        XHost& h = xhost();
        std::erase(h.editors, this);
        if (win_ != 0) {
            XDestroyWindow(h.dpy, win_);
            XFlush(h.dpy);
        }
        win_ = 0;
    }

    // Raises the window (and asks the window manager to focus it).
    void bring_to_front() noexcept override {
        if (!open_) return;
        Display* d = xhost().dpy;
        XMapRaised(d, win_);
        XEvent ev{};
        ev.xclient.type = ClientMessage;
        ev.xclient.window = win_;
        ev.xclient.message_type = XInternAtom(d, "_NET_ACTIVE_WINDOW", False);
        ev.xclient.format = 32;
        ev.xclient.data.l[0] = 2; // source: pager / user action
        ev.xclient.data.l[1] = CurrentTime;
        XSendEvent(d, DefaultRootWindow(d), False, SubstructureRedirectMask | SubstructureNotifyMask, &ev);
        XFlush(d);
    }

    Window window() const noexcept { return win_; }

    // One event for this window (from pump_events()).
    void handle(const XEvent& ev) noexcept {
        const XHost& h = xhost();
        if (ev.type == ClientMessage && ev.xclient.message_type == h.wm_protocols &&
            static_cast<Atom>(ev.xclient.data.l[0]) == h.wm_delete) {
            close();
        } else if (ev.type == ConfigureNotify && open_) {
            const auto w = static_cast<uint32_t>(ev.xconfigure.width);
            const auto hgt = static_cast<uint32_t>(ev.xconfigure.height);
            if (w != width_ || hgt != height_) {
                width_ = w;
                height_ = hgt;
                if (!in_resize_) window_resized(w, hgt);
            }
        }
    }

protected:
    // The user resized the window to w x h (no-op by default).
    virtual void window_resized(uint32_t, uint32_t) noexcept {}
    // Detaches and destroys the plugin's view (the window still exists).
    virtual void teardown() noexcept = 0;

    // Creates the (unmapped) window, w x h, and makes sure the server knows it
    // before its id goes to the plugin (which uses a connection of its own).
    bool create_window(const std::string& title, uint32_t w, uint32_t h, bool resizable, std::string& err) {
        Display* d = display(err);
        if (d == nullptr) return false;
        XHost& xh = xhost();
        width_ = std::max<uint32_t>(w, 100);
        height_ = std::max<uint32_t>(h, 50);
        resizable_ = resizable;
        XSetWindowAttributes a{};
        a.event_mask = StructureNotifyMask;
        a.background_pixel = BlackPixel(d, DefaultScreen(d));
        win_ = XCreateWindow(d, DefaultRootWindow(d), 0, 0, width_, height_, 0, CopyFromParent, InputOutput,
                             CopyFromParent, CWEventMask | CWBackPixel, &a);
        if (win_ == 0) {
            err = "XCreateWindow failed";
            return false;
        }
        const std::string name = title.empty() ? std::string("Plugin") : title;
        XStoreName(d, win_, name.c_str());
        XChangeProperty(d, win_, xh.net_wm_name, xh.utf8, 8, PropModeReplace,
                        reinterpret_cast<const unsigned char*>(name.data()), static_cast<int>(name.size()));
        XClassHint cls{};
        std::string res_name = "device-studio", res_class = kWindowClass;
        cls.res_name = res_name.data();
        cls.res_class = res_class.data();
        XSetClassHint(d, win_, &cls);
        Atom protocols[] = {xh.wm_delete};
        XSetWMProtocols(d, win_, protocols, 1);
        apply_size_hints();
        XSync(d, False);
        xh.editors.push_back(this);
        open_ = true;
        return true;
    }

    // Maps the window.
    void show() noexcept {
        XMapRaised(xhost().dpy, win_);
        XFlush(xhost().dpy);
    }

    // Resizes the window to w x h (a plugin request: no window_resized()).
    void set_client_size(uint32_t w, uint32_t h) noexcept {
        if (!open_) return;
        width_ = std::max<uint32_t>(w, 1);
        height_ = std::max<uint32_t>(h, 1);
        apply_size_hints();
        in_resize_ = true;
        XResizeWindow(xhost().dpy, win_, width_, height_);
        XFlush(xhost().dpy);
        in_resize_ = false;
    }

    // Fixed-size windows: min = max = the current size.
    void apply_size_hints() noexcept {
        XSizeHints* hints = XAllocSizeHints();
        if (hints == nullptr) return;
        if (!resizable_) {
            hints->flags = PMinSize | PMaxSize;
            hints->min_width = hints->max_width = static_cast<int>(width_);
            hints->min_height = hints->max_height = static_cast<int>(height_);
        }
        XSetWMNormalHints(xhost().dpy, win_, hints);
        XFree(hints);
    }

    Window win_ = 0;
    uint32_t width_ = 0, height_ = 0;
    bool open_ = false;
    bool resizable_ = false;
    bool in_resize_ = false;
};

// Handles the host connection's queued events.
void pump_events() {
    XHost& h = xhost();
    if (h.dpy == nullptr) return;
    while (XPending(h.dpy) > 0) {
        XEvent ev{};
        XNextEvent(h.dpy, &ev);
        for (X11Editor* e : h.editors)
            if (e->window() == ev.xany.window) {
                e->handle(ev); // may close (and unlist) the editor
                break;
            }
    }
}

// ---------------------------------------------------------------- VST3

class Vst3Editor;

// Fd watches of VST3 views, counted per (handler, fd). Plugins often share
// one handler between their views: the fd is registered once, under the
// handler, so it is reported once per readiness however many views asked
// for it, and closing one view leaves the others' watch in place.
struct FdWatch {
    Steinberg::Linux::IEventHandler* handler;
    int fd;
    int views;
};
std::vector<FdWatch> g_fd_watches; // main thread only

bool watch_fd(Steinberg::Linux::IEventHandler* handler, int fd) {
    for (FdWatch& w : g_fd_watches)
        if (w.handler == handler && w.fd == fd) {
            ++w.views;
            return true;
        }
    if (!run_loop::add_fd(handler, fd, run_loop::kRead, [handler](int f, uint32_t) { handler->onFDIsSet(f); }))
        return false;
    g_fd_watches.push_back({handler, fd, 1});
    return true;
}

void unwatch_fd(Steinberg::Linux::IEventHandler* handler, int fd) {
    for (auto it = g_fd_watches.begin(); it != g_fd_watches.end(); ++it)
        if (it->handler == handler && it->fd == fd) {
            if (--it->views == 0) {
                run_loop::remove_fd(handler, fd);
                g_fd_watches.erase(it);
            }
            return;
        }
}

// IPlugFrame + Linux::IRunLoop given to a VST3 view: resize requests go to
// the editor, fds and timers to the plugin run loop.
class PlugFrame final
    : public Steinberg::U::Implements<Steinberg::U::Directly<Steinberg::IPlugFrame, Steinberg::Linux::IRunLoop>> {
public:
    explicit PlugFrame(Vst3Editor* owner) : owner_(owner) {}
    ~PlugFrame() override { detach(); }

    Steinberg::tresult PLUGIN_API resizeView(Steinberg::IPlugView* view, Steinberg::ViewRect* r) override;

    Steinberg::tresult PLUGIN_API registerEventHandler(Steinberg::Linux::IEventHandler* handler,
                                                       Steinberg::Linux::FileDescriptor fd) override {
        if (handler == nullptr || fd < 0) return Steinberg::kInvalidArgument;
        if (std::find(fds_.begin(), fds_.end(), std::pair{handler, static_cast<int>(fd)}) != fds_.end())
            return Steinberg::kResultOk; // already watched for this view
        if (!watch_fd(handler, fd)) return Steinberg::kResultFalse;
        handler->addRef();
        fds_.push_back({handler, fd});
        return Steinberg::kResultOk;
    }

    Steinberg::tresult PLUGIN_API unregisterEventHandler(Steinberg::Linux::IEventHandler* handler) override {
        bool found = false;
        for (auto it = fds_.begin(); it != fds_.end();) {
            if (it->first != handler) {
                ++it;
                continue;
            }
            unwatch_fd(handler, it->second);
            handler->release();
            it = fds_.erase(it);
            found = true;
        }
        return found ? Steinberg::kResultOk : Steinberg::kInvalidArgument;
    }

    Steinberg::tresult PLUGIN_API registerTimer(Steinberg::Linux::ITimerHandler* handler,
                                                Steinberg::Linux::TimerInterval ms) override {
        if (handler == nullptr || ms == 0) return Steinberg::kInvalidArgument;
        const uint32_t id = run_loop::add_timer(this, static_cast<uint32_t>(std::min<uint64_t>(ms, 1u << 30)),
                                                [handler](uint32_t) { handler->onTimer(); });
        if (id == 0) return Steinberg::kResultFalse;
        handler->addRef();
        timers_.push_back({handler, id});
        return Steinberg::kResultOk;
    }

    Steinberg::tresult PLUGIN_API unregisterTimer(Steinberg::Linux::ITimerHandler* handler) override {
        bool found = false;
        for (auto it = timers_.begin(); it != timers_.end();) {
            if (it->first != handler) {
                ++it;
                continue;
            }
            run_loop::remove_timer(this, it->second);
            handler->release();
            it = timers_.erase(it);
            found = true;
        }
        return found ? Steinberg::kResultOk : Steinberg::kInvalidArgument;
    }

    // Stops forwarding resizes and drops every fd and timer the view left
    // registered (the editor is going away).
    void detach() {
        owner_ = nullptr;
        for (auto& [h, fd] : fds_) {
            unwatch_fd(h, fd);
            h->release();
        }
        fds_.clear();
        for (auto& [h, id] : timers_) {
            run_loop::remove_timer(this, id);
            h->release();
        }
        timers_.clear();
    }

private:
    Vst3Editor* owner_;
    std::vector<std::pair<Steinberg::Linux::IEventHandler*, int>> fds_;
    std::vector<std::pair<Steinberg::Linux::ITimerHandler*, uint32_t>> timers_;
};

// A VST3 IPlugView embedded in a top-level window (kPlatformTypeX11EmbedWindowID).
class Vst3Editor final : public X11Editor {
public:
    // Takes ownership of one reference to `view`.
    explicit Vst3Editor(Steinberg::IPlugView* view) : view_(view) {}

    ~Vst3Editor() override {
        close();
        teardown();
        if (view_ != nullptr) view_->release();
        view_ = nullptr;
    }

    // Creates the window at the view's size (400 x 300 if it reports none),
    // attaches the view and maps the window. False with `err` set on failure.
    bool open(const std::string& title, std::string& err) {
        using namespace Steinberg;
        if (view_->isPlatformTypeSupported(kPlatformTypeX11EmbedWindowID) != kResultTrue) {
            err = "the VST3 editor does not support X11 embedding";
            return false;
        }
        ViewRect r;
        if (view_->getSize(&r) != kResultOk || r.getWidth() <= 0 || r.getHeight() <= 0) r = ViewRect(0, 0, 400, 300);
        if (!create_window(title, static_cast<uint32_t>(r.getWidth()), static_cast<uint32_t>(r.getHeight()),
                           view_->canResize() == kResultTrue, err))
            return false;
        frame_ = new PlugFrame(this);
        view_->setFrame(frame_);
        // The X window id travels as the pointer value.
        if (view_->attached(reinterpret_cast<void*>(win_), kPlatformTypeX11EmbedWindowID) != kResultOk) {
            err = "IPlugView::attached failed";
            close();
            return false;
        }
        attached_ = true;
        show();
        return true;
    }

    // IPlugFrame::resizeView: resizes the window and confirms the size to the view.
    bool resize_from_plugin(Steinberg::ViewRect* r) {
        if (!open_ || r == nullptr) return false;
        set_client_size(static_cast<uint32_t>(r->getWidth()), static_cast<uint32_t>(r->getHeight()));
        view_->onSize(r);
        return true;
    }

private:
    // User resize: snaps to the view's constraints and passes the size on.
    void window_resized(uint32_t w, uint32_t h) noexcept override {
        if (!resizable_ || !attached_) return;
        Steinberg::ViewRect r(0, 0, static_cast<Steinberg::int32>(w), static_cast<Steinberg::int32>(h));
        view_->checkSizeConstraint(&r);
        if (r.getWidth() != static_cast<Steinberg::int32>(w) || r.getHeight() != static_cast<Steinberg::int32>(h))
            set_client_size(static_cast<uint32_t>(r.getWidth()), static_cast<uint32_t>(r.getHeight()));
        view_->onSize(&r);
    }

    // Removes the view and releases the frame (dropping its fds and timers).
    void teardown() noexcept override {
        if (attached_) {
            view_->removed();
            attached_ = false;
        }
        if (frame_ != nullptr) {
            view_->setFrame(nullptr);
            frame_->detach();
            frame_->release();
            frame_ = nullptr;
        }
    }

    Steinberg::IPlugView* view_;
    PlugFrame* frame_ = nullptr;
    bool attached_ = false;
};

Steinberg::tresult PLUGIN_API PlugFrame::resizeView(Steinberg::IPlugView*, Steinberg::ViewRect* r) {
    return owner_ != nullptr && owner_->resize_from_plugin(r) ? Steinberg::kResultOk : Steinberg::kResultFalse;
}

// ---------------------------------------------------------------- CLAP

// A CLAP clap.gui embedded in a top-level window (CLAP_WINDOW_API_X11).
class ClapEditor final : public X11Editor, public ClapEditorHost {
public:
    ClapEditor(const clap_plugin* p, const clap_plugin_gui* g, std::function<void(ClapEditorHost*)> attach)
        : plugin_(p), gui_(g), attach_(std::move(attach)) {}

    ~ClapEditor() override {
        close();
        teardown();
    }

    // Creates the plugin GUI and the window at the GUI's size (400 x 300 if it
    // reports none), parents the GUI in it and shows both. False with `err`
    // set on failure.
    bool open(const std::string& title, std::string& err) {
        if (!gui_->is_api_supported(plugin_, CLAP_WINDOW_API_X11, false)) {
            err = "the CLAP editor does not support embedded X11 windows";
            return false;
        }
        attach_(this);
        if (!gui_->create(plugin_, CLAP_WINDOW_API_X11, false)) {
            attach_(nullptr);
            err = "clap.gui create failed";
            return false;
        }
        created_ = true;
        uint32_t w = 0, h = 0;
        if (!gui_->get_size(plugin_, &w, &h) || w == 0 || h == 0) {
            w = 400;
            h = 300;
        }
        if (!create_window(title, w, h, gui_->can_resize(plugin_), err)) {
            teardown();
            return false;
        }
        clap_window_t win{};
        win.api = CLAP_WINDOW_API_X11;
        win.x11 = win_;
        if (!gui_->set_parent(plugin_, &win)) {
            err = "clap.gui set_parent failed";
            close();
            return false;
        }
        show();
        if (!gui_->show(plugin_)) {
            err = "clap.gui show failed";
            close();
            return false;
        }
        return true;
    }

    // Plugin-initiated resize.
    bool request_resize(uint32_t w, uint32_t h) noexcept override {
        if (!open_) return false;
        set_client_size(w, h);
        return true;
    }

    // The plugin closed its GUI: close the window too.
    void plugin_closed() noexcept override { close(); }

private:
    // User resize: lets the plugin adjust the size, snaps the window to it and
    // applies it to the GUI.
    void window_resized(uint32_t w, uint32_t h) noexcept override {
        if (!resizable_ || !created_) return;
        uint32_t aw = w, ah = h;
        gui_->adjust_size(plugin_, &aw, &ah);
        if (aw != w || ah != h) set_client_size(aw, ah);
        gui_->set_size(plugin_, aw, ah);
    }

    // Hides and destroys the plugin GUI and unregisters this editor.
    void teardown() noexcept override {
        if (!created_) return;
        gui_->hide(plugin_);
        gui_->destroy(plugin_);
        created_ = false;
        attach_(nullptr);
    }

    const clap_plugin* plugin_;
    const clap_plugin_gui* gui_;
    std::function<void(ClapEditorHost*)> attach_;
    bool created_ = false;
};

} // namespace

std::unique_ptr<PluginEditor> create_vst3_editor(Steinberg::IPlugView* view, const std::string& title,
                                                 std::string& err) {
    auto e = std::make_unique<Vst3Editor>(view);
    if (!e->open(title, err)) return nullptr; // ~Vst3Editor releases the view
    return e;
}

std::unique_ptr<PluginEditor> create_clap_editor(const clap_plugin* plugin, const clap_plugin_gui* gui,
                                                 const std::string& title, std::function<void(ClapEditorHost*)> attach,
                                                 std::string& err) {
    auto e = std::make_unique<ClapEditor>(plugin, gui, std::move(attach));
    if (!e->open(title, err)) return nullptr;
    return e;
}

// Plugin fds and timers, then the editor windows' own events.
void platform_plugin_idle() {
    run_loop::dispatch();
    pump_events();
}

} // namespace dstudio

#endif // __linux__ && DSTUDIO_HAVE_X11
