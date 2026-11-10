// Manual check of the Linux (X11) plugin editor host with a real plugin:
// loads it as an insert, opens its editor, runs se_plugin_idle at ~60 Hz (as
// the app's UI timer does) for a few seconds, then closes the editor through
// the API.
//
//   plugin_editor_check_linux clap|vst3 <bundle path> <plugin id> [seconds=5]
//
// Find the id with `saine_plugin_scanner --scan-one clap|vst3 <path>`. Needs an
// X server (DISPLAY); under Xvfb, take a screenshot meanwhile with
// `import -window root shot.png`.
#include "saine_engine.h"

#include <chrono>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <thread>

int main(int argc, char** argv) {
    if (argc < 4) {
        std::fprintf(stderr, "usage: %s clap|vst3 <bundle path> <plugin id> [seconds=5]\n", argv[0]);
        return 2;
    }
    const bool vst3 = std::strcmp(argv[1], "vst3") == 0;
    const double seconds = argc > 4 ? std::atof(argv[4]) : 5.0;
    se_config cfg{};
    cfg.abi_version = SE_ABI_VERSION;
    cfg.sample_rate = 48000.0;
    cfg.max_block_size = 256;
    cfg.output_channels = 2;
    se_engine* e = se_engine_create(&cfg);
    if (e == nullptr) return 1;
    se_device_desc dev{};
    dev.device_id = 1;
    dev.name = "Plugin";
    dev.plugin_id = argv[3];
    dev.path = argv[2];
    dev.format = vst3 ? SE_PLUGIN_VST3 : SE_PLUGIN_CLAP;
    dev.enabled = 1;
    se_track_desc_v5 t{};
    t.track_id = 1;
    t.gain = 1.0f;
    t.devices = &dev;
    t.device_count = 1;
    t.track_name = "Audio 1";
    se_session_desc_v5 s{&t, 1, 1.0f};
    if (se_session_publish_v5(e, &s) != SE_OK) {
        std::fprintf(stderr, "publish failed: %s\n", se_last_error());
        return 1;
    }
    if (se_plugin_editor_open(e, 1) != SE_OK) {
        std::fprintf(stderr, "editor open failed: %s\n", se_last_error());
        return 1;
    }
    std::printf("editor open\n");
    std::fflush(stdout);
    const auto end = std::chrono::steady_clock::now() + std::chrono::duration<double>(seconds);
    long idles = 0;
    while (std::chrono::steady_clock::now() < end && se_plugin_editor_is_open(e, 1) == 1) {
        se_plugin_idle(e);
        ++idles;
        std::this_thread::sleep_for(std::chrono::milliseconds(16));
    }
    const bool user_closed = se_plugin_editor_is_open(e, 1) == 0;
    se_plugin_editor_close(e, 1);
    std::printf("%s after %ld idles\n", user_closed ? "closed by the user" : "closed", idles);
    se_engine_destroy(e);
    return 0;
}
