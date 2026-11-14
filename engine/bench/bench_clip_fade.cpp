// Clip fade benchmark: 32 audio tracks x 4 clips through se_render_offline
// (the device-callback process path) at 48 kHz / 256 frames, the clips
// without fades against the same clips with fades (se_session_publish_v11):
// half-second fades in and out (a few percent of the blocks fade), and fades
// over the whole clip (every block of every clip fades), 1:1 and Re-pitch
// warped (identity map). Prints measurements only; no pass/fail thresholds.
//
// usage: dstudio_clip_fade_bench [blocks=4000]
#include "platform/os.hpp"
#include "dstudio_engine.h"

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
constexpr uint32_t kTracks = 32;
constexpr uint32_t kClips = 4;

bool write_float_wav(const std::string& path, const std::vector<float>& l, const std::vector<float>& r) {
    std::FILE* f = dstudio::os::fopen_utf8(path.c_str(), "wb");
    if (!f) return false;
    const auto frames = static_cast<uint32_t>(l.size());
    const uint32_t data = frames * 8;
    auto u32 = [&](uint32_t v) { std::fwrite(&v, 4, 1, f); }; // little-endian host assumed
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
    double avg = 0.0, p99 = 0.0, max = 0.0;
};

// fade -1: no fades; 0: each clip fades over half its length in and out; else
// fade-in and fade-out frames. warped: Re-pitch along an identity map.
Result run(const std::string& file, uint32_t blocks, int64_t fade, bool warped) {
    se_config cfg{SE_ABI_VERSION, kRate, kBlock, 2};
    se_engine* e = se_engine_create(&cfg);
    if (!e) {
        std::fprintf(stderr, "create failed: %s\n", se_last_error());
        std::exit(1);
    }
    se_asset_info info{};
    if (se_asset_load_file(e, file.c_str(), &info) != SE_OK) {
        std::fprintf(stderr, "load failed: %s\n", se_last_error());
        std::exit(1);
    }
    // Back-to-back clips covering the whole rendered range, as in dstudio_engine_bench.
    const int64_t total = static_cast<int64_t>(blocks) * kBlock;
    const int64_t clip_len = total / kClips + 1024;
    std::vector<std::vector<se_clip_desc>> clips(kTracks);
    std::vector<se_track_desc_v6> tds(kTracks);
    std::vector<se_warp_point> points;
    std::vector<se_clip_warp_desc> warps;
    std::vector<se_clip_fade_desc> fades;
    points.reserve(2 * kTracks * kClips);
    for (uint32_t t = 0; t < kTracks; ++t) {
        for (uint32_t c = 0; c < kClips; ++c) {
            se_clip_desc cd{};
            cd.clip_id = t * kClips + c + 1;
            cd.asset_id = info.asset_id;
            cd.timeline_start = static_cast<int64_t>(c) * (total / kClips);
            cd.source_offset = (t * 131) % 1000;
            cd.length = clip_len;
            cd.gain = 0.5f;
            clips[t].push_back(cd);
            if (warped) {
                const auto at = static_cast<double>(cd.source_offset);
                points.push_back({cd.timeline_start, at});
                points.push_back({cd.timeline_start + clip_len, at + static_cast<double>(clip_len)});
                se_clip_warp_desc w{};
                w.clip_id = cd.clip_id;
                w.mode = SE_WARP_REPITCH;
                w.points = &points[points.size() - 2];
                w.point_count = 2;
                warps.push_back(w);
            }
            if (fade >= 0) {
                const int64_t f = fade == 0 ? clip_len / 2 : fade + t % 7;
                fades.push_back({cd.clip_id, f, f});
            }
        }
        se_track_desc_v6& td = tds[t];
        td = se_track_desc_v6{};
        td.track_id = t + 1;
        td.gain = 0.25f;
        td.pan = std::sin(static_cast<float>(t));
        td.clips = clips[t].data();
        td.clip_count = kClips;
    }
    se_session_desc_v11 d{};
    d.v10.v9.v8.v7.v6.tracks = tds.data();
    d.v10.v9.v8.v7.v6.track_count = kTracks;
    d.v10.v9.v8.v7.v6.master_gain = 0.5f;
    d.v10.v9.v8.v7.warps = warps.empty() ? nullptr : warps.data();
    d.v10.v9.v8.v7.warp_count = static_cast<uint32_t>(warps.size());
    d.fades = fades.empty() ? nullptr : fades.data();
    d.fade_count = static_cast<uint32_t>(fades.size());
    if (se_session_publish_v11(e, &d) != SE_OK) {
        std::fprintf(stderr, "publish failed: %s\n", se_last_error());
        std::exit(1);
    }
    se_transport_play(e);
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
    std::sort(times.begin(), times.end());
    return {wall_us / blocks, times[static_cast<std::size_t>(0.99 * (blocks - 1))], times.back()};
}

} // namespace

int main(int argc, char** argv) {
    const uint32_t blocks = argc > 1 ? static_cast<uint32_t>(std::atoi(argv[1])) : 4000;
    // One 20 s stereo asset.
    const std::size_t frames = static_cast<std::size_t>(kRate * 20);
    std::vector<float> l(frames), r(frames);
    for (std::size_t i = 0; i < frames; ++i) {
        const double t = static_cast<double>(i) / kRate;
        l[i] = static_cast<float>(0.3 * std::sin(2 * std::numbers::pi * 110.0 * t));
        r[i] = static_cast<float>(0.3 * std::sin(2 * std::numbers::pi * 165.0 * t));
    }
    const std::string path = dstudio::os::path_to_utf8(std::filesystem::temp_directory_path() / "dstudio_clip_fade_bench.wav");
    if (!write_float_wav(path, l, r)) {
        std::fprintf(stderr, "cannot write %s\n", path.c_str());
        return 1;
    }
    std::printf("dstudio_clip_fade_bench: %u tracks x %u clips, %u blocks x %u frames @ %.0f Hz (deadline %.1f us/block)\n",
                kTracks, kClips, blocks, kBlock, kRate, kBlock / kRate * 1e6);
    std::printf("%30s %14s %14s %14s\n", "clips", "avg us/block", "p99 us/block", "max us/block");
    const struct {
        const char* name;
        int64_t fade;
        bool warped;
    } cases[] = {{"1:1, no fades", -1, false},           {"1:1, 0.5 s fades", 24000, false},
                 {"1:1, whole-clip fades", 0, false},    {"Re-pitch, no fades", -1, true},
                 {"Re-pitch, 0.5 s fades", 24000, true}, {"Re-pitch, whole-clip fades", 0, true}};
    for (const auto& c : cases) {
        const Result res = run(path, blocks, c.fade, c.warped);
        std::printf("%30s %14.2f %14.2f %14.2f\n", c.name, res.avg, res.p99, res.max);
    }
    std::filesystem::remove(path);
    return 0;
}
