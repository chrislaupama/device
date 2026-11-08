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

// Runs the scanner coordinator (stdout captured) with SAINE_PLUGIN_PATH set
// to the test plugin folder.
std::string run_scan(const std::string& cache, bool rescan, int* exit_code) {
    os::set_env("SAINE_PLUGIN_PATH", SAINE_TEST_PLUGIN_DIR);
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
    TempFile cache("plugin_cache.json");
    int rc = -1;
    const std::string r1 = run_scan(cache.path, false, &rc);
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
    const std::string r2 = run_scan(cache.path, false, &rc);
    REQUIRE(rc == 0);
    CHECK_EQ(scanned(r2), 0L);
    CHECK(has(r2, "\"from_cache\":true"));
    CHECK(has(r2, "SaineTestCrash.clap\",\"format\":\"clap\",\"reason\":\"crashed"));
    CHECK(has(r2, "\"id\":\"studio.saine.test.gain\""));

    // Touch the gain bundle: only it is re-scanned.
    // (macOS: the bundle folder; Windows/Linux: the single-file .clap.)
    const std::string bundle_utf8 = std::string(SAINE_TEST_PLUGIN_DIR) + "/SaineTestGain.clap";
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
    const std::string r3 = run_scan(cache.path, false, &rc);
    REQUIRE(rc == 0);
    CHECK_EQ(scanned(r3), 1L);
    if (scanned(r3) != 1L) std::fprintf(stderr, "  report after touching:\n%s\n", r3.c_str());
    CHECK(has(r3, "\"id\":\"studio.saine.test.gain\""));

    // --rescan scans everything again (the crash is retried and fails again).
    const std::string r4 = run_scan(cache.path, true, &rc);
    REQUIRE(rc == 0);
    CHECK_EQ(scanned(r4), first);
    CHECK(has(r4, "SaineTestCrash.clap\",\"format\":\"clap\",\"reason\":\"crashed"));
}
