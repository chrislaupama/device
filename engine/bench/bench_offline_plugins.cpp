// Offline render benchmark for the clip menu's renders (Consolidate, Bounce, Export audio): a
// private engine renders 8 s (4 bars at 120 BPM) of an audio track in chunks of 4096 frames on a
// worker thread, with and without a plugin chain. The chain is prepared on the main thread
// (se_plugin_prepare, one device at a time) and torn down there (se_engine_destroy), so the table
// shows what the main thread is held for (the longest single call) next to what the worker spends.
// Prints measurements only; no pass/fail thresholds.
//
// usage: dstudio_offline_plugin_bench [runs=9]
#include "platform/os.hpp"
#include "dstudio_engine.h"

#include <algorithm>
#include <chrono>
#include <cmath>
#include <cstdio>
#include <cstdlib>
#include <deque>
#include <filesystem>
#include <numbers>
#include <string>
#include <thread>
#include <vector>

namespace {

constexpr double kRate = 48000.0;
constexpr uint32_t kChunk = 4096;
constexpr uint32_t kFrames = 8 * 48000;

using Clock = std::chrono::steady_clock;
double ms_since(Clock::time_point t) { return std::chrono::duration<double, std::milli>(Clock::now() - t).count(); }

struct Chain {
    const char* name;
    std::vector<se_device_desc> devices;
};

std::deque<std::string> g_paths; // the descriptors point into it

se_device_desc device(uint64_t id, uint8_t format, const char* plugin_id, const std::string& path_in) {
    const std::string& path = g_paths.emplace_back(path_in);
    se_device_desc d{};
    d.device_id = id;
    d.name = plugin_id;
    d.plugin_id = plugin_id;
    d.path = path.empty() ? nullptr : path.c_str();
    d.format = format;
    d.enabled = 1;
    return d;
}

struct Result {
    double prepare_total = 0, prepare_max = 0, worker = 0, destroy = 0;
};

Result run(const std::string& wav, const Chain& chain, bool detached_worker) {
    Result r;
    se_config cfg{};
    cfg.abi_version = SE_ABI_VERSION;
    cfg.sample_rate = kRate;
    cfg.max_block_size = 512;
    cfg.output_channels = 2;
    se_engine* e = se_engine_create(&cfg);
    for (const se_device_desc& d : chain.devices) {
        const auto t = Clock::now();
        if (se_plugin_prepare(e, &d, 0) != SE_OK) std::fprintf(stderr, "prepare failed: %s\n", se_last_error());
        const double ms = ms_since(t);
        r.prepare_total += ms;
        r.prepare_max = std::max(r.prepare_max, ms);
    }
    const auto t0 = Clock::now();
    std::thread worker([&] {
        if (detached_worker) se_plugin_set_detached(e, 1);
        se_asset_info info{};
        se_asset_load_file(e, wav.c_str(), &info);
        se_clip_desc clip{1, info.asset_id, 0, 0, static_cast<int64_t>(kFrames), 1.0f};
        se_track_desc_v5 t{};
        t.track_id = 1;
        t.gain = 1.0f;
        t.clips = &clip;
        t.clip_count = 1;
        t.devices = chain.devices.data();
        t.device_count = static_cast<uint32_t>(chain.devices.size());
        se_session_desc_v5 s{};
        s.tracks = &t;
        s.track_count = 1;
        s.master_gain = 1.0f;
        se_session_publish_v5(e, &s);
        se_transport_seek(e, 0);
        se_transport_play(e);
        std::vector<float> out(static_cast<std::size_t>(kChunk) * 2);
        for (uint32_t done = 0; done < kFrames; done += kChunk) se_render_offline(e, out.data(), std::min(kChunk, kFrames - done));
    });
    worker.join();
    r.worker = ms_since(t0);
    const auto t1 = Clock::now();
    se_engine_destroy(e);
    r.destroy = ms_since(t1);
    return r;
}

double median(std::vector<double> v) {
    std::sort(v.begin(), v.end());
    return v[v.size() / 2];
}

} // namespace

int main(int argc, char** argv) {
    const int runs = argc > 1 ? std::atoi(argv[1]) : 9;
    const std::string dir = DSTUDIO_TEST_PLUGIN_DIR;
    std::vector<float> l(kFrames);
    for (uint32_t i = 0; i < kFrames; ++i) l[i] = static_cast<float>(0.3 * std::sin(2 * std::numbers::pi * 220.0 * i / kRate));
    const std::string wav = dstudio::os::path_to_utf8(std::filesystem::temp_directory_path() / "dstudio_offline_plugin_bench.wav");
    {
        std::FILE* f = dstudio::os::fopen_utf8(wav.c_str(), "wb");
        if (!f) return 1;
        const uint32_t data = kFrames * 4;
        auto u32 = [&](uint32_t v) { std::fwrite(&v, 4, 1, f); };
        auto u16 = [&](uint16_t v) { std::fwrite(&v, 2, 1, f); };
        std::fwrite("RIFF", 1, 4, f); u32(36 + data); std::fwrite("WAVEfmt ", 1, 8, f); u32(16); u16(3); u16(1); u32(48000); u32(48000 * 4); u16(4); u16(32);
        std::fwrite("data", 1, 4, f); u32(data);
        std::fwrite(l.data(), 4, kFrames, f);
        std::fclose(f);
    }
    std::vector<Chain> chains = {{"no plugins", {}}};
    chains.push_back({"CLAP gain + VST3 gain", {device(10, SE_PLUGIN_CLAP, "studio.dstudio.test.gain", dir + "/DStudioTestGain.clap"), device(11, SE_PLUGIN_VST3, "5A17E5A16E0000018A1E7E576A170001", dir + "/DStudioTestGain.vst3")}});
#if defined(__APPLE__)
    chains.push_back({"AUMatrixReverb + AUDelay + AULowpass", {device(10, SE_PLUGIN_AU, "aufx mrev appl", ""), device(11, SE_PLUGIN_AU, "aufx dely appl", ""), device(12, SE_PLUGIN_AU, "aufx lpas appl", "")}});
#endif
    std::printf("dstudio_offline_plugin_bench: 8 s (4 bars at 120 BPM) rendered in %u-frame chunks on a worker, median of %d runs\n", kChunk, runs);
    std::printf("%-40s %14s %14s %12s %14s %12s\n", "chain", "prepare total", "prepare max", "worker", "destroy (main)", "x real time");
    for (const Chain& c : chains) {
        std::vector<double> pt, pm, w, d;
        run(wav, c, true); // warm-up (loads the bundles)
        for (int i = 0; i < runs; ++i) {
            const Result r = run(wav, c, true);
            pt.push_back(r.prepare_total);
            pm.push_back(r.prepare_max);
            w.push_back(r.worker);
            d.push_back(r.destroy);
        }
        std::printf("%-40s %11.2f ms %11.2f ms %9.1f ms %11.2f ms %11.0fx\n", c.name, median(pt), median(pm), median(w), median(d), 8000.0 / median(w));
    }
    std::filesystem::remove(wav);
    return 0;
}
