// Plugin editor windows on macOS: an NSWindow per editor hosting the
// plugin's NSView. MAIN THREAD ONLY (callers check pthread_main_np).
//
//  - AU:   kAudioUnitProperty_CocoaUI factory (AUCocoaUIBase), else
//          CoreAudioKit's AUGenericView.
//  - VST3: IPlugView attached to an NSView ("NSView" platform type), with
//          IPlugFrame::resizeView and user resizing when canResize().
//  - CLAP: clap.gui embedded in an NSView (CLAP_WINDOW_API_COCOA).
// Closing the window (close button or close()) tears the plugin view down
// first; is_open() then reports false.
#if defined(__APPLE__)

#include "../plugins/plugin_editor.hpp"

#import <AudioToolbox/AudioToolbox.h>
#import <AudioUnit/AUCocoaUIView.h>
#import <Cocoa/Cocoa.h>
#import <CoreAudioKit/CoreAudioKit.h>

#include <clap/clap.h>
#include <pluginterfaces/base/funknownimpl.h>
#include <pluginterfaces/gui/iplugview.h>

#include <cstdlib>

namespace dstudio {
class MacEditor;
}

@interface DStudioEditorDelegate : NSObject <NSWindowDelegate>
@property(nonatomic, assign) dstudio::MacEditor* owner;
@end

namespace dstudio {

class MacEditor : public PluginEditor {
public:
    ~MacEditor() override { close(); }

    bool is_open() const noexcept override { return open_; }

    void close() noexcept override {
        if (!open_ || window_ == nil) return;
        [window_ close]; // -> windowWillClose -> window_closing()
    }

    void bring_to_front() noexcept override {
        if (open_ && window_ != nil) [window_ makeKeyAndOrderFront:nil];
    }

    // Delegate callbacks.
    void window_closing() noexcept {
        if (!open_) return;
        open_ = false;
        teardown();
        window_.delegate = nil;
        delegate_.owner = nullptr;
        window_.contentView = [[NSView alloc] initWithFrame:NSZeroRect];
    }
    virtual void window_resized(NSSize) noexcept {}

protected:
    // Creates and shows the window around `content` (sized to its frame).
    void show(NSView* content, const std::string& title, bool resizable) {
        NSWindowStyleMask style = NSWindowStyleMaskTitled | NSWindowStyleMaskClosable | NSWindowStyleMaskMiniaturizable;
        if (resizable) style |= NSWindowStyleMaskResizable;
        NSRect frame = NSMakeRect(0, 0, std::max<CGFloat>(content.frame.size.width, 100),
                                  std::max<CGFloat>(content.frame.size.height, 50));
        window_ = [[NSWindow alloc] initWithContentRect:frame styleMask:style backing:NSBackingStoreBuffered defer:NO];
        window_.releasedWhenClosed = NO;
        NSString* t = [NSString stringWithUTF8String:title.c_str()];
        window_.title = t != nil ? t : @"Plugin";
        delegate_ = [[DStudioEditorDelegate alloc] init];
        delegate_.owner = this;
        window_.delegate = delegate_;
        window_.contentView = content;
        [window_ center];
        [window_ makeKeyAndOrderFront:nil];
        open_ = true;
    }

    void set_content_size(double w, double h) {
        if (window_ != nil) [window_ setContentSize:NSMakeSize(w, h)];
    }

    virtual void teardown() noexcept = 0;

    NSWindow* window_ = nil;
    DStudioEditorDelegate* delegate_ = nil;
    bool open_ = false;
};

// ---------------------------------------------------------------- AU

class AuEditor final : public MacEditor {
public:
    ~AuEditor() override { close(); }

    bool open(AudioUnit unit, const std::string& title, std::string& err) {
        NSView* view = nil;
        UInt32 size = 0;
        Boolean writable = false;
        if (AudioUnitGetPropertyInfo(unit, kAudioUnitProperty_CocoaUI, kAudioUnitScope_Global, 0, &size, &writable) ==
                noErr &&
            size >= sizeof(AudioUnitCocoaViewInfo)) {
            auto* info = static_cast<AudioUnitCocoaViewInfo*>(std::malloc(size));
            if (AudioUnitGetProperty(unit, kAudioUnitProperty_CocoaUI, kAudioUnitScope_Global, 0, info, &size) ==
                noErr) {
                NSURL* url = (__bridge_transfer NSURL*)info->mCocoaAUViewBundleLocation;
                NSString* cls_name = (__bridge_transfer NSString*)info->mCocoaAUViewClass[0];
                const UInt32 classes = (size - sizeof(CFURLRef)) / sizeof(CFStringRef);
                for (UInt32 i = 1; i < classes; ++i) CFRelease(info->mCocoaAUViewClass[i]);
                NSBundle* bundle = url != nil ? [NSBundle bundleWithURL:url] : nil;
                Class cls = bundle != nil ? [bundle classNamed:cls_name] : nil;
                if (cls != nil && [cls conformsToProtocol:@protocol(AUCocoaUIBase)]) {
                    id<AUCocoaUIBase> factory = [[cls alloc] init];
                    view = [factory uiViewForAudioUnit:unit withSize:NSZeroSize];
                }
            }
            std::free(info);
        }
        if (view == nil) {
            AUGenericView* generic = [[AUGenericView alloc] initWithAudioUnit:unit];
            generic.showsExpertParameters = YES;
            view = generic;
        }
        if (view == nil) {
            err = "the Audio Unit has no view and AUGenericView failed";
            return false;
        }
        view_ = view;
        show(view, title, false);
        return true;
    }

private:
    void teardown() noexcept override {
        [view_ removeFromSuperview];
        view_ = nil;
    }
    NSView* view_ = nil;
};

std::unique_ptr<PluginEditor> create_au_editor(AudioUnit unit, const std::string& title, std::string& err) {
    auto e = std::make_unique<AuEditor>();
    if (!e->open(unit, title, err)) return nullptr;
    return e;
}

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

class Vst3Editor final : public MacEditor {
public:
    explicit Vst3Editor(Steinberg::IPlugView* view) : view_(view) {}

    bool open(const std::string& title, std::string& err) {
        using namespace Steinberg;
        if (view_->isPlatformTypeSupported(kPlatformTypeNSView) != kResultTrue) {
            err = "the VST3 editor does not support NSView";
            return false;
        }
        ViewRect r;
        if (view_->getSize(&r) != kResultOk || r.getWidth() <= 0 || r.getHeight() <= 0) r = ViewRect(0, 0, 400, 300);
        container_ = [[NSView alloc] initWithFrame:NSMakeRect(0, 0, r.getWidth(), r.getHeight())];
        frame_ = new PlugFrame(this);
        view_->setFrame(frame_);
        if (view_->attached((__bridge void*)container_, kPlatformTypeNSView) != kResultOk) {
            view_->setFrame(nullptr);
            err = "IPlugView::attached failed";
            return false;
        }
        attached_ = true;
        resizable_ = view_->canResize() == kResultTrue;
        show(container_, title, resizable_);
        return true;
    }

    bool resize_from_plugin(Steinberg::ViewRect* r) {
        if (!open_ || r == nullptr) return false;
        in_resize_ = true;
        set_content_size(r->getWidth(), r->getHeight());
        in_resize_ = false;
        view_->onSize(r);
        return true;
    }

    void window_resized(NSSize s) noexcept override {
        if (!resizable_ || in_resize_ || !attached_) return;
        Steinberg::ViewRect r(0, 0, static_cast<Steinberg::int32>(s.width), static_cast<Steinberg::int32>(s.height));
        view_->checkSizeConstraint(&r);
        if (r.getWidth() != static_cast<Steinberg::int32>(s.width) ||
            r.getHeight() != static_cast<Steinberg::int32>(s.height)) {
            in_resize_ = true;
            set_content_size(r.getWidth(), r.getHeight());
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
        [container_ removeFromSuperview];
        container_ = nil;
    }

    Steinberg::IPlugView* view_;
    PlugFrame* frame_ = nullptr;
    NSView* container_ = nil;
    bool attached_ = false;
    bool resizable_ = false;
    bool in_resize_ = false;
};

Steinberg::tresult PLUGIN_API PlugFrame::resizeView(Steinberg::IPlugView*, Steinberg::ViewRect* r) {
    return owner_ != nullptr && owner_->resize_from_plugin(r) ? Steinberg::kResultOk : Steinberg::kResultFalse;
}

std::unique_ptr<PluginEditor> create_vst3_editor(Steinberg::IPlugView* view, const std::string& title,
                                                 std::string& err) {
    auto e = std::make_unique<Vst3Editor>(view);
    if (!e->open(title, err)) return nullptr; // ~Vst3Editor releases the view
    return e;
}

// ---------------------------------------------------------------- CLAP

class ClapEditor final : public MacEditor, public ClapEditorHost {
public:
    ClapEditor(const clap_plugin* p, const clap_plugin_gui* g, std::function<void(ClapEditorHost*)> attach)
        : plugin_(p), gui_(g), attach_(std::move(attach)) {}

    bool open(const std::string& title, std::string& err) {
        if (!gui_->is_api_supported(plugin_, CLAP_WINDOW_API_COCOA, false)) {
            err = "the CLAP editor does not support embedded cocoa views";
            return false;
        }
        attach_(this);
        if (!gui_->create(plugin_, CLAP_WINDOW_API_COCOA, false)) {
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
        container_ = [[NSView alloc] initWithFrame:NSMakeRect(0, 0, w, h)];
        clap_window_t win{};
        win.api = CLAP_WINDOW_API_COCOA;
        win.cocoa = (__bridge void*)container_;
        if (!gui_->set_parent(plugin_, &win)) {
            err = "clap.gui set_parent failed";
            return false;
        }
        resizable_ = gui_->can_resize(plugin_);
        show(container_, title, resizable_);
        gui_->show(plugin_);
        return true;
    }

    bool request_resize(uint32_t w, uint32_t h) noexcept override {
        if (!open_) return false;
        in_resize_ = true;
        set_content_size(w, h);
        in_resize_ = false;
        return true;
    }

    void plugin_closed() noexcept override { close(); }

    void window_resized(NSSize s) noexcept override {
        if (!resizable_ || in_resize_ || !created_) return;
        uint32_t w = static_cast<uint32_t>(s.width), h = static_cast<uint32_t>(s.height);
        gui_->adjust_size(plugin_, &w, &h);
        if (w != static_cast<uint32_t>(s.width) || h != static_cast<uint32_t>(s.height)) {
            in_resize_ = true;
            set_content_size(w, h);
            in_resize_ = false;
        }
        gui_->set_size(plugin_, w, h);
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
        [container_ removeFromSuperview];
        container_ = nil;
    }

    const clap_plugin* plugin_;
    const clap_plugin_gui* gui_;
    std::function<void(ClapEditorHost*)> attach_;
    NSView* container_ = nil;
    bool created_ = false;
    bool resizable_ = false;
    bool in_resize_ = false;
};

std::unique_ptr<PluginEditor> create_clap_editor(const clap_plugin* plugin, const clap_plugin_gui* gui,
                                                 const std::string& title, std::function<void(ClapEditorHost*)> attach,
                                                 std::string& err) {
    auto e = std::make_unique<ClapEditor>(plugin, gui, std::move(attach));
    if (!e->open(title, err)) return nullptr;
    return e;
}

} // namespace dstudio

@implementation DStudioEditorDelegate
- (void)windowWillClose:(NSNotification*)note {
    (void)note;
    if (self.owner != nullptr) self.owner->window_closing();
}
- (void)windowDidResize:(NSNotification*)note {
    NSWindow* w = note.object;
    if (self.owner != nullptr) self.owner->window_resized(w.contentView.frame.size);
}
@end

#endif
