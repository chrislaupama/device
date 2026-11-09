// Shared harness for built-in device tests (docs/devices.md, "Testing").
//
// DeviceRig wraps one BuiltinInstance outside any engine: feed it signals
// (impulse, sine, steps, noise), set parameters by id, and measure the
// output. every_builtin_is_rt_safe() in test_builtin.cpp runs every
// registered device through the same allocation and robustness checks, so a
// new device only needs tests for its own behaviour.
//
// DeviceSession publishes v6 sessions whose tracks hold built-in devices,
// for end-to-end checks through the engine (chains, bypass, automation,
// sidechains).
#pragma once

#include "devices/builtin.hpp"
#include "test_harness.hpp"
#include "test_util.hpp"

#include <algorithm>
#include <cmath>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <memory>
#include <string>
#include <utility>
#include <vector>

namespace devtest {

using Signal = std::vector<float>;

inline Signal impulse(std::size_t n, float amp = 1.0f, std::size_t at = 0) {
    Signal s(n, 0.0f);
    if (at < n) s[at] = amp;
    return s;
}
inline Signal tone(std::size_t n, double hz, double rate, double amp) {
    Signal s(n);
    for (std::size_t i = 0; i < n; ++i) s[i] = static_cast<float>(amp * std::sin(2.0 * 3.14159265358979323846 * hz * static_cast<double>(i) / rate));
    return s;
}
inline Signal noise(std::size_t n, float amp, uint32_t seed = 1) {
    Signal s(n);
    for (std::size_t i = 0; i < n; ++i) {
        seed = seed * 1664525u + 1013904223u;
        s[i] = amp * (static_cast<float>(seed >> 8) / 8388608.0f - 1.0f);
    }
    return s;
}
inline Signal concat(Signal a, const Signal& b) {
    a.insert(a.end(), b.begin(), b.end());
    return a;
}
inline double db(double g) { return g > 1e-12 ? 20.0 * std::log10(g) : -240.0; }
inline double peak(const Signal& s, std::size_t from = 0, std::size_t to = SIZE_MAX) {
    double m = 0.0;
    for (std::size_t i = from; i < std::min(to, s.size()); ++i) m = std::max(m, static_cast<double>(std::fabs(s[i])));
    return m;
}
inline double rms(const Signal& s, std::size_t from = 0, std::size_t to = SIZE_MAX) {
    double a = 0.0;
    std::size_t n = 0;
    for (std::size_t i = from; i < std::min(to, s.size()); ++i, ++n) a += static_cast<double>(s[i]) * s[i];
    return n > 0 ? std::sqrt(a / static_cast<double>(n)) : 0.0;
}
inline bool all_finite(const Signal& s) {
    for (float v : s)
        if (!std::isfinite(v)) return false;
    return true;
}

// The se_builtin_state blob for values (+ optional sidechain source).
inline std::vector<uint8_t> state_blob(const std::vector<float>& values, uint64_t sidechain = 0) {
    se_builtin_state h{SE_BUILTIN_STATE_MAGIC, static_cast<uint32_t>(values.size()), sidechain};
    std::vector<uint8_t> out(sizeof h + 4 * values.size());
    std::memcpy(out.data(), &h, sizeof h);
    if (!values.empty()) std::memcpy(out.data() + sizeof h, values.data(), 4 * values.size());
    return out;
}

inline uint32_t param_index(const saine::BuiltinDescriptor& d, const char* id) {
    for (std::size_t i = 0; i < d.params.size(); ++i)
        if (std::strcmp(d.params[i], id) == 0) return static_cast<uint32_t>(i);
    std::fprintf(stderr, "unknown parameter %s of %s\n", id, d.id);
    std::abort();
}

class DeviceRig {
public:
    // `defaults`: initial values in registry order (the Rust definition's
    // defaults; each device test keeps its own copy).
    DeviceRig(const char* id, std::vector<float> defaults, double rate = 48000.0, uint32_t block = 256)
        : rate_(rate), block_(block), values_(std::move(defaults)) {
        saine::PluginSpec spec;
        spec.device_id = 1;
        spec.format = saine::PluginFormat::Builtin;
        spec.plugin_id = id;
        spec.state = state_blob(values_);
        std::string err;
        inst_ = saine::create_builtin_instance(spec, rate, block, err);
        if (!inst_) {
            std::fprintf(stderr, "create_builtin_instance(%s): %s\n", id, err.c_str());
            std::abort();
        }
        b_ = inst_->as_builtin();
        sc_l_.assign(block, 0.0f);
        sc_r_.assign(block, 0.0f);
    }

    saine::BuiltinInstance& instance() { return *b_; }
    const saine::BuiltinDescriptor& desc() const { return b_->descriptor(); }
    double rate() const { return rate_; }
    uint32_t block() const { return block_; }

    void set(const char* id, float v) {
        const uint32_t i = param_index(desc(), id);
        values_[i] = v;
        b_->set_value(i, v);
    }
    float get(const char* id) const { return values_[param_index(desc(), id)]; }
    // Transport state passed to the device (playing locks synced LFOs to the beat).
    void set_playing(bool on) { playing_ = on; }
    void reset() { b_->processor().reset(); }

    // Process l/r (same length) in blocks of block(); `sc` optional sidechain.
    // `events` are automation events at absolute frames (sorted).
    std::pair<Signal, Signal> run(Signal l, Signal r, const Signal* sc_l = nullptr, const Signal* sc_r = nullptr,
                                  const std::vector<std::pair<std::size_t, std::pair<uint32_t, float>>>& events = {}) {
        saine::RtTransport tr;
        tr.sample_rate = rate_;
        tr.playing = playing_;
        std::vector<saine::ParamEvent> ev;
        ev.reserve(saine::PluginInstance::kMaxParamEvents);
        std::size_t ei = 0;
        for (std::size_t pos = 0; pos < l.size(); pos += block_) {
            const auto n = static_cast<uint32_t>(std::min<std::size_t>(block_, l.size() - pos));
            ev.clear();
            while (ei < events.size() && events[ei].first < pos + n) {
                ev.push_back(saine::ParamEvent{static_cast<uint32_t>(events[ei].first - pos), events[ei].second.first,
                                               events[ei].second.second});
                ++ei;
            }
            if (sc_l != nullptr) {
                std::memcpy(sc_l_.data(), sc_l->data() + pos, 4 * n);
                std::memcpy(sc_r_.data(), (sc_r != nullptr ? sc_r : sc_l)->data() + pos, 4 * n);
                b_->set_sidechain(sc_l_.data(), sc_r_.data());
            }
            tr.position = static_cast<int64_t>(pos);
            b_->run(l.data() + pos, r.data() + pos, n, nullptr, 0, tr, ev.data(), static_cast<uint32_t>(ev.size()));
        }
        return {std::move(l), std::move(r)};
    }
    Signal run_mono(const Signal& x) { return run(x, x).first; }

    std::vector<float> readouts() {
        std::vector<float> r(desc().readouts.size());
        b_->readouts(r.data(), static_cast<uint32_t>(r.size()));
        return r;
    }

private:
    double rate_;
    uint32_t block_;
    std::vector<float> values_;
    bool playing_ = false;
    std::shared_ptr<saine::PluginInstance> inst_;
    saine::BuiltinInstance* b_ = nullptr;
    Signal sc_l_, sc_r_;
};

// v6 session builder for tracks that play a clip through built-in devices.
struct DeviceSession {
    struct Track {
        se_track_desc_v6 d{};
        std::vector<se_clip_desc> clips;
        std::vector<se_device_desc> devices;
        std::vector<std::vector<uint8_t>> states;
        std::vector<std::string> ids;
        std::vector<se_auto_lane_desc> lanes;
        std::vector<std::vector<se_auto_point>> points;
    };
    std::vector<Track> tracks;
    std::vector<se_track_desc_v6> descs;

    Track& add_track(uint64_t id, uint64_t asset, int64_t length, float gain = 1.0f) {
        Track t;
        t.d.track_id = id;
        t.d.gain = gain;
        if (asset != 0) t.clips.push_back(testutil::clip(id * 100, asset, 0, 0, length));
        tracks.push_back(std::move(t));
        return tracks.back();
    }
    static void add_device(Track& t, uint64_t device_id, const char* builtin_id, const std::vector<float>& values,
                           bool enabled = true, uint64_t sidechain = 0) {
        t.ids.emplace_back(builtin_id);
        t.states.push_back(state_blob(values, sidechain));
        se_device_desc d{};
        d.device_id = device_id;
        d.format = SE_PLUGIN_BUILTIN;
        d.enabled = enabled ? 1 : 0;
        t.devices.push_back(d);
    }
    static void add_lane(Track& t, uint64_t device_id, uint32_t param, std::vector<se_auto_point> pts, bool log = false) {
        se_auto_lane_desc l{};
        l.target = SE_AUTO_BUILTIN;
        l.device_id = device_id;
        l.param_id = param;
        l.logarithmic = log ? 1 : 0;
        t.points.push_back(std::move(pts));
        t.lanes.push_back(l);
    }
    int32_t publish(se_engine* e) {
        descs.clear();
        for (Track& t : tracks) {
            for (std::size_t i = 0; i < t.devices.size(); ++i) {
                t.devices[i].plugin_id = t.ids[i].c_str();
                t.devices[i].state = t.states[i].data();
                t.devices[i].state_len = t.states[i].size();
            }
            for (std::size_t i = 0; i < t.lanes.size(); ++i) {
                t.lanes[i].points = t.points[i].data();
                t.lanes[i].point_count = static_cast<uint32_t>(t.points[i].size());
            }
            se_track_desc_v6 d = t.d;
            d.clips = t.clips.data();
            d.clip_count = static_cast<uint32_t>(t.clips.size());
            d.devices = t.devices.data();
            d.device_count = static_cast<uint32_t>(t.devices.size());
            d.lanes = t.lanes.data();
            d.lane_count = static_cast<uint32_t>(t.lanes.size());
            descs.push_back(d);
        }
        se_session_desc_v6 s{};
        s.tracks = descs.data();
        s.track_count = static_cast<uint32_t>(descs.size());
        s.master_gain = 1.0f;
        return se_session_publish_v6(e, &s);
    }
};

} // namespace devtest
