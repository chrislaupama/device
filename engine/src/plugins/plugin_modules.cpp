#include "plugin_modules.hpp"

#include <clap/clap.h>
#include <pluginterfaces/base/ipluginbase.h>

#include <map>
#include <mutex>

#if defined(__APPLE__)
#include <CoreFoundation/CoreFoundation.h>
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
#endif

} // namespace

ClapModule::~ClapModule() {
    if (entry != nullptr) entry->deinit();
#if defined(__APPLE__)
    if (bundle != nullptr) CFRelease(static_cast<CFBundleRef>(bundle));
#endif
}

Vst3Module::~Vst3Module() {
    if (factory != nullptr) factory->release();
    if (exit != nullptr) exit();
#if defined(__APPLE__)
    if (bundle != nullptr) CFRelease(static_cast<CFBundleRef>(bundle));
#endif
}

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
    err = "CLAP loading is only implemented on macOS";
    return nullptr;
#endif
}

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
    err = "VST3 loading is only implemented on macOS";
    return nullptr;
#endif
}

std::string vst3_uid_to_string(const char uid[16]) {
    static const char* hex = "0123456789ABCDEF";
    std::string s(32, '0');
    for (int i = 0; i < 16; ++i) {
        const auto b = static_cast<uint8_t>(uid[i]);
        s[static_cast<std::size_t>(2 * i)] = hex[b >> 4];
        s[static_cast<std::size_t>(2 * i + 1)] = hex[b & 15];
    }
    return s;
}

bool vst3_uid_from_string(const std::string& s, char uid[16]) {
    if (s.size() != 32) return false;
    const auto nib = [](char c) -> int {
        if (c >= '0' && c <= '9') return c - '0';
        if (c >= 'a' && c <= 'f') return c - 'a' + 10;
        if (c >= 'A' && c <= 'F') return c - 'A' + 10;
        return -1;
    };
    for (int i = 0; i < 16; ++i) {
        const int hi = nib(s[static_cast<std::size_t>(2 * i)]), lo = nib(s[static_cast<std::size_t>(2 * i + 1)]);
        if (hi < 0 || lo < 0) return false;
        uid[i] = static_cast<char>((hi << 4) | lo);
    }
    return true;
}

} // namespace saine
