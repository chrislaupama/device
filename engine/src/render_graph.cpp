#include "render_graph.hpp"

#include <algorithm>
#include <cmath>
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
    if (!std::isfinite(desc.master_gain)) {
        err = "master_gain is not finite";
        return SE_ERR_INVALID_ARG;
    }

    auto g = std::make_unique<RenderGraph>();
    g->master_gain = desc.master_gain;
    g->tracks.reserve(desc.track_count);

    bool any_solo = false;
    for (uint32_t t = 0; t < desc.track_count; ++t) any_solo |= desc.tracks[t].solo != 0;

    std::unordered_map<uint64_t, AssetRef> used; // dedupe keep-alive refs

    for (uint32_t t = 0; t < desc.track_count; ++t) {
        const se_track_desc& td = desc.tracks[t];
        if (td.clip_count > 0 && td.clips == nullptr) {
            err = "track " + std::to_string(td.track_id) + " has clip_count > 0 but clips == NULL";
            return SE_ERR_INVALID_ARG;
        }
        if (!std::isfinite(td.gain) || !std::isfinite(td.pan)) {
            err = "track " + std::to_string(td.track_id) + " has a non-finite gain or pan";
            return SE_ERR_INVALID_ARG;
        }

        TrackNode node;
        node.track_id = td.track_id;
        float pl = 0.0f, pr = 0.0f;
        constant_power_pan(td.pan, pl, pr);
        node.gain_l = td.gain * pl;
        node.gain_r = td.gain * pr;
        // Mute always wins; with any solo active only soloed tracks are audible.
        node.audible = td.mute == 0 && (!any_solo || td.solo != 0);
        node.clip_begin = static_cast<uint32_t>(g->clips.size());

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
        g->tracks.push_back(node);
    }

    g->assets.reserve(used.size());
    for (auto& [id, ref] : used) g->assets.push_back(std::move(ref));

    g->meter_count = g->tracks.size();
    g->meters = std::make_unique<TrackMeter[]>(g->meter_count);
    for (std::size_t i = 0; i < g->meter_count; ++i) g->meters[i].track_id = g->tracks[i].track_id;

    out = std::move(g);
    return SE_OK;
}

} // namespace saine
