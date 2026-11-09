// Shaper parameter layout (saine_core::builtin::shaper) and registry defaults
// for the device tests and the benchmark.
#pragma once

#include "devices/builtin.hpp"

#include <cstddef>
#include <vector>

namespace devtest {

// A registered device's default plain values in parameter order.
inline std::vector<float> builtin_defaults(const char* id) {
    const saine::BuiltinDescriptor* d = saine::find_builtin(id);
    return {d->defaults.begin(), d->defaults.end()};
}

// Shaper parameter layout: 11 globals, then 7 lanes of 54.
constexpr std::size_t kShaperGlobals = 11;
constexpr std::size_t kShaperStride = 54;
inline std::size_t shaper_lane(int lane, int offset) { return kShaperGlobals + kShaperStride * static_cast<std::size_t>(lane) + static_cast<std::size_t>(offset); }
// Point k (0-based) of a lane: x, y, bend at offsets 6 + 3k, +1, +2.
inline std::size_t shaper_point(int lane, int k, int field) { return shaper_lane(lane, 6 + 3 * k + field); }

} // namespace devtest
