#include "test_harness.hpp"
#include "test_util.hpp"
#include "wav.hpp"

#include <cmath>

using namespace testutil;

namespace {
se_preview_info pv(se_engine* e) {
    se_preview_info i{};
    se_preview_get(e, &i);
    return i;
}
bool near(float a, float b) { return std::fabs(a - b) < 1e-4f; }
} // namespace

TEST_CASE(preview_plays_from_start_with_transport_stopped) {
    EngineHandle e(48000, 64);
    const auto src = ramp(2000);
    const uint64_t a = load_asset(e, {src});
    REQUIRE(se_preview_play(e, a, 0, 1.0f, 1.0f, 0) == SE_OK);
    auto out = render(e, 300, 64);
    // A 96-frame fade-in, then the asset as recorded, in both channels.
    CHECK(std::fabs(out[0]) <= std::fabs(src[0]) + 1e-6f);
    CHECK(near(out[2 * 200], src[200]) && near(out[2 * 200 + 1], src[200]));
    const auto i = pv(e);
    CHECK_EQ(i.playing, 1);
    CHECK_EQ(i.asset_id, a);
    CHECK_EQ(i.position, 300);
    CHECK_EQ(i.frames, 2000);
    CHECK(i.peak_l > 0.0f);
    CHECK(pv(e).peak_l == 0.0f); // peaks reset on read
    se_transport_info t{};
    se_transport_get(e, &t);
    CHECK_EQ(t.playing, 0);
    CHECK_EQ(t.position, 0);
}

TEST_CASE(preview_rate_gain_and_end) {
    EngineHandle e(48000, 64);
    const auto src = ramp(1000);
    const uint64_t a = load_asset(e, {src});
    REQUIRE(se_preview_play(e, a, 0, 0.5f, 2.0f, 0) == SE_OK);
    auto out = render(e, 200, 64);
    CHECK(near(out[2 * 150], 0.5f * src[300]));
    // Runs out after 1000 / 2 frames and stops by itself.
    render(e, 400, 64);
    const auto i = pv(e);
    CHECK_EQ(i.playing, 0);
    CHECK_EQ(i.asset_id, 0u);
    out = render(e, 10, 10);
    CHECK(out[0] == 0.0f);
}

TEST_CASE(preview_solo_silences_the_arrangement) {
    EngineHandle e(48000, 64);
    const auto song = ramp(20000);
    std::vector<float> tone(4000, 0.25f);
    const uint64_t a = load_asset(e, {song});
    const uint64_t b = load_asset(e, {tone});
    Session s;
    s.add_track(1, {clip(1, a, 0, 0, 20000)}, 1.0f, -1.0f);
    REQUIRE(s.publish(e) == SE_OK);
    REQUIRE(se_transport_play(e) == SE_OK);
    REQUIRE(se_preview_play(e, b, 0, 1.0f, 1.0f, 1) == SE_OK);
    auto out = render(e, 300, 64);
    CHECK(near(out[2 * 200], 0.25f));
    // Unsolo: the arrangement is heard under the preview again.
    REQUIRE(se_preview_set(e, 1.0f, 1.0f, 0) == SE_OK);
    out = render(e, 10, 10);
    CHECK(near(out[0], 0.25f + song[300]));
    // Stop fades out; afterwards only the arrangement.
    REQUIRE(se_preview_stop(e) == SE_OK);
    render(e, 200, 64);
    out = render(e, 10, 10);
    CHECK(near(out[0], song[510]));
    CHECK_EQ(pv(e).playing, 0);
}

TEST_CASE(preview_replace_crossfades_and_survives_asset_release) {
    EngineHandle e(48000, 64);
    std::vector<float> one(5000, 0.5f), two(5000, -0.25f);
    const uint64_t a = load_asset(e, {one});
    const uint64_t b = load_asset(e, {two});
    REQUIRE(se_preview_play(e, a, 0, 1.0f, 1.0f, 0) == SE_OK);
    render(e, 200, 64);
    REQUIRE(se_preview_play(e, b, 0, 1.0f, 1.0f, 0) == SE_OK);
    // The project drops both assets; the preview keeps what it plays alive.
    REQUIRE(se_asset_release(e, a) == SE_OK);
    REQUIRE(se_asset_release(e, b) == SE_OK);
    auto out = render(e, 64, 64);
    CHECK(near(out[2 * 48], 0.125f)); // mid crossfade: 0.5 * 0.5 + 0.5 * -0.25
    out = render(e, 300, 64);
    CHECK(near(out[2 * 299], -0.25f));
    CHECK_EQ(pv(e).asset_id, b);
}

TEST_CASE(preview_resumes_from_start_frame) {
    EngineHandle e(48000, 64);
    const auto src = ramp(2000);
    const uint64_t a = load_asset(e, {src});
    REQUIRE(se_preview_play(e, a, 700, 1.0f, 1.0f, 0) == SE_OK);
    auto out = render(e, 200, 64);
    CHECK(near(out[2 * 150], src[850]));
    CHECK_EQ(pv(e).position, 900);
    // Past the end: clamped, nothing sounds.
    REQUIRE(se_preview_play(e, a, 99999, 1.0f, 1.0f, 0) == SE_OK);
    render(e, 200, 64);
    CHECK_EQ(pv(e).playing, 0);
}

TEST_CASE(export_ignores_the_preview) {
    EngineHandle e(48000, 64);
    const auto src = ramp(4000);
    std::vector<float> tone(48000, 0.25f);
    const uint64_t a = load_asset(e, {src});
    const uint64_t b = load_asset(e, {tone});
    Session s;
    s.add_track(1, {clip(1, a, 0, 0, 4000)}, 1.0f, -1.0f);
    REQUIRE(s.publish(e) == SE_OK);
    REQUIRE(se_preview_play(e, b, 0, 1.0f, 1.0f, 1) == SE_OK);
    render(e, 200, 64);
    REQUIRE(pv(e).playing == 1);
    TempFile f("export_preview.wav");
    REQUIRE(se_export_wav(e, f.path.c_str(), 0, 4000) == SE_OK);
    dstudio::DecodedAudio d;
    std::string err;
    REQUIRE(dstudio::decode_wav_file(f.path.c_str(), d, err) == SE_OK);
    REQUIRE(d.frames == 4000);
    bool same = true;
    for (std::size_t i = 0; i < 4000; ++i) same = same && d.planar[0][i] == src[i] && d.planar[1][i] == 0.0f;
    CHECK(same);
}

TEST_CASE(preview_rejects_bad_arguments) {
    EngineHandle e(48000, 64);
    const uint64_t a = load_asset(e, {ramp(100)});
    CHECK_EQ(se_preview_play(e, 9999, 0, 1.0f, 1.0f, 0), SE_ERR_NOT_FOUND);
    CHECK_EQ(se_preview_play(e, a, 0, 2.5f, 1.0f, 0), SE_ERR_INVALID_ARG);
    CHECK_EQ(se_preview_play(e, a, 0, 1.0f, 0.1f, 0), SE_ERR_INVALID_ARG);
    CHECK_EQ(se_preview_set(e, NAN, 1.0f, 0), SE_ERR_INVALID_ARG);
    CHECK_EQ(se_preview_get(e, nullptr), SE_ERR_INVALID_ARG);
}
