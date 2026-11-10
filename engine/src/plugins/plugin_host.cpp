#include "plugin_host.hpp"

#include "../devices/builtin.hpp"

#include <algorithm>

namespace dstudio {

PluginHost::PluginHost(double sample_rate, uint32_t max_block) : sample_rate_(sample_rate), max_block_(max_block) {}

PluginHost::~PluginHost() { shutdown_all(); }

void PluginHost::destroy(std::shared_ptr<PluginInstance>& inst) noexcept {
    if (!inst) return;
    inst->shutdown();
    inst.reset(); // a CLAP main-queue callback may briefly hold a reference; it only calls on_main_thread()
}

void PluginHost::begin_publish() {
    for (auto& [id, e] : entries_) e.used = false;
}

PluginSpec PluginHost::spec_of(const se_device_desc& d, bool instrument, const char* track_name) {
    PluginSpec key;
    key.device_id = d.device_id;
    key.format = static_cast<PluginFormat>(d.format);
    key.plugin_id = d.plugin_id != nullptr ? d.plugin_id : "";
    key.path = d.path != nullptr && d.format != SE_PLUGIN_AU ? d.path : "";
    key.name = d.name != nullptr ? d.name : key.plugin_id;
    key.track = track_name != nullptr ? track_name : "";
    key.instrument = instrument;
    return key;
}

PluginInstance* PluginHost::use(const se_device_desc& d, bool instrument, const char* track_name) {
    const PluginSpec key = spec_of(d, instrument, track_name);

    auto it = entries_.find(d.device_id);
    if (it != entries_.end()) {
        Entry& e = it->second;
        if (e.spec.same_plugin(key)) {
            e.used = true;
            e.spec.name = key.name;
            e.spec.track = key.track;
            if (e.inst && d.format == SE_PLUGIN_BUILTIN) {
                e.inst->update_state(d.state, d.state_len);
                if (BuiltinInstance* b = e.inst->as_builtin()) b->bind_samples(assets_);
            }
            return e.inst.get();
        }
        // Same id, different plugin or role: replace the instance.
        if (auto ed = editors_.find(d.device_id); ed != editors_.end()) {
            ed->second->close();
            editors_.erase(ed);
        }
        if (e.inst) pending_.push_back(std::move(e.inst));
        entries_.erase(it);
    }
    return create(d, key);
}

PluginInstance* PluginHost::prepare(const se_device_desc& d, bool instrument, const char* track_name, std::string& err) {
    const PluginSpec key = spec_of(d, instrument, track_name);
    if (auto it = entries_.find(d.device_id); it != entries_.end() && it->second.spec.same_plugin(key)) {
        err = it->second.error;
        return it->second.inst.get();
    }
    PluginInstance* inst = use(d, instrument, track_name);
    if (inst == nullptr) err = entries_.at(d.device_id).error;
    return inst;
}

PluginInstance* PluginHost::create(const se_device_desc& d, const PluginSpec& key) {
    Entry e;
    e.spec = key;
    e.used = true;
    if (detached_ && d.format != SE_PLUGIN_BUILTIN) { // built-in devices have no thread affinity
        e.error = "the plugin was not prepared on the main thread";
        entries_.emplace(d.device_id, std::move(e));
        return nullptr;
    }
    PluginSpec full = key;
    if (d.state != nullptr && d.state_len > 0) full.state.assign(d.state, d.state + d.state_len);
    std::string err;
    try {
        e.inst = create_plugin_instance(full, sample_rate_, max_block_, err);
    } catch (const std::exception& ex) {
        err = std::string("exception while loading: ") + ex.what();
        e.inst.reset();
    }
    instantiations_.fetch_add(1);
    if (!e.inst) e.error = err.empty() ? "could not load the plugin" : err;
    if (e.inst) {
        e.inst->refresh_latency(); // cached for delay compensation
        e.inst->attach_bus(&bus_);
        if (BuiltinInstance* b = e.inst->as_builtin()) b->bind_samples(assets_);
    }
    PluginInstance* raw = e.inst.get();
    entries_.emplace(d.device_id, std::move(e));
    return raw;
}

void PluginHost::retire(uint64_t device_id, Entry& e, uint64_t seq) {
    if (auto ed = editors_.find(device_id); ed != editors_.end()) {
        ed->second->close();
        editors_.erase(ed);
    }
    if (e.inst) retired_.push_back(Retired{std::move(e.inst), seq});
}

void PluginHost::end_publish(uint64_t seq) {
    for (auto& inst : pending_) retired_.push_back(Retired{std::move(inst), seq});
    pending_.clear();
    for (auto it = entries_.begin(); it != entries_.end();) {
        if (!it->second.used) {
            retire(it->first, it->second, seq);
            it = entries_.erase(it);
        } else {
            ++it;
        }
    }
}

void PluginHost::collect(uint64_t rt_active) {
    for (auto& [id, e] : entries_)
        if (BuiltinInstance* b = e.inst ? e.inst->as_builtin() : nullptr) b->reclaim();
    if (detached_ || retired_.empty() || rt_active == kBusy) return;
    for (auto it = retired_.begin(); it != retired_.end();) {
        if (rt_active == kIdle || rt_active >= it->seq) {
            destroy(it->inst);
            it = retired_.erase(it);
        } else {
            ++it;
        }
    }
}

void PluginHost::idle(uint64_t rt_active) {
    collect(rt_active);
    if (detached_) return;
    for (auto& [id, e] : entries_)
        if (e.inst) e.inst->on_main_thread();
    for (auto it = editors_.begin(); it != editors_.end();)
        it = it->second->is_open() ? std::next(it) : editors_.erase(it);
}

bool PluginHost::service_latencies() {
    bool changed = false;
    if (detached_) return changed;
    for (auto& [id, e] : entries_) {
        if (!e.inst) continue;
        if (e.inst->take_restart_request()) {
            e.inst->suspend();
            e.inst->restart();
            e.inst->resume();
        }
        changed |= e.inst->refresh_latency();
    }
    return changed;
}

void PluginHost::shutdown_all() {
    for (auto& [id, ed] : editors_) ed->close();
    editors_.clear();
    for (auto& r : retired_) destroy(r.inst);
    retired_.clear();
    for (auto& inst : pending_) destroy(inst);
    pending_.clear();
    for (auto& [id, e] : entries_) destroy(e.inst);
    entries_.clear();
}

bool PluginHost::is_live(uint64_t device_id) const { return entries_.contains(device_id); }

void PluginHost::load_errors(std::vector<std::pair<uint64_t, std::string>>& out) const {
    out.clear();
    for (const auto& [id, e] : entries_)
        if (!e.inst) out.emplace_back(id, e.error);
    std::sort(out.begin(), out.end());
}

std::size_t PluginHost::save_states() {
    saved_.clear();
    std::vector<uint64_t> ids;
    for (const auto& [id, e] : entries_)
        if (e.inst) ids.push_back(id);
    std::sort(ids.begin(), ids.end());
    for (uint64_t id : ids) {
        std::vector<uint8_t> bytes;
        std::string err;
        if (entries_.at(id).inst->save_state(bytes, err)) saved_.emplace_back(id, std::move(bytes));
    }
    return saved_.size();
}

int32_t PluginHost::save_state_of(uint64_t device_id, std::string& err) {
    saved_.clear();
    auto it = entries_.find(device_id);
    if (it == entries_.end() || !it->second.inst) {
        err = "no live plugin instance for this device";
        return SE_ERR_NOT_FOUND;
    }
    std::vector<uint8_t> bytes;
    if (!it->second.inst->save_state(bytes, err)) return SE_ERR_STATE;
    saved_.emplace_back(device_id, std::move(bytes));
    return SE_OK;
}

bool PluginHost::saved_state(std::size_t index, uint64_t& id, const uint8_t*& data, std::size_t& len) const {
    if (index >= saved_.size()) return false;
    id = saved_[index].first;
    data = saved_[index].second.data();
    len = saved_[index].second.size();
    return true;
}

bool PluginHost::flush_pending() const {
    if (detached_) return false;
    for (const auto& [id, e] : entries_)
        if (e.inst && e.inst->wants_flush()) return true;
    return false;
}

void PluginHost::flush_params() {
    for (auto& [id, e] : entries_)
        if (e.inst && e.inst->wants_flush()) e.inst->flush_params();
}

PluginInstance* PluginHost::instance(uint64_t device_id) const {
    auto it = entries_.find(device_id);
    return it == entries_.end() ? nullptr : it->second.inst.get();
}

int32_t PluginHost::editor_open(uint64_t device_id, std::string& err) {
    auto it = entries_.find(device_id);
    if (it == entries_.end() || !it->second.inst) {
        err = it == entries_.end() ? "no such device" : "the plugin is not loaded: " + it->second.error;
        return SE_ERR_NOT_FOUND;
    }
    if (auto ed = editors_.find(device_id); ed != editors_.end()) {
        if (ed->second->is_open()) {
            ed->second->bring_to_front();
            return SE_OK;
        }
        editors_.erase(ed);
    }
    const PluginSpec& s = it->second.spec;
    const std::string title = s.track.empty() ? s.name : s.name + ": " + s.track;
    auto editor = it->second.inst->open_editor(title, err);
    if (!editor) {
        if (err.empty()) err = "the plugin has no editor";
        return SE_ERR_UNSUPPORTED_FORMAT;
    }
    editors_[device_id] = std::move(editor);
    return SE_OK;
}

void PluginHost::editor_close(uint64_t device_id) {
    if (auto ed = editors_.find(device_id); ed != editors_.end()) {
        ed->second->close();
        editors_.erase(ed);
    }
}

bool PluginHost::editor_is_open(uint64_t device_id) {
    auto ed = editors_.find(device_id);
    if (ed == editors_.end()) return false;
    if (ed->second->is_open()) return true;
    editors_.erase(ed); // closed by the user
    return false;
}

bool PluginHost::latency(uint64_t device_id, uint32_t& frames) {
    PluginInstance* inst = instance(device_id);
    frames = inst != nullptr ? inst->latency() : 0;
    return inst != nullptr;
}

} // namespace dstudio
