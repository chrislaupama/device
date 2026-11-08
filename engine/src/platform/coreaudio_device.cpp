// macOS Core Audio backend: AUHAL default-output unit.
//
// We hand the output unit float32, non-interleaved, N-channel audio at the
// *engine* sample rate. If the hardware runs at a different nominal rate the
// output unit's built-in AUConverter resamples (adds a little latency/CPU).
// TODO: optionally switch the device's nominal rate, select non-default
// devices, and follow default-device changes.
#if defined(__APPLE__)

#include "audio_device.hpp"
#include "saine_engine.h"

#include <AudioToolbox/AudioToolbox.h>
#include <CoreAudio/CoreAudio.h>

#include <atomic>

namespace saine {
namespace {

constexpr uint32_t kMaxDeviceChannels = 16;

std::string os_error(const char* what, OSStatus st) {
    return std::string(what) + " failed (OSStatus " + std::to_string(static_cast<int>(st)) + ")";
}

class CoreAudioDevice final : public AudioDevice {
public:
    ~CoreAudioDevice() override { stop(); }

    int32_t start(const Config& config, Callback* callback, std::string& err) override {
        if (unit_ != nullptr) {
            err = "device already started";
            return SE_ERR_STATE;
        }
        if (callback == nullptr || config.channels == 0 || config.channels > kMaxDeviceChannels) {
            err = "invalid device configuration";
            return SE_ERR_INVALID_ARG;
        }
        callback_ = callback;
        channels_ = config.channels;

        AudioComponentDescription desc{};
        desc.componentType = kAudioUnitType_Output;
        desc.componentSubType = kAudioUnitSubType_DefaultOutput;
        desc.componentManufacturer = kAudioUnitManufacturer_Apple;
        AudioComponent comp = AudioComponentFindNext(nullptr, &desc);
        if (comp == nullptr) {
            err = "no default output AudioUnit found";
            return SE_ERR_DEVICE;
        }
        OSStatus st = AudioComponentInstanceNew(comp, &unit_);
        if (st != noErr || unit_ == nullptr) {
            unit_ = nullptr;
            err = os_error("AudioComponentInstanceNew", st);
            return SE_ERR_DEVICE;
        }

        AudioStreamBasicDescription fmt{};
        fmt.mSampleRate = config.sample_rate;
        fmt.mFormatID = kAudioFormatLinearPCM;
        fmt.mFormatFlags = kAudioFormatFlagIsFloat | kAudioFormatFlagIsPacked | kAudioFormatFlagIsNonInterleaved;
        fmt.mBytesPerPacket = sizeof(float);
        fmt.mFramesPerPacket = 1;
        fmt.mBytesPerFrame = sizeof(float);
        fmt.mChannelsPerFrame = config.channels;
        fmt.mBitsPerChannel = 32;
        st = AudioUnitSetProperty(unit_, kAudioUnitProperty_StreamFormat, kAudioUnitScope_Input, 0, &fmt,
                                  sizeof fmt);
        if (st != noErr) return fail(os_error("setting stream format", st), err);

        AURenderCallbackStruct cb{};
        cb.inputProc = &CoreAudioDevice::render_proc;
        cb.inputProcRefCon = this;
        st = AudioUnitSetProperty(unit_, kAudioUnitProperty_SetRenderCallback, kAudioUnitScope_Input, 0, &cb,
                                  sizeof cb);
        if (st != noErr) return fail(os_error("setting render callback", st), err);

        st = AudioUnitInitialize(unit_);
        if (st != noErr) return fail(os_error("AudioUnitInitialize", st), err);
        initialized_ = true;

        st = AudioOutputUnitStart(unit_);
        if (st != noErr) return fail(os_error("AudioOutputUnitStart", st), err);
        running_.store(true, std::memory_order_release);
        return SE_OK;
    }

    int32_t stop() override {
        if (unit_ == nullptr) return SE_OK;
        if (running_.load(std::memory_order_acquire)) AudioOutputUnitStop(unit_); // synchronous w.r.t. the IO proc
        running_.store(false, std::memory_order_release);
        if (initialized_) AudioUnitUninitialize(unit_);
        initialized_ = false;
        AudioComponentInstanceDispose(unit_); // guarantees no further callbacks
        unit_ = nullptr;
        return SE_OK;
    }

    bool is_running() const override { return running_.load(std::memory_order_acquire); }

private:
    int32_t fail(std::string msg, std::string& err) {
        err = std::move(msg);
        stop();
        return SE_ERR_DEVICE;
    }

    static OSStatus render_proc(void* ref, AudioUnitRenderActionFlags* /*flags*/, const AudioTimeStamp* /*ts*/,
                                UInt32 /*bus*/, UInt32 frames, AudioBufferList* io) {
        auto* self = static_cast<CoreAudioDevice*>(ref);
        float* ptrs[kMaxDeviceChannels];
        uint32_t n = 0;
        for (UInt32 b = 0; b < io->mNumberBuffers && n < kMaxDeviceChannels; ++b) {
            AudioBuffer& buf = io->mBuffers[b];
            // Non-interleaved: one channel per buffer. Be defensive anyway.
            if (buf.mNumberChannels != 1 || buf.mData == nullptr ||
                buf.mDataByteSize < frames * sizeof(float))
                continue;
            ptrs[n++] = static_cast<float*>(buf.mData);
        }
        self->callback_->render(ptrs, n, frames);
        return noErr;
    }

    AudioComponentInstance unit_ = nullptr;
    bool initialized_ = false;
    std::atomic<bool> running_{false};
    Callback* callback_ = nullptr;
    uint32_t channels_ = 0;
};

} // namespace

std::unique_ptr<AudioDevice> create_platform_audio_device() { return std::make_unique<CoreAudioDevice>(); }

double query_default_output_sample_rate() {
    AudioObjectPropertyAddress addr{kAudioHardwarePropertyDefaultOutputDevice, kAudioObjectPropertyScopeGlobal,
                                    kAudioObjectPropertyElementMain};
    AudioDeviceID dev = kAudioObjectUnknown;
    UInt32 size = sizeof dev;
    if (AudioObjectGetPropertyData(kAudioObjectSystemObject, &addr, 0, nullptr, &size, &dev) != noErr ||
        dev == kAudioObjectUnknown)
        return 0.0;
    Float64 rate = 0.0;
    size = sizeof rate;
    addr.mSelector = kAudioDevicePropertyNominalSampleRate;
    if (AudioObjectGetPropertyData(dev, &addr, 0, nullptr, &size, &rate) != noErr) return 0.0;
    return rate;
}

} // namespace saine

#endif // __APPLE__
