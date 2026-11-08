// macOS Core Audio backend.
//
// Output only (the default, and whenever no input is configured):
//   - no output device named: AUHAL kAudioUnitSubType_DefaultOutput, which
//     follows the system default device (the historical behaviour);
//   - output device named: kAudioUnitSubType_HALOutput bound to it.
// We hand the unit float32 non-interleaved audio at the *engine* rate; if
// the hardware runs at another nominal rate the unit's converter resamples.
//
// Duplex (input configured): one HALOutput unit with input enabled, pulling
// the input bus (AudioUnitRender on element 1) inside the output render
// callback, so input and output of one IO cycle arrive together.
//   - input and output are the same device: the unit runs on it directly;
//   - different devices (e.g. MacBook microphone + speakers, two
//     independent clocks): we create a *private, non-persistent aggregate
//     device* (kAudioAggregateDeviceIsPrivateKey) whose main/clock
//     sub-device is the output and whose input sub-device has drift
//     compensation enabled. The HAL then resamples the input device onto
//     the output's clock. It is visible only to this process and destroyed
//     on stop (and by the system when the process exits).
// AUHAL does not convert the sample rate on its input side, so with an input
// the hardware is switched to the engine rate (validate() rejects devices
// that cannot run at it).
//
// Input is opened only when Config::input_enabled is set (se_device_configure
// with input_enabled). If opening it fails (permission denied, aggregate
// creation fails, device vanished...), the device restarts output-only and
// status().input_error says why.
#if defined(__APPLE__)

#include "audio_device.hpp"
#include "saine_engine.h"

#include <AudioToolbox/AudioToolbox.h>
#include <CoreAudio/CoreAudio.h>
#include <CoreFoundation/CoreFoundation.h>
#include <dlfcn.h>
#include <unistd.h>

#include <algorithm>
#include <atomic>
#include <chrono>
#include <cmath>
#include <cstring>
#include <mutex>
#include <thread>

namespace saine {
namespace {

constexpr uint32_t kMaxDeviceChannels = 16;
constexpr uint32_t kDefaultMaxFrames = 4096;

std::string os_error(const char* what, OSStatus st) {
    return std::string(what) + " failed (OSStatus " + std::to_string(static_cast<int>(st)) + ")";
}

AudioObjectPropertyAddress prop(AudioObjectPropertySelector sel,
                                AudioObjectPropertyScope scope = kAudioObjectPropertyScopeGlobal) {
    return AudioObjectPropertyAddress{sel, scope, kAudioObjectPropertyElementMain};
}

template <class T>
bool get_prop(AudioObjectID id, AudioObjectPropertySelector sel, AudioObjectPropertyScope scope, T& out) {
    const AudioObjectPropertyAddress a = prop(sel, scope);
    UInt32 size = sizeof(T);
    return AudioObjectGetPropertyData(id, &a, 0, nullptr, &size, &out) == noErr && size == sizeof(T);
}

std::string cf_to_utf8(CFStringRef s) {
    if (s == nullptr) return {};
    const CFIndex len = CFStringGetLength(s);
    const CFIndex max = CFStringGetMaximumSizeForEncoding(len, kCFStringEncodingUTF8) + 1;
    std::string out(static_cast<std::size_t>(max), '\0');
    if (!CFStringGetCString(s, out.data(), max, kCFStringEncodingUTF8)) return {};
    out.resize(std::strlen(out.c_str()));
    return out;
}

std::string device_string(AudioObjectID id, AudioObjectPropertySelector sel) {
    CFStringRef s = nullptr;
    if (!get_prop(id, sel, kAudioObjectPropertyScopeGlobal, s) || s == nullptr) return {};
    std::string out = cf_to_utf8(s);
    CFRelease(s);
    return out;
}

std::string device_uid(AudioDeviceID id) { return device_string(id, kAudioDevicePropertyDeviceUID); }
std::string device_name(AudioDeviceID id) { return device_string(id, kAudioObjectPropertyName); }

std::vector<AudioDeviceID> all_devices() {
    const AudioObjectPropertyAddress a = prop(kAudioHardwarePropertyDevices);
    UInt32 size = 0;
    if (AudioObjectGetPropertyDataSize(kAudioObjectSystemObject, &a, 0, nullptr, &size) != noErr) return {};
    std::vector<AudioDeviceID> ids(size / sizeof(AudioDeviceID));
    if (ids.empty()) return ids;
    if (AudioObjectGetPropertyData(kAudioObjectSystemObject, &a, 0, nullptr, &size, ids.data()) != noErr) return {};
    ids.resize(size / sizeof(AudioDeviceID));
    return ids;
}

uint32_t channel_count(AudioDeviceID id, AudioObjectPropertyScope scope) {
    const AudioObjectPropertyAddress a = prop(kAudioDevicePropertyStreamConfiguration, scope);
    UInt32 size = 0;
    if (AudioObjectGetPropertyDataSize(id, &a, 0, nullptr, &size) != noErr || size < sizeof(AudioBufferList)) return 0;
    std::vector<unsigned char> storage(size);
    auto* abl = reinterpret_cast<AudioBufferList*>(storage.data());
    if (AudioObjectGetPropertyData(id, &a, 0, nullptr, &size, abl) != noErr) return 0;
    uint32_t n = 0;
    for (UInt32 b = 0; b < abl->mNumberBuffers; ++b) n += abl->mBuffers[b].mNumberChannels;
    return n;
}

std::vector<AudioValueRange> rate_ranges(AudioDeviceID id) {
    const AudioObjectPropertyAddress a = prop(kAudioDevicePropertyAvailableNominalSampleRates);
    UInt32 size = 0;
    if (AudioObjectGetPropertyDataSize(id, &a, 0, nullptr, &size) != noErr) return {};
    std::vector<AudioValueRange> r(size / sizeof(AudioValueRange));
    if (r.empty()) return r;
    if (AudioObjectGetPropertyData(id, &a, 0, nullptr, &size, r.data()) != noErr) return {};
    r.resize(size / sizeof(AudioValueRange));
    return r;
}

bool rate_supported(AudioDeviceID id, double rate) {
    for (const AudioValueRange& r : rate_ranges(id))
        if (rate >= r.mMinimum - 0.5 && rate <= r.mMaximum + 0.5) return true;
    return false;
}

double nominal_rate(AudioDeviceID id) {
    Float64 r = 0.0;
    return get_prop(id, kAudioDevicePropertyNominalSampleRate, kAudioObjectPropertyScopeGlobal, r) ? r : 0.0;
}

AudioDeviceID default_device(bool input) {
    AudioDeviceID dev = kAudioObjectUnknown;
    get_prop(kAudioObjectSystemObject,
             input ? kAudioHardwarePropertyDefaultInputDevice : kAudioHardwarePropertyDefaultOutputDevice,
             kAudioObjectPropertyScopeGlobal, dev);
    return dev;
}

AudioDeviceID find_device(const std::string& uid) {
    for (AudioDeviceID id : all_devices())
        if (device_uid(id) == uid) return id;
    return kAudioObjectUnknown;
}

AudioDeviceID resolve(const std::string& uid, bool input) {
    return uid.empty() ? default_device(input) : find_device(uid);
}

std::string rate_str(double r) { return std::to_string(static_cast<long long>(std::llround(r))) + " Hz"; }

// Sets the device's nominal rate and waits (up to 2 s) until it reports it.
int32_t set_nominal_rate(AudioDeviceID id, double rate, std::string& err) {
    if (std::fabs(nominal_rate(id) - rate) < 0.5) return SE_OK;
    if (!rate_supported(id, rate)) {
        err = device_name(id) + " does not support " + rate_str(rate);
        return SE_ERR_INVALID_ARG;
    }
    const AudioObjectPropertyAddress a = prop(kAudioDevicePropertyNominalSampleRate);
    const Float64 r = rate;
    const OSStatus st = AudioObjectSetPropertyData(id, &a, 0, nullptr, sizeof r, &r);
    if (st != noErr) {
        err = os_error(("setting " + device_name(id) + " to " + rate_str(rate)).c_str(), st);
        return SE_ERR_DEVICE;
    }
    for (int i = 0; i < 100; ++i) {
        if (std::fabs(nominal_rate(id) - rate) < 0.5) return SE_OK;
        std::this_thread::sleep_for(std::chrono::milliseconds(20));
    }
    err = device_name(id) + " did not switch to " + rate_str(rate);
    return SE_ERR_DEVICE;
}

bool buffer_range(AudioDeviceID id, double& lo, double& hi) {
    AudioValueRange r{};
    if (!get_prop(id, kAudioDevicePropertyBufferFrameSizeRange, kAudioObjectPropertyScopeGlobal, r)) return false;
    lo = r.mMinimum;
    hi = r.mMaximum;
    return true;
}

uint32_t buffer_frames(AudioDeviceID id) {
    UInt32 n = 0;
    return get_prop(id, kAudioDevicePropertyBufferFrameSize, kAudioObjectPropertyScopeGlobal, n) ? n : 0;
}

// Device latency + safety offset + first stream's latency + one IO buffer
// for one direction (the usual Core Audio round-trip accounting).
uint32_t side_latency(AudioDeviceID id, AudioObjectPropertyScope scope) {
    UInt32 lat = 0, safety = 0, stream_lat = 0;
    get_prop(id, kAudioDevicePropertyLatency, scope, lat);
    get_prop(id, kAudioDevicePropertySafetyOffset, scope, safety);
    const AudioObjectPropertyAddress a = prop(kAudioDevicePropertyStreams, scope);
    UInt32 size = 0;
    if (AudioObjectGetPropertyDataSize(id, &a, 0, nullptr, &size) == noErr && size >= sizeof(AudioStreamID)) {
        std::vector<AudioStreamID> streams(size / sizeof(AudioStreamID));
        if (AudioObjectGetPropertyData(id, &a, 0, nullptr, &size, streams.data()) == noErr && !streams.empty())
            get_prop(streams[0], kAudioStreamPropertyLatency, kAudioObjectPropertyScopeGlobal, stream_lat);
    }
    return lat + safety + stream_lat + buffer_frames(id);
}

// AVCaptureDevice authorizationStatusForMediaType:AVMediaTypeAudio, read
// through the Objective-C runtime (no ObjC compile, no link dependency).
// Reading the status never prompts. -1 unknown, 0 not determined,
// 1 restricted, 2 denied, 3 authorized.
long microphone_authorization() {
    void* av = dlopen("/System/Library/Frameworks/AVFoundation.framework/AVFoundation", RTLD_LAZY | RTLD_LOCAL);
    if (av == nullptr) return -1;
    using GetClass = void* (*)(const char*);
    using RegisterSel = void* (*)(const char*);
    using MsgSend = long (*)(void*, void*, void*);
    auto get_class = reinterpret_cast<GetClass>(dlsym(RTLD_DEFAULT, "objc_getClass"));
    auto reg_sel = reinterpret_cast<RegisterSel>(dlsym(RTLD_DEFAULT, "sel_registerName"));
    auto msg_send = reinterpret_cast<MsgSend>(dlsym(RTLD_DEFAULT, "objc_msgSend"));
    auto* media = static_cast<void* const*>(dlsym(av, "AVMediaTypeAudio"));
    if (get_class == nullptr || reg_sel == nullptr || msg_send == nullptr || media == nullptr || *media == nullptr)
        return -1;
    void* cls = get_class("AVCaptureDevice");
    if (cls == nullptr) return -1;
    return msg_send(cls, reg_sel("authorizationStatusForMediaType:"), *media);
}

// Lets the HAL deliver its notifications on its own thread (we have no
// main run loop guarantee). Process-local HAL client setting.
void use_hal_notification_thread() {
    static std::once_flag once;
    std::call_once(once, [] {
        CFRunLoopRef rl = nullptr;
        const AudioObjectPropertyAddress a = prop(kAudioHardwarePropertyRunLoop);
        AudioObjectSetPropertyData(kAudioObjectSystemObject, &a, 0, nullptr, sizeof rl, &rl);
    });
}

void dict_set_int(CFMutableDictionaryRef d, CFStringRef key, int v) {
    CFNumberRef n = CFNumberCreate(nullptr, kCFNumberIntType, &v);
    CFDictionarySetValue(d, key, n);
    CFRelease(n);
}

void dict_set_str(CFMutableDictionaryRef d, CFStringRef key, const std::string& v) {
    CFStringRef s = CFStringCreateWithCString(nullptr, v.c_str(), kCFStringEncodingUTF8);
    CFDictionarySetValue(d, key, s);
    CFRelease(s);
}

CFMutableDictionaryRef new_dict() {
    return CFDictionaryCreateMutable(nullptr, 0, &kCFTypeDictionaryKeyCallBacks, &kCFTypeDictionaryValueCallBacks);
}

class CoreAudioDevice final : public AudioDevice {
public:
    ~CoreAudioDevice() override { stop(); }

    int32_t validate(const Config& c, std::string& err) const override {
        const AudioDeviceID out = resolve(c.output_uid, false);
        if (out == kAudioObjectUnknown) {
            err = c.output_uid.empty() ? "no default output device" : "output device '" + c.output_uid + "' not found";
            return SE_ERR_NOT_FOUND;
        }
        if (channel_count(out, kAudioObjectPropertyScopeOutput) == 0) {
            err = device_name(out) + " has no output channels";
            return SE_ERR_INVALID_ARG;
        }
        if (!std::isfinite(c.device_rate) || c.device_rate < 0.0) {
            err = "sample_rate must be finite and >= 0";
            return SE_ERR_INVALID_ARG;
        }
        AudioDeviceID in = kAudioObjectUnknown;
        if (c.input_enabled) {
            in = resolve(c.input_uid, true);
            if (in == kAudioObjectUnknown) {
                err = c.input_uid.empty() ? "no default input device" : "input device '" + c.input_uid + "' not found";
                return SE_ERR_NOT_FOUND;
            }
            if (channel_count(in, kAudioObjectPropertyScopeInput) == 0) {
                err = device_name(in) + " has no input channels";
                return SE_ERR_INVALID_ARG;
            }
            if (c.device_rate > 0.0 && std::fabs(c.device_rate - c.sample_rate) >= 0.5) {
                err = "with an input the device must run at the engine rate (" + rate_str(c.sample_rate) +
                      "); the input path does not resample";
                return SE_ERR_INVALID_ARG;
            }
            for (AudioDeviceID d : {out, in})
                if (!rate_supported(d, c.sample_rate)) {
                    err = device_name(d) + " cannot run at the engine rate " + rate_str(c.sample_rate) +
                          ", which recording needs";
                    return SE_ERR_INVALID_ARG;
                }
        } else if (c.device_rate > 0.0 && !rate_supported(out, c.device_rate)) {
            err = device_name(out) + " does not support " + rate_str(c.device_rate);
            return SE_ERR_INVALID_ARG;
        }
        if (c.buffer_size != 0) {
            if (c.buffer_size < 16 || c.buffer_size > 4096) {
                err = "buffer_size must be 0 or 16..4096";
                return SE_ERR_INVALID_ARG;
            }
            for (AudioDeviceID d : {out, in}) {
                double lo = 0, hi = 0;
                if (d != kAudioObjectUnknown && buffer_range(d, lo, hi) &&
                    (c.buffer_size < lo || c.buffer_size > hi)) {
                    err = device_name(d) + " supports buffer sizes " + std::to_string(static_cast<long>(lo)) + ".." +
                          std::to_string(static_cast<long>(hi));
                    return SE_ERR_INVALID_ARG;
                }
            }
        }
        return SE_OK;
    }

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
        status_ = Status{};
        std::string input_error;
        if (config.input_enabled) {
            const int32_t rc = open(config, true, input_error);
            if (rc == SE_OK) return SE_OK;
            teardown();
            if (input_error.empty()) input_error = "input could not be opened";
        }
        const int32_t rc = open(config, false, err);
        if (rc != SE_OK) {
            teardown();
            status_ = Status{};
            if (!input_error.empty()) err += " (input: " + input_error + ")";
            return rc;
        }
        status_.input_error = input_error;
        return SE_OK;
    }

    int32_t stop() override {
        teardown();
        return SE_OK;
    }

    bool is_running() const override { return running_.load(std::memory_order_acquire); }
    Status status() const override { return status_; }

private:
    int32_t open(const Config& c, bool with_input, std::string& err) {
        int32_t rc = validate(c, err);
        if (rc != SE_OK) return rc;
        const AudioDeviceID out = resolve(c.output_uid, false);
        AudioDeviceID in = kAudioObjectUnknown;
        AudioDeviceID run = out;
        const bool default_unit = !with_input && c.output_uid.empty();

        if (with_input) {
            const long auth = microphone_authorization();
            if (auth == 1 || auth == 2) {
                err = "microphone access is denied for this application (System Settings > Privacy & Security > "
                      "Microphone)";
                return SE_ERR_DEVICE;
            }
            in = resolve(c.input_uid, true);
            if ((rc = set_nominal_rate(out, c.sample_rate, err)) != SE_OK) return rc;
            if (in != out) {
                if ((rc = set_nominal_rate(in, c.sample_rate, err)) != SE_OK) return rc;
                if ((rc = create_aggregate(out, in, err)) != SE_OK) return rc;
                run = aggregate_;
                if ((rc = set_nominal_rate(run, c.sample_rate, err)) != SE_OK) return rc;
            }
        } else if (c.device_rate > 0.0) {
            if ((rc = set_nominal_rate(out, c.device_rate, err)) != SE_OK) return rc;
        }
        if (c.buffer_size != 0) {
            const AudioObjectPropertyAddress a = prop(kAudioDevicePropertyBufferFrameSize);
            const UInt32 n = c.buffer_size;
            const OSStatus st = AudioObjectSetPropertyData(run, &a, 0, nullptr, sizeof n, &n);
            if (st != noErr) {
                err = os_error("setting the buffer size", st);
                return SE_ERR_DEVICE;
            }
        }

        AudioComponentDescription desc{};
        desc.componentType = kAudioUnitType_Output;
        desc.componentSubType = default_unit ? kAudioUnitSubType_DefaultOutput : kAudioUnitSubType_HALOutput;
        desc.componentManufacturer = kAudioUnitManufacturer_Apple;
        AudioComponent comp = AudioComponentFindNext(nullptr, &desc);
        if (comp == nullptr) {
            err = "no output AudioUnit found";
            return SE_ERR_DEVICE;
        }
        OSStatus st = AudioComponentInstanceNew(comp, &unit_);
        if (st != noErr || unit_ == nullptr) {
            unit_ = nullptr;
            err = os_error("AudioComponentInstanceNew", st);
            return SE_ERR_DEVICE;
        }
        if (!default_unit) {
            if (with_input) {
                const UInt32 one = 1;
                st = AudioUnitSetProperty(unit_, kAudioOutputUnitProperty_EnableIO, kAudioUnitScope_Input, 1, &one,
                                          sizeof one);
                if (st != noErr) return fail(os_error("enabling input", st), err);
            }
            st = AudioUnitSetProperty(unit_, kAudioOutputUnitProperty_CurrentDevice, kAudioUnitScope_Global, 0, &run,
                                      sizeof run);
            if (st != noErr) return fail(os_error("selecting the device", st), err);
        }

        AudioStreamBasicDescription fmt{};
        fmt.mSampleRate = c.sample_rate;
        fmt.mFormatID = kAudioFormatLinearPCM;
        fmt.mFormatFlags = kAudioFormatFlagIsFloat | kAudioFormatFlagIsPacked | kAudioFormatFlagIsNonInterleaved;
        fmt.mBytesPerPacket = sizeof(float);
        fmt.mFramesPerPacket = 1;
        fmt.mBytesPerFrame = sizeof(float);
        fmt.mChannelsPerFrame = c.channels;
        fmt.mBitsPerChannel = 32;
        st = AudioUnitSetProperty(unit_, kAudioUnitProperty_StreamFormat, kAudioUnitScope_Input, 0, &fmt, sizeof fmt);
        if (st != noErr) return fail(os_error("setting stream format", st), err);

        uint32_t in_channels = 0;
        if (with_input) {
            AudioStreamBasicDescription hw{};
            UInt32 size = sizeof hw;
            st = AudioUnitGetProperty(unit_, kAudioUnitProperty_StreamFormat, kAudioUnitScope_Input, 1, &hw, &size);
            if (st != noErr) return fail(os_error("reading the input format", st), err);
            in_channels = std::min<uint32_t>(hw.mChannelsPerFrame, kMaxDeviceInputChannels);
            if (in_channels == 0) return fail("the input device reports no input channels", err);
            if (std::fabs(hw.mSampleRate - c.sample_rate) >= 0.5)
                return fail("the input runs at " + rate_str(hw.mSampleRate) + ", the engine at " +
                                rate_str(c.sample_rate),
                            err);
            AudioStreamBasicDescription in_fmt = fmt;
            in_fmt.mChannelsPerFrame = in_channels;
            st = AudioUnitSetProperty(unit_, kAudioUnitProperty_StreamFormat, kAudioUnitScope_Output, 1, &in_fmt,
                                      sizeof in_fmt);
            if (st != noErr) return fail(os_error("setting the input format", st), err);
        }

        UInt32 max_frames = std::max<UInt32>(kDefaultMaxFrames, 2 * c.buffer_size);
        AudioUnitSetProperty(unit_, kAudioUnitProperty_MaximumFramesPerSlice, kAudioUnitScope_Global, 0, &max_frames,
                             sizeof max_frames);

        AURenderCallbackStruct cb{};
        cb.inputProc = &CoreAudioDevice::render_proc;
        cb.inputProcRefCon = this;
        st = AudioUnitSetProperty(unit_, kAudioUnitProperty_SetRenderCallback, kAudioUnitScope_Input, 0, &cb,
                                  sizeof cb);
        if (st != noErr) return fail(os_error("setting render callback", st), err);

        st = AudioUnitInitialize(unit_);
        if (st != noErr) return fail(os_error("AudioUnitInitialize", st), err);
        initialized_ = true;

        if (with_input) {
            UInt32 size = sizeof max_frames;
            if (AudioUnitGetProperty(unit_, kAudioUnitProperty_MaximumFramesPerSlice, kAudioUnitScope_Global, 0,
                                     &max_frames, &size) != noErr)
                max_frames = kDefaultMaxFrames;
            max_frames_ = max_frames;
            in_storage_.assign(static_cast<std::size_t>(in_channels) * max_frames, 0.0f);
            in_abl_storage_.assign(offsetof(AudioBufferList, mBuffers) + sizeof(AudioBuffer) * in_channels, 0);
            in_abl_ = reinterpret_cast<AudioBufferList*>(in_abl_storage_.data());
            in_abl_->mNumberBuffers = in_channels;
            in_channels_ = in_channels;
        }

        st = AudioOutputUnitStart(unit_);
        if (st != noErr) return fail(os_error("AudioOutputUnitStart", st), err);
        running_.store(true, std::memory_order_release);

        // Status. For an aggregate, a direction's latency is the larger of
        // what the aggregate and the sub-device report.
        status_.output_name = device_name(out);
        status_.device_rate = nominal_rate(run);
        status_.buffer_size = buffer_frames(run);
        status_.output_channels = channel_count(run, kAudioObjectPropertyScopeOutput);
        status_.output_latency = side_latency(run, kAudioObjectPropertyScopeOutput);
        if (run != out) status_.output_latency = std::max(status_.output_latency, side_latency(out, kAudioObjectPropertyScopeOutput));
        if (with_input) {
            status_.input_open = true;
            status_.input_name = device_name(in);
            status_.input_channels = in_channels;
            status_.input_latency = side_latency(run, kAudioObjectPropertyScopeInput);
            if (run != in) status_.input_latency = std::max(status_.input_latency, side_latency(in, kAudioObjectPropertyScopeInput));
        }
        return SE_OK;
    }

    int32_t create_aggregate(AudioDeviceID out, AudioDeviceID in, std::string& err) {
        use_hal_notification_thread();
        static std::atomic<unsigned> counter{0};
        const std::string uid = "com.sainestudio.engine.duplex." + std::to_string(getpid()) + "." +
                                std::to_string(counter.fetch_add(1));
        const std::string out_uid = device_uid(out);
        const std::string in_uid = device_uid(in);

        CFMutableDictionaryRef desc = new_dict();
        dict_set_str(desc, CFSTR(kAudioAggregateDeviceUIDKey), uid);
        dict_set_str(desc, CFSTR(kAudioAggregateDeviceNameKey), "Saine Studio Duplex");
        dict_set_int(desc, CFSTR(kAudioAggregateDeviceIsPrivateKey), 1);
        dict_set_int(desc, CFSTR(kAudioAggregateDeviceIsStackedKey), 0);
        CFMutableArrayRef subs = CFArrayCreateMutable(nullptr, 0, &kCFTypeArrayCallBacks);
        CFMutableDictionaryRef out_sub = new_dict();
        dict_set_str(out_sub, CFSTR(kAudioSubDeviceUIDKey), out_uid);
        dict_set_int(out_sub, CFSTR(kAudioSubDeviceDriftCompensationKey), 0);
        CFMutableDictionaryRef in_sub = new_dict();
        dict_set_str(in_sub, CFSTR(kAudioSubDeviceUIDKey), in_uid);
        dict_set_int(in_sub, CFSTR(kAudioSubDeviceDriftCompensationKey), 1);
        dict_set_int(in_sub, CFSTR(kAudioSubDeviceDriftCompensationQualityKey),
                     static_cast<int>(kAudioSubDeviceDriftCompensationHighQuality));
        CFArrayAppendValue(subs, out_sub); // output first: aggregate output channels 0/1 = the output device
        CFArrayAppendValue(subs, in_sub);
        CFDictionarySetValue(desc, CFSTR(kAudioAggregateDeviceSubDeviceListKey), subs);
        dict_set_str(desc, CFSTR(kAudioAggregateDeviceMainSubDeviceKey), out_uid); // clock = output device
        AudioDeviceID agg = kAudioObjectUnknown;
        const OSStatus st = AudioHardwareCreateAggregateDevice(desc, &agg);
        CFRelease(out_sub);
        CFRelease(in_sub);
        CFRelease(subs);
        CFRelease(desc);
        if (st != noErr || agg == kAudioObjectUnknown) {
            err = os_error("creating the duplex aggregate device", st);
            return SE_ERR_DEVICE;
        }
        aggregate_ = agg;
        // The HAL publishes the new device asynchronously.
        for (int i = 0; i < 200; ++i) {
            if (channel_count(agg, kAudioObjectPropertyScopeInput) > 0 &&
                channel_count(agg, kAudioObjectPropertyScopeOutput) > 0)
                return SE_OK;
            std::this_thread::sleep_for(std::chrono::milliseconds(10));
        }
        err = "the duplex aggregate device did not become ready";
        return SE_ERR_DEVICE;
    }

    int32_t fail(std::string msg, std::string& err) {
        err = std::move(msg);
        return SE_ERR_DEVICE; // caller tears down
    }

    void teardown() {
        if (unit_ != nullptr) {
            if (running_.load(std::memory_order_acquire)) AudioOutputUnitStop(unit_); // synchronous w.r.t. the IO proc
            running_.store(false, std::memory_order_release);
            if (initialized_) AudioUnitUninitialize(unit_);
            initialized_ = false;
            AudioComponentInstanceDispose(unit_); // guarantees no further callbacks
            unit_ = nullptr;
        }
        running_.store(false, std::memory_order_release);
        if (aggregate_ != kAudioObjectUnknown) {
            AudioHardwareDestroyAggregateDevice(aggregate_);
            aggregate_ = kAudioObjectUnknown;
        }
        in_channels_ = 0;
        in_abl_ = nullptr;
        in_abl_storage_.clear();
        in_storage_.clear();
    }

    static OSStatus render_proc(void* ref, AudioUnitRenderActionFlags* /*flags*/, const AudioTimeStamp* ts,
                                UInt32 /*bus*/, UInt32 frames, AudioBufferList* io) {
        auto* self = static_cast<CoreAudioDevice*>(ref);
        const float* in_ptrs[kMaxDeviceInputChannels];
        uint32_t n_in = 0;
        if (self->in_channels_ > 0 && frames <= self->max_frames_) {
            AudioBufferList* abl = self->in_abl_;
            for (uint32_t c = 0; c < self->in_channels_; ++c) {
                abl->mBuffers[c].mNumberChannels = 1;
                abl->mBuffers[c].mDataByteSize = frames * sizeof(float);
                abl->mBuffers[c].mData = self->in_storage_.data() + static_cast<std::size_t>(c) * self->max_frames_;
            }
            AudioUnitRenderActionFlags in_flags = 0;
            if (AudioUnitRender(self->unit_, &in_flags, ts, 1, frames, abl) == noErr) {
                for (uint32_t c = 0; c < self->in_channels_; ++c)
                    in_ptrs[c] = static_cast<const float*>(abl->mBuffers[c].mData);
                n_in = self->in_channels_;
            }
        }
        float* ptrs[kMaxDeviceChannels];
        uint32_t n = 0;
        for (UInt32 b = 0; b < io->mNumberBuffers && n < kMaxDeviceChannels; ++b) {
            AudioBuffer& buf = io->mBuffers[b];
            // Non-interleaved: one channel per buffer. Be defensive anyway.
            if (buf.mNumberChannels != 1 || buf.mData == nullptr || buf.mDataByteSize < frames * sizeof(float))
                continue;
            ptrs[n++] = static_cast<float*>(buf.mData);
        }
        self->callback_->render(ptrs, n, in_ptrs, n_in, frames);
        return noErr;
    }

    AudioComponentInstance unit_ = nullptr;
    bool initialized_ = false;
    std::atomic<bool> running_{false};
    Callback* callback_ = nullptr;
    AudioDeviceID aggregate_ = kAudioObjectUnknown;
    // Input pull buffers (allocated before start, read by the IO thread).
    uint32_t in_channels_ = 0;
    uint32_t max_frames_ = 0;
    std::vector<float> in_storage_;
    std::vector<unsigned char> in_abl_storage_;
    AudioBufferList* in_abl_ = nullptr;
    Status status_;
};

} // namespace

std::unique_ptr<AudioDevice> create_platform_audio_device() { return std::make_unique<CoreAudioDevice>(); }

int32_t list_audio_devices(std::vector<DeviceInfo>& out, std::string& /*err*/) {
    out.clear();
    const AudioDeviceID def_in = default_device(true);
    const AudioDeviceID def_out = default_device(false);
    for (AudioDeviceID id : all_devices()) {
        DeviceInfo d;
        d.uid = device_uid(id);
        if (d.uid.empty()) continue;
        d.name = device_name(id);
        d.input_channels = channel_count(id, kAudioObjectPropertyScopeInput);
        d.output_channels = channel_count(id, kAudioObjectPropertyScopeOutput);
        for (double r : kStandardRates)
            if (rate_supported(id, r)) d.sample_rates.push_back(r);
        d.is_default_input = id == def_in && d.input_channels > 0;
        d.is_default_output = id == def_out && d.output_channels > 0;
        out.push_back(std::move(d));
    }
    return SE_OK;
}

double query_default_output_sample_rate() {
    const AudioDeviceID dev = default_device(false);
    return dev == kAudioObjectUnknown ? 0.0 : nominal_rate(dev);
}

// Exposed for engine/tools/record_check (manual tool): microphone
// permission status without prompting.
long coreaudio_microphone_authorization() { return microphone_authorization(); }

} // namespace saine

#endif // __APPLE__
