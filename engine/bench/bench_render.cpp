// Offline render benchmark: N tracks x M clips through se_render_offline
// (the device-callback process path) at 48 kHz / 256 frames.
// Prints measurements only; no pass/fail thresholds.
//
// usage: saine_engine_bench [blocks=4000] [clips_per_track=4] [features=0]
// A last table renders 8 armed + monitored tracks fed by injected input,
// with recording off and on (the recording writer thread runs concurrently).
//   features=1: also enable a 48 001-frame loop (odd length, so the wrap
//   lands mid-block) and the metronome at 120 BPM.
// A second table renders MIDI: instrument tracks (no clips) with 8 notes
// held for the whole run plus one short note retriggered every 100 ms, so
// 8-9 synth voices sound on every track in every block.
// A plugin table renders 32 audio tracks (4 clips each) with two test-gain
// inserts per track (the SaineTestGain CLAP and VST3 bundles built with the
// engine), i.e. 64 plugin process calls per block.
// A delay compensation table renders the same 32 audio tracks with one
// SaineTestLatency insert (N frames) on the first track, PDC on and off:
// N = 0 (L = 0, nothing to compensate) and N = 4096 (31 tracks + the
// metronome line delayed by 4096 frames).
#include "saine_engine.h"

#include <algorithm>
#include <chrono>
#include <cmath>
#include <cstdio>
#include <cstdlib>
#include <filesystem>
#include <numbers>
#include <string>
#include <thread>
#include <vector>

namespace {

constexpr double kRate = 48000.0;
constexpr uint32_t kBlock = 256;

bool write_float_wav(const std::string& path, const std::vector<float>& l, const std::vector<float>& r) {
    std::FILE* f = std::fopen(path.c_str(), "wb");
    if (!f) return false;
    const auto frames = static_cast<uint32_t>(l.size());
    const uint32_t data = frames * 8;
    auto u32 = [&](uint32_t v) { std::fwrite(&v, 4, 1, f); }; // little-endian host assumed (macOS/arm64, x86)
    auto u16 = [&](uint16_t v) { std::fwrite(&v, 2, 1, f); };
    std::fwrite("RIFF", 1, 4, f);
    u32(36 + data);
    std::fwrite("WAVEfmt ", 1, 8, f);
    u32(16);
    u16(3);
    u16(2);
    u32(48000);
    u32(48000 * 8);
    u16(8);
    u16(32);
    std::fwrite("data", 1, 4, f);
    u32(data);
    for (uint32_t i = 0; i < frames; ++i) {
        std::fwrite(&l[i], 4, 1, f);
        std::fwrite(&r[i], 4, 1, f);
    }
    return std::fclose(f) == 0;
}

struct Result {
    double us_per_block_avg;
    double us_per_block_p99;
    double us_per_block_max;
    double rt_ratio;
};

// plugins: 0 = none (se_session_publish), 1 = CLAP gain + VST3 gain per
// track, 2 = two CLAP gains per track (se_session_publish_v5), 3 = one
// SaineTestLatency (N = `latency`) on track 1 only, delay compensation
// `pdc` on/off.
Result run(uint32_t tracks, uint32_t clips_per_track, uint32_t blocks, const std::vector<std::string>& files,
           bool features, int plugins = 0, uint32_t latency = 0, bool pdc = true) {
    se_config cfg{SE_ABI_VERSION, kRate, kBlock, 2};
    se_engine* e = se_engine_create(&cfg);
    if (!e) {
        std::fprintf(stderr, "create failed: %s\n", se_last_error());
        std::exit(1);
    }
    std::vector<uint64_t> assets;
    for (const auto& f : files) {
        se_asset_info info{};
        if (se_asset_load_file(e, f.c_str(), &info) != SE_OK) {
            std::fprintf(stderr, "load failed: %s\n", se_last_error());
            std::exit(1);
        }
        assets.push_back(info.asset_id);
    }
    // Every track has back-to-back (slightly overlapping) clips covering the
    // whole rendered range, so all tracks are active in every block.
    const int64_t total = static_cast<int64_t>(blocks) * kBlock;
    const int64_t clip_len = total / clips_per_track + 1024;
    std::vector<std::vector<se_clip_desc>> clips(tracks);
    std::vector<se_track_desc> tds(tracks);
    for (uint32_t t = 0; t < tracks; ++t) {
        for (uint32_t c = 0; c < clips_per_track; ++c) {
            se_clip_desc cd{};
            cd.clip_id = c + 1;
            cd.asset_id = assets[(t + c) % assets.size()];
            cd.timeline_start = static_cast<int64_t>(c) * (total / clips_per_track);
            cd.source_offset = (t * 131) % 1000;
            cd.length = clip_len;
            cd.gain = 0.5f;
            clips[t].push_back(cd);
        }
        se_track_desc& td = tds[t];
        td = se_track_desc{};
        td.track_id = t + 1;
        td.gain = 0.25f;
        td.pan = std::sin(static_cast<float>(t));
        td.clips = clips[t].data();
        td.clip_count = clips_per_track;
    }
    se_session_desc sd{tds.data(), tracks, 0.5f};
    const std::string clap_gain = std::string(SAINE_TEST_PLUGIN_DIR) + "/SaineTestGain.clap";
    const std::string vst3_gain = std::string(SAINE_TEST_PLUGIN_DIR) + "/SaineTestGain.vst3";
    const std::string clap_latency = std::string(SAINE_TEST_PLUGIN_DIR) + "/SaineTestLatency.clap";
    const uint8_t latency_state[4] = {static_cast<uint8_t>(latency), static_cast<uint8_t>(latency >> 8),
                                      static_cast<uint8_t>(latency >> 16), static_cast<uint8_t>(latency >> 24)};
    se_pdc_set_enabled(e, pdc ? 1 : 0);
    std::vector<std::vector<se_device_desc>> devices(tracks);
    std::vector<se_track_desc_v5> t5(tracks);
    for (uint32_t t = 0; t < tracks && plugins != 0; ++t) {
        for (int k = 0; k < 2 && plugins != 3; ++k) {
            se_device_desc d{};
            d.device_id = 1000 + t * 2 + static_cast<uint32_t>(k);
            const bool vst3 = plugins == 1 && k == 1;
            d.format = vst3 ? SE_PLUGIN_VST3 : SE_PLUGIN_CLAP;
            d.plugin_id = vst3 ? "5A17E5A16E0000018A1E7E576A170001" : "studio.saine.test.gain";
            d.path = vst3 ? vst3_gain.c_str() : clap_gain.c_str();
            d.enabled = 1;
            devices[t].push_back(d);
        }
        if (plugins == 3 && t == 0) {
            se_device_desc d{};
            d.device_id = 1000;
            d.format = SE_PLUGIN_CLAP;
            d.plugin_id = "studio.saine.test.latency";
            d.path = clap_latency.c_str();
            d.state = latency_state;
            d.state_len = 4;
            d.enabled = 1;
            devices[t].push_back(d);
        }
        se_track_desc_v5& d = t5[t];
        d = se_track_desc_v5{};
        d.track_id = tds[t].track_id;
        d.gain = tds[t].gain;
        d.pan = tds[t].pan;
        d.clips = tds[t].clips;
        d.clip_count = tds[t].clip_count;
        d.devices = devices[t].data();
        d.device_count = static_cast<uint32_t>(devices[t].size());
    }
    se_session_desc_v5 sd5{t5.data(), tracks, 0.5f};
    if ((plugins == 0 ? se_session_publish(e, &sd) : se_session_publish_v5(e, &sd5)) != SE_OK) {
        std::fprintf(stderr, "publish failed: %s\n", se_last_error());
        std::exit(1);
    }
    if (plugins != 0) {
        size_t errors = 0;
        se_plugin_load_errors(e, nullptr, 0, &errors);
        if (errors != 0) {
            std::fprintf(stderr, "test plugins failed to load (built? %s)\n", SAINE_TEST_PLUGIN_DIR);
            std::exit(1);
        }
    }
    se_transport_play(e);
    if (features) {
        // Odd loop length so the wrap lands mid-block; 120 BPM click.
        se_transport_set_loop(e, 1, 1000, 1000 + 48001);
        se_metronome_desc m{};
        m.enabled = 1;
        m.gain = 0.5f;
        m.frames_per_beat = 24000.0;
        m.beats_per_bar = 4;
        se_metronome_set(e, &m);
    }

    std::vector<float> buf(kBlock * 2);
    for (int i = 0; i < 50; ++i) se_render_offline(e, buf.data(), kBlock); // warm-up
    se_transport_seek(e, 0);

    std::vector<double> times(blocks);
    const auto t_all = std::chrono::steady_clock::now();
    for (uint32_t b = 0; b < blocks; ++b) {
        const auto t0 = std::chrono::steady_clock::now();
        se_render_offline(e, buf.data(), kBlock);
        times[b] = std::chrono::duration<double, std::micro>(std::chrono::steady_clock::now() - t0).count();
    }
    const double wall_us = std::chrono::duration<double, std::micro>(std::chrono::steady_clock::now() - t_all).count();
    se_engine_destroy(e);

    std::vector<double> sorted = times;
    std::sort(sorted.begin(), sorted.end());
    Result r{};
    r.us_per_block_avg = wall_us / blocks;
    r.us_per_block_p99 = sorted[static_cast<std::size_t>(0.99 * (blocks - 1))];
    r.us_per_block_max = sorted.back();
    const double audio_us = static_cast<double>(blocks) * kBlock / kRate * 1e6;
    r.rt_ratio = audio_us / wall_us;
    return r;
}

Result run_midi(uint32_t tracks, uint32_t blocks, const se_synth_params* params) {
    se_config cfg{SE_ABI_VERSION, kRate, kBlock, 2};
    se_engine* e = se_engine_create(&cfg);
    if (!e) {
        std::fprintf(stderr, "create failed: %s\n", se_last_error());
        std::exit(1);
    }
    const int64_t total = static_cast<int64_t>(blocks) * kBlock;
    std::vector<std::vector<se_note_desc>> notes(tracks);
    std::vector<se_track_desc_v3> tds(tracks);
    for (uint32_t t = 0; t < tracks; ++t) {
        for (uint32_t v = 0; v < 8; ++v) {
            se_note_desc n{};
            n.start = static_cast<int64_t>(v) * 7; // staggered onsets
            n.end = total + 48000;
            n.pitch = static_cast<uint8_t>(36 + (t * 5 + v * 7) % 48);
            n.velocity = static_cast<uint8_t>(60 + v * 8);
            notes[t].push_back(n);
        }
        for (int64_t s = 4800; s < total; s += 4800) { // retriggered short note
            se_note_desc n{};
            n.start = s + t;
            n.end = s + t + 1200;
            n.pitch = static_cast<uint8_t>(72 + t % 12);
            n.velocity = 90;
            notes[t].push_back(n);
        }
        se_track_desc_v3& td = tds[t];
        td = se_track_desc_v3{};
        td.track_id = t + 1;
        td.gain = 0.25f;
        td.pan = std::sin(static_cast<float>(t));
        td.instrument = 1;
        td.notes = notes[t].data();
        td.note_count = static_cast<uint32_t>(notes[t].size());
        td.synth = params;
    }
    se_session_desc_v3 sd{tds.data(), tracks, 0.5f};
    if (se_session_publish_v3(e, &sd) != SE_OK) {
        std::fprintf(stderr, "publish failed: %s\n", se_last_error());
        std::exit(1);
    }
    se_transport_play(e);
    std::vector<float> buf(kBlock * 2);
    // No warm-up seek here: a seek would fade the held voices. Time from 0.
    std::vector<double> times(blocks);
    const auto t_all = std::chrono::steady_clock::now();
    for (uint32_t b = 0; b < blocks; ++b) {
        const auto t0 = std::chrono::steady_clock::now();
        se_render_offline(e, buf.data(), kBlock);
        times[b] = std::chrono::duration<double, std::micro>(std::chrono::steady_clock::now() - t0).count();
    }
    const double wall_us = std::chrono::duration<double, std::micro>(std::chrono::steady_clock::now() - t_all).count();
    se_engine_destroy(e);
    std::vector<double> sorted = times;
    std::sort(sorted.begin(), sorted.end());
    Result r{};
    r.us_per_block_avg = wall_us / blocks;
    r.us_per_block_p99 = sorted[static_cast<std::size_t>(0.99 * (blocks - 1))];
    r.us_per_block_max = sorted.back();
    r.rt_ratio = static_cast<double>(blocks) * kBlock / kRate * 1e6 / wall_us;
    return r;
}

// Input: `tracks` armed + monitored audio tracks (mono inputs spread over 8
// injected channels, every 3rd one stereo), optionally recording to a temp
// dir, rendered through se_render_offline_with_input.
Result run_input(uint32_t tracks, uint32_t blocks, bool record) {
    se_config cfg{SE_ABI_VERSION, kRate, kBlock, 2};
    se_engine* e = se_engine_create(&cfg);
    if (!e) {
        std::fprintf(stderr, "create failed: %s\n", se_last_error());
        std::exit(1);
    }
    constexpr uint32_t kInCh = 8;
    std::vector<se_track_desc_v4> tds(tracks);
    for (uint32_t t = 0; t < tracks; ++t) {
        se_track_desc_v4& td = tds[t];
        td = se_track_desc_v4{};
        td.track_id = t + 1;
        td.gain = 0.25f;
        td.pan = std::sin(static_cast<float>(t));
        td.arm = 1;
        td.monitor = 1;
        td.input_mode = t % 3 == 0 ? SE_INPUT_STEREO : SE_INPUT_MONO;
        td.input_channel = static_cast<uint16_t>(t % (kInCh - 1));
    }
    se_session_desc_v4 sd{tds.data(), tracks, 0.5f};
    if (se_session_publish_v4(e, &sd) != SE_OK) {
        std::fprintf(stderr, "publish failed: %s\n", se_last_error());
        std::exit(1);
    }
    std::vector<float> in(static_cast<std::size_t>(kBlock) * kInCh);
    for (std::size_t i = 0; i < in.size(); ++i) in[i] = 0.1f * std::sin(0.01f * static_cast<float>(i));
    const auto dir = std::filesystem::temp_directory_path() / "saine_bench_record";
    if (record && se_record_start(e, dir.string().c_str()) != SE_OK) {
        std::fprintf(stderr, "record failed: %s\n", se_last_error());
        std::exit(1);
    }
    se_transport_play(e);
    std::vector<float> buf(kBlock * 2);
    for (int i = 0; i < 50; ++i) se_render_offline_with_input(e, in.data(), kInCh, buf.data(), kBlock); // warm-up
    // Offline rendering runs thousands of times faster than real time, so
    // with recording on we pause (untimed) every 256 blocks (~1.4 s of
    // audio, under the 2 s ring) to let the writer thread catch up, and time
    // only the render calls. Otherwise every block would hit the (cheaper)
    // overrun path instead of the copy we want to measure.
    std::vector<double> times(blocks);
    double wall_us = 0.0;
    for (uint32_t b = 0; b < blocks; ++b) {
        if (record && b % 256 == 255) std::this_thread::sleep_for(std::chrono::milliseconds(30));
        const auto t0 = std::chrono::steady_clock::now();
        se_render_offline_with_input(e, in.data(), kInCh, buf.data(), kBlock);
        times[b] = std::chrono::duration<double, std::micro>(std::chrono::steady_clock::now() - t0).count();
        wall_us += times[b];
    }
    if (record) {
        std::size_t n = 0;
        se_record_stop(e, &n);
        se_record_stats st{};
        se_record_stats_get(e, &st);
        if (st.overruns != 0) std::printf("   (recording overruns: %llu)\n", static_cast<unsigned long long>(st.overruns));
        std::filesystem::remove_all(dir);
    }
    se_engine_destroy(e);
    std::vector<double> sorted = times;
    std::sort(sorted.begin(), sorted.end());
    Result r{};
    r.us_per_block_avg = wall_us / blocks;
    r.us_per_block_p99 = sorted[static_cast<std::size_t>(0.99 * (blocks - 1))];
    r.us_per_block_max = sorted.back();
    r.rt_ratio = static_cast<double>(blocks) * kBlock / kRate * 1e6 / wall_us;
    return r;
}

} // namespace

int main(int argc, char** argv) {
    const uint32_t blocks = argc > 1 ? static_cast<uint32_t>(std::atoi(argv[1])) : 4000;
    const uint32_t clips_per_track = argc > 2 ? static_cast<uint32_t>(std::atoi(argv[2])) : 4;
    const bool features = argc > 3 && std::atoi(argv[3]) != 0;

    // A handful of 20 s stereo assets (decoded once per engine).
    std::vector<std::string> files;
    const auto dir = std::filesystem::temp_directory_path();
    const std::size_t frames = static_cast<std::size_t>(kRate * 20);
    for (int a = 0; a < 4; ++a) {
        std::vector<float> l(frames), r(frames);
        for (std::size_t i = 0; i < frames; ++i) {
            const double t = static_cast<double>(i) / kRate;
            l[i] = static_cast<float>(0.3 * std::sin(2 * std::numbers::pi * (110.0 * (a + 1)) * t));
            r[i] = static_cast<float>(0.3 * std::sin(2 * std::numbers::pi * (165.0 * (a + 1)) * t));
        }
        const std::string path = (dir / ("saine_bench_asset_" + std::to_string(a) + ".wav")).string();
        if (!write_float_wav(path, l, r)) {
            std::fprintf(stderr, "cannot write %s\n", path.c_str());
            return 1;
        }
        files.push_back(path);
    }

    const double deadline_us = kBlock / kRate * 1e6;
    std::printf("saine_engine_bench: %u blocks x %u frames @ %.0f Hz (deadline %.1f us/block), %u clips/track%s\n",
                blocks, kBlock, kRate, deadline_us, clips_per_track, features ? ", loop + metronome" : "");
    std::printf("%8s %14s %14s %14s %16s\n", "tracks", "avg us/block", "p99 us/block", "max us/block",
                "realtime ratio");
    for (uint32_t tracks : {8u, 32u, 128u}) {
        const Result r = run(tracks, clips_per_track, blocks, files, features);
        std::printf("%8u %14.2f %14.2f %14.2f %15.1fx\n", tracks, r.us_per_block_avg, r.us_per_block_p99,
                    r.us_per_block_max, r.rt_ratio);
    }
    std::printf("\nPlugins: 32 audio tracks x %u clips + 2 test-gain inserts per track\n", clips_per_track);
    std::printf("%22s %14s %14s %14s %16s\n", "inserts", "avg us/block", "p99 us/block", "max us/block",
                "realtime ratio");
    for (int mode : {0, 1, 2}) {
        const Result r = run(32, clips_per_track, blocks, files, features, mode);
        const char* name = mode == 0 ? "none (baseline)" : mode == 1 ? "CLAP gain + VST3 gain" : "2 x CLAP gain";
        std::printf("%22s %14.2f %14.2f %14.2f %15.1fx\n", name, r.us_per_block_avg, r.us_per_block_p99,
                    r.us_per_block_max, r.rt_ratio);
    }
    std::printf("\nDelay compensation: 32 audio tracks x %u clips, one SaineTestLatency insert (N frames) on track 1\n",
                clips_per_track);
    std::printf("%22s %14s %14s %14s %16s\n", "N / PDC", "avg us/block", "p99 us/block", "max us/block",
                "realtime ratio");
    for (const uint32_t n : {0u, 4096u})
        for (const bool pdc : {false, true}) {
            const Result r = run(32, clips_per_track, blocks, files, features, 3, n, pdc);
            char name[32];
            std::snprintf(name, sizeof name, "N=%u, PDC %s", n, pdc ? "on" : "off");
            std::printf("%22s %14.2f %14.2f %14.2f %15.1fx\n", name, r.us_per_block_avg, r.us_per_block_p99,
                        r.us_per_block_max, r.rt_ratio);
        }
    for (const auto& f : files) std::filesystem::remove(f);

    // Expensive patch: 4-voice unison, cross FM, sub + noise, resonant
    // band-pass with filter envelope, LFO on pitch / cutoff / amplitude.
    se_synth_params heavy{};
    se_synth_params_default(&heavy);
    heavy.unison = 4;
    heavy.unison_detune_cents = 20.0f;
    heavy.cross_fm = 0.6f;
    heavy.osc1_wave = SE_WAVE_TRIANGLE;
    heavy.osc1_shape = 0.4f;
    heavy.osc2_wave = SE_WAVE_SINE;
    heavy.osc2_shape = 0.3f;
    heavy.sub_level = 0.4f;
    heavy.noise_level = 0.05f;
    heavy.filter_mode = SE_FILTER_BANDPASS;
    heavy.filter_resonance = 0.5f;
    heavy.filter_env_amount = 0.4f;
    heavy.lfo_to_pitch_cents = 10.0f;
    heavy.lfo_to_cutoff_oct = 0.5f;
    heavy.lfo_to_amp = 0.2f;
    const struct {
        const char* name;
        const se_synth_params* params;
    } patches[] = {{"default params", nullptr}, {"heavy patch: unison 4 + cross FM + sub/noise + LFO", &heavy}};
    for (const auto& patch : patches) {
        std::printf("\nMIDI (%s): instrument tracks x 8 held voices (+1 retriggered note / 100 ms), no clips\n",
                    patch.name);
        std::printf("%8s %14s %14s %14s %16s\n", "tracks", "avg us/block", "p99 us/block", "max us/block",
                    "realtime ratio");
        for (uint32_t tracks : {8u, 32u, 128u}) {
            const Result r = run_midi(tracks, blocks, patch.params);
            std::printf("%8u %14.2f %14.2f %14.2f %15.1fx\n", tracks, r.us_per_block_avg, r.us_per_block_p99,
                        r.us_per_block_max, r.rt_ratio);
        }
    }

    std::printf("\nInput: armed + monitored audio tracks, injected 8-channel input (no clips)\n");
    std::printf("%8s %10s %14s %14s %14s %16s\n", "tracks", "recording", "avg us/block", "p99 us/block",
                "max us/block", "realtime ratio");
    for (bool record : {false, true}) {
        const Result r = run_input(8, blocks, record);
        std::printf("%8u %10s %14.2f %14.2f %14.2f %15.1fx\n", 8u, record ? "on" : "off", r.us_per_block_avg,
                    r.us_per_block_p99, r.us_per_block_max, r.rt_ratio);
    }
    return 0;
}
