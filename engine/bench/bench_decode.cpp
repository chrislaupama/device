// WAV import benchmark: decode_wav_memory and decode_audio_file on a 60 s stereo 24-bit file
// written as plain RIFF, RF64 and BW64 (ds64 sizes, data size 0xFFFFFFFF). Prints measurements
// only; no pass/fail thresholds.
//
// usage: saine_decode_bench [reps=20]
#include "platform/os.hpp"
#include "saine_engine.h"
#include "wav.hpp"

#include <algorithm>
#include <chrono>
#include <cmath>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <filesystem>
#include <string>
#include <vector>

namespace {

void put32(std::vector<uint8_t>& b, uint32_t v) {
    for (int i = 0; i < 4; ++i) b.push_back(static_cast<uint8_t>(v >> (8 * i)));
}
void put64(std::vector<uint8_t>& b, uint64_t v) {
    for (int i = 0; i < 8; ++i) b.push_back(static_cast<uint8_t>(v >> (8 * i)));
}
void tag(std::vector<uint8_t>& b, const char* t) { b.insert(b.end(), t, t + 4); }

std::vector<uint8_t> make(const char* kind, uint32_t frames) {
    const bool wide = std::strcmp(kind, "RIFF") != 0;
    const uint64_t data = static_cast<uint64_t>(frames) * 6;
    std::vector<uint8_t> b;
    tag(b, kind);
    put32(b, wide ? 0xFFFFFFFFu : static_cast<uint32_t>(4 + 24 + 8 + data));
    tag(b, "WAVE");
    if (wide) {
        tag(b, "ds64");
        put32(b, 28);
        put64(b, 4 + 36 + 24 + 8 + data);
        put64(b, data);
        put64(b, frames);
        put32(b, 0);
    }
    tag(b, "fmt ");
    put32(b, 16);
    b.push_back(1); b.push_back(0); b.push_back(2); b.push_back(0);
    put32(b, 48000);
    put32(b, 48000 * 6);
    b.push_back(6); b.push_back(0); b.push_back(24); b.push_back(0);
    tag(b, "data");
    put32(b, wide ? 0xFFFFFFFFu : static_cast<uint32_t>(data));
    for (uint32_t f = 0; f < frames; ++f) {
        const double t = f / 48000.0;
        for (double hz : {220.0, 330.0}) {
            const int32_t s = static_cast<int32_t>(std::lround(std::sin(2.0 * M_PI * hz * t) * 0.5 * 8388607.0));
            b.push_back(static_cast<uint8_t>(s));
            b.push_back(static_cast<uint8_t>(s >> 8));
            b.push_back(static_cast<uint8_t>(s >> 16));
        }
    }
    return b;
}

double median(std::vector<double> v) {
    std::sort(v.begin(), v.end());
    return v[v.size() / 2];
}

} // namespace

int main(int argc, char** argv) {
    const int reps = argc > 1 ? std::max(1, std::atoi(argv[1])) : 20;
    const uint32_t frames = 60 * 48000;
    std::printf("WAV import, 60 s stereo 24-bit 48 kHz (%.1f MB), median of %d runs\n\n", frames * 6 / 1e6, reps);
    std::printf("| container | decode_wav_memory ms | decode_audio_file ms (read + decode) |\n|---|---:|---:|\n");
    for (const char* kind : {"RIFF", "RF64", "BW64"}) {
        const std::vector<uint8_t> bytes = make(kind, frames);
        const auto path = std::filesystem::temp_directory_path() / (std::string("saine_decode_bench_") + kind + ".wav");
        const std::string p8 = saine::os::path_to_utf8(path);
        if (std::FILE* f = std::fopen(p8.c_str(), "wb")) {
            std::fwrite(bytes.data(), 1, bytes.size(), f);
            std::fclose(f);
        }
        std::vector<double> mem, file;
        bool ok = true;
        for (int i = 0; i < reps; ++i) {
            saine::DecodedAudio d;
            std::string err;
            auto t0 = std::chrono::steady_clock::now();
            ok &= saine::decode_wav_memory(bytes.data(), bytes.size(), d, err) == SE_OK && d.frames == frames;
            mem.push_back(std::chrono::duration<double, std::milli>(std::chrono::steady_clock::now() - t0).count());
            t0 = std::chrono::steady_clock::now();
            ok &= saine::decode_audio_file(p8.c_str(), d, err) == SE_OK && d.frames == frames;
            file.push_back(std::chrono::duration<double, std::milli>(std::chrono::steady_clock::now() - t0).count());
        }
        std::filesystem::remove(path);
        if (!ok) {
            std::printf("| %s | unsupported | unsupported |\n", kind);
            continue;
        }
        std::printf("| %s | %.2f | %.2f |\n", kind, median(mem), median(file));
    }
    return 0;
}
