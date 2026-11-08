// A fixed, deterministic session exercising most of the render path (clips,
// built-in synth, instrument plugin, CLAP/VST3 inserts, bypass, mute,
// monitoring + recording with injected input, metronome, loop, seeks,
// pause/stop, odd block sizes, export). Used by test_pdc.cpp to prove that
// with delay compensation OFF the engine is bit-identical to the engine
// before PDC existed: the expected hashes were produced by running this
// exact scenario against the pre-PDC library (commit bf473ce), see
// pdc_off_is_bit_identical_to_the_pre_pdc_engine.
//
// Only the public C ABI and test_util are used, so the same file compiles
// against the old library. `with_latency` adds SaineTestLatency instances
// (CLAP N=1000 enabled, N=500 bypassed, VST3 N=300, N=3000 on a muted track).
#pragma once

#include "saine_engine.h"
#include "test_util.hpp"

#include <cmath>
#include <cstdint>
#include <cstdio>
#include <filesystem>
#include <string>
#include <vector>

namespace pdc_golden {

struct Hasher {
    uint64_t h = 1469598103934665603ull;
    void bytes(const void* p, std::size_t n) {
        const auto* b = static_cast<const uint8_t*>(p);
        for (std::size_t i = 0; i < n; ++i) {
            h ^= b[i];
            h *= 1099511628211ull;
        }
    }
    template <class T>
    void value(const T& v) {
        bytes(&v, sizeof v);
    }
};

inline se_device_desc dev(uint64_t id, uint8_t format, const char* plugin_id, const std::string& path,
                          bool enabled = true, const uint8_t* state = nullptr, std::size_t state_len = 0) {
    se_device_desc d{};
    d.device_id = id;
    d.name = "dev";
    d.plugin_id = plugin_id;
    d.path = path.c_str();
    d.format = format;
    d.enabled = enabled ? 1 : 0;
    d.state = state;
    d.state_len = state_len;
    return d;
}

// Returns a hash of every rendered sample, the export file and the takes.
// `configure(engine)` runs right after creation (e.g. to switch PDC off).
template <class Configure>
uint64_t run(const std::string& plugin_dir, bool with_latency, Configure&& configure) {
    using namespace testutil;
    EngineHandle e(48000, 256);
    configure(e.get());
    Hasher hash;

    const uint64_t a = load_asset(e, {sine(60000, 330.0, 48000.0, 0.3), sine(60000, 220.0, 48000.0, 0.25)});
    std::vector<float> imp(20000, 0.0f);
    for (std::size_t i = 0; i < imp.size(); i += 997) imp[i] = 0.5f - static_cast<float>(i % 7) * 0.05f;
    const uint64_t b = load_asset(e, {imp});

    const std::string clap_gain = plugin_dir + "/SaineTestGain.clap";
    const std::string clap_sine = plugin_dir + "/SaineTestSine.clap";
    const std::string clap_lat = plugin_dir + "/SaineTestLatency.clap";
    const std::string vst3_gain = plugin_dir + "/SaineTestGain.vst3";
    const std::string vst3_lat = plugin_dir + "/SaineTestLatency.vst3";
    const char* const vst3_gain_uid = "5A17E5A16E0000018A1E7E576A170001";
    const char* const vst3_lat_uid = "5A17E5A16E0000038A1E7E576A170003";
    const uint8_t n1000[4] = {0xE8, 0x03, 0, 0}, n500[4] = {0xF4, 0x01, 0, 0}, n300[4] = {0x2C, 0x01, 0, 0},
                  n200[4] = {0xC8, 0, 0, 0}, n3000[4] = {0xB8, 0x0B, 0, 0};

    // VST3 state: "SVS3" container (u32 version 1, u64 len + component state, u64 len + controller state).
    const uint8_t vst3_n300[] = {'S', 'V', 'S', '3', 1, 0, 0, 0, 4, 0, 0, 0, 0, 0, 0, 0, n300[0], n300[1], n300[2], n300[3],
                                 0, 0, 0, 0, 0, 0, 0, 0};
    std::vector<std::vector<se_device_desc>> devs(6);
    devs[0] = {dev(10, SE_PLUGIN_CLAP, "studio.saine.test.gain", clap_gain)};
    devs[1] = {dev(11, SE_PLUGIN_VST3, vst3_gain_uid, vst3_gain)};
    devs[2] = {dev(13, SE_PLUGIN_CLAP, "studio.saine.test.gain", clap_gain)};
    devs[3] = {};
    devs[4] = {};
    devs[5] = {};
    if (with_latency) {
        devs[1].push_back(dev(12, SE_PLUGIN_CLAP, "studio.saine.test.latency", clap_lat, true, n1000, 4));
        devs[2].push_back(dev(14, SE_PLUGIN_CLAP, "studio.saine.test.latency", clap_lat, false, n500, 4));
        devs[3].push_back(dev(16, SE_PLUGIN_VST3, vst3_lat_uid, vst3_lat, true, vst3_n300, sizeof vst3_n300));
        devs[4].push_back(dev(17, SE_PLUGIN_CLAP, "studio.saine.test.latency", clap_lat, true, n200, 4));
        devs[5].push_back(dev(18, SE_PLUGIN_CLAP, "studio.saine.test.latency", clap_lat, true, n3000, 4));
    }
    const se_device_desc sine_plug = dev(15, SE_PLUGIN_CLAP, "studio.saine.test.sine", clap_sine);

    std::vector<std::vector<se_clip_desc>> clips = {
        {clip(1, a, 0, 0, 25000), clip(2, a, 30000, 1000, 20000)},
        {clip(3, b, 1000, 0, 20000)},
        {clip(4, b, 0, 0, 9000)},
        {},
        {clip(5, a, 5000, 0, 40000)},
        {clip(6, b, 0, 0, 20000)}};
    std::vector<se_note_desc> notes;
    for (int i = 0; i < 40; ++i) {
        se_note_desc n{};
        n.start = i * 1500;
        n.end = i * 1500 + 2200;
        n.pitch = static_cast<uint8_t>(50 + (i * 5) % 24);
        n.velocity = static_cast<uint8_t>(60 + i % 60);
        notes.push_back(n);
    }

    std::vector<se_track_desc_v5> td(6);
    for (std::size_t i = 0; i < td.size(); ++i) {
        se_track_desc_v5& t = td[i];
        t = se_track_desc_v5{};
        t.track_id = i + 1;
        t.gain = 0.6f + 0.1f * static_cast<float>(i);
        t.pan = -0.5f + 0.2f * static_cast<float>(i);
        t.clips = clips[i].empty() ? nullptr : clips[i].data();
        t.clip_count = static_cast<uint32_t>(clips[i].size());
        t.devices = devs[i].empty() ? nullptr : devs[i].data();
        t.device_count = static_cast<uint32_t>(devs[i].size());
    }
    td[2].instrument = 1; // built-in synth + clip
    td[2].notes = notes.data();
    td[2].note_count = static_cast<uint32_t>(notes.size());
    td[3].instrument = 1; // instrument plugin
    td[3].notes = notes.data();
    td[3].note_count = static_cast<uint32_t>(notes.size());
    td[3].instrument_plugin = &sine_plug;
    td[4].arm = 1; // monitored + armed input
    td[4].monitor = 1;
    td[4].input_mode = SE_INPUT_MONO;
    td[4].input_channel = 1;
    td[5].mute = 1;
    se_session_desc_v5 sd{td.data(), static_cast<uint32_t>(td.size()), 0.9f};
    if (se_session_publish_v5(e, &sd) != SE_OK) return 0;
    std::size_t errors = 0;
    se_plugin_load_errors(e, nullptr, 0, &errors);
    if (errors != 0) return 1;

    const auto input_at = [](uint32_t c, std::size_t k) {
        return static_cast<float>(std::sin(0.013 * static_cast<double>(k) + c)) * 0.2f;
    };
    std::size_t in_k = 0;
    const auto render_in = [&](uint32_t frames, const std::vector<uint32_t>& blocks) {
        std::vector<float> in(static_cast<std::size_t>(frames) * 2), out(static_cast<std::size_t>(frames) * 2);
        for (std::size_t k = 0; k < frames; ++k)
            for (uint32_t c = 0; c < 2; ++c) in[k * 2 + c] = input_at(c, in_k + k);
        in_k += frames;
        std::size_t done = 0;
        for (std::size_t i = 0; done < frames; ++i) {
            const auto n = static_cast<uint32_t>(std::min<std::size_t>(blocks[i % blocks.size()], frames - done));
            if (se_render_offline_with_input(e, in.data() + done * 2, 2, out.data() + done * 2, n) != SE_OK) return;
            done += n;
        }
        hash.bytes(out.data(), out.size() * sizeof(float));
    };

    se_metronome_desc m{};
    m.enabled = 1;
    m.gain = 0.3f;
    m.frames_per_beat = 12000.0;
    m.beats_per_bar = 4;
    se_metronome_set(e, &m);
    se_transport_play(e);
    render_in(9000, {256, 1, 333, 64, 255});
    se_transport_seek(e, 2500);
    render_in(4000, {128, 97});
    se_transport_pause(e);
    render_in(3000, {256});
    se_transport_play(e);
    render_in(5000, {512, 7, 256});
    se_transport_set_loop(e, 1, 20000, 50000);
    render_in(40000, {256, 300, 1});

    // Recording (injected input: 0 device latency).
    const std::string dir =
        (std::filesystem::temp_directory_path() / ("saine_pdc_golden_" + std::to_string(with_latency))).string();
    std::filesystem::remove_all(dir);
    se_transport_set_loop(e, 0, 0, 0);
    se_transport_seek(e, 12000);
    if (se_record_start(e, dir.c_str()) == SE_OK) {
        render_in(6000, {256, 100});
        se_transport_seek(e, 30000);
        render_in(3000, {256});
        std::size_t takes = 0;
        se_record_stop(e, &takes);
        hash.value(takes);
        for (std::size_t i = 0; i < takes; ++i) {
            se_recorded_take t{};
            se_record_get_take(e, i, &t);
            hash.value(t.track_id);
            hash.value(t.start_frame);
            hash.value(t.frames);
        }
    }
    std::filesystem::remove_all(dir);
    se_transport_stop(e);
    render_in(2000, {256});
    se_transport_play(e);
    render_in(3000, {256});

    // Export (takes the RT ownership because of the plugins).
    se_transport_stop(e);
    TempFile f("pdc_golden_export.wav");
    if (se_export_wav(e, f.path.c_str(), 1000, 41000) != SE_OK) return 2;
    if (FILE* fp = testutil::fopen_utf8(f.path.c_str(), "rb")) {
        std::vector<uint8_t> bytes(1 << 20);
        bytes.resize(std::fread(bytes.data(), 1, bytes.size(), fp));
        std::fclose(fp);
        hash.value(bytes.size());
        hash.bytes(bytes.data(), bytes.size());
    }
    return hash.h;
}

} // namespace pdc_golden
