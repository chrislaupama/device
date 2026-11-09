// Four-lane float vector for the built-in device kernels (header only, RT
// safe): NEON on ARM, SSE on x86, plain scalar code elsewhere. Only the
// handful of operations the kernels need.
#pragma once

#if defined(__ARM_NEON) || defined(__ARM_NEON__) || defined(_M_ARM64)
#include <arm_neon.h>
#define SAINE_SIMD_NEON 1
#elif defined(__SSE2__) || defined(_M_X64) || (defined(_M_IX86_FP) && _M_IX86_FP >= 2)
#include <emmintrin.h>
#define SAINE_SIMD_SSE 1
#endif

namespace saine::dsp {

struct f32x4 {
#if defined(SAINE_SIMD_NEON)
    float32x4_t v;
    static f32x4 zero() noexcept { return {vdupq_n_f32(0.0f)}; }
    static f32x4 set1(float x) noexcept { return {vdupq_n_f32(x)}; }
    static f32x4 load(const float* p) noexcept { return {vld1q_f32(p)}; }
    void store(float* p) const noexcept { vst1q_f32(p, v); }
    friend f32x4 operator+(f32x4 a, f32x4 b) noexcept { return {vaddq_f32(a.v, b.v)}; }
    friend f32x4 operator-(f32x4 a, f32x4 b) noexcept { return {vsubq_f32(a.v, b.v)}; }
    friend f32x4 operator*(f32x4 a, f32x4 b) noexcept { return {vmulq_f32(a.v, b.v)}; }
    // a + b * c
    static f32x4 madd(f32x4 a, f32x4 b, f32x4 c) noexcept { return {vfmaq_f32(a.v, b.v, c.v)}; }
    float sum() const noexcept { return vaddvq_f32(v); }
#elif defined(SAINE_SIMD_SSE)
    __m128 v;
    static f32x4 zero() noexcept { return {_mm_setzero_ps()}; }
    static f32x4 set1(float x) noexcept { return {_mm_set1_ps(x)}; }
    static f32x4 load(const float* p) noexcept { return {_mm_loadu_ps(p)}; }
    void store(float* p) const noexcept { _mm_storeu_ps(p, v); }
    friend f32x4 operator+(f32x4 a, f32x4 b) noexcept { return {_mm_add_ps(a.v, b.v)}; }
    friend f32x4 operator-(f32x4 a, f32x4 b) noexcept { return {_mm_sub_ps(a.v, b.v)}; }
    friend f32x4 operator*(f32x4 a, f32x4 b) noexcept { return {_mm_mul_ps(a.v, b.v)}; }
    static f32x4 madd(f32x4 a, f32x4 b, f32x4 c) noexcept { return {_mm_add_ps(a.v, _mm_mul_ps(b.v, c.v))}; }
    float sum() const noexcept {
        const __m128 s = _mm_add_ps(v, _mm_movehl_ps(v, v));
        return _mm_cvtss_f32(_mm_add_ss(s, _mm_shuffle_ps(s, s, 1)));
    }
#else
    float v[4];
    static f32x4 zero() noexcept { return {{0.0f, 0.0f, 0.0f, 0.0f}}; }
    static f32x4 set1(float x) noexcept { return {{x, x, x, x}}; }
    static f32x4 load(const float* p) noexcept { return {{p[0], p[1], p[2], p[3]}}; }
    void store(float* p) const noexcept {
        for (int i = 0; i < 4; ++i) p[i] = v[i];
    }
    friend f32x4 operator+(f32x4 a, f32x4 b) noexcept { return {{a.v[0] + b.v[0], a.v[1] + b.v[1], a.v[2] + b.v[2], a.v[3] + b.v[3]}}; }
    friend f32x4 operator-(f32x4 a, f32x4 b) noexcept { return {{a.v[0] - b.v[0], a.v[1] - b.v[1], a.v[2] - b.v[2], a.v[3] - b.v[3]}}; }
    friend f32x4 operator*(f32x4 a, f32x4 b) noexcept { return {{a.v[0] * b.v[0], a.v[1] * b.v[1], a.v[2] * b.v[2], a.v[3] * b.v[3]}}; }
    static f32x4 madd(f32x4 a, f32x4 b, f32x4 c) noexcept { return a + b * c; }
    float sum() const noexcept { return (v[0] + v[1]) + (v[2] + v[3]); }
#endif
};

// Dot product of n floats (n a multiple of 4).
inline float dot4(const float* a, const float* b, int n) noexcept {
    f32x4 acc0 = f32x4::zero(), acc1 = f32x4::zero();
    int i = 0;
    for (; i + 8 <= n; i += 8) {
        acc0 = f32x4::madd(acc0, f32x4::load(a + i), f32x4::load(b + i));
        acc1 = f32x4::madd(acc1, f32x4::load(a + i + 4), f32x4::load(b + i + 4));
    }
    if (i < n) acc0 = f32x4::madd(acc0, f32x4::load(a + i), f32x4::load(b + i));
    return (acc0 + acc1).sum();
}

} // namespace saine::dsp
