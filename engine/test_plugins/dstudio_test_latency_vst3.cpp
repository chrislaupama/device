// DStudioTestLatency.vst3: original test plugin for plugin delay compensation,
// written directly against the VST 3 pluginterfaces (like DStudioTestGain.vst3).
// A stereo effect that delays its input by exactly N frames and reports N
// from IAudioProcessor::getLatencySamples.
//   - component state = N as a little-endian uint32 (default 0)
//   - parameter 0 "Latency": normalized value = N / kMaxLatency. The
//     controller answers a changed value with
//     IComponentHandler::restartComponent(kLatencyChanged); the processor
//     stores the value it receives in process() and applies it in the next
//     setActive(true) (the host re-activates the component on its main
//     thread), where the delay buffer is allocated.
// process() does not allocate.
#include <pluginterfaces/base/funknownimpl.h>
#include <pluginterfaces/base/ibstream.h>
#include <pluginterfaces/base/ipluginbase.h>
#include <pluginterfaces/gui/iplugview.h>
#include <pluginterfaces/vst/ivstaudioprocessor.h>
#include <pluginterfaces/vst/ivstcomponent.h>
#include <pluginterfaces/vst/ivsteditcontroller.h>
#include <pluginterfaces/vst/ivstmessage.h>
#include <pluginterfaces/vst/ivstparameterchanges.h>
#include <pluginterfaces/vst/ivstunits.h>

#if defined(__APPLE__)
#include <CoreFoundation/CoreFoundation.h>
#endif

#include <atomic>
#include <cmath>
#include <cstdio>
#include <cstring>
#include <vector>

using namespace Steinberg;
using namespace Steinberg::Vst;

namespace {

const TUID kProcessorUID = INLINE_UID(0x5A17E5A1, 0x6E000003, 0x8A1E7E57, 0x6A170003);
const TUID kControllerUID = INLINE_UID(0x5A17E5A1, 0x6E000004, 0x8A1E7E57, 0x6A170004);
constexpr uint32_t kMaxLatency = 1u << 20;

void copy_str16(String128 dst, const char* src) {
    std::size_t i = 0;
    for (; src[i] != 0 && i < 127; ++i) dst[i] = static_cast<char16>(src[i]);
    dst[i] = 0;
}

bool read_n(IBStream* s, uint32_t& n) {
    if (s == nullptr) return false;
    uint8_t b[4] = {};
    int32 got = 0;
    if (s->read(b, 4, &got) != kResultOk || got != 4) return false;
    n = 0;
    for (int i = 0; i < 4; ++i) n |= static_cast<uint32_t>(b[i]) << (8 * i);
    return n <= kMaxLatency;
}

uint32_t norm_to_n(ParamValue v) {
    if (!(v >= 0.0)) return 0;
    if (v > 1.0) v = 1.0;
    return static_cast<uint32_t>(std::lround(v * kMaxLatency));
}

class Processor : public U::Implements<U::Directly<IComponent, IAudioProcessor, IConnectionPoint>,
                                       U::Indirectly<IPluginBase>> {
public:
    tresult PLUGIN_API initialize(FUnknown*) override { return kResultOk; }
    tresult PLUGIN_API terminate() override { return kResultOk; }
    tresult PLUGIN_API getControllerClassId(TUID cid) override {
        std::memcpy(cid, kControllerUID, sizeof(TUID));
        return kResultOk;
    }
    tresult PLUGIN_API setIoMode(IoMode) override { return kResultOk; }
    int32 PLUGIN_API getBusCount(MediaType type, BusDirection) override { return type == kAudio ? 1 : 0; }
    tresult PLUGIN_API getBusInfo(MediaType type, BusDirection dir, int32 index, BusInfo& bus) override {
        if (type != kAudio || index != 0) return kInvalidArgument;
        bus.mediaType = kAudio;
        bus.direction = dir;
        bus.channelCount = 2;
        copy_str16(bus.name, dir == kInput ? "Input" : "Output");
        bus.busType = kMain;
        bus.flags = BusInfo::kDefaultActive;
        return kResultOk;
    }
    tresult PLUGIN_API getRoutingInfo(RoutingInfo&, RoutingInfo&) override { return kNotImplemented; }
    tresult PLUGIN_API activateBus(MediaType, BusDirection, int32, TBool) override { return kResultOk; }
    tresult PLUGIN_API setActive(TBool state) override {
        if (state) { // main thread: allocation allowed
            latency_ = requested_.load();
            for (auto& b : buf_) b.assign(latency_, 0.0f);
            pos_ = 0;
        }
        return kResultOk;
    }
    tresult PLUGIN_API setState(IBStream* s) override {
        uint32_t n = 0;
        if (!read_n(s, n)) return kResultFalse;
        requested_.store(n);
        return kResultOk;
    }
    tresult PLUGIN_API getState(IBStream* s) override {
        if (s == nullptr) return kInvalidArgument;
        const uint32_t n = requested_.load();
        uint8_t b[4];
        for (int i = 0; i < 4; ++i) b[i] = static_cast<uint8_t>(n >> (8 * i));
        int32 written = 0;
        return s->write(b, 4, &written) == kResultOk && written == 4 ? kResultOk : kResultFalse;
    }
    tresult PLUGIN_API setBusArrangements(SpeakerArrangement* ins, int32 n_in, SpeakerArrangement* outs,
                                          int32 n_out) override {
        return n_in == 1 && n_out == 1 && ins[0] == SpeakerArr::kStereo && outs[0] == SpeakerArr::kStereo
                   ? kResultOk
                   : kResultFalse;
    }
    tresult PLUGIN_API getBusArrangement(BusDirection, int32 index, SpeakerArrangement& arr) override {
        if (index != 0) return kInvalidArgument;
        arr = SpeakerArr::kStereo;
        return kResultOk;
    }
    tresult PLUGIN_API canProcessSampleSize(int32 size) override {
        return size == kSample32 ? kResultOk : kResultFalse;
    }
    uint32 PLUGIN_API getLatencySamples() override { return latency_; }
    tresult PLUGIN_API setupProcessing(ProcessSetup&) override { return kResultOk; }
    tresult PLUGIN_API setProcessing(TBool) override { return kResultOk; }
    tresult PLUGIN_API process(ProcessData& d) override {
        if (IParameterChanges* pc = d.inputParameterChanges) {
            const int32 n = pc->getParameterCount();
            for (int32 i = 0; i < n; ++i) {
                IParamValueQueue* q = pc->getParameterData(i);
                if (q == nullptr || q->getParameterId() != 0 || q->getPointCount() <= 0) continue;
                int32 offset = 0;
                ParamValue v = 0;
                if (q->getPoint(q->getPointCount() - 1, offset, v) == kResultOk) requested_.store(norm_to_n(v));
            }
        }
        if (d.numInputs < 1 || d.numOutputs < 1 || d.numSamples <= 0) return kResultOk;
        const auto n = static_cast<uint32_t>(d.numSamples);
        for (int32 c = 0; c < 2; ++c) {
            const float* in = d.inputs[0].channelBuffers32[c];
            float* out = d.outputs[0].channelBuffers32[c];
            if (latency_ == 0) {
                if (out != in) std::memmove(out, in, sizeof(float) * n);
                continue;
            }
            float* ring = buf_[c].data();
            uint32_t pos = pos_;
            for (uint32_t i = 0; i < n; ++i) {
                const float x = in[i];
                out[i] = ring[pos];
                ring[pos] = x;
                if (++pos == latency_) pos = 0;
            }
        }
        if (latency_ != 0) pos_ = static_cast<uint32_t>((pos_ + static_cast<uint64_t>(n)) % latency_);
        d.outputs[0].silenceFlags = 0;
        return kResultOk;
    }
    uint32 PLUGIN_API getTailSamples() override { return kNoTail; }
    tresult PLUGIN_API connect(IConnectionPoint*) override { return kResultOk; }
    tresult PLUGIN_API disconnect(IConnectionPoint*) override { return kResultOk; }
    tresult PLUGIN_API notify(IMessage*) override { return kResultOk; }

private:
    std::atomic<uint32_t> requested_{0};
    uint32_t latency_ = 0;
    std::vector<float> buf_[2];
    uint32_t pos_ = 0;
};

class Controller : public U::Implements<U::Directly<IEditController, IConnectionPoint>, U::Indirectly<IPluginBase>> {
public:
    ~Controller() override {
        if (handler_ != nullptr) handler_->release();
    }
    tresult PLUGIN_API initialize(FUnknown*) override { return kResultOk; }
    tresult PLUGIN_API terminate() override { return kResultOk; }
    tresult PLUGIN_API setComponentState(IBStream* s) override {
        uint32_t n = 0;
        if (!read_n(s, n)) return kResultFalse;
        norm_ = static_cast<double>(n) / kMaxLatency;
        return kResultOk;
    }
    tresult PLUGIN_API setState(IBStream*) override { return kResultOk; }
    tresult PLUGIN_API getState(IBStream*) override { return kResultOk; }
    int32 PLUGIN_API getParameterCount() override { return 1; }
    tresult PLUGIN_API getParameterInfo(int32 index, ParameterInfo& info) override {
        if (index != 0) return kInvalidArgument;
        info = ParameterInfo{};
        info.id = 0;
        copy_str16(info.title, "Latency");
        copy_str16(info.shortTitle, "Latency");
        info.stepCount = static_cast<int32>(kMaxLatency);
        info.defaultNormalizedValue = 0.0;
        info.unitId = kRootUnitId;
        info.flags = ParameterInfo::kNoFlags;
        return kResultOk;
    }
    tresult PLUGIN_API getParamStringByValue(ParamID id, ParamValue v, String128 out) override {
        if (id != 0) return kInvalidArgument;
        char buf[32];
        std::snprintf(buf, sizeof buf, "%u", norm_to_n(v));
        copy_str16(out, buf);
        return kResultOk;
    }
    tresult PLUGIN_API getParamValueByString(ParamID, TChar*, ParamValue&) override { return kNotImplemented; }
    ParamValue PLUGIN_API normalizedParamToPlain(ParamID, ParamValue v) override { return norm_to_n(v); }
    ParamValue PLUGIN_API plainParamToNormalized(ParamID, ParamValue v) override { return v / kMaxLatency; }
    ParamValue PLUGIN_API getParamNormalized(ParamID id) override { return id == 0 ? norm_ : 0.0; }
    tresult PLUGIN_API setParamNormalized(ParamID id, ParamValue v) override {
        if (id != 0) return kInvalidArgument;
        const bool changed = norm_to_n(v) != norm_to_n(norm_);
        norm_ = v;
        if (changed && handler_ != nullptr) handler_->restartComponent(kLatencyChanged);
        return kResultOk;
    }
    tresult PLUGIN_API setComponentHandler(IComponentHandler* h) override {
        if (h != nullptr) h->addRef();
        if (handler_ != nullptr) handler_->release();
        handler_ = h;
        return kResultOk;
    }
    IPlugView* PLUGIN_API createView(FIDString) override { return nullptr; }
    tresult PLUGIN_API connect(IConnectionPoint*) override { return kResultOk; }
    tresult PLUGIN_API disconnect(IConnectionPoint*) override { return kResultOk; }
    tresult PLUGIN_API notify(IMessage*) override { return kResultOk; }

private:
    ParamValue norm_ = 0.0;
    IComponentHandler* handler_ = nullptr;
};

class Factory : public U::Implements<U::Directly<IPluginFactory2>, U::Indirectly<IPluginFactory>> {
public:
    tresult PLUGIN_API getFactoryInfo(PFactoryInfo* info) override {
        *info = PFactoryInfo("DStudio", "", "", PFactoryInfo::kNoFlags);
        return kResultOk;
    }
    int32 PLUGIN_API countClasses() override { return 2; }
    tresult PLUGIN_API getClassInfo(int32 index, PClassInfo* info) override {
        if (index == 0)
            *info = PClassInfo(kProcessorUID, PClassInfo::kManyInstances, kVstAudioEffectClass, "DStudio Test Latency");
        else if (index == 1)
            *info = PClassInfo(kControllerUID, PClassInfo::kManyInstances, kVstComponentControllerClass,
                               "DStudio Test Latency Controller");
        else return kInvalidArgument;
        return kResultOk;
    }
    tresult PLUGIN_API getClassInfo2(int32 index, PClassInfo2* info) override {
        if (index == 0)
            *info = PClassInfo2(kProcessorUID, PClassInfo::kManyInstances, kVstAudioEffectClass, "DStudio Test Latency",
                                0, "Fx", "DStudio", "1.0.0", kVstVersionString);
        else if (index == 1)
            *info = PClassInfo2(kControllerUID, PClassInfo::kManyInstances, kVstComponentControllerClass,
                                "DStudio Test Latency Controller", 0, "", "DStudio", "1.0.0", kVstVersionString);
        else return kInvalidArgument;
        return kResultOk;
    }
    tresult PLUGIN_API createInstance(FIDString cid, FIDString iid, void** obj) override {
        FUnknown* u = nullptr;
        if (std::memcmp(cid, kProcessorUID, sizeof(TUID)) == 0) u = static_cast<IComponent*>(new Processor());
        else if (std::memcmp(cid, kControllerUID, sizeof(TUID)) == 0) u = static_cast<IEditController*>(new Controller());
        if (u == nullptr) {
            *obj = nullptr;
            return kNoInterface;
        }
        const tresult r = u->queryInterface(iid, obj);
        u->release();
        return r;
    }
};

} // namespace

extern "C" {
SMTG_EXPORT_SYMBOL IPluginFactory* PLUGIN_API GetPluginFactory() { return new Factory(); }
// Module entry points of the VST 3 module architecture, per platform.
#if defined(__APPLE__)
SMTG_EXPORT_SYMBOL bool bundleEntry(CFBundleRef) { return true; }
SMTG_EXPORT_SYMBOL bool bundleExit() { return true; }
#elif defined(_WIN32)
SMTG_EXPORT_SYMBOL bool InitDll() { return true; }
SMTG_EXPORT_SYMBOL bool ExitDll() { return true; }
#else
SMTG_EXPORT_SYMBOL bool ModuleEntry(void*) { return true; }
SMTG_EXPORT_SYMBOL bool ModuleExit() { return true; }
#endif
}
