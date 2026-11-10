// Audio clip warping (se_session_publish_v7) and onset detection.
#include "test_harness.hpp"
#include "test_hooks.hpp"
#include "test_util.hpp"
#include "wav.hpp"

#include <algorithm>
#include <chrono>
#include <thread>
#include <cmath>
#include <cstring>
#include <memory>
#include <string>
#include <vector>

using namespace testutil;

namespace {

constexpr double kRate = 48000.0;
constexpr float kCentre = 0.70710678f; // constant-power pan at centre (mono sources here)

// One track per clip; warps reference the clips by id.
struct WarpSession {
    std::vector<se_clip_desc> clips;
    std::vector<se_track_desc_v6> tracks;
    std::vector<std::vector<se_warp_point>> points;
    std::vector<std::vector<se_warp_slice>> slices;
    std::vector<se_clip_warp_desc> warps;

    void add(uint64_t clip_id, uint64_t asset, int64_t start, int64_t length, float gain = 1.0f) {
        clips.push_back(clip(clip_id, asset, start, 0, length, gain));
    }
    void warp(uint64_t clip_id, uint8_t mode, std::vector<se_warp_point> p, std::vector<se_warp_slice> s = {},
              float transpose = 0.0f) {
        points.push_back(std::move(p));
        slices.push_back(std::move(s));
        se_clip_warp_desc w{};
        w.clip_id = clip_id;
        w.mode = mode;
        w.transpose = transpose;
        warps.push_back(w);
    }
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
            warps[i].slices = slices[i].empty() ? nullptr : slices[i].data();
            warps[i].slice_count = static_cast<uint32_t>(slices[i].size());
        }
        se_session_desc_v7 d{};
        d.v6.tracks = tracks.data();
        d.v6.track_count = static_cast<uint32_t>(tracks.size());
        d.v6.master_gain = 1.0f;
        d.warps = warps.empty() ? nullptr : warps.data();
        d.warp_count = static_cast<uint32_t>(warps.size());
        return se_session_publish_v7(e, &d);
    }
};

// An engine whose warped clips always stretch live (no background renders).
struct LiveEngine : EngineHandle {
    LiveEngine() { dstudio::testing::set_warp_renders_enabled(e, false); }
};

// Waits for the background renders, then attaches them (se_plugin_idle).
bool attach_renders(se_engine* e) {
    for (int i = 0; i < 1000; ++i) {
        std::size_t n = 1;
        if (se_warp_renders_pending(e, &n) != SE_OK) return false;
        if (n == 0) return se_plugin_idle(e) == SE_OK;
        std::this_thread::sleep_for(std::chrono::milliseconds(10));
    }
    return false;
}

// A straight map: source = src0 + (t - t0) * slope.
std::vector<se_warp_point> line(int64_t t0, double src0, double slope, int64_t span) {
    return {{t0, src0}, {t0 + span, src0 + static_cast<double>(span) * slope}};
}

std::vector<float> left(const std::vector<float>& inter) {
    std::vector<float> l(inter.size() / 2);
    for (std::size_t i = 0; i < l.size(); ++i) l[i] = inter[2 * i];
    return l;
}

// Frequency from positive-going zero crossings over [a, b).
double frequency(const std::vector<float>& x, std::size_t a, std::size_t b) {
    std::size_t first = 0, last = 0, n = 0;
    for (std::size_t i = a + 1; i < b; ++i)
        if (x[i - 1] < 0.0f && x[i] >= 0.0f) {
            if (n == 0) first = i;
            last = i;
            ++n;
        }
    return n < 2 ? 0.0 : static_cast<double>(n - 1) * kRate / static_cast<double>(last - first);
}

double rms(const std::vector<float>& x, std::size_t a, std::size_t b) {
    double e = 0.0;
    for (std::size_t i = a; i < b; ++i) e += static_cast<double>(x[i]) * x[i];
    return std::sqrt(e / static_cast<double>(b - a));
}

// First frame at or after "from" whose magnitude exceeds "level".
std::size_t first_above(const std::vector<float>& x, std::size_t from, float level) {
    for (std::size_t i = from; i < x.size(); ++i)
        if (std::fabs(x[i]) > level) return i;
    return x.size();
}

// Short decaying clicks (1 kHz, 20 ms) at the given frames.
std::vector<float> clicks(std::size_t n, const std::vector<std::size_t>& at) {
    std::vector<float> v(n, 0.0f);
    for (std::size_t s : at)
        for (std::size_t i = 0; i < 960 && s + i < n; ++i)
            v[s + i] += static_cast<float>(0.8 * std::exp(-static_cast<double>(i) / 150.0) *
                                           std::sin(2.0 * 3.14159265358979 * 1000.0 * static_cast<double>(i) / kRate));
    return v;
}

} // namespace

TEST_CASE(warp_rejects_invalid_descriptions) {
    EngineHandle e;
    const uint64_t a = load_asset(e, {sine(48000, 440.0, kRate)});
    const auto try_publish = [&](auto&& edit) {
        WarpSession s;
        s.add(7, a, 0, 48000);
        s.warp(7, SE_WARP_STRETCH, line(0, 0.0, 0.9, 48000));
        edit(s);
        return s.publish(e);
    };
    CHECK(try_publish([](WarpSession&) {}) == SE_OK);
    CHECK(try_publish([](WarpSession& s) { s.warps[0].mode = 9; }) == SE_ERR_INVALID_ARG);
    CHECK(try_publish([](WarpSession& s) { s.warps[0].transpose = NAN; }) == SE_ERR_INVALID_ARG);
    CHECK(try_publish([](WarpSession& s) { s.points[0].pop_back(); }) == SE_ERR_INVALID_ARG);
    CHECK(try_publish([](WarpSession& s) { s.points[0][1].source = -1.0; }) == SE_ERR_INVALID_ARG);
    CHECK(try_publish([](WarpSession& s) { s.points[0][1].frame = 0; }) == SE_ERR_INVALID_ARG);
    CHECK(try_publish([](WarpSession& s) { s.warps[0].clip_id = 99; }) == SE_ERR_INVALID_ARG);
    CHECK(try_publish([](WarpSession& s) { s.warp(7, SE_WARP_REPITCH, line(0, 0.0, 1.0, 10)); }) ==
          SE_ERR_INVALID_ARG);
    CHECK(try_publish([](WarpSession& s) { s.slices[0] = {{100.0, 1, {}}, {50.0, 0, {}}}; }) == SE_ERR_INVALID_ARG);
    // Two clips with the warped id 7 and a warp for a missing clip 8: the counts match, still invalid.
    CHECK(try_publish([&](WarpSession& s) {
              s.add(7, a, 1000, 48000);
              s.warp(8, SE_WARP_STRETCH, line(0, 0.0, 0.9, 48000));
          }) == SE_ERR_INVALID_ARG);
}

TEST_CASE(warp_repitch_identity_matches_unwarped_playback) {
    EngineHandle e;
    const uint64_t a = load_asset(e, {sine(24000, 330.0, kRate), sine(24000, 550.0, kRate)});
    WarpSession plain;
    plain.add(1, a, 1000, 20000);
    REQUIRE(plain.publish(e) == SE_OK);
    REQUIRE(se_transport_play(e) == SE_OK);
    const std::vector<float> ref = render(e, 24000, 256);
    REQUIRE(se_transport_seek(e, 0) == SE_OK);
    WarpSession warped;
    warped.add(1, a, 1000, 20000);
    warped.warp(1, SE_WARP_REPITCH, line(1000, 0.0, 1.0, 20000));
    REQUIRE(warped.publish(e) == SE_OK);
    const std::vector<float> got = render(e, 24000, 256);
    float diff = 0.0f;
    for (std::size_t i = 0; i < ref.size(); ++i) diff = std::max(diff, std::fabs(ref[i] - got[i]));
    CHECK(diff < 1e-6f);
}

TEST_CASE(warp_repitch_moves_pitch_with_speed) {
    EngineHandle e;
    const uint64_t a = load_asset(e, {sine(96000, 440.0, kRate)});
    WarpSession s;
    s.add(1, a, 0, 48000);
    s.warp(1, SE_WARP_REPITCH, line(0, 0.0, 1.5, 48000)); // 1.5x faster
    REQUIRE(s.publish(e) == SE_OK);
    REQUIRE(se_transport_play(e) == SE_OK);
    const std::vector<float> l = left(render(e, 48000, 256));
    CHECK_NEAR(frequency(l, 4800, 43200), 660.0, 1.0);
}

TEST_CASE(warp_stretch_keeps_pitch_and_level) {
    for (const double slope : {0.5, 0.8, 1.25, 2.0}) {
        LiveEngine e;
        const uint64_t a = load_asset(e, {sine(4 * 48000, 440.0, kRate, 0.5)});
        WarpSession s;
        const auto span = static_cast<int64_t>(48000 / slope);
        s.add(1, a, 0, span);
        s.warp(1, SE_WARP_STRETCH, line(0, 0.0, slope, span));
        REQUIRE(s.publish(e) == SE_OK);
        REQUIRE(se_transport_play(e) == SE_OK);
        const std::vector<float> l = left(render(e, static_cast<uint32_t>(span), 256));
        const auto n = static_cast<std::size_t>(span);
        CHECK_NEAR(frequency(l, n / 4, 3 * n / 4), 440.0, 2.0);
        CHECK_NEAR(rms(l, n / 4, 3 * n / 4), 0.5 / std::sqrt(2.0) * kCentre, 0.04);
    }
}

TEST_CASE(warp_stretch_transposes) {
    for (const uint8_t mode : {uint8_t{SE_WARP_STRETCH}, uint8_t{SE_WARP_STRETCH_FORMANT}}) {
        LiveEngine e;
        const uint64_t a = load_asset(e, {sine(3 * 48000, 440.0, kRate, 0.5)});
        WarpSession s;
        s.add(1, a, 0, 48000);
        s.warp(1, mode, line(0, 0.0, 1.0, 48000), {}, 7.0f);
        REQUIRE(s.publish(e) == SE_OK);
        REQUIRE(se_transport_play(e) == SE_OK);
        const std::vector<float> l = left(render(e, 48000, 256));
        CHECK_NEAR(frequency(l, 12000, 36000), 440.0 * std::exp2(7.0 / 12.0), 4.0);
    }
}

TEST_CASE(warp_stretch_is_aligned_to_the_map) {
    // A tone burst at source frame 24000 must come out where the map puts it,
    // also when playback starts in the middle of the clip.
    LiveEngine e;
    std::vector<float> src(96000, 0.0f);
    const std::vector<float> tone = sine(24000, 440.0, kRate, 0.5);
    std::copy(tone.begin(), tone.end(), src.begin() + 24000);
    const uint64_t a = load_asset(e, {src});
    WarpSession s;
    s.add(1, a, 4800, 96000);
    s.warp(1, SE_WARP_STRETCH, line(4800, 0.0, 0.5, 96000)); // twice as long
    REQUIRE(s.publish(e) == SE_OK);
    REQUIRE(se_transport_play(e) == SE_OK);
    const std::vector<float> l = left(render(e, 110000, 256));
    const std::size_t expected = 4800 + 48000;
    const std::size_t got = first_above(l, 4800, 0.1f);
    CHECK(got + 960 > expected && got < expected + 960); // within 20 ms
    // Seek into the clip just before the burst: same place.
    REQUIRE(se_transport_seek(e, 4800 + 40000) == SE_OK);
    const std::vector<float> l2 = left(render(e, 20000, 333));
    const std::size_t got2 = 4800 + 40000 + first_above(l2, 0, 0.1f);
    CHECK(got2 + 960 > expected && got2 < expected + 960);
}

TEST_CASE(warp_slice_places_hits_on_the_map) {
    EngineHandle e;
    const std::vector<std::size_t> hits = {0, 12000, 24000, 36000};
    const uint64_t a = load_asset(e, {clicks(48000, hits)});
    WarpSession s;
    const double slope = 0.75; // slower: hits move apart, gaps open
    s.add(1, a, 1000, 64000);
    std::vector<se_warp_slice> sl;
    for (std::size_t h : hits) sl.push_back(se_warp_slice{static_cast<double>(h), 1, {}});
    s.warp(1, SE_WARP_SLICE, line(1000, 0.0, slope, 64000), sl);
    REQUIRE(s.publish(e) == SE_OK);
    REQUIRE(se_transport_play(e) == SE_OK);
    const std::vector<float> l = left(render(e, 70000, 256));
    const std::vector<float> ref = clicks(48000, {0});
    for (std::size_t h : hits) {
        const auto at = static_cast<std::size_t>(std::llround(1000.0 + static_cast<double>(h) / slope));
        // The hit is copied sample for sample (no smearing).
        float diff = 0.0f;
        for (std::size_t i = 0; i < 600; ++i) diff = std::max(diff, std::fabs(l[at + i] - ref[i] * kCentre));
        CHECK(diff < 1e-5f);
    }
    // Faster than the source: the next slice starts on time.
    WarpSession f;
    f.add(1, a, 0, 32000);
    f.warp(1, SE_WARP_SLICE, line(0, 0.0, 1.5, 32000), sl);
    REQUIRE(f.publish(e) == SE_OK);
    REQUIRE(se_transport_seek(e, 0) == SE_OK);
    const std::vector<float> l2 = left(render(e, 32000, 256));
    for (std::size_t h : hits) {
        const auto at = static_cast<std::size_t>(std::llround(static_cast<double>(h) / 1.5));
        CHECK(first_above(l2, at > 0 ? at - 1 : 0, 0.05f) <= at + 2);
    }
}

TEST_CASE(warp_auto_slices_percussive_passages_and_stretches_the_rest) {
    LiveEngine e;
    std::vector<float> src = clicks(96000, {0, 12000, 24000});
    const std::vector<float> tone = sine(48000, 440.0, kRate, 0.4);
    std::copy(tone.begin(), tone.end(), src.begin() + 48000);
    const uint64_t a = load_asset(e, {src});
    WarpSession s;
    s.add(1, a, 0, 120000);
    s.warp(1, SE_WARP_AUTO, line(0, 0.0, 0.8, 120000),
           {{0.0, 1, {}}, {12000.0, 1, {}}, {24000.0, 1, {}}, {48000.0, 0, {}}});
    REQUIRE(s.publish(e) == SE_OK);
    REQUIRE(se_transport_play(e) == SE_OK);
    const std::vector<float> l = left(render(e, 120000, 256));
    // Percussive passage: the second hit (at 12000 / 0.8 = 15000), copied sample for sample.
    const std::vector<float> ref = clicks(4800, {0});
    float diff = 0.0f;
    for (std::size_t i = 200; i < 600; ++i) diff = std::max(diff, std::fabs(l[15000 + i] - ref[i] * kCentre));
    CHECK(diff < 1e-5f);
    // Tonal passage: stretched, pitch kept.
    CHECK_NEAR(frequency(l, 70000, 110000), 440.0, 2.0);
}

TEST_CASE(warp_voice_survives_republish) {
    LiveEngine e;
    const uint64_t a = load_asset(e, {sine(4 * 48000, 440.0, kRate, 0.5), sine(4 * 48000, 660.0, kRate, 0.5)});
    WarpSession s;
    s.add(1, a, 0, 96000);
    s.warp(1, SE_WARP_STRETCH, line(0, 0.0, 0.9, 96000));
    REQUIRE(s.publish(e) == SE_OK);
    REQUIRE(se_transport_play(e) == SE_OK);
    const std::vector<float> ref = render(e, 48000, 256);
    REQUIRE(se_transport_seek(e, 0) == SE_OK);
    std::vector<float> got = render(e, 24064, 256);
    REQUIRE(s.publish(e) == SE_OK); // same session again mid-play
    const std::vector<float> rest = render(e, 48000 - 24064, 256);
    got.insert(got.end(), rest.begin(), rest.end());
    float diff = 0.0f;
    for (std::size_t i = 0; i < ref.size(); ++i) diff = std::max(diff, std::fabs(ref[i] - got[i]));
    CHECK(diff < 1e-6f);
}

TEST_CASE(warp_render_path_does_not_allocate) {
    LiveEngine e;
    const uint64_t a = load_asset(e, {sine(96000, 220.0, kRate, 0.3), sine(96000, 330.0, kRate, 0.3)});
    const uint64_t m = load_asset(e, {clicks(96000, {0, 9000, 18000, 30000, 47000})});
    WarpSession s;
    const uint8_t modes[] = {SE_WARP_REPITCH, SE_WARP_SLICE, SE_WARP_STRETCH, SE_WARP_STRETCH_FORMANT, SE_WARP_AUTO};
    for (uint64_t i = 0; i < 10; ++i) {
        const auto t0 = static_cast<int64_t>(i) * 700;
        s.add(i + 1, i % 2 ? m : a, t0, 80000);
        s.warp(i + 1, modes[i % 5], {{t0, 0.0}, {t0 + 40000, 30000.0}, {t0 + 80000, 96000.0}},
               {{0.0, 1, {}}, {9000.0, 0, {}}, {18000.0, 1, {}}, {30000.0, 0, {}}, {47000.0, 1, {}}},
               static_cast<float>(i % 3) - 1.0f);
    }
    REQUIRE(s.publish(e) == SE_OK);
    std::vector<float> buf(2 * 8192);
    REQUIRE(se_transport_play(e) == SE_OK);
    REQUIRE(se_render_offline(e, buf.data(), 256) == SE_OK);
    const long before = rt_alloc_count();
    const uint32_t sizes[] = {256, 1, 333, 8192, 64, 255};
    for (int i = 0; i < 500; ++i) {
        if (i % 97 == 50) se_transport_seek(e, i * 150); // restarts every voice
        RtAllocGuard g;
        REQUIRE(se_render_offline(e, buf.data(), sizes[i % 6]) == SE_OK);
    }
    CHECK_EQ(rt_alloc_count() - before, 0);
}

TEST_CASE(warp_export_renders_warped_clips) {
    LiveEngine e;
    const uint64_t a = load_asset(e, {sine(2 * 48000, 440.0, kRate, 0.5)});
    WarpSession s;
    s.add(1, a, 0, 60000);
    s.warp(1, SE_WARP_STRETCH, line(0, 0.0, 0.8, 60000));
    REQUIRE(s.publish(e) == SE_OK);
    REQUIRE(se_transport_play(e) == SE_OK);
    const std::vector<float> live = left(render(e, 60000, 256));
    TempFile f("warp_export");
    REQUIRE(se_export_wav(e, f.path.c_str(), 0, 60000) == SE_OK);
    dstudio::DecodedAudio d;
    std::string err;
    REQUIRE(dstudio::decode_wav_file(f.path.c_str(), d, err) == SE_OK);
    REQUIRE(d.frames == 60000);
    float diff = 0.0f;
    for (std::size_t i = 0; i < 60000; ++i) diff = std::max(diff, std::fabs(d.planar[0][i] - live[i]));
    CHECK(diff < 1e-6f); // fresh voices from the same start: the same audio
}

TEST_CASE(warp_renders_take_over_from_live_stretching) {
    const auto session = [](uint64_t asset, int64_t start) {
        WarpSession s;
        s.add(1, asset, start, 72000);
        s.warp(1, SE_WARP_STRETCH, line(start, 0.0, 0.85, 72000));
        return s;
    };
    const std::vector<float> src = sine(2 * 48000, 440.0, kRate, 0.5);
    std::vector<float> live;
    {
        LiveEngine e;
        WarpSession s = session(load_asset(e, {src}), 0);
        REQUIRE(s.publish(e) == SE_OK);
        REQUIRE(se_transport_play(e) == SE_OK);
        live = left(render(e, 72000, 256));
    }
    EngineHandle e;
    const uint64_t a = load_asset(e, {src});
    WarpSession s = session(a, 0);
    REQUIRE(s.publish(e) == SE_OK);
    REQUIRE(attach_renders(e));
    REQUIRE(se_transport_play(e) == SE_OK);
    const std::vector<float> got = left(render(e, 72000, 256));
    // The same stretch, rendered in bigger blocks ahead of time.
    float diff = 0.0f;
    for (std::size_t i = 0; i < live.size(); ++i) diff = std::max(diff, std::fabs(live[i] - got[i]));
    CHECK(diff < 0.02f);
    CHECK_NEAR(frequency(got, 18000, 54000), 440.0, 2.0);
    // Moving the clip keeps its render: nothing to do.
    WarpSession moved = session(a, 24000);
    REQUIRE(moved.publish(e) == SE_OK);
    std::size_t pending = 1;
    REQUIRE(se_warp_renders_pending(e, &pending) == SE_OK);
    CHECK_EQ(pending, std::size_t{0});
    // A new speed needs a new render.
    WarpSession faster;
    faster.add(1, a, 0, 60000);
    faster.warp(1, SE_WARP_STRETCH, line(0, 0.0, 1.1, 60000));
    REQUIRE(faster.publish(e) == SE_OK);
    REQUIRE(se_warp_renders_pending(e, &pending) == SE_OK);
    CHECK(pending >= 1);
    REQUIRE(attach_renders(e));
}

TEST_CASE(warp_render_swap_does_not_allocate) {
    EngineHandle e;
    dstudio::testing::set_warp_renders_enabled(e, false);
    const uint64_t a = load_asset(e, {sine(96000, 220.0, kRate, 0.3), sine(96000, 330.0, kRate, 0.3)});
    WarpSession s;
    for (uint64_t i = 0; i < 4; ++i) {
        s.add(i + 1, a, 0, 90000);
        s.warp(i + 1, static_cast<uint8_t>(i % 2 ? SE_WARP_AUTO : SE_WARP_STRETCH), line(0, 0.0, 0.9 + 0.05 * static_cast<double>(i), 90000),
               {{0.0, 1, {}}, {20000.0, 0, {}}});
    }
    REQUIRE(s.publish(e) == SE_OK);
    std::vector<float> buf(2 * 8192);
    REQUIRE(se_transport_play(e) == SE_OK);
    REQUIRE(se_render_offline(e, buf.data(), 256) == SE_OK);
    dstudio::testing::set_warp_renders_enabled(e, true);
    REQUIRE(s.publish(e) == SE_OK); // queues the renders; the voices keep playing
    const long before = rt_alloc_count();
    for (int i = 0; i < 40; ++i) {
        RtAllocGuard g;
        REQUIRE(se_render_offline(e, buf.data(), 256) == SE_OK);
    }
    REQUIRE(attach_renders(e)); // swaps them in mid-play: one crossfaded block each
    for (int i = 0; i < 40; ++i) {
        RtAllocGuard g;
        REQUIRE(se_render_offline(e, buf.data(), 256) == SE_OK);
    }
    CHECK_EQ(rt_alloc_count() - before, 0);
}

TEST_CASE(onsets_find_hits_and_classify_them) {
    EngineHandle e;
    const std::vector<std::size_t> hits = {4800, 20000, 31000, 52000, 70000};
    const uint64_t a = load_asset(e, {clicks(96000, hits)});
    std::vector<se_onset> on(64);
    std::size_t n = 0;
    REQUIRE(se_asset_detect_onsets(e, a, on.data(), on.size(), &n) == SE_OK);
    REQUIRE(n == hits.size());
    for (std::size_t i = 0; i < n; ++i) {
        CHECK(std::llabs(on[i].frame - static_cast<int64_t>(hits[i])) <= 144); // 3 ms
        CHECK(on[i].percussive == 1);
        CHECK(on[i].strength > 0.3f && on[i].strength <= 1.0f);
    }
    // A held tone (faded out): one onset at its start, not percussive.
    std::vector<float> held(96000, 0.0f);
    std::vector<float> tone = sine(72000, 330.0, kRate, 0.5);
    for (std::size_t i = 0; i < 4800; ++i) tone[tone.size() - 1 - i] *= static_cast<float>(i) / 4800.0f;
    std::copy(tone.begin(), tone.end(), held.begin() + 12000);
    const uint64_t b = load_asset(e, {held});
    REQUIRE(se_asset_detect_onsets(e, b, on.data(), on.size(), &n) == SE_OK);
    REQUIRE(n == 1);
    CHECK(std::llabs(on[0].frame - 12000) <= 144);
    CHECK(on[0].percussive == 0);
    CHECK(se_asset_detect_onsets(e, 12345, on.data(), on.size(), &n) == SE_ERR_NOT_FOUND);
}

TEST_CASE(asset_reverse_plays_back_to_front) {
    EngineHandle e;
    const std::vector<float> l = ramp(1000, 1e-3f), r = constant(1000, 0.25f);
    const uint64_t a = load_asset(e, {l, r});
    se_asset_info info{};
    REQUIRE(se_asset_reverse(e, a, &info) == SE_OK);
    CHECK(info.asset_id != a);
    CHECK(info.frames == 1000);
    CHECK(info.channels == 2);
    CHECK(se_asset_reverse(e, 9999, nullptr) == SE_ERR_NOT_FOUND);
    // Play the copy: the left channel is the ramp backwards.
    WarpSession s;
    s.add(1, info.asset_id, 0, 1000);
    REQUIRE(s.publish(e) == SE_OK);
    REQUIRE(se_transport_play(e) == SE_OK);
    const std::vector<float> out = render(e, 1000, 250);
    for (std::size_t f : {0u, 1u, 500u, 999u}) CHECK_NEAR(out[2 * f] / kCentre, l[999 - f], 1e-6);
    CHECK_NEAR(out[1] / kCentre, 0.25, 1e-6);
    // The original is untouched, and the copy is released on its own.
    CHECK(se_asset_release(e, info.asset_id) == SE_OK);
    se_asset_info orig{};
    CHECK(se_asset_get_info(e, a, &orig) == SE_OK);
}
