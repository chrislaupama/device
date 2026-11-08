// SaineTestGain.vst3: a minimal, original VST3 effect for the engine tests,
// written directly against the VST 3 pluginterfaces (no SDK base classes).
// Separate processor and controller classes, connected through
// IConnectionPoint by the host, like most real plugins.
//   - one stereo in / stereo out audio bus
//   - parameter 0 "Gain": normalized value == linear gain 0..1, default 0.5,
//     applied sample-accurately at each IParamValueQueue point's offset
//     (step from that sample on)
//   - editor-gesture test hook: the controller's setParamNormalized on the
//     hidden, unlisted parameter 1000 with value v acts like a knob drag in
//     the editor: beginEdit(0), setParamNormalized(0, v), performEdit(0, v),
//     endEdit(0) through the host's IComponentHandler
//   - component state = the gain as a little-endian float32; controller
//     state is empty
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

#include <CoreFoundation/CoreFoundation.h>

#include <algorithm>
#include <atomic>
#include <cmath>
#include <cstdio>
#include <cstring>

using namespace Steinberg;
using namespace Steinberg::Vst;

namespace {

const TUID kProcessorUID = INLINE_UID(0x5A17E5A1, 0x6E000001, 0x8A1E7E57, 0x6A170001);
const TUID kControllerUID = INLINE_UID(0x5A17E5A1, 0x6E000002, 0x8A1E7E57, 0x6A170002);

void copy_str16(String128 dst, const char* src) {
    std::size_t i = 0;
    for (; src[i] != 0 && i < 127; ++i) dst[i] = static_cast<char16>(src[i]);
    dst[i] = 0;
}

bool read_gain(IBStream* s, float& g) {
    if (s == nullptr) return false;
    uint8_t b[4] = {};
    int32 got = 0;
    if (s->read(b, 4, &got) != kResultOk || got != 4) return false;
    std::memcpy(&g, b, 4); // little-endian hosts only (arm64/x86-64)
    return std::isfinite(g) && g >= 0.0f && g <= 1.0f;
}

class Processor : public U::Implements<U::Directly<IComponent, IAudioProcessor, IConnectionPoint>,
                                       U::Indirectly<IPluginBase>> {
public:
    // IPluginBase
    tresult PLUGIN_API initialize(FUnknown*) override { return kResultOk; }
    tresult PLUGIN_API terminate() override { return kResultOk; }
    // IComponent
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
    tresult PLUGIN_API setActive(TBool) override { return kResultOk; }
    tresult PLUGIN_API setState(IBStream* s) override {
        float g = 0.0f;
        if (!read_gain(s, g)) return kResultFalse;
        gain_.store(g);
        return kResultOk;
    }
    tresult PLUGIN_API getState(IBStream* s) override {
        if (s == nullptr) return kInvalidArgument;
        const float g = gain_.load();
        uint8_t b[4];
        std::memcpy(b, &g, 4);
        int32 written = 0;
        return s->write(b, 4, &written) == kResultOk && written == 4 ? kResultOk : kResultFalse;
    }
    // IAudioProcessor
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
    uint32 PLUGIN_API getLatencySamples() override { return 0; }
    tresult PLUGIN_API setupProcessing(ProcessSetup&) override { return kResultOk; }
    tresult PLUGIN_API setProcessing(TBool) override { return kResultOk; }
    tresult PLUGIN_API process(ProcessData& d) override {
        IParamValueQueue* gain_queue = nullptr;
        if (IParameterChanges* pc = d.inputParameterChanges) {
            const int32 n = pc->getParameterCount();
            for (int32 i = 0; i < n; ++i) {
                IParamValueQueue* q = pc->getParameterData(i);
                if (q != nullptr && q->getParameterId() == 0 && q->getPointCount() > 0) gain_queue = q;
            }
        }
        if (d.numInputs < 1 || d.numOutputs < 1 || d.numSamples <= 0) { // parameter flush
            int32 offset = 0;
            ParamValue v = 0;
            if (gain_queue != nullptr && gain_queue->getPoint(gain_queue->getPointCount() - 1, offset, v) == kResultOk)
                gain_.store(static_cast<float>(v));
            return kResultOk;
        }
        // Sample-accurate: each point applies from its offset on.
        const int32 points = gain_queue != nullptr ? gain_queue->getPointCount() : 0;
        int32 pos = 0;
        for (int32 k = 0; k <= points; ++k) {
            int32 offset = d.numSamples;
            ParamValue v = 0;
            if (k < points && gain_queue->getPoint(k, offset, v) != kResultOk) continue;
            const int32 end = std::min(std::max(offset, pos), d.numSamples);
            const float g = gain_.load();
            for (int32 c = 0; c < 2; ++c) {
                const float* in = d.inputs[0].channelBuffers32[c];
                float* out = d.outputs[0].channelBuffers32[c];
                for (int32 i = pos; i < end; ++i) out[i] = in[i] * g;
            }
            pos = end;
            if (k < points) gain_.store(static_cast<float>(v));
        }
        d.outputs[0].silenceFlags = 0;
        return kResultOk;
    }
    uint32 PLUGIN_API getTailSamples() override { return kNoTail; }
    // IConnectionPoint
    tresult PLUGIN_API connect(IConnectionPoint* other) override {
        peer_ = other;
        return kResultOk;
    }
    tresult PLUGIN_API disconnect(IConnectionPoint*) override {
        peer_ = nullptr;
        return kResultOk;
    }
    tresult PLUGIN_API notify(IMessage*) override { return kResultOk; }

private:
    std::atomic<float> gain_{0.5f};
    IConnectionPoint* peer_ = nullptr;
};

class Controller : public U::Implements<U::Directly<IEditController, IConnectionPoint>, U::Indirectly<IPluginBase>> {
public:
    static constexpr ParamID kGestureParam = 1000;
    ~Controller() override {
        if (handler_ != nullptr) handler_->release();
    }
    // IPluginBase
    tresult PLUGIN_API initialize(FUnknown*) override { return kResultOk; }
    tresult PLUGIN_API terminate() override { return kResultOk; }
    // IEditController
    tresult PLUGIN_API setComponentState(IBStream* s) override {
        float g = 0.0f;
        if (!read_gain(s, g)) return kResultFalse;
        norm_ = g;
        return kResultOk;
    }
    tresult PLUGIN_API setState(IBStream*) override { return kResultOk; }
    tresult PLUGIN_API getState(IBStream*) override { return kResultOk; }
    int32 PLUGIN_API getParameterCount() override { return 1; }
    tresult PLUGIN_API getParameterInfo(int32 index, ParameterInfo& info) override {
        if (index != 0) return kInvalidArgument;
        info = ParameterInfo{};
        info.id = 0;
        copy_str16(info.title, "Gain");
        copy_str16(info.shortTitle, "Gain");
        info.stepCount = 0;
        info.defaultNormalizedValue = 0.5;
        info.unitId = kRootUnitId;
        info.flags = ParameterInfo::kCanAutomate;
        return kResultOk;
    }
    tresult PLUGIN_API getParamStringByValue(ParamID id, ParamValue v, String128 out) override {
        if (id != 0) return kInvalidArgument;
        char buf[32];
        std::snprintf(buf, sizeof buf, "%.3f", v);
        copy_str16(out, buf);
        return kResultOk;
    }
    tresult PLUGIN_API getParamValueByString(ParamID, TChar*, ParamValue&) override { return kNotImplemented; }
    ParamValue PLUGIN_API normalizedParamToPlain(ParamID, ParamValue v) override { return v; }
    ParamValue PLUGIN_API plainParamToNormalized(ParamID, ParamValue v) override { return v; }
    ParamValue PLUGIN_API getParamNormalized(ParamID id) override { return id == 0 ? norm_ : 0.0; }
    tresult PLUGIN_API setParamNormalized(ParamID id, ParamValue v) override {
        if (id == kGestureParam) { // test hook: an editor knob drag
            if (handler_ == nullptr) return kResultFalse;
            handler_->beginEdit(0);
            norm_ = v;
            handler_->performEdit(0, v);
            handler_->endEdit(0);
            return kResultOk;
        }
        if (id != 0) return kInvalidArgument;
        norm_ = v;
        return kResultOk;
    }
    tresult PLUGIN_API setComponentHandler(IComponentHandler* h) override {
        if (h != nullptr) h->addRef();
        if (handler_ != nullptr) handler_->release();
        handler_ = h;
        return kResultOk;
    }
    IPlugView* PLUGIN_API createView(FIDString) override { return nullptr; }
    // IConnectionPoint
    tresult PLUGIN_API connect(IConnectionPoint*) override { return kResultOk; }
    tresult PLUGIN_API disconnect(IConnectionPoint*) override { return kResultOk; }
    tresult PLUGIN_API notify(IMessage*) override { return kResultOk; }

private:
    ParamValue norm_ = 0.5;
    IComponentHandler* handler_ = nullptr;
};

class Factory : public U::Implements<U::Directly<IPluginFactory2>, U::Indirectly<IPluginFactory>> {
public:
    tresult PLUGIN_API getFactoryInfo(PFactoryInfo* info) override {
        *info = PFactoryInfo("Saine", "", "", PFactoryInfo::kNoFlags);
        return kResultOk;
    }
    int32 PLUGIN_API countClasses() override { return 2; }
    tresult PLUGIN_API getClassInfo(int32 index, PClassInfo* info) override {
        if (index == 0) *info = PClassInfo(kProcessorUID, PClassInfo::kManyInstances, kVstAudioEffectClass, "Saine Test Gain");
        else if (index == 1)
            *info = PClassInfo(kControllerUID, PClassInfo::kManyInstances, kVstComponentControllerClass,
                               "Saine Test Gain Controller");
        else return kInvalidArgument;
        return kResultOk;
    }
    tresult PLUGIN_API getClassInfo2(int32 index, PClassInfo2* info) override {
        if (index == 0)
            *info = PClassInfo2(kProcessorUID, PClassInfo::kManyInstances, kVstAudioEffectClass, "Saine Test Gain", 0,
                                "Fx", "Saine", "1.0.0", kVstVersionString);
        else if (index == 1)
            *info = PClassInfo2(kControllerUID, PClassInfo::kManyInstances, kVstComponentControllerClass,
                                "Saine Test Gain Controller", 0, "", "Saine", "1.0.0", kVstVersionString);
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
SMTG_EXPORT_SYMBOL bool bundleEntry(CFBundleRef) { return true; }
SMTG_EXPORT_SYMBOL bool bundleExit() { return true; }
}
