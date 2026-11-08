// Windows backend placeholder.
//
// TODO(windows): implement
//   - WASAPI shared mode (event-driven, IAudioClient3 low-latency periods) as
//     the default, WASAPI exclusive mode as an option;
//   - ASIO (behind a build option, since the SDK has a separate licence).
// The RT thread should be registered with MMCSS ("Pro Audio") and must call
// the same AudioDevice::Callback::render() as the Core Audio backend.
#if defined(_WIN32)

#include "audio_device.hpp"
#include "saine_engine.h"

namespace saine {
namespace {

class WasapiDevice final : public AudioDevice {
public:
    int32_t validate(const Config&, std::string&) const override { return SE_OK; }
    int32_t start(const Config&, Callback*, std::string& err) override {
        err = "TODO: WASAPI/ASIO output backend is not implemented yet";
        return SE_ERR_DEVICE;
    }
    int32_t stop() override { return SE_OK; }
    bool is_running() const override { return false; }
    Status status() const override { return {}; }
};

} // namespace

std::unique_ptr<AudioDevice> create_platform_audio_device() { return std::make_unique<WasapiDevice>(); }
int32_t list_audio_devices(std::vector<DeviceInfo>& out, std::string&) {
    out.clear();
    return SE_OK;
}
double query_default_output_sample_rate() { return 0.0; }

} // namespace saine

#endif // _WIN32
