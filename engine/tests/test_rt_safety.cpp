// Real-time safety: the process path must not touch the heap.
//
// test_main.cpp replaces global operator new/delete and counts calls made
// while the current thread is inside an RtAllocGuard. We put the guard
// around se_render_offline, which runs exactly the code the device callback
// runs (command drain, graph acquire, render_block, meters, stats).
#include "test_harness.hpp"
#include "test_util.hpp"

#include <memory>

using namespace testutil;

void* volatile g_alloc_sink = nullptr;

TEST_CASE(rt_alloc_counter_is_live) {
    // Prove the instrumentation actually catches allocations.
    const long before = rt_alloc_count();
    {
        RtAllocGuard g;
        // Direct operator new call (not a new-expression), so the compiler
        // may not elide it; the volatile store keeps the pointer escaping.
        void* p = ::operator new(64);
        g_alloc_sink = p;
        ::operator delete(p);
    }
    CHECK_EQ(rt_alloc_count() - before, 1);
    CHECK(rt_free_count() > 0);
}

TEST_CASE(rt_render_path_does_not_allocate) {
    EngineHandle e(48000, 256);
    std::vector<uint64_t> assets;
    for (int i = 0; i < 4; ++i)
        assets.push_back(load_asset(e, {sine(48000, 220.0 * (i + 1), 48000.0, 0.2), sine(48000, 330.0, 48000.0)}));
    assets.push_back(load_asset(e, {sine(30000, 100.0, 48000.0)}));
    Session s;
    for (int t = 0; t < 32; ++t) {
        std::vector<se_clip_desc> clips;
        for (int c = 0; c < 4; ++c)
            clips.push_back(clip(static_cast<uint64_t>(c), assets[static_cast<std::size_t>((t + c) % 5)],
                                 c * 9000 + t, c * 7, 12000, 0.8f));
        s.add_track(static_cast<uint64_t>(t + 1), std::move(clips), 0.7f, static_cast<float>(t % 5) * 0.5f - 1.0f,
                    t % 9 == 0, false);
    }
    REQUIRE(s.publish(e) == SE_OK);

    std::vector<float> buf(2 * 8192);
    // Warm up (first-touch of any lazily-initialised state outside the guard).
    REQUIRE(se_transport_play(e) == SE_OK);
    REQUIRE(se_render_offline(e, buf.data(), 256) == SE_OK);

    const long allocs_before = rt_alloc_count();
    const long frees_before = rt_free_count();
    const uint32_t sizes[] = {256, 1, 333, 8192 /* > max_block: chunked */, 64, 255};
    for (int i = 0; i < 600; ++i) {
        // Transport commands are queued outside the guard (producer side may lock).
        if (i % 100 == 50) se_transport_seek(e, i * 100);
        if (i % 100 == 70) se_transport_pause(e);
        if (i % 100 == 75) se_transport_play(e);
        if (i % 200 == 199) se_transport_stop(e), se_transport_play(e);
        RtAllocGuard g;
        if (se_render_offline(e, buf.data(), sizes[i % 6]) != SE_OK) {
            CHECK(false);
            break;
        }
    }
    CHECK_EQ(rt_alloc_count() - allocs_before, 0);
    CHECK_EQ(rt_free_count() - frees_before, 0);

    // Also with a snapshot swap between blocks (publish outside the guard).
    Session s2;
    s2.add_track(1, {clip(1, assets[0], 0, 0, 10000)});
    REQUIRE(s2.publish(e) == SE_OK);
    {
        RtAllocGuard g;
        for (int i = 0; i < 50; ++i) se_render_offline(e, buf.data(), 256);
    }
    CHECK_EQ(rt_alloc_count() - allocs_before, 0);
    CHECK_EQ(rt_free_count() - frees_before, 0);
}
