// Out-of-process scanner: finds Apple's AUs and the test plugins (via
// SAINE_PLUGIN_PATH), blocklists SaineTestCrash.clap (crashes in its
// entry) while the scan completes, serves unchanged entries from the cache,
// re-scans a bundle whose modification time changed, and --rescan clears
// the blocklist.
#include "test_harness.hpp"
#include "test_util.hpp"

#include <cstdio>
#include <string>
#include <sys/stat.h>
#include <sys/time.h>

using namespace testutil;

namespace {

std::string run_scan(const std::string& cache, bool rescan, int* exit_code) {
    const std::string cmd = std::string("SAINE_PLUGIN_PATH='") + SAINE_TEST_PLUGIN_DIR +
                            "' SAINE_PLUGIN_SCAN_TIMEOUT_MS=8000 '" + SAINE_PLUGIN_SCANNER_PATH +
                            "' --scan --cache '" + cache + "'" + (rescan ? " --rescan" : "") + " 2>/dev/null";
    std::string out;
    FILE* p = ::popen(cmd.c_str(), "r");
    if (p == nullptr) return out;
    char buf[4096];
    std::size_t n;
    while ((n = std::fread(buf, 1, sizeof buf, p)) > 0) out.append(buf, n);
    const int st = ::pclose(p);
    if (exit_code != nullptr) *exit_code = WIFEXITED(st) ? WEXITSTATUS(st) : -1;
    return out;
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
    // Apple AUs.
    CHECK(has(r1, "\"id\":\"aufx lpas appl\""));
    CHECK(has(r1, "\"id\":\"aufx dely appl\""));
    CHECK(has(r1, "\"id\":\"aumu dls  appl\""));
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
    const std::string bundle = std::string(SAINE_TEST_PLUGIN_DIR) + "/SaineTestGain.clap";
    struct timeval now[2];
    ::gettimeofday(&now[0], nullptr);
    now[0].tv_sec += 5;
    now[1] = now[0];
    REQUIRE(::utimes(bundle.c_str(), now) == 0);
    const std::string r3 = run_scan(cache.path, false, &rc);
    REQUIRE(rc == 0);
    CHECK_EQ(scanned(r3), 1L);
    CHECK(has(r3, "\"id\":\"studio.saine.test.gain\""));

    // --rescan scans everything again (the crash is retried and fails again).
    const std::string r4 = run_scan(cache.path, true, &rc);
    REQUIRE(rc == 0);
    CHECK_EQ(scanned(r4), first);
    CHECK(has(r4, "SaineTestCrash.clap\",\"format\":\"clap\",\"reason\":\"crashed"));
}
