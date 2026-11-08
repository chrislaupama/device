// SaineTestCrash.clap: a deliberately broken CLAP bundle whose clap_entry
// init crashes. The scanner must blocklist it while the scan completes.
#include <clap/clap.h>

#include <csignal>

namespace {
bool entry_init(const char*) {
    std::raise(SIGSEGV);
    return false;
}
void entry_deinit() {}
const void* entry_get_factory(const char*) { return nullptr; }
} // namespace

extern "C" CLAP_EXPORT const clap_plugin_entry_t clap_entry = {CLAP_VERSION_INIT, entry_init, entry_deinit,
                                                                entry_get_factory};
