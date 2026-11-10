// MANUAL smoke test for plugin editor windows (needs a GUI login session;
// not part of CTest). Opens the editor of one plugin through the C API,
// pumps the event loop so the window is shown, then checks:
//   1. the window exists, is visible and titled "<name>: <track>";
//   2. closing it like the close button (performClose:) -> is_open == 0;
//   3. reopen + se_plugin_editor_close -> is_open == 0;
//   4. reopen + a republish without the device -> closed automatically.
//
//   plugin_editor_check                         AULowpass (AUGenericView)
//   plugin_editor_check au "aufx dely appl"
//   plugin_editor_check clap <bundle.clap> <plugin id>
//   plugin_editor_check vst3 <bundle.vst3> <class uid>
//   DSTUDIO_EDITOR_SECONDS=3 plugin_editor_check ...   keep each window up longer
#import <Cocoa/Cocoa.h>

#include "dstudio_engine.h"

#include <cstdio>
#include <cstdlib>
#include <string>

static void pump(double seconds) {
    NSDate* until = [NSDate dateWithTimeIntervalSinceNow:seconds];
    for (;;) {
        NSEvent* ev = [NSApp nextEventMatchingMask:NSEventMaskAny untilDate:until inMode:NSDefaultRunLoopMode dequeue:YES];
        if (ev == nil) break;
        [NSApp sendEvent:ev];
    }
}

static NSWindow* find_window(NSString* title) {
    for (NSWindow* w in [NSApp windows])
        if ([w.title isEqualToString:title] && w.isVisible) return w;
    return nil;
}

int main(int argc, char** argv) {
    @autoreleasepool {
        std::string fmt = argc > 1 ? argv[1] : "au";
        std::string path, id = "aufx lpas appl";
        if (fmt == "au" && argc > 2) id = argv[2];
        if ((fmt == "clap" || fmt == "vst3") && argc > 3) {
            path = argv[2];
            id = argv[3];
        } else if (fmt != "au") {
            std::fprintf(stderr, "usage: plugin_editor_check [au <id> | clap <bundle> <id> | vst3 <bundle> <uid>]\n");
            return 64;
        }
        const double secs = std::getenv("DSTUDIO_EDITOR_SECONDS") ? std::atof(std::getenv("DSTUDIO_EDITOR_SECONDS")) : 1.0;

        [NSApplication sharedApplication];
        [NSApp setActivationPolicy:NSApplicationActivationPolicyRegular];
        [NSApp finishLaunching];

        se_config cfg{SE_ABI_VERSION, 48000.0, 512, 2};
        se_engine* e = se_engine_create(&cfg);
        se_device_desc d{};
        d.device_id = 1;
        d.name = "Editor Check";
        d.plugin_id = id.c_str();
        d.path = path.empty() ? nullptr : path.c_str();
        d.format = fmt == "au" ? SE_PLUGIN_AU : fmt == "clap" ? SE_PLUGIN_CLAP : SE_PLUGIN_VST3;
        d.enabled = 1;
        se_track_desc_v5 t{};
        t.track_id = 1;
        t.gain = 1.0f;
        t.devices = &d;
        t.device_count = 1;
        t.track_name = "Audio 1";
        se_session_desc_v5 s{};
        s.tracks = &t;
        s.track_count = 1;
        s.master_gain = 1.0f;
        int failures = 0;
        const auto expect = [&](bool ok, const char* what) {
            std::printf("%s %s\n", ok ? "ok  " : "FAIL", what);
            if (!ok) ++failures;
        };
        if (se_session_publish_v5(e, &s) != SE_OK) {
            std::fprintf(stderr, "publish failed: %s\n", se_last_error());
            return 1;
        }
        se_plugin_message msg{};
        size_t n = 0;
        se_plugin_load_errors(e, &msg, 1, &n);
        if (n > 0) {
            std::fprintf(stderr, "load error: %s\n", msg.message);
            return 1;
        }
        NSString* title = @"Editor Check: Audio 1";

        int rc = se_plugin_editor_open(e, 1);
        if (rc != SE_OK) std::fprintf(stderr, "open: %s\n", se_last_error());
        expect(rc == SE_OK && se_plugin_editor_is_open(e, 1) == 1, "editor opens");
        pump(secs);
        NSWindow* w = find_window(title);
        expect(w != nil, "window visible with title \"Editor Check: Audio 1\"");
        if (w != nil)
            std::printf("     content %.0f x %.0f\n", w.contentView.frame.size.width, w.contentView.frame.size.height);
        [w performClose:nil]; // what the close button does
        pump(0.2);
        expect(se_plugin_editor_is_open(e, 1) == 0, "close button -> is_open false");
        expect(find_window(title) == nil, "window gone");

        expect(se_plugin_editor_open(e, 1) == SE_OK, "reopen");
        pump(secs);
        expect(se_plugin_editor_close(e, 1) == SE_OK && se_plugin_editor_is_open(e, 1) == 0, "se_plugin_editor_close");
        pump(0.2);
        expect(find_window(title) == nil, "window gone");

        expect(se_plugin_editor_open(e, 1) == SE_OK, "reopen");
        pump(secs);
        t.device_count = 0;
        expect(se_session_publish_v5(e, &s) == SE_OK, "republish without the device");
        expect(se_plugin_editor_is_open(e, 1) == 0, "editor of the retired instance closed");
        pump(0.2);
        expect(find_window(title) == nil, "window gone");

        se_engine_destroy(e);
        std::printf("%s\n", failures == 0 ? "PASS" : "FAILED");
        return failures == 0 ? 0 : 1;
    }
}
