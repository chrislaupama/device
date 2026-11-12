// Parameter automation (docs/adr/0005-automation.md): the evaluation
// contract, volume / pan / master ramps, synth and plugin parameters (CLAP,
// VST3, AU), loop wrap and seek, delay-compensation alignment, export,
// editor parameter-change capture, and RT safety with dense automation.
//
// Audio tracks play a DC clip (1.0) panned hard left, so the left channel is
// the fader gain itself.
#include "test_harness.hpp"
#include "test_hooks.hpp"
#include "test_util.hpp"
#include "wav.hpp"

#include <algorithm>
#include <chrono>
#include <cmath>
#include <cstring>
#include <string>
#include <thread>
#include <vector>

using namespace testutil;

namespace {

const std::string kPluginDir = DSTUDIO_TEST_PLUGIN_DIR;
const std::string kClapGain = kPluginDir + "/DStudioTestGain.clap";
const std::string kVst3Gain = kPluginDir + "/DStudioTestGain.vst3";
const std::string kClapLatency = kPluginDir + "/DStudioTestLatency.clap";
const char* const kVst3GainUid = "5A17E5A16E0000018A1E7E576A170001";
const char* const kAuLowpass = "aufx lpas appl";
#if defined(__APPLE__)
constexpr bool kHaveAu = true;
#else
constexpr bool kHaveAu = false;
#endif

// ---- reference evaluation (independent of the engine's code)
struct Pt {
    int64_t frame;
    float value;
    float curve;
};

float ref_shape(float t, float c) {
    t = std::clamp(t, 0.0f, 1.0f);
    c = std::clamp(c, -1.0f, 1.0f);
    if (std::fabs(c) < 1e-6f) return t;
    return c > 0.0f ? std::pow(t, 1.0f + 3.0f * c) : 1.0f - std::pow(1.0f - t, 1.0f - 3.0f * c);
}

double ref_value(const std::vector<Pt>& p, int64_t t, bool log = false) {
    if (t <= p.front().frame) return p.front().value;
    std::size_t i = 0;
    while (i < p.size() && p[i].frame <= t) ++i;
    if (i >= p.size()) return p.back().value;
    const Pt& a = p[i - 1];
    const Pt& b = p[i];
    const double s = ref_shape(static_cast<float>(t - a.frame) / static_cast<float>(b.frame - a.frame), a.curve);
    if (log && a.value > 0 && b.value > 0)
        return std::exp(std::log(static_cast<double>(a.value)) +
                        (std::log(static_cast<double>(b.value)) - std::log(static_cast<double>(a.value))) * s);
    return a.value + (static_cast<double>(b.value) - a.value) * s;
}

double db_gain(double db) { return db <= -60.0 ? 0.0 : std::pow(10.0, db / 20.0); }

// ---- session with lanes (owned storage)
struct Lane {
    se_auto_lane_desc d{};
    std::vector<se_auto_point> points;
};

Lane lane(uint8_t target, const std::vector<Pt>& pts, uint64_t device = 0, uint32_t param = 0, uint16_t synth = 0,
          bool log = false) {
    Lane l;
    l.d.target = target;
    l.d.device_id = device;
    l.d.param_id = param;
    l.d.synth_param = synth;
    l.d.logarithmic = log ? 1 : 0;
    for (const Pt& p : pts) l.points.push_back(se_auto_point{p.frame, p.value, p.curve});
    return l;
}

struct ASession {
    struct Track {
        se_track_desc_v6 d{};
        std::vector<se_clip_desc> clips;
        std::vector<se_note_desc> notes;
        std::vector<se_device_desc> devices;
        std::vector<Lane> lanes;
        std::vector<se_auto_lane_desc> lane_descs;
    };
    std::vector<Track> tracks;
    std::vector<Lane> master;
    std::vector<se_auto_lane_desc> master_descs;
    float master_gain = 1.0f;

    Track& audio(uint64_t id, std::vector<se_clip_desc> clips, std::vector<Lane> lanes = {},
                 std::vector<se_device_desc> devices = {}, float pan = -1.0f) {
        Track t;
        t.d.track_id = id;
        t.d.gain = 1.0f;
        t.d.pan = pan;
        t.clips = std::move(clips);
        t.lanes = std::move(lanes);
        t.devices = std::move(devices);
        tracks.push_back(std::move(t));
        return tracks.back();
    }
    int32_t publish(se_engine* e) {
        std::vector<se_track_desc_v6> v;
        for (Track& t : tracks) {
            t.lane_descs.clear();
            for (Lane& l : t.lanes) {
                se_auto_lane_desc d = l.d;
                d.points = l.points.data();
                d.point_count = static_cast<uint32_t>(l.points.size());
                t.lane_descs.push_back(d);
            }
            se_track_desc_v6 d = t.d;
            d.clips = t.clips.data();
            d.clip_count = static_cast<uint32_t>(t.clips.size());
            d.notes = t.notes.data();
            d.note_count = static_cast<uint32_t>(t.notes.size());
            d.devices = t.devices.data();
            d.device_count = static_cast<uint32_t>(t.devices.size());
            d.lanes = t.lane_descs.data();
            d.lane_count = static_cast<uint32_t>(t.lane_descs.size());
            d.track_name = "Track";
            v.push_back(d);
        }
        master_descs.clear();
        for (Lane& l : master) {
            se_auto_lane_desc d = l.d;
            d.points = l.points.data();
            d.point_count = static_cast<uint32_t>(l.points.size());
            master_descs.push_back(d);
        }
        se_session_desc_v6 s{};
        s.tracks = v.data();
        s.track_count = static_cast<uint32_t>(v.size());
        s.master_gain = master_gain;
        s.master_lanes = master_descs.data();
        s.master_lane_count = static_cast<uint32_t>(master_descs.size());
        return se_session_publish_v6(e, &s);
    }
};

se_device_desc clap_dev(uint64_t id, const std::string& path, const char* plugin_id) {
    se_device_desc d{};
    d.device_id = id;
    d.name = "CLAP";
    d.plugin_id = plugin_id;
    d.path = path.c_str();
    d.format = SE_PLUGIN_CLAP;
    d.enabled = 1;
    return d;
}
se_device_desc clap_gain(uint64_t id) { return clap_dev(id, kClapGain, "studio.dstudio.test.gain"); }
se_device_desc vst3_gain(uint64_t id) {
    se_device_desc d{};
    d.device_id = id;
    d.name = "VST3";
    d.plugin_id = kVst3GainUid;
    d.path = kVst3Gain.c_str();
    d.format = SE_PLUGIN_VST3;
    d.enabled = 1;
    return d;
}
se_device_desc au_dev(uint64_t id, const char* au_id) {
    se_device_desc d{};
    d.device_id = id;
    d.name = "AU";
    d.plugin_id = au_id;
    d.format = SE_PLUGIN_AU;
    d.enabled = 1;
    return d;
}
std::vector<std::vector<uint8_t>> g_blobs; // state blobs outlive the descs
se_device_desc clap_latency(uint64_t id, uint32_t n) {
    g_blobs.push_back({static_cast<uint8_t>(n), static_cast<uint8_t>(n >> 8), static_cast<uint8_t>(n >> 16),
                       static_cast<uint8_t>(n >> 24)});
    se_device_desc d = clap_dev(id, kClapLatency, "studio.dstudio.test.latency");
    d.state = g_blobs.back().data();
    d.state_len = g_blobs.back().size();
    return d;
}

std::vector<float> chan(const std::vector<float>& inter, int c) {
    std::vector<float> v(inter.size() / 2);
    for (std::size_t i = 0; i < v.size(); ++i) v[i] = inter[2 * i + static_cast<std::size_t>(c)];
    return v;
}

std::vector<float> play(se_engine* e, uint32_t frames, int64_t from = 0, uint32_t block = 256) {
    se_transport_seek(e, from);
    se_transport_play(e);
    return render(e, frames, block);
}

uint64_t dc_asset(se_engine* e, std::size_t frames, float v = 1.0f) { return load_asset(e, {constant(frames, v)}); }

// High-frequency content: RMS of the second difference relative to the RMS.
double brightness(const std::vector<float>& x, std::size_t from, std::size_t to) {
    double d = 0, a = 0;
    for (std::size_t i = from + 2; i < to; ++i) {
        const double d2 = static_cast<double>(x[i]) - 2.0 * x[i - 1] + x[i - 2];
        d += d2 * d2;
        a += static_cast<double>(x[i]) * x[i];
    }
    return a > 0 ? std::sqrt(d / a) : 0.0;
}

std::vector<float> noise(std::size_t n, float amp = 0.3f) {
    std::vector<float> v(n);
    uint32_t s = 0x1234567u;
    for (float& x : v) {
        s = s * 1664525u + 1013904223u;
        x = amp * (static_cast<float>(s >> 8) / 8388608.0f - 1.0f);
    }
    return v;
}

std::vector<se_plugin_param_change> take_changes(se_engine* e) {
    std::vector<se_plugin_param_change> out;
    se_plugin_param_change buf[16];
    for (;;) {
        std::size_t n = 0;
        REQUIRE(se_plugin_take_param_changes(e, buf, 16, &n) == SE_OK);
        out.insert(out.end(), buf, buf + n);
        if (n < 16) return out;
    }
}

} // namespace

// ---------------------------------------------------------------- evaluation

TEST_CASE(automation_eval_contract) {
    // Same cases as dstudio_core::automation's tests, through se_automation_eval.
    Lane l = lane(SE_AUTO_VOLUME, {{100, -12.0f, 0.0f}, {200, 0.0f, 0.0f}, {200, -6.0f, 0.0f}});
    l.d.points = l.points.data();
    l.d.point_count = 3;
    const int64_t frames[] = {0, 100, 150, 199, 200, 10000};
    float out[6] = {};
    REQUIRE(se_automation_eval(&l.d, frames, 6, out) == SE_OK);
    CHECK_EQ(out[0], -12.0f);
    CHECK_EQ(out[1], -12.0f);
    CHECK_EQ(out[2], -6.0f);
    CHECK_NEAR(out[3], -0.12, 1e-5);
    CHECK_EQ(out[4], -6.0f); // a vertical step takes the later point
    CHECK_EQ(out[5], -6.0f);

    // Curves and geometric interpolation against the independent reference.
    const std::vector<Pt> pts = {{0, 100.0f, 0.7f}, {1000, 10000.0f, -0.4f}, {3000, 500.0f, 0.0f}, {3000, 800.0f, 1.0f},
                                 {5000, 20.0f, 0.0f}};
    for (bool log : {false, true}) {
        Lane c = lane(SE_AUTO_SYNTH, pts, 0, 0, SE_SYNTH_P_FILTER_CUTOFF, log);
        c.d.points = c.points.data();
        c.d.point_count = static_cast<uint32_t>(c.points.size());
        std::vector<int64_t> f;
        for (int64_t t = -10; t < 5100; t += 7) f.push_back(t);
        std::vector<float> v(f.size());
        REQUIRE(se_automation_eval(&c.d, f.data(), f.size(), v.data()) == SE_OK);
        for (std::size_t i = 0; i < f.size(); ++i) {
            const double r = ref_value(pts, f[i], log);
            if (!(std::fabs(v[i] - r) <= 1e-5 * std::max(1.0, std::fabs(r)))) {
                CHECK_NEAR(v[i], r, 1e-5 * std::max(1.0, std::fabs(r)));
                break;
            }
        }
    }
    // Validation.
    Lane bad = lane(SE_AUTO_VOLUME, {{10, 0.0f, 0.0f}, {5, 0.0f, 0.0f}});
    bad.d.points = bad.points.data();
    bad.d.point_count = 2;
    CHECK(se_automation_eval(&bad.d, frames, 1, out) == SE_ERR_INVALID_ARG);
    Lane bad_curve = lane(SE_AUTO_VOLUME, {{0, 0.0f, 2.0f}});
    bad_curve.d.points = bad_curve.points.data();
    bad_curve.d.point_count = 1;
    CHECK(se_automation_eval(&bad_curve.d, frames, 1, out) == SE_ERR_INVALID_ARG);
    Lane bad_synth = lane(SE_AUTO_SYNTH, {{0, 5.0f, 0.0f}}, 0, 0, SE_SYNTH_P_FILTER_CUTOFF);
    bad_synth.d.points = bad_synth.points.data();
    bad_synth.d.point_count = 1;
    CHECK(se_automation_eval(&bad_synth.d, frames, 1, out) == SE_ERR_INVALID_ARG);
}

TEST_CASE(automation_publish_validation) {
    EngineHandle e(48000, 256);
    const uint64_t a = dc_asset(e, 1000);
    ASession s;
    s.audio(1, {clip(1, a, 0, 0, 1000)}, {lane(SE_AUTO_VOLUME, {{10, 0.0f, 0.0f}, {5, 0.0f, 0.0f}})});
    CHECK(s.publish(e) == SE_ERR_INVALID_ARG);
    s.tracks[0].lanes = {lane(9, {{0, 0.0f, 0.0f}})};
    CHECK(s.publish(e) == SE_ERR_INVALID_ARG);
    s.tracks[0].lanes = {lane(SE_AUTO_SYNTH, {{0, 0.5f, 0.0f}}, 0, 0, SE_SYNTH_P_COUNT)};
    CHECK(s.publish(e) == SE_ERR_INVALID_ARG);
    s.tracks[0].lanes = {lane(SE_AUTO_PAN, {{0, NAN, 0.0f}})};
    CHECK(s.publish(e) == SE_ERR_INVALID_ARG);
    s.tracks[0].lanes = {lane(SE_AUTO_PLUGIN, {{0, 0.5f, 0.0f}}, 77, 0)}; // device not on the track: inactive
    CHECK(s.publish(e) == SE_OK);
}

// ---------------------------------------------------------------- volume / pan / master

TEST_CASE(automation_volume_ramp_follows_value_at_without_zipper) {
    EngineHandle e(48000, 512);
    const uint64_t a = dc_asset(e, 60000);
    const std::vector<Pt> pts = {{0, -60.0f, 0.0f}, {48000, 0.0f, 0.0f}};
    ASession s;
    s.audio(1, {clip(1, a, 0, 0, 60000)}, {lane(SE_AUTO_VOLUME, pts)});
    REQUIRE(s.publish(e) == SE_OK);
    const std::vector<float> l = chan(play(e, 52000, 0, 333), 0); // odd block size
    double max_err = 0, max_step = 0;
    for (std::size_t i = 0; i < l.size(); ++i) {
        max_err = std::max(max_err, std::fabs(l[i] - db_gain(ref_value(pts, static_cast<int64_t>(i)))));
        if (i > 1) max_step = std::max(max_step, std::fabs(static_cast<double>(l[i]) - l[i - 1]));
    }
    std::fprintf(stderr, "volume ramp: max error %.3g, max step %.3g\n", max_err, max_step);
    CHECK(max_err <= 1e-4);
    CHECK(max_step <= 2e-4); // the steepest exact slope is ~1.44e-4 per sample
    CHECK(l[1] <= 1.01e-3);  // leaving silence: -59.999 dB
    CHECK_EQ(l[0], 0.0f);    // -60 dB is silence
    CHECK_NEAR(l[50000], 1.0, 1e-6);

    // The automated value replaces the static gain (0.25 is ignored).
    s.tracks[0].d.gain = 0.25f;
    REQUIRE(s.publish(e) == SE_OK);
    CHECK_NEAR(chan(play(e, 100, 50000), 0)[0], 1.0, 1e-6);
}

TEST_CASE(automation_pan_and_master_volume) {
    EngineHandle e(48000, 256);
    const uint64_t a = dc_asset(e, 20000);
    const std::vector<Pt> pan = {{0, -1.0f, 0.0f}, {9600, 1.0f, -0.5f}};
    ASession s;
    s.audio(1, {clip(1, a, 0, 0, 20000)}, {lane(SE_AUTO_PAN, pan)}, {}, 0.0f);
    REQUIRE(s.publish(e) == SE_OK);
    const std::vector<float> out = play(e, 12000);
    double max_err = 0;
    for (std::size_t i = 0; i < out.size() / 2; ++i) {
        const double p = ref_value(pan, static_cast<int64_t>(i));
        const double th = (p + 1.0) * 3.14159265358979323846 / 4.0;
        max_err = std::max({max_err, std::fabs(out[2 * i] - std::cos(th)), std::fabs(out[2 * i + 1] - std::sin(th))});
    }
    std::fprintf(stderr, "pan: max error %.3g\n", max_err);
    CHECK(max_err <= 1e-4);
    CHECK_NEAR(out[0], 1.0, 1e-6); // hard left at the start
    CHECK_NEAR(out[1], 0.0, 1e-6);

    // Master volume: replaces master_gain, applies after the track sum.
    const std::vector<Pt> mv = {{0, 0.0f, 0.0f}, {4800, -20.0f, 0.3f}, {4800, -6.0f, 0.0f}};
    ASession m;
    m.audio(1, {clip(1, a, 0, 0, 20000)});
    m.master = {lane(SE_AUTO_VOLUME, mv), lane(SE_AUTO_PAN, {{0, 1.0f, 0.0f}})}; // pan on master: ignored
    m.master_gain = 0.1f;
    REQUIRE(m.publish(e) == SE_OK);
    const std::vector<float> l = chan(play(e, 8000), 0);
    max_err = 0;
    for (std::size_t i = 0; i < l.size(); ++i)
        max_err = std::max(max_err, std::fabs(l[i] - db_gain(ref_value(mv, static_cast<int64_t>(i)))));
    std::fprintf(stderr, "master: max error %.3g\n", max_err);
    CHECK(max_err <= 1e-4);
    CHECK_NEAR(l[4799], db_gain(-20.0), 1e-4); // the step lands exactly on its frame
    CHECK_NEAR(l[4800], db_gain(-6.0), 1e-6);
}

TEST_CASE(automation_lane_removal_restores_static_values_and_empty_lanes_are_ignored) {
    EngineHandle e(48000, 256);
    const uint64_t a = dc_asset(e, 20000);
    ASession s;
    s.audio(1, {clip(1, a, 0, 0, 20000)}, {lane(SE_AUTO_VOLUME, {{0, -60.0f, 0.0f}})});
    s.tracks[0].d.gain = 0.5f;
    REQUIRE(s.publish(e) == SE_OK);
    CHECK_EQ(chan(play(e, 512), 0)[300], 0.0f); // lane: silence
    s.tracks[0].lanes = {lane(SE_AUTO_VOLUME, {})}; // a lane without points does nothing
    REQUIRE(s.publish(e) == SE_OK);
    CHECK_NEAR(chan(play(e, 512), 0)[300], 0.5, 1e-7);
    s.tracks[0].lanes = {lane(SE_AUTO_VOLUME, {{0, -60.0f, 0.0f}})};
    REQUIRE(s.publish(e) == SE_OK);
    CHECK_EQ(chan(play(e, 512), 0)[300], 0.0f);
    s.tracks[0].lanes.clear(); // removed: the static gain again, bit-exact
    REQUIRE(s.publish(e) == SE_OK);
    const std::vector<float> l = chan(play(e, 512), 0);
    for (float x : l) CHECK_EQ(x, 0.5f);
}

// ---------------------------------------------------------------- loop / seek

TEST_CASE(automation_loop_wrap_and_seek_jump_to_the_right_value) {
    EngineHandle e(48000, 256);
    const uint64_t a = dc_asset(e, 60000);
    const std::vector<Pt> pts = {{0, -40.0f, 0.0f}, {48000, 0.0f, 0.0f}};
    ASession s;
    s.audio(1, {clip(1, a, 0, 0, 60000)}, {lane(SE_AUTO_VOLUME, pts)});
    REQUIRE(s.publish(e) == SE_OK);
    REQUIRE(se_transport_set_loop(e, 1, 24000, 30001) == SE_OK);
    const std::vector<float> l = chan(play(e, 20000, 20000, 300), 0);
    int64_t pos = 20000;
    double max_err = 0;
    for (std::size_t i = 0; i < l.size(); ++i) {
        max_err = std::max(max_err, std::fabs(l[i] - db_gain(ref_value(pts, pos))));
        if (++pos == 30001) pos = 24000;
    }
    CHECK(max_err <= 1e-4); // including the samples right after each wrap
    REQUIRE(se_transport_set_loop(e, 0, 0, 0) == SE_OK);
    // Seek: the first sample after the jump has the new value.
    se_transport_seek(e, 40000);
    const std::vector<float> after = chan(render(e, 64, 64), 0);
    CHECK_NEAR(after[0], db_gain(ref_value(pts, 40000)), 1e-5);
    CHECK_NEAR(after[63], db_gain(ref_value(pts, 40063)), 1e-4);
    se_transport_seek(e, 1000);
    CHECK_NEAR(chan(render(e, 64, 64), 0)[0], db_gain(ref_value(pts, 1000)), 1e-5);
    // Stopped: holds the value at the playhead (tails); playing resumes the curve.
    se_transport_pause(e);
    se_transport_seek(e, 48000);
    se_transport_play(e);
    CHECK_NEAR(chan(render(e, 64, 64), 0)[10], 1.0, 1e-5);
}

// ---------------------------------------------------------------- synth

TEST_CASE(automation_synth_cutoff_sweep_brightens) {
    EngineHandle e(48000, 512);
    ASession s;
    ASession::Track& t = s.audio(1, {}, {lane(SE_AUTO_SYNTH, {{0, 150.0f, 0.0f}, {96000, 12000.0f, 0.0f}}, 0, 0,
                                               SE_SYNTH_P_FILTER_CUTOFF, true)});
    t.d.instrument = 1;
    t.notes = {se_note_desc{0, 96000, 48, 100, {}}};
    REQUIRE(s.publish(e) == SE_OK);
    const std::vector<float> l = chan(play(e, 96000, 0, 512), 0);
    const double b1 = brightness(l, 4800, 19200), b2 = brightness(l, 38400, 52800), b3 = brightness(l, 76800, 91200);
    std::fprintf(stderr, "synth cutoff sweep brightness %.4f %.4f %.4f\n", b1, b2, b3);
    CHECK(b2 > 1.5 * b1);
    CHECK(b3 > 1.5 * b2);

    // Without the lane the static cutoff (default) applies and is constant.
    s.tracks[0].lanes.clear();
    REQUIRE(s.publish(e) == SE_OK);
    const std::vector<float> st = chan(play(e, 96000, 0, 512), 0);
    const double s1 = brightness(st, 4800, 19200), s3 = brightness(st, 76800, 91200);
    CHECK(std::fabs(s3 - s1) < 0.2 * s1);
}

// ---------------------------------------------------------------- plugins

namespace {
// Expected output of the sample-accurate test gain plugin with a DC input:
// the lane value at the latest event frame <= i, where events are sent at
// each block start, every 64 timeline frames and at every point.
std::vector<double> expected_plugin_gain(const std::vector<Pt>& pts, uint32_t frames, uint32_t block) {
    std::vector<double> out(frames);
    double cur = 0;
    for (uint32_t i = 0; i < frames; ++i) {
        bool ev = i % block == 0 || i % 64 == 0;
        for (const Pt& p : pts) ev |= p.frame == i;
        if (ev) cur = std::clamp(ref_value(pts, i), 0.0, 1.0);
        out[i] = cur;
    }
    return out;
}
} // namespace

TEST_CASE(automation_clap_and_vst3_gain_follow_the_lane_sample_accurately) {
    const std::vector<Pt> pts = {{0, 0.0f, 0.0f},    {4800, 1.0f, 0.0f}, {6000, 1.0f, 0.0f},
                                 {6000, 0.25f, 0.0f}, {7001, 0.25f, 0.0f}, {7001, 0.75f, -0.6f},
                                 {9000, 0.1f, 0.0f}};
    for (int fmt = 0; fmt < 2; ++fmt) {
        EngineHandle e(48000, 256);
        const uint64_t a = dc_asset(e, 20000);
        ASession s;
        s.audio(1, {clip(1, a, 0, 0, 20000)}, {lane(SE_AUTO_PLUGIN, pts, 5, 0)}, {fmt == 0 ? clap_gain(5) : vst3_gain(5)});
        REQUIRE(s.publish(e) == SE_OK);
        const std::vector<float> l = chan(play(e, 10000, 0, 256), 0);
        const std::vector<double> want = expected_plugin_gain(pts, 10000, 256);
        std::size_t bad = 0;
        for (std::size_t i = 0; i < l.size(); ++i)
            if (std::fabs(l[i] - want[i]) > 1e-6) ++bad;
        std::fprintf(stderr, "%s: %zu samples off\n", fmt == 0 ? "CLAP" : "VST3", bad);
        CHECK_EQ(bad, std::size_t{0});
        CHECK_NEAR(l[5999], 1.0, 1e-6); // the step lands on its frame, between grid points
        CHECK_NEAR(l[6000], 0.25, 1e-6);
        CHECK_NEAR(l[7000], 0.25, 1e-6);
        CHECK_NEAR(l[7001], 0.75, 1e-6);
        // A lane on a parameter the plugin does not have is inactive.
        s.tracks[0].lanes = {lane(SE_AUTO_PLUGIN, {{0, 0.0f, 0.0f}}, 5, 42)};
        REQUIRE(s.publish(e) == SE_OK);
        CHECK_NEAR(chan(play(e, 300, 0), 0)[200], 0.1, 1e-6); // last value sent before stays
    }
}

TEST_CASE(automation_au_lowpass_cutoff_brightens) {
    if (!kHaveAu) SKIP("Audio Units are macOS-only");
    EngineHandle e(48000, 512);
    const uint64_t a = load_asset(e, {noise(110000)});
    ASession s;
    // AULowpass parameter 0 = cutoff (10 Hz .. Nyquist); 0.004 -> ~200 Hz.
    s.audio(1, {clip(1, a, 0, 0, 110000)}, {lane(SE_AUTO_PLUGIN, {{0, 0.004f, 0.0f}, {96000, 1.0f, 0.8f}}, 9, 0)},
            {au_dev(9, kAuLowpass)});
    REQUIRE(s.publish(e) == SE_OK);
    std::size_t errs = 0;
    se_plugin_load_errors(e, nullptr, 0, &errs);
    REQUIRE(errs == 0);
    const std::vector<float> l = chan(play(e, 96000, 0, 512), 0);
    const double b1 = brightness(l, 2000, 12000), b2 = brightness(l, 60000, 70000), b3 = brightness(l, 86000, 96000);
    std::fprintf(stderr, "AU lowpass sweep brightness %.4f %.4f %.4f\n", b1, b2, b3);
    CHECK(b2 > 2.0 * b1);
    CHECK(b3 > 1.2 * b2);
}

TEST_CASE(automation_plugin_param_queries) {
    EngineHandle e(48000, 256);
    const uint64_t a = dc_asset(e, 1000);
    ASession s;
    std::vector<se_device_desc> devs = {clap_gain(1), vst3_gain(2)};
    if (kHaveAu) devs.push_back(au_dev(3, kAuLowpass));
    s.audio(1, {clip(1, a, 0, 0, 1000)}, {}, devs);
    REQUIRE(s.publish(e) == SE_OK);
    for (uint64_t dev : {1u, 2u}) {
        se_plugin_param_info info[8];
        std::size_t n = 0;
        REQUIRE(se_plugin_params(e, dev, info, 8, &n) == SE_OK);
        REQUIRE(n == 1); // the hidden gesture hooks are not listed
        CHECK_EQ(info[0].id, 0u);
        CHECK(std::string(info[0].name) == "Gain");
        CHECK_NEAR(info[0].default_normalized, 0.5, 1e-6);
        CHECK_EQ(info[0].steps, 0u);
        CHECK_EQ(info[0].automatable, 1);
        float v = 0;
        REQUIRE(se_plugin_param_value(e, dev, 0, &v) == SE_OK);
        CHECK_NEAR(v, 0.5, 1e-6);
        char text[64];
        REQUIRE(se_plugin_param_text(e, dev, 0, 0.25f, text, sizeof text) == SE_OK);
        CHECK(std::string(text) == "0.250");
        CHECK(se_plugin_param_value(e, dev, 99, &v) == SE_ERR_NOT_FOUND);
    }
    std::size_t n = 0;
    CHECK(se_plugin_params(e, 77, nullptr, 0, &n) == SE_ERR_NOT_FOUND);
    if (!kHaveAu) {
        std::fprintf(stderr, "  (Audio Unit half skipped: Audio Units are macOS-only)\n");
        return;
    }
    REQUIRE(se_plugin_params(e, 3, nullptr, 0, &n) == SE_OK);
    REQUIRE(n >= 2);
    std::vector<se_plugin_param_info> au(n);
    REQUIRE(se_plugin_params(e, 3, au.data(), au.size(), &n) == SE_OK);
    CHECK(std::string(au[0].name).find("Cutoff") != std::string::npos);
    CHECK_EQ(au[0].automatable, 1);
    char text[64];
    REQUIRE(se_plugin_param_text(e, 3, au[0].id, 0.5f, text, sizeof text) == SE_OK);
    std::fprintf(stderr, "AU %s at 0.5: '%s'\n", au[0].name, text);
    CHECK(std::string(text).find("Hz") != std::string::npos);
}

// ---------------------------------------------------------------- editor changes

TEST_CASE(automation_take_plugin_param_changes_reports_editor_gestures) {
    EngineHandle e(48000, 256);
    const uint64_t a = dc_asset(e, 20000);
    ASession s;
    std::vector<se_device_desc> devs = {clap_gain(1), vst3_gain(2)};
    if (kHaveAu) devs.push_back(au_dev(3, kAuLowpass));
    s.audio(1, {clip(1, a, 0, 0, 20000)}, {}, devs);
    REQUIRE(s.publish(e) == SE_OK);
    CHECK(take_changes(e).empty());
    const auto check_gesture = [](const std::vector<se_plugin_param_change>& c, uint64_t dev, uint32_t param,
                                  float before, float value, int64_t frame) {
        REQUIRE(c.size() == 3);
        for (const auto& x : c) {
            CHECK_EQ(x.device_id, dev);
            CHECK_EQ(x.param_id, param);
            if (frame >= 0) CHECK_EQ(x.frame, frame);
        }
        CHECK(c[0].gesture_begin == 1 && c[0].gesture_end == 0);
        CHECK(c[1].gesture_begin == 0 && c[1].gesture_end == 0);
        CHECK(c[2].gesture_begin == 0 && c[2].gesture_end == 1);
        if (before >= 0) CHECK_NEAR(c[0].normalized, before, 1e-6);
        CHECK_NEAR(c[1].normalized, value, 1e-6);
        CHECK_NEAR(c[2].normalized, value, 1e-6);
    };

    // VST3: the controller's IComponentHandler begin/perform/end (main thread).
    se_transport_seek(e, 4321);
    REQUIRE(dstudio::testing::plugin_set_parameter(e, 2, 1000, 0.3));
    check_gesture(take_changes(e), 2, 0, 0.5f, 0.3f, 4321);

    // CLAP, from process(): output events, stamped sample-accurately.
    se_transport_seek(e, 1000);
    se_transport_play(e);
    REQUIRE(dstudio::testing::plugin_set_parameter(e, 1, 1000, 0.7));
    render(e, 256, 256);
    check_gesture(take_changes(e), 1, 0, 0.5f, 0.7f, 1000);
    // The gesture changed the gain (the plugin applies it; the VST3 gain
    // after it is at 0.3 from its own gesture).
    CHECK_NEAR(chan(render(e, 256, 256), 0)[100], 0.7 * 0.3, 1e-6);

    // CLAP, outside process(): request_callback -> request_flush -> flush in
    // se_plugin_idle (nothing processes in between).
    se_transport_pause(e);
    REQUIRE(dstudio::testing::plugin_set_parameter(e, 1, 1001, 0.2));
    render(e, 64, 64); // delivers the hook event (paused: chains still run)
    CHECK(take_changes(e).empty());
    REQUIRE(se_plugin_idle(e) == SE_OK);
    check_gesture(take_changes(e), 1, 0, 0.7f, 0.2f, -1);

    // AU: begin gesture, AUParameterSet, end gesture (what AU views do),
    // delivered on a dispatch queue.
    if (kHaveAu) {
        REQUIRE(dstudio::testing::plugin_simulate_gesture(e, 3, 0, 0.5f));
        std::vector<se_plugin_param_change> au;
        for (int i = 0; i < 200 && au.size() < 3; ++i) {
            const auto c = take_changes(e);
            au.insert(au.end(), c.begin(), c.end());
            if (au.size() < 3) std::this_thread::sleep_for(std::chrono::milliseconds(5));
        }
        check_gesture(au, 3, 0, -1.0f, 0.5f, -1);
        float v = 0;
        REQUIRE(se_plugin_param_value(e, 3, 0, &v) == SE_OK);
        CHECK_NEAR(v, 0.5, 1e-4);
    } else {
        std::fprintf(stderr, "  (Audio Unit gesture skipped: Audio Units are macOS-only)\n");
    }

    // Automation itself is not reported.
    ASession s2 = s;
    s2.tracks[0].lanes = {lane(SE_AUTO_PLUGIN, {{0, 0.0f, 0.0f}, {5000, 1.0f, 0.0f}}, 1, 0),
                          lane(SE_AUTO_PLUGIN, {{0, 0.0f, 0.0f}, {5000, 1.0f, 0.0f}}, 2, 0)};
    if (kHaveAu) s2.tracks[0].lanes.push_back(lane(SE_AUTO_PLUGIN, {{0, 0.0f, 0.0f}, {5000, 1.0f, 0.0f}}, 3, 0));
    REQUIRE(s2.publish(e) == SE_OK);
    play(e, 6000);
    std::this_thread::sleep_for(std::chrono::milliseconds(50));
    CHECK(take_changes(e).empty());
}

// ---------------------------------------------------------------- delay compensation

TEST_CASE(automation_stays_aligned_with_audio_under_pdc) {
    constexpr uint32_t N = 1000;
    EngineHandle e(48000, 256);
    const uint64_t a = dc_asset(e, 40000);
    // A vertical step at 10000 (the later point wins at its frame).
    const std::vector<Pt> step = {{0, -60.0f, 0.0f}, {10000, -60.0f, 0.0f}, {10000, 0.0f, 0.0f}};
    ASession s;
    s.audio(1, {clip(1, a, 0, 0, 40000)}, {lane(SE_AUTO_VOLUME, step)}, {clap_latency(7, N)}, -1.0f);
    s.audio(2, {clip(2, a, 0, 0, 40000)}, {lane(SE_AUTO_VOLUME, step)}, {}, 1.0f); // delayed by the compensation
    REQUIRE(s.publish(e) == SE_OK);
    se_pdc_report r{};
    REQUIRE(se_pdc_report_get(e, &r, nullptr, 0, nullptr) == SE_OK);
    REQUIRE(r.total_frames == N);
    const std::vector<float> out = play(e, 14000);
    // Both tracks: the step follows the audio (which is N late), not the transport.
    CHECK_EQ(out[2 * (10000 + N - 1)], 0.0f);
    CHECK_NEAR(out[2 * (10000 + N)], 1.0, 1e-6);
    CHECK_EQ(out[2 * (10000 + N - 1) + 1], 0.0f);
    CHECK_NEAR(out[2 * (10000 + N) + 1], 1.0, 1e-6);

    // Across loop wraps: the fader evaluates at the frame the audio it gets
    // was read at, also for the N samples after each wrap (pre-wrap audio).
    REQUIRE(se_transport_set_loop(e, 1, 8000, 12000) == SE_OK);
    const std::vector<float> lo = chan(play(e, 20000, 0, 300), 0);
    std::vector<int64_t> pos(lo.size());
    int64_t p = 0;
    for (std::size_t i = 0; i < pos.size(); ++i) {
        pos[i] = p;
        if (++p == 12000) p = 8000;
    }
    std::size_t bad = 0;
    for (std::size_t i = N; i < lo.size(); ++i) {
        const int64_t src = pos[i - N];
        const float want = src >= 10000 ? 1.0f : 0.0f;
        if (std::fabs(lo[i] - want) > 1e-6f) ++bad;
    }
    std::fprintf(stderr, "pdc loop: %zu samples off\n", bad);
    CHECK_EQ(bad, std::size_t{0});
    REQUIRE(se_transport_set_loop(e, 0, 0, 0) == SE_OK);

    // Off: no compensation of anything; lanes run on the transport frame.
    REQUIRE(se_pdc_set_enabled(e, 0) == SE_OK);
    const std::vector<float> off = play(e, 12000);
    CHECK_EQ(off[2 * 9999 + 1], 0.0f);
    CHECK_NEAR(off[2 * 10000 + 1], 1.0, 1e-6);

    // An insert after a latent one evaluates its lanes at the frame its input
    // audio was read at (500 earlier): the gain step meets the audio.
    REQUIRE(se_pdc_set_enabled(e, 1) == SE_OK);
    ASession c;
    c.audio(1, {clip(1, a, 0, 0, 40000)},
            {lane(SE_AUTO_PLUGIN, {{0, 0.0f, 0.0f}, {10000, 0.0f, 0.0f}, {10000, 1.0f, 0.0f}}, 21, 0)},
            {clap_latency(20, 500), clap_gain(21)});
    REQUIRE(c.publish(e) == SE_OK);
    const std::vector<float> ch = chan(play(e, 12000), 0);
    CHECK_EQ(ch[10499], 0.0f);
    CHECK_NEAR(ch[10500], 1.0, 1e-6);
}

// ---------------------------------------------------------------- export

TEST_CASE(automation_export_renders_lanes) {
    EngineHandle e(48000, 256);
    const uint64_t a = dc_asset(e, 30000);
    const std::vector<Pt> pts = {{0, -30.0f, 0.4f}, {20000, 0.0f, 0.0f}};
    const std::vector<Pt> mv = {{0, 0.0f, 0.0f}, {15000, 0.0f, 0.0f}, {15000, -6.0f, 0.0f}};
    ASession s;
    s.audio(1, {clip(1, a, 0, 0, 30000)}, {lane(SE_AUTO_VOLUME, pts)}, {clap_latency(4, 300)});
    s.master = {lane(SE_AUTO_VOLUME, mv)};
    REQUIRE(s.publish(e) == SE_OK);
    TempFile f("auto_export.wav");
    REQUIRE(se_export_wav(e, f.path.c_str(), 1000, 25000) == SE_OK);
    dstudio::DecodedAudio d;
    std::string err;
    REQUIRE(dstudio::decode_wav_file(f.path.c_str(), d, err) == SE_OK);
    REQUIRE(d.frames == 24000);
    double max_err = 0;
    for (std::size_t i = 0; i < d.frames; ++i) {
        const auto t = static_cast<int64_t>(i) + 1000;
        max_err = std::max(max_err, std::fabs(d.planar[0][i] - db_gain(ref_value(pts, t)) * db_gain(ref_value(mv, t))));
    }
    std::fprintf(stderr, "export: max error %.3g\n", max_err);
    CHECK(max_err <= 1e-4);
}

// ---------------------------------------------------------------- RT safety

TEST_CASE(automation_dense_lanes_do_not_allocate) {
    EngineHandle e(48000, 256);
    const uint64_t a = load_asset(e, {sine(30000, 220.0, 48000.0, 0.3)});
    // Dense lanes: a point every 7 frames with alternating curves.
    const auto dense = [](float lo, float hi, int64_t len) {
        std::vector<Pt> p;
        for (int64_t t = 0; t < len; t += 7)
            p.push_back({t, (t / 7) % 2 == 0 ? lo : hi, static_cast<float>((t / 7) % 5) * 0.4f - 0.8f});
        return p;
    };
    ASession s;
    for (uint64_t t = 1; t <= 24; ++t) {
        std::vector<se_device_desc> dev;
        std::vector<Lane> lanes = {lane(SE_AUTO_VOLUME, dense(-30.0f, 0.0f, 30000)),
                                   lane(SE_AUTO_PAN, dense(-1.0f, 1.0f, 30000))};
        if (t % 3 == 0) {
            dev = {t % 2 == 0 ? clap_gain(100 + t) : vst3_gain(100 + t)};
            lanes.push_back(lane(SE_AUTO_PLUGIN, dense(0.0f, 1.0f, 30000), 100 + t, 0));
        }
        s.audio(t, {clip(t, a, static_cast<int64_t>(t) * 13, 0, 30000)}, std::move(lanes), std::move(dev), 0.0f);
    }
    ASession::Track& syn = s.audio(50, {}, {lane(SE_AUTO_SYNTH, dense(200.0f, 8000.0f, 30000), 0, 0,
                                                 SE_SYNTH_P_FILTER_CUTOFF, true),
                                            lane(SE_AUTO_SYNTH, dense(0.0f, 1.0f, 30000), 0, 0, SE_SYNTH_P_OSC1_SHAPE)});
    syn.d.instrument = 1;
    for (int k = 0; k < 40; ++k)
        syn.notes.push_back(se_note_desc{k * 700, k * 700 + 2000, static_cast<uint8_t>(48 + k % 12), 90, {}});
    s.audio(51, {clip(51, a, 0, 0, 30000)}, {lane(SE_AUTO_VOLUME, dense(-10.0f, 0.0f, 30000))},
            {clap_latency(300, 700)});
    s.master = {lane(SE_AUTO_VOLUME, dense(-6.0f, 0.0f, 30000))};
    REQUIRE(s.publish(e) == SE_OK);
    REQUIRE(se_transport_set_loop(e, 1, 3000, 9000) == SE_OK);

    std::vector<float> buf(2 * 8192);
    REQUIRE(se_transport_play(e) == SE_OK);
    REQUIRE(se_render_offline(e, buf.data(), 256) == SE_OK); // warm-up
    const long allocs = rt_alloc_count(), frees = rt_free_count();
    const uint32_t sizes[] = {256, 1, 333, 8192, 64, 255, 77};
    for (int i = 0; i < 400; ++i) {
        if (i % 50 == 25) se_transport_seek(e, 1000 + i * 37);
        if (i % 90 == 60) se_transport_pause(e);
        if (i % 90 == 65) se_transport_play(e);
        RtAllocGuard g;
        if (se_render_offline(e, buf.data(), sizes[i % 7]) != SE_OK) {
            CHECK(false);
            break;
        }
    }
    CHECK_EQ(rt_alloc_count() - allocs, 0);
    CHECK_EQ(rt_free_count() - frees, 0);
}

// ---------------------------------------------------------------- switches (mute, device power)

TEST_CASE(automation_stepped_lanes_hold_each_value) {
    Lane l = lane(SE_AUTO_DEVICE_ON, {{0, 0.0f, 0.5f}, {100, 1.0f, 0.0f}, {200, 0.0f, 0.0f}}, 5);
    l.d.flags = SE_AUTO_FLAG_STEPPED;
    l.d.points = l.points.data();
    l.d.point_count = 3;
    const int64_t frames[] = {-5, 50, 99, 100, 150, 199, 200, 900};
    float out[8] = {};
    REQUIRE(se_automation_eval(&l.d, frames, 8, out) == SE_OK);
    const float want[] = {0, 0, 0, 1, 1, 1, 0, 0}; // no ramp, no curve
    for (int i = 0; i < 8; ++i) CHECK_EQ(out[i], want[i]);
    l.d.flags = 2; // unknown flag
    CHECK(se_automation_eval(&l.d, frames, 1, out) == SE_ERR_INVALID_ARG);
    l.d.flags = 0;
    l.d.target = SE_AUTO_DEVICE_ON + 1;
    CHECK(se_automation_eval(&l.d, frames, 1, out) == SE_ERR_INVALID_ARG);
}

TEST_CASE(automation_mute_lane_replaces_the_static_mute) {
    EngineHandle e(48000, 256);
    const uint64_t a = dc_asset(e, 4000);
    ASession s;
    Lane m = lane(SE_AUTO_MUTE, {{0, 0.0f, 0.0f}, {1000, 1.0f, 0.0f}, {2000, 0.0f, 0.0f}});
    m.d.flags = SE_AUTO_FLAG_STEPPED;
    s.audio(1, {clip(1, a, 0, 0, 4000)}, {m});
    s.tracks[0].d.mute = 1; // the lane replaces it
    REQUIRE(s.publish(e) == SE_OK);
    const std::vector<float> l = chan(play(e, 3000, 0, 256), 0);
    CHECK_NEAR(l[999], 1.0, 1e-6);
    CHECK_EQ(l[1000], 0.0f); // muted from its point, mid-block
    CHECK_EQ(l[1999], 0.0f);
    CHECK_NEAR(l[2000], 1.0, 1e-6);
    // Without the lane the static mute is back.
    s.tracks[0].lanes.clear();
    REQUIRE(s.publish(e) == SE_OK);
    CHECK_EQ(chan(play(e, 300, 0), 0)[200], 0.0f);
}

TEST_CASE(automation_device_on_lane_switches_an_insert_per_block) {
    EngineHandle e(48000, 256);
    const uint64_t a = dc_asset(e, 4000);
    ASession s;
    Lane on = lane(SE_AUTO_DEVICE_ON, {{0, 1.0f, 0.0f}, {1024, 0.0f, 0.0f}, {2100, 1.0f, 0.0f}}, 5);
    on.d.flags = SE_AUTO_FLAG_STEPPED;
    se_device_desc dev = clap_gain(5);
    dev.enabled = 0; // the lane replaces the static switch
    s.audio(1, {clip(1, a, 0, 0, 4000)}, {lane(SE_AUTO_PLUGIN, {{0, 0.25f, 0.0f}}, 5, 0), on}, {dev});
    REQUIRE(s.publish(e) == SE_OK);
    const std::vector<float> l = chan(play(e, 3000, 0, 256), 0);
    CHECK_NEAR(l[500], 0.25, 1e-6);  // on: the gain plugin runs
    CHECK_NEAR(l[1023], 0.25, 1e-6);
    CHECK_NEAR(l[1024], 1.0, 1e-6);  // off from the block that starts on the point: bypassed
    CHECK_NEAR(l[2303], 1.0, 1e-6);  // the switch back waits for the next block boundary (2304)
    CHECK_NEAR(l[2304], 0.25, 1e-6);
}
