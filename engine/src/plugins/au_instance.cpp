// Audio Unit (v2 API) hosting. macOS only.
#include "plugin_instance.hpp"

#if defined(__APPLE__)

#include "au_util.hpp"
#include "plugin_editor.hpp"

#include <AudioToolbox/AudioToolbox.h>

#include <algorithm>
#include <cmath>
#include <cstring>

namespace saine {

bool parse_au_id(const std::string& id, AudioComponentDescription& d) {
    // "type subtype manufacturer": three four-char codes separated by spaces.
    if (id.size() != 14 || id[4] != ' ' || id[9] != ' ') return false;
    const auto code = [&](std::size_t at) {
        return (static_cast<uint32_t>(static_cast<uint8_t>(id[at])) << 24) |
               (static_cast<uint32_t>(static_cast<uint8_t>(id[at + 1])) << 16) |
               (static_cast<uint32_t>(static_cast<uint8_t>(id[at + 2])) << 8) |
               static_cast<uint32_t>(static_cast<uint8_t>(id[at + 3]));
    };
    d = AudioComponentDescription{};
    d.componentType = code(0);
    d.componentSubType = code(5);
    d.componentManufacturer = code(10);
    return true;
}

std::string four_cc(uint32_t c) {
    std::string s(4, ' ');
    for (int i = 0; i < 4; ++i) s[static_cast<std::size_t>(i)] = static_cast<char>((c >> (24 - 8 * i)) & 0xff);
    return s;
}

std::string au_id_string(const AudioComponentDescription& d) {
    return four_cc(d.componentType) + " " + four_cc(d.componentSubType) + " " + four_cc(d.componentManufacturer);
}

std::string cf_to_string(CFStringRef s) {
    if (s == nullptr) return {};
    const CFIndex len = CFStringGetLength(s);
    const CFIndex max = CFStringGetMaximumSizeForEncoding(len, kCFStringEncodingUTF8) + 1;
    std::string out(static_cast<std::size_t>(max), '\0');
    if (!CFStringGetCString(s, out.data(), max, kCFStringEncodingUTF8)) return {};
    out.resize(std::strlen(out.c_str()));
    return out;
}

namespace {

AudioStreamBasicDescription float_format(double rate, uint32_t channels) {
    AudioStreamBasicDescription f{};
    f.mSampleRate = rate;
    f.mFormatID = kAudioFormatLinearPCM;
    f.mFormatFlags = static_cast<AudioFormatFlags>(kAudioFormatFlagsNativeFloatPacked) |
                     static_cast<AudioFormatFlags>(kAudioFormatFlagIsNonInterleaved);
    f.mBytesPerPacket = 4;
    f.mFramesPerPacket = 1;
    f.mBytesPerFrame = 4;
    f.mChannelsPerFrame = channels;
    f.mBitsPerChannel = 32;
    return f;
}

std::string os_err(const char* what, OSStatus st) {
    return std::string(what) + " failed (OSStatus " + std::to_string(st) + ")";
}

struct StereoBufferList {
    UInt32 mNumberBuffers = 2;
    AudioBuffer mBuffers[2] = {};
    AudioBufferList* get() noexcept { return reinterpret_cast<AudioBufferList*>(this); }
};
static_assert(offsetof(StereoBufferList, mBuffers) == offsetof(AudioBufferList, mBuffers));

class AuInstance final : public PluginInstance {
public:
    AuInstance(PluginSpec spec, double rate, uint32_t max_block)
        : PluginInstance(std::move(spec)), rate_(rate), max_block_(max_block), in_l_(max_block), in_r_(max_block),
          out_l_(max_block), out_r_(max_block) {}

    ~AuInstance() override {
        if (unit_ != nullptr) {
            if (initialized_) AudioUnitUninitialize(unit_);
            AudioComponentInstanceDispose(unit_);
        }
    }

    bool init(std::string& err) {
        AudioComponentDescription desc{};
        if (!parse_au_id(spec_.plugin_id, desc)) {
            err = "invalid Audio Unit id '" + spec_.plugin_id + "' (expected \"type subtype manufacturer\")";
            return false;
        }
        AudioComponent comp = AudioComponentFindNext(nullptr, &desc);
        if (comp == nullptr) {
            err = "Audio Unit '" + spec_.plugin_id + "' is not installed";
            return false;
        }
        OSStatus st = AudioComponentInstanceNew(comp, &unit_);
        if (st != noErr || unit_ == nullptr) {
            unit_ = nullptr;
            err = os_err("AudioComponentInstanceNew", st);
            return false;
        }
        is_instrument_ = desc.componentType == kAudioUnitType_MusicDevice;

        UInt32 max = max_block_;
        AudioUnitSetProperty(unit_, kAudioUnitProperty_MaximumFramesPerSlice, kAudioUnitScope_Global, 0, &max,
                             sizeof max);

        // Output: stereo, else mono (mapped to both sides).
        out_channels_ = set_format(kAudioUnitScope_Output);
        if (out_channels_ == 0) {
            err = "Audio Unit output bus accepts neither stereo nor mono float32";
            return false;
        }
        UInt32 in_buses = 0;
        UInt32 size = sizeof in_buses;
        AudioUnitGetProperty(unit_, kAudioUnitProperty_ElementCount, kAudioUnitScope_Input, 0, &in_buses, &size);
        if (!is_instrument_ || in_buses > 0) {
            if (in_buses > 0) {
                in_channels_ = set_format(kAudioUnitScope_Input);
                if (in_channels_ == 0 && !is_instrument_) {
                    err = "Audio Unit input bus accepts neither stereo nor mono float32";
                    return false;
                }
                if (in_channels_ > 0) {
                    AURenderCallbackStruct cb{&AuInstance::input_proc, this};
                    st = AudioUnitSetProperty(unit_, kAudioUnitProperty_SetRenderCallback, kAudioUnitScope_Input, 0,
                                              &cb, sizeof cb);
                    if (st != noErr) {
                        err = os_err("setting the input callback", st);
                        return false;
                    }
                }
            } else if (!is_instrument_) {
                err = "Audio Unit effect has no input bus";
                return false;
            }
        }

        HostCallbackInfo hc{};
        hc.hostUserData = this;
        hc.beatAndTempoProc = &AuInstance::beat_and_tempo;
        hc.transportStateProc2 = &AuInstance::transport_state;
        AudioUnitSetProperty(unit_, kAudioUnitProperty_HostCallbacks, kAudioUnitScope_Global, 0, &hc, sizeof hc);

        st = AudioUnitInitialize(unit_);
        if (st != noErr) {
            err = os_err("AudioUnitInitialize", st);
            return false;
        }
        initialized_ = true;

        if (!spec_.state.empty() && !load_state(spec_.state, err)) return false;
        spec_.state.clear();
        spec_.state.shrink_to_fit();
        ts_.mFlags = kAudioTimeStampSampleTimeValid;
        ts_.mSampleTime = 0;
        return true;
    }

    void process(float* l, float* r, uint32_t frames, const MidiEvent* events, uint32_t event_count,
                 const RtTransport& transport) noexcept override {
        transport_ = transport;
        ParamChange pc;
        while (pop_param(pc))
            AudioUnitSetParameter(unit_, pc.id, kAudioUnitScope_Global, 0, static_cast<AudioUnitParameterValue>(pc.value),
                                  0);
        if (is_instrument_)
            for (uint32_t i = 0; i < event_count; ++i)
                MusicDeviceMIDIEvent(unit_, events[i].status, events[i].data1, events[i].data2, events[i].offset);
        cur_l_ = l;
        cur_r_ = r;
        cur_frames_ = frames;
        StereoBufferList abl;
        abl.mNumberBuffers = out_channels_;
        abl.mBuffers[0] = AudioBuffer{1, frames * 4, out_l_.data()};
        abl.mBuffers[1] = AudioBuffer{1, frames * 4, out_r_.data()};
        AudioUnitRenderActionFlags flags = 0;
        const OSStatus st = AudioUnitRender(unit_, &flags, &ts_, 0, frames, abl.get());
        ts_.mSampleTime += frames;
        if (st != noErr) {
            if (is_instrument_) {
                std::memset(l, 0, sizeof(float) * frames);
                std::memset(r, 0, sizeof(float) * frames);
            }
            return; // effect: pass through
        }
        const auto* ol = static_cast<const float*>(abl.mBuffers[0].mData);
        const auto* orr = out_channels_ > 1 ? static_cast<const float*>(abl.mBuffers[1].mData) : ol;
        std::memcpy(l, ol, sizeof(float) * frames);
        std::memcpy(r, orr, sizeof(float) * frames);
    }

    bool save_state(std::vector<uint8_t>& out, std::string& err) override {
        CFPropertyListRef plist = nullptr;
        UInt32 size = sizeof plist;
        const OSStatus st =
            AudioUnitGetProperty(unit_, kAudioUnitProperty_ClassInfo, kAudioUnitScope_Global, 0, &plist, &size);
        if (st != noErr || plist == nullptr) {
            err = os_err("reading kAudioUnitProperty_ClassInfo", st);
            return false;
        }
        CFErrorRef cferr = nullptr;
        CFDataRef data = CFPropertyListCreateData(kCFAllocatorDefault, plist, kCFPropertyListBinaryFormat_v1_0, 0,
                                                  &cferr);
        CFRelease(plist);
        if (data == nullptr) {
            if (cferr) CFRelease(cferr);
            err = "could not serialize the Audio Unit class info";
            return false;
        }
        const auto* p = CFDataGetBytePtr(data);
        out.assign(p, p + CFDataGetLength(data));
        CFRelease(data);
        return true;
    }

    uint32_t latency() override {
        Float64 secs = 0;
        UInt32 size = sizeof secs;
        if (AudioUnitGetProperty(unit_, kAudioUnitProperty_Latency, kAudioUnitScope_Global, 0, &secs, &size) != noErr)
            return 0;
        return static_cast<uint32_t>(std::llround(std::max(0.0, secs) * rate_));
    }

    std::unique_ptr<PluginEditor> open_editor(const std::string& title, std::string& err) override {
        return create_au_editor(unit_, title, err);
    }

private:
    // Tries stereo, then mono, on bus 0 of `scope`; returns the channel count or 0.
    uint32_t set_format(AudioUnitScope scope) {
        for (uint32_t ch : {2u, 1u}) {
            const AudioStreamBasicDescription f = float_format(rate_, ch);
            if (AudioUnitSetProperty(unit_, kAudioUnitProperty_StreamFormat, scope, 0, &f, sizeof f) == noErr)
                return ch;
        }
        return 0;
    }

    bool load_state(const std::vector<uint8_t>& bytes, std::string& err) {
        CFDataRef data = CFDataCreate(kCFAllocatorDefault, bytes.data(), static_cast<CFIndex>(bytes.size()));
        CFPropertyListRef plist =
            CFPropertyListCreateWithData(kCFAllocatorDefault, data, kCFPropertyListImmutable, nullptr, nullptr);
        CFRelease(data);
        if (plist == nullptr) {
            err = "saved Audio Unit state is not a property list";
            return false;
        }
        const OSStatus st =
            AudioUnitSetProperty(unit_, kAudioUnitProperty_ClassInfo, kAudioUnitScope_Global, 0, &plist, sizeof plist);
        CFRelease(plist);
        if (st != noErr) {
            err = os_err("restoring the Audio Unit state", st);
            return false;
        }
        // Tell listeners (e.g. an open view) that parameters changed.
        AudioUnitParameter changed{unit_, kAUParameterListener_AnyParameter, kAudioUnitScope_Global, 0};
        AUParameterListenerNotify(nullptr, nullptr, &changed);
        return true;
    }

    // RT: pulls our input into the AU.
    static OSStatus input_proc(void* ref, AudioUnitRenderActionFlags*, const AudioTimeStamp*, UInt32, UInt32 frames,
                               AudioBufferList* io) {
        auto* self = static_cast<AuInstance*>(ref);
        if (frames > self->cur_frames_ || frames > self->max_block_) return kAudioUnitErr_TooManyFramesToProcess;
        const uint32_t nb = io->mNumberBuffers;
        for (uint32_t b = 0; b < nb; ++b) {
            float* dst = b == 0 ? self->in_l_.data() : self->in_r_.data();
            if (nb == 1 && self->in_channels_ == 1) {
                for (uint32_t i = 0; i < frames; ++i) dst[i] = 0.5f * (self->cur_l_[i] + self->cur_r_[i]);
            } else {
                std::memcpy(dst, b == 0 ? self->cur_l_ : self->cur_r_, sizeof(float) * frames);
            }
            if (io->mBuffers[b].mData != nullptr && io->mBuffers[b].mData != dst)
                std::memcpy(io->mBuffers[b].mData, dst, sizeof(float) * frames);
            else
                io->mBuffers[b].mData = dst;
            io->mBuffers[b].mDataByteSize = frames * 4;
        }
        return noErr;
    }

    static OSStatus beat_and_tempo(void* ref, Float64* beat, Float64* tempo) {
        auto* self = static_cast<AuInstance*>(ref);
        if (beat) *beat = self->transport_.beats();
        if (tempo) *tempo = self->transport_.tempo_bpm;
        return noErr;
    }

    static OSStatus transport_state(void* ref, Boolean* playing, Boolean* recording, Boolean* changed,
                                    Float64* sample, Boolean* cycling, Float64* cycle_start, Float64* cycle_end) {
        auto* self = static_cast<AuInstance*>(ref);
        if (playing) *playing = self->transport_.playing;
        if (recording) *recording = false;
        if (changed) {
            *changed = self->transport_.playing != self->last_playing_;
            self->last_playing_ = self->transport_.playing;
        }
        if (sample) *sample = static_cast<Float64>(self->transport_.position);
        if (cycling) *cycling = false;
        if (cycle_start) *cycle_start = 0;
        if (cycle_end) *cycle_end = 0;
        return noErr;
    }

    double rate_;
    uint32_t max_block_;
    AudioComponentInstance unit_ = nullptr;
    bool initialized_ = false;
    bool is_instrument_ = false;
    uint32_t in_channels_ = 0;
    uint32_t out_channels_ = 0;
    std::vector<float> in_l_, in_r_, out_l_, out_r_;
    // RT-owned
    AudioTimeStamp ts_{};
    const float* cur_l_ = nullptr;
    const float* cur_r_ = nullptr;
    uint32_t cur_frames_ = 0;
    RtTransport transport_{};
    bool last_playing_ = false;
};

} // namespace

std::shared_ptr<PluginInstance> create_au_instance(const PluginSpec& spec, double sample_rate, uint32_t max_block,
                                                   std::string& err) {
    auto inst = std::make_shared<AuInstance>(spec, sample_rate, max_block);
    if (!inst->init(err)) return nullptr;
    return inst;
}

} // namespace saine

#else

namespace saine {
std::shared_ptr<PluginInstance> create_au_instance(const PluginSpec&, double, uint32_t, std::string& err) {
    err = "Audio Units are only available on macOS";
    return nullptr;
}
} // namespace saine

#endif
