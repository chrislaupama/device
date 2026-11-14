// Audio clip fades (se_session_publish_v11): the gain ramps, fades on looped
// and warped clips, clamping, validation, and no allocations on the render path.
#include "test_harness.hpp"
#include "test_util.hpp"

#include <algorithm>
#include <cmath>
#include <vector>

using namespace testutil;

namespace {

constexpr float kCentre = 0.70710678f; // constant-power pan at centre (mono sources here)

// One track per clip; loops, warps and fades reference the clips by id.
struct FadeSession {
    std::vector<se_clip_desc> clips;
    std::vector<se_track_desc_v6> tracks;
    std::vector<se_warp_point> points;
    std::vector<se_clip_warp_desc> warps;
    std::vector<se_clip_loop_desc> loops;
    std::vector<se_clip_fade_desc> fades;

    void add(uint64_t id, uint64_t asset, int64_t start, int64_t offset, int64_t length) {
        clips.push_back(clip(id, asset, start, offset, length));
    }
    void fade(uint64_t id, int64_t in, int64_t out) { fades.push_back({id, in, out}); }
    int32_t publish(se_engine* e) {
        tracks.assign(clips.size(), se_track_desc_v6{});
        for (std::size_t i = 0; i < clips.size(); ++i) {
            tracks[i].track_id = i + 1;
            tracks[i].gain = 1.0f;
            tracks[i].clips = &clips[i];
            tracks[i].clip_count = 1;
        }
        for (auto& w : warps) {
            w.points = points.data();
            w.point_count = static_cast<uint32_t>(points.size());
        }
        se_session_desc_v11 d{};
        d.v10.v9.v8.v7.v6.tracks = tracks.data();
        d.v10.v9.v8.v7.v6.track_count = static_cast<uint32_t>(tracks.size());
        d.v10.v9.v8.v7.v6.master_gain = 1.0f;
        d.v10.v9.v8.v7.warps = warps.empty() ? nullptr : warps.data();
        d.v10.v9.v8.v7.warp_count = static_cast<uint32_t>(warps.size());
        d.v10.v9.v8.loops = loops.empty() ? nullptr : loops.data();
        d.v10.v9.v8.loop_count = static_cast<uint32_t>(loops.size());
        d.fades = fades.empty() ? nullptr : fades.data();
        d.fade_count = static_cast<uint32_t>(fades.size());
        return se_session_publish_v11(e, &d);
    }
};

std::vector<float> left(const std::vector<float>& inter) {
    std::vector<float> l(inter.size() / 2);
    for (std::size_t i = 0; i < l.size(); ++i) l[i] = inter[2 * i];
    return l;
}

// The fade gain at clip-relative frame r of a clip `len` frames long.
float gain(int64_t r, int64_t len, int64_t in, int64_t out) {
    float g = 1.0f;
    if (r < in) g = static_cast<float>(r) / static_cast<float>(in);
    if (len - r < out) g *= static_cast<float>(len - r) / static_cast<float>(out);
    return g;
}

} // namespace

TEST_CASE(clip_fade_ramps_in_and_out) {
    EngineHandle e;
    const uint64_t a = load_asset(e, {constant(10000, 0.5f)});
    FadeSession s;
    s.add(1, a, 1000, 0, 8000);
    s.fade(1, 3000, 2000);
    REQUIRE(s.publish(e) == SE_OK);
    REQUIRE(se_transport_play(e) == SE_OK);
    const std::vector<float> got = left(render(e, 10000, 333)); // fade edges land mid-block
    float diff = 0.0f;
    for (int64_t r = 0; r < 8000; ++r)
        diff = std::max(diff, std::fabs(got[1000 + r] - 0.5f * kCentre * gain(r, 8000, 3000, 2000)));
    CHECK(diff < 1e-6f);
    CHECK_EQ(got[1000], 0.0f);                    // silent at the clip start
    CHECK(std::fabs(got[5000] - 0.5f * kCentre) < 1e-6f); // full level between the fades
    CHECK(std::fabs(got[8999] - 0.5f * kCentre / 2000.0f) < 1e-6f); // last frame: 1/2000
    CHECK_EQ(got[9000], 0.0f);
}

TEST_CASE(clip_fade_follows_the_timeline_of_a_looped_clip) {
    EngineHandle e;
    const std::vector<float> src = ramp(1000, 1e-3f);
    const uint64_t a = load_asset(e, {src});
    FadeSession s;
    s.add(1, a, 0, 0, 3000); // three passes of the asset
    s.loops.push_back({1, 0, 1000});
    s.fade(1, 0, 1500); // the fade-out spans the second and third passes
    REQUIRE(s.publish(e) == SE_OK);
    REQUIRE(se_transport_play(e) == SE_OK);
    const std::vector<float> got = left(render(e, 3000, 256));
    float diff = 0.0f;
    for (int64_t r = 0; r < 3000; ++r) diff = std::max(diff, std::fabs(got[r] - src[r % 1000] * kCentre * gain(r, 3000, 0, 1500)));
    CHECK(diff < 1e-6f);
}

TEST_CASE(clip_fade_shapes_a_warped_clip) {
    EngineHandle e;
    const std::vector<float> src = sine(20000, 300.0, 48000.0, 0.5);
    const uint64_t a = load_asset(e, {src});
    FadeSession s;
    s.add(1, a, 0, 0, 12000);
    s.points = {{0, 0.0}, {12000, 12000.0}};
    se_clip_warp_desc w{};
    w.clip_id = 1;
    w.mode = SE_WARP_REPITCH;
    s.warps.push_back(w);
    s.fade(1, 4000, 4000);
    REQUIRE(s.publish(e) == SE_OK);
    REQUIRE(se_transport_play(e) == SE_OK);
    const std::vector<float> got = left(render(e, 12000, 256));
    float diff = 0.0f;
    for (int64_t r = 0; r < 12000; ++r) diff = std::max(diff, std::fabs(got[r] - src[r] * kCentre * gain(r, 12000, 4000, 4000)));
    CHECK(diff < 1e-3f);
}

TEST_CASE(clip_fade_clamps_to_the_played_range) {
    EngineHandle e;
    const uint64_t a = load_asset(e, {constant(2000, 1.0f)});
    FadeSession s;
    s.add(1, a, 0, 0, 5000); // plays 2000 frames: the asset ends
    s.fade(1, 1500, 9000);   // fade-in kept, fade-out shortened to the 500 left
    REQUIRE(s.publish(e) == SE_OK);
    REQUIRE(se_transport_play(e) == SE_OK);
    const std::vector<float> got = left(render(e, 2500, 256));
    float diff = 0.0f;
    for (int64_t r = 0; r < 2000; ++r) diff = std::max(diff, std::fabs(got[r] - kCentre * gain(r, 2000, 1500, 500)));
    CHECK(diff < 1e-6f);
    CHECK_EQ(got[2100], 0.0f);
}

TEST_CASE(clip_fade_validation) {
    EngineHandle e;
    const uint64_t a = load_asset(e, {ramp(1000)});
    {
        FadeSession s;
        s.add(1, a, 0, 0, 1000);
        s.fade(1, -1, 0);
        CHECK_EQ(s.publish(e), SE_ERR_INVALID_ARG);
    }
    {
        FadeSession s;
        s.add(1, a, 0, 0, 1000);
        s.fade(2, 100, 100);
        CHECK_EQ(s.publish(e), SE_ERR_INVALID_ARG);
    }
    {
        FadeSession s;
        s.add(1, a, 0, 0, 1000);
        s.fade(1, 100, 0);
        s.fade(1, 0, 100);
        CHECK_EQ(s.publish(e), SE_ERR_INVALID_ARG);
    }
    {
        FadeSession s;
        s.add(1, a, 0, 0, 1000);
        s.fade(1, 0, 0); // no fade: fine
        CHECK_EQ(s.publish(e), SE_OK);
    }
    se_session_desc_v11 d{};
    d.fade_count = 1;
    CHECK_EQ(se_session_publish_v11(e, &d), SE_ERR_INVALID_ARG);
    CHECK_EQ(se_session_publish_v11(e, nullptr), SE_ERR_INVALID_ARG);
}

TEST_CASE(clip_fade_render_path_does_not_allocate) {
    EngineHandle e;
    const uint64_t a = load_asset(e, {sine(96000, 220.0, 48000.0, 0.3)});
    FadeSession s;
    for (uint64_t i = 0; i < 12; ++i) {
        const auto t0 = static_cast<int64_t>(i) * 700;
        s.add(i + 1, a, t0, 0, 20000 + static_cast<int64_t>(i) * 100);
        s.fade(i + 1, 300 + static_cast<int64_t>(i) * 211, 900 + static_cast<int64_t>(i) * 97);
    }
    REQUIRE(s.publish(e) == SE_OK);
    std::vector<float> buf(2 * 8192);
    REQUIRE(se_transport_play(e) == SE_OK);
    REQUIRE(se_render_offline(e, buf.data(), 256) == SE_OK);
    const long allocs = rt_alloc_count();
    const long frees = rt_free_count();
    const uint32_t sizes[] = {256, 1, 333, 8192, 64, 255};
    for (int i = 0; i < 300; ++i) {
        if (i % 97 == 50) se_transport_seek(e, i * 170);
        RtAllocGuard g;
        REQUIRE(se_render_offline(e, buf.data(), sizes[i % 6]) == SE_OK);
    }
    CHECK_EQ(rt_alloc_count() - allocs, 0);
    CHECK_EQ(rt_free_count() - frees, 0);
}
