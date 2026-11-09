// LFO wave shapes (handoffs/device-specs/lfo.md), shared by the LFO kernel
// and its tests. Mirrors saine_core::modulation::lfo_wave / cycle_random bit
// for bit in the integer hash, so the app draws the waves the engine plays.
#pragma once

#include <cmath>
#include <cstdint>

namespace saine::lfo {

enum Shape : uint32_t { kSine = 0, kTriangle, kSawUp, kSawDown, kSquare, kRandom, kSmoothRandom, kShapeCount };

// Tempo divisions in quarter notes, slowest first (saine_core::builtin::lfo::DIVISIONS).
inline constexpr double kDivisionQuarters[20] = {32.0,  16.0,  8.0,         4.0,    3.0,  2.0,         4.0 / 3.0,
                                                 1.5,   1.0,   2.0 / 3.0,   0.75,   0.5,  1.0 / 3.0,   0.375,
                                                 0.25,  1.0 / 6.0, 0.125,   0.09375, 0.0625, 1.0 / 24.0};

// One value in -1..1 per cycle, repeating every 4096 cycles.
inline float cycle_random(int64_t cycle) noexcept {
    const auto n = static_cast<uint32_t>(((cycle % 4096) + 4096) % 4096);
    uint32_t x = (n * 0x9E3779B1u) ^ 0x85EBCA6Bu;
    x ^= x >> 15;
    x *= 0x2C1B3C6Du;
    x ^= x >> 12;
    x *= 0x297A2D39u;
    x ^= x >> 15;
    return static_cast<float>(x >> 8) / 8388608.0f - 1.0f;
}

// Bipolar value of `shape` at cycle position `x` (cycles; the fraction is the phase).
inline float wave(uint32_t shape, double x) noexcept {
    const double c = std::floor(x);
    const auto p = static_cast<float>(x - c);
    switch (shape) {
    case kSine: return std::sin(6.28318530717958647692f * p);
    case kTriangle: return p < 0.25f ? 4.0f * p : p < 0.75f ? 2.0f - 4.0f * p : 4.0f * p - 4.0f;
    case kSawUp: return 2.0f * p - 1.0f;
    case kSawDown: return 1.0f - 2.0f * p;
    case kSquare: return p < 0.5f ? 1.0f : -1.0f;
    case kRandom: return cycle_random(static_cast<int64_t>(c));
    default: {
        const float a = cycle_random(static_cast<int64_t>(c));
        const float b = cycle_random(static_cast<int64_t>(c) + 1);
        return a + (b - a) * (1.0f - std::cos(3.14159265358979323846f * p)) * 0.5f;
    }
    }
}

} // namespace saine::lfo
