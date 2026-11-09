// Out-of-process scanner: finds Apple's AUs and the test plugins (via
// SAINE_PLUGIN_PATH), blocklists SaineTestCrash.clap (crashes in its
// entry) while the scan completes, serves unchanged entries from the cache,
// re-scans a bundle whose modification time changed, and --rescan clears
// the blocklist.
#include "test_harness.hpp"
#include "test_util.hpp"

#include "platform/os.hpp"

#include <chrono>
#include <cstdio>
#include <filesystem>
#include <fstream>
#include <sstream>
#include <string>

using namespace testutil;
namespace os = saine::os;

namespace {

std::string slurp(const std::string& path) {
    std::ifstream f(os::path_from_utf8(path), std::ios::binary);
    std::ostringstream ss;
    ss << f.rdbuf();
    return ss.str();
}

// A private copy of the test plugin folder, removed by the destructor. CTest
// runs test cases in parallel and they share SAINE_TEST_PLUGIN_DIR, so a test
// that changes a bundle's modification time works on its own copy.
struct PluginDirCopy {
    PluginDirCopy() {
        static int counter = 0;
        const auto dir = std::filesystem::temp_directory_path() /
                         ("saine_test_" + std::to_string(os::process_id()) + "_" + std::to_string(counter++) + "_plugins");
        std::error_code ec;
        std::filesystem::copy(os::path_from_utf8(SAINE_TEST_PLUGIN_DIR), dir,
                              std::filesystem::copy_options::recursive | std::filesystem::copy_options::copy_symlinks,
                              ec);
        if (!ec) path = os::path_to_utf8(dir);
    }
    ~PluginDirCopy() {
        std::error_code ec;
        if (!path.empty()) std::filesystem::remove_all(os::path_from_utf8(path), ec);
    }
    PluginDirCopy(const PluginDirCopy&) = delete;
    PluginDirCopy& operator=(const PluginDirCopy&) = delete;
    std::string path;
};

// Runs the scanner coordinator (stdout captured) with SAINE_PLUGIN_PATH set
// to `plugin_dir`.
std::string run_scan(const std::string& cache, bool rescan, int* exit_code,
                     const std::string& plugin_dir = SAINE_TEST_PLUGIN_DIR) {
    os::set_env("SAINE_PLUGIN_PATH", plugin_dir.c_str());
    os::set_env("SAINE_PLUGIN_SCAN_TIMEOUT_MS", "8000");
    TempFile out("scan.out"), err("scan.err");
    std::vector<std::string> args = {"--scan", "--cache", cache};
    if (rescan) args.push_back("--rescan");
    os::ChildProcess p;
    std::string spawn_err;
    if (exit_code != nullptr) *exit_code = -1;
    if (!p.spawn(SAINE_PLUGIN_SCANNER_PATH, args, out.path, err.path, spawn_err)) {
        std::fprintf(stderr, "  spawn failed: %s\n", spawn_err.c_str());
        return {};
    }
    os::ExitStatus st;
    if (!p.wait_for(std::chrono::minutes(2), st)) {
        p.kill();
        return {};
    }
    if (exit_code != nullptr) *exit_code = st.crashed ? -1 : st.code;
    return slurp(out.path);
}

bool has(const std::string& hay, const std::string& needle) { return hay.find(needle) != std::string::npos; }

long scanned(const std::string& report) {
    const auto at = report.find("\"scanned\":");
    return at == std::string::npos ? -1 : std::atol(report.c_str() + at + 10);
}

} // namespace

TEST_CASE(plugin_scanner_finds_plugins_and_blocklists_crashes) {
    const PluginDirCopy plugins;
    REQUIRE(!plugins.path.empty());
    TempFile cache("plugin_cache.json");
    int rc = -1;
    const std::string r1 = run_scan(cache.path, false, &rc, plugins.path);
    REQUIRE(rc == 0);
#if defined(__APPLE__)
    // Apple AUs.
    CHECK(has(r1, "\"id\":\"aufx lpas appl\""));
    CHECK(has(r1, "\"id\":\"aufx dely appl\""));
    CHECK(has(r1, "\"id\":\"aumu dls  appl\""));
#else
    std::fprintf(stderr, "  (Audio Unit checks skipped: Audio Units are macOS-only)\n");
#endif
    // Test plugins, with their kinds.
    CHECK(has(r1, "\"id\":\"studio.saine.test.gain\""));
    CHECK(has(r1, "\"id\":\"studio.saine.test.sine\",\"path\""));
    CHECK(has(r1, "\"id\":\"5A17E5A16E0000018A1E7E576A170001\""));
    CHECK(has(r1, "\"name\":\"Saine Test Sine\",\"vendor\":\"Saine\",\"version\":\"1.0.0\",\"kind\":\"instrument\""));
    // The crashing bundle is reported and blocklisted; the scan completed.
    CHECK(has(r1, "SaineTestCrash.clap\",\"format\":\"clap\",\"reason\":\"crashed"));
    CHECK(has(r1, "\"from_cache\":false"));
    const long first = scanned(r1);
    CHECK(first > 5);

    // Second run: everything from the cache, crash still blocklisted.
    const std::string r2 = run_scan(cache.path, false, &rc, plugins.path);
    REQUIRE(rc == 0);
    CHECK_EQ(scanned(r2), 0L);
    CHECK(has(r2, "\"from_cache\":true"));
    CHECK(has(r2, "SaineTestCrash.clap\",\"format\":\"clap\",\"reason\":\"crashed"));
    CHECK(has(r2, "\"id\":\"studio.saine.test.gain\""));

    // Touch the gain bundle: only it is re-scanned.
    // (macOS: the bundle folder; Windows/Linux: the single-file .clap.)
    const std::string bundle_utf8 = plugins.path + "/SaineTestGain.clap";
    const auto bundle = os::path_from_utf8(bundle_utf8);
    const int64_t before = os::file_mtime_ns(bundle_utf8);
    std::error_code ec;
    std::filesystem::last_write_time(
        bundle, std::filesystem::file_time_type::clock::now() + std::chrono::seconds(5), ec);
    REQUIRE(!ec);
    const int64_t after = os::file_mtime_ns(bundle_utf8);
    std::fprintf(stderr, "  touched %s: mtime %lld -> %lld\n", bundle_utf8.c_str(), static_cast<long long>(before),
                 static_cast<long long>(after));
    CHECK(after != before);
    const std::string r3 = run_scan(cache.path, false, &rc, plugins.path);
    REQUIRE(rc == 0);
    CHECK_EQ(scanned(r3), 1L);
    if (scanned(r3) != 1L) std::fprintf(stderr, "  report after touching:\n%s\n", r3.c_str());
    CHECK(has(r3, "\"id\":\"studio.saine.test.gain\""));

    // --rescan scans everything again (the crash is retried and fails again).
    const std::string r4 = run_scan(cache.path, true, &rc, plugins.path);
    REQUIRE(rc == 0);
    CHECK_EQ(scanned(r4), first);
    CHECK(has(r4, "SaineTestCrash.clap\",\"format\":\"clap\",\"reason\":\"crashed"));
}

namespace {

// Replaces every occurrence of `from` in `s` with `to`; returns the count.
int replace_all(std::string& s, const std::string& from, const std::string& to) {
    int n = 0;
    for (std::size_t at = s.find(from); at != std::string::npos; at = s.find(from, at + to.size())) {
        s.replace(at, from.size(), to);
        ++n;
    }
    return n;
}

// Overwrites the file at `path` (UTF-8) with `text`.
void write_text(const std::string& path, const std::string& text) {
    std::ofstream f(os::path_from_utf8(path), std::ios::binary | std::ios::trunc);
    f << text;
}

} // namespace

// The cache parser decodes valid \uXXXX escapes, and a cache with a malformed
// one (non-hex digits) is treated as unreadable: the scanner rescans instead
// of crashing.
TEST_CASE(plugin_scanner_cache_unicode_escapes) {
    TempFile cache("plugin_cache.json");
    int rc = -1;
    const std::string r1 = run_scan(cache.path, false, &rc);
    REQUIRE(rc == 0);
    const long first = scanned(r1);
    REQUIRE(first > 0);
    const std::string original = slurp(cache.path);

    // Valid escape: "\u006Fk" is "ok", so every entry is still served from
    // the cache.
    std::string valid = original;
    const int ok_entries = replace_all(valid, "\"status\":\"ok\"", "\"status\":\"\\u006Fk\"");
    REQUIRE(ok_entries > 0);
    write_text(cache.path, valid);
    const std::string r2 = run_scan(cache.path, false, &rc);
    REQUIRE(rc == 0);
    CHECK_EQ(scanned(r2), 0L);
    CHECK(has(r2, "\"from_cache\":true"));
    // The escape decoded to "ok": the scanner rewrites the cache with the same
    // successful entries (a wrong decode would turn them into failures).
    std::string rewritten = slurp(cache.path);
    CHECK_EQ(replace_all(rewritten, "\"status\":\"ok\"", "\"status\":\"ok\""), ok_entries);

    // Malformed escapes: the cache is ignored and everything is rescanned.
    for (const char* bad : {"\\uZZZZ", "\\u12G4", "\\u+123", "\\u 123"}) {
        std::string broken = original;
        REQUIRE(replace_all(broken, "\"status\":\"ok\"", std::string("\"status\":\"") + bad + "\"") > 0);
        write_text(cache.path, broken);
        const std::string r = run_scan(cache.path, false, &rc);
        std::fprintf(stderr, "  cache with %s: exit %d\n", bad, rc);
        REQUIRE(rc == 0);
        CHECK_EQ(scanned(r), first);
        CHECK(has(r, "\"from_cache\":false"));
    }
}
