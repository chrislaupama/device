// Plugin editor windows on a Linux build without X11 development files
// (libX11 not found at configure time). Opening an editor fails with
// SE_ERR_UNSUPPORTED_FORMAT and this message (plugin_host.cpp); audio
// processing, parameters, state and automation are unaffected, and the plugin
// run loop (fds and timers) still runs.
// macOS: plugin_editor_mac.mm; Windows: plugin_editor_win.cpp; Linux with
// X11: plugin_editor_x11.cpp.
#if !defined(__APPLE__) && !defined(_WIN32) && !defined(SAINE_HAVE_X11)

#include "plugins/plugin_editor.hpp"
#include "run_loop.hpp"

#include <pluginterfaces/gui/iplugview.h>

namespace saine {

namespace {
constexpr const char* kUnsupported =
    "plugin editor windows need X11: this build was configured without libX11";
} // namespace

// No editor windows on this platform: releases the view and fails with a
// "not implemented" message.
std::unique_ptr<PluginEditor> create_vst3_editor(Steinberg::IPlugView* view, const std::string&, std::string& err) {
    if (view != nullptr) view->release(); // the editor owns the view reference
    err = kUnsupported;
    return nullptr;
}

// No editor windows on this platform: fails with a "not implemented" message.
std::unique_ptr<PluginEditor> create_clap_editor(const clap_plugin*, const clap_plugin_gui*, const std::string&,
                                                 std::function<void(ClapEditorHost*)>, std::string& err) {
    err = kUnsupported;
    return nullptr;
}

#if defined(__linux__)
// Plugin fds and timers (no editor windows to pump).
void platform_plugin_idle() { run_loop::dispatch(); }
#endif

} // namespace saine

#endif
