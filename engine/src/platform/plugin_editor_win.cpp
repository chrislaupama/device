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

constexpr wchar_t kWindowClass[] = L"SaineStudioPluginEditor";

class WinEditor : public PluginEditor {
public:
    ~WinEditor() override { close(); }

    bool is_open() const noexcept override { return open_; }

    void close() noexcept override {
        if (hwnd_ != nullptr) DestroyWindow(hwnd_); // -> WM_DESTROY -> window_closing()
    }

    void bring_to_front() noexcept override {
        if (hwnd_ == nullptr) return;
        if (IsIconic(hwnd_)) ShowWindow(hwnd_, SW_RESTORE);
        SetForegroundWindow(hwnd_);
    }

    void window_closing() noexcept {
        if (!open_) return;
        open_ = false;
        teardown(); // the plugin's child window is still alive here
        SetWindowLongPtrW(hwnd_, GWLP_USERDATA, 0);
        hwnd_ = nullptr;
    }
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

    void show() noexcept {
        ShowWindow(hwnd_, SW_SHOW);
        SetForegroundWindow(hwnd_);
    }

    void set_client_size(uint32_t w, uint32_t h) noexcept {
        if (hwnd_ == nullptr) return;
        RECT r{0, 0, static_cast<LONG>(w), static_cast<LONG>(h)};
        AdjustWindowRectEx(&r, style_, FALSE, 0);
        SetWindowPos(hwnd_, nullptr, 0, 0, r.right - r.left, r.bottom - r.top,
                     SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE);
    }

    virtual void teardown() noexcept = 0;

    HWND hwnd_ = nullptr;
    DWORD style_ = 0;
    bool open_ = false;

private:
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

class PlugFrame final : public Steinberg::U::Implements<Steinberg::U::Directly<Steinberg::IPlugFrame>> {
public:
    explicit PlugFrame(Vst3Editor* owner) : owner_(owner) {}
    Steinberg::tresult PLUGIN_API resizeView(Steinberg::IPlugView* view, Steinberg::ViewRect* r) override;
    void detach() { owner_ = nullptr; }

private:
    Vst3Editor* owner_;
};

class Vst3Editor final : public WinEditor {
public:
    explicit Vst3Editor(Steinberg::IPlugView* view) : view_(view) {}

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

    bool resize_from_plugin(Steinberg::ViewRect* r) {
        if (!open_ || r == nullptr) return false;
        in_resize_ = true;
        set_client_size(static_cast<uint32_t>(r->getWidth()), static_cast<uint32_t>(r->getHeight()));
        in_resize_ = false;
        view_->onSize(r);
        return true;
    }

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

    ~Vst3Editor() override {
        close();
        teardown();
        if (view_ != nullptr) view_->release();
        view_ = nullptr;
    }

private:
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

Steinberg::tresult PLUGIN_API PlugFrame::resizeView(Steinberg::IPlugView*, Steinberg::ViewRect* r) {
    return owner_ != nullptr && owner_->resize_from_plugin(r) ? Steinberg::kResultOk : Steinberg::kResultFalse;
}

// ---------------------------------------------------------------- CLAP

class ClapEditor final : public WinEditor, public ClapEditorHost {
public:
    ClapEditor(const clap_plugin* p, const clap_plugin_gui* g, std::function<void(ClapEditorHost*)> attach)
        : plugin_(p), gui_(g), attach_(std::move(attach)) {}

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

    bool request_resize(uint32_t w, uint32_t h) noexcept override {
        if (!open_) return false;
        in_resize_ = true;
        set_client_size(w, h);
        in_resize_ = false;
        return true;
    }

    void plugin_closed() noexcept override { close(); }

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

    ~ClapEditor() override {
        close();
        teardown();
    }

private:
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

} // namespace saine

#endif // _WIN32
