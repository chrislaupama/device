// Manual device check: plays a generated sine (or a given WAV) through the
// default Core Audio output for ~1 s at a quiet level (-24 dBFS).
//
// usage: play_sine_or_wav [file.wav] [seconds=1.0]
#include "platform/os.hpp"
#include "saine_engine.h"

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

bool write_sine_wav(const std::string& path, double rate, double seconds, double freq, double amp) {
    std::FILE* f = saine::os::fopen_utf8(path.c_str(), "wb");
    if (!f) return false;
    const auto frames = static_cast<uint32_t>(rate * seconds);
    const uint32_t data = frames * 4;
    auto u32 = [&](uint32_t v) { std::fwrite(&v, 4, 1, f); };
    auto u16 = [&](uint16_t v) { std::fwrite(&v, 2, 1, f); };
    std::fwrite("RIFF", 1, 4, f);
    u32(36 + data);
    std::fwrite("WAVEfmt ", 1, 8, f);
    u32(16);
    u16(3); // IEEE float
    u16(1);
    u32(static_cast<uint32_t>(rate));
    u32(static_cast<uint32_t>(rate) * 4);
    u16(4);
    u16(32);
    std::fwrite("data", 1, 4, f);
    u32(data);
    const uint32_t fade = static_cast<uint32_t>(rate * 0.01); // 10 ms fades, no clicks
    for (uint32_t i = 0; i < frames; ++i) {
        double g = 1.0;
        if (i < fade) g = static_cast<double>(i) / fade;
        if (frames - i < fade) g = static_cast<double>(frames - i) / fade;
        const auto s = static_cast<float>(amp * g * std::sin(2.0 * std::numbers::pi * freq * i / rate));
        std::fwrite(&s, 4, 1, f);
    }
    return std::fclose(f) == 0;
}

} // namespace

int main(int argc, char** argv) {
    const double seconds = argc > 2 ? std::atof(argv[2]) : 1.0;
    se_config cfg{SE_ABI_VERSION, 0.0 /* use device rate */, 512, 2};
    se_engine* e = se_engine_create(&cfg);
    if (!e) {
        std::fprintf(stderr, "se_engine_create failed: %s\n", se_last_error());
        return 1;
    }
    const double rate = se_engine_sample_rate(e);
    std::printf("engine sample rate: %.0f Hz\n", rate);

    std::string path;
    bool generated = false;
    if (argc > 1) {
        path = argv[1];
    } else {
        path = saine::os::path_to_utf8(std::filesystem::temp_directory_path() / "saine_play_sine.wav");
        const double amp = std::pow(10.0, -24.0 / 20.0); // -24 dBFS
        if (!write_sine_wav(path, rate, seconds + 0.2, 440.0, amp)) {
            std::fprintf(stderr, "cannot write %s\n", path.c_str());
            return 1;
        }
        generated = true;
        std::printf("generated 440 Hz sine at -24 dBFS: %s\n", path.c_str());
    }

    se_asset_info info{};
    if (se_asset_load_file(e, path.c_str(), &info) != SE_OK) {
        std::fprintf(stderr, "load failed: %s\n", se_last_error());
        return 1;
    }
    std::printf("asset %llu: %llu frames, %u ch, source rate %.0f\n", static_cast<unsigned long long>(info.asset_id),
                static_cast<unsigned long long>(info.frames), info.channels, info.source_sample_rate);

    se_clip_desc clip{1, info.asset_id, 0, 0, static_cast<int64_t>(info.frames), 1.0f};
    // A user-supplied file is attenuated to -24 dB as well; pan centre (-3 dB)
    // is compensated so the generated sine reaches the device at -24 dBFS.
    const float file_gain = generated ? 1.0f : static_cast<float>(std::pow(10.0, -24.0 / 20.0));
    se_track_desc track{};
    track.track_id = 1;
    track.gain = file_gain * static_cast<float>(std::numbers::sqrt2);
    track.pan = 0.0f;
    track.clips = &clip;
    track.clip_count = 1;
    se_session_desc session{&track, 1, 1.0f};
    if (se_session_publish(e, &session) != SE_OK) {
        std::fprintf(stderr, "publish failed: %s\n", se_last_error());
        return 1;
    }
    se_transport_play(e);

    if (se_device_start(e) != SE_OK) {
        std::fprintf(stderr, "se_device_start failed: %s\n", se_last_error());
        se_engine_destroy(e);
        return 2;
    }
    std::printf("device running: %d\n", se_device_is_running(e));

    // Offline render must be refused while the device owns the RT path.
    float dummy[2];
    const int32_t offline_rc = se_render_offline(e, dummy, 1);
    std::printf("se_render_offline while running -> %d (expected %d)\n", offline_rc, SE_ERR_STATE);

    float max_peak = 0.0f;
    const auto until = std::chrono::steady_clock::now() + std::chrono::duration<double>(seconds);
    while (std::chrono::steady_clock::now() < until) {
        std::this_thread::sleep_for(std::chrono::milliseconds(100));
        se_meter m[2];
        size_t n = 0;
        se_meters_read(e, m, 2, &n);
        if (n > 0) max_peak = std::fmax(max_peak, std::fmax(m[0].peak_l, m[0].peak_r));
    }
    se_transport_info t{};
    se_transport_get(e, &t);
    se_stats st{};
    se_stats_get(e, &st);
    se_device_stop(e);

    std::printf("played %.3f s (position %lld frames)\n", static_cast<double>(t.position) / rate,
                static_cast<long long>(t.position));
    std::printf("master peak: %.1f dBFS\n", 20.0 * std::log10(std::fmax(max_peak, 1e-9f)));
    std::printf("callbacks=%llu overloads=%llu avg=%.2f us max=%.2f us last=%.2f us deadline=%.1f us\n",
                static_cast<unsigned long long>(st.callbacks), static_cast<unsigned long long>(st.xruns_or_overloads),
                st.avg_callback_us, st.max_callback_us, st.last_callback_us, st.block_deadline_us);
    se_engine_destroy(e);
    if (generated) std::filesystem::remove(path);
    const bool ok = st.callbacks > 0 && t.position > 0 && offline_rc == SE_ERR_STATE;
    std::printf("%s\n", ok ? "DEVICE CHECK OK" : "DEVICE CHECK FAILED");
    return ok ? 0 : 3;
}
