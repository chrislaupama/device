#include "asset.hpp"
#include "test_harness.hpp"
#include "test_util.hpp"

using namespace testutil;

TEST_CASE(asset_release_while_referenced_keeps_audio_alive) {
    EngineHandle e(48000, 128);
    const int64_t base = dstudio::Asset::live_count();
    const auto src = ramp(4000);
    const uint64_t a = load_asset(e, {src});
    CHECK_EQ(dstudio::Asset::live_count(), base + 1);

    Session s;
    s.add_track(1, {clip(1, a, 0, 0, 4000)}, 1.0f, -1.0f);
    REQUIRE(s.publish(e) == SE_OK);
    REQUIRE(se_transport_play(e) == SE_OK);
    render(e, 128, 128);

    // Release while the published graph still references it.
    REQUIRE(se_asset_release(e, a) == SE_OK);
    se_asset_info info{};
    CHECK_EQ(se_asset_get_info(e, a, &info), SE_ERR_NOT_FOUND);
    CHECK_EQ(se_asset_release(e, a), SE_ERR_NOT_FOUND); // double release
    CHECK_EQ(dstudio::Asset::live_count(), base + 1);     // still alive: graph holds it

    // Audio keeps playing correctly from the released asset.
    const auto out = render(e, 256, 100);
    CHECK(out[0] == src[128]);
    CHECK(out[2 * 255] == src[383]);

    // New snapshots can no longer reference the released id.
    CHECK_EQ(s.publish(e), SE_ERR_NOT_FOUND);

    // Publish a session without it: the old graph is retired. The RT side is
    // idle (no render in flight), so reclamation frees it immediately.
    Session empty;
    REQUIRE(empty.publish(e) == SE_OK);
    CHECK_EQ(dstudio::Asset::live_count(), base);
}

TEST_CASE(retired_graphs_reclaimed_off_rt_thread) {
    EngineHandle e(48000, 64);
    const uint64_t a = load_asset(e, {constant(1000, 0.5f)});
    for (int i = 0; i < 50; ++i) {
        Session s;
        s.add_track(1, {clip(1, a, i, 0, 500)});
        REQUIRE(s.publish(e) == SE_OK);
        render(e, 64, 64);
    }
    // With the RT path idle every retired graph is reclaimed at publish time.
    const int64_t before = dstudio::Asset::live_count();
    REQUIRE(se_asset_release(e, a) == SE_OK);
    CHECK_EQ(dstudio::Asset::live_count(), before); // current graph still holds it
    Session empty;
    REQUIRE(empty.publish(e) == SE_OK);
    CHECK_EQ(dstudio::Asset::live_count(), before - 1);
}

TEST_CASE(engine_destroy_frees_everything) {
    const int64_t base = dstudio::Asset::live_count();
    {
        EngineHandle e;
        const uint64_t a = load_asset(e, {constant(100, 0.1f)});
        const uint64_t b = load_asset(e, {constant(100, 0.1f), constant(100, 0.2f)});
        Session s;
        s.add_track(1, {clip(1, a, 0, 0, 100), clip(2, b, 0, 0, 100)});
        REQUIRE(s.publish(e) == SE_OK);
        REQUIRE(s.publish(e) == SE_OK);
        CHECK_EQ(dstudio::Asset::live_count(), base + 2);
    }
    CHECK_EQ(dstudio::Asset::live_count(), base);
}
