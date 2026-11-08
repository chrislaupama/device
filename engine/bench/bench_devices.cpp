// Built-in device benchmark: per-block DSP cost of every registered device
// (and every compressor character), the parameter update paths, and a
// 32-track session with two compressors per track.
//
//   saine_device_bench [blocks]      (Release build; numbers in us per 256-frame block)
#include "devices/builtin.hpp"
#include "platform/os.hpp"
#include "saine_engine.h"

#include <algorithm>
#include <chrono>
#include <cmath>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <filesystem>
#include <string>
#include <vector>

using Clock = std::chrono::steady_clock;

namespace {

constexpr double kRate = 48000.0;
constexpr uint32_t kBlock = 256;
const std::vector<float> kCompressor = {-24.f, 4.f, 10.f, 100.f, 6.f, 0.f, 100.f, 0.f, 0.f, 0.f, 0.f};
// Defaults of the time / pitch devices (Rust definitions, registry order).
const std::vector<float> kDelay = {120.f, 8.f, 50.f, 0.f, 0.0692f, 0.f, 100.f, 50.f, 0.f,
                                   20.f, 0.f, 0.71f, 0.f, 150.f, 0.f, 1.f, 1.f, 500.f, 0.f, 1.f, 1.f,
                                   3000.f, 0.f, 1.f, 1.f, 8000.f, 0.f, 1.f, 1.f, 20000.f, 0.f, 0.71f, 0.f,
                                   1.f, 1.f, 500.f, 1.f, 8.f, 500.f};
const std::vector<float> kReverb = {50.f, 2.f, 50.f, 10.f, 30.f, 100.f, 1.f, 12000.f, 20.f, 1.f, 20.f, 1.f, 7.f, 10.f, 60.f, 50.f};
const std::vector<float> kPitch = {7.f, 0.f, 0.f, 100.f};
const std::vector<float> kGrains = {0.f, 0.f, 250.f, 3.f, 120.f, 100.f, 24.f, 8.f, 100.f, 40.f, 0.f, 0.f, 4.f,
                                    0.f, 50.f, 50.f, 0.f, 0.f, 0.f, 0.f, 0.f, 0.f, 50.f, 0.f, 0.f};
std::vector<float> echo_defaults() {
    std::vector<float> v = {0.f, 3.f, 1.f, 0.f, 11.f, 0.5f, 1.f, 0.f, 10.f, 12.f, 5.f, 35.f, 0.f, 0.f, 0.f, 5.f, 5.f,
                            0.f, 1.f, 0.f, 0.f, 100.f, 45.f, 0.45f, 0.f, 25.f, 100.f, 100.f, 100.f, 0.f, 100.f, 0.f,
                            1.f, 20.f, 6200.f, 240.f, 20000.f, 0.f, 60.f, -24.f, 5.f, 180.f, 0.f, 0.f, 0.f, 6.f,
                            20673.f, 0.f, 0.f, 0.f, 0.f};
    for (int h = 0; h < 6; ++h)
        for (float x : {0.f, 0.f, 1.f, 0.f, 0.f, 0.f, 0.f, 0.f, 0.f, 1.f, 0.f}) v.push_back(x);
    v.push_back(0.f);
    return v;
}

std::vector<uint8_t> blob(const std::vector<float>& v, uint64_t sc = 0) {
    se_builtin_state h{SE_BUILTIN_STATE_MAGIC, static_cast<uint32_t>(v.size()), sc};
    std::vector<uint8_t> out(sizeof h + 4 * v.size());
    std::memcpy(out.data(), &h, sizeof h);
    std::memcpy(out.data() + sizeof h, v.data(), 4 * v.size());
    return out;
}

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

struct Stats {
    double avg = 0, p99 = 0, max = 0;
};

Stats summarize(std::vector<double>& t) {
    std::sort(t.begin(), t.end());
    Stats s;
    for (double x : t) s.avg += x;
    s.avg /= static_cast<double>(t.size());
    s.p99 = t[static_cast<std::size_t>(0.99 * static_cast<double>(t.size() - 1))];
    s.max = t.back();
    return s;
}

double us(Clock::duration d) { return std::chrono::duration<double, std::micro>(d).count(); }

Stats bench_kernel(const char* id, std::vector<float> values, uint32_t blocks, uint32_t events_per_block) {
    saine::PluginSpec spec;
    spec.device_id = 1;
    spec.format = saine::PluginFormat::Builtin;
    spec.plugin_id = id;
    spec.state = blob(values);
    std::string err;
    auto inst = saine::create_builtin_instance(spec, kRate, kBlock, err);
    if (!inst) {
        std::fprintf(stderr, "%s: %s\n", id, err.c_str());
        std::exit(1);
    }
    std::vector<float> l(kBlock), r(kBlock);
    uint32_t seed = 7;
    std::vector<saine::ParamEvent> ev;
    for (uint32_t i = 0; i < events_per_block; ++i)
        ev.push_back(saine::ParamEvent{i * (kBlock / std::max(1u, events_per_block)), 0, -30.0 + i});
    saine::RtTransport tr;
    std::vector<double> t;
    t.reserve(blocks);
    for (uint32_t b = 0; b < blocks + 200; ++b) {
        for (uint32_t i = 0; i < kBlock; ++i) {
            seed = seed * 1664525u + 1013904223u;
            const float n = static_cast<float>(seed >> 8) / 8388608.0f - 1.0f;
            l[i] = 0.8f * n;
            r[i] = 0.7f * n;
        }
        const auto t0 = Clock::now();
        inst->run(l.data(), r.data(), kBlock, nullptr, 0, tr, ev.data(), static_cast<uint32_t>(ev.size()));
        const auto t1 = Clock::now();
        if (b >= 200) t.push_back(us(t1 - t0));
    }
    return summarize(t);
}

} // namespace

int main(int argc, char** argv) {
    const uint32_t blocks = argc > 1 ? static_cast<uint32_t>(std::atoi(argv[1])) : 20000;
    const double deadline = 1e6 * kBlock / kRate;
    std::printf("saine_device_bench: %u blocks x %u frames @ %.0f Hz (deadline %.1f us/block), stereo noise in\n\n", blocks,
                kBlock, kRate, deadline);
    std::printf("%-34s %12s %12s %12s %14s\n", "device", "avg us", "p99 us", "max us", "x real-time");
    const auto row = [&](const char* name, Stats s) {
        std::printf("%-34s %12.3f %12.3f %12.3f %13.0fx\n", name, s.avg, s.p99, s.max, deadline / s.avg);
    };
    const char* styles[] = {"Clean", "Opto", "Punch", "Smooth"};
    for (int st = 0; st < 4; ++st) {
        std::vector<float> v = kCompressor;
        v[7] = static_cast<float>(st);
        char name[64];
        std::snprintf(name, sizeof name, "compressor (%s)", styles[st]);
        row(name, bench_kernel("compressor", v, blocks, 0));
    }
    row("compressor (Clean, 16 auto events)", bench_kernel("compressor", kCompressor, blocks, 16));

    // Time and pitch devices, one row per mode / machine (value index, value).
    struct Variant {
        const char* id;
        const char* name;
        std::vector<float> values;
        int index;
        float value;
    };
    const Variant variants[] = {
        {"delay", "delay (Tape)", kDelay, 8, 0.f},
        {"delay", "delay (Digital)", kDelay, 8, 1.f},
        {"delay", "delay (BBD)", kDelay, 8, 2.f},
        {"echo", "echo (Tape Loop)", echo_defaults(), 0, 0.f},
        {"echo", "echo (Drum)", echo_defaults(), 0, 1.f},
        {"echo", "echo (Cartridge)", echo_defaults(), 0, 2.f},
        {"reverb", "reverb (Room)", kReverb, 6, 0.f},
        {"reverb", "reverb (Hall)", kReverb, 6, 1.f},
        {"reverb", "reverb (Plate)", kReverb, 6, 2.f},
        {"reverb", "reverb (Spring)", kReverb, 6, 3.f},
        {"reverb", "reverb (Shimmer)", kReverb, 6, 4.f},
        {"reverb", "reverb (Haze)", kReverb, 6, 5.f},
        {"pitch-shifter", "pitch-shifter (+7 st)", kPitch, 2, 0.f},
        {"pitch-shifter", "pitch-shifter (+7 st, formant)", kPitch, 2, 1.f},
        {"grains", "grains (Cloud)", kGrains, 0, 0.f},
        {"grains", "grains (Rhythm)", kGrains, 0, 1.f},
    };
    for (const auto& v : variants) {
        std::vector<float> values = v.values;
        values[static_cast<std::size_t>(v.index)] = v.value;
        row(v.name, bench_kernel(v.id, values, blocks, 0));
    }

    // Parameter update paths and session cost: 32 tracks, one clip and two
    // compressors each.
    const std::string wav = (std::filesystem::temp_directory_path() / "saine_device_bench.wav").string();
    {
        std::vector<float> l(8 * 48000), r(8 * 48000);
        uint32_t seed = 3;
        for (std::size_t i = 0; i < l.size(); ++i) {
            seed = seed * 1664525u + 1013904223u;
            l[i] = 0.5f * (static_cast<float>(seed >> 8) / 8388608.0f - 1.0f);
            r[i] = 0.9f * l[i];
        }
        if (!write_float_wav(wav, l, r)) {
            std::fprintf(stderr, "cannot write %s\n", wav.c_str());
            return 1;
        }
    }
    se_config cfg{SE_ABI_VERSION, kRate, kBlock, 2};
    se_engine* e = se_engine_create(&cfg);
    if (!e) {
        std::fprintf(stderr, "create failed: %s\n", se_last_error());
        return 1;
    }
    se_asset_info info{};
    if (se_asset_load_file(e, wav.c_str(), &info) != SE_OK) {
        std::fprintf(stderr, "load failed: %s\n", se_last_error());
        return 1;
    }
    const auto state = blob(kCompressor);
    std::vector<se_clip_desc> clips(32);
    std::vector<se_device_desc> devs(64);
    std::vector<se_track_desc_v5> tracks(32);
    for (uint32_t t = 0; t < 32; ++t) {
        clips[t] = se_clip_desc{};
        clips[t].clip_id = t + 1;
        clips[t].asset_id = info.asset_id;
        clips[t].source_offset = (t * 131) % 1000;
        clips[t].length = 7 * 48000;
        clips[t].gain = 1.0f;
        for (uint32_t k = 0; k < 2; ++k) {
            se_device_desc& d = devs[2 * t + k];
            d = se_device_desc{};
            d.device_id = 1000 + 2 * t + k;
            d.plugin_id = "compressor";
            d.format = SE_PLUGIN_BUILTIN;
            d.enabled = 1;
            d.state = state.data();
            d.state_len = state.size();
        }
        se_track_desc_v5& td = tracks[t];
        td = se_track_desc_v5{};
        td.track_id = t + 1;
        td.gain = 0.25f;
        td.clips = &clips[t];
        td.clip_count = 1;
        td.devices = &devs[2 * t];
        td.device_count = 2;
    }
    se_session_desc_v5 s{tracks.data(), 32, 0.5f};
    if (se_session_publish_v5(e, &s) != SE_OK) {
        std::fprintf(stderr, "publish failed: %s\n", se_last_error());
        return 1;
    }
    std::printf("\nParameter updates (main thread, 64 live compressors)\n");
    {
        const int n = 200000;
        const auto t0 = Clock::now();
        for (int i = 0; i < n; ++i)
            se_builtin_set_param(e, 1000 + static_cast<uint64_t>(i & 63), 0, -20.0f - static_cast<float>(i & 7));
        std::printf("  %-32s %10.3f us/call\n", "se_builtin_set_param", us(Clock::now() - t0) / n);
    }
    {
        const int n = 2000;
        const auto t0 = Clock::now();
        for (int i = 0; i < n; ++i) se_session_publish_v5(e, &s);
        std::printf("  %-32s %10.3f us/publish\n", "republish (unchanged chain)", us(Clock::now() - t0) / n);
    }

    std::printf("\nSession: 32 audio tracks x 1 clip, offline render\n");
    std::printf("%-34s %12s %12s %12s %14s\n", "inserts", "avg us", "p99 us", "max us", "x real-time");
    std::vector<float> buf(2 * kBlock);
    const uint32_t session_blocks = std::min(blocks, 1200u);
    for (int with = 1; with >= 0; --with) {
        for (auto& td : tracks) td.device_count = with ? 2 : 0;
        se_session_publish_v5(e, &s);
        se_transport_seek(e, 0);
        se_transport_play(e);
        std::vector<double> t;
        for (uint32_t b = 0; b < session_blocks + 50; ++b) {
            const auto t0 = Clock::now();
            se_render_offline(e, buf.data(), kBlock);
            const auto t1 = Clock::now();
            if (b >= 50) t.push_back(us(t1 - t0));
        }
        row(with ? "2 compressors per track (64)" : "none (same session)", summarize(t));
    }
    se_engine_destroy(e);
    std::filesystem::remove(wav);
    return 0;
}
