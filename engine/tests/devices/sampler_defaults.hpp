// Default values (registry order) of the sampler devices: their descriptors'
// defaults (equal to the Rust definitions, checked by saine-engine's tests).
#pragma once

#include "devices/builtin.hpp"

#include <vector>

namespace devtest {

inline std::vector<float> builtin_defaults(const char* id) {
    const auto d = saine::find_builtin(id)->defaults;
    return {d.begin(), d.end()};
}

inline std::vector<float> sampler_defaults() { return builtin_defaults("sampler-steam"); }
inline std::vector<float> pad_defaults() { return builtin_defaults("pad-sampler"); }
inline std::vector<float> drum_rack_defaults() { return builtin_defaults("drum-rack"); }

} // namespace devtest
