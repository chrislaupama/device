// Defaults of saine_core::builtin::trance_gate and ::shaper in registry
// order (the Rust test engine_registry_matches_core_definitions keeps the
// order in step; these values mirror the definitions' defaults).
#pragma once

#include <cstddef>
#include <vector>

namespace devtest {

inline std::vector<float> trance_gate_defaults() {
    return {0.f,  7.f,  3.f,     160.f, 60.f, 40.f,    0.f,  100.f, 16.f, 23387.f, 0.f,  16.f, 21845.f, 0.f,  16.f, 46967.f,
            0.f,  16.f, 19983.f, 0.f,   12.f, 2779.f,  0.f,  32.f,  44397.f, 30125.f, 16.f, 18761.f, 0.f,  16.f, 56669.f, 0.f};
}

// Shaper parameter layout: 11 globals, then 7 lanes of 54.
constexpr std::size_t kShaperGlobals = 11;
constexpr std::size_t kShaperStride = 54;
inline std::size_t shaper_lane(int lane, int offset) { return kShaperGlobals + kShaperStride * static_cast<std::size_t>(lane) + static_cast<std::size_t>(offset); }
// Point k (0-based) of a lane: x, y, bend at offsets 6 + 3k, +1, +2.
inline std::size_t shaper_point(int lane, int k, int field) { return shaper_lane(lane, 6 + 3 * k + field); }

inline std::vector<float> shaper_defaults() {
    std::vector<float> v = {0.f, -24.f, 100.f, 0.f, 0.f, 0.f, 1.2973f, 150.f, 12000.f, 6.f, 8.f};
    struct Lane {
        float on, rate, points;
        std::vector<float> x, y, bend;
        float rest_y;
    };
    const Lane lanes[7] = {
        {1, 8, 3, {0, .5f, 1}, {0, 100, 100}, {55, 0, 0}, 100},
        {0, 8, 4, {0, .5f, .5f, 1}, {0, 0, 50, 50}, {0, 0, 0, 0}, 50},
        {0, 5, 3, {0, .5f, 1}, {50, 100, 50}, {45, 45, 0}, 50},
        {0, 5, 5, {0, .25f, .5f, .75f, 1}, {50, 100, 50, 0, 50}, {50, 50, -50, -50, 0}, 50},
        {0, 3, 2, {0, 1}, {100, 15}, {40, 0}, 15},
        {0, 8, 2, {0, 1}, {100, 0}, {-50, 0}, 0},
        {0, 8, 2, {0, 1}, {0, -12}, {50, 0}, -12},
    };
    for (const Lane& l : lanes) {
        v.insert(v.end(), {l.on, l.rate, 100.f, 0.f, 150.f, l.points});
        for (std::size_t k = 0; k < 16; ++k) {
            v.push_back(k < l.x.size() ? l.x[k] : 1.f);
            v.push_back(k < l.y.size() ? l.y[k] : l.rest_y);
            v.push_back(k < l.bend.size() ? l.bend[k] : 0.f);
        }
    }
    return v;
}

} // namespace devtest
