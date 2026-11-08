// Plugin delay compensation (docs/adr/0004-delay-compensation.md): track
// alignment, bypass, chains, metronome, export, monitoring, recording,
// runtime latency changes, seek/stop/pause/loop semantics, the cap, RT
// safety, and bit-identity with the pre-PDC engine when switched off.
//
// Uses SaineTestLatency (.clap / .vst3): delays by exactly N frames and
// reports N. Tracks are panned hard left so the left channel carries the
// track signal unscaled.
#include "pdc_golden_scenario.hpp"
#include "test_harness.hpp"
#include "test_hooks.hpp"
#include "test_util.hpp"
#include "platform/os.hpp"
#include "wav.hpp"

#include <algorithm>
#include <cmath>
#include <cstring>
#include <filesystem>
#include <string>
#include <vector>

using namespace testutil;

namespace {

const std::string kPluginDir = SAINE_TEST_PLUGIN_DIR;
const std::string kClapLatency = kPluginDir + "/SaineTestLatency.clap";
const std::string kVst3Latency = kPluginDir + "/SaineTestLatency.vst3";
const std::string kClapSine = kPluginDir + "/SaineTestSine.clap";
const char* const kVst3LatencyUid = "5A17E5A16E0000038A1E7E576A170003";
constexpr double kVst3MaxLatency = 1048576.0; // normalized parameter = N / 2^20

// Owned state blobs for se_device_desc (pointers must stay valid).
struct StateStore {
    std::vector<std::vector<uint8_t>> blobs;
    const std::vector<uint8_t>& clap_n(uint32_t n) {
        blobs.push_back({static_cast<uint8_t>(n), static_cast<uint8_t>(n >> 8), static_cast<uint8_t>(n >> 16),
                         static_cast<uint8_t>(n >> 24)});
        return blobs.back();
    }
    const std::vector<uint8_t>& vst3_n(uint32_t n) {
        std::vector<uint8_t> v = {'S', 'V', 'S', '3', 1, 0, 0, 0, 4, 0, 0, 0, 0, 0, 0, 0};
        for (int i = 0; i < 4; ++i) v.push_back(static_cast<uint8_t>(n >> (8 * i)));
        for (int i = 0; i < 8; ++i) v.push_back(0);
        blobs.push_back(std::move(v));
        return blobs.back();
    }
};
StateStore g_states; // lives for the whole test binary

se_device_desc clap_latency(uint64_t id, uint32_t n, bool enabled = true) {
    const auto& st = g_states.clap_n(n);
    se_device_desc d{};
    d.device_id = id;
    d.name = "Latency";
    d.plugin_id = "studio.saine.test.latency";
    d.path = kClapLatency.c_str();
    d.format = SE_PLUGIN_CLAP;
    d.enabled = enabled ? 1 : 0;
    d.state = st.data();
    d.state_len = st.size();
    return d;
}

se_device_desc vst3_latency(uint64_t id, uint32_t n, bool enabled = true) {
    const auto& st = g_states.vst3_n(n);
    se_device_desc d{};
    d.device_id = id;
    d.name = "Latency";
    d.plugin_id = kVst3LatencyUid;
    d.path = kVst3Latency.c_str();
    d.format = SE_PLUGIN_VST3;
    d.enabled = enabled ? 1 : 0;
    d.state = st.data();
    d.state_len = st.size();
    return d;
}

struct PSession {
    struct Track {
        se_track_desc_v5 d{};
        std::vector<se_clip_desc> clips;
        std::vector<se_note_desc> notes;
        std::vector<se_device_desc> devices;
        se_device_desc instrument{};
        bool has_instrument = false;
    };
    std::vector<Track> tracks;

    Track& audio(uint64_t id, std::vector<se_clip_desc> clips, std::vector<se_device_desc> devices = {}) {
        Track t;
        t.d.track_id = id;
        t.d.gain = 1.0f;
        t.d.pan = -1.0f; // hard left: L = track signal
        t.clips = std::move(clips);
        t.devices = std::move(devices);
        tracks.push_back(std::move(t));
        return tracks.back();
    }
    int32_t publish(se_engine* e) {
        std::vector<se_track_desc_v5> v;
        for (Track& t : tracks) {
            se_track_desc_v5 d = t.d;
            d.clips = t.clips.data();
            d.clip_count = static_cast<uint32_t>(t.clips.size());
            d.notes = t.notes.data();
            d.note_count = static_cast<uint32_t>(t.notes.size());
            d.devices = t.devices.data();
            d.device_count = static_cast<uint32_t>(t.devices.size());
            d.instrument_plugin = t.has_instrument ? &t.instrument : nullptr;
            v.push_back(d);
        }
        se_session_desc_v5 s{v.data(), static_cast<uint32_t>(v.size()), 1.0f};
        return se_session_publish_v5(e, &s);
    }
};

std::vector<float> left(const std::vector<float>& inter) {
    std::vector<float> l(inter.size() / 2);
    for (std::size_t i = 0; i < l.size(); ++i) l[i] = inter[2 * i];
    return l;
}

std::vector<float> play_left(se_engine* e, uint32_t frames, int64_t from = 0, uint32_t block = 256) {
    se_transport_seek(e, from);
    se_transport_play(e);
    return left(render(e, frames, block));
}

// Indices of the non-zero samples.
std::vector<std::size_t> nonzero(const std::vector<float>& v) {
    std::vector<std::size_t> r;
    for (std::size_t i = 0; i < v.size(); ++i)
        if (v[i] != 0.0f) r.push_back(i);
    return r;
}

std::vector<float> impulse(std::size_t n, std::size_t at, float v = 0.5f) {
    std::vector<float> x(n, 0.0f);
    x[at] = v;
    return x;
}

se_pdc_report report(se_engine* e, std::vector<se_pdc_track>* tracks = nullptr) {
    se_pdc_report r{};
    std::vector<se_pdc_track> t(64);
    std::size_t n = 0;
    REQUIRE(se_pdc_report_get(e, &r, t.data(), t.size(), &n) == SE_OK);
    t.resize(n);
    if (tracks != nullptr) *tracks = t;
    return r;
}

std::size_t error_count(se_engine* e) {
    std::size_t n = 0;
    se_plugin_load_errors(e, nullptr, 0, &n);
    return n;
}

// Renders `frames` with injected mono input f(k) (k = frames since `first`).
std::vector<float> render_with_input(se_engine* e, uint32_t frames, std::size_t first,
                                     const std::vector<float>& input, uint32_t block = 256) {
    std::vector<float> in(frames, 0.0f), out(static_cast<std::size_t>(frames) * 2);
    for (uint32_t k = 0; k < frames; ++k)
        if (first + k < input.size()) in[k] = input[first + k];
    for (uint32_t done = 0; done < frames;) {
        const uint32_t n = std::min(block, frames - done);
        REQUIRE(se_render_offline_with_input(e, in.data() + done, 1, out.data() + static_cast<std::size_t>(done) * 2,
                                             n) == SE_OK);
        done += n;
    }
    return out;
}

} // namespace

// ---------------------------------------------------------------- bit identity

TEST_CASE(pdc_off_is_bit_identical_to_the_pre_pdc_engine) {
    // Hashes of pdc_golden::run() against the library of commit bf473ce
    // (before delay compensation existed), identical in Release and
    // ASan/Debug builds.
    constexpr uint64_t kGoldenNoLatency = 0x2f4322e480930045ull;
    constexpr uint64_t kGoldenWithLatency = 0xbf8049e94649e878ull;
    const auto off = [](se_engine* e) { REQUIRE(se_pdc_set_enabled(e, 0) == SE_OK); };
    const auto on = [](se_engine* e) { CHECK(se_pdc_is_enabled(e) == 1); };
    CHECK_EQ(pdc_golden::run(kPluginDir, false, off), kGoldenNoLatency);
    CHECK_EQ(pdc_golden::run(kPluginDir, true, off), kGoldenWithLatency);
    // PDC on with L = 0 changes nothing either.
    CHECK_EQ(pdc_golden::run(kPluginDir, false, on), kGoldenNoLatency);
    // ... and with latency it does change the output (sanity).
    CHECK(pdc_golden::run(kPluginDir, true, on) != kGoldenWithLatency);
}

// ---------------------------------------------------------------- alignment

TEST_CASE(pdc_aligns_two_tracks_and_reports_audible_position) {
    EngineHandle e(48000, 256);
    const uint64_t a = load_asset(e, {impulse(8000, 0)});
    PSession s;
    s.audio(1, {clip(1, a, 5000, 0, 8000)}, {clap_latency(10, 1000)});
    s.audio(2, {clip(2, a, 5000, 0, 8000)});
    REQUIRE(s.publish(e) == SE_OK);
    REQUIRE(error_count(e) == 0);
    uint32_t lat = 0;
    CHECK(se_plugin_latency(e, 10, &lat) == SE_OK);
    CHECK_EQ(lat, 1000u);

    std::vector<se_pdc_track> tr;
    se_pdc_report r = report(e, &tr);
    CHECK_EQ(r.enabled, uint8_t{1});
    CHECK_EQ(r.total_frames, 1000u);
    CHECK_EQ(r.uncapped_frames, 1000u);
    CHECK_EQ(r.clamped, uint8_t{0});
    REQUIRE(tr.size() == 2);
    CHECK(tr[0].track_id == 1 && tr[0].latency_frames == 1000 && tr[0].delay_frames == 0);
    CHECK(tr[1].track_id == 2 && tr[1].latency_frames == 0 && tr[1].delay_frames == 1000);

    // One impulse of double amplitude, shifted by L.
    const std::vector<float> on = play_left(e, 9000);
    const auto nz = nonzero(on);
    REQUIRE(nz.size() == 1);
    CHECK_EQ(nz[0], std::size_t{6000});
    CHECK_EQ(on[6000], 1.0f);
    // Audible position: transport minus L while playing (offline: no device latency).
    se_transport_info ti{};
    se_transport_get(e, &ti);
    int64_t audible = 0;
    REQUIRE(se_transport_audible_position(e, &audible) == SE_OK);
    CHECK_EQ(ti.position, int64_t{9000});
    CHECK_EQ(audible, int64_t{8000});
    se_transport_pause(e);
    se_transport_audible_position(e, &audible);
    CHECK_EQ(audible, int64_t{9000});

    // Off: two impulses 1000 apart, exactly what the engine did before PDC.
    REQUIRE(se_pdc_set_enabled(e, 0) == SE_OK);
    CHECK(se_pdc_is_enabled(e) == 0);
    r = report(e, &tr);
    CHECK_EQ(r.total_frames, 0u);
    CHECK_EQ(r.uncapped_frames, 1000u);
    CHECK(tr[0].latency_frames == 1000 && tr[1].delay_frames == 0);
    const std::vector<float> off = play_left(e, 9000);
    CHECK(nonzero(off) == (std::vector<std::size_t>{5000, 6000}));
    CHECK_EQ(off[5000], 0.5f);
    CHECK_EQ(off[6000], 0.5f);
    se_transport_audible_position(e, &audible);
    CHECK_EQ(audible, int64_t{9000});

    // Back on.
    REQUIRE(se_pdc_set_enabled(e, 1) == SE_OK);
    CHECK(nonzero(play_left(e, 9000)) == std::vector<std::size_t>{6000});
}

TEST_CASE(pdc_bypass_keeps_timing_and_chains_align) {
    EngineHandle e(48000, 256);
    const uint64_t a = load_asset(e, {impulse(4000, 0)});
    PSession s;
    s.audio(1, {clip(1, a, 3000, 0, 4000)}, {clap_latency(10, 1000)});
    s.audio(2, {clip(2, a, 3000, 0, 4000)});
    REQUIRE(s.publish(e) == SE_OK);
    CHECK(nonzero(play_left(e, 6000)) == std::vector<std::size_t>{4000});
    // Bypassed: the slot delays by the plugin's latency itself.
    s.tracks[0].devices[0].enabled = 0;
    REQUIRE(s.publish(e) == SE_OK);
    CHECK_EQ(report(e).total_frames, 1000u);
    std::vector<float> out = play_left(e, 6000);
    CHECK(nonzero(out) == std::vector<std::size_t>{4000});
    CHECK_EQ(out[4000], 1.0f);
    // Bypass toggled mid-play: the impulse after the toggle still lands at t + L.
    s.tracks[0].devices[0].enabled = 1;
    REQUIRE(s.publish(e) == SE_OK);
    se_transport_seek(e, 0);
    se_transport_play(e);
    std::vector<float> part1 = left(render(e, 2000, 256));
    s.tracks[0].devices[0].enabled = 0;
    REQUIRE(s.publish(e) == SE_OK);
    std::vector<float> part2 = left(render(e, 4000, 256));
    part1.insert(part1.end(), part2.begin(), part2.end());
    CHECK(nonzero(part1) == std::vector<std::size_t>{4000});

    // Three tracks with different chains (CLAP 1000 + VST3 300, CLAP 500
    // bypassed, nothing) plus a muted track with the largest latency:
    // muted tracks count, so L = 2000 and everything audible lands at t + 2000.
    PSession s3;
    s3.audio(1, {clip(1, a, 3000, 0, 4000)}, {clap_latency(20, 1000), vst3_latency(21, 300)});
    s3.audio(2, {clip(2, a, 3000, 0, 4000)}, {clap_latency(22, 500, false)});
    s3.audio(3, {clip(3, a, 3000, 0, 4000)});
    s3.audio(4, {clip(4, a, 3000, 0, 4000)}, {clap_latency(23, 2000)}).d.mute = 1;
    REQUIRE(s3.publish(e) == SE_OK);
    REQUIRE(error_count(e) == 0);
    std::vector<se_pdc_track> tr;
    CHECK_EQ(report(e, &tr).total_frames, 2000u);
    REQUIRE(tr.size() == 4);
    CHECK(tr[0].latency_frames == 1300 && tr[1].latency_frames == 500 && tr[2].latency_frames == 0 &&
          tr[3].latency_frames == 2000);
    CHECK(tr[0].delay_frames == 700 && tr[1].delay_frames == 1500 && tr[2].delay_frames == 2000);
    out = play_left(e, 8000);
    CHECK(nonzero(out) == std::vector<std::size_t>{5000});
    CHECK_EQ(out[5000], 1.5f);
    // Unmuting the latent track does not move anything.
    s3.tracks[3].d.mute = 0;
    REQUIRE(s3.publish(e) == SE_OK);
    out = play_left(e, 8000);
    CHECK(nonzero(out) == std::vector<std::size_t>{5000});
    CHECK_EQ(out[5000], 2.0f);
}

TEST_CASE(pdc_instrument_plugin_track_aligns_with_audio) {
    // SaineTestSine onsets exactly on the note frame (cosine at full
    // amplitude); with an insert of latency 200 on it and another track at
    // 1000, the onset lands at t + 1000 like the audio impulse.
    EngineHandle e(48000, 256);
    const uint64_t a = load_asset(e, {impulse(4000, 0)});
    PSession s;
    s.audio(1, {clip(1, a, 3000, 0, 4000)}, {clap_latency(10, 1000)});
    PSession::Track& inst = s.audio(2, {}, {clap_latency(11, 200)});
    inst.d.instrument = 1;
    inst.notes = {se_note_desc{3000, 3500, 69, 127, {}}};
    inst.instrument.device_id = 12;
    inst.instrument.name = "Sine";
    inst.instrument.plugin_id = "studio.saine.test.sine";
    inst.instrument.path = kClapSine.c_str();
    inst.instrument.format = SE_PLUGIN_CLAP;
    inst.instrument.enabled = 1;
    inst.has_instrument = true;
    REQUIRE(s.publish(e) == SE_OK);
    REQUIRE(error_count(e) == 0);
    const std::vector<float> out = play_left(e, 6000);
    // Before t + L: silence; at t + L: impulse 0.5 + sine 0.25 (velocity 127).
    for (std::size_t i = 0; i < 4000; ++i)
        if (out[i] != 0.0f) {
            CHECK(false);
            break;
        }
    CHECK_NEAR(out[4000], 0.5 + 0.25, 1e-6);
    CHECK(out[4001] != 0.0f);
}

TEST_CASE(pdc_clips_on_a_latent_instrument_track_line_up_with_its_output) {
    // SaineTestLatency used as the instrument plugin: silent (it delays its
    // empty input) but reports latency 1000, so the clips on that track are
    // delayed by 1000 to line up with the instrument's output, and the
    // other track by L - 0 = 1000.
    EngineHandle e(48000, 256);
    const uint64_t a = load_asset(e, {impulse(4000, 0)});
    PSession s;
    PSession::Track& inst = s.audio(1, {clip(1, a, 3000, 0, 4000)});
    inst.d.instrument = 1;
    inst.instrument = clap_latency(10, 1000);
    inst.has_instrument = true;
    s.audio(2, {clip(2, a, 3000, 0, 4000)});
    REQUIRE(s.publish(e) == SE_OK);
    REQUIRE(error_count(e) == 0);
    std::vector<se_pdc_track> tr;
    CHECK_EQ(report(e, &tr).total_frames, 1000u);
    CHECK(tr[0].latency_frames == 1000 && tr[0].delay_frames == 0 && tr[1].delay_frames == 1000);
    std::vector<float> out = play_left(e, 6000);
    CHECK(nonzero(out) == std::vector<std::size_t>{4000});
    CHECK_EQ(out[4000], 1.0f);
    // Bypassed instrument plugin: still counts (silent), same timing.
    s.tracks[0].instrument.enabled = 0;
    REQUIRE(s.publish(e) == SE_OK);
    out = play_left(e, 6000);
    CHECK(nonzero(out) == std::vector<std::size_t>{4000});
    // Off: no delays at all.
    REQUIRE(se_pdc_set_enabled(e, 0) == SE_OK);
    out = play_left(e, 6000);
    CHECK(nonzero(out) == std::vector<std::size_t>{3000});
}

// ---------------------------------------------------------------- metronome

TEST_CASE(pdc_metronome_is_delayed_by_L) {
    const auto clicks = [](bool pdc) {
        EngineHandle e(48000, 256);
        REQUIRE(se_pdc_set_enabled(e, pdc ? 1 : 0) == SE_OK);
        PSession s;
        s.audio(1, {}, {clap_latency(10, 1000)});
        REQUIRE(s.publish(e) == SE_OK);
        se_metronome_desc m{};
        m.enabled = 1;
        m.gain = 0.5f;
        m.frames_per_beat = 6000.0;
        m.beats_per_bar = 4;
        REQUIRE(se_metronome_set(e, &m) == SE_OK);
        return play_left(e, 20000, 0, 333);
    };
    const std::vector<float> off = clicks(false);
    const std::vector<float> on = clicks(true);
    CHECK(off[0] != 0.0f); // beat 0 sounds on frame 0
    for (std::size_t i = 0; i < 1000; ++i)
        if (on[i] != 0.0f) {
            CHECK(false);
            break;
        }
    long bad = 0;
    for (std::size_t i = 1000; i < on.size(); ++i)
        if (on[i] != off[i - 1000]) ++bad;
    CHECK_EQ(bad, 0L);
    CHECK(on[1000] != 0.0f && on[7000] != 0.0f);
}

// ---------------------------------------------------------------- export

TEST_CASE(pdc_export_is_timeline_aligned) {
    for (const bool pdc : {true, false}) {
        EngineHandle e(48000, 256);
        REQUIRE(se_pdc_set_enabled(e, pdc ? 1 : 0) == SE_OK);
        const uint64_t a = load_asset(e, {impulse(4000, 0)});
        PSession s;
        s.audio(1, {clip(1, a, 3000, 0, 4000)}, {clap_latency(10, 1000)});
        s.audio(2, {clip(2, a, 3000, 0, 4000)});
        REQUIRE(s.publish(e) == SE_OK);
        TempFile f("pdc_export.wav");
        REQUIRE(se_export_wav(e, f.path.c_str(), 1000, 7000) == SE_OK);
        saine::DecodedAudio d;
        std::string err;
        REQUIRE(saine::decode_wav_file(f.path.c_str(), d, err) == SE_OK);
        CHECK_EQ(d.frames, uint64_t{6000});
        const auto nz = nonzero(d.planar[0]);
        if (pdc) {
            // Timeline frame 3000 -> file frame 2000, both tracks together.
            CHECK(nz == std::vector<std::size_t>{2000});
            CHECK_EQ(d.planar[0][2000], 1.0f);
        } else {
            CHECK(nz == (std::vector<std::size_t>{2000, 3000}));
        }
    }
}

// ---------------------------------------------------------------- monitoring & recording

TEST_CASE(pdc_monitored_input_is_not_delayed_but_its_playback_is) {
    EngineHandle e(48000, 256);
    const uint64_t a = load_asset(e, {impulse(4000, 0, 0.25f)});
    PSession s;
    PSession::Track& mon = s.audio(1, {clip(1, a, 4000, 0, 4000)});
    mon.d.monitor = 1;
    mon.d.input_mode = SE_INPUT_MONO;
    mon.d.input_channel = 0;
    PSession::Track& lat = s.audio(2, {}, {clap_latency(10, 1000)});
    lat.d.pan = 1.0f; // right: keeps the left channel for the monitored track
    REQUIRE(s.publish(e) == SE_OK);
    std::vector<se_pdc_track> tr;
    CHECK_EQ(report(e, &tr).total_frames, 1000u);
    CHECK_EQ(tr[0].delay_frames, 1000u);

    const std::vector<float> input = impulse(8000, 2000, 0.5f);
    se_transport_seek(e, 0);
    se_transport_play(e);
    const std::vector<float> out = left(render_with_input(e, 8000, 0, input, 100));
    // Input impulse at 2000 (low-latency path), clip impulse at 4000 + L.
    CHECK(nonzero(out) == (std::vector<std::size_t>{2000, 5000}));
    CHECK_EQ(out[2000], 0.5f);
    CHECK_EQ(out[5000], 0.25f);

    // Also while stopped (monitoring keeps working, nothing pending).
    se_transport_stop(e);
    const std::vector<float> stopped = left(render_with_input(e, 4000, 0, input));
    CHECK(nonzero(stopped) == std::vector<std::size_t>{2000});
}

TEST_CASE(pdc_recorded_take_is_placed_L_earlier) {
    for (const bool pdc : {true, false}) {
        EngineHandle e(48000, 256);
        REQUIRE(se_pdc_set_enabled(e, pdc ? 1 : 0) == SE_OK);
        const uint64_t a = load_asset(e, {impulse(4000, 0)});
        PSession s;
        PSession::Track& rec = s.audio(1, {});
        rec.d.arm = 1;
        rec.d.input_mode = SE_INPUT_MONO;
        rec.d.input_channel = 0;
        s.audio(2, {clip(1, a, 4000, 0, 4000)}, {clap_latency(10, 1000)});
        REQUIRE(s.publish(e) == SE_OK);
        const std::string dir = (std::filesystem::temp_directory_path() /
                                 ("saine_pdc_rec_" + std::to_string(saine::os::process_id()) + (pdc ? "_on" : "_off")))
                                    .string();
        std::filesystem::remove_all(dir);
        se_transport_seek(e, 0);
        REQUIRE(se_record_start(e, dir.c_str()) == SE_OK);
        // The performer hears the clip impulse at render frame 4000 + L
        // (= 5000 either way: the plugin delays the track by 1000) and plays
        // an input impulse "in time" with it.
        const std::vector<float> out = left(render_with_input(e, 8000, 0, impulse(8000, 5000, 0.5f)));
        CHECK(nonzero(out) == std::vector<std::size_t>{5000});
        std::size_t takes = 0;
        REQUIRE(se_record_stop(e, &takes) == SE_OK);
        REQUIRE(takes == 1);
        se_recorded_take t{};
        REQUIRE(se_record_get_take(e, 0, &t) == SE_OK);
        CHECK_EQ(t.start_frame, pdc ? int64_t{-1000} : int64_t{0});
        saine::DecodedAudio d;
        std::string err;
        REQUIRE(saine::decode_wav_file(t.path, d, err) == SE_OK);
        const auto nz = nonzero(d.planar[0]);
        REQUIRE(nz.size() == 1);
        // With PDC the input lands on the clip impulse's timeline frame.
        const int64_t timeline = t.start_frame + static_cast<int64_t>(nz[0]);
        CHECK_EQ(timeline, pdc ? int64_t{4000} : int64_t{5000});
        std::filesystem::remove_all(dir);
    }
}

TEST_CASE(pdc_change_of_L_while_recording_starts_a_new_take) {
    EngineHandle e(48000, 256);
    PSession s;
    PSession::Track& rec = s.audio(1, {});
    rec.d.arm = 1;
    rec.d.input_mode = SE_INPUT_MONO;
    s.audio(2, {}, {clap_latency(10, 1000)});
    REQUIRE(s.publish(e) == SE_OK);
    const std::string dir =
        (std::filesystem::temp_directory_path() / ("saine_pdc_rec_change_" + std::to_string(saine::os::process_id()))).string();
    std::filesystem::remove_all(dir);
    se_transport_seek(e, 0);
    REQUIRE(se_record_start(e, dir.c_str()) == SE_OK);
    const std::vector<float> input(10000, 0.1f);
    render_with_input(e, 4000, 0, input);
    REQUIRE(saine::testing::plugin_set_parameter(e, 10, 0, 2000.0));
    render_with_input(e, 256, 4000, input); // delivers the parameter; the plugin asks for a restart
    REQUIRE(se_plugin_idle(e) == SE_OK);    // L: 1000 -> 2000
    CHECK_EQ(report(e).total_frames, 2000u);
    render_with_input(e, 2000, 4256, input);
    std::size_t takes = 0;
    REQUIRE(se_record_stop(e, &takes) == SE_OK);
    REQUIRE(takes == 2);
    se_recorded_take t0{}, t1{};
    se_record_get_take(e, 0, &t0);
    se_record_get_take(e, 1, &t1);
    CHECK_EQ(t0.start_frame, int64_t{-1000});
    CHECK_EQ(t0.frames, uint64_t{4256});
    CHECK_EQ(t1.start_frame, int64_t{4256 - 2000});
    CHECK_EQ(t1.frames, uint64_t{2000});
    std::filesystem::remove_all(dir);
}

// ---------------------------------------------------------------- dynamics

TEST_CASE(pdc_latency_change_at_runtime_resizes_and_realigns) {
    EngineHandle e(48000, 256);
    std::vector<float> train(60000, 0.0f);
    for (std::size_t i = 0; i < train.size(); i += 6000) train[i] = 0.5f;
    const uint64_t a = load_asset(e, {train});
    PSession s;
    s.audio(1, {clip(1, a, 0, 0, 60000)}, {clap_latency(10, 1000)});
    s.audio(2, {clip(2, a, 0, 0, 60000)}, {vst3_latency(11, 0)});
    s.audio(3, {clip(3, a, 0, 0, 60000)});
    REQUIRE(s.publish(e) == SE_OK);
    REQUIRE(error_count(e) == 0);
    CHECK_EQ(report(e).total_frames, 1000u);
    se_transport_seek(e, 0);
    se_transport_play(e);
    std::vector<float> out = left(render(e, 12000, 256));
    CHECK(nonzero(out) == (std::vector<std::size_t>{1000, 7000}));
    CHECK_EQ(out[7000], 1.5f);

    // CLAP: the parameter reaches the plugin at the next block, which asks
    // for a restart; se_plugin_idle restarts it and re-plans (lines grow
    // from 2048 to 4096 frames).
    REQUIRE(saine::testing::plugin_set_parameter(e, 10, 0, 2500.0));
    out = left(render(e, 256, 256));
    CHECK_EQ(report(e).total_frames, 1000u); // not yet: restart pending
    REQUIRE(se_plugin_idle(e) == SE_OK);
    uint32_t lat = 0;
    se_plugin_latency(e, 10, &lat);
    CHECK_EQ(lat, 2500u);
    std::vector<se_pdc_track> tr;
    CHECK_EQ(report(e, &tr).total_frames, 2500u);
    CHECK(tr[0].delay_frames == 0 && tr[1].delay_frames == 2500 && tr[2].delay_frames == 2500);
    // After the transient, impulses (every 6000) land at t + 2500, aligned.
    se_transport_seek(e, 0);
    out = left(render(e, 30000, 256));
    CHECK(nonzero(out) == (std::vector<std::size_t>{2500, 8500, 14500, 20500, 26500}));
    CHECK_EQ(out[8500], 1.5f);

    // VST3: restartComponent(kLatencyChanged) from the controller.
    REQUIRE(saine::testing::plugin_set_parameter(e, 11, 0, 4000.0 / kVst3MaxLatency));
    render(e, 256, 256);
    REQUIRE(se_plugin_idle(e) == SE_OK);
    se_plugin_latency(e, 11, &lat);
    CHECK_EQ(lat, 4000u);
    CHECK_EQ(report(e, &tr).total_frames, 4000u);
    CHECK(tr[0].delay_frames == 1500 && tr[1].delay_frames == 0 && tr[2].delay_frames == 4000);
    se_transport_seek(e, 0);
    out = left(render(e, 30000, 256));
    // Seek flushes our delay lines, not the plugins' own buffers: the CLAP
    // instance (not restarted this time) still holds the last 2500 frames
    // before the seek, which come out within the first L frames.
    std::vector<std::size_t> after_l;
    for (std::size_t i : nonzero(out))
        if (i >= 4000) after_l.push_back(i);
    CHECK(after_l == (std::vector<std::size_t>{4000, 10000, 16000, 22000, 28000}));
    CHECK_EQ(out[10000], 1.5f);

    // Shrinking back to 0 (CLAP 0, VST3 0): no compensation, no lines.
    REQUIRE(saine::testing::plugin_set_parameter(e, 10, 0, 0.0));
    REQUIRE(saine::testing::plugin_set_parameter(e, 11, 0, 0.0));
    render(e, 256, 256);
    REQUIRE(se_plugin_idle(e) == SE_OK);
    CHECK_EQ(report(e).total_frames, 0u);
    se_transport_seek(e, 0);
    out = left(render(e, 7000, 256));
    CHECK(nonzero(out) == (std::vector<std::size_t>{0, 6000}));
}

TEST_CASE(pdc_seek_and_stop_flush_pause_rings_out_loop_continues) {
    EngineHandle e(48000, 256);
    std::vector<float> ramp_v(60000);
    for (std::size_t i = 0; i < ramp_v.size(); ++i) ramp_v[i] = 1e-5f * static_cast<float>(i + 1);
    const uint64_t a = load_asset(e, {ramp_v});
    PSession s;
    s.audio(1, {}, {clap_latency(10, 1000)}); // only sets L = 1000
    s.audio(2, {clip(1, a, 0, 0, 60000)});    // delayed by 1000
    REQUIRE(s.publish(e) == SE_OK);
    const auto at = [&](int64_t timeline) { return ramp_v[static_cast<std::size_t>(timeline)]; };

    // Seek flushes: after seeking to 20000 the first L frames are silent,
    // then the content from 20000 follows (no pre-seek audio).
    se_transport_seek(e, 0);
    se_transport_play(e);
    std::vector<float> out = left(render(e, 5000, 256));
    CHECK_EQ(out[999], 0.0f);
    CHECK_EQ(out[1000], at(0));
    CHECK_EQ(out[4999], at(3999));
    se_transport_seek(e, 20000);
    out = left(render(e, 3000, 256));
    long bad = 0;
    for (std::size_t i = 0; i < 1000; ++i) bad += out[i] != 0.0f;
    for (std::size_t i = 1000; i < 3000; ++i) bad += out[i] != at(20000 + static_cast<int64_t>(i) - 1000);
    CHECK_EQ(bad, 0L);

    // Pause: the last L frames ring out, then silence.
    se_transport_pause(e); // transport at 23000; content up to 21999 heard
    out = left(render(e, 2000, 256));
    bad = 0;
    for (std::size_t i = 0; i < 1000; ++i) bad += out[i] != at(22000 + static_cast<int64_t>(i));
    for (std::size_t i = 1000; i < 2000; ++i) bad += out[i] != 0.0f;
    CHECK_EQ(bad, 0L);
    // Play continues where it paused (the line restarts from silence: the
    // tail already played).
    se_transport_play(e);
    out = left(render(e, 2000, 256));
    bad = 0;
    for (std::size_t i = 0; i < 1000; ++i) bad += out[i] != 0.0f;
    for (std::size_t i = 1000; i < 2000; ++i) bad += out[i] != at(23000 + static_cast<int64_t>(i) - 1000);
    CHECK_EQ(bad, 0L);

    // Stop flushes immediately (no tail).
    se_transport_stop(e);
    out = left(render(e, 1500, 256));
    CHECK(nonzero(out).empty());

    // Loop wrap keeps the lines running: the output is the playback
    // history delayed by L, also across wraps (loop end's content is heard
    // after the wrap, then the loop start's).
    REQUIRE(se_transport_set_loop(e, 1, 2000, 6000) == SE_OK);
    se_transport_seek(e, 0);
    se_transport_play(e);
    out = left(render(e, 15000, 333));
    std::vector<int64_t> history;
    for (int64_t p = 0; history.size() < out.size();) {
        history.push_back(p);
        if (++p == 6000) p = 2000;
    }
    bad = 0;
    for (std::size_t i = 0; i < out.size(); ++i) {
        const float want = i < 1000 ? 0.0f : at(history[i - 1000]);
        bad += out[i] != want;
    }
    CHECK_EQ(bad, 0L);
}

TEST_CASE(pdc_cap_clamps_and_reports) {
    EngineHandle e(48000, 256);
    const uint64_t a = load_asset(e, {impulse(1000, 0)});
    PSession s;
    s.audio(1, {}, {clap_latency(10, 200000)});
    s.audio(2, {clip(1, a, 100, 0, 1000)});
    REQUIRE(s.publish(e) == SE_OK);
    REQUIRE(error_count(e) == 0);
    std::vector<se_pdc_track> tr;
    const se_pdc_report r = report(e, &tr);
    CHECK_EQ(r.clamped, uint8_t{1});
    CHECK_EQ(r.total_frames, uint32_t{SE_PDC_MAX_DELAY_FRAMES});
    CHECK_EQ(r.uncapped_frames, 200000u);
    CHECK_EQ(tr[0].latency_frames, 200000u);
    CHECK_EQ(tr[0].delay_frames, 0u);
    CHECK_EQ(tr[1].delay_frames, uint32_t{SE_PDC_MAX_DELAY_FRAMES});
    // The other track is delayed by the cap (the latent one stays late).
    const std::vector<float> out = play_left(e, SE_PDC_MAX_DELAY_FRAMES + 2000, 0, 4096);
    CHECK(nonzero(out) == std::vector<std::size_t>{100 + SE_PDC_MAX_DELAY_FRAMES});
    // A bypassed slot beyond the cap is clamped too.
    s.tracks[0].devices[0].enabled = 0;
    REQUIRE(s.publish(e) == SE_OK);
    CHECK_EQ(report(e).clamped, uint8_t{1});
    // Disabled: nothing is clamped (nothing is applied).
    REQUIRE(se_pdc_set_enabled(e, 0) == SE_OK);
    CHECK_EQ(report(e).clamped, uint8_t{0});
}

// ---------------------------------------------------------------- RT safety

TEST_CASE(pdc_rt_render_does_not_allocate) {
    EngineHandle e(48000, 256);
    const uint64_t a = load_asset(e, {sine(48000, 330.0, 48000.0, 0.3), sine(48000, 220.0, 48000.0, 0.3)});
    PSession s;
    for (uint64_t t = 0; t < 6; ++t)
        s.audio(t + 1, {clip(t + 1, a, static_cast<int64_t>(t) * 100, 0, 40000)},
                {clap_latency(10 + 2 * t, 100 * static_cast<uint32_t>(t), t % 2 == 0),
                 vst3_latency(11 + 2 * t, 50 * static_cast<uint32_t>(t))});
    PSession::Track& mon = s.audio(7, {clip(7, a, 0, 0, 40000)});
    mon.d.monitor = 1;
    mon.d.arm = 1;
    mon.d.input_mode = SE_INPUT_MONO;
    s.audio(8, {clip(8, a, 0, 0, 40000)}).d.mute = 1;
    REQUIRE(s.publish(e) == SE_OK);
    REQUIRE(error_count(e) == 0);
    CHECK(report(e).total_frames > 0);
    se_metronome_desc m{};
    m.enabled = 1;
    m.gain = 0.3f;
    m.frames_per_beat = 12000.0;
    m.beats_per_bar = 4;
    REQUIRE(se_metronome_set(e, &m) == SE_OK);
    std::vector<float> in(8192, 0.1f), buf(2 * 8192);
    REQUIRE(se_transport_play(e) == SE_OK);
    REQUIRE(se_render_offline_with_input(e, in.data(), 1, buf.data(), 256) == SE_OK); // warm-up
    REQUIRE(se_transport_set_loop(e, 1, 1000, 30000) == SE_OK);
    const long before = rt_alloc_count();
    const uint32_t sizes[] = {256, 1, 333, 8192, 64, 255};
    for (int i = 0; i < 600; ++i) {
        if (i % 97 == 50) se_transport_seek(e, i * 37);
        if (i % 150 == 149) {
            se_transport_pause(e);
            for (int k = 0; k < 4; ++k) {
                RtAllocGuard g;
                REQUIRE(se_render_offline_with_input(e, in.data(), 1, buf.data(), 256) == SE_OK);
            }
            se_transport_play(e);
        }
        if (i % 120 == 60) { // latency changes: restart + re-plan on the main thread (not under the guard)
            saine::testing::plugin_set_parameter(e, 10, 0, 1000.0 + i);
            saine::testing::plugin_set_parameter(e, 13, 0, (300.0 + i) / kVst3MaxLatency);
            REQUIRE(se_render_offline_with_input(e, in.data(), 1, buf.data(), 64) == SE_OK);
            REQUIRE(se_plugin_idle(e) == SE_OK);
        }
        if (i == 300) REQUIRE(se_pdc_set_enabled(e, 0) == SE_OK);
        if (i == 400) REQUIRE(se_pdc_set_enabled(e, 1) == SE_OK);
        const uint32_t n = sizes[i % 6];
        RtAllocGuard g;
        REQUIRE(se_render_offline_with_input(e, in.data(), 1, buf.data(), n) == SE_OK);
    }
    CHECK(report(e).total_frames >= 1000u);
    CHECK_EQ(rt_alloc_count() - before, 0L);
}
