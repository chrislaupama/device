#include "builtin.hpp"

#include <algorithm>
#include <cmath>
#include <cstring>

namespace saine {

// ---------------------------------------------------------------- registry
// One line per built-in device; the descriptor lives next to its processor.
extern const BuiltinDescriptor kCompressorDescriptor;
extern const BuiltinDescriptor kChorusDescriptor;
extern const BuiltinDescriptor kCharacterModDescriptor;
extern const BuiltinDescriptor kFlangerDescriptor;
extern const BuiltinDescriptor kPhaserDescriptor;
extern const BuiltinDescriptor kTremoloDescriptor;
extern const BuiltinDescriptor kAutopannerDescriptor;
extern const BuiltinDescriptor kRingModDescriptor;
extern const BuiltinDescriptor kTranceGateDescriptor;
extern const BuiltinDescriptor kShaperDescriptor;

namespace {

const BuiltinDescriptor* const kBuiltinRegistry[] = {
    &kCompressorDescriptor,
    &kChorusDescriptor,
    &kCharacterModDescriptor,
    &kFlangerDescriptor,
    &kPhaserDescriptor,
    &kTremoloDescriptor,
    &kAutopannerDescriptor,
    &kRingModDescriptor,
    &kTranceGateDescriptor,
    &kShaperDescriptor,
};

struct CRegistry {
    std::vector<BuiltinDescriptor> list;
    std::vector<se_builtin_desc> c;
    CRegistry() {
        for (const BuiltinDescriptor* d : kBuiltinRegistry) list.push_back(*d);
        for (const BuiltinDescriptor& d : list)
            c.push_back(se_builtin_desc{d.id, d.params.data(), d.readouts.data(), static_cast<uint32_t>(d.params.size()),
                                        static_cast<uint32_t>(d.readouts.size()), d.defaults.data()});
    }
};

const CRegistry& registry() {
    static const CRegistry r;
    return r;
}

} // namespace

std::span<const BuiltinDescriptor> builtin_registry() noexcept { return registry().list; }

const BuiltinDescriptor* find_builtin(std::string_view id) noexcept {
    for (const BuiltinDescriptor& d : registry().list)
        if (id == d.id) return &d;
    return nullptr;
}

const se_builtin_desc* builtin_registry_c(std::size_t& count) noexcept {
    count = registry().c.size();
    return registry().c.data();
}

const char* parse_builtin_state(const uint8_t* data, std::size_t len, const uint8_t*& values, uint32_t& count,
                                uint64_t& sidechain) noexcept {
    values = nullptr;
    count = 0;
    sidechain = 0;
    if (data == nullptr || len == 0) return nullptr;
    if (len < sizeof(se_builtin_state)) return "built-in state shorter than its header";
    se_builtin_state h;
    std::memcpy(&h, data, sizeof h);
    if (h.magic != SE_BUILTIN_STATE_MAGIC) return "built-in state has a wrong magic";
    if (len != sizeof h + std::size_t{4} * h.value_count) return "built-in state length does not match value_count";
    values = data + sizeof h;
    count = h.value_count;
    sidechain = h.sidechain_track_id;
    for (uint32_t i = 0; i < count; ++i) {
        float v;
        std::memcpy(&v, values + 4 * i, 4);
        if (!std::isfinite(v)) return "built-in state value is not finite";
    }
    return nullptr;
}

// ---------------------------------------------------------------- instance

BuiltinInstance::BuiltinInstance(PluginSpec spec, const BuiltinDescriptor& desc, std::unique_ptr<BuiltinProcessor> proc,
                                 double sample_rate, uint32_t max_block)
    : PluginInstance(std::move(spec)), desc_(desc), proc_(std::move(proc)),
      target_(new std::atomic<float>[std::max<std::size_t>(1, desc.params.size())]), current_(desc.params.size(), 0.0f),
      readout_atomic_(new std::atomic<float>[std::max<std::size_t>(1, desc.readouts.size())]),
      readout_scratch_(desc.readouts.size(), 0.0f), readout_segment_(desc.readouts.size(), 0.0f),
      latency_values_(desc.params.size(), 0.0f) {
    for (std::size_t i = 0; i < desc_.params.size(); ++i) target_[i].store(0.0f, std::memory_order_relaxed);
    for (std::size_t i = 0; i < desc_.readouts.size(); ++i)
        readout_atomic_[i].store(i < desc_.idle_readouts.size() ? desc_.idle_readouts[i] : 0.0f, std::memory_order_relaxed);
    update_state(spec_.state.data(), spec_.state.size());
    for (std::size_t i = 0; i < desc_.params.size(); ++i) current_[i] = target_[i].load(std::memory_order_relaxed);
    proc_->prepare(sample_rate, max_block);
    proc_->reset();
}

void BuiltinInstance::set_value(uint32_t index, float value) noexcept {
    if (index < param_count() && std::isfinite(value)) target_[index].store(value, std::memory_order_relaxed);
}

float BuiltinInstance::value(uint32_t index) const noexcept {
    return index < param_count() ? target_[index].load(std::memory_order_relaxed) : 0.0f;
}

void BuiltinInstance::update_state(const uint8_t* data, std::size_t len) noexcept {
    const uint8_t* values = nullptr;
    uint32_t count = 0;
    uint64_t sc = 0;
    if (parse_builtin_state(data, len, values, count, sc) != nullptr || values == nullptr) return;
    count = std::min(count, param_count());
    for (uint32_t i = 0; i < count; ++i) {
        float v;
        std::memcpy(&v, values + 4 * i, 4);
        target_[i].store(v, std::memory_order_relaxed);
    }
}

uint32_t BuiltinInstance::readouts(float* out, uint32_t capacity) const noexcept {
    const auto n = static_cast<uint32_t>(desc_.readouts.size());
    for (uint32_t i = 0; i < n && i < capacity; ++i) out[i] = readout_atomic_[i].load(std::memory_order_relaxed);
    return n;
}

void BuiltinInstance::process(float* l, float* r, uint32_t frames, const MidiEvent*, uint32_t, const RtTransport& transport,
                              const ParamEvent* params, uint32_t param_count) noexcept {
    const auto n_params = static_cast<uint32_t>(current_.size());
    float* const cur = current_.data();
    for (uint32_t i = 0; i < n_params; ++i) cur[i] = target_[i].load(std::memory_order_relaxed);
    BuiltinContext ctx;
    ctx.sc_l = sc_l_;
    ctx.sc_r = sc_r_;
    ctx.transport = &transport;
    float* const ro = readout_scratch_.data();
    // Automation events split the block (sorted by offset): every segment
    // runs with the values in force at its first frame. The block's readouts
    // keep, per readout, the segment value farthest from its idle value
    // (highest peak, deepest reduction).
    uint32_t pos = 0, e = 0;
    while (pos < frames) {
        while (e < param_count && params[e].offset <= pos) {
            if (params[e].id < n_params) cur[params[e].id] = static_cast<float>(params[e].value);
            ++e;
        }
        const uint32_t end = e < param_count ? std::min(frames, params[e].offset) : frames;
        BuiltinContext seg = ctx;
        seg.offset = pos;
        if (seg.sc_l != nullptr) {
            seg.sc_l += pos;
            seg.sc_r += pos;
        }
        if (pos == 0) {
            proc_->process(l, r, end, cur, seg, ro);
        } else {
            float* const sr = readout_segment_.data();
            proc_->process(l + pos, r + pos, end - pos, cur, seg, sr);
            for (std::size_t i = 0; i < readout_segment_.size(); ++i) {
                const float idle = i < desc_.idle_readouts.size() ? desc_.idle_readouts[i] : 0.0f;
                if (std::fabs(sr[i] - idle) > std::fabs(ro[i] - idle)) ro[i] = sr[i];
            }
        }
        pos = end;
    }
    for (std::size_t i = 0; i < readout_scratch_.size(); ++i) readout_atomic_[i].store(ro[i], std::memory_order_relaxed);
    sc_l_ = sc_r_ = nullptr;
}

uint32_t BuiltinInstance::latency() {
    for (std::size_t i = 0; i < latency_values_.size(); ++i) latency_values_[i] = target_[i].load(std::memory_order_relaxed);
    return proc_->latency_for(latency_values_.data());
}

bool BuiltinInstance::save_state(std::vector<uint8_t>& out, std::string&) {
    se_builtin_state h{SE_BUILTIN_STATE_MAGIC, param_count(), 0};
    out.resize(sizeof h + 4 * std::size_t{param_count()});
    std::memcpy(out.data(), &h, sizeof h);
    for (uint32_t i = 0; i < param_count(); ++i) {
        const float v = value(i);
        std::memcpy(out.data() + sizeof h + 4 * i, &v, 4);
    }
    return true;
}

std::unique_ptr<PluginEditor> BuiltinInstance::open_editor(const std::string&, std::string& err) {
    err = "built-in devices have no plugin window";
    return nullptr;
}

void BuiltinInstance::params(std::vector<ParamInfo>& out) {
    out.clear();
    for (uint32_t i = 0; i < param_count(); ++i) {
        ParamInfo p;
        p.id = i;
        p.name = desc_.params[i];
        p.min = -1e30;
        p.max = 1e30;
        out.push_back(std::move(p));
    }
}

bool BuiltinInstance::param_range(uint32_t id, double& min, double& max) {
    // Built-in lanes carry plain values (SE_AUTO_BUILTIN): no mapping.
    min = 0.0;
    max = 1.0;
    return id < param_count();
}

std::shared_ptr<PluginInstance> create_builtin_instance(const PluginSpec& spec, double sample_rate, uint32_t max_block,
                                                        std::string& err) {
    const BuiltinDescriptor* d = find_builtin(spec.plugin_id);
    if (d == nullptr) {
        err = "unknown built-in device \"" + spec.plugin_id + "\"";
        return nullptr;
    }
    if (spec.instrument) {
        err = "built-in effects can't be used as an instrument";
        return nullptr;
    }
    return std::make_shared<BuiltinInstance>(spec, *d, d->create(), sample_rate, max_block);
}

} // namespace saine
