// Plugin hosting: insert chains (AU / VST3 / CLAP), instrument plugins,
// state, lifetime across republishes, load errors and RT safety.
//
// Uses Apple's built-in Audio Units (AULowpass, AUDistortion,
// DLSMusicDevice) and the test plugins built into <build>/test_plugins.
// Tracks are panned hard left so the left channel is the track signal
// unscaled by the pan law.
#include "test_harness.hpp"
#include "test_hooks.hpp"
#include "test_util.hpp"

#include <algorithm>
#include <cmath>
#include <cstring>
#include <string>
#include <thread>
#include <vector>

using namespace testutil;

namespace {

const std::string kPluginDir = SAINE_TEST_PLUGIN_DIR;
const std::string kClapGain = kPluginDir + "/SaineTestGain.clap";
const std::string kClapSine = kPluginDir + "/SaineTestSine.clap";
const std::string kVst3Gain = kPluginDir + "/SaineTestGain.vst3";
const char* const kVst3GainUid = "5A17E5A16E0000018A1E7E576A170001";
const char* const kAuLowpass = "aufx lpas appl";
const char* const kAuDistortion = "aufx dist appl";
const char* const kAuDls = "aumu dls  appl";

se_device_desc au(uint64_t id, const char* au_id, bool enabled = true) {
    se_device_desc d{};
    d.device_id = id;
    d.name = "AU";
    d.plugin_id = au_id;
    d.format = SE_PLUGIN_AU;
    d.enabled = enabled ? 1 : 0;
    return d;
}

se_device_desc clap(uint64_t id, const std::string& path, const char* plugin_id, bool enabled = true) {
    se_device_desc d{};
    d.device_id = id;
    d.name = "CLAP";
    d.plugin_id = plugin_id;
    d.path = path.c_str();
    d.format = SE_PLUGIN_CLAP;
    d.enabled = enabled ? 1 : 0;
    return d;
}

se_device_desc clap_gain(uint64_t id, bool enabled = true) { return clap(id, kClapGain, "studio.saine.test.gain", enabled); }

se_device_desc vst3_gain(uint64_t id, bool enabled = true) {
    se_device_desc d{};
    d.device_id = id;
    d.name = "VST3";
    d.plugin_id = kVst3GainUid;
    d.path = kVst3Gain.c_str();
    d.format = SE_PLUGIN_VST3;
    d.enabled = enabled ? 1 : 0;
    return d;
}

// One session description with owned storage.
struct Session5 {
    struct Track {
        se_track_desc_v5 d{};
        std::vector<se_clip_desc> clips;
        std::vector<se_note_desc> notes;
        std::vector<se_device_desc> devices;
        se_device_desc instrument{};
        bool has_instrument = false;
    };
    std::vector<Track> tracks;
    float master_gain = 1.0f;

    Track& audio(uint64_t id, std::vector<se_clip_desc> clips, std::vector<se_device_desc> devices) {
        Track t;
        t.d.track_id = id;
        t.d.gain = 1.0f;
        t.d.pan = -1.0f; // hard left: L = track signal
        t.clips = std::move(clips);
        t.devices = std::move(devices);
        tracks.push_back(std::move(t));
        return tracks.back();
    }
    Track& instrument(uint64_t id, std::vector<se_note_desc> notes, const se_device_desc* plugin,
                      std::vector<se_device_desc> devices = {}) {
        Track t;
        t.d.track_id = id;
        t.d.gain = 1.0f;
        t.d.pan = -1.0f;
        t.d.instrument = 1;
        t.notes = std::move(notes);
        t.devices = std::move(devices);
        if (plugin != nullptr) {
            t.instrument = *plugin;
            t.has_instrument = true;
        }
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
            d.track_name = "Track";
            v.push_back(d);
        }
        se_session_desc_v5 s{};
        s.tracks = v.data();
        s.track_count = static_cast<uint32_t>(v.size());
        s.master_gain = master_gain;
        return se_session_publish_v5(e, &s);
    }
};

se_note_desc note(int64_t start, int64_t end, uint8_t pitch, uint8_t velocity = 100) {
    se_note_desc n{};
    n.start = start;
    n.end = end;
    n.pitch = pitch;
    n.velocity = velocity;
    return n;
}

std::vector<float> left(const std::vector<float>& inter, std::size_t from = 0, std::size_t to = SIZE_MAX) {
    std::vector<float> l;
    for (std::size_t i = from; i < inter.size() / 2 && i < to; ++i) l.push_back(inter[2 * i]);
    return l;
}

double rms(const std::vector<float>& v) {
    double s = 0;
    for (float x : v) s += static_cast<double>(x) * x;
    return v.empty() ? 0.0 : std::sqrt(s / static_cast<double>(v.size()));
}

std::vector<float> play_render(se_engine* e, uint32_t frames, uint32_t block = 256) {
    se_transport_seek(e, 0);
    se_transport_play(e);
    return render(e, frames, block);
}

std::size_t error_count(se_engine* e) {
    std::size_t n = 0;
    se_plugin_load_errors(e, nullptr, 0, &n);
    return n;
}

} // namespace

// ---------------------------------------------------------------- effects

TEST_CASE(plugin_au_lowpass_attenuates_high_and_passes_low) {
    for (const double freq : {200.0, 15000.0}) {
        EngineHandle e(48000, 256);
        const uint64_t a = load_asset(e, {sine(48000, freq, 48000.0, 0.5)});
        Session5 dry, wet;
        dry.audio(1, {clip(1, a, 0, 0, 48000)}, {});
        wet.audio(1, {clip(1, a, 0, 0, 48000)}, {au(10, kAuLowpass)});
        REQUIRE(dry.publish(e) == SE_OK);
        const auto ref = left(play_render(e, 24000), 12000);
        REQUIRE(wet.publish(e) == SE_OK);
        CHECK_EQ(error_count(e), std::size_t{0});
        const auto out = left(play_render(e, 24000), 12000);
        const double ratio = rms(out) / rms(ref);
        if (freq < 1000) CHECK(ratio > 0.95 && ratio < 1.05);
        else CHECK(ratio < 0.25);
    }
}

TEST_CASE(plugin_clap_and_vst3_gain_halve_the_signal) {
    for (int fmt = 0; fmt < 2; ++fmt) {
        EngineHandle e(48000, 256);
        const uint64_t a = load_asset(e, {sine(9600, 440.0, 48000.0, 0.5)});
        Session5 dry, wet;
        dry.audio(1, {clip(1, a, 0, 0, 9600)}, {});
        wet.audio(1, {clip(1, a, 0, 0, 9600)}, {fmt == 0 ? clap_gain(10) : vst3_gain(10)});
        REQUIRE(dry.publish(e) == SE_OK);
        const auto ref = play_render(e, 9600, 333);
        REQUIRE(wet.publish(e) == SE_OK);
        CHECK_EQ(error_count(e), std::size_t{0});
        const auto out = play_render(e, 9600, 333);
        double max_err = 0;
        for (std::size_t i = 0; i < out.size(); ++i) max_err = std::max(max_err, std::fabs(out[i] - 0.5 * ref[i]));
        CHECK(max_err < 1e-7);
        CHECK(rms(left(out)) > 0.1);
    }
}

TEST_CASE(plugin_bypass_passes_bit_exact) {
    EngineHandle e(48000, 256);
    const uint64_t a = load_asset(e, {sine(9600, 3000.0, 48000.0, 0.5), sine(9600, 220.0, 48000.0, 0.3)});
    Session5 dry, bypassed;
    dry.audio(1, {clip(1, a, 0, 0, 9600)}, {});
    dry.tracks[0].d.pan = 0.3f;
    bypassed.audio(1, {clip(1, a, 0, 0, 9600)},
                   {au(10, kAuLowpass, false), clap_gain(11, false), vst3_gain(12, false)});
    bypassed.tracks[0].d.pan = 0.3f;
    REQUIRE(dry.publish(e) == SE_OK);
    const auto ref = play_render(e, 9600);
    REQUIRE(bypassed.publish(e) == SE_OK);
    CHECK_EQ(error_count(e), std::size_t{0});
    const auto out = play_render(e, 9600);
    CHECK(out == ref);
}

TEST_CASE(plugin_chain_order_matters) {
    // Distortion is nonlinear: gain -> distortion != distortion -> gain.
    EngineHandle e(48000, 256);
    const uint64_t a = load_asset(e, {sine(24000, 220.0, 48000.0, 0.9)});
    Session5 ab, ba, gg;
    ab.audio(1, {clip(1, a, 0, 0, 24000)}, {clap_gain(10), au(11, kAuDistortion)});
    ba.audio(1, {clip(1, a, 0, 0, 24000)}, {au(11, kAuDistortion), clap_gain(10)});
    REQUIRE(ab.publish(e) == SE_OK);
    const auto out_ab = left(play_render(e, 24000), 4800);
    REQUIRE(ba.publish(e) == SE_OK); // same device ids, reordered: instances are kept
    const auto out_ba = left(play_render(e, 24000), 4800);
    CHECK_EQ(saine::testing::plugin_instantiations(e), uint64_t{2});
    double diff = 0;
    for (std::size_t i = 0; i < out_ab.size(); ++i) diff = std::max(diff, static_cast<double>(std::fabs(out_ab[i] - out_ba[i])));
    CHECK(diff > 0.01);
    CHECK(rms(out_ab) > 0.01 && rms(out_ba) > 0.01);
    // Linear chain sanity: gain -> gain = quarter.
    gg.audio(1, {clip(1, a, 0, 0, 24000)}, {clap_gain(20), vst3_gain(21)});
    Session5 dry;
    dry.audio(1, {clip(1, a, 0, 0, 24000)}, {});
    REQUIRE(dry.publish(e) == SE_OK);
    const auto ref = left(play_render(e, 4800));
    REQUIRE(gg.publish(e) == SE_OK);
    const auto out = left(play_render(e, 4800));
    double max_err = 0;
    for (std::size_t i = 0; i < out.size(); ++i) max_err = std::max(max_err, static_cast<double>(std::fabs(out[i] - 0.25f * ref[i])));
    CHECK(max_err < 1e-7);
}

TEST_CASE(plugin_instances_survive_republish) {
    EngineHandle e(48000, 256);
    const uint64_t a = load_asset(e, {constant(4800, 0.5f)});
    Session5 s;
    s.audio(1, {clip(1, a, 0, 0, 4800)}, {clap_gain(10), vst3_gain(11), au(12, kAuLowpass)});
    REQUIRE(s.publish(e) == SE_OK);
    CHECK_EQ(saine::testing::plugin_instantiations(e), uint64_t{3});
    for (int i = 0; i < 6; ++i) {
        s.tracks[0].d.gain = 0.5f + 0.1f * static_cast<float>(i);
        s.tracks[0].devices[1].enabled = static_cast<uint8_t>(i % 2);
        s.tracks[0].d.mute = static_cast<uint8_t>(i == 3);
        REQUIRE(s.publish(e) == SE_OK);
        play_render(e, 512);
    }
    // Moving a device to another track keeps it too.
    s.audio(2, {}, {});
    const se_device_desc moved = s.tracks[0].devices[2];
    s.tracks[0].devices.pop_back();
    s.tracks[1].devices.push_back(moved);
    REQUIRE(s.publish(e) == SE_OK);
    CHECK_EQ(saine::testing::plugin_instantiations(e), uint64_t{3});
    CHECK(se_plugin_is_live(e, 12) == 1);
    // Removing retires; re-adding (undo) creates a new instance.
    s.tracks[1].devices.clear();
    REQUIRE(s.publish(e) == SE_OK);
    CHECK(se_plugin_is_live(e, 12) == 0);
    s.tracks[1].devices.push_back(moved);
    REQUIRE(s.publish(e) == SE_OK);
    CHECK_EQ(saine::testing::plugin_instantiations(e), uint64_t{4});
    // Same id, different plugin: replaced.
    s.tracks[1].devices[0] = clap_gain(12);
    REQUIRE(s.publish(e) == SE_OK);
    CHECK_EQ(saine::testing::plugin_instantiations(e), uint64_t{5});
    // A v4 publish (no devices) retires everything.
    se_session_desc_v4 empty{};
    REQUIRE(se_session_publish_v4(e, &empty) == SE_OK);
    CHECK(se_plugin_is_live(e, 10) == 0 && se_plugin_is_live(e, 11) == 0);
    REQUIRE(se_plugin_idle(e) == SE_OK);
}

TEST_CASE(plugin_publish_v5_validation) {
    EngineHandle e(48000, 256);
    Session5 dup;
    dup.audio(1, {}, {clap_gain(10)});
    dup.audio(2, {}, {clap_gain(10)});
    CHECK(dup.publish(e) == SE_ERR_INVALID_ARG);
    Session5 zero;
    zero.audio(1, {}, {clap_gain(0)});
    CHECK(zero.publish(e) == SE_ERR_INVALID_ARG);
    Session5 fmt;
    fmt.audio(1, {}, {clap_gain(5)});
    fmt.tracks[0].devices[0].format = 7;
    CHECK(fmt.publish(e) == SE_ERR_INVALID_ARG);
    Session5 st;
    st.audio(1, {}, {clap_gain(5)});
    st.tracks[0].devices[0].state_len = 4;
    CHECK(st.publish(e) == SE_ERR_INVALID_ARG);
    CHECK_EQ(saine::testing::plugin_instantiations(e), uint64_t{0}); // nothing instantiated on rejected publishes
}

// ---------------------------------------------------------------- state

TEST_CASE(plugin_state_round_trip_clap_and_vst3) {
    for (int fmt = 0; fmt < 2; ++fmt) {
        std::vector<uint8_t> state;
        std::vector<float> out1;
        {
            EngineHandle e(48000, 256);
            const uint64_t a = load_asset(e, {constant(2048, 0.8f)});
            Session5 s;
            s.audio(1, {clip(1, a, 0, 0, 2048)}, {fmt == 0 ? clap_gain(10) : vst3_gain(10)});
            REQUIRE(s.publish(e) == SE_OK);
            REQUIRE(saine::testing::plugin_set_parameter(e, 10, 0, 0.25));
            out1 = play_render(e, 1024);
            CHECK_NEAR(out1[2 * 100], 0.8 * 0.25, 1e-6);
            std::size_t n = 0;
            REQUIRE(se_plugin_save_states(e, &n) == SE_OK);
            REQUIRE(n == 1);
            uint64_t id = 0;
            const uint8_t* data = nullptr;
            std::size_t len = 0;
            REQUIRE(se_plugin_saved_state(e, 0, &id, &data, &len) == SE_OK);
            CHECK_EQ(id, uint64_t{10});
            state.assign(data, data + len);
            CHECK(se_plugin_saved_state(e, 1, &id, &data, &len) == SE_ERR_NOT_FOUND);
        }
        EngineHandle e(48000, 256);
        const uint64_t a = load_asset(e, {constant(2048, 0.8f)});
        Session5 s;
        s.audio(1, {clip(1, a, 0, 0, 2048)}, {fmt == 0 ? clap_gain(10) : vst3_gain(10)});
        s.tracks[0].devices[0].state = state.data();
        s.tracks[0].devices[0].state_len = state.size();
        REQUIRE(s.publish(e) == SE_OK);
        CHECK_EQ(error_count(e), std::size_t{0});
        const auto out2 = play_render(e, 1024);
        CHECK(out1 == out2);
        // Corrupt state: load error, pass-through.
        Session5 bad;
        std::vector<uint8_t> junk = {1, 2};
        bad.audio(1, {clip(1, a, 0, 0, 2048)}, {fmt == 0 ? clap_gain(11) : vst3_gain(11)});
        bad.tracks[0].devices[0].state = junk.data();
        bad.tracks[0].devices[0].state_len = junk.size();
        REQUIRE(bad.publish(e) == SE_OK);
        CHECK_EQ(error_count(e), std::size_t{1});
        CHECK_NEAR(play_render(e, 256)[2 * 10], 0.8, 1e-6);
    }
}

TEST_CASE(plugin_state_round_trip_audio_unit) {
    // AULowpass cutoff (parameter 0) changed -> ClassInfo -> new engine.
    const auto run = [](const std::vector<uint8_t>* state, double cutoff, std::vector<uint8_t>* save) {
        EngineHandle e(48000, 256);
        const uint64_t a = load_asset(e, {sine(24000, 2000.0, 48000.0, 0.5)});
        Session5 s;
        s.audio(1, {clip(1, a, 0, 0, 24000)}, {au(10, kAuLowpass)});
        if (state != nullptr) {
            s.tracks[0].devices[0].state = state->data();
            s.tracks[0].devices[0].state_len = state->size();
        }
        if (s.publish(e) != SE_OK || error_count(e) != 0) return std::vector<float>{};
        if (cutoff > 0) saine::testing::plugin_set_parameter(e, 10, 0, cutoff);
        auto out = left(play_render(e, 24000));
        if (save != nullptr) {
            std::size_t n = 0;
            se_plugin_save_states(e, &n);
            uint64_t id = 0;
            const uint8_t* data = nullptr;
            std::size_t len = 0;
            if (n == 1 && se_plugin_saved_state(e, 0, &id, &data, &len) == SE_OK) save->assign(data, data + len);
        }
        return out;
    };
    std::vector<uint8_t> state;
    const auto changed = run(nullptr, 500.0, &state);
    REQUIRE(!state.empty());
    CHECK(std::memcmp(state.data(), "bplist", 6) == 0);
    const auto restored = run(&state, 0, nullptr);
    const auto fresh = run(nullptr, 0, nullptr);
    REQUIRE(restored.size() == changed.size() && fresh.size() == changed.size());
    CHECK(restored == changed); // same parameter from the first sample
    CHECK(rms(restored) < 0.5 * rms(fresh)); // 2 kHz well above the 500 Hz cutoff
}

// ---------------------------------------------------------------- instruments

TEST_CASE(plugin_instrument_clap_sine_onset_and_pitch) {
    EngineHandle e(48000, 256);
    const se_device_desc sine_plug = clap(30, kClapSine, "studio.saine.test.sine");
    Session5 s;
    s.instrument(1, {note(4800, 14400, 69, 127), note(20000, 22000, 81, 127)}, &sine_plug);
    REQUIRE(s.publish(e) == SE_OK);
    CHECK_EQ(error_count(e), std::size_t{0});
    const auto l = left(play_render(e, 24000, 333));
    std::size_t onset = 0;
    while (onset < l.size() && std::fabs(l[onset]) < 1e-4f) ++onset;
    CHECK_EQ(onset, std::size_t{4800});
    CHECK_NEAR(l[4800], 0.25, 1e-6); // cosine at full amplitude, velocity 127
    // Note-off exactly at 14400; second note at 20000.
    CHECK(std::fabs(l[14399]) > 0.0f);
    bool silent = true;
    for (std::size_t i = 14400; i < 20000; ++i) silent = silent && l[i] == 0.0f;
    CHECK(silent);
    CHECK_NEAR(l[20000], 0.25, 1e-6);
    // Pitch: count positive-going zero crossings over 0.1 s of the A4 note.
    int crossings = 0;
    for (std::size_t i = 5000; i < 9800; ++i) crossings += l[i - 1] < 0.0f && l[i] >= 0.0f;
    CHECK(crossings >= 43 && crossings <= 45); // 440 Hz * 0.1 s
}

TEST_CASE(plugin_instrument_dls_onset) {
    EngineHandle e(48000, 256);
    const se_device_desc dls = au(31, kAuDls);
    Session5 s;
    s.instrument(1, {note(4800, 24000, 60, 110)}, &dls);
    REQUIRE(s.publish(e) == SE_OK);
    CHECK_EQ(error_count(e), std::size_t{0});
    const auto l = left(play_render(e, 28800, 256));
    float before = 0;
    for (std::size_t i = 0; i < 4800; ++i) before = std::max(before, std::fabs(l[i]));
    CHECK(before < 1e-5f);
    std::size_t onset = 4800;
    while (onset < l.size() && std::fabs(l[onset]) < 1e-3f) ++onset;
    std::printf("    DLSMusicDevice onset: %zu frames after the note start\n", onset - 4800);
    CHECK(onset >= 4800 && onset < 4800 + 960); // within 20 ms (attack)
    CHECK(rms(std::vector<float>(l.begin() + 6000, l.begin() + 20000)) > 0.005);
}

TEST_CASE(plugin_instrument_seek_and_stop_send_all_notes_off) {
    const se_device_desc sine_plug = clap(30, kClapSine, "studio.saine.test.sine");
    const se_device_desc dls = au(31, kAuDls);
    for (int which = 0; which < 2; ++which) {
        // Controls, rendered straight through, window = note age 300..400 ms:
        // the note held (`held`) and the note ending at 200 ms (`released`,
        // the same note-off the seek/stop should produce).
        const auto window = [&](int64_t end) {
            EngineHandle e(48000, 256);
            Session5 s;
            s.instrument(1, {note(0, end, 64, 120)}, which == 0 ? &sine_plug : &dls);
            if (s.publish(e) != SE_OK) return -1.0;
            const auto l = left(play_render(e, 19200));
            return rms(std::vector<float>(l.begin() + 14400, l.end()));
        };
        const double control = window(480000);
        const double released = window(9600);
        CHECK(control > 0.01);
        CHECK(released < 0.6 * control);
        EngineHandle e(48000, 256);
        Session5 s;
        s.instrument(1, {note(0, 480000, 64, 120)}, which == 0 ? &sine_plug : &dls);
        REQUIRE(s.publish(e) == SE_OK);
        auto held = left(play_render(e, 9600));
        CHECK(rms(std::vector<float>(held.begin() + 4800, held.end())) > 0.01);
        // Seek into the middle of the note: note-off (no retrigger).
        REQUIRE(se_transport_seek(e, 96000) == SE_OK);
        auto after = left(render(e, 9600, 256));
        double tail = rms(std::vector<float>(after.begin() + 4800, after.end()));
        if (which == 0) CHECK(rms(after) == 0.0); // the test sine stops on the note-off sample
        else CHECK(std::fabs(tail - released) < 0.1 * released); // DLS: same as a note-off (piano + reverb tail)
        std::printf("    %s: seek tail/held = %.3f (note-off reference %.3f)\n", which == 0 ? "sine" : "DLS",
                    tail / control, released / control);
        // Play from the note start again, then stop.
        REQUIRE(se_transport_seek(e, 0) == SE_OK);
        held = left(render(e, 9600, 256));
        CHECK(rms(held) > 0.01);
        REQUIRE(se_transport_stop(e) == SE_OK);
        after = left(render(e, 9600, 256));
        tail = rms(std::vector<float>(after.begin() + 4800, after.end()));
        if (which == 0) CHECK(rms(after) == 0.0);
        else CHECK(tail < 1.1 * released); // (earlier tails in this engine differ slightly)
        std::printf("    %s: stop tail/held = %.3f\n", which == 0 ? "sine" : "DLS", tail / control);
    }
}

TEST_CASE(plugin_instrument_with_insert_chain_and_loop) {
    // Instrument plugin -> CLAP gain insert; loop wraps retrigger the note at the loop start.
    EngineHandle e(48000, 256);
    const se_device_desc sine_plug = clap(30, kClapSine, "studio.saine.test.sine");
    Session5 s;
    s.instrument(1, {note(1000, 3000, 69, 127)}, &sine_plug, {clap_gain(40)});
    REQUIRE(s.publish(e) == SE_OK);
    REQUIRE(se_transport_set_loop(e, 1, 0, 4000) == SE_OK);
    const auto l = left(play_render(e, 12000, 333));
    for (int k = 0; k < 3; ++k) {
        const std::size_t base = static_cast<std::size_t>(k) * 4000;
        CHECK(l[base + 999] == 0.0f);
        CHECK_NEAR(l[base + 1000], 0.125, 1e-6); // 0.25 * 0.5 gain
        CHECK(l[base + 3000] == 0.0f);
    }
}

// ---------------------------------------------------------------- failures

TEST_CASE(plugin_missing_is_load_error_and_passes_through) {
    EngineHandle e(48000, 256);
    const uint64_t a = load_asset(e, {sine(4800, 440.0, 48000.0, 0.5)});
    Session5 dry, broken;
    dry.audio(1, {clip(1, a, 0, 0, 4800)}, {});
    const std::string missing = kPluginDir + "/DoesNotExist.clap";
    broken.audio(1, {clip(1, a, 0, 0, 4800)},
                 {au(10, "aufx zzzz zzzz"), clap(11, missing, "x.y"), clap(12, kClapGain, "wrong.id"),
                  vst3_gain(13)});
    broken.tracks[0].devices[3].plugin_id = "00000000000000000000000000000000";
    const se_device_desc missing_inst = au(14, "aumu zzzz zzzz");
    broken.instrument(2, {note(0, 4800, 60)}, &missing_inst);
    REQUIRE(dry.publish(e) == SE_OK);
    const auto ref = play_render(e, 4800);
    REQUIRE(broken.publish(e) == SE_OK);
    std::vector<se_plugin_message> msgs(8);
    std::size_t n = 0;
    REQUIRE(se_plugin_load_errors(e, msgs.data(), msgs.size(), &n) == SE_OK);
    REQUIRE(n == 5);
    for (std::size_t i = 0; i < n; ++i) {
        CHECK_EQ(msgs[i].device_id, uint64_t{10 + i});
        CHECK(std::strlen(msgs[i].message) > 0);
        std::printf("    load error %llu: %s\n", static_cast<unsigned long long>(msgs[i].device_id), msgs[i].message);
    }
    const auto out = play_render(e, 4800);
    CHECK(out == ref); // inserts pass through, the missing instrument is silent
    // Not retried on republish while published with the same plugin.
    const uint64_t count = saine::testing::plugin_instantiations(e);
    REQUIRE(broken.publish(e) == SE_OK);
    CHECK_EQ(saine::testing::plugin_instantiations(e), count);
    CHECK(se_plugin_is_live(e, 11) == 1);
    uint32_t lat = 7;
    CHECK(se_plugin_latency(e, 11, &lat) == SE_ERR_NOT_FOUND && lat == 0);
    CHECK(se_plugin_editor_is_open(e, 11) == 0);
}

TEST_CASE(plugin_latency_and_editor_thread_check) {
    EngineHandle e(48000, 256);
    Session5 s;
    s.audio(1, {}, {au(10, kAuLowpass), clap_gain(11), vst3_gain(12)});
    REQUIRE(s.publish(e) == SE_OK);
    for (uint64_t id : {10, 11, 12}) {
        uint32_t lat = 99;
        CHECK(se_plugin_latency(e, id, &lat) == SE_OK);
        CHECK_EQ(lat, uint32_t{0});
    }
    // Editors are main-thread only.
    int32_t rc = 0;
    std::thread t([&] { rc = se_plugin_editor_open(e, 10); });
    t.join();
    CHECK(rc == SE_ERR_STATE);
    CHECK(se_plugin_editor_is_open(e, 10) == 0);
    // Plugins without an editor say so (no window is created).
    CHECK(se_plugin_editor_open(e, 11) == SE_ERR_UNSUPPORTED_FORMAT);
    CHECK(se_plugin_editor_open(e, 12) == SE_ERR_UNSUPPORTED_FORMAT);
    CHECK(se_plugin_editor_open(e, 99) == SE_ERR_NOT_FOUND);
}

TEST_CASE(plugin_export_uses_the_chain) {
    EngineHandle e(48000, 256);
    const uint64_t a = load_asset(e, {constant(4800, 0.5f)});
    Session5 s;
    s.audio(1, {clip(1, a, 0, 0, 4800)}, {clap_gain(10)});
    REQUIRE(s.publish(e) == SE_OK);
    TempFile f("plugin_export.wav");
    REQUIRE(se_export_wav(e, f.path.c_str(), 0, 4800) == SE_OK);
    std::vector<uint8_t> bytes;
    {
        FILE* fp = std::fopen(f.path.c_str(), "rb");
        REQUIRE(fp != nullptr);
        bytes.resize(44 + 4800 * 8 + 64);
        bytes.resize(std::fread(bytes.data(), 1, bytes.size(), fp));
        std::fclose(fp);
    }
    // Last sample of the data chunk (left channel of the last frame).
    float last_l = 0;
    std::memcpy(&last_l, bytes.data() + bytes.size() - 8, 4);
    CHECK_NEAR(last_l, 0.5 * 0.5, 1e-6);
}

// ---------------------------------------------------------------- RT safety

TEST_CASE(plugin_rt_render_does_not_allocate) {
    // Our code around plugins must not allocate. Only our own test plugins
    // are used here (third-party plugins may allocate; out of our control).
    EngineHandle e(48000, 256);
    const uint64_t a = load_asset(e, {sine(48000, 330.0, 48000.0, 0.3), sine(48000, 220.0, 48000.0, 0.3)});
    const se_device_desc sine_plug = clap(100, kClapSine, "studio.saine.test.sine");
    Session5 s;
    for (uint64_t t = 0; t < 8; ++t)
        s.audio(t + 1, {clip(1, a, static_cast<int64_t>(t) * 100, 0, 40000)},
                {clap_gain(10 + 2 * t), vst3_gain(11 + 2 * t, t % 3 != 0)});
    std::vector<se_note_desc> notes;
    for (int i = 0; i < 200; ++i) notes.push_back(note(i * 400, i * 400 + 900, static_cast<uint8_t>(48 + i % 24)));
    s.instrument(20, notes, &sine_plug, {clap_gain(200)});
    REQUIRE(s.publish(e) == SE_OK);
    CHECK_EQ(error_count(e), std::size_t{0});
    std::vector<float> buf(2 * 8192);
    REQUIRE(se_transport_play(e) == SE_OK);
    REQUIRE(se_render_offline(e, buf.data(), 256) == SE_OK); // warm-up (plugins' start_processing etc.)
    REQUIRE(se_transport_set_loop(e, 1, 1000, 30000) == SE_OK);
    const long before = rt_alloc_count();
    const uint32_t sizes[] = {256, 1, 333, 8192, 64, 255};
    for (int i = 0; i < 600; ++i) {
        if (i % 97 == 50) se_transport_seek(e, i * 37);
        if (i % 150 == 149) {
            se_transport_pause(e);
            se_transport_play(e);
        }
        if (i % 200 == 100) {
            saine::testing::plugin_set_parameter(e, 10, 0, 0.3);
            saine::testing::plugin_set_parameter(e, 11, 0, 0.7);
        }
        const uint32_t n = sizes[i % 6];
        RtAllocGuard g;
        REQUIRE(se_render_offline(e, buf.data(), n) == SE_OK);
    }
    // A republish (instances kept) in between, rendered under the guard too.
    s.tracks[0].d.gain = 0.4f;
    REQUIRE(s.publish(e) == SE_OK);
    for (int i = 0; i < 50; ++i) {
        RtAllocGuard g;
        REQUIRE(se_render_offline(e, buf.data(), 256) == SE_OK);
    }
    CHECK_EQ(rt_alloc_count() - before, 0L);
}
