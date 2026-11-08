// Test runner + global allocation tracking for the RT-safety tests.
//
// operator new/delete are replaced for the whole test binary. When a thread
// has entered an "RT scope" (test_util.hpp: RtAllocGuard), every allocation
// or deallocation on that thread is counted. Tests assert the count is zero
// across rendering.
#include "test_harness.hpp"
#include "test_util.hpp"

#include "platform/os.hpp"

#include <atomic>
#include <cstdlib>
#include <cstring>
#include <new>

#if defined(_WIN32)
#include <malloc.h>
#endif

namespace {
thread_local bool t_in_rt_scope = false;
std::atomic<long> g_rt_allocs{0};
std::atomic<long> g_rt_frees{0};

// Every operator new in the binary goes through here, so one allocator
// serves both plain and over-aligned requests and one free function serves
// every operator delete. Windows has no posix_memalign and memory from
// _aligned_malloc must be released with _aligned_free, so there every
// allocation uses the aligned CRT pair.
void* do_alloc(std::size_t size, std::size_t align) {
    if (t_in_rt_scope) g_rt_allocs.fetch_add(1, std::memory_order_relaxed);
    if (size == 0) size = 1;
#if defined(_WIN32)
    return _aligned_malloc(size, align > alignof(std::max_align_t) ? align : alignof(std::max_align_t));
#else
    void* p = nullptr;
    if (align <= alignof(std::max_align_t)) {
        p = std::malloc(size);
    } else if (posix_memalign(&p, align, size) != 0) {
        p = nullptr;
    }
    return p;
#endif
}

void do_free(void* p) noexcept {
    if (p != nullptr && t_in_rt_scope) g_rt_frees.fetch_add(1, std::memory_order_relaxed);
#if defined(_WIN32)
    _aligned_free(p);
#else
    std::free(p);
#endif
}
} // namespace

namespace testutil {
void set_rt_scope(bool on) noexcept { t_in_rt_scope = on; }
long rt_alloc_count() noexcept { return g_rt_allocs.load(); }
long rt_free_count() noexcept { return g_rt_frees.load(); }
} // namespace testutil

void* operator new(std::size_t n) {
    if (void* p = do_alloc(n, 0)) return p;
    throw std::bad_alloc();
}
void* operator new[](std::size_t n) {
    if (void* p = do_alloc(n, 0)) return p;
    throw std::bad_alloc();
}
void* operator new(std::size_t n, const std::nothrow_t&) noexcept { return do_alloc(n, 0); }
void* operator new[](std::size_t n, const std::nothrow_t&) noexcept { return do_alloc(n, 0); }
void* operator new(std::size_t n, std::align_val_t a) {
    if (void* p = do_alloc(n, static_cast<std::size_t>(a))) return p;
    throw std::bad_alloc();
}
void* operator new[](std::size_t n, std::align_val_t a) {
    if (void* p = do_alloc(n, static_cast<std::size_t>(a))) return p;
    throw std::bad_alloc();
}
void operator delete(void* p) noexcept { do_free(p); }
void operator delete[](void* p) noexcept { do_free(p); }
void operator delete(void* p, std::size_t) noexcept { do_free(p); }
void operator delete[](void* p, std::size_t) noexcept { do_free(p); }
void operator delete(void* p, std::align_val_t) noexcept { do_free(p); }
void operator delete[](void* p, std::align_val_t) noexcept { do_free(p); }
void operator delete(void* p, std::size_t, std::align_val_t) noexcept { do_free(p); }
void operator delete[](void* p, std::size_t, std::align_val_t) noexcept { do_free(p); }

static int run_one(const th::TestCase& tc) {
    const int before = th::failure_count();
    th::skip_requested() = false;
    std::fprintf(stderr, "[ RUN  ] %s\n", tc.name);
    try {
        tc.fn();
    } catch (const th::RequireFailed&) {
    } catch (const std::exception& e) {
        th::report(__FILE__, __LINE__, std::string("uncaught exception: ") + e.what());
    }
    const bool ok = th::failure_count() == before;
    if (ok && th::skip_requested()) {
        std::fprintf(stderr, "[ SKIP ] %s\n", tc.name);
        return th::kSkipExitCode;
    }
    std::fprintf(stderr, "[ %s ] %s\n", ok ? " OK " : "FAIL", tc.name);
    return ok ? 0 : 1;
}

int main(int argc, char** argv) {
    auto& tests = th::registry();
    if (argc >= 2 && std::strcmp(argv[1], "--list") == 0) {
        for (auto& t : tests) std::printf("%s\n", t.name);
        return 0;
    }
    if (argc >= 4 && std::strcmp(argv[1], "--write-ctest") == 0) {
        std::FILE* f = saine::os::fopen_utf8(argv[2], "w");
        if (f == nullptr) return 1;
        for (auto& t : tests) {
            std::fprintf(f, "add_test(\"%s\" \"%s\" --run \"%s\")\n", t.name, argv[3], t.name);
            std::fprintf(f, "set_tests_properties(\"%s\" PROPERTIES TIMEOUT 300 SKIP_RETURN_CODE %d)\n", t.name,
                         th::kSkipExitCode);
        }
        std::fclose(f);
        return 0;
    }
    if (argc >= 3 && std::strcmp(argv[1], "--run") == 0) {
        for (auto& t : tests)
            if (std::strcmp(t.name, argv[2]) == 0) return run_one(t);
        std::fprintf(stderr, "no such test: %s\n", argv[2]);
        return 2;
    }
    int failed = 0, skipped = 0;
    for (auto& t : tests) {
        const int rc = run_one(t);
        if (rc == th::kSkipExitCode) ++skipped;
        else failed += rc;
    }
    std::fprintf(stderr, "%zu tests, %d failed, %d skipped\n", tests.size(), failed, skipped);
    return failed == 0 ? 0 : 1;
}
