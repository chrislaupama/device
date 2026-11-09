// Default values (registry order) of the sampler devices, copied from their
// Rust definitions (crates/saine-core/src/builtin/{sampler,pad_sampler,drum_rack}.rs).
#pragma once

#include <vector>

namespace devtest {

inline std::vector<float> sampler_defaults() {
    return {1, 0, 0, 8, 0, 60, 0, 0, 0, 0, 0, 2, 1, 0, 2, 120, 1, 48, 0, 1, 20000, 0, 0, 0, 0, 0, 300, 0, 200,
            0, 2, 0, 0, 0, 0, 1, 0, 1, 0, 0, 0, 0, 0, 60, 0, 50, 2};
}

inline std::vector<float> pad_defaults() { return {0, 0, 0, 0, 0, 0, 1, 0, 0, 0}; }

inline std::vector<float> drum_rack_defaults() {
    std::vector<float> v;
    for (int p = 0; p < 16; ++p) {
        v.push_back(1.0f);
        for (float x : pad_defaults()) v.push_back(x);
    }
    return v;
}

} // namespace devtest
