// Offline render benchmark: N tracks x M clips through se_render_offline
// (the device-callback process path) at 48 kHz / 256 frames.
// Prints measurements only; no pass/fail thresholds.
//
// usage: saine_engine_bench [blocks=4000] [clips_per_track=4] [features=0]
//   features=1: also enable a 48 001-frame loop (odd length, so the wrap
//   lands mid-block) and the metronome at 120 BPM.
#include "saine_engine.h"

#include <algorithm>
#include <chrono>
#include <cmath>
#include <cstdio>
#include <cstdlib>
#include <filesystem>
#include <numbers>
#include <string>
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

Result run(uint32_t tracks, uint32_t clips_per_track, uint32_t blocks, const std::vector<std::string>& files,
           bool features) {
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
    if (se_session_publish(e, &sd) != SE_OK) {
        std::fprintf(stderr, "publish failed: %s\n", se_last_error());
        std::exit(1);
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
    for (const auto& f : files) std::filesystem::remove(f);
    return 0;
}
