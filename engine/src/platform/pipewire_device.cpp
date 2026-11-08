// Linux backend placeholder.
//
// TODO(linux): implement a PipeWire stream backend (pw_stream, F32P) as the
// default and a JACK client backend as an alternative. Both must call the
// same AudioDevice::Callback::render() as the Core Audio backend from their
// process callbacks. ALSA direct can come later if needed.
#if defined(__linux__)

#include "audio_device.hpp"
#include "saine_engine.h"

namespace saine {
namespace {

class PipeWireDevice final : public AudioDevice {
public:
    int32_t start(const Config&, Callback*, std::string& err) override {
        err = "TODO: PipeWire/JACK output backend is not implemented yet";
        return SE_ERR_DEVICE;
    }
    int32_t stop() override { return SE_OK; }
    bool is_running() const override { return false; }
};

} // namespace

std::unique_ptr<AudioDevice> create_platform_audio_device() { return std::make_unique<PipeWireDevice>(); }
double query_default_output_sample_rate() { return 0.0; }

} // namespace saine

#endif // __linux__
