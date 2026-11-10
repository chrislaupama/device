// Clip warping benchmark: N tracks, each playing one warped stereo clip for the
// whole run, through se_render_offline (the device-callback process path) at
// 48 kHz / 256 frames. Prints measurements only; no pass/fail thresholds.
//
// usage: saine_warp_bench [blocks=4000]
//
// Table 1: per mode (and unwarped 1:1 playback for reference), the average,
// 99th-percentile and worst block time, the cost per warped track and block,
// and how many such tracks one core could run in real time, with the warped
// clips stretched live on the audio thread (background renders off). The map
// runs the source 7 % slower than written (a 120 BPM loop in a 112 BPM project).
// Table 2: the same for the stretch modes once their background renders are
// attached (the normal state a moment after an edit), plus the time those
// renders took on the background thread.
// Table 3: transport jumps every 50 blocks on 32 stretch tracks, live and
// rendered, which is the worst case for a single block.
// Table 4: onset detection time per minute of audio.
#include "platform/os.hpp"
#include "saine_engine.h"
#include "test_hooks.hpp"

#include <algorithm>
#include <chrono>
#include <cmath>
#include <cstdio>
#include <cstdlib>
#include <filesystem>
#include <string>
#include <thread>
#include <vector>

namespace {

constexpr double kRate = 48000.0;
constexpr uint32_t kBlock = 256;
constexpr double kSlope = 112.0 / 120.0;

bool write_float_wav(const std::string& path, const std::vector<float>& l, const std::vector<float>& r) {
    std::FILE* f = saine::os::fopen_utf8(path.c_str(), "wb");
    if (!f) return false;
    const auto frames = static_cast<uint32_t>(l.size());
    const uint32_t data = frames * 8;
    auto u32 = [&](uint32_t v) { std::fwrite(&v, 4, 1, f); };
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

// A loop-like test signal: chords plus decaying noise hits every 250 ms.
std::string make_loop(const std::string& dir, uint32_t seconds) {
    const std::size_t n = static_cast<std::size_t>(seconds) * 48000;
    std::vector<float> l(n), r(n);
    uint32_t rng = 1;
    for (std::size_t i = 0; i < n; ++i) {
        const double t = static_cast<double>(i) / kRate;
        double v = 0.1 * (std::sin(2 * 3.14159265 * 220 * t) + std::sin(2 * 3.14159265 * 277.2 * t) +
                          std::sin(2 * 3.14159265 * 329.6 * t));
        rng = rng * 1664525u + 1013904223u;
        const double noise = (static_cast<double>(rng >> 8) / 8388608.0 - 1.0);
        v += 0.4 * noise * std::exp(-static_cast<double>(i % 12000) / 900.0);
        l[i] = static_cast<float>(v);
        r[i] = static_cast<float>(0.9 * v);
    }
    const std::string path = dir + "/warp_bench_loop.wav";
    if (!write_float_wav(path, l, r)) std::exit(1);
    return path;
}

struct Result {
    double avg = 0.0, p99 = 0.0, max = 0.0, render_ms = 0.0;
};

// Waits for the background renders and attaches them; returns the wait in ms.
double attach_renders(se_engine* e) {
    const auto t0 = std::chrono::steady_clock::now();
    for (;;) {
        std::size_t n = 0;
        se_warp_renders_pending(e, &n);
        if (n == 0) break;
        std::this_thread::sleep_for(std::chrono::milliseconds(1));
    }
    se_plugin_idle(e);
    return std::chrono::duration<double, std::milli>(std::chrono::steady_clock::now() - t0).count();
}

// mode < 0: unwarped. rendered: wait for the background renders before
// measuring, else stretch live. restart_every > 0: seek every that many blocks.
Result run(se_engine* e, uint64_t asset, uint32_t tracks, int mode, uint32_t blocks, bool rendered,
           uint32_t restart_every = 0) {
    static uint32_t salt = 0;
    ++salt;
    const int64_t total = static_cast<int64_t>(blocks) * kBlock;
    std::vector<se_clip_desc> clips(tracks);
    std::vector<se_track_desc_v6> tds(tracks);
    std::vector<se_warp_point> pts[2];
    std::vector<se_warp_slice> slices;
    for (int64_t s = 0; s < 20 * 48000; s += 12000) slices.push_back(se_warp_slice{static_cast<double>(s), static_cast<uint8_t>((s / 12000) % 4 != 3), {}});
    std::vector<std::vector<se_warp_point>> points(tracks);
    std::vector<se_clip_warp_desc> warps;
    for (uint32_t t = 0; t < tracks; ++t) {
        clips[t] = se_clip_desc{t + 1, asset, 0, (t * 131) % 1000, total, 0.5f};
        const double slope = kSlope + 1e-6 * static_cast<double>(salt);
        points[t] = {{0, static_cast<double>((t * 131) % 1000)},
                     {total, static_cast<double>((t * 131) % 1000) + static_cast<double>(total) * slope}};
        tds[t] = se_track_desc_v6{};
        tds[t].track_id = t + 1;
        tds[t].gain = 0.25f;
        tds[t].pan = std::sin(static_cast<float>(t));
        tds[t].clips = &clips[t];
        tds[t].clip_count = 1;
        if (mode >= 0) {
            se_clip_warp_desc w{};
            w.clip_id = t + 1;
            w.mode = static_cast<uint8_t>(mode);
            w.points = points[t].data();
            w.point_count = 2;
            w.slices = slices.data();
            w.slice_count = static_cast<uint32_t>(slices.size());
            warps.push_back(w);
        }
    }
    saine::testing::set_warp_renders_enabled(e, rendered);
    se_session_desc_v7 d{};
    d.v6 = se_session_desc_v6{tds.data(), tracks, 0.5f, nullptr, 0, 0};
    d.warps = warps.empty() ? nullptr : warps.data();
    d.warp_count = static_cast<uint32_t>(warps.size());
    if (se_session_publish_v7(e, &d) != SE_OK) {
        std::fprintf(stderr, "publish failed: %s\n", se_last_error());
        std::exit(1);
    }
    Result r;
    if (rendered) r.render_ms = attach_renders(e);
    se_transport_seek(e, 0);
    se_transport_play(e);
    std::vector<float> buf(2 * kBlock);
    for (int i = 0; i < 64; ++i) se_render_offline(e, buf.data(), kBlock); // warm up
    se_transport_seek(e, 0);
    std::vector<double> times;
    times.reserve(blocks);
    for (uint32_t b = 0; b < blocks; ++b) {
        if (restart_every > 0 && b % restart_every == 0) se_transport_seek(e, static_cast<int64_t>(b) * kBlock);
        const auto t0 = std::chrono::steady_clock::now();
        se_render_offline(e, buf.data(), kBlock);
        times.push_back(std::chrono::duration<double, std::micro>(std::chrono::steady_clock::now() - t0).count());
    }
    se_transport_stop(e);
    for (double t : times) r.avg += t;
    r.avg /= static_cast<double>(times.size());
    std::sort(times.begin(), times.end());
    r.p99 = times[times.size() * 99 / 100];
    r.max = times.back();
    return r;
}

const char* mode_name(int mode) {
    switch (mode) {
    case -1: return "unwarped 1:1";
    case SE_WARP_REPITCH: return "Re-pitch";
    case SE_WARP_SLICE: return "Drums (slice)";
    case SE_WARP_STRETCH: return "Melody (stretch)";
    case SE_WARP_STRETCH_FORMANT: return "Vocal (stretch+formant)";
    default: return "Auto (3/4 percussive)";
    }
}

} // namespace

int main(int argc, char** argv) {
    const uint32_t blocks = argc > 1 ? static_cast<uint32_t>(std::max(1, std::atoi(argv[1]))) : 4000;
    const std::string dir = std::filesystem::temp_directory_path().string();
    const std::string file = make_loop(dir, 20);
    se_config cfg{SE_ABI_VERSION, kRate, kBlock, 2};
    se_engine* e = se_engine_create(&cfg);
    se_asset_info info{};
    if (!e || se_asset_load_file(e, file.c_str(), &info) != SE_OK) {
        std::fprintf(stderr, "setup failed: %s\n", se_last_error());
        return 1;
    }
    const double budget = 1e6 * kBlock / kRate;
    std::printf("Warped clip render, %u blocks of %u frames at 48 kHz (budget %.0f us per block), stereo source\n\n",
                blocks, kBlock, budget);
    std::printf("| mode | tracks | avg us/block | p99 | max | us per track | tracks per core |\n|---|---:|---:|---:|---:|---:|---:|\n");
    for (int mode : {-1, int{SE_WARP_REPITCH}, int{SE_WARP_SLICE}, int{SE_WARP_STRETCH}, int{SE_WARP_STRETCH_FORMANT},
                     int{SE_WARP_AUTO}})
        for (uint32_t tracks : {1u, 8u, 32u, 64u}) {
            const Result r = run(e, info.asset_id, tracks, mode, blocks, false);
            std::printf("| %s | %u | %.1f | %.1f | %.1f | %.2f | %.0f |\n", mode_name(mode), tracks, r.avg, r.p99, r.max,
                        r.avg / tracks, budget / (r.avg / tracks));
        }
    const double clip_s = static_cast<double>(blocks) * kBlock / kRate;
    std::printf("\nStretch modes with background renders attached (render: wall time on the background thread, "
                "%.1f s clips)\n\n", clip_s);
    std::printf("| mode | tracks | avg us/block | p99 | max | us per track | render ms | render ms per track-minute |\n"
                "|---|---:|---:|---:|---:|---:|---:|---:|\n");
    for (int mode : {int{SE_WARP_STRETCH}, int{SE_WARP_STRETCH_FORMANT}, int{SE_WARP_AUTO}})
        for (uint32_t tracks : {1u, 8u, 32u, 64u}) {
            const Result r = run(e, info.asset_id, tracks, mode, blocks, true);
            std::printf("| %s | %u | %.1f | %.1f | %.1f | %.2f | %.0f | %.0f |\n", mode_name(mode), tracks, r.avg, r.p99,
                        r.max, r.avg / tracks, r.render_ms, r.render_ms / tracks * 60.0 / clip_s);
        }
    std::printf("\nTransport jumps (Melody, 32 tracks, a jump every 50 blocks)\n\n");
    std::printf("| case | avg us/block | p99 | max |\n|---|---:|---:|---:|\n");
    for (bool rendered : {false, true}) {
        const Result r = run(e, info.asset_id, 32, SE_WARP_STRETCH, blocks, rendered, 50);
        std::printf("| %s | %.1f | %.1f | %.1f |\n", rendered ? "rendered" : "live", r.avg, r.p99, r.max);
    }
    const auto t0 = std::chrono::steady_clock::now();
    std::size_t count = 0;
    se_asset_detect_onsets(e, info.asset_id, nullptr, 0, &count);
    const double ms = std::chrono::duration<double, std::milli>(std::chrono::steady_clock::now() - t0).count();
    std::printf("\nOnset detection: %zu onsets in 20 s of audio, %.1f ms (%.1f ms per minute)\n", count, ms, ms * 3.0);
    se_engine_destroy(e);
    std::filesystem::remove(file);
    return 0;
}
