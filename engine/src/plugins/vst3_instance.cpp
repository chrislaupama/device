// VST3 hosting: IComponent + IAudioProcessor + IEditController (separate
// controller connected through IConnectionPoint, or a single component that
// implements IEditController). Built against the pluginterfaces of the VST 3
// SDK (MIT, fetched at configure time); the SDK's HostApplication supplies
// IHostApplication / IMessage / IAttributeList.
//
// State container ("SVS3", little endian), produced by save_state():
//   char[4] "SVS3" | u32 version (1) | u64 n | n bytes IComponent::getState
//                  | u64 m | m bytes IEditController::getState
#include "plugin_instance.hpp"

#include "plugin_editor.hpp"
#include "plugin_modules.hpp"

#include <pluginterfaces/base/funknownimpl.h>
#include <pluginterfaces/base/ibstream.h>
#include <pluginterfaces/base/ipluginbase.h>
#include <pluginterfaces/gui/iplugview.h>
#include <pluginterfaces/vst/ivstaudioprocessor.h>
#include <pluginterfaces/vst/ivstcomponent.h>
#include <pluginterfaces/vst/ivsteditcontroller.h>
#include <pluginterfaces/vst/ivstevents.h>
#include <pluginterfaces/vst/ivstmessage.h>
#include <pluginterfaces/vst/ivstparameterchanges.h>
#include <pluginterfaces/vst/ivstprocesscontext.h>
#include <public.sdk/source/vst/hosting/hostclasses.h>

#include <algorithm>
#include <cstring>

namespace saine {

using namespace Steinberg;
using namespace Steinberg::Vst;

// ---------------------------------------------------------------- helpers

// IBStream over a byte vector (non-RT; state only).
class VecStream final : public U::Implements<U::Directly<IBStream>> {
public:
    VecStream() = default;
    explicit VecStream(std::vector<uint8_t> d) : data_(std::move(d)) {}
    tresult PLUGIN_API read(void* buffer, int32 n, int32* read) override {
        const auto avail = static_cast<int64>(data_.size()) - pos_;
        const int32 k = static_cast<int32>(std::clamp<int64>(n, 0, std::max<int64>(avail, 0)));
        if (k > 0) std::memcpy(buffer, data_.data() + pos_, static_cast<std::size_t>(k));
        pos_ += k;
        if (read) *read = k;
        return kResultOk;
    }
    tresult PLUGIN_API write(void* buffer, int32 n, int32* written) override {
        if (n < 0) return kInvalidArgument;
        const auto end = static_cast<std::size_t>(pos_) + static_cast<std::size_t>(n);
        if (end > data_.size()) data_.resize(end);
        std::memcpy(data_.data() + pos_, buffer, static_cast<std::size_t>(n));
        pos_ += n;
        if (written) *written = n;
        return kResultOk;
    }
    tresult PLUGIN_API seek(int64 pos, int32 mode, int64* result) override {
        int64 base = 0;
        if (mode == kIBSeekCur) base = pos_;
        if (mode == kIBSeekEnd) base = static_cast<int64>(data_.size());
        const int64 np = base + pos;
        if (np < 0) return kInvalidArgument;
        pos_ = np;
        if (result) *result = pos_;
        return kResultOk;
    }
    tresult PLUGIN_API tell(int64* pos) override {
        if (pos) *pos = pos_;
        return kResultOk;
    }
    std::vector<uint8_t>& data() { return data_; }

private:
    std::vector<uint8_t> data_;
    int64 pos_ = 0;
};

// Fixed-capacity event list (RT: no allocation).
class FixedEventList final : public U::Implements<U::Directly<IEventList>> {
public:
    static constexpr int32 kCap = static_cast<int32>(PluginInstance::kMaxEvents);
    int32 PLUGIN_API getEventCount() override { return count_; }
    tresult PLUGIN_API getEvent(int32 index, Event& e) override {
        if (index < 0 || index >= count_) return kInvalidArgument;
        e = events_[index];
        return kResultOk;
    }
    tresult PLUGIN_API addEvent(Event& e) override {
        if (count_ >= kCap) return kResultFalse;
        events_[count_++] = e;
        return kResultOk;
    }
    void clear() noexcept { count_ = 0; }

private:
    Event events_[kCap] = {};
    int32 count_ = 0;
};

class FixedParamQueue final : public U::Implements<U::Directly<IParamValueQueue>> {
public:
    static constexpr int32 kCap = 16;
    ParamID PLUGIN_API getParameterId() override { return id_; }
    int32 PLUGIN_API getPointCount() override { return count_; }
    tresult PLUGIN_API getPoint(int32 index, int32& offset, ParamValue& value) override {
        if (index < 0 || index >= count_) return kInvalidArgument;
        offset = offsets_[index];
        value = values_[index];
        return kResultOk;
    }
    tresult PLUGIN_API addPoint(int32 offset, ParamValue value, int32& index) override {
        if (count_ > 0 && offsets_[count_ - 1] == offset) { // same time: replace
            values_[count_ - 1] = value;
            index = count_ - 1;
            return kResultOk;
        }
        if (count_ >= kCap) return kResultFalse;
        offsets_[count_] = offset;
        values_[count_] = value;
        index = count_++;
        return kResultOk;
    }
    void reset(ParamID id) noexcept {
        id_ = id;
        count_ = 0;
    }

private:
    ParamID id_ = 0;
    int32 count_ = 0;
    int32 offsets_[kCap] = {};
    ParamValue values_[kCap] = {};
};

class FixedParamChanges final : public U::Implements<U::Directly<IParameterChanges>> {
public:
    static constexpr int32 kCap = 64;
    int32 PLUGIN_API getParameterCount() override { return count_; }
    IParamValueQueue* PLUGIN_API getParameterData(int32 index) override {
        return index >= 0 && index < count_ ? &queues_[index] : nullptr;
    }
    IParamValueQueue* PLUGIN_API addParameterData(const ParamID& id, int32& index) override {
        for (int32 i = 0; i < count_; ++i)
            if (queues_[i].getParameterId() == id) {
                index = i;
                return &queues_[i];
            }
        if (count_ >= kCap) return nullptr;
        queues_[count_].reset(id);
        index = count_;
        return &queues_[count_++];
    }
    void clear() noexcept { count_ = 0; }

private:
    FixedParamQueue queues_[kCap];
    int32 count_ = 0;
};

namespace {

HostApplication& host_application() {
    static HostApplication* app = [] {
        auto* a = new HostApplication();
        a->addRef();
        return a;
    }();
    return *app;
}

void put_u32(std::vector<uint8_t>& v, uint32_t x) {
    for (int i = 0; i < 4; ++i) v.push_back(static_cast<uint8_t>(x >> (8 * i)));
}
void put_u64(std::vector<uint8_t>& v, uint64_t x) {
    for (int i = 0; i < 8; ++i) v.push_back(static_cast<uint8_t>(x >> (8 * i)));
}
bool get_u64(const std::vector<uint8_t>& v, std::size_t& at, uint64_t& x) {
    if (at + 8 > v.size()) return false;
    x = 0;
    for (int i = 0; i < 8; ++i) x |= static_cast<uint64_t>(v[at + static_cast<std::size_t>(i)]) << (8 * i);
    at += 8;
    return true;
}

class Vst3Instance;

class ComponentHandler final : public U::Implements<U::Directly<IComponentHandler>> {
public:
    explicit ComponentHandler(Vst3Instance* owner) : owner_(owner) {}
    tresult PLUGIN_API beginEdit(ParamID) override { return kResultOk; }
    tresult PLUGIN_API performEdit(ParamID id, ParamValue value) override;
    tresult PLUGIN_API endEdit(ParamID) override { return kResultOk; }
    tresult PLUGIN_API restartComponent(int32 flags) override;
    void detach() { owner_ = nullptr; }

private:
    Vst3Instance* owner_;
};

class Vst3Instance final : public PluginInstance {
public:
    Vst3Instance(PluginSpec spec, double rate, uint32_t max_block)
        : PluginInstance(std::move(spec)), rate_(rate), max_block_(max_block), in_l_(max_block), in_r_(max_block),
          out_l_(max_block), out_r_(max_block), scratch_in_(max_block, 0.0f), scratch_out_(max_block) {}

    ~Vst3Instance() override { shutdown(); }

    bool init(std::string& err) {
        TUID cid;
        if (!vst3_uid_from_string(spec_.plugin_id, cid)) {
            err = "invalid VST3 class UID '" + spec_.plugin_id + "' (expected 32 hex chars)";
            return false;
        }
        mod_ = load_vst3_module(spec_.path, err);
        if (!mod_) return false;
        IPluginFactory* f = mod_->factory;
        if (f->createInstance(cid, IComponent::iid, reinterpret_cast<void**>(&component_)) != kResultOk ||
            component_ == nullptr) {
            component_ = nullptr;
            err = "VST3 class " + spec_.plugin_id + " not found in " + spec_.path;
            return false;
        }
        if (component_->initialize(&host_application()) != kResultOk) {
            err = "IComponent::initialize failed";
            component_->release();
            component_ = nullptr;
            return false;
        }
        component_initialized_ = true;

        // Controller: the component itself, or a separate class.
        if (component_->queryInterface(IEditController::iid, reinterpret_cast<void**>(&controller_)) != kResultOk)
            controller_ = nullptr;
        if (controller_ == nullptr) {
            TUID ccid;
            if (component_->getControllerClassId(ccid) == kResultOk &&
                f->createInstance(ccid, IEditController::iid, reinterpret_cast<void**>(&controller_)) == kResultOk &&
                controller_ != nullptr) {
                if (controller_->initialize(&host_application()) != kResultOk) {
                    controller_->release();
                    controller_ = nullptr;
                } else {
                    separate_controller_ = true;
                }
            }
        }
        if (controller_ != nullptr) {
            handler_ = new ComponentHandler(this); // refcount 1 (ours)
            controller_->setComponentHandler(handler_);
            if (separate_controller_) {
                component_->queryInterface(IConnectionPoint::iid, reinterpret_cast<void**>(&cp_component_));
                controller_->queryInterface(IConnectionPoint::iid, reinterpret_cast<void**>(&cp_controller_));
                if (cp_component_ && cp_controller_) {
                    cp_component_->connect(cp_controller_);
                    cp_controller_->connect(cp_component_);
                    connected_ = true;
                }
            }
        }

        if (component_->queryInterface(IAudioProcessor::iid, reinterpret_cast<void**>(&processor_)) != kResultOk ||
            processor_ == nullptr) {
            processor_ = nullptr;
            err = "the VST3 component has no IAudioProcessor";
            return false;
        }
        if (processor_->canProcessSampleSize(kSample32) != kResultOk) {
            err = "the VST3 processor cannot process 32-bit float";
            return false;
        }

        // State: saved state, else sync the controller with the component's defaults.
        if (!spec_.state.empty()) {
            if (!load_state(spec_.state, err)) return false;
        } else if (controller_ != nullptr && separate_controller_) {
            auto s = owned(new VecStream());
            if (component_->getState(s) == kResultOk) {
                s->seek(0, IBStream::kIBSeekSet, nullptr);
                controller_->setComponentState(s);
            }
        }
        spec_.state.clear();
        spec_.state.shrink_to_fit();

        if (!setup_buses(err)) return false;

        ProcessSetup setup{kRealtime, kSample32, static_cast<int32>(max_block_), rate_};
        if (processor_->setupProcessing(setup) != kResultOk) {
            err = "IAudioProcessor::setupProcessing failed";
            return false;
        }
        if (component_->setActive(true) != kResultOk) {
            err = "IComponent::setActive failed";
            return false;
        }
        active_ = true;
        processor_->setProcessing(true); // may return kNotImplemented
        processing_ = true;
        return true;
    }

    void process(float* l, float* r, uint32_t frames, const MidiEvent* events, uint32_t event_count,
                 const RtTransport& t) noexcept override {
        if (!active_) { // a re-activation failed: effects pass through, instruments are silent
            if (spec_.instrument) {
                std::memset(l, 0, sizeof(float) * frames);
                std::memset(r, 0, sizeof(float) * frames);
            }
            return;
        }
        params_.clear();
        ParamChange pc;
        while (pop_param(pc)) {
            int32 qi = 0, pi = 0;
            if (IParamValueQueue* q = params_.addParameterData(pc.id, qi)) q->addPoint(0, pc.value, pi);
        }
        events_.clear();
        for (uint32_t i = 0; i < event_count; ++i) {
            const MidiEvent& m = events[i];
            const uint8_t kind = m.status & 0xF0;
            if (kind != 0x90 && kind != 0x80) continue;
            Event e{};
            e.busIndex = 0;
            e.sampleOffset = static_cast<int32>(m.offset);
            e.flags = Event::kIsLive;
            if (kind == 0x90) {
                e.type = Event::kNoteOnEvent;
                e.noteOn = NoteOnEvent{0, static_cast<int16>(m.data1), 0.0f, static_cast<float>(m.data2) / 127.0f, 0, -1};
            } else {
                e.type = Event::kNoteOffEvent;
                e.noteOff = NoteOffEvent{0, static_cast<int16>(m.data1), 0.0f, -1, 0.0f};
            }
            events_.addEvent(e);
        }
        ctx_ = ProcessContext{};
        ctx_.state = ProcessContext::kTempoValid | ProcessContext::kProjectTimeMusicValid |
                     ProcessContext::kTimeSigValid | (t.playing ? ProcessContext::kPlaying : 0u);
        ctx_.sampleRate = t.sample_rate;
        ctx_.projectTimeSamples = t.position;
        ctx_.projectTimeMusic = t.beats();
        ctx_.tempo = t.tempo_bpm;
        ctx_.timeSigNumerator = 4;
        ctx_.timeSigDenominator = 4;

        if (main_in_channels_ == 2) {
            std::memcpy(in_l_.data(), l, sizeof(float) * frames);
            std::memcpy(in_r_.data(), r, sizeof(float) * frames);
        } else if (main_in_channels_ == 1) {
            for (uint32_t i = 0; i < frames; ++i) in_l_[i] = 0.5f * (l[i] + r[i]);
        }
        ProcessData d;
        d.processMode = kRealtime;
        d.symbolicSampleSize = kSample32;
        d.numSamples = static_cast<int32>(frames);
        d.numInputs = static_cast<int32>(in_bufs_.size());
        d.numOutputs = static_cast<int32>(out_bufs_.size());
        d.inputs = in_bufs_.empty() ? nullptr : in_bufs_.data();
        d.outputs = out_bufs_.data();
        d.inputParameterChanges = &params_;
        d.inputEvents = has_event_bus_ ? &events_ : nullptr;
        d.processContext = &ctx_;
        if (processor_->process(d) != kResultOk) {
            if (spec_.instrument) {
                std::memset(l, 0, sizeof(float) * frames);
                std::memset(r, 0, sizeof(float) * frames);
            }
            return;
        }
        std::memcpy(l, out_l_.data(), sizeof(float) * frames);
        std::memcpy(r, main_out_channels_ > 1 ? out_r_.data() : out_l_.data(), sizeof(float) * frames);
    }

    bool save_state(std::vector<uint8_t>& out, std::string& err) override {
        auto comp = owned(new VecStream());
        if (component_->getState(comp) != kResultOk) {
            err = "IComponent::getState failed";
            return false;
        }
        auto ctrl = owned(new VecStream());
        if (controller_ != nullptr && separate_controller_ && controller_->getState(ctrl) != kResultOk)
            ctrl->data().clear(); // controller state is optional
        out.clear();
        out.insert(out.end(), {'S', 'V', 'S', '3'});
        put_u32(out, 1);
        put_u64(out, comp->data().size());
        out.insert(out.end(), comp->data().begin(), comp->data().end());
        put_u64(out, ctrl->data().size());
        out.insert(out.end(), ctrl->data().begin(), ctrl->data().end());
        return true;
    }

    uint32_t latency() override { return processor_ != nullptr ? processor_->getLatencySamples() : 0; }

    std::unique_ptr<PluginEditor> open_editor(const std::string& title, std::string& err) override {
        if (controller_ == nullptr) {
            err = "the plugin has no edit controller";
            return nullptr;
        }
        IPlugView* view = controller_->createView(ViewType::kEditor);
        if (view == nullptr) {
            err = "the plugin has no editor";
            return nullptr;
        }
        return create_vst3_editor(view, title, err);
    }

    void shutdown() noexcept override {
        if (component_ == nullptr) return;
        if (processing_ && processor_) processor_->setProcessing(false);
        processing_ = false;
        if (active_) component_->setActive(false);
        active_ = false;
        if (connected_) {
            cp_component_->disconnect(cp_controller_);
            cp_controller_->disconnect(cp_component_);
            connected_ = false;
        }
        if (cp_component_) cp_component_->release();
        if (cp_controller_) cp_controller_->release();
        cp_component_ = cp_controller_ = nullptr;
        if (controller_ != nullptr) {
            controller_->setComponentHandler(nullptr);
            if (separate_controller_) controller_->terminate();
            controller_->release();
            controller_ = nullptr;
        }
        if (handler_ != nullptr) {
            handler_->detach();
            handler_->release();
            handler_ = nullptr;
        }
        if (processor_) processor_->release();
        processor_ = nullptr;
        if (component_initialized_) component_->terminate();
        component_->release();
        component_ = nullptr;
        mod_.reset();
    }

    // kLatencyChanged: the host must deactivate and re-activate the component
    // for the new latency to apply. Main thread, instance suspended (the RT
    // side cannot be inside process()).
    void restart() noexcept override {
        if (component_ == nullptr || processor_ == nullptr) return;
        if (processing_) processor_->setProcessing(false);
        processing_ = false;
        if (active_) component_->setActive(false);
        active_ = component_->setActive(true) == kResultOk;
        if (active_) {
            processor_->setProcessing(true);
            processing_ = true;
        }
    }

    // UI edits from the controller go to the processor (main thread).
    void ui_edit(ParamID id, ParamValue v) noexcept { push_param(ParamChange{id, v}); }

protected:
    void on_parameter_set(const ParamChange& c) noexcept override {
        if (controller_ != nullptr) controller_->setParamNormalized(c.id, c.value);
    }

private:
    bool load_state(const std::vector<uint8_t>& s, std::string& err) {
        std::size_t at = 8;
        uint64_t n = 0, m = 0;
        if (s.size() < 8 || std::memcmp(s.data(), "SVS3", 4) != 0 || !get_u64(s, at, n) || at + n > s.size()) {
            err = "saved VST3 state is not an SVS3 container";
            return false;
        }
        std::vector<uint8_t> comp(s.begin() + static_cast<std::ptrdiff_t>(at),
                                  s.begin() + static_cast<std::ptrdiff_t>(at + n));
        at += n;
        if (!get_u64(s, at, m) || at + m > s.size()) {
            err = "truncated SVS3 container";
            return false;
        }
        std::vector<uint8_t> ctrl(s.begin() + static_cast<std::ptrdiff_t>(at),
                                  s.begin() + static_cast<std::ptrdiff_t>(at + m));
        auto cs = owned(new VecStream(comp));
        if (component_->setState(cs) != kResultOk) {
            err = "IComponent::setState rejected the saved state";
            return false;
        }
        if (controller_ != nullptr) {
            auto cs2 = owned(new VecStream(std::move(comp)));
            controller_->setComponentState(cs2);
            if (separate_controller_ && !ctrl.empty()) {
                auto ks = owned(new VecStream(std::move(ctrl)));
                controller_->setState(ks);
            }
        }
        return true;
    }

    bool setup_buses(std::string& err) {
        const int32 n_in = component_->getBusCount(kAudio, kInput);
        const int32 n_out = component_->getBusCount(kAudio, kOutput);
        if (n_out < 1) {
            err = "the VST3 plugin has no audio output bus";
            return false;
        }
        if (!spec_.instrument && n_in < 1) {
            err = "the VST3 effect has no audio input bus";
            return false;
        }
        std::vector<SpeakerArrangement> ins(static_cast<std::size_t>(n_in)), outs(static_cast<std::size_t>(n_out));
        for (int32 i = 0; i < n_in; ++i) processor_->getBusArrangement(kInput, i, ins[static_cast<std::size_t>(i)]);
        for (int32 i = 0; i < n_out; ++i) processor_->getBusArrangement(kOutput, i, outs[static_cast<std::size_t>(i)]);
        if (n_in > 0) ins[0] = SpeakerArr::kStereo;
        outs[0] = SpeakerArr::kStereo;
        if (processor_->setBusArrangements(n_in > 0 ? ins.data() : nullptr, n_in, outs.data(), n_out) != kResultOk) {
            // Keep whatever the plugin chose; accept mono/stereo main buses.
            for (int32 i = 0; i < n_in; ++i) processor_->getBusArrangement(kInput, i, ins[static_cast<std::size_t>(i)]);
            for (int32 i = 0; i < n_out; ++i)
                processor_->getBusArrangement(kOutput, i, outs[static_cast<std::size_t>(i)]);
        }
        main_in_channels_ = n_in > 0 ? static_cast<uint32_t>(SpeakerArr::getChannelCount(ins[0])) : 0;
        main_out_channels_ = static_cast<uint32_t>(SpeakerArr::getChannelCount(outs[0]));
        if (main_out_channels_ < 1 || main_out_channels_ > 2 || main_in_channels_ > 2 ||
            (!spec_.instrument && main_in_channels_ < 1)) {
            err = "unsupported VST3 bus layout (main buses must be mono or stereo)";
            return false;
        }
        if (n_in > 0) component_->activateBus(kAudio, kInput, 0, true);
        component_->activateBus(kAudio, kOutput, 0, true);
        has_event_bus_ = component_->getBusCount(kEvent, kInput) > 0;
        if (has_event_bus_) component_->activateBus(kEvent, kInput, 0, true);

        // Buffers for every bus (the API wants them even for inactive buses).
        in_ptrs_.assign(static_cast<std::size_t>(n_in), {});
        out_ptrs_.assign(static_cast<std::size_t>(n_out), {});
        for (int32 i = 0; i < n_in; ++i) {
            const auto ch = static_cast<std::size_t>(SpeakerArr::getChannelCount(ins[static_cast<std::size_t>(i)]));
            auto& v = in_ptrs_[static_cast<std::size_t>(i)];
            v.assign(std::max<std::size_t>(ch, 1), scratch_in_.data());
            if (i == 0) {
                v[0] = in_l_.data();
                if (ch > 1) v[1] = in_r_.data();
            }
        }
        for (int32 i = 0; i < n_out; ++i) {
            const auto ch = static_cast<std::size_t>(SpeakerArr::getChannelCount(outs[static_cast<std::size_t>(i)]));
            auto& v = out_ptrs_[static_cast<std::size_t>(i)];
            v.assign(std::max<std::size_t>(ch, 1), scratch_out_.data());
            if (i == 0) {
                v[0] = out_l_.data();
                if (ch > 1) v[1] = out_r_.data();
            }
        }
        in_bufs_.assign(static_cast<std::size_t>(n_in), AudioBusBuffers{});
        out_bufs_.assign(static_cast<std::size_t>(n_out), AudioBusBuffers{});
        for (int32 i = 0; i < n_in; ++i) {
            auto& b = in_bufs_[static_cast<std::size_t>(i)];
            b.numChannels = SpeakerArr::getChannelCount(ins[static_cast<std::size_t>(i)]);
            b.channelBuffers32 = in_ptrs_[static_cast<std::size_t>(i)].data();
        }
        for (int32 i = 0; i < n_out; ++i) {
            auto& b = out_bufs_[static_cast<std::size_t>(i)];
            b.numChannels = SpeakerArr::getChannelCount(outs[static_cast<std::size_t>(i)]);
            b.channelBuffers32 = out_ptrs_[static_cast<std::size_t>(i)].data();
        }
        return true;
    }

    double rate_;
    uint32_t max_block_;
    std::shared_ptr<Vst3Module> mod_;
    IComponent* component_ = nullptr;
    IAudioProcessor* processor_ = nullptr;
    IEditController* controller_ = nullptr;
    IConnectionPoint* cp_component_ = nullptr;
    IConnectionPoint* cp_controller_ = nullptr;
    ComponentHandler* handler_ = nullptr;
    bool component_initialized_ = false;
    bool separate_controller_ = false;
    bool connected_ = false;
    bool active_ = false;
    bool processing_ = false;
    bool has_event_bus_ = false;
    uint32_t main_in_channels_ = 0, main_out_channels_ = 0;
    std::vector<float> in_l_, in_r_, out_l_, out_r_, scratch_in_, scratch_out_;
    std::vector<std::vector<float*>> in_ptrs_, out_ptrs_;
    std::vector<AudioBusBuffers> in_bufs_, out_bufs_;
    // RT-owned
    FixedEventList events_;
    FixedParamChanges params_;
    ProcessContext ctx_{};
};

// Only kLatencyChanged is acted on (re-activation on the main thread, see
// Vst3Instance::restart()); other flags are ignored as before.
tresult PLUGIN_API ComponentHandler::restartComponent(int32 flags) {
    if (owner_ != nullptr && (flags & kLatencyChanged) != 0) owner_->request_restart();
    return kResultOk;
}

tresult PLUGIN_API ComponentHandler::performEdit(ParamID id, ParamValue value) {
    if (owner_ != nullptr) owner_->ui_edit(id, value);
    return kResultOk;
}

} // namespace

std::shared_ptr<PluginInstance> create_vst3_instance(const PluginSpec& spec, double sample_rate, uint32_t max_block,
                                                     std::string& err) {
    if (spec.path.empty()) {
        err = "VST3 device has no bundle path";
        return nullptr;
    }
    auto inst = std::make_shared<Vst3Instance>(spec, sample_rate, max_block);
    if (!inst->init(err)) return nullptr;
    return inst;
}

} // namespace saine
