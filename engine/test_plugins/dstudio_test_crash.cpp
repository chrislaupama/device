// DStudioTestCrash.clap: a deliberately broken CLAP bundle whose clap_entry
// init crashes. The scanner must blocklist it while the scan completes.
#include <clap/clap.h>

#include <csignal>

namespace {
// clap_entry.init: crashes on purpose (a real access violation on Windows,
// SIGSEGV elsewhere).
bool entry_init(const char*) {
#if defined(_WIN32)
    // raise(SIGSEGV) on Windows is a clean exit(3), not a crash: fault for real.
    *static_cast<volatile int*>(nullptr) = 1;
#else
    std::raise(SIGSEGV);
#endif
    return false;
}
// Never reached.
void entry_deinit() {}
// No factory.
const void* entry_get_factory(const char*) { return nullptr; }
} // namespace

extern "C" CLAP_EXPORT const clap_plugin_entry_t clap_entry = {CLAP_VERSION_INIT, entry_init, entry_deinit,
                                                                entry_get_factory};
