// Stress tests: snapshot publication, transport commands and asset churn on
// non-RT threads while another thread renders through the live (RT) path.
// Run under ASan/UBSan (asan preset) to catch use-after-free in reclamation.
#include "asset.hpp"
#include "test_harness.hpp"
#include "test_util.hpp"

#include <atomic>
#include <chrono>
#include <cmath>
#include <thread>

using namespace testutil;

TEST_CASE(snapshot_republish_while_rendering_stress) {
    EngineHandle e(48000, 128);
    const int64_t base = dstudio::Asset::live_count();
    std::vector<uint64_t> stable;
    for (int i = 0; i < 4; ++i)
        stable.push_back(load_asset(e, {constant(5000, 0.1f * static_cast<float>(i + 1))}));
    // Pre-write a file so the churn thread only does decode + register.
    TempFile churn_file("churn.wav");
    write_float_wav(churn_file.path, {constant(3000, 0.05f), constant(3000, -0.05f)}, 44100);

    REQUIRE(se_transport_play(e) == SE_OK);

    std::atomic<bool> stop{false};
    std::atomic<long> renders{0}, publishes{0}, churns{0};
    std::atomic<bool> bad_sample{false};

    std::thread renderer([&] {
        std::vector<float> buf(2 * 1024);
        uint32_t sizes[] = {1, 64, 127, 128, 300, 1024};
        std::size_t k = 0;
        while (!stop.load()) {
            const uint32_t n = sizes[k++ % 6];
            if (se_render_offline(e, buf.data(), n) != SE_OK) {
                bad_sample = true;
                break;
            }
            for (uint32_t i = 0; i < 2 * n; ++i)
                if (!std::isfinite(buf[i]) || std::fabs(buf[i]) > 100.0f) bad_sample = true;
            renders.fetch_add(1);
        }
    });

    std::thread publisher([&] {
        uint64_t iter = 0;
        while (!stop.load()) {
            Session s;
            const int tracks = 1 + static_cast<int>(iter % 16);
            for (int t = 0; t < tracks; ++t) {
                std::vector<se_clip_desc> clips;
                for (int c = 0; c < 3; ++c)
                    clips.push_back(clip(static_cast<uint64_t>(c), stable[static_cast<std::size_t>((t + c) % 4)],
                                         static_cast<int64_t>((iter * 37 + static_cast<uint64_t>(c) * 1000) % 20000),
                                         c * 10, 4000, 0.5f));
                s.add_track(static_cast<uint64_t>(t + 1), std::move(clips), 0.5f,
                            static_cast<float>(t % 3) - 1.0f, iter % 7 == 0, t == 0 && iter % 5 == 0);
            }
            const se_session_desc d = s.desc();
            if (se_session_publish(e, &d) != SE_OK) bad_sample = true;
            publishes.fetch_add(1);
            ++iter;
        }
    });

    std::thread transport([&] {
        int64_t i = 0;
        while (!stop.load()) {
            se_transport_seek(e, (i * 977) % 25000);
            if (i % 50 == 0) se_transport_pause(e);
            if (i % 50 == 1) se_transport_play(e);
            ++i;
            std::this_thread::yield();
        }
        se_transport_play(e);
    });

    // Asset churn: load, reference from a snapshot, release while referenced.
    std::thread churner([&] {
        while (!stop.load()) {
            se_asset_info info{};
            if (se_asset_load_file(e, churn_file.path.c_str(), &info) != SE_OK) {
                bad_sample = true;
                break;
            }
            Session s;
            s.add_track(99, {clip(1, info.asset_id, 0, 0, 3000), clip(2, info.asset_id, 1500, 100, 2000)});
            const se_session_desc d = s.desc();
            se_session_publish(e, &d);
            se_asset_release(e, info.asset_id); // graph may still be rendering it
            se_meter m[32];
            std::size_t n = 0;
            se_meters_read(e, m, 32, &n);
            churns.fetch_add(1);
        }
    });

    std::this_thread::sleep_for(std::chrono::milliseconds(1500));
    stop = true;
    renderer.join();
    publisher.join();
    transport.join();
    churner.join();

    std::fprintf(stderr, "  renders=%ld publishes=%ld churns=%ld\n", renders.load(), publishes.load(),
                 churns.load());
    CHECK(!bad_sample.load());
    CHECK(renders.load() > 100);
    CHECK(publishes.load() > 100);
    CHECK(churns.load() > 5);

    // Quiesce: one more publish with the RT side idle reclaims everything retired.
    Session empty;
    REQUIRE(empty.publish(e) == SE_OK);
    for (uint64_t id : stable) REQUIRE(se_asset_release(e, id) == SE_OK);
    CHECK_EQ(dstudio::Asset::live_count(), base); // no leaked or prematurely-kept assets
}

TEST_CASE(offline_render_is_exclusive) {
    // Two threads racing se_render_offline: one gets SE_ERR_STATE or they
    // serialise; neither may corrupt the other. Exercise the exclusivity CAS.
    EngineHandle e(48000, 64);
    std::atomic<int> ok{0}, busy{0}, other{0};
    auto worker = [&] {
        std::vector<float> buf(2 * 4096);
        for (int i = 0; i < 2000; ++i) {
            const int32_t rc = se_render_offline(e, buf.data(), 4096);
            if (rc == SE_OK)
                ok++;
            else if (rc == SE_ERR_STATE)
                busy++;
            else
                other++;
        }
    };
    std::thread a(worker), b(worker);
    a.join();
    b.join();
    CHECK(ok.load() > 0);
    CHECK_EQ(other.load(), 0);
    CHECK_EQ(ok.load() + busy.load(), 4000);
}
