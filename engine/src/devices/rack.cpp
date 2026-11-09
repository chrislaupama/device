// Chain (device rack) head (handoffs/device-specs/chain-head.md): audio
// passes through; its eight macros are modulation outputs (value =
// macro / 100, gain 1) that the engine applies to the members' parameters
// as absolute routes. Bypassing the head bypasses its members (the app
// publishes them disabled).
#include "builtin.hpp"

#include <algorithm>

namespace saine {
namespace {

constexpr const char* kParams[] = {"macro1", "macro2", "macro3", "macro4", "macro5", "macro6", "macro7", "macro8"};
constexpr float kDefaults[] = {0.0f, 0.0f, 0.0f, 0.0f, 0.0f, 0.0f, 0.0f, 0.0f};
constexpr uint32_t kMacros = 8;

class Rack final : public BuiltinProcessor {
public:
    void prepare(double, uint32_t) override {}
    void reset() noexcept override {}
    void process(float*, float*, uint32_t, const float*, const BuiltinContext&, float*) noexcept override {}
    uint32_t mod_outputs() const noexcept override { return kMacros; }
    void modulate(const float* p, uint32_t, const BuiltinContext&, float* out) noexcept override {
        for (uint32_t i = 0; i < kMacros; ++i) {
            out[2 * i] = std::clamp(p[i] / 100.0f, 0.0f, 1.0f);
            out[2 * i + 1] = 1.0f;
        }
    }
};

std::unique_ptr<BuiltinProcessor> create() { return std::make_unique<Rack>(); }

} // namespace

extern const BuiltinDescriptor kRackDescriptor;
const BuiltinDescriptor kRackDescriptor{"device-rack", kParams, {}, &create, {}, kDefaults};

} // namespace saine
