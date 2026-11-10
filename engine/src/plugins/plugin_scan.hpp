// In-process inspection of one plugin (AU component, VST3 or CLAP bundle).
// Used only by the out-of-process scanner (scanner/scanner_main.cpp): it
// loads and instantiates plugin code, which may crash or hang, so the
// engine/app never calls these directly.
#pragma once

#include <cstdint>
#include <string>
#include <vector>

namespace dstudio {

struct ScannedPlugin {
    std::string format; // "audio_unit" | "vst3" | "clap" (dstudio_core PluginFormat serde names)
    std::string id;     // AU "type subtype manufacturer" / VST3 class UID hex / CLAP id
    std::string path;   // bundle path ("" for AU)
    std::string name;
    std::string vendor;
    std::string version;
    bool instrument = false;
    bool has_editor = false;
    uint32_t audio_inputs = 0;  // main input channels
    uint32_t audio_outputs = 0; // main output channels
};

struct AuComponent {
    std::string id;
    std::string name;
    std::string vendor;
    uint32_t version = 0;
};

// Enumerates registered Audio Units of type aufx, aumf and aumu (does not
// instantiate them). Empty on other platforms.
std::vector<AuComponent> enumerate_audio_units();

bool scan_audio_unit(const std::string& id, std::vector<ScannedPlugin>& out, std::string& err);
bool scan_vst3_bundle(const std::string& path, std::vector<ScannedPlugin>& out, std::string& err);
bool scan_clap_bundle(const std::string& path, std::vector<ScannedPlugin>& out, std::string& err);

} // namespace dstudio
