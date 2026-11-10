// Audio Unit (v2 API) hosting. macOS only.
#include "plugin_instance.hpp"

#if defined(__APPLE__)

#include "au_util.hpp"
#include "plugin_editor.hpp"

#include <AudioToolbox/AudioToolbox.h>
#include <AudioToolbox/AudioUnitUtilities.h>
#include <dispatch/dispatch.h>

#include <algorithm>
#include <cmath>
#include <cstdio>
#include <cstring>
#include <memory>
#include <mutex>

namespace dstudio {

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
        if (listen_) {
            std::lock_guard lock(listen_->mutex);
            listen_->self = nullptr; // a notification already queued becomes a no-op
        }
        if (listener_ != nullptr) AUListenerDispose(listener_);
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
        build_param_cache();
        start_listening();
        ts_.mFlags = kAudioTimeStampSampleTimeValid;
        ts_.mSampleTime = 0;
        return true;
    }

    void process(float* l, float* r, uint32_t frames, const MidiEvent* events, uint32_t event_count,
                 const RtTransport& transport, const ParamEvent* auto_events, uint32_t auto_count) noexcept override {
        transport_ = transport;
        ParamChange pc;
        while (pop_param(pc))
            AudioUnitSetParameter(unit_, pc.id, kAudioUnitScope_Global, 0, static_cast<AudioUnitParameterValue>(pc.value),
                                  0);
        // Automation: scheduled immediate events at their sample offsets
        // (units that do not schedule get a per-block set with the offset).
        for (uint32_t i = 0; i < auto_count;) {
            AudioUnitParameterEvent batch[64];
            UInt32 n = 0;
            for (; i < auto_count && n < 64; ++i, ++n) {
                AudioUnitParameterEvent& ev = batch[n];
                ev = AudioUnitParameterEvent{};
                ev.scope = kAudioUnitScope_Global;
                ev.element = 0;
                ev.parameter = auto_events[i].id;
                ev.eventType = kParameterEvent_Immediate;
                ev.eventValues.immediate.bufferOffset = auto_events[i].offset;
                ev.eventValues.immediate.value = static_cast<AudioUnitParameterValue>(auto_events[i].value);
            }
            if (AudioUnitScheduleParameters(unit_, batch, n) != noErr)
                for (UInt32 k = 0; k < n; ++k)
                    AudioUnitSetParameter(unit_, batch[k].parameter, kAudioUnitScope_Global, 0,
                                          batch[k].eventValues.immediate.value,
                                          batch[k].eventValues.immediate.bufferOffset);
        }
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

    // ---- parameters (main thread)
    void params(std::vector<ParamInfo>& out) override {
        out.clear();
        for (AudioUnitParameterID id : param_ids()) {
            AudioUnitParameterInfo pi{};
            UInt32 size = sizeof pi;
            if (AudioUnitGetProperty(unit_, kAudioUnitProperty_ParameterInfo, kAudioUnitScope_Global, id, &pi, &size) !=
                noErr)
                continue;
            ParamInfo info;
            info.id = id;
            if ((pi.flags & kAudioUnitParameterFlag_HasCFNameString) != 0 && pi.cfNameString != nullptr) {
                info.name = cf_to_string(pi.cfNameString);
                if ((pi.flags & kAudioUnitParameterFlag_CFNameRelease) != 0) CFRelease(pi.cfNameString);
            } else {
                info.name = std::string(pi.name, strnlen(pi.name, sizeof pi.name));
            }
            if ((pi.flags & kAudioUnitParameterFlag_HasClump) != 0) {
                AudioUnitParameterNameInfo clump{};
                clump.inID = pi.clumpID;
                clump.inDesiredLength = kAudioUnitParameterName_Full;
                UInt32 cs = sizeof clump;
                if (AudioUnitGetProperty(unit_, kAudioUnitProperty_ParameterClumpName, kAudioUnitScope_Global, 0, &clump,
                                         &cs) == noErr &&
                    clump.outName != nullptr) {
                    info.group = cf_to_string(clump.outName);
                    CFRelease(clump.outName);
                }
            }
            info.min = static_cast<double>(pi.minValue);
            info.max = static_cast<double>(pi.maxValue);
            const double span = static_cast<double>(pi.maxValue) - static_cast<double>(pi.minValue);
            info.default_normalized =
                span > 0.0 ? static_cast<float>(std::clamp(static_cast<double>(pi.defaultValue - pi.minValue) / span, 0.0, 1.0)) : 0.0f;
            info.steps = (pi.unit == kAudioUnitParameterUnit_Indexed || pi.unit == kAudioUnitParameterUnit_Boolean) &&
                                 span > 0.0
                             ? static_cast<uint32_t>(std::lround(span))
                             : 0;
            info.automatable = (pi.flags & kAudioUnitParameterFlag_IsWritable) != 0;
            out.push_back(std::move(info));
        }
    }

    bool param_range(uint32_t id, double& min, double& max) override {
        const AuParam* p = cached(id);
        if (p == nullptr) return false;
        min = p->min;
        max = p->max;
        return true;
    }

    bool param_value(uint32_t id, float& normalized) override {
        const AuParam* p = cached(id);
        AudioUnitParameterValue v = 0;
        if (p == nullptr || AudioUnitGetParameter(unit_, id, kAudioUnitScope_Global, 0, &v) != noErr) return false;
        normalized = p->normalize(static_cast<double>(v));
        return true;
    }

    bool param_text(uint32_t id, float normalized, std::string& out) override {
        const AuParam* p = cached(id);
        if (p == nullptr) return false;
        const auto value = static_cast<AudioUnitParameterValue>(p->plain(normalized));
        AudioUnitParameterStringFromValue sv{};
        sv.inParamID = id;
        sv.inValue = &value;
        UInt32 size = sizeof sv;
        if (AudioUnitGetProperty(unit_, kAudioUnitProperty_ParameterStringFromValue, kAudioUnitScope_Global, 0, &sv,
                                 &size) == noErr &&
            sv.outString != nullptr) {
            out = cf_to_string(sv.outString);
            CFRelease(sv.outString);
            if (!out.empty()) return true;
        }
        char buf[64];
        std::snprintf(buf, sizeof buf, std::fabs(value) >= 100.0f ? "%.0f" : "%.2f", static_cast<double>(value));
        out = buf;
        const std::string u = unit_label(id, p->unit);
        if (!u.empty()) out += " " + u;
        return true;
    }

    // Test hook: exactly what an AU view does for a knob drag.
    bool simulate_editor_gesture(uint32_t id, float normalized) override {
        const AuParam* p = cached(id);
        if (p == nullptr) return false;
        AudioUnitEvent ev{};
        ev.mArgument.mParameter = AudioUnitParameter{unit_, id, kAudioUnitScope_Global, 0};
        ev.mEventType = kAudioUnitEvent_BeginParameterChangeGesture;
        AUEventListenerNotify(nullptr, nullptr, &ev);
        AUParameterSet(nullptr, nullptr, &ev.mArgument.mParameter, static_cast<AudioUnitParameterValue>(p->plain(normalized)),
                       0);
        ev.mEventType = kAudioUnitEvent_EndParameterChangeGesture;
        AUEventListenerNotify(nullptr, nullptr, &ev);
        return true;
    }

private:
    struct AuParam {
        AudioUnitParameterID id = 0;
        double min = 0.0, max = 1.0;
        AudioUnitParameterUnit unit = kAudioUnitParameterUnit_Generic;
        float normalize(double v) const noexcept {
            return max > min ? static_cast<float>(std::clamp((v - min) / (max - min), 0.0, 1.0)) : 0.0f;
        }
        double plain(float n) const noexcept { return min + std::clamp(static_cast<double>(n), 0.0, 1.0) * (max - min); }
    };
    // Shared with the listener block; `self` is cleared (under the mutex)
    // before the instance goes away.
    struct ListenState {
        std::mutex mutex;
        AuInstance* self = nullptr;
    };

    std::vector<AudioUnitParameterID> param_ids() const {
        UInt32 size = 0;
        if (AudioUnitGetPropertyInfo(unit_, kAudioUnitProperty_ParameterList, kAudioUnitScope_Global, 0, &size,
                                     nullptr) != noErr ||
            size == 0)
            return {};
        std::vector<AudioUnitParameterID> ids(size / sizeof(AudioUnitParameterID));
        if (AudioUnitGetProperty(unit_, kAudioUnitProperty_ParameterList, kAudioUnitScope_Global, 0, ids.data(),
                                 &size) != noErr)
            return {};
        ids.resize(size / sizeof(AudioUnitParameterID));
        return ids;
    }

    // Ranges by id (immutable after init; read by the listener thread).
    void build_param_cache() {
        for (AudioUnitParameterID id : param_ids()) {
            AudioUnitParameterInfo pi{};
            UInt32 size = sizeof pi;
            if (AudioUnitGetProperty(unit_, kAudioUnitProperty_ParameterInfo, kAudioUnitScope_Global, id, &pi, &size) !=
                noErr)
                continue;
            if ((pi.flags & kAudioUnitParameterFlag_HasCFNameString) != 0 &&
                (pi.flags & kAudioUnitParameterFlag_CFNameRelease) != 0 && pi.cfNameString != nullptr)
                CFRelease(pi.cfNameString);
            pinfo_.push_back(AuParam{id, static_cast<double>(pi.minValue), static_cast<double>(pi.maxValue), pi.unit});
        }
        std::sort(pinfo_.begin(), pinfo_.end(), [](const AuParam& a, const AuParam& b) { return a.id < b.id; });
    }

    const AuParam* cached(AudioUnitParameterID id) const noexcept {
        auto it = std::lower_bound(pinfo_.begin(), pinfo_.end(), id,
                                   [](const AuParam& a, AudioUnitParameterID v) { return a.id < v; });
        return it != pinfo_.end() && it->id == id ? &*it : nullptr;
    }

    std::string unit_label(AudioUnitParameterID id, AudioUnitParameterUnit u) const {
        switch (u) {
        case kAudioUnitParameterUnit_Hertz: return "Hz";
        case kAudioUnitParameterUnit_Decibels: return "dB";
        case kAudioUnitParameterUnit_Percent: return "%";
        case kAudioUnitParameterUnit_Seconds: return "s";
        case kAudioUnitParameterUnit_Milliseconds: return "ms";
        case kAudioUnitParameterUnit_Cents: return "cents";
        case kAudioUnitParameterUnit_RelativeSemiTones: return "st";
        case kAudioUnitParameterUnit_Degrees: return "deg";
        case kAudioUnitParameterUnit_BPM: return "BPM";
        case kAudioUnitParameterUnit_CustomUnit: {
            AudioUnitParameterInfo pi{};
            UInt32 size = sizeof pi;
            if (AudioUnitGetProperty(unit_, kAudioUnitProperty_ParameterInfo, kAudioUnitScope_Global, id, &pi, &size) !=
                noErr)
                return {};
            if ((pi.flags & kAudioUnitParameterFlag_HasCFNameString) != 0 &&
                (pi.flags & kAudioUnitParameterFlag_CFNameRelease) != 0 && pi.cfNameString != nullptr)
                CFRelease(pi.cfNameString);
            return pi.unitName != nullptr ? cf_to_string(pi.unitName) : std::string{};
        }
        default: return {};
        }
    }

    // Parameter changes and begin/end gestures from AU views (AUParameterSet,
    // AUEventListenerNotify) arrive on a private serial dispatch queue.
    void start_listening() {
        static dispatch_queue_t queue = dispatch_queue_create("studio.dstudio.au-param-events", DISPATCH_QUEUE_SERIAL);
        listen_ = std::make_shared<ListenState>();
        listen_->self = this;
        std::shared_ptr<ListenState> state = listen_;
        const OSStatus st = AUEventListenerCreateWithDispatchQueue(
            &listener_, 0.0f, 0.0f, queue,
            ^(void*, const AudioUnitEvent* ev, UInt64, AudioUnitParameterValue value) {
                std::lock_guard lock(state->mutex);
                if (state->self != nullptr && ev != nullptr) state->self->on_au_event(*ev, value);
            });
        if (st != noErr) {
            listener_ = nullptr;
            return;
        }
        for (const AuParam& p : pinfo_) {
            AudioUnitEvent ev{};
            ev.mArgument.mParameter = AudioUnitParameter{unit_, p.id, kAudioUnitScope_Global, 0};
            for (AudioUnitEventType type : {kAudioUnitEvent_ParameterValueChange,
                                            kAudioUnitEvent_BeginParameterChangeGesture,
                                            kAudioUnitEvent_EndParameterChangeGesture}) {
                ev.mEventType = type;
                AUEventListenerAddEventType(listener_, nullptr, &ev);
            }
        }
    }

    void on_au_event(const AudioUnitEvent& ev, AudioUnitParameterValue value) noexcept {
        const AudioUnitParameter& par = ev.mArgument.mParameter;
        const AuParam* p = cached(par.mParameterID);
        if (p == nullptr || par.mAudioUnit != unit_) return;
        switch (ev.mEventType) {
        case kAudioUnitEvent_ParameterValueChange:
            report_change(par.mParameterID, p->normalize(static_cast<double>(value)), false, false);
            break;
        case kAudioUnitEvent_BeginParameterChangeGesture:
        case kAudioUnitEvent_EndParameterChangeGesture: {
            AudioUnitParameterValue v = 0;
            AudioUnitGetParameter(unit_, par.mParameterID, kAudioUnitScope_Global, 0, &v);
            const bool begin = ev.mEventType == kAudioUnitEvent_BeginParameterChangeGesture;
            report_change(par.mParameterID, p->normalize(static_cast<double>(v)), begin, !begin);
            break;
        }
        default: break;
        }
    }

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
    // Parameters.
    std::vector<AuParam> pinfo_;
    std::shared_ptr<ListenState> listen_;
    AUEventListenerRef listener_ = nullptr;
};

} // namespace

std::shared_ptr<PluginInstance> create_au_instance(const PluginSpec& spec, double sample_rate, uint32_t max_block,
                                                   std::string& err) {
    auto inst = std::make_shared<AuInstance>(spec, sample_rate, max_block);
    if (!inst->init(err)) return nullptr;
    return inst;
}

} // namespace dstudio

#else

namespace dstudio {
std::shared_ptr<PluginInstance> create_au_instance(const PluginSpec&, double, uint32_t, std::string& err) {
    err = "Audio Units are only available on macOS";
    return nullptr;
}
} // namespace dstudio

#endif
