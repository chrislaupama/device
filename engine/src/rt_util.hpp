// Small helpers that are safe to use on the real-time thread.
#pragma once

#include <atomic>
#include <chrono>
#include <cstdint>

#if defined(__SSE__) || defined(_M_X64)
#include <xmmintrin.h>
#endif

namespace dstudio {

// Enables flush-to-zero / denormals-are-zero for the scope, restoring the
// previous FP control state on exit. Denormals in decaying signals can cost
// 10-100x per operation on some CPUs.
class ScopedFlushDenormals {
public:
    ScopedFlushDenormals() noexcept {
#if defined(__aarch64__)
        // Inline asm rather than __builtin_arm_rsr64/wsr64, which GCC lacks.
        asm volatile("mrs %0, fpcr" : "=r"(saved_));
        const uint64_t fz = saved_ | (uint64_t{1} << 24); // FZ
        asm volatile("msr fpcr, %0" : : "r"(fz));
#elif defined(__SSE__) || defined(_M_X64)
        saved_ = _mm_getcsr();
        _mm_setcsr(static_cast<unsigned>(saved_) | 0x8040u); // FTZ | DAZ
#endif
    }
    ~ScopedFlushDenormals() {
#if defined(__aarch64__)
        asm volatile("msr fpcr, %0" : : "r"(saved_));
#elif defined(__SSE__) || defined(_M_X64)
        _mm_setcsr(static_cast<unsigned>(saved_));
#endif
    }
    ScopedFlushDenormals(const ScopedFlushDenormals&) = delete;
    ScopedFlushDenormals& operator=(const ScopedFlushDenormals&) = delete;

private:
    uint64_t saved_ = 0;
};

// Monotonic nanoseconds. steady_clock maps to clock_gettime/mach time on the
// platforms we target: no allocation, no locks.
inline uint64_t now_ns() noexcept {
    return static_cast<uint64_t>(std::chrono::duration_cast<std::chrono::nanoseconds>(
                                     std::chrono::steady_clock::now().time_since_epoch())
                                     .count());
}

// Single-writer "raise to max" for a float atomic that a reader may reset
// with exchange(0). Lock-free; the CAS only retries if the reader raced us.
inline void atomic_store_max(std::atomic<float>& a, float v) noexcept {
    float cur = a.load(std::memory_order_relaxed);
    while (v > cur && !a.compare_exchange_weak(cur, v, std::memory_order_relaxed)) {
    }
}

} // namespace dstudio
