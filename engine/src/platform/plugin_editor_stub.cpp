// Plugin editor windows on platforms without an implementation yet
// (Windows: HWND child window; Linux: X11 embedding). Opening an editor fails
// with SE_ERR_UNSUPPORTED_FORMAT and this message (plugin_host.cpp); audio
// processing, parameters, state and automation are unaffected.
// macOS: plugin_editor_mac.mm.
#if !defined(__APPLE__)

#include "plugins/plugin_editor.hpp"

#include <pluginterfaces/gui/iplugview.h>

namespace saine {

namespace {
constexpr const char* kUnsupported =
#if defined(_WIN32)
    "plugin editor windows are not implemented on Windows yet (HWND hosting is planned)";
#else
    "plugin editor windows are not implemented on Linux yet (X11 embedding is planned)";
#endif
} // namespace

std::unique_ptr<PluginEditor> create_vst3_editor(Steinberg::IPlugView* view, const std::string&, std::string& err) {
    if (view != nullptr) view->release(); // the editor owns the view reference
    err = kUnsupported;
    return nullptr;
}

std::unique_ptr<PluginEditor> create_clap_editor(const clap_plugin*, const clap_plugin_gui*, const std::string&,
                                                 std::function<void(ClapEditorHost*)>, std::string& err) {
    err = kUnsupported;
    return nullptr;
}

} // namespace saine

#endif
