// Fallback for platforms without any audio backend (keeps the engine
// buildable for offline rendering everywhere).
#if !defined(__APPLE__) && !defined(_WIN32) && !defined(__linux__)

#include "audio_device.hpp"
#include "dstudio_engine.h"

namespace dstudio {
namespace {

class NullDevice final : public AudioDevice {
public:
    int32_t validate(const Config&, std::string&) const override { return SE_OK; }
    int32_t start(const Config&, Callback*, std::string& err) override {
        err = "no audio device backend for this platform";
        return SE_ERR_DEVICE;
    }
    int32_t stop() override { return SE_OK; }
    bool is_running() const override { return false; }
    Status status() const override { return {}; }
};

} // namespace

std::unique_ptr<AudioDevice> create_platform_audio_device() { return std::make_unique<NullDevice>(); }
int32_t list_audio_devices(std::vector<DeviceInfo>& out, std::string&) {
    out.clear();
    return SE_OK;
}
double query_default_output_sample_rate() { return 0.0; }

} // namespace dstudio

#endif
