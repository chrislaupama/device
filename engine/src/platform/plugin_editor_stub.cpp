// Plugin editor windows on platforms without an implementation yet (Linux:
// X11 embedding, CLAP_WINDOW_API_X11 / kPlatformTypeX11EmbedWindowID, needs
// an X event loop and the plugins' fd/timer callbacks). Opening an editor
// fails with SE_ERR_UNSUPPORTED_FORMAT and this message (plugin_host.cpp);
// audio processing, parameters, state and automation are unaffected.
// macOS: plugin_editor_mac.mm; Windows: plugin_editor_win.cpp.
#if !defined(__APPLE__) && !defined(_WIN32)

#include "plugins/plugin_editor.hpp"

#include <pluginterfaces/gui/iplugview.h>

namespace saine {

namespace {
constexpr const char* kUnsupported =
    "plugin editor windows are not implemented on Linux yet (X11 embedding is planned)";
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

} // namespace saine

#endif
