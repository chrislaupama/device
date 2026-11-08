// Minimal self-contained test harness (doctest-flavoured).
//
//   TEST_CASE(name) { CHECK(x); REQUIRE(y); CHECK_NEAR(a, b, eps); }
//
// Binary usage: (no args) run all | --list | --run <name> |
//               --write-ctest <out.cmake> <path-to-binary>
#pragma once

#include <cmath>
#include <cstdio>
#include <string>
#include <vector>

namespace th {

struct TestCase {
    const char* name;
    void (*fn)();
};

inline std::vector<TestCase>& registry() {
    static std::vector<TestCase> r;
    return r;
}

struct Registrar {
    Registrar(const char* name, void (*fn)()) { registry().push_back({name, fn}); }
};

struct RequireFailed {};

inline int& failure_count() {
    static int n = 0;
    return n;
}

// Set by SKIP(): the test does not apply on this platform/configuration.
// The runner exits with kSkipExitCode, which CTest reports as "Skipped".
inline bool& skip_requested() {
    static bool b = false;
    return b;
}
inline constexpr int kSkipExitCode = 77;

inline void report(const char* file, int line, const std::string& what) {
    ++failure_count();
    std::fprintf(stderr, "%s:%d: FAILED: %s\n", file, line, what.c_str());
}

} // namespace th

#define TH_CONCAT2(a, b) a##b
#define TH_CONCAT(a, b) TH_CONCAT2(a, b)

#define TEST_CASE(name)                                                                                                \
    static void name();                                                                                                \
    static const th::Registrar TH_CONCAT(th_reg_, name){#name, &name};                                                 \
    static void name()

// Ends the current test as skipped (not passed, not failed) with a reason.
#define SKIP(reason)                                                                                                   \
    do {                                                                                                               \
        std::fprintf(stderr, "  SKIPPED: %s\n", reason);                                                              \
        th::skip_requested() = true;                                                                                   \
        return;                                                                                                        \
    } while (0)

#define CHECK(cond)                                                                                                    \
    do {                                                                                                               \
        if (!(cond)) th::report(__FILE__, __LINE__, #cond);                                                            \
    } while (0)

#define REQUIRE(cond)                                                                                                  \
    do {                                                                                                               \
        if (!(cond)) {                                                                                                 \
            th::report(__FILE__, __LINE__, "REQUIRE " #cond);                                                          \
            throw th::RequireFailed{};                                                                                 \
        }                                                                                                              \
    } while (0)

#define CHECK_NEAR(a, b, eps)                                                                                          \
    do {                                                                                                               \
        const double th_a_ = static_cast<double>(a), th_b_ = static_cast<double>(b);                                   \
        if (!(std::fabs(th_a_ - th_b_) <= static_cast<double>(eps)))                                                   \
            th::report(__FILE__, __LINE__,                                                                             \
                       std::string(#a " ~= " #b ": ") + std::to_string(th_a_) + " vs " + std::to_string(th_b_));       \
    } while (0)

#define CHECK_EQ(a, b)                                                                                                 \
    do {                                                                                                               \
        const auto th_a_ = (a);                                                                                        \
        const auto th_b_ = (b);                                                                                        \
        if (!(th_a_ == th_b_))                                                                                         \
            th::report(__FILE__, __LINE__,                                                                             \
                       std::string(#a " == " #b ": ") + std::to_string(th_a_) + " vs " + std::to_string(th_b_));       \
    } while (0)
