#include "builtin.hpp"

#include <algorithm>
#include <cmath>
#include <cstring>

namespace saine {

// ---------------------------------------------------------------- registry
// One line per built-in device; the descriptor lives next to its processor.
extern const BuiltinDescriptor kCompressorDescriptor;
extern const BuiltinDescriptor kLfoDescriptor;
extern const BuiltinDescriptor kRackDescriptor;
extern const BuiltinDescriptor kDistortionDescriptor;
extern const BuiltinDescriptor kTapeSaturationDescriptor;
extern const BuiltinDescriptor kSoftClipperDescriptor;
extern const BuiltinDescriptor kBitCrusherDescriptor;
extern const BuiltinDescriptor kWidenerDescriptor;
extern const BuiltinDescriptor kUtilityDescriptor;
extern const BuiltinDescriptor kChorusDescriptor;
extern const BuiltinDescriptor kCharacterModDescriptor;
extern const BuiltinDescriptor kFlangerDescriptor;
extern const BuiltinDescriptor kPhaserDescriptor;
extern const BuiltinDescriptor kTremoloDescriptor;
extern const BuiltinDescriptor kAutopannerDescriptor;
extern const BuiltinDescriptor kRingModDescriptor;
extern const BuiltinDescriptor kSynthV2Descriptor;
extern const BuiltinDescriptor kDeClipperDescriptor;
extern const BuiltinDescriptor kDeEsserDescriptor;
extern const BuiltinDescriptor kDeHummerDescriptor;
extern const BuiltinDescriptor kDeNoiserDescriptor;

namespace {

const BuiltinDescriptor* const kBuiltinRegistry[] = {
    &kCompressorDescriptor,
    &kLfoDescriptor,
    &kRackDescriptor,
    &kDistortionDescriptor,
    &kTapeSaturationDescriptor,
    &kSoftClipperDescriptor,
    &kBitCrusherDescriptor,
    &kWidenerDescriptor,
    &kUtilityDescriptor,
    &kChorusDescriptor,
    &kCharacterModDescriptor,
    &kFlangerDescriptor,
    &kPhaserDescriptor,
    &kTremoloDescriptor,
    &kAutopannerDescriptor,
    &kRingModDescriptor,
    &kSynthV2Descriptor,
    &kDeClipperDescriptor, &kDeEsserDescriptor, &kDeHummerDescriptor, &kDeNoiserDescriptor,
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
    const uint8_t* routes = nullptr;
    uint32_t route_count = 0;
    return parse_builtin_state(data, len, values, count, sidechain, routes, route_count);
}

const char* parse_builtin_state(const uint8_t* data, std::size_t len, const uint8_t*& values, uint32_t& count,
                                uint64_t& sidechain, const uint8_t*& routes, uint32_t& route_count) noexcept {
    values = nullptr;
    count = 0;
    sidechain = 0;
    routes = nullptr;
    route_count = 0;
    if (data == nullptr || len == 0) return nullptr;
    if (len < sizeof(se_builtin_state)) return "built-in state shorter than its header";
    uint32_t magic;
    std::memcpy(&magic, data, 4);
    std::size_t header = sizeof(se_builtin_state);
    if (magic == SE_BUILTIN_STATE_MAGIC) {
        se_builtin_state h;
        std::memcpy(&h, data, sizeof h);
        count = h.value_count;
        sidechain = h.sidechain_track_id;
    } else if (magic == SE_BUILTIN_STATE_MAGIC2) {
        if (len < sizeof(se_builtin_state_v2)) return "built-in state shorter than its header";
        se_builtin_state_v2 h;
        std::memcpy(&h, data, sizeof h);
        header = sizeof h;
        count = h.value_count;
        sidechain = h.sidechain_track_id;
        route_count = h.route_count;
    } else {
        return "built-in state has a wrong magic";
    }
    if (len != header + std::size_t{4} * count + sizeof(se_mod_route) * std::size_t{route_count}) {
        count = route_count = 0;
        sidechain = 0;
        return "built-in state length does not match value_count";
    }
    values = data + header;
    for (uint32_t i = 0; i < count; ++i) {
        float v;
        std::memcpy(&v, values + 4 * i, 4);
        if (!std::isfinite(v)) return "built-in state value is not finite";
    }
    if (route_count > 0) routes = values + std::size_t{4} * count;
    for (uint32_t i = 0; i < route_count; ++i) {
        se_mod_route r;
        std::memcpy(&r, routes + sizeof r * i, sizeof r);
        if (r.kind > SE_MOD_PLUGIN) return "modulation route has an unknown kind";
        if (!std::isfinite(r.min) || !std::isfinite(r.max) || !std::isfinite(r.p_min) || !std::isfinite(r.p_max) ||
            !std::isfinite(r.p_skew) || !std::isfinite(r.p_step))
            return "modulation route range is not finite";
    }
    return nullptr;
}

float mod_to_norm(const BuiltinMod& m, float v) noexcept {
    if (!(m.max > m.min)) return 0.0f;
    v = std::clamp(v, m.min, m.max);
    float n;
    if (m.log) {
        n = m.min > 0.0f ? std::log(v / m.min) / std::log(m.max / m.min) : 0.0f;
    } else {
        n = std::clamp((v - m.min) / (m.max - m.min), 0.0f, 1.0f);
        if (m.skew != 1.0f && n > 0.0f) n = std::pow(n, m.skew);
    }
    return std::isfinite(n) ? std::clamp(n, 0.0f, 1.0f) : 0.0f;
}

float mod_from_norm(const BuiltinMod& m, float n) noexcept {
    if (!(m.max > m.min)) return std::isfinite(m.min) ? m.min : 0.0f;
    n = std::isfinite(n) ? std::clamp(n, 0.0f, 1.0f) : 0.0f;
    float v;
    if (m.log && m.min > 0.0f) {
        v = m.min * std::pow(m.max / m.min, n); // max > min > 0
    } else {
        if (m.skew != 1.0f && m.skew > 0.0f && n > 0.0f) n = std::pow(n, 1.0f / m.skew);
        v = m.min + (m.max - m.min) * n;
    }
    v = std::clamp(v, m.min, m.max);
    if (m.step > 0.0f && !m.log) v = std::clamp(m.min + std::floor((v - m.min) / m.step + 0.501f) * m.step, m.min, m.max);
    return v;
}

// ---------------------------------------------------------------- instance

BuiltinInstance::BuiltinInstance(PluginSpec spec, const BuiltinDescriptor& desc, std::unique_ptr<BuiltinProcessor> proc,
                                 double sample_rate, uint32_t max_block)
    : PluginInstance(std::move(spec)), desc_(desc), proc_(std::move(proc)),
      target_(new std::atomic<float>[std::max<std::size_t>(1, desc.params.size())]), current_(desc.params.size(), 0.0f),
      readout_atomic_(new std::atomic<float>[std::max<std::size_t>(1, desc.readouts.size())]),
      readout_scratch_(desc.readouts.size(), 0.0f), readout_segment_(desc.readouts.size(), 0.0f),
      midi_scratch_(desc.instrument ? kMaxEvents : 0) {
    for (std::size_t i = 0; i < desc_.params.size(); ++i) target_[i].store(0.0f, std::memory_order_relaxed);
    for (std::size_t i = 0; i < desc_.readouts.size(); ++i)
        readout_atomic_[i].store(i < desc_.idle_readouts.size() ? desc_.idle_readouts[i] : 0.0f, std::memory_order_relaxed);
    update_state(spec_.state.data(), spec_.state.size());
    for (std::size_t i = 0; i < desc_.params.size(); ++i) current_[i] = target_[i].load(std::memory_order_relaxed);
    modded_.assign(desc_.params.size(), 0.0f);
    mod_params_.assign(desc_.params.size(), 0.0f);
    proc_->prepare(sample_rate, max_block);
    proc_->reset();
    mod_outputs_ = proc_->mod_outputs();
    if (mod_outputs_ > 0) mod_out_.assign(2 * std::size_t{mod_outputs_} * (max_block / kGrid + 2), 0.0f);
}

void BuiltinInstance::run_modulation(uint32_t frames, const RtTransport& transport, const ParamEvent* params,
                                     uint32_t param_count) noexcept {
    if (mod_outputs_ == 0) return;
    const auto n_params = static_cast<uint32_t>(mod_params_.size());
    float* const cur = mod_params_.data();
    for (uint32_t i = 0; i < n_params; ++i) cur[i] = target_[i].load(std::memory_order_relaxed);
    const uint32_t cap = static_cast<uint32_t>(mod_out_.size() / (2 * std::size_t{mod_outputs_}));
    uint32_t e = 0;
    for (uint32_t k = 0, pos = 0; pos < frames && k < cap; ++k, pos += kGrid) {
        while (e < param_count && params[e].offset <= pos) {
            if (params[e].id < n_params) cur[params[e].id] = static_cast<float>(params[e].value);
            ++e;
        }
        RtTransport t = transport;
        if (t.playing) t.position += pos;
        BuiltinContext ctx;
        ctx.transport = &t;
        proc_->modulate(cur, std::min(kGrid, frames - pos), ctx, mod_out_.data() + 2 * std::size_t{k} * mod_outputs_);
    }
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

void BuiltinInstance::process(float* l, float* r, uint32_t frames, const MidiEvent* events, uint32_t event_count,
                              const RtTransport& transport,
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
    // runs with the values in force at its first frame. Modulation splits it
    // further at every grid point (SE_MOD_GRID_FRAMES). The block's readouts
    // keep, per readout, the segment value farthest from its idle value
    // (highest peak, deepest reduction). MIDI events go to the segment they
    // fall in, with segment-relative offsets.
    const bool modulated = mod_count_ > 0;
    float* const run_params = modulated ? modded_.data() : cur;
    MidiEvent* const seg_events = midi_scratch_.data();
    uint32_t pos = 0, e = 0, m = 0;
    while (pos < frames) {
        while (e < param_count && params[e].offset <= pos) {
            if (params[e].id < n_params) cur[params[e].id] = static_cast<float>(params[e].value);
            ++e;
        }
        uint32_t end = e < param_count ? std::min(frames, params[e].offset) : frames;
        if (modulated) {
            const uint32_t k = pos / kGrid;
            end = std::min(end, (k + 1) * kGrid);
            std::memcpy(run_params, cur, sizeof(float) * n_params);
            for (uint32_t j = 0; j < mod_count_; ++j) {
                const BuiltinMod& md = mods_[j];
                if (md.index >= n_params) continue;
                const float a = md.abs != nullptr ? md.abs[k] : std::nanf("");
                const float n = (std::isnan(a) ? mod_to_norm(md, cur[md.index]) : a) + (md.off != nullptr ? md.off[k] : 0.0f);
                run_params[md.index] = mod_from_norm(md, n);
            }
        }
        BuiltinContext seg = ctx;
        seg.offset = pos;
        if (seg.sc_l != nullptr) {
            seg.sc_l += pos;
            seg.sc_r += pos;
        }
        uint32_t nm = 0;
        for (; m < event_count && (events[m].offset < end || end == frames); ++m) {
            if (nm == midi_scratch_.size()) continue;
            MidiEvent ev = events[m];
            ev.offset = ev.offset > pos ? std::min(ev.offset, end - 1) - pos : 0;
            seg_events[nm++] = ev;
        }
        seg.events = nm > 0 ? seg_events : nullptr;
        seg.event_count = nm;
        if (pos == 0) {
            proc_->process(l, r, end, run_params, seg, ro);
        } else {
            float* const sr = readout_segment_.data();
            proc_->process(l + pos, r + pos, end - pos, run_params, seg, sr);
            for (std::size_t i = 0; i < readout_segment_.size(); ++i) {
                const float idle = i < desc_.idle_readouts.size() ? desc_.idle_readouts[i] : 0.0f;
                if (std::fabs(sr[i] - idle) > std::fabs(ro[i] - idle)) ro[i] = sr[i];
            }
        }
        pos = end;
    }
    for (std::size_t i = 0; i < readout_scratch_.size(); ++i) readout_atomic_[i].store(ro[i], std::memory_order_relaxed);
    sc_l_ = sc_r_ = nullptr;
    mods_ = nullptr;
    mod_count_ = 0;
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
    if (spec.instrument != d->instrument) {
        err = d->instrument ? "built-in instruments go in a track's instrument slot"
                            : "built-in effects can't be used as an instrument";
        return nullptr;
    }
    return std::make_shared<BuiltinInstance>(spec, *d, d->create(), sample_rate, max_block);
}

} // namespace saine
