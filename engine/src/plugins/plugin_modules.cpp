#include "plugin_modules.hpp"

#include <clap/clap.h>
#include <pluginterfaces/base/ipluginbase.h>

#include <filesystem>
#include <map>
#include <mutex>

#if defined(__APPLE__)
#include <CoreFoundation/CoreFoundation.h>
#elif defined(_WIN32)
#include "platform/os.hpp"
#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <windows.h>
#else
#include "platform/os.hpp"
#include <dlfcn.h>
#endif

namespace saine {

namespace {

std::mutex g_modules_mutex;
std::map<std::string, std::weak_ptr<ClapModule>> g_clap_modules;
std::map<std::string, std::weak_ptr<Vst3Module>> g_vst3_modules;

#if defined(__APPLE__)
CFBundleRef open_bundle(const std::string& path, std::string& err) {
    CFURLRef url = CFURLCreateFromFileSystemRepresentation(
        kCFAllocatorDefault, reinterpret_cast<const UInt8*>(path.c_str()), static_cast<CFIndex>(path.size()), true);
    if (url == nullptr) {
        err = "invalid bundle path";
        return nullptr;
    }
    CFBundleRef bundle = CFBundleCreate(kCFAllocatorDefault, url);
    CFRelease(url);
    if (bundle == nullptr) {
        err = "not a bundle: " + path;
        return nullptr;
    }
    CFErrorRef cferr = nullptr;
    if (!CFBundleLoadExecutableAndReturnError(bundle, &cferr)) {
        err = "could not load the bundle executable of " + path;
        if (cferr != nullptr) {
            if (CFStringRef d = CFErrorCopyDescription(cferr)) {
                char buf[512];
                if (CFStringGetCString(d, buf, sizeof buf, kCFStringEncodingUTF8)) err += std::string(": ") + buf;
                CFRelease(d);
            }
            CFRelease(cferr);
        }
        CFRelease(bundle);
        return nullptr;
    }
    return bundle;
}
#else // Windows / Linux: plain shared libraries

// Architecture folder of the VST3 bundle layout
// (<name>.vst3/Contents/<arch>-win/<name>.vst3, .../<arch>-linux/<name>.so).
constexpr const char* vst3_arch() {
#if defined(_M_ARM64EC)
    return "arm64ec";
#elif defined(_M_ARM64) || defined(__aarch64__)
#if defined(_WIN32)
    return "arm64";
#else
    return "aarch64";
#endif
#elif defined(_M_X64) || defined(__x86_64__)
    return "x86_64";
#elif defined(_M_IX86) || defined(__i386__)
#if defined(_WIN32)
    return "x86";
#else
    return "i386";
#endif
#else
    return "unknown";
#endif
}

// A .vst3 bundle directory resolves to the binary for this architecture; a
// plain file (legacy single-file .vst3 on Windows) is loaded as is.
std::string vst3_binary_path(const std::string& path) {
    namespace fs = std::filesystem;
    const fs::path p = os::path_from_utf8(path);
    std::error_code ec;
    if (!fs::is_directory(p, ec)) return path;
    const fs::path stem = p.stem();
#if defined(_WIN32)
    return os::path_to_utf8(p / "Contents" / (std::string(vst3_arch()) + "-win") / (stem.native() + L".vst3"));
#else
    return os::path_to_utf8(p / "Contents" / (std::string(vst3_arch()) + "-linux") / (stem.native() + ".so"));
#endif
}

// Loads a shared library (Windows: LoadLibraryExW on the absolute path, so the
// plugin's folder is searched for its DLLs; elsewhere: dlopen). nullptr with
// `err` set on failure.
void* open_library(const std::string& path, std::string& err) {
#if defined(_WIN32)
    // Altered search path: the plugin's own folder is searched for the DLLs
    // it depends on.
    // The flag is undefined for relative paths (e.g. relative SAINE_PLUGIN_PATH
    // entries), so resolve first and refuse rather than pass a relative path.
    std::error_code ec;
    const std::filesystem::path abs = std::filesystem::absolute(os::path_from_utf8(path), ec);
    if (ec) {
        err = "could not load " + path + " (cannot resolve an absolute path: " + ec.message() + ")";
        return nullptr;
    }
    HMODULE h = LoadLibraryExW(abs.c_str(), nullptr, LOAD_WITH_ALTERED_SEARCH_PATH);
    if (h == nullptr) {
        err = "could not load " + path + " (error " + std::to_string(GetLastError()) + ")";
        return nullptr;
    }
    return static_cast<void*>(h);
#else
    void* h = ::dlopen(path.c_str(), RTLD_NOW | RTLD_LOCAL);
    if (h == nullptr) {
        const char* e = ::dlerror();
        err = "could not load " + path + (e != nullptr ? std::string(": ") + e : std::string());
        return nullptr;
    }
    return h;
#endif
}

// Looks up an exported symbol in a library from open_library(); nullptr if absent.
void* find_symbol(void* lib, const char* name) {
#if defined(_WIN32)
    return reinterpret_cast<void*>(GetProcAddress(static_cast<HMODULE>(lib), name));
#else
    return ::dlsym(lib, name);
#endif
}

// find_symbol() cast to a function-pointer type.
template <class Fn> Fn find_function(void* lib, const char* name) {
    return reinterpret_cast<Fn>(find_symbol(lib, name));
}

#endif

} // namespace

// Deinitialises the CLAP entry (macOS: also releases the bundle).
ClapModule::~ClapModule() {
    if (entry != nullptr) entry->deinit();
#if defined(__APPLE__)
    if (bundle != nullptr) CFRelease(static_cast<CFBundleRef>(bundle));
#endif
}

// Releases the factory and calls the module's exit function.
Vst3Module::~Vst3Module() {
    if (factory != nullptr) factory->release();
    if (exit != nullptr) exit();
#if defined(__APPLE__)
    if (bundle != nullptr) CFRelease(static_cast<CFBundleRef>(bundle));
#endif
}

// Loads (or reuses, while still referenced) the CLAP module at `path` and
// initialises its clap_entry. nullptr with `err` set on failure.
std::shared_ptr<ClapModule> load_clap_module(const std::string& path, std::string& err) {
    std::lock_guard lock(g_modules_mutex);
    if (auto it = g_clap_modules.find(path); it != g_clap_modules.end())
        if (auto m = it->second.lock()) return m;
#if defined(__APPLE__)
    CFBundleRef bundle = open_bundle(path, err);
    if (bundle == nullptr) return nullptr;
    auto m = std::make_shared<ClapModule>();
    m->path = path;
    m->bundle = bundle;
    const auto* entry = static_cast<const clap_plugin_entry*>(CFBundleGetDataPointerForName(bundle, CFSTR("clap_entry")));
    if (entry == nullptr) {
        err = "no clap_entry symbol in " + path;
        return nullptr;
    }
    if (!clap_version_is_compatible(entry->clap_version)) {
        err = "incompatible CLAP version in " + path;
        return nullptr;
    }
    if (!entry->init(path.c_str())) {
        err = "clap_entry.init failed for " + path;
        return nullptr;
    }
    m->entry = entry;
    g_clap_modules[path] = m;
    return m;
#else
    // Windows/Linux: the .clap is a single shared library. The library is
    // never unloaded (see the header).
    void* lib = open_library(path, err);
    if (lib == nullptr) return nullptr;
    auto m = std::make_shared<ClapModule>();
    m->path = path;
    m->bundle = lib;
    const auto* entry = static_cast<const clap_plugin_entry*>(find_symbol(lib, "clap_entry"));
    if (entry == nullptr) {
        err = "no clap_entry symbol in " + path;
        return nullptr;
    }
    if (!clap_version_is_compatible(entry->clap_version)) {
        err = "incompatible CLAP version in " + path;
        return nullptr;
    }
    if (!entry->init(path.c_str())) {
        err = "clap_entry.init failed for " + path;
        return nullptr;
    }
    m->entry = entry;
    g_clap_modules[path] = m;
    return m;
#endif
}

// Loads (or reuses, while still referenced) the VST3 module at `path`, runs
// its platform entry function and gets its factory. A bundle directory
// resolves to the binary for this architecture. nullptr with `err` set on
// failure.
std::shared_ptr<Vst3Module> load_vst3_module(const std::string& path, std::string& err) {
    std::lock_guard lock(g_modules_mutex);
    if (auto it = g_vst3_modules.find(path); it != g_vst3_modules.end())
        if (auto m = it->second.lock()) return m;
#if defined(__APPLE__)
    CFBundleRef bundle = open_bundle(path, err);
    if (bundle == nullptr) return nullptr;
    auto m = std::make_shared<Vst3Module>();
    m->path = path;
    m->bundle = bundle;
    using EntryFn = bool (*)(CFBundleRef);
    using FactoryFn = Steinberg::IPluginFactory* (*)();
    auto entry = reinterpret_cast<EntryFn>(CFBundleGetFunctionPointerForName(bundle, CFSTR("bundleEntry")));
    auto exitfn = reinterpret_cast<Vst3Module::ExitFn>(CFBundleGetFunctionPointerForName(bundle, CFSTR("bundleExit")));
    auto get_factory = reinterpret_cast<FactoryFn>(CFBundleGetFunctionPointerForName(bundle, CFSTR("GetPluginFactory")));
    if (entry == nullptr || exitfn == nullptr || get_factory == nullptr) {
        err = "not a VST3 module (bundleEntry/bundleExit/GetPluginFactory missing): " + path;
        return nullptr;
    }
    if (!entry(bundle)) {
        err = "bundleEntry failed for " + path;
        return nullptr;
    }
    m->exit = exitfn;
    m->factory = get_factory();
    if (m->factory == nullptr) {
        err = "GetPluginFactory returned NULL for " + path;
        return nullptr;
    }
    g_vst3_modules[path] = m;
    return m;
#else
    const std::string binary = vst3_binary_path(path);
    void* lib = open_library(binary, err);
    if (lib == nullptr) return nullptr;
    auto m = std::make_shared<Vst3Module>();
    m->path = path;
    m->bundle = lib;
    using FactoryFn = Steinberg::IPluginFactory* (*)();
    auto get_factory = find_function<FactoryFn>(lib, "GetPluginFactory");
    if (get_factory == nullptr) {
        err = "not a VST3 module (GetPluginFactory missing): " + binary;
        return nullptr;
    }
#if defined(_WIN32)
    // InitDll/ExitDll are optional on Windows.
    using InitFn = bool (*)();
    if (auto init = find_function<InitFn>(lib, "InitDll"); init != nullptr && !init()) {
        err = "InitDll failed for " + binary;
        return nullptr;
    }
    m->exit = find_function<Vst3Module::ExitFn>(lib, "ExitDll");
#else
    // Linux: ModuleEntry(handle) / ModuleExit() are mandatory.
    using EntryFn = bool (*)(void*);
    auto entry = find_function<EntryFn>(lib, "ModuleEntry");
    auto exitfn = find_function<Vst3Module::ExitFn>(lib, "ModuleExit");
    if (entry == nullptr || exitfn == nullptr) {
        err = "not a VST3 module (ModuleEntry/ModuleExit missing): " + binary;
        return nullptr;
    }
    if (!entry(lib)) {
        err = "ModuleEntry failed for " + binary;
        return nullptr;
    }
    m->exit = exitfn;
#endif
    m->factory = get_factory();
    if (m->factory == nullptr) {
        err = "GetPluginFactory returned NULL for " + binary;
        return nullptr;
    }
    g_vst3_modules[path] = m;
    return m;
#endif
}

// VST3 class ids as text: the canonical form is the 32 hex digits of the
// four 32-bit words of INLINE_UID(l1, l2, l3, l4) / DECLARE_UID, which is
// what FUID::toString prints on every platform. In memory a TUID holds those
// bytes in order, except where the SDK is COM compatible (Windows): there
// the first 8 bytes are a GUID's Data1 (32-bit) and Data2/Data3 (16-bit)
// in little-endian order. Converting here keeps ids saved in projects
// identical across platforms.
namespace {
// Position (in bytes) of the in-memory TUID byte `memory_index` in the
// canonical text order.
constexpr int text_index(int memory_index) {
#if COM_COMPATIBLE
    constexpr int kComOrder[16] = {3, 2, 1, 0, 5, 4, 7, 6, 8, 9, 10, 11, 12, 13, 14, 15};
    return kComOrder[memory_index];
#else
    return memory_index;
#endif
}
} // namespace

// Formats a VST3 class id as 32 uppercase hex digits in the canonical
// (platform-independent) order.
std::string vst3_uid_to_string(const char uid[16]) {
    static const char* hex = "0123456789ABCDEF";
    std::string s(32, '0');
    for (int i = 0; i < 16; ++i) {
        const auto b = static_cast<uint8_t>(uid[i]);
        const auto at = static_cast<std::size_t>(2 * text_index(i));
        s[at] = hex[b >> 4];
        s[at + 1] = hex[b & 15];
    }
    return s;
}

// Parses 32 hex digits in the canonical order into a TUID; false if malformed.
bool vst3_uid_from_string(const std::string& s, char uid[16]) {
    if (s.size() != 32) return false;
    const auto nib = [](char c) -> int {
        if (c >= '0' && c <= '9') return c - '0';
        if (c >= 'a' && c <= 'f') return c - 'a' + 10;
        if (c >= 'A' && c <= 'F') return c - 'A' + 10;
        return -1;
    };
    for (int i = 0; i < 16; ++i) {
        const auto at = static_cast<std::size_t>(2 * text_index(i));
        const int hi = nib(s[at]), lo = nib(s[at + 1]);
        if (hi < 0 || lo < 0) return false;
        uid[i] = static_cast<char>((hi << 4) | lo);
    }
    return true;
}

} // namespace saine
