// Manual device / recording check.
//
//   record_check                         list devices (enumeration only: opens
//                                        nothing, never prompts for the mic)
//   SAINE_TEST_REAL_INPUT=1 record_check [seconds=2] [input_uid]
//                                        ALSO opens the default (or given)
//                                        input with the default output, records
//                                        ~2 s on one armed, unmonitored track
//                                        (nothing is played back, so no
//                                        feedback) and reports level + latency.
//
// Opening a real input shows the macOS microphone-permission prompt for the
// host application (Terminal, iTerm, ...) the first time. Automated tests
// never do this; run the real-input mode only by hand.
#include "saine_engine.h"

#include <chrono>
#include <cmath>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <filesystem>
#include <string>
#include <thread>
#include <vector>

#if defined(__APPLE__)
namespace saine {
long coreaudio_microphone_authorization();
}
#endif

namespace {

int list_devices() {
    std::size_t total = 0;
    if (se_device_list(nullptr, 0, &total) != SE_OK) {
        std::fprintf(stderr, "se_device_list failed: %s\n", se_last_error());
        return 1;
    }
    std::vector<se_device_info> devs(total);
    std::size_t n = 0;
    se_device_list(devs.data(), devs.size(), &n);
    std::printf("%zu audio device(s):\n", n);
    for (std::size_t i = 0; i < n && i < devs.size(); ++i) {
        const se_device_info& d = devs[i];
        std::printf("  %s%s %-40s in %2u  out %2u  uid=%s\n", d.is_default_input ? "I" : " ",
                    d.is_default_output ? "O" : " ", d.name, d.input_channels, d.output_channels, d.uid);
        std::printf("      rates:");
        for (uint32_t r = 0; r < d.sample_rate_count; ++r) std::printf(" %.0f", d.sample_rates[r]);
        std::printf("\n");
    }
    std::printf("  (I = default input, O = default output)\n");
#if defined(__APPLE__)
    static const char* kAuth[] = {"not determined", "restricted", "denied", "authorized"};
    const long a = saine::coreaudio_microphone_authorization();
    std::printf("microphone permission for this host app: %s\n", a >= 0 && a <= 3 ? kAuth[a] : "unknown");
#endif
    return 0;
}

int real_input(double seconds, const char* input_uid) {
    se_config cfg{SE_ABI_VERSION, 0.0 /* default output rate */, 512, 2};
    se_engine* e = se_engine_create(&cfg);
    if (e == nullptr) {
        std::fprintf(stderr, "create failed: %s\n", se_last_error());
        return 1;
    }
    se_device_config dc{};
    dc.input_enabled = 1;
    dc.input_uid = input_uid;
    if (se_device_configure(e, &dc) != SE_OK) {
        std::fprintf(stderr, "configure failed: %s\n", se_last_error());
        se_engine_destroy(e);
        return 1;
    }
    if (se_device_start(e) != SE_OK) {
        std::fprintf(stderr, "start failed: %s\n", se_last_error());
        se_engine_destroy(e);
        return 1;
    }
    se_device_status st{};
    se_device_status_get(e, &st);
    std::printf("output: %s\ninput:  %s%s%s\nrate %.0f Hz, buffer %u, in %u ch, latency in %u + out %u frames "
                "(%.2f ms round trip)\n",
                st.output_name, st.input_open ? st.input_name : "(none)", st.input_error[0] ? "  error: " : "",
                st.input_error, st.sample_rate, st.buffer_size, st.input_channels, st.input_latency_frames,
                st.output_latency_frames,
                1000.0 * (st.input_latency_frames + st.output_latency_frames) / se_engine_sample_rate(e));
    if (!st.input_open) {
        se_engine_destroy(e);
        return 1;
    }
    se_track_desc_v4 t{};
    t.track_id = 1;
    t.gain = 1.0f;
    t.arm = 1;     // armed, not monitored: metered, not heard
    t.input_mode = SE_INPUT_MONO;
    t.input_channel = 0;
    se_session_desc_v4 sd{&t, 1, 1.0f};
    se_session_publish_v4(e, &sd);

    const auto dir = std::filesystem::temp_directory_path() / "saine_record_check";
    if (se_record_start(e, dir.string().c_str()) != SE_OK) {
        std::fprintf(stderr, "record start failed: %s\n", se_last_error());
        se_engine_destroy(e);
        return 1;
    }
    const auto t0 = std::chrono::steady_clock::now();
    while (std::chrono::steady_clock::now() - t0 < std::chrono::duration<double>(seconds)) {
        std::this_thread::sleep_for(std::chrono::milliseconds(250));
        se_meter m[2]{};
        std::size_t n = 0;
        se_meters_peek(e, m, 2, &n);
        if (n == 2) std::printf("  input meter %.1f dBFS\n", 20.0 * std::log10(std::max(1e-9f, m[1].peak_l)));
    }
    std::size_t takes = 0;
    se_record_stop(e, &takes);
    se_transport_stop(e);
    se_record_stats rs{};
    se_record_stats_get(e, &rs);
    for (std::size_t i = 0; i < takes; ++i) {
        se_recorded_take tk{};
        se_record_get_take(e, i, &tk);
        se_asset_info info{};
        float peak = 0.0f;
        if (se_asset_load_file(e, tk.path, &info) == SE_OK) {
            std::vector<float> mm(2 * 4096);
            std::size_t pn = 0;
            const uint32_t fpp = static_cast<uint32_t>(std::max<uint64_t>(1, info.frames / 4096 + 1));
            se_asset_compute_peaks(e, info.asset_id, 0, fpp, mm.data(), 4096, &pn);
            for (std::size_t k = 0; k < pn; ++k) {
                peak = std::max(peak, std::max(std::fabs(mm[2 * k]), std::fabs(mm[2 * k + 1])));
            }
        }
        std::printf("take %zu: %s\n  start_frame %lld (latency compensated), %llu frames, %u ch, peak %.1f dBFS\n", i,
                    tk.path, static_cast<long long>(tk.start_frame), static_cast<unsigned long long>(tk.frames),
                    tk.channels, 20.0 * std::log10(std::max(1e-9f, peak)));
    }
    std::printf("overruns %llu, dropped frames %llu, write errors %llu\n",
                static_cast<unsigned long long>(rs.overruns), static_cast<unsigned long long>(rs.dropped_frames),
                static_cast<unsigned long long>(rs.write_errors));
    se_engine_destroy(e);
    return 0;
}

} // namespace

int main(int argc, char** argv) {
    const int rc = list_devices();
    const char* env = std::getenv("SAINE_TEST_REAL_INPUT");
    if (env == nullptr || std::strcmp(env, "1") != 0) {
        std::printf("\n(set SAINE_TEST_REAL_INPUT=1 to record from the real input; this opens the microphone)\n");
        return rc;
    }
    const double seconds = argc > 1 ? std::atof(argv[1]) : 2.0;
    return real_input(seconds > 0.0 ? seconds : 2.0, argc > 2 ? argv[2] : nullptr);
}
