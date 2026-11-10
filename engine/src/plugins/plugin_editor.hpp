// Native plugin editor windows (src/platform/plugin_editor_mac.mm, _win.cpp, _x11.cpp).
// Every function here is MAIN THREAD ONLY.
#pragma once

#include "plugin_instance.hpp"

#include <functional>
#include <memory>
#include <string>

#if defined(__APPLE__)
#include <AudioToolbox/AudioToolbox.h>
#endif

struct clap_plugin;
struct clap_plugin_gui;
namespace Steinberg {
class IPlugView;
}

namespace saine {

#if defined(__APPLE__)
// Hosts the AU's Cocoa view (kAudioUnitProperty_CocoaUI), else AUGenericView.
std::unique_ptr<PluginEditor> create_au_editor(AudioUnit unit, const std::string& title, std::string& err);
#endif

// Hosts an IPlugView (already created by the controller; the editor takes
// over the reference). Resizable when the view says so.
std::unique_ptr<PluginEditor> create_vst3_editor(Steinberg::IPlugView* view, const std::string& title,
                                                 std::string& err);

// CLAP gui bridge: the instance forwards host gui callbacks here.
class ClapEditorHost {
public:
    virtual ~ClapEditorHost() = default;
    virtual bool request_resize(uint32_t width, uint32_t height) noexcept = 0;
    virtual void plugin_closed() noexcept = 0; // plugin asked to close (clap_host_gui.closed)
};

// Creates the gui (embedded cocoa) and shows it. `host_out` receives the
// bridge the instance must forward clap_host_gui callbacks to while the
// editor lives (cleared via `on_destroy`).
std::unique_ptr<PluginEditor> create_clap_editor(const clap_plugin* plugin, const clap_plugin_gui* gui,
                                                 const std::string& title, std::function<void(ClapEditorHost*)> attach,
                                                 std::string& err);

#if defined(__linux__)
// Main-thread plugin housekeeping that Linux needs from the host: runs the
// plugin run loop (fds and timers, platform/run_loop.hpp) and the editor
// windows' X events. Called from PluginHost::idle (se_plugin_idle).
void platform_plugin_idle();
#else
// macOS and Windows plugins use the system event loop.
inline void platform_plugin_idle() {}
#endif

} // namespace saine
