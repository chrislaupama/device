#include "processor.hpp"
#include "test_harness.hpp"
#include "test_util.hpp"

#include <bit>
#include <cmath>
#include <numbers>

using namespace testutil;

TEST_CASE(kernels_basic) {
    std::vector<float> a = {1, 2, 3, 4, 5, 6, 7}, b = {1, 1, 1, 1, 1, 1, 1};
    saine::kernel_add_scaled(a.data(), b.data(), 0.5f, 7);
    CHECK(a[0] == 1.5f && a[6] == 7.5f);
    saine::kernel_scale(a.data(), 2.0f, 7);
    CHECK(a[0] == 3.0f && a[6] == 15.0f);
    a[3] = -100.0f;
    CHECK(saine::kernel_peak(a.data(), 7) == 100.0f);
    CHECK(saine::kernel_peak(a.data(), 0) == 0.0f);
}

TEST_CASE(kernel_peak_simd_matches_scalar) {
    for (uint32_t n : {8u, 9u, 16u, 37u, 256u, 1023u}) {
        for (uint32_t at = 0; at < n; at += (n / 7) + 1) {
            std::vector<float> v(n);
            for (uint32_t i = 0; i < n; ++i) v[i] = 0.01f * static_cast<float>(i % 13) - 0.06f;
            v[at] = (at % 2) ? -0.875f : 0.75f;
            const float expect = (at % 2) ? 0.875f : 0.75f;
            CHECK(saine::kernel_peak(v.data(), n) == expect);
        }
    }
}

TEST_CASE(silence_without_session_and_position_advances) {
    EngineHandle e;
    REQUIRE(se_transport_play(e) == SE_OK);
    const auto out = render(e, 1000, 256);
    bool silent = true;
    for (float v : out) silent &= v == 0.0f;
    CHECK(silent);
    se_transport_info t{};
    REQUIRE(se_transport_get(e, &t) == SE_OK);
    CHECK_EQ(t.position, 1000);
    CHECK_EQ(t.playing, 1);
}

TEST_CASE(clip_playback_is_sample_exact) {
    EngineHandle e(48000, 64);
    const auto src = ramp(4000); // src[i] = (i+1) * 1e-5
    const uint64_t a = load_asset(e, {src});
    Session s;
    // Hard left: L gain exactly 1, R exactly 0 -> bit-transparent path.
    s.add_track(1, {clip(1, a, 1000, 250, 500)}, 1.0f, -1.0f);
    REQUIRE(s.publish(e) == SE_OK);
    REQUIRE(se_transport_play(e) == SE_OK);
    const auto out = render(e, 3000, 64);
    int mismatches = 0;
    for (int t = 0; t < 3000; ++t) {
        const float expect = (t >= 1000 && t < 1500) ? src[static_cast<std::size_t>(250 + t - 1000)] : 0.0f;
        if (out[2 * static_cast<std::size_t>(t)] != expect) ++mismatches;
        if (out[2 * static_cast<std::size_t>(t) + 1] != 0.0f) ++mismatches;
    }
    CHECK_EQ(mismatches, 0);
    // Exact edges.
    CHECK(out[2 * 999] == 0.0f);
    CHECK(out[2 * 1000] == src[250]);
    CHECK(out[2 * 1499] == src[749]);
    CHECK(out[2 * 1500] == 0.0f);
}

TEST_CASE(clip_clamped_to_asset_end_and_gain_applied) {
    EngineHandle e(48000, 128);
    const auto src = ramp(1000);
    const uint64_t a = load_asset(e, {src});
    Session s;
    // Asks for 5000 frames from offset 900 -> only 100 exist.
    s.add_track(1, {clip(1, a, 10, 900, 5000, 0.5f), clip(2, a, 2000, 5000, 100) /* fully past end */}, 1.0f,
                -1.0f);
    REQUIRE(s.publish(e) == SE_OK);
    REQUIRE(se_transport_play(e) == SE_OK);
    const auto out = render(e, 2500, 100);
    int bad = 0;
    for (int t = 0; t < 2500; ++t) {
        const float expect = (t >= 10 && t < 110) ? src[static_cast<std::size_t>(900 + t - 10)] * 0.5f : 0.0f;
        if (out[2 * static_cast<std::size_t>(t)] != expect) ++bad;
    }
    CHECK_EQ(bad, 0);
}

TEST_CASE(stereo_asset_and_overlapping_clips_sum) {
    EngineHandle e(48000, 256);
    const uint64_t st = load_asset(e, {constant(1000, 0.25f), constant(1000, -0.5f)});
    Session s;
    // Two overlapping clips on one track: [0,600) and [400,1000).
    s.add_track(1, {clip(1, st, 400, 0, 600), clip(2, st, 0, 0, 600)}, 1.0f, 0.0f);
    REQUIRE(s.publish(e) == SE_OK);
    REQUIRE(se_transport_play(e) == SE_OK);
    const auto out = render(e, 1000, 256);
    const float c = std::cos(static_cast<float>(std::numbers::pi / 4.0));
    float pl = 0, pr = 0;
    saine::constant_power_pan(0.0f, pl, pr);
    CHECK_NEAR(pl, c, 1e-7);
    CHECK_NEAR(out[2 * 100], 0.25f * pl, 1e-7);
    CHECK_NEAR(out[2 * 100 + 1], -0.5f * pr, 1e-7);
    CHECK_NEAR(out[2 * 500], 0.5f * pl, 1e-7);  // overlap: both clips
    CHECK_NEAR(out[2 * 500 + 1], -1.0f * pr, 1e-7);
    CHECK_NEAR(out[2 * 800], 0.25f * pl, 1e-7);
}

TEST_CASE(multitrack_summing_exact) {
    EngineHandle e(48000, 256);
    const uint64_t a = load_asset(e, {constant(2000, 0.25f)});
    const uint64_t b = load_asset(e, {constant(2000, 0.125f)});
    const uint64_t c = load_asset(e, {constant(2000, 0.0625f)});
    Session s;
    s.add_track(1, {clip(1, a, 0, 0, 2000)}, 1.0f, -1.0f);
    s.add_track(2, {clip(2, b, 500, 0, 1000)}, 1.0f, -1.0f);
    s.add_track(3, {clip(3, c, 0, 0, 2000)}, 2.0f, 1.0f); // right only, gain 2
    REQUIRE(s.publish(e) == SE_OK);
    REQUIRE(se_transport_play(e) == SE_OK);
    const auto out = render(e, 2000, 77);
    CHECK(out[2 * 100] == 0.25f);
    CHECK(out[2 * 700] == 0.375f);
    CHECK(out[2 * 1500] == 0.25f);
    CHECK(out[2 * 100 + 1] == 0.125f);

    // Master gain scales everything.
    s.master_gain = 0.5f;
    REQUIRE(s.publish(e) == SE_OK);
    REQUIRE(se_transport_seek(e, 700) == SE_OK);
    const auto out2 = render(e, 1, 1);
    CHECK(out2[0] == 0.1875f);
    CHECK(out2[1] == 0.0625f);
}

TEST_CASE(constant_power_pan_law) {
    EngineHandle e(48000, 64);
    const uint64_t a = load_asset(e, {constant(256, 1.0f)});
    const float pans[] = {-1.0f, -0.75f, -0.5f, -0.25f, 0.0f, 0.25f, 0.5f, 0.75f, 1.0f};
    for (float pan : pans) {
        Session s;
        s.add_track(1, {clip(1, a, 0, 0, 256)}, 1.0f, pan);
        REQUIRE(s.publish(e) == SE_OK);
        REQUIRE(se_transport_seek(e, 0) == SE_OK);
        REQUIRE(se_transport_play(e) == SE_OK);
        const auto out = render(e, 64, 64);
        const double l = out[2 * 10], r = out[2 * 10 + 1];
        CHECK_NEAR(l * l + r * r, 1.0, 1e-6); // constant power
        const double theta = (static_cast<double>(pan) + 1.0) * std::numbers::pi / 4.0;
        CHECK_NEAR(l, std::cos(theta), 1e-6);
        CHECK_NEAR(r, std::sin(theta), 1e-6);
    }
    // Centre is -3.01 dB per side; hard pans are exact.
    float l = 0, r = 0;
    saine::constant_power_pan(0.0f, l, r);
    CHECK_NEAR(20.0 * std::log10(static_cast<double>(l)), -3.0103, 1e-3);
    saine::constant_power_pan(-1.0f, l, r);
    CHECK(l == 1.0f && r == 0.0f);
    saine::constant_power_pan(1.0f, l, r);
    CHECK(l == 0.0f && r == 1.0f);
    saine::constant_power_pan(5.0f, l, r); // clamped
    CHECK(l == 0.0f && r == 1.0f);
}

TEST_CASE(mute_and_solo_semantics) {
    EngineHandle e(48000, 64);
    const uint64_t a = load_asset(e, {constant(128, 0.5f)});
    const uint64_t b = load_asset(e, {constant(128, 0.25f)});
    const uint64_t c = load_asset(e, {constant(128, 0.125f)});
    struct Case {
        bool mute[3];
        bool solo[3];
        float expect;
    };
    const Case cases[] = {
        {{false, false, false}, {false, false, false}, 0.875f}, // all audible
        {{false, true, false}, {false, false, false}, 0.625f},  // B muted
        {{false, false, false}, {true, false, false}, 0.5f},    // solo A
        {{false, false, false}, {true, false, true}, 0.625f},   // solo A + C
        {{false, true, false}, {false, true, false}, 0.0f},     // solo B, but B muted: mute wins
        {{true, false, false}, {true, false, true}, 0.125f},    // A soloed+muted, C soloed
        {{true, true, true}, {false, false, false}, 0.0f},      // all muted
    };
    for (const Case& k : cases) {
        Session s;
        s.add_track(1, {clip(1, a, 0, 0, 128)}, 1.0f, -1.0f, k.mute[0], k.solo[0]);
        s.add_track(2, {clip(2, b, 0, 0, 128)}, 1.0f, -1.0f, k.mute[1], k.solo[1]);
        s.add_track(3, {clip(3, c, 0, 0, 128)}, 1.0f, -1.0f, k.mute[2], k.solo[2]);
        REQUIRE(s.publish(e) == SE_OK);
        REQUIRE(se_transport_seek(e, 0) == SE_OK);
        REQUIRE(se_transport_play(e) == SE_OK);
        const auto out = render(e, 64, 64);
        CHECK(out[2 * 5] == k.expect);
    }
}

TEST_CASE(meters_report_and_reset) {
    EngineHandle e(48000, 64);
    const uint64_t a = load_asset(e, {constant(1000, 0.5f)});
    Session s;
    s.add_track(7, {clip(1, a, 0, 0, 1000)}, 1.0f, -1.0f);
    s.add_track(9, {clip(2, a, 0, 0, 1000)}, 0.5f, 1.0f);
    s.add_track(11, {}, 1.0f, 0.0f);
    REQUIRE(s.publish(e) == SE_OK);
    REQUIRE(se_transport_play(e) == SE_OK);
    render(e, 256, 64);

    se_meter m[8]{};
    std::size_t n = 0;
    REQUIRE(se_meters_read(e, m, 8, &n) == SE_OK);
    REQUIRE(n == 4);
    CHECK(m[0].track_id == 0); // master first
    CHECK(m[0].peak_l == 0.5f);
    CHECK(m[0].peak_r == 0.25f);
    CHECK(m[1].track_id == 7 && m[1].peak_l == 0.5f && m[1].peak_r == 0.0f);
    CHECK(m[2].track_id == 9 && m[2].peak_l == 0.0f && m[2].peak_r == 0.25f);
    CHECK(m[3].track_id == 11 && m[3].peak_l == 0.0f);

    // Peak hold is reset by reading.
    REQUIRE(se_meters_read(e, m, 8, &n) == SE_OK);
    CHECK(m[0].peak_l == 0.0f && m[1].peak_l == 0.0f);
    // Capacity is respected.
    REQUIRE(se_meters_read(e, m, 2, &n) == SE_OK);
    CHECK_EQ(n, 2u);
}

TEST_CASE(stats_count_callbacks) {
    EngineHandle e(48000, 256);
    se_stats_reset(e);
    render(e, 2560, 256);
    se_stats st{};
    REQUIRE(se_stats_get(e, &st) == SE_OK);
    CHECK_EQ(st.callbacks, 10u);
    CHECK(st.max_callback_us >= st.avg_callback_us);
    CHECK_NEAR(st.block_deadline_us, 256.0 / 48000.0 * 1e6, 0.01);
    se_stats_reset(e);
    REQUIRE(se_stats_get(e, &st) == SE_OK);
    CHECK_EQ(st.callbacks, 0u);
}

TEST_CASE(asset_peaks_for_waveforms) {
    EngineHandle e;
    std::vector<float> l(1000), r(1000);
    for (std::size_t i = 0; i < 1000; ++i) {
        l[i] = (i % 100 == 50) ? 0.8f : 0.1f;
        r[i] = (i % 100 == 20) ? -0.6f : 0.1f;
    }
    const uint64_t a = load_asset(e, {l, r});
    float mm[2 * 16];
    std::size_t n = 0;
    REQUIRE(se_asset_compute_peaks(e, a, 0, 100, mm, 16, &n) == SE_OK);
    CHECK_EQ(n, 10u);
    CHECK_NEAR(mm[0], 0.5f * (0.1f - 0.6f), 1e-7); // min (mono mix)
    CHECK_NEAR(mm[1], 0.5f * (0.8f + 0.1f), 1e-7); // max
    // Start offset + partial last bucket + capacity limit.
    REQUIRE(se_asset_compute_peaks(e, a, 950, 100, mm, 16, &n) == SE_OK);
    CHECK_EQ(n, 1u);
    REQUIRE(se_asset_compute_peaks(e, a, 0, 10, mm, 3, &n) == SE_OK);
    CHECK_EQ(n, 3u);
    REQUIRE(se_asset_compute_peaks(e, a, 5000, 10, mm, 3, &n) == SE_OK);
    CHECK_EQ(n, 0u);
    CHECK_EQ(se_asset_compute_peaks(e, a, 0, 0, mm, 3, &n), SE_ERR_INVALID_ARG);
    CHECK_EQ(se_asset_compute_peaks(e, 999, 0, 10, mm, 3, &n), SE_ERR_NOT_FOUND);
}

TEST_CASE(api_argument_validation) {
    CHECK_EQ(se_abi_version(), SE_ABI_VERSION);
    se_config bad{SE_ABI_VERSION + 1, 48000, 256, 2};
    CHECK(se_engine_create(&bad) == nullptr);
    CHECK(std::string(se_last_error()).find("ABI") != std::string::npos);
    se_config bad2{SE_ABI_VERSION, 48000, 0, 2};
    CHECK(se_engine_create(&bad2) == nullptr);
    se_config bad3{SE_ABI_VERSION, 48000, 256, 6};
    CHECK(se_engine_create(&bad3) == nullptr);
    CHECK(se_engine_create(nullptr) == nullptr);
    se_engine_destroy(nullptr); // no-op

    EngineHandle e(44100.0, 512);
    CHECK(se_engine_sample_rate(e) == 44100.0);
    CHECK_EQ(se_device_is_running(e), 0);
    CHECK_EQ(se_render_offline(e, nullptr, 16), SE_ERR_INVALID_ARG);
    CHECK_EQ(se_render_offline(nullptr, nullptr, 0), SE_ERR_INVALID_ARG);
    CHECK_EQ(se_session_publish(e, nullptr), SE_ERR_INVALID_ARG);
    CHECK_EQ(se_asset_release(e, 12345), SE_ERR_NOT_FOUND);
    se_asset_info info{};
    CHECK_EQ(se_asset_get_info(e, 12345, &info), SE_ERR_NOT_FOUND);

    Session s;
    s.add_track(1, {clip(1, 777, 0, 0, 10)});
    CHECK_EQ(s.publish(e), SE_ERR_NOT_FOUND); // unknown asset

    const uint64_t a = load_asset(e, {constant(10, 0.1f)}, 44100);
    Session neg;
    neg.add_track(1, {clip(1, a, 0, -5, 10)});
    CHECK_EQ(neg.publish(e), SE_ERR_INVALID_ARG);
    Session nan_gain;
    nan_gain.add_track(1, {clip(1, a, 0, 0, 10)}, std::nanf(""));
    CHECK_EQ(nan_gain.publish(e), SE_ERR_INVALID_ARG);
    se_track_desc t{};
    t.clip_count = 3; // clips == NULL
    se_session_desc d{&t, 1, 1.0f};
    CHECK_EQ(se_session_publish(e, &d), SE_ERR_INVALID_ARG);
    se_session_desc empty{nullptr, 0, 1.0f};
    CHECK_EQ(se_session_publish(e, &empty), SE_OK);
}
