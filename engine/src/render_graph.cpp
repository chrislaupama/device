#include "render_graph.hpp"

#include <algorithm>
#include <cmath>
#include <cstring>
#include <limits>
#include <numbers>
#include <unordered_map>

namespace saine {

void constant_power_pan(float pan, float& left, float& right) noexcept {
    const double p = std::clamp(static_cast<double>(pan), -1.0, 1.0);
    const double theta = (p + 1.0) * (std::numbers::pi / 4.0);
    left = static_cast<float>(std::cos(theta));
    right = static_cast<float>(std::sin(theta));
    // Make the extremes exact so hard-panned material is bit-transparent.
    if (p <= -1.0) right = 0.0f;
    if (p >= 1.0) left = 0.0f;
}

int32_t build_render_graph(const se_session_desc& desc, const AssetLookup& lookup,
                           std::unique_ptr<RenderGraph>& out, std::string& err) {
    if (desc.track_count > 0 && desc.tracks == nullptr) {
        err = "session has track_count > 0 but tracks == NULL";
        return SE_ERR_INVALID_ARG;
    }
    std::vector<se_track_desc_v3> tracks(desc.track_count);
    for (uint32_t t = 0; t < desc.track_count; ++t) {
        const se_track_desc& a = desc.tracks[t];
        se_track_desc_v3& b = tracks[t];
        b = se_track_desc_v3{};
        b.track_id = a.track_id;
        b.gain = a.gain;
        b.pan = a.pan;
        b.mute = a.mute;
        b.solo = a.solo;
        b.clips = a.clips;
        b.clip_count = a.clip_count;
    }
    se_session_desc_v3 d3{};
    d3.tracks = tracks.data();
    d3.track_count = desc.track_count;
    d3.master_gain = desc.master_gain;
    return build_render_graph(d3, lookup, out, err);
}

int32_t build_render_graph(const se_session_desc_v2& desc, const AssetLookup& lookup,
                           std::unique_ptr<RenderGraph>& out, std::string& err) {
    if (desc.track_count > 0 && desc.tracks == nullptr) {
        err = "session has track_count > 0 but tracks == NULL";
        return SE_ERR_INVALID_ARG;
    }
    std::vector<se_track_desc_v3> tracks(desc.track_count);
    for (uint32_t t = 0; t < desc.track_count; ++t) {
        const se_track_desc_v2& a = desc.tracks[t];
        se_track_desc_v3& b = tracks[t];
        b = se_track_desc_v3{};
        b.track_id = a.track_id;
        b.gain = a.gain;
        b.pan = a.pan;
        b.mute = a.mute;
        b.solo = a.solo;
        b.instrument = a.instrument;
        b.clips = a.clips;
        b.clip_count = a.clip_count;
        b.notes = a.notes;
        b.note_count = a.note_count;
        b.synth = nullptr;
    }
    se_session_desc_v3 d3{};
    d3.tracks = tracks.data();
    d3.track_count = desc.track_count;
    d3.master_gain = desc.master_gain;
    return build_render_graph(d3, lookup, out, err);
}

int32_t build_render_graph(const se_session_desc_v3& desc, const AssetLookup& lookup,
                           std::unique_ptr<RenderGraph>& out, std::string& err) {
    if (desc.track_count > 0 && desc.tracks == nullptr) {
        err = "session has track_count > 0 but tracks == NULL";
        return SE_ERR_INVALID_ARG;
    }
    std::vector<se_track_desc_v4> tracks(desc.track_count);
    for (uint32_t t = 0; t < desc.track_count; ++t) {
        const se_track_desc_v3& a = desc.tracks[t];
        se_track_desc_v4& b = tracks[t];
        b = se_track_desc_v4{};
        b.track_id = a.track_id;
        b.gain = a.gain;
        b.pan = a.pan;
        b.mute = a.mute;
        b.solo = a.solo;
        b.instrument = a.instrument;
        b.clips = a.clips;
        b.clip_count = a.clip_count;
        b.notes = a.notes;
        b.note_count = a.note_count;
        b.synth = a.synth;
    }
    se_session_desc_v4 d4{};
    d4.tracks = tracks.data();
    d4.track_count = desc.track_count;
    d4.master_gain = desc.master_gain;
    return build_render_graph(d4, lookup, out, err);
}

int32_t build_render_graph(const se_session_desc_v4& desc, const AssetLookup& lookup,
                           std::unique_ptr<RenderGraph>& out, std::string& err) {
    if (desc.track_count > 0 && desc.tracks == nullptr) {
        err = "session has track_count > 0 but tracks == NULL";
        return SE_ERR_INVALID_ARG;
    }
    std::vector<se_track_desc_v5> tracks(desc.track_count);
    for (uint32_t t = 0; t < desc.track_count; ++t) {
        const se_track_desc_v4& a = desc.tracks[t];
        se_track_desc_v5& b = tracks[t];
        b = se_track_desc_v5{};
        b.track_id = a.track_id;
        b.gain = a.gain;
        b.pan = a.pan;
        b.mute = a.mute;
        b.solo = a.solo;
        b.instrument = a.instrument;
        b.arm = a.arm;
        b.monitor = a.monitor;
        b.input_mode = a.input_mode;
        b.input_channel = a.input_channel;
        b.clips = a.clips;
        b.clip_count = a.clip_count;
        b.notes = a.notes;
        b.note_count = a.note_count;
        b.synth = a.synth;
    }
    se_session_desc_v5 d5{};
    d5.tracks = tracks.data();
    d5.track_count = desc.track_count;
    d5.master_gain = desc.master_gain;
    return build_render_graph(d5, lookup, out, err);
}

namespace {

const char* validate_device(const se_device_desc& d) {
    if (d.device_id == 0) return "device_id must not be 0";
    if (d.format > SE_PLUGIN_CLAP) return "invalid plugin format";
    if (d.plugin_id == nullptr) return "plugin_id is NULL";
    if (d.state_len > 0 && d.state == nullptr) return "state_len > 0 but state == NULL";
    return nullptr;
}

} // namespace

int32_t build_render_graph(const se_session_desc_v5& desc, const AssetLookup& lookup,
                           std::unique_ptr<RenderGraph>& out, std::string& err) {
    if (desc.track_count > 0 && desc.tracks == nullptr) {
        err = "session has track_count > 0 but tracks == NULL";
        return SE_ERR_INVALID_ARG;
    }
    std::vector<se_track_desc_v6> tracks(desc.track_count);
    for (uint32_t t = 0; t < desc.track_count; ++t) {
        const se_track_desc_v5& a = desc.tracks[t];
        se_track_desc_v6& b = tracks[t];
        b = se_track_desc_v6{};
        std::memcpy(&b, &a, sizeof a); // v6 extends v5 field by field (checked in c_api.cpp)
        b.lanes = nullptr;
        b.lane_count = 0;
    }
    se_session_desc_v6 d6{};
    d6.tracks = tracks.data();
    d6.track_count = desc.track_count;
    d6.master_gain = desc.master_gain;
    return build_render_graph(d6, lookup, out, err);
}

const char* validate_lane(const se_auto_lane_desc& d) noexcept {
    if (d.target > SE_AUTO_PLUGIN) return "invalid automation target";
    if (d.target == SE_AUTO_SYNTH && d.synth_param >= SE_SYNTH_P_COUNT) return "invalid synth parameter";
    if (d.point_count > 0 && d.points == nullptr) return "point_count > 0 but points == NULL";
    float lo = -std::numeric_limits<float>::infinity(), hi = std::numeric_limits<float>::infinity();
    if (d.target == SE_AUTO_SYNTH) synth_param_range(d.synth_param, lo, hi);
    for (uint32_t i = 0; i < d.point_count; ++i) {
        const se_auto_point& q = d.points[i];
        if (i > 0 && q.frame < d.points[i - 1].frame) return "automation points must be sorted by frame";
        if (!std::isfinite(q.value) || !std::isfinite(q.curve)) return "automation point is not finite";
        if (q.curve < -1.0f || q.curve > 1.0f) return "automation curve must be -1..1";
        if (q.value < lo || q.value > hi) return "synth automation value outside the parameter's range";
    }
    return nullptr;
}

namespace {

uint32_t add_lane(RenderGraph& g, const se_auto_lane_desc& d) {
    AutoLane l;
    l.point_begin = static_cast<uint32_t>(g.auto_points.size());
    l.point_count = d.point_count;
    l.target = d.target;
    l.log = d.logarithmic != 0;
    l.synth_param = d.synth_param;
    l.param_id = d.param_id;
    l.device_id = d.device_id;
    const std::size_t at = g.auto_points.size();
    g.auto_points.resize(at + d.point_count);
    std::memcpy(g.auto_points.data() + at, d.points, sizeof(se_auto_point) * d.point_count);
    g.lanes.push_back(l);
    return static_cast<uint32_t>(g.lanes.size() - 1);
}

} // namespace

int32_t build_render_graph(const se_session_desc_v6& desc, const AssetLookup& lookup,
                           std::unique_ptr<RenderGraph>& out, std::string& err) {
    if (desc.track_count > 0 && desc.tracks == nullptr) {
        err = "session has track_count > 0 but tracks == NULL";
        return SE_ERR_INVALID_ARG;
    }
    if (desc.master_lane_count > 0 && desc.master_lanes == nullptr) {
        err = "session has master_lane_count > 0 but master_lanes == NULL";
        return SE_ERR_INVALID_ARG;
    }
    if (!std::isfinite(desc.master_gain)) {
        err = "master_gain is not finite";
        return SE_ERR_INVALID_ARG;
    }

    auto g = std::make_unique<RenderGraph>();
    g->master_gain = desc.master_gain;
    g->tracks.reserve(desc.track_count);
    g->synth_params.reserve(desc.track_count);

    bool any_solo = false;
    for (uint32_t t = 0; t < desc.track_count; ++t) any_solo |= desc.tracks[t].solo != 0;

    std::unordered_map<uint64_t, AssetRef> used; // dedupe keep-alive refs

    std::unordered_map<uint64_t, uint64_t> device_ids; // device id -> track id
    for (uint32_t t = 0; t < desc.track_count; ++t) {
        const se_track_desc_v6& td = desc.tracks[t];
        if (td.device_count > 0 && td.devices == nullptr) {
            err = "track " + std::to_string(td.track_id) + " has device_count > 0 but devices == NULL";
            return SE_ERR_INVALID_ARG;
        }
        const auto check_device = [&](const se_device_desc& d) -> int32_t {
            if (const char* why = validate_device(d)) {
                err = "track " + std::to_string(td.track_id) + " device " + std::to_string(d.device_id) + ": " + why;
                return SE_ERR_INVALID_ARG;
            }
            if (!device_ids.emplace(d.device_id, td.track_id).second) {
                err = "device id " + std::to_string(d.device_id) + " is used more than once";
                return SE_ERR_INVALID_ARG;
            }
            return SE_OK;
        };
        for (uint32_t i = 0; i < td.device_count; ++i)
            if (const int32_t rc = check_device(td.devices[i]); rc != SE_OK) return rc;
        if (td.instrument_plugin != nullptr && td.instrument != 0)
            if (const int32_t rc = check_device(*td.instrument_plugin); rc != SE_OK) return rc;
        if (td.clip_count > 0 && td.clips == nullptr) {
            err = "track " + std::to_string(td.track_id) + " has clip_count > 0 but clips == NULL";
            return SE_ERR_INVALID_ARG;
        }
        if (!std::isfinite(td.gain) || !std::isfinite(td.pan)) {
            err = "track " + std::to_string(td.track_id) + " has a non-finite gain or pan";
            return SE_ERR_INVALID_ARG;
        }
        if (td.note_count > 0 && td.notes == nullptr) {
            err = "track " + std::to_string(td.track_id) + " has note_count > 0 but notes == NULL";
            return SE_ERR_INVALID_ARG;
        }

        if (td.input_mode > SE_INPUT_STEREO) {
            err = "track " + std::to_string(td.track_id) + " has an invalid input_mode";
            return SE_ERR_INVALID_ARG;
        }

        se_synth_params params = default_synth_params();
        if (td.synth != nullptr) {
            params = *td.synth;
            std::memset(params._pad, 0, sizeof params._pad); // reserved bytes are ignored
            if (const char* why = validate_synth_params(params)) {
                err = "track " + std::to_string(td.track_id) + ": " + why;
                return SE_ERR_INVALID_ARG;
            }
        }

        if (td.lane_count > 0 && td.lanes == nullptr) {
            err = "track " + std::to_string(td.track_id) + " has lane_count > 0 but lanes == NULL";
            return SE_ERR_INVALID_ARG;
        }
        for (uint32_t i = 0; i < td.lane_count; ++i)
            if (const char* why = validate_lane(td.lanes[i])) {
                err = "track " + std::to_string(td.track_id) + " automation lane " + std::to_string(i) + ": " + why;
                return SE_ERR_INVALID_ARG;
            }

        TrackNode node;
        node.track_id = td.track_id;
        float pl = 0.0f, pr = 0.0f;
        constant_power_pan(td.pan, pl, pr);
        node.gain_l = td.gain * pl;
        node.gain_r = td.gain * pr;
        node.gain = td.gain;
        node.pan = td.pan;
        // Mute always wins; with any solo active only soloed tracks are audible.
        node.audible = td.mute == 0 && (!any_solo || td.solo != 0);
        node.clip_begin = static_cast<uint32_t>(g->clips.size());
        node.arm = td.arm != 0;
        node.monitor = td.monitor != 0;
        node.input_mode = td.input_mode;
        node.input_channel = td.input_channel;
        if (node.input_mode != SE_INPUT_NONE && (node.arm || node.monitor)) g->uses_input = true;

        for (uint32_t c = 0; c < td.clip_count; ++c) {
            const se_clip_desc& cd = td.clips[c];
            if (cd.source_offset < 0 || cd.length < 0) {
                err = "clip " + std::to_string(cd.clip_id) + " has a negative source_offset or length";
                return SE_ERR_INVALID_ARG;
            }
            if (!std::isfinite(cd.gain)) {
                err = "clip " + std::to_string(cd.clip_id) + " has a non-finite gain";
                return SE_ERR_INVALID_ARG;
            }
            if (cd.length > std::numeric_limits<int64_t>::max() - std::max<int64_t>(cd.timeline_start, 0)) {
                err = "clip " + std::to_string(cd.clip_id) + " extends past the end of the timeline";
                return SE_ERR_INVALID_ARG;
            }

            AssetRef asset;
            if (auto it = used.find(cd.asset_id); it != used.end()) {
                asset = it->second;
            } else {
                asset = lookup(cd.asset_id);
                if (!asset) {
                    err = "clip " + std::to_string(cd.clip_id) + " references unknown asset " +
                          std::to_string(cd.asset_id);
                    return SE_ERR_NOT_FOUND;
                }
                used.emplace(cd.asset_id, asset);
            }

            // Clamp the played range to the asset; drop empty clips.
            const auto asset_frames = static_cast<int64_t>(asset->frames);
            if (cd.source_offset >= asset_frames) continue;
            const int64_t len = std::min(cd.length, asset_frames - cd.source_offset);
            if (len <= 0) continue;

            ClipNode clip;
            clip.src_l = asset->channel(0);
            clip.src_r = asset->channel(1);
            clip.timeline_start = cd.timeline_start;
            clip.timeline_end = cd.timeline_start + len;
            clip.source_offset = cd.source_offset;
            clip.gain = cd.gain;
            g->clips.push_back(clip);
        }

        node.clip_count = static_cast<uint32_t>(g->clips.size()) - node.clip_begin;
        std::sort(g->clips.begin() + node.clip_begin, g->clips.end(),
                  [](const ClipNode& a, const ClipNode& b) { return a.timeline_start < b.timeline_start; });

        // Notes: validated always, kept only for instrument tracks.
        node.instrument = td.instrument != 0;
        node.note_begin = static_cast<uint32_t>(g->notes.size());
        for (uint32_t n = 0; n < td.note_count; ++n) {
            const se_note_desc& nd = td.notes[n];
            if (nd.pitch > 127 || nd.velocity == 0 || nd.velocity > 127) {
                err = "track " + std::to_string(td.track_id) + " note " + std::to_string(n) +
                      ": pitch must be 0..127 and velocity 1..127";
                return SE_ERR_INVALID_ARG;
            }
            if (!node.instrument || nd.end <= nd.start) continue;
            g->notes.push_back(NoteEvent{nd.start, nd.end, nd.pitch, nd.velocity});
        }
        node.note_count = static_cast<uint32_t>(g->notes.size()) - node.note_begin;
        node.fx_begin = static_cast<uint32_t>(g->fx.size());
        node.fx_count = td.device_count;
        for (uint32_t i = 0; i < td.device_count; ++i)
            g->fx.push_back(FxSlot{nullptr, td.devices[i].device_id, td.devices[i].enabled != 0, 0});
        node.has_instrument_plugin = node.instrument && td.instrument_plugin != nullptr;
        node.instrument_plugin_enabled = node.has_instrument_plugin && td.instrument_plugin->enabled != 0;
        if (node.fx_count > 0 || node.has_instrument_plugin) g->has_plugins = true;

        // Automation lanes (last lane per target wins; empty lanes ignored).
        {
            const auto last_of = [&](auto&& pred) -> const se_auto_lane_desc* {
                for (uint32_t i = td.lane_count; i-- > 0;)
                    if (td.lanes[i].point_count > 0 && pred(td.lanes[i])) return &td.lanes[i];
                return nullptr;
            };
            if (const auto* d = last_of([](const se_auto_lane_desc& l) { return l.target == SE_AUTO_VOLUME; }))
                node.vol_lane = static_cast<int32_t>(add_lane(*g, *d));
            if (const auto* d = last_of([](const se_auto_lane_desc& l) { return l.target == SE_AUTO_PAN; }))
                node.pan_lane = static_cast<int32_t>(add_lane(*g, *d));
            node.synth_lane_begin = static_cast<uint32_t>(g->lanes.size());
            if (node.instrument && !node.has_instrument_plugin)
                for (uint16_t sp = 0; sp < SE_SYNTH_P_COUNT; ++sp)
                    if (const auto* d = last_of([sp](const se_auto_lane_desc& l) {
                            return l.target == SE_AUTO_SYNTH && l.synth_param == sp;
                        }))
                        add_lane(*g, *d);
            node.synth_lane_count = static_cast<uint32_t>(g->lanes.size()) - node.synth_lane_begin;
            // Plugin lanes of one device: one per parameter (the last given).
            const auto device_lanes = [&](uint64_t device_id, uint32_t& begin, uint32_t& count) {
                begin = static_cast<uint32_t>(g->lanes.size());
                for (uint32_t i = 0; i < td.lane_count; ++i) {
                    const se_auto_lane_desc& d = td.lanes[i];
                    if (d.target != SE_AUTO_PLUGIN || d.device_id != device_id || d.point_count == 0) continue;
                    bool later = false;
                    for (uint32_t j = i + 1; j < td.lane_count && !later; ++j)
                        later = td.lanes[j].target == SE_AUTO_PLUGIN && td.lanes[j].device_id == device_id &&
                                td.lanes[j].param_id == d.param_id && td.lanes[j].point_count > 0;
                    if (!later) add_lane(*g, d);
                }
                count = static_cast<uint32_t>(g->lanes.size()) - begin;
                if (count > 0) g->has_plugin_lanes = true;
            };
            if (node.has_instrument_plugin)
                device_lanes(td.instrument_plugin->device_id, node.iplug_lane_begin, node.iplug_lane_count);
            for (uint32_t i = 0; i < node.fx_count; ++i) {
                FxSlot& slot = g->fx[node.fx_begin + i];
                device_lanes(slot.device_id, slot.lane_begin, slot.lane_count);
            }
        }
        std::stable_sort(g->notes.begin() + node.note_begin, g->notes.end(), [](const NoteEvent& a, const NoteEvent& b) {
            return a.start != b.start ? a.start < b.start : a.pitch < b.pitch;
        });
        g->tracks.push_back(node);
        g->synth_params.push_back(params);
    }

    for (uint32_t i = desc.master_lane_count; i-- > 0;) {
        const se_auto_lane_desc& d = desc.master_lanes[i];
        if (const char* why = validate_lane(d)) {
            err = std::string("master automation lane ") + std::to_string(i) + ": " + why;
            return SE_ERR_INVALID_ARG;
        }
    }
    for (uint32_t i = desc.master_lane_count; i-- > 0;)
        if (desc.master_lanes[i].target == SE_AUTO_VOLUME && desc.master_lanes[i].point_count > 0) {
            g->master_vol_lane = static_cast<int32_t>(add_lane(*g, desc.master_lanes[i]));
            break;
        }

    g->assets.reserve(used.size());
    for (auto& [id, ref] : used) g->assets.push_back(std::move(ref));

    g->synth_refs.resize(g->tracks.size());
    g->synths.assign(g->tracks.size(), nullptr);

    g->meter_count = g->tracks.size();
    g->meters = std::make_unique<TrackMeter[]>(g->meter_count);
    for (std::size_t i = 0; i < g->meter_count; ++i) g->meters[i].track_id = g->tracks[i].track_id;

    out = std::move(g);
    return SE_OK;
}

std::unique_ptr<RenderGraph> clone_render_graph(const RenderGraph& g) {
    auto c = std::make_unique<RenderGraph>();
    c->master_gain = g.master_gain;
    c->uses_input = g.uses_input;
    c->tracks = g.tracks;
    c->clips = g.clips;
    c->notes = g.notes;
    c->fx = g.fx;
    c->has_plugins = g.has_plugins;
    c->lanes = g.lanes;
    c->auto_points = g.auto_points;
    c->master_vol_lane = g.master_vol_lane;
    c->has_plugin_lanes = g.has_plugin_lanes;
    c->synth_params = g.synth_params;
    c->assets = g.assets;
    c->synth_refs = g.synth_refs;
    c->synths = g.synths;
    c->meter_count = g.meter_count;
    c->meters = std::make_unique<TrackMeter[]>(c->meter_count);
    for (std::size_t i = 0; i < c->meter_count; ++i) {
        const TrackMeter& from = g.meters[i];
        TrackMeter& to = c->meters[i];
        to.track_id = from.track_id;
        to.peak_l.store(from.peak_l.load(std::memory_order_relaxed), std::memory_order_relaxed);
        to.peak_r.store(from.peak_r.load(std::memory_order_relaxed), std::memory_order_relaxed);
        for (uint32_t k = 0; k < kPeekBuckets; ++k) {
            to.win_l[k].store(from.win_l[k].load(std::memory_order_relaxed), std::memory_order_relaxed);
            to.win_r[k].store(from.win_r[k].load(std::memory_order_relaxed), std::memory_order_relaxed);
        }
    }
    c->pdc = g.pdc;
    c->line_refs = g.line_refs;
    c->lines = g.lines;
    return c;
}

} // namespace saine
