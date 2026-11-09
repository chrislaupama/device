// Plugin editor windows on Windows: one top-level window per editor whose
// client area hosts the plugin's child HWND. MAIN THREAD ONLY (the thread
// that runs the app's message loop; on_platform_main_thread()).
//
//  - VST3: IPlugView attached with kPlatformTypeHWND, IPlugFrame::resizeView,
//          user resizing when canResize().
//  - CLAP: clap.gui embedded (CLAP_WINDOW_API_WIN32), request_resize,
//          can_resize/adjust_size.
// The windows need no message loop of their own: the app's (winit) loop
// dispatches messages for every window of its thread. Closing the window
// (close box or close()) tears the plugin view down first (WM_DESTROY);
// is_open() then reports false.
//
// Not handled yet: per-monitor DPI scaling (IPlugViewContentScaleSupport /
// clap.gui set_scale); the window is created at the plugin's pixel size.
#if defined(_WIN32)

#include "os.hpp"
#include "plugins/plugin_editor.hpp"

#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <windows.h>

#include <clap/clap.h>
#include <pluginterfaces/base/funknownimpl.h>
#include <pluginterfaces/gui/iplugview.h>

#include <algorithm>

namespace saine {
namespace {

// Window class shared by every editor window.
constexpr wchar_t kWindowClass[] = L"SaineStudioPluginEditor";

// Base of the Win32 editor windows: owns the top-level window and routes
// its close and resize messages to the plugin-specific subclass.
class WinEditor : public PluginEditor {
public:
    // Closes the window (tearing the plugin view down) if it is still open.
    ~WinEditor() override { close(); }

    // False once the window has been closed by the user or by close().
    bool is_open() const noexcept override { return open_; }

    // Destroys the window; WM_DESTROY then tears the plugin view down.
    void close() noexcept override {
        if (hwnd_ != nullptr) DestroyWindow(hwnd_); // -> WM_DESTROY -> window_closing()
    }

    // Restores the window if minimised and brings it to the foreground.
    void bring_to_front() noexcept override {
        if (hwnd_ == nullptr) return;
        if (IsIconic(hwnd_)) ShowWindow(hwnd_, SW_RESTORE);
        SetForegroundWindow(hwnd_);
    }

    // WM_DESTROY: tears the plugin view down while its child window still exists
    // and detaches this object from the window. Runs once.
    void window_closing() noexcept {
        if (!open_) return;
        open_ = false;
        teardown(); // the plugin's child window is still alive here
        SetWindowLongPtrW(hwnd_, GWLP_USERDATA, 0);
        hwnd_ = nullptr;
    }
    // The user resized the window to a client area of w x h (no-op by default).
    virtual void window_resized(uint32_t, uint32_t) noexcept {}

protected:
    // Creates the (hidden) window with a client area of w x h.
    bool create_window(const std::string& title, uint32_t w, uint32_t h, bool resizable, std::string& err) {
        static const ATOM cls = [] {
            WNDCLASSEXW wc{};
            wc.cbSize = sizeof wc;
            wc.style = CS_HREDRAW | CS_VREDRAW;
            wc.lpfnWndProc = &WinEditor::wndproc;
            wc.hInstance = GetModuleHandleW(nullptr);
            wc.hCursor = LoadCursorW(nullptr, MAKEINTRESOURCEW(32512)); // IDC_ARROW
            wc.hbrBackground = reinterpret_cast<HBRUSH>(static_cast<INT_PTR>(COLOR_WINDOW + 1));
            wc.lpszClassName = kWindowClass;
            return RegisterClassExW(&wc);
        }();
        if (cls == 0) {
            err = "RegisterClassExW failed (error " + std::to_string(GetLastError()) + ")";
            return false;
        }
        style_ = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX | WS_CLIPCHILDREN;
        if (resizable) style_ |= WS_THICKFRAME | WS_MAXIMIZEBOX;
        RECT r{0, 0, static_cast<LONG>(std::max<uint32_t>(w, 100)), static_cast<LONG>(std::max<uint32_t>(h, 50))};
        AdjustWindowRectEx(&r, style_, FALSE, 0);
        const std::wstring wtitle = os::widen(title.empty() ? std::string("Plugin") : title);
        hwnd_ = CreateWindowExW(0, kWindowClass, wtitle.c_str(), style_, CW_USEDEFAULT, CW_USEDEFAULT, r.right - r.left,
                                r.bottom - r.top, nullptr, nullptr, GetModuleHandleW(nullptr), this);
        if (hwnd_ == nullptr) {
            err = "CreateWindowExW failed (error " + std::to_string(GetLastError()) + ")";
            return false;
        }
        open_ = true;
        return true;
    }

    // Shows the window and brings it to the foreground.
    void show() noexcept {
        ShowWindow(hwnd_, SW_SHOW);
        SetForegroundWindow(hwnd_);
    }

    // Resizes the window so its client area is w x h.
    void set_client_size(uint32_t w, uint32_t h) noexcept {
        if (hwnd_ == nullptr) return;
        RECT r{0, 0, static_cast<LONG>(w), static_cast<LONG>(h)};
        AdjustWindowRectEx(&r, style_, FALSE, 0);
        SetWindowPos(hwnd_, nullptr, 0, 0, r.right - r.left, r.bottom - r.top,
                     SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE);
    }

    // Detaches and destroys the plugin's view (called while the window exists).
    virtual void teardown() noexcept = 0;

    HWND hwnd_ = nullptr;
    DWORD style_ = 0;
    bool open_ = false;

private:
    // Window procedure: binds the window to its WinEditor on WM_NCCREATE and
    // forwards close, destroy and resize to it.
    static LRESULT CALLBACK wndproc(HWND hwnd, UINT msg, WPARAM wp, LPARAM lp) {
        if (msg == WM_NCCREATE) {
            const auto* cs = reinterpret_cast<const CREATESTRUCTW*>(lp);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, reinterpret_cast<LONG_PTR>(cs->lpCreateParams));
        }
        auto* self = reinterpret_cast<WinEditor*>(GetWindowLongPtrW(hwnd, GWLP_USERDATA));
        switch (msg) {
        case WM_CLOSE: DestroyWindow(hwnd); return 0;
        case WM_DESTROY:
            if (self != nullptr) self->window_closing();
            return 0;
        case WM_SIZE:
            if (self != nullptr && self->open_ && wp != SIZE_MINIMIZED)
                self->window_resized(LOWORD(lp), HIWORD(lp));
            return 0;
        default: break;
        }
        return DefWindowProcW(hwnd, msg, wp, lp);
    }
};

// ---------------------------------------------------------------- VST3

class Vst3Editor;

// IPlugFrame given to a VST3 view so it can ask its window to resize.
class PlugFrame final : public Steinberg::U::Implements<Steinberg::U::Directly<Steinberg::IPlugFrame>> {
public:
    // Frame for `owner`'s view.
    explicit PlugFrame(Vst3Editor* owner) : owner_(owner) {}
    Steinberg::tresult PLUGIN_API resizeView(Steinberg::IPlugView* view, Steinberg::ViewRect* r) override;
    // Stops forwarding resize requests (the editor is going away).
    void detach() { owner_ = nullptr; }

private:
    Vst3Editor* owner_;
};

// A VST3 IPlugView embedded in a top-level window (kPlatformTypeHWND).
class Vst3Editor final : public WinEditor {
public:
    // Takes ownership of one reference to `view`.
    explicit Vst3Editor(Steinberg::IPlugView* view) : view_(view) {}

    // Creates the window at the view's size (400 x 300 if it reports none),
    // attaches the view and shows it. False with `err` set on failure.
    bool open(const std::string& title, std::string& err) {
        using namespace Steinberg;
        if (view_->isPlatformTypeSupported(kPlatformTypeHWND) != kResultTrue) {
            err = "the VST3 editor does not support HWND embedding";
            return false;
        }
        ViewRect r;
        if (view_->getSize(&r) != kResultOk || r.getWidth() <= 0 || r.getHeight() <= 0) r = ViewRect(0, 0, 400, 300);
        resizable_ = view_->canResize() == kResultTrue;
        if (!create_window(title, static_cast<uint32_t>(r.getWidth()), static_cast<uint32_t>(r.getHeight()),
                           resizable_, err))
            return false;
        frame_ = new PlugFrame(this);
        view_->setFrame(frame_);
        if (view_->attached(hwnd_, kPlatformTypeHWND) != kResultOk) {
            view_->setFrame(nullptr);
            err = "IPlugView::attached failed";
            close();
            return false;
        }
        attached_ = true;
        show();
        return true;
    }

    // IPlugFrame::resizeView: resizes the window to the plugin's request and
    // confirms the new size to the view.
    bool resize_from_plugin(Steinberg::ViewRect* r) {
        if (!open_ || r == nullptr) return false;
        in_resize_ = true;
        set_client_size(static_cast<uint32_t>(r->getWidth()), static_cast<uint32_t>(r->getHeight()));
        in_resize_ = false;
        view_->onSize(r);
        return true;
    }

    // User resize: snaps the size to the view's constraints and passes it to the view.
    void window_resized(uint32_t w, uint32_t h) noexcept override {
        if (!resizable_ || in_resize_ || !attached_) return;
        Steinberg::ViewRect r(0, 0, static_cast<Steinberg::int32>(w), static_cast<Steinberg::int32>(h));
        view_->checkSizeConstraint(&r);
        if (r.getWidth() != static_cast<Steinberg::int32>(w) || r.getHeight() != static_cast<Steinberg::int32>(h)) {
            in_resize_ = true;
            set_client_size(static_cast<uint32_t>(r.getWidth()), static_cast<uint32_t>(r.getHeight()));
            in_resize_ = false;
        }
        view_->onSize(&r);
    }

    // Closes the window, detaches the view and releases it.
    ~Vst3Editor() override {
        close();
        teardown();
        if (view_ != nullptr) view_->release();
        view_ = nullptr;
    }

private:
    // Removes the view from the window and releases the frame.
    void teardown() noexcept override {
        if (attached_) {
            view_->removed();
            view_->setFrame(nullptr);
            attached_ = false;
        }
        if (frame_ != nullptr) {
            frame_->detach();
            frame_->release();
            frame_ = nullptr;
        }
    }

    Steinberg::IPlugView* view_;
    PlugFrame* frame_ = nullptr;
    bool attached_ = false;
    bool resizable_ = false;
    bool in_resize_ = false;
};

// Forwards the plugin's resize request to the editor, if it still exists.
Steinberg::tresult PLUGIN_API PlugFrame::resizeView(Steinberg::IPlugView*, Steinberg::ViewRect* r) {
    return owner_ != nullptr && owner_->resize_from_plugin(r) ? Steinberg::kResultOk : Steinberg::kResultFalse;
}

// ---------------------------------------------------------------- CLAP

// A CLAP clap.gui embedded in a top-level window (CLAP_WINDOW_API_WIN32).
class ClapEditor final : public WinEditor, public ClapEditorHost {
public:
    // `attach` registers this editor with the plugin instance (nullptr unregisters)
    // so the plugin's resize and closed requests reach it.
    ClapEditor(const clap_plugin* p, const clap_plugin_gui* g, std::function<void(ClapEditorHost*)> attach)
        : plugin_(p), gui_(g), attach_(std::move(attach)) {}

    // Creates the plugin GUI and the window at the GUI's size (400 x 300 if it
    // reports none), parents the GUI in it and shows it. False with `err` set on
    // failure.
    bool open(const std::string& title, std::string& err) {
        if (!gui_->is_api_supported(plugin_, CLAP_WINDOW_API_WIN32, false)) {
            err = "the CLAP editor does not support embedded Win32 windows";
            return false;
        }
        attach_(this);
        if (!gui_->create(plugin_, CLAP_WINDOW_API_WIN32, false)) {
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
        resizable_ = gui_->can_resize(plugin_);
        if (!create_window(title, w, h, resizable_, err)) return false;
        clap_window_t win{};
        win.api = CLAP_WINDOW_API_WIN32;
        win.win32 = hwnd_;
        if (!gui_->set_parent(plugin_, &win)) {
            err = "clap.gui set_parent failed";
            close();
            return false;
        }
        show();
        gui_->show(plugin_);
        return true;
    }

    // Plugin-initiated resize: sets the window's client area to w x h.
    bool request_resize(uint32_t w, uint32_t h) noexcept override {
        if (!open_) return false;
        in_resize_ = true;
        set_client_size(w, h);
        in_resize_ = false;
        return true;
    }

    // The plugin closed its GUI: close the window too.
    void plugin_closed() noexcept override { close(); }

    // User resize: lets the plugin adjust the size, snaps the window to it and
    // applies it to the GUI.
    void window_resized(uint32_t w, uint32_t h) noexcept override {
        if (!resizable_ || in_resize_ || !created_) return;
        uint32_t aw = w, ah = h;
        gui_->adjust_size(plugin_, &aw, &ah);
        if (aw != w || ah != h) {
            in_resize_ = true;
            set_client_size(aw, ah);
            in_resize_ = false;
        }
        gui_->set_size(plugin_, aw, ah);
    }

    // Closes the window and destroys the plugin GUI.
    ~ClapEditor() override {
        close();
        teardown();
    }

private:
    // Hides and destroys the plugin GUI and unregisters this editor.
    void teardown() noexcept override {
        if (created_) {
            gui_->hide(plugin_);
            gui_->destroy(plugin_);
            created_ = false;
            attach_(nullptr);
        }
    }

    const clap_plugin* plugin_;
    const clap_plugin_gui* gui_;
    std::function<void(ClapEditorHost*)> attach_;
    bool created_ = false;
    bool resizable_ = false;
    bool in_resize_ = false;
};

} // namespace

// Opens a VST3 editor window; nullptr with `err` set on failure (the view is
// released either way on failure).
std::unique_ptr<PluginEditor> create_vst3_editor(Steinberg::IPlugView* view, const std::string& title,
                                                 std::string& err) {
    auto e = std::make_unique<Vst3Editor>(view);
    if (!e->open(title, err)) return nullptr; // ~Vst3Editor releases the view
    return e;
}

// Opens a CLAP editor window; nullptr with `err` set on failure.
std::unique_ptr<PluginEditor> create_clap_editor(const clap_plugin* plugin, const clap_plugin_gui* gui,
                                                 const std::string& title, std::function<void(ClapEditorHost*)> attach,
                                                 std::string& err) {
    auto e = std::make_unique<ClapEditor>(plugin, gui, std::move(attach));
    if (!e->open(title, err)) return nullptr;
    return e;
}

} // namespace saine

#endif // _WIN32
