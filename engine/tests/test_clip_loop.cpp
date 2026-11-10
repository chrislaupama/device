// Audio clip looping (se_session_publish_v8): the content a looped clip plays,
// warped loops, validation, and no allocations on the render path.
#include "test_harness.hpp"
#include "test_hooks.hpp"
#include "test_util.hpp"

#include <algorithm>
#include <chrono>
#include <cmath>
#include <thread>
#include <vector>

using namespace testutil;

namespace {

constexpr double kRate = 48000.0;
constexpr float kCentre = 0.70710678f; // constant-power pan at centre (mono sources here)

// One track per clip; loops and warps reference the clips by id.
struct LoopSession {
    std::vector<se_clip_desc> clips;
    std::vector<se_track_desc_v6> tracks;
    std::vector<std::vector<se_warp_point>> points;
    std::vector<se_clip_warp_desc> warps;
    std::vector<se_clip_loop_desc> loops;

    void add(uint64_t id, uint64_t asset, int64_t start, int64_t offset, int64_t length) {
        clips.push_back(clip(id, asset, start, offset, length));
    }
    void warp(uint64_t id, uint8_t mode, std::vector<se_warp_point> p) {
        points.push_back(std::move(p));
        se_clip_warp_desc w{};
        w.clip_id = id;
        w.mode = mode;
        warps.push_back(w);
    }
    void loop(uint64_t id, int64_t start, int64_t end) { loops.push_back({id, start, end}); }
    int32_t publish(se_engine* e) {
        tracks.assign(clips.size(), se_track_desc_v6{});
        for (std::size_t i = 0; i < clips.size(); ++i) {
            tracks[i].track_id = i + 1;
            tracks[i].gain = 1.0f;
            tracks[i].clips = &clips[i];
            tracks[i].clip_count = 1;
        }
        for (std::size_t i = 0; i < warps.size(); ++i) {
            warps[i].points = points[i].data();
            warps[i].point_count = static_cast<uint32_t>(points[i].size());
        }
        se_session_desc_v8 d{};
        d.v7.v6.tracks = tracks.data();
        d.v7.v6.track_count = static_cast<uint32_t>(tracks.size());
        d.v7.v6.master_gain = 1.0f;
        d.v7.warps = warps.empty() ? nullptr : warps.data();
        d.v7.warp_count = static_cast<uint32_t>(warps.size());
        d.loops = loops.empty() ? nullptr : loops.data();
        d.loop_count = static_cast<uint32_t>(loops.size());
        return se_session_publish_v8(e, &d);
    }
};

std::vector<float> left(const std::vector<float>& inter) {
    std::vector<float> l(inter.size() / 2);
    for (std::size_t i = 0; i < l.size(); ++i) l[i] = inter[2 * i];
    return l;
}

// What a clip starting at 0 with this loop plays at frame r (content frame).
int64_t content(int64_t r, int64_t ls, int64_t le) {
    return r < le ? r : ls + (r - le) % (le - ls);
}

bool attach_renders(se_engine* e) {
    for (int i = 0; i < 1000; ++i) {
        std::size_t n = 1;
        if (se_warp_renders_pending(e, &n) != SE_OK) return false;
        if (n == 0) return se_plugin_idle(e) == SE_OK;
        std::this_thread::sleep_for(std::chrono::milliseconds(10));
    }
    return false;
}

} // namespace

TEST_CASE(clip_loop_plays_first_pass_then_repeats) {
    EngineHandle e;
    const std::vector<float> src = ramp(4000, 1e-4f);
    const uint64_t a = load_asset(e, {src});
    LoopSession s;
    s.add(1, a, 0, 100, 9000);
    s.loop(1, 200, 700);
    REQUIRE(s.publish(e) == SE_OK);
    REQUIRE(se_transport_play(e) == SE_OK);
    const std::vector<float> got = left(render(e, 9500, 333)); // wraps land mid-block
    float diff = 0.0f;
    for (int64_t r = 0; r < 9000; ++r)
        diff = std::max(diff, std::fabs(got[r] - src[100 + content(r, 200, 700)] * kCentre));
    CHECK(diff < 1e-6f);
    for (int64_t r = 9000; r < 9500; ++r) CHECK_EQ(got[r], 0.0f);
}

TEST_CASE(clip_loop_runs_past_the_asset_end) {
    EngineHandle e;
    const std::vector<float> src = ramp(1000, 1e-3f);
    const uint64_t a = load_asset(e, {src});
    LoopSession s;
    s.add(1, a, 500, 0, 3000); // three passes of the whole asset
    s.loop(1, 0, 5000);        // clamped to the asset: 0..1000
    REQUIRE(s.publish(e) == SE_OK);
    REQUIRE(se_transport_play(e) == SE_OK);
    const std::vector<float> got = left(render(e, 4000, 256));
    float diff = 0.0f;
    for (int64_t r = 0; r < 3000; ++r) diff = std::max(diff, std::fabs(got[500 + r] - src[r % 1000] * kCentre));
    CHECK(diff < 1e-6f);
    CHECK_EQ(got[3500], 0.0f);
}

TEST_CASE(clip_loop_validation) {
    EngineHandle e;
    const uint64_t a = load_asset(e, {ramp(1000)});
    {
        LoopSession s;
        s.add(1, a, 0, 0, 1000);
        s.loop(1, 300, 300);
        CHECK_EQ(s.publish(e), SE_ERR_INVALID_ARG);
    }
    {
        LoopSession s;
        s.add(1, a, 0, 0, 1000);
        s.loop(1, -1, 300);
        CHECK_EQ(s.publish(e), SE_ERR_INVALID_ARG);
    }
    {
        LoopSession s;
        s.add(1, a, 0, 0, 1000);
        s.loop(2, 0, 300);
        CHECK_EQ(s.publish(e), SE_ERR_INVALID_ARG);
    }
    {
        LoopSession s;
        s.add(1, a, 0, 0, 1000);
        s.loop(1, 0, 300);
        s.loop(1, 0, 400);
        CHECK_EQ(s.publish(e), SE_ERR_INVALID_ARG);
    }
    se_session_desc_v8 d{};
    d.loop_count = 1;
    CHECK_EQ(se_session_publish_v8(e, &d), SE_ERR_INVALID_ARG);
    CHECK_EQ(se_session_publish_v8(e, nullptr), SE_ERR_INVALID_ARG);
}

TEST_CASE(clip_loop_warped_repitch_matches_one_to_one) {
    EngineHandle e;
    const std::vector<float> src = sine(20000, 300.0, kRate, 0.5);
    const uint64_t a = load_asset(e, {src});
    LoopSession s;
    s.add(1, a, 0, 0, 12000);
    s.warp(1, SE_WARP_REPITCH, {{0, 0.0}, {12000, 12000.0}});
    s.loop(1, 1000, 4000);
    REQUIRE(s.publish(e) == SE_OK);
    REQUIRE(se_transport_play(e) == SE_OK);
    const std::vector<float> got = left(render(e, 12000, 256));
    float diff = 0.0f;
    for (int64_t r = 0; r < 12000; ++r) diff = std::max(diff, std::fabs(got[r] - src[content(r, 1000, 4000)] * kCentre));
    CHECK(diff < 1e-3f);
}

TEST_CASE(clip_loop_warped_render_repeats_its_loop) {
    EngineHandle e;
    const uint64_t a = load_asset(e, {sine(96000, 440.0, kRate, 0.5)});
    LoopSession s;
    s.add(1, a, 0, 0, 80000);
    s.warp(1, SE_WARP_STRETCH, {{0, 0.0}, {80000, 64000.0}});
    s.loop(1, 6000, 30000);
    REQUIRE(s.publish(e) == SE_OK);
    REQUIRE(attach_renders(e));
    REQUIRE(se_transport_play(e) == SE_OK);
    const std::vector<float> got = left(render(e, 80000, 256));
    // The render covers the content to the loop end; every repeat is a copy of the loop.
    float diff = 0.0f;
    for (int64_t r = 30000; r < 80000; ++r) diff = std::max(diff, std::fabs(got[r] - got[content(r, 6000, 30000)]));
    CHECK(diff < 1e-6f);
    float peak = 0.0f;
    for (int64_t r = 40000; r < 80000; ++r) peak = std::max(peak, std::fabs(got[r]));
    CHECK(peak > 0.2f);
}

TEST_CASE(clip_loop_render_path_does_not_allocate) {
    EngineHandle e;
    dstudio::testing::set_warp_renders_enabled(e, false); // stretch voices restart at every wrap
    const uint64_t a = load_asset(e, {sine(96000, 220.0, kRate, 0.3), sine(96000, 330.0, kRate, 0.3)});
    const uint64_t m = load_asset(e, {sine(5000, 110.0, kRate, 0.3)});
    LoopSession s;
    const uint8_t modes[] = {SE_WARP_REPITCH, SE_WARP_STRETCH, SE_WARP_AUTO};
    for (uint64_t i = 0; i < 12; ++i) {
        const auto t0 = static_cast<int64_t>(i) * 500;
        s.add(i + 1, i % 2 ? m : a, t0, static_cast<int64_t>(i) * 10, 200000);
        if (i % 4 == 0) s.warp(i + 1, modes[(i / 4) % 3], {{t0, 0.0}, {t0 + 60000, 50000.0}});
        s.loop(i + 1, 100 + static_cast<int64_t>(i) * 7, 700 + static_cast<int64_t>(i) * 331); // short loops: many wraps
    }
    REQUIRE(s.publish(e) == SE_OK);
    std::vector<float> buf(2 * 8192);
    REQUIRE(se_transport_play(e) == SE_OK);
    REQUIRE(se_render_offline(e, buf.data(), 256) == SE_OK);
    const long allocs = rt_alloc_count();
    const long frees = rt_free_count();
    const uint32_t sizes[] = {256, 1, 333, 8192, 64, 255};
    for (int i = 0; i < 600; ++i) {
        if (i % 97 == 50) se_transport_seek(e, i * 170);
        RtAllocGuard g;
        REQUIRE(se_render_offline(e, buf.data(), sizes[i % 6]) == SE_OK);
    }
    CHECK_EQ(rt_alloc_count() - allocs, 0);
    CHECK_EQ(rt_free_count() - frees, 0);
}
