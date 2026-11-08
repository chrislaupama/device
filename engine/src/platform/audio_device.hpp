// Platform audio-device abstraction. All platform-specific code lives in
// engine/src/platform/; the rest of the engine only sees this interface.
//
// Exactly one backend translation unit provides create_platform_audio_device()
// and query_default_output_sample_rate() for the current platform:
//   coreaudio_device.cpp  macOS  (AudioUnit kAudioUnitSubType_DefaultOutput)
//   wasapi_device.cpp     Windows (TODO: WASAPI exclusive/shared + ASIO)
//   pipewire_device.cpp   Linux  (TODO: PipeWire, JACK)
//   audio_device.cpp      fallback for any other platform
#pragma once

#include <cstdint>
#include <memory>
#include <string>

namespace saine {

class AudioDevice {
public:
    // Implemented by the engine. Called on the device's real-time thread.
    class Callback {
    public:
        // `channels` holds `num_channels` non-interleaved float32 buffers of
        // `frames` frames each. Must fill all of them. Must be RT safe.
        virtual void render(float* const* channels, uint32_t num_channels, uint32_t frames) noexcept = 0;

    protected:
        ~Callback() = default;
    };

    struct Config {
        double sample_rate = 48000.0; // engine rate; backend converts if the hardware differs
        uint32_t channels = 2;
        uint32_t preferred_block = 256;
    };

    virtual ~AudioDevice() = default;

    // Opens the default output device and starts the callback.
    // Returns an se_result code; `err` gets a description on failure.
    virtual int32_t start(const Config& config, Callback* callback, std::string& err) = 0;
    // Stops the stream. When this returns the callback is no longer running
    // and will not be called again.
    virtual int32_t stop() = 0;
    virtual bool is_running() const = 0;
};

std::unique_ptr<AudioDevice> create_platform_audio_device();

// Nominal sample rate of the default output device, or 0 if unknown.
double query_default_output_sample_rate();

} // namespace saine
