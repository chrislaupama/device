// Loading VST3 / CLAP plugin bundles (shared by hosting and the scanner).
//
// Modules are cached by bundle path and shared by every instance created
// from them; a module's exit/deinit runs when the last user releases it.
// The executable is never unloaded (unloading plugin code is a common source
// of crashes; the address space cost is acceptable). Main thread only.
#pragma once

#include <memory>
#include <string>

struct clap_plugin_entry;
namespace Steinberg {
class IPluginFactory;
}

namespace saine {

struct ClapModule {
    std::string path;
    const clap_plugin_entry* entry = nullptr;
    void* bundle = nullptr; // CFBundleRef (macOS) / HMODULE / dlopen handle (never unloaded)
    ~ClapModule();
};

struct Vst3Module {
    std::string path;
    Steinberg::IPluginFactory* factory = nullptr; // owned reference
    void* bundle = nullptr;                       // CFBundleRef (macOS) / HMODULE / dlopen handle
    using ExitFn = bool (*)();
    ExitFn exit = nullptr;
    ~Vst3Module();
};

std::shared_ptr<ClapModule> load_clap_module(const std::string& path, std::string& err);
std::shared_ptr<Vst3Module> load_vst3_module(const std::string& path, std::string& err);

// 32 upper-case hex chars of the 16 TUID bytes, in memory order.
std::string vst3_uid_to_string(const char uid[16]);
bool vst3_uid_from_string(const std::string& s, char uid[16]);

} // namespace saine
