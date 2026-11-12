// Channel strip trim, width and mono (se_session_publish_v10, spec 004):
// levels, mid/side width, the mono fold, identity, validation and RT safety.
//
// Tracks are centred (constant-power: both sides scaled by cos(pi/4)); the
// clip is 0.5 on the left and 0.1 on the right, so mid 0.3, side 0.2.
#include "test_harness.hpp"
#include "test_util.hpp"

#include <cmath>
#include <tuple>
#include <utility>
#include <vector>

using namespace testutil;

namespace {

constexpr float kCentre = 0.70710678f;

struct SSession {
    se_track_desc_v6 track{};
    std::vector<se_clip_desc> clips;
    std::vector<se_track_strip> strips;

    explicit SSession(se_engine* e) {
        const uint64_t a = load_asset(e, {constant(4096, 0.5f), constant(4096, 0.1f)});
        clips = {clip(1, a, 0, 0, 4096)};
        track.track_id = 1;
        track.gain = 1.0f;
    }
    void strip(float trim, float width, bool mono, uint64_t id = 1) {
        strips.push_back(se_track_strip{id, trim, width, static_cast<uint8_t>(mono ? 1 : 0), {}});
    }
    int32_t publish(se_engine* e) {
        se_track_desc_v6 d = track;
        d.clips = clips.data();
        d.clip_count = static_cast<uint32_t>(clips.size());
        se_session_desc_v10 s{};
        s.v9.v8.v7.v6.tracks = &d;
        s.v9.v8.v7.v6.track_count = 1;
        s.v9.v8.v7.v6.master_gain = 1.0f;
        s.strips = strips.data();
        s.strip_count = static_cast<uint32_t>(strips.size());
        return se_session_publish_v10(e, &s);
    }
};

// Left and right at frame 100.
std::pair<float, float> level(se_engine* e) {
    se_transport_seek(e, 0);
    se_transport_play(e);
    const std::vector<float> out = render(e, 512, 256);
    return {out[200], out[201]};
}

} // namespace

TEST_CASE(strip_trim_scales_the_level_by_its_gain) {
    EngineHandle e;
    SSession s(e);
    s.strip(std::pow(10.0f, 6.0f / 20.0f), 1.0f, false); // +6 dB
    REQUIRE(s.publish(e) == SE_OK);
    const auto [l, r] = level(e);
    CHECK_NEAR(l, 0.5f * kCentre * 1.99526f, 1e-5f);
    CHECK_NEAR(r, 0.1f * kCentre * 1.99526f, 1e-5f);
}

TEST_CASE(strip_width_scales_the_side_signal_and_zero_is_mono) {
    EngineHandle e;
    SSession s(e);
    s.strip(1.0f, 0.0f, false);
    REQUIRE(s.publish(e) == SE_OK);
    auto [l, r] = level(e);
    CHECK_NEAR(l, 0.3f * kCentre, 1e-6f);
    CHECK_EQ(l, r); // identical channels
    s.strips.clear();
    s.strip(1.0f, 2.0f, false); // side doubled: 0.3 +/- 0.4
    REQUIRE(s.publish(e) == SE_OK);
    std::tie(l, r) = level(e);
    CHECK_NEAR(l, 0.7f * kCentre, 1e-6f);
    CHECK_NEAR(r, -0.1f * kCentre, 1e-6f);
}

TEST_CASE(strip_mono_folds_to_the_average_whatever_the_width) {
    EngineHandle e;
    SSession s(e);
    s.strip(1.0f, 2.0f, true);
    REQUIRE(s.publish(e) == SE_OK);
    const auto [l, r] = level(e);
    CHECK_NEAR(l, 0.3f * kCentre, 1e-6f);
    CHECK_EQ(l, r);
}

TEST_CASE(strip_at_unity_is_bit_identical_to_no_strip) {
    EngineHandle e;
    SSession s(e);
    REQUIRE(s.publish(e) == SE_OK);
    const auto plain = level(e);
    s.strip(1.0f, 1.0f, false);
    REQUIRE(s.publish(e) == SE_OK);
    const auto unity = level(e);
    CHECK_EQ(plain.first, unity.first);
    CHECK_EQ(plain.second, unity.second);
}

TEST_CASE(strip_rejects_bad_settings) {
    EngineHandle e;
    const auto bad = [&](float trim, float width, uint64_t id, bool twice) {
        SSession s(e);
        s.strip(trim, width, false, id);
        if (twice) s.strip(1.0f, 1.0f, false, id);
        return s.publish(e);
    };
    CHECK_EQ(bad(17.0f, 1.0f, 1, false), SE_ERR_INVALID_ARG);
    CHECK_EQ(bad(-0.1f, 1.0f, 1, false), SE_ERR_INVALID_ARG);
    CHECK_EQ(bad(NAN, 1.0f, 1, false), SE_ERR_INVALID_ARG);
    CHECK_EQ(bad(1.0f, 2.5f, 1, false), SE_ERR_INVALID_ARG);
    CHECK_EQ(bad(1.0f, -0.5f, 1, false), SE_ERR_INVALID_ARG);
    CHECK_EQ(bad(1.0f, 1.0f, 7, false), SE_ERR_INVALID_ARG); // unknown track
    CHECK_EQ(bad(1.0f, 1.0f, 1, true), SE_ERR_INVALID_ARG);  // listed twice
    se_session_desc_v10 d{};
    d.strip_count = 1; // strips == NULL
    CHECK_EQ(se_session_publish_v10(e, &d), SE_ERR_INVALID_ARG);
}

TEST_CASE(strip_render_does_not_allocate) {
    EngineHandle e;
    SSession s(e);
    s.strip(0.5f, 1.5f, false);
    REQUIRE(s.publish(e) == SE_OK);
    se_transport_seek(e, 0);
    se_transport_play(e);
    std::vector<float> buf(2 * 256);
    REQUIRE(se_render_offline(e, buf.data(), 256) == SE_OK); // warm-up
    const long before = rt_alloc_count();
    for (int i = 0; i < 8; ++i) {
        RtAllocGuard g;
        REQUIRE(se_render_offline(e, buf.data(), 256) == SE_OK);
    }
    CHECK_EQ(rt_alloc_count() - before, 0L);
}
