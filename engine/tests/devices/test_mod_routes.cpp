// Modulation: the LFO and chain head (rack) sources, routes into built-in
// and plugin parameters (se_mod_route), bypass, plugin bases and RT safety.
#include "device_harness.hpp"
#include "devices/lfo.hpp"

using namespace testutil;
using namespace devtest;

namespace {

// LFO defaults (saine_core::builtin::lfo): shape, sync, rate, rateSync, depth, phase.
const std::vector<float> kLfo = {0.f, 1.f, 1.f, 8.f, 100.f, 0.f};
const std::vector<float> kRack(8, 0.0f);
const std::vector<float> kComp = {-24.f, 4.f, 10.f, 100.f, 6.f, 0.f, 100.f, 0.f, 0.f, 0.f, 0.f};
// Compressor makeup_gain (index 5): -12..24 dB linear; 0 dB is 1/3 of the range.
constexpr uint32_t kMakeup = 5;

se_mod_route makeup_route(uint64_t target, float min, float max, bool absolute = false, uint8_t slot = 0) {
    se_mod_route r{};
    r.target_device_id = target;
    r.param_id = kMakeup;
    r.kind = SE_MOD_BUILTIN;
    r.slot = slot;
    r.absolute = absolute ? 1 : 0;
    r.min = min;
    r.max = max;
    r.p_min = -12.f;
    r.p_max = 24.f;
    r.p_skew = 1.f;
    r.p_step = 0.f;
    return r;
}

// Values of output `o` at every grid point over `frames` (blocks of 256).
std::vector<float> lfo_points(DeviceRig& rig, std::size_t frames, bool playing, double tempo = 120.0, int64_t start = 0) {
    std::vector<float> out;
    saine::RtTransport tr;
    tr.sample_rate = rig.rate();
    tr.playing = playing;
    tr.tempo_bpm = tempo;
    for (std::size_t pos = 0; pos < frames; pos += 256) {
        tr.position = start + static_cast<int64_t>(pos);
        rig.instance().run_modulation(256, tr, nullptr, 0);
        for (uint32_t k = 0; k < 256 / SE_MOD_GRID_FRAMES; ++k) out.push_back(rig.instance().mod_point(k, 0)[0]);
    }
    return out;
}

// Left-channel peak of an interleaved render between two frames.
double peak_l(const std::vector<float>& out, std::size_t from, std::size_t to) {
    double m = 0.0;
    for (std::size_t i = 2 * from; i < 2 * to; i += 2) m = std::max(m, static_cast<double>(std::fabs(out[i])));
    return m;
}

} // namespace

TEST_CASE(lfo_shapes_follow_the_spec) {
    using namespace saine::lfo;
    CHECK_NEAR(wave(kSine, 0.25), 1.0, 1e-5);
    CHECK_NEAR(wave(kTriangle, 0.25), 1.0, 1e-6);
    CHECK_NEAR(wave(kTriangle, 0.75), -1.0, 1e-6);
    CHECK_NEAR(wave(kSawUp, 0.0), -1.0, 1e-6);
    CHECK_NEAR(wave(kSawDown, 0.0), 1.0, 1e-6);
    CHECK(wave(kSquare, 0.2) == 1.0f && wave(kSquare, 0.7) == -1.0f);
    CHECK(wave(kRandom, 3.1) == wave(kRandom, 3.9));
    CHECK(wave(kRandom, 3.5) == wave(kRandom, 4096.0 + 3.5));
    CHECK_NEAR(wave(kSmoothRandom, 7.0), cycle_random(7), 1e-6);
    for (int64_t c = -5; c < 5000; ++c) CHECK(cycle_random(c) >= -1.0f && cycle_random(c) <= 1.0f);
    // The same hash as saine_core::modulation::cycle_random (pinned values).
    CHECK(cycle_random(0) == cycle_random(4096));
}

TEST_CASE(lfo_free_rate_and_synced_clock) {
    // Free, 1 Hz sine: the wave peaks at 0.25 s and bottoms out at 0.75 s.
    std::vector<float> v = kLfo;
    v[1] = 0.f;
    DeviceRig rig("lfo", v);
    auto p = lfo_points(rig, 48000, false);
    CHECK_NEAR(p[0], 0.5, 1e-4);
    CHECK_NEAR(p[12000 / SE_MOD_GRID_FRAMES], 1.0, 1e-3);
    CHECK_NEAR(p[36000 / SE_MOD_GRID_FRAMES], 0.0, 1e-3);
    // Phase 25% starts at the peak.
    DeviceRig shifted("lfo", v);
    shifted.set("phase", 25.f);
    CHECK_NEAR(lfo_points(shifted, 256, false)[0], 1.0, 1e-4);

    // Sync 1/4 at 120 BPM: one cycle per 24000 frames, locked to the song position.
    DeviceRig sync("lfo", kLfo);
    auto s = lfo_points(sync, 48000, true, 120.0, 48000);
    CHECK_NEAR(s[0], 0.5, 1e-4);                             // a bar line: phase 0
    CHECK_NEAR(s[6000 / SE_MOD_GRID_FRAMES], 1.0, 1e-3);     // a sixteenth later
    CHECK_NEAR(s[30000 / SE_MOD_GRID_FRAMES], 1.0, 1e-3);    // one cycle later
    // Stopped, it keeps counting at the tempo from where it was: 188 blocks
    // ended 128 frames past beat 8, so the peak comes 5872 frames later.
    auto stopped = lfo_points(sync, 6144, false);
    CHECK_NEAR(stopped[0], 0.5 + 0.5 * std::sin(2.0 * 3.14159265358979323846 * 128.0 / 24000.0), 1e-3);
    CHECK_NEAR(stopped[5888 / SE_MOD_GRID_FRAMES], 1.0, 1e-3);
    // 1/8 doubles the speed.
    DeviceRig eighth("lfo", kLfo);
    eighth.set("rateSync", 11.f);
    CHECK_NEAR(lfo_points(eighth, 3072, true)[3008 / SE_MOD_GRID_FRAMES], 1.0, 1e-3);
    // Depth is the gain output; readouts report the latest position.
    eighth.set("depth", 40.f);
    lfo_points(eighth, 256, true);
    CHECK_NEAR(eighth.instance().mod_point(0, 0)[1], 0.4, 1e-6);
    eighth.run_mono(Signal(256, 0.1f));
    CHECK(eighth.readouts()[1] == 0.4f);
}

TEST_CASE(rack_macros_are_outputs) {
    DeviceRig rig("device-rack", kRack);
    rig.set("macro3", 75.f);
    saine::RtTransport tr;
    rig.instance().run_modulation(64, tr, nullptr, 0);
    CHECK(rig.instance().mod_outputs() == 8);
    CHECK_NEAR(rig.instance().mod_point(1, 2)[0], 0.75, 1e-6);
    CHECK(rig.instance().mod_point(1, 2)[1] == 1.0f);
    // Automation events land on their grid point.
    const saine::ParamEvent ev[] = {{64, 0, 50.0}};
    rig.instance().run_modulation(128, tr, ev, 1);
    CHECK(rig.instance().mod_point(1, 0)[0] == 0.0f);
    CHECK_NEAR(rig.instance().mod_point(2, 0)[0], 0.5, 1e-6);
    // Audio passes through untouched.
    Signal x = tone(256, 440.0, 48000.0, 0.3);
    CHECK(rig.run_mono(x) == x);
}

TEST_CASE(state_v2_blobs_carry_routes) {
    const uint8_t* v = nullptr;
    const uint8_t* routes = nullptr;
    uint32_t count = 0, rc = 0;
    uint64_t sc = 0;
    auto ok = state_blob_v2(kLfo, {makeup_route(7, -0.2f, 0.2f)});
    CHECK(saine::parse_builtin_state(ok.data(), ok.size(), v, count, sc, routes, rc) == nullptr);
    CHECK(count == 6 && rc == 1 && routes != nullptr);
    auto bad = state_blob_v2(kLfo, {makeup_route(7, std::nanf(""), 0.2f)});
    CHECK(saine::parse_builtin_state(bad.data(), bad.size(), v, count, sc, routes, rc) != nullptr);
    auto kind = makeup_route(7, 0.f, 0.f);
    kind.kind = 9;
    auto bad2 = state_blob_v2(kLfo, {kind});
    CHECK(saine::parse_builtin_state(bad2.data(), bad2.size(), v, count, sc, routes, rc) != nullptr);
    auto cut = ok;
    cut.pop_back();
    CHECK(saine::parse_builtin_state(cut.data(), cut.size(), v, count, sc, routes, rc) != nullptr);
}

TEST_CASE(norm_conversions_match_the_core_codec) {
    saine::BuiltinMod log{0, true, 1.f, 1000.f, 1.f, 0.f, nullptr, nullptr};
    CHECK_NEAR(saine::mod_from_norm(log, 0.5f), std::sqrt(1000.0), 1e-2);
    CHECK_NEAR(saine::mod_to_norm(log, 10.f), 1.0 / 3.0, 1e-5);
    saine::BuiltinMod stepped{0, false, 0.f, 6.f, 1.f, 1.f, nullptr, nullptr};
    CHECK(saine::mod_from_norm(stepped, 0.49f) == 3.0f);
    saine::BuiltinMod skew{0, false, 0.f, 100.f, 2.f, 0.f, nullptr, nullptr};
    CHECK_NEAR(saine::mod_from_norm(skew, saine::mod_to_norm(skew, 30.f)), 30.0, 1e-3);
}

TEST_CASE(lfo_moves_a_builtin_parameter_in_the_engine) {
    EngineHandle e(48000, 256);
    const uint64_t a = load_asset(e, {sine(48000, 400.0, 48000.0, 0.1), sine(48000, 400.0, 48000.0, 0.1)});
    std::vector<float> comp = kComp;
    comp[0] = 0.f; // threshold at 0 dB, ratio 1: no compression
    comp[1] = 1.f;
    std::vector<float> lfo = kLfo;
    lfo[0] = 4.f; // square
    lfo[1] = 0.f; // free, 1 Hz
    uint64_t id = 10; // fresh instances per run: a free LFO starts its cycle when created
    const auto run = [&](bool lfo_on, float depth) {
        DeviceSession s;
        auto& t = s.add_track(1, a, 48000);
        std::vector<float> l = lfo;
        l[4] = depth;
        id += 2;
        DeviceSession::add_device_state(t, id, "lfo", state_blob_v2(l, {makeup_route(id + 1, -1.f / 3.f, 1.f / 3.f)}), lfo_on);
        DeviceSession::add_device(t, id + 1, "compressor", comp);
        REQUIRE(s.publish(e) == SE_OK);
        REQUIRE(se_transport_seek(e, 0) == SE_OK);
        REQUIRE(se_transport_play(e) == SE_OK);
        return render(e, 48000, 256);
    };
    const double in = 0.1 * 0.7071; // centre pan law
    auto out = run(true, 100.f);
    // First half: +1/3 of the range -> +12 dB; second half: -1/3 -> -12 dB.
    CHECK_NEAR(db(peak_l(out, 6000, 22000)) - db(in), 12.0, 0.5);
    CHECK_NEAR(db(peak_l(out, 30000, 46000)) - db(in), -12.0, 0.5);
    // Depth scales the movement.
    out = run(true, 50.f);
    CHECK_NEAR(db(peak_l(out, 6000, 22000)) - db(in), 6.0, 0.5);
    // A bypassed LFO contributes nothing.
    out = run(false, 100.f);
    CHECK_NEAR(db(peak_l(out, 6000, 46000)) - db(in), 0.0, 0.3);
}

TEST_CASE(macros_set_member_parameters) {
    EngineHandle e(48000, 256);
    const uint64_t a = load_asset(e, {sine(24000, 400.0, 48000.0, 0.1), sine(24000, 400.0, 48000.0, 0.1)});
    std::vector<float> comp = kComp;
    comp[0] = 0.f;
    comp[1] = 1.f;
    comp[kMakeup] = -12.f; // the knob's own value is ignored while a macro maps it
    std::vector<float> rack = kRack;
    rack[2] = 100.f;
    DeviceSession s;
    auto& t = s.add_track(1, a, 24000);
    // Macro 3 maps makeup over 0..2/3 of its range: at 100% -> +12 dB.
    DeviceSession::add_device_state(t, 20, "device-rack", state_blob_v2(rack, {makeup_route(21, 0.f, 2.f / 3.f, true, 2)}));
    DeviceSession::add_device(t, 21, "compressor", comp);
    REQUIRE(s.publish(e) == SE_OK);
    REQUIRE(se_transport_play(e) == SE_OK);
    auto out = render(e, 24000, 256);
    CHECK_NEAR(db(peak_l(out, 4000, 20000)) - db(0.1 * 0.7071), 12.0, 0.5);
    // Moving the macro live (no republish) moves the target.
    REQUIRE(se_builtin_set_param(e, 20, 2, 50.f) == SE_OK);
    REQUIRE(se_transport_seek(e, 0) == SE_OK);
    out = render(e, 24000, 256);
    CHECK_NEAR(db(peak_l(out, 4000, 20000)) - db(0.1 * 0.7071), 0.0, 0.5);
}

TEST_CASE(an_lfo_without_targets_keeps_running) {
    EngineHandle e(48000, 256);
    const uint64_t a = load_asset(e, {sine(48000, 400.0, 48000.0, 0.1), sine(48000, 400.0, 48000.0, 0.1)});
    std::vector<float> lfo = kLfo;
    lfo[1] = 0.f; // free, 1 Hz
    DeviceSession s;
    auto& t = s.add_track(1, a, 48000);
    DeviceSession::add_device_state(t, 40, "lfo", state_blob_v2(lfo, {}));
    REQUIRE(s.publish(e) == SE_OK);
    REQUIRE(se_transport_play(e) == SE_OK);
    float ro[3] = {};
    std::size_t n = 0;
    render(e, 12000, 256);
    REQUIRE(se_builtin_readouts(e, 40, ro, 3, &n) == SE_OK);
    const float first = ro[2];
    render(e, 12000, 256);
    REQUIRE(se_builtin_readouts(e, 40, ro, 3, &n) == SE_OK);
    CHECK(n == 3);
    CHECK_NEAR(ro[2] - first, 0.25, 0.02); // a quarter cycle later
}

TEST_CASE(lfo_moves_a_plugin_parameter_and_restores_its_base) {
    const std::string clap_gain = std::string(SAINE_TEST_PLUGIN_DIR) + "/SaineTestGain.clap";
    EngineHandle e(48000, 256);
    const uint64_t a = load_asset(e, {sine(48000, 400.0, 48000.0, 0.5), sine(48000, 400.0, 48000.0, 0.5)});
    std::vector<float> lfo = kLfo;
    lfo[0] = 4.f;
    lfo[1] = 0.f;
    se_mod_route r{};
    r.target_device_id = 31;
    r.param_id = 0; // Gain, 0..1, default 0.5
    r.kind = SE_MOD_PLUGIN;
    r.min = -0.25f;
    r.max = 0.25f;
    const auto session = [&](bool routed) {
        DeviceSession s;
        auto& t = s.add_track(1, a, 48000);
        DeviceSession::add_device_state(t, 30, "lfo", state_blob_v2(lfo, routed ? std::vector<se_mod_route>{r} : std::vector<se_mod_route>{}));
        DeviceSession::add_clap(t, 31, clap_gain, "studio.saine.test.gain");
        REQUIRE(s.publish(e) == SE_OK);
    };
    session(true);
    REQUIRE(se_transport_play(e) == SE_OK);
    auto out = render(e, 48000, 256);
    const double in = 0.5 * 0.7071;
    CHECK_NEAR(peak_l(out, 6000, 22000) / in, 0.75, 0.02);
    CHECK_NEAR(peak_l(out, 30000, 46000) / in, 0.25, 0.02);
    float base = 0.f;
    REQUIRE(se_mod_base(e, 31, 0, &base) == SE_OK);
    CHECK_NEAR(base, 0.5, 1e-6);
    // The card sets the base while modulated.
    REQUIRE(se_plugin_set_param(e, 31, 0, 0.4f) == SE_OK);
    REQUIRE(se_transport_seek(e, 0) == SE_OK);
    out = render(e, 24000, 256);
    CHECK_NEAR(peak_l(out, 6000, 22000) / in, 0.65, 0.02);
    // Removing the route puts the base back.
    session(false);
    CHECK(se_mod_base(e, 31, 0, &base) == SE_ERR_NOT_FOUND);
    out = render(e, 4096, 256);
    CHECK_NEAR(peak_l(out, 2048, 4096) / in, 0.4, 0.02);
    // Unmodulated, the card sets the plugin directly.
    REQUIRE(se_plugin_set_param(e, 31, 0, 0.1f) == SE_OK);
    out = render(e, 4096, 256);
    CHECK_NEAR(peak_l(out, 2048, 4096) / in, 0.1, 0.02);
    CHECK(se_plugin_set_param(e, 30, 0, 0.1f) == SE_ERR_NOT_FOUND); // a built-in
    CHECK(se_plugin_set_param(e, 31, 0, std::nanf("")) == SE_ERR_INVALID_ARG);
}

TEST_CASE(modulated_engine_path_does_not_allocate) {
    EngineHandle e(48000, 256);
    const uint64_t a = load_asset(e, {sine(48000, 440.0, 48000.0, 0.5), sine(48000, 220.0, 48000.0, 0.5)});
    DeviceSession s;
    for (uint64_t t = 1; t <= 4; ++t) {
        auto& tr = s.add_track(t, a, 48000);
        std::vector<float> lfo = kLfo;
        lfo[0] = static_cast<float>(t % 7);
        DeviceSession::add_device_state(tr, t * 10, "lfo", state_blob_v2(lfo, {makeup_route(t * 10 + 2, -0.2f, 0.2f)}));
        std::vector<float> rack = kRack;
        rack[0] = 40.f;
        DeviceSession::add_device_state(tr, t * 10 + 1, "device-rack", state_blob_v2(rack, {makeup_route(t * 10 + 2, 0.1f, 0.6f, true)}));
        DeviceSession::add_device(tr, t * 10 + 2, "compressor", kComp);
        DeviceSession::add_lane(tr, t * 10 + 1, 0, {{0, 0.0f, 0.0f}, {48000, 100.0f, 0.0f}});
    }
    REQUIRE(s.publish(e) == SE_OK);
    REQUIRE(se_transport_play(e) == SE_OK);
    std::vector<float> buf(2 * 1024);
    REQUIRE(se_render_offline(e, buf.data(), 256) == SE_OK);
    const long before = rt_alloc_count();
    for (int i = 0; i < 300; ++i) {
        RtAllocGuard g;
        REQUIRE(se_render_offline(e, buf.data(), (i % 3 == 0) ? 1 : 256) == SE_OK);
    }
    CHECK_EQ(rt_alloc_count() - before, 0);
    for (float x : buf) CHECK(std::isfinite(x));
}
