// MIDI input (specs/003-midi-input-and-recording): routing to instrument
// tracks, held-note release, closing ports, capture positions, queue limits
// and RT safety. Input is injected (se_midi_inject) or comes from a fake
// backend; no MIDI hardware is needed.
#include "devices/device_harness.hpp"
#include "devices/synth_v2_params.hpp"
#include "midi_input.hpp"
#include "test_harness.hpp"
#include "test_hooks.hpp"
#include "test_util.hpp"

#include <algorithm>
#include <cmath>
#include <map>
#include <mutex>
#include <string>
#include <vector>

using namespace testutil;

namespace {

constexpr uint32_t kBlock = 256;

// Instrument tracks on the built-in synth (no instrument device), or with a
// built-in instrument device; pan -1 / +1 puts each on its own side.
struct Rig {
    struct T {
        se_track_desc_v6 d{};
        se_device_desc inst{};
        std::vector<uint8_t> state;
    };
    EngineHandle e{48000.0, kBlock};
    std::vector<T> tracks;

    T& add(uint64_t id, float pan, const char* builtin_instrument = nullptr) {
        T t;
        t.d.track_id = id;
        t.d.gain = 1.0f;
        t.d.pan = pan;
        t.d.instrument = 1;
        if (builtin_instrument != nullptr) {
            t.inst.device_id = id * 10;
            t.inst.plugin_id = builtin_instrument;
            t.inst.format = SE_PLUGIN_BUILTIN;
            t.inst.enabled = 1;
            t.state = devtest::state_blob(std::vector<float>(std::begin(dstudio::synth_v2::kDefaults),
                                                             std::end(dstudio::synth_v2::kDefaults)));
        }
        tracks.push_back(t);
        return tracks.back();
    }
    void publish() {
        std::vector<se_track_desc_v6> ds;
        for (T& t : tracks) {
            se_track_desc_v6 d = t.d;
            t.inst.state = t.state.empty() ? nullptr : t.state.data();
            t.inst.state_len = t.state.size();
            d.instrument_plugin = t.inst.plugin_id != nullptr ? &t.inst : nullptr;
            ds.push_back(d);
        }
        se_session_desc_v6 s{};
        s.tracks = ds.data();
        s.track_count = static_cast<uint32_t>(ds.size());
        s.master_gain = 1.0f;
        REQUIRE(se_session_publish_v6(e, &s) == SE_OK);
    }
    // Peak of the left and right channels over `frames`.
    std::pair<double, double> peaks(uint32_t frames) {
        const std::vector<float> out = render(e, frames, kBlock);
        double l = 0.0, r = 0.0;
        for (std::size_t i = 0; i + 1 < out.size(); i += 2) {
            l = std::max(l, static_cast<double>(std::fabs(out[i])));
            r = std::max(r, static_cast<double>(std::fabs(out[i + 1])));
        }
        return {l, r};
    }
};

// Fake backend: ports are added by the test; rx is called from the test thread.
class FakeMidi final : public dstudio::MidiBackend {
public:
    struct Port {
        Receive rx = nullptr;
        void* ctx = nullptr;
    };
    std::vector<dstudio::MidiPortInfo> present;
    std::map<std::string, Port> open_ports;
    std::mutex mutex;

    void list(std::vector<dstudio::MidiPortInfo>& out) override { out = present; }
    bool open(const std::string& id, Receive rx, void* ctx, std::string&) override {
        std::lock_guard lock(mutex);
        open_ports[id] = Port{rx, ctx};
        return true;
    }
    void close(const std::string& id) override {
        std::lock_guard lock(mutex);
        open_ports.erase(id);
    }
    bool send(const std::string& id, uint8_t s, uint8_t d1, uint8_t d2) {
        std::lock_guard lock(mutex);
        auto it = open_ports.find(id);
        if (it == open_ports.end()) return false;
        const uint8_t msg[3] = {s, d1, d2};
        it->second.rx(it->second.ctx, msg, 3);
        return true;
    }
};

} // namespace

TEST_CASE(midi_input_plays_the_focus_track_and_stops_on_note_off) {
    Rig rig;
    rig.add(1, 0.0f);
    rig.publish();
    REQUIRE(se_midi_inject(rig.e, 0x90, 60, 100) == SE_OK);
    CHECK(rig.peaks(4800).first < 1e-6); // no focus: nobody plays
    REQUIRE(se_midi_inject(rig.e, 0x80, 60, 0) == SE_OK);
    REQUIRE(se_midi_set_focus(rig.e, 1) == SE_OK);
    REQUIRE(se_midi_inject(rig.e, 0x92, 60, 100) == SE_OK); // any channel
    CHECK(rig.peaks(4800).first > 0.05);                    // plays while stopped
    REQUIRE(se_midi_inject(rig.e, 0x92, 60, 0) == SE_OK);   // note-on velocity 0 = note-off
    rig.peaks(96000);                                       // release tail
    CHECK(rig.peaks(4800).first < 1e-5);
}

TEST_CASE(midi_input_goes_to_armed_or_monitoring_tracks_over_the_focus) {
    Rig rig;
    rig.add(1, -1.0f);
    rig.add(2, 1.0f).d.arm = 1;
    rig.add(3, 1.0f);
    rig.publish();
    REQUIRE(se_midi_set_focus(rig.e, 1) == SE_OK);
    REQUIRE(se_midi_inject(rig.e, 0x90, 64, 100) == SE_OK);
    auto [l, r] = rig.peaks(4800);
    CHECK(l < 1e-6); // the focus track is overridden
    CHECK(r > 0.05); // the armed track plays
    // Monitoring counts too: track 3 (right) receives, track 2 is disarmed and released.
    rig.tracks[1].d.arm = 0;
    rig.tracks[2].d.monitor = 1;
    rig.publish();
    rig.peaks(96000); // track 2's release tail dies away first (same side as track 3)
    CHECK(rig.peaks(4800).second < 1e-5);
    REQUIRE(se_midi_inject(rig.e, 0x90, 67, 100) == SE_OK);
    std::tie(l, r) = rig.peaks(4800);
    CHECK(l < 1e-6);
    CHECK(r > 0.05);
}

TEST_CASE(midi_input_releases_held_notes_when_routing_moves) {
    Rig rig;
    rig.add(1, -1.0f);
    rig.add(2, 1.0f);
    rig.publish();
    REQUIRE(se_midi_set_focus(rig.e, 1) == SE_OK);
    REQUIRE(se_midi_inject(rig.e, 0x90, 60, 100) == SE_OK);
    CHECK(rig.peaks(4800).first > 0.05);
    REQUIRE(se_midi_set_focus(rig.e, 2) == SE_OK); // key still down
    rig.peaks(96000);
    auto [l, r] = rig.peaks(4800);
    CHECK(l < 1e-5); // released, not hanging
    CHECK(r < 1e-6); // the new track never got the note-on
    // Its note-off arrives later and goes nowhere audible.
    REQUIRE(se_midi_inject(rig.e, 0x80, 60, 0) == SE_OK);
    std::tie(l, r) = rig.peaks(4800);
    CHECK(r < 1e-6);
}

TEST_CASE(midi_input_plays_a_builtin_instrument) {
    Rig rig;
    rig.add(1, 0.0f, "synth-v2").d.monitor = 1;
    rig.publish();
    REQUIRE(se_midi_inject(rig.e, 0x90, 60, 110) == SE_OK);
    CHECK(rig.peaks(9600).first > 0.01);
    REQUIRE(se_midi_inject(rig.e, 0x80, 60, 0) == SE_OK);
    rig.peaks(5 * 48000);
    CHECK(rig.peaks(4800).first < 1e-4);
}

TEST_CASE(midi_input_closing_a_port_releases_its_notes) {
    Rig rig;
    auto fake = std::make_unique<FakeMidi>();
    FakeMidi* port = fake.get();
    port->present.push_back({"fake:1", "Fake Keys"});
    dstudio::testing::set_midi_backend(rig.e, std::move(fake));
    rig.add(1, 0.0f).d.arm = 1;
    rig.publish();
    se_midi_input_info info[4]{};
    size_t n = 0;
    REQUIRE(se_midi_inputs_list(rig.e, info, 4, &n) == SE_OK);
    REQUIRE(n == 1);
    CHECK(std::string(info[0].name) == "Fake Keys");
    CHECK(info[0].open == 0);
    const char* ids[] = {"fake:1", "not:present"};
    REQUIRE(se_midi_inputs_open(rig.e, ids, 2) == SE_OK); // absent ids are ignored
    REQUIRE(port->send("fake:1", 0x90, 72, 100));
    CHECK(rig.peaks(4800).first > 0.05);
    REQUIRE(se_midi_inputs_list(rig.e, info, 4, &n) == SE_OK);
    CHECK(info[0].open == 1);
    CHECK(info[0].events == 1);
    REQUIRE(se_midi_inputs_open(rig.e, nullptr, 0) == SE_OK); // unplugged / turned off with the key down
    CHECK(!port->send("fake:1", 0x80, 72, 0));
    rig.peaks(96000);
    CHECK(rig.peaks(4800).first < 1e-5);
    // The slot is free again: reopening works and plays.
    REQUIRE(se_midi_inputs_open(rig.e, ids, 1) == SE_OK);
    REQUIRE(port->send("fake:1", 0x90, 72, 100));
    CHECK(rig.peaks(4800).first > 0.05);
}

// The real platform backend (Core MIDI, WinMM, ALSA sequencer): listing and
// opening work, or give an empty list, on machines without a MIDI system
// (CI runners, containers); the engine still renders afterwards.
TEST_CASE(midi_input_platform_backend_lists_without_a_midi_system) {
    Rig rig;
    rig.add(1, 0.0f).d.monitor = 1;
    rig.publish();
    size_t total = 0;
    REQUIRE(se_midi_inputs_list(rig.e, nullptr, 0, &total) == SE_OK);
    std::vector<se_midi_input_info> info(total + 1);
    size_t n = 0;
    REQUIRE(se_midi_inputs_list(rig.e, info.data(), info.size(), &n) == SE_OK);
    CHECK(n <= info.size());
    for (size_t i = 0; i < std::min(n, info.size()); ++i) CHECK(info[i].id[0] != 0);
    const char* ids[] = {"no such MIDI input"};
    CHECK(se_midi_inputs_open(rig.e, ids, 1) == SE_OK); // absent ids are ignored
    CHECK(se_midi_inputs_open(rig.e, nullptr, 0) == SE_OK);
    REQUIRE(se_midi_inject(rig.e, 0x90, 60, 100) == SE_OK);
    CHECK(rig.peaks(4800).first > 0.05);
}

TEST_CASE(midi_input_drops_stray_note_offs_program_changes_and_overflow) {
    EngineHandle e(48000.0, kBlock);
    se_midi_stats st{};
    REQUIRE(se_midi_inject(e, 0x80, 60, 0) == SE_OK); // no matching note-on
    REQUIRE(se_midi_inject(e, 0xC0, 5, 0) == SE_OK);  // program change
    render(e, kBlock, kBlock);
    REQUIRE(se_midi_stats_get(e, &st) == SE_OK);
    CHECK(st.received == 2);
    CHECK(st.delivered == 0);
    CHECK(se_midi_inject(e, 0xF8, 0, 0) == SE_ERR_INVALID_ARG); // not a channel message
    int full = 0;
    for (int i = 0; i < 1100; ++i)
        if (se_midi_inject(e, 0xB0, 1, static_cast<uint8_t>(i & 0x7F)) == SE_ERR_STATE) ++full;
    CHECK(full == 1100 - static_cast<int>(dstudio::MidiInputHub::kQueueSize));
    REQUIRE(se_midi_stats_get(e, &st) == SE_OK);
    CHECK(st.dropped == static_cast<uint64_t>(full));
    // At most 256 a callback; the rest waits for the next ones.
    render(e, kBlock, kBlock);
    REQUIRE(se_midi_stats_get(e, &st) == SE_OK);
    CHECK(st.delivered == dstudio::MidiInputHub::kMaxBlockEvents);
    render(e, 4 * kBlock, kBlock);
    REQUIRE(se_midi_stats_get(e, &st) == SE_OK);
    CHECK(st.delivered == dstudio::MidiInputHub::kQueueSize);
}

TEST_CASE(midi_capture_records_note_positions_while_playing) {
    Rig rig;
    rig.add(1, 0.0f).d.arm = 1;
    rig.publish();
    REQUIRE(se_midi_capture_enable(rig.e, 1) == SE_OK);
    REQUIRE(se_midi_inject(rig.e, 0x90, 60, 90) == SE_OK); // stopped: heard, not captured
    render(rig.e, kBlock, kBlock);
    REQUIRE(se_transport_play(rig.e) == SE_OK);
    render(rig.e, 10 * kBlock, kBlock);
    REQUIRE(se_midi_inject(rig.e, 0x80, 60, 0) == SE_OK);
    REQUIRE(se_midi_inject(rig.e, 0x90, 62, 80) == SE_OK);
    render(rig.e, kBlock, kBlock); // block at 10 * kBlock (offline: no latency)
    REQUIRE(se_midi_inject(rig.e, 0x80, 62, 0) == SE_OK);
    render(rig.e, kBlock, kBlock);
    se_midi_captured c[8]{};
    size_t n = 0;
    REQUIRE(se_midi_capture_read(rig.e, c, 8, &n) == SE_OK);
    REQUIRE(n == 3);
    CHECK(c[0].status == 0x80 && c[0].data1 == 60 && c[0].frame == 10 * kBlock);
    CHECK(c[1].status == 0x90 && c[1].data1 == 62 && c[1].data2 == 80 && c[1].frame == 10 * kBlock);
    CHECK(c[2].status == 0x80 && c[2].frame == 11 * kBlock);
    REQUIRE(se_midi_capture_read(rig.e, c, 8, &n) == SE_OK);
    CHECK(n == 0);
    // Disabled: nothing is captured.
    REQUIRE(se_midi_capture_enable(rig.e, 0) == SE_OK);
    REQUIRE(se_midi_inject(rig.e, 0x90, 64, 80) == SE_OK);
    render(rig.e, kBlock, kBlock);
    REQUIRE(se_midi_capture_read(rig.e, c, 8, &n) == SE_OK);
    CHECK(n == 0);
}

namespace {
// Hub over a fake backend with `ports` ports "fake:0".."fake:N-1".
struct HubRig {
    FakeMidi* port = nullptr;
    std::unique_ptr<dstudio::MidiInputHub> hub;
    explicit HubRig(int ports) {
        auto fake = std::make_unique<FakeMidi>();
        port = fake.get();
        for (int i = 0; i < ports; ++i) port->present.push_back({"fake:" + std::to_string(i), "Fake " + std::to_string(i)});
        hub = std::make_unique<dstudio::MidiInputHub>(std::move(fake));
    }
    int32_t open(std::vector<std::string> ids, const dstudio::MidiInputHub::Reclaim& reclaim = {}) {
        std::string err;
        return hub->open_only(ids, err, reclaim);
    }
    std::vector<dstudio::MidiEvent> drain() {
        std::vector<dstudio::MidiEvent> out(dstudio::MidiInputHub::kMaxBlockEvents);
        out.resize(hub->drain(out.data(), static_cast<uint32_t>(out.size())));
        return out;
    }
};
bool is(const dstudio::MidiEvent& e, uint8_t status, uint8_t pitch) { return e.status == status && e.data1 == pitch; }
} // namespace

TEST_CASE(midi_input_same_key_on_two_ports_ends_with_the_last_release) {
    HubRig r(2);
    REQUIRE(r.open({"fake:0", "fake:1"}) == SE_OK);
    REQUIRE(r.port->send("fake:0", 0x90, 60, 100));
    REQUIRE(r.port->send("fake:1", 0x91, 60, 90)); // another port, another channel, same key
    auto ev = r.drain();
    REQUIRE(ev.size() == 2);
    CHECK(is(ev[0], 0x90, 60) && is(ev[1], 0x90, 60));
    REQUIRE(r.port->send("fake:0", 0x80, 60, 0));
    CHECK(r.drain().empty()); // still held on the other port
    REQUIRE(r.port->send("fake:1", 0x90, 60, 0)); // note-on velocity 0
    ev = r.drain();
    REQUIRE(ev.size() == 1);
    CHECK(is(ev[0], 0x80, 60) && ev[0].offset == 0);
}

TEST_CASE(midi_input_queue_overflow_releases_the_ports_keys) {
    HubRig r(0);
    REQUIRE(r.hub->inject(0x90, 60, 100));
    REQUIRE(r.drain().size() == 1);
    // Key 61 goes down, then a burst fills the queue and its note-off is lost.
    REQUIRE(r.hub->inject(0x90, 61, 100));
    for (std::size_t i = 1; i < dstudio::MidiInputHub::kQueueSize; ++i) REQUIRE(r.hub->inject(0xB0, 1, 0));
    CHECK(!r.hub->inject(0x80, 61, 0));
    CHECK(!r.hub->inject(0x80, 60, 0));
    std::vector<dstudio::MidiEvent> all;
    for (int i = 0; i < 8; ++i)
        for (const auto& e : r.drain()) all.push_back(e);
    REQUIRE(all.size() == dstudio::MidiInputHub::kQueueSize + 2);
    CHECK(is(all.front(), 0x90, 61)); // what was queued is delivered first...
    CHECK(is(all[all.size() - 2], 0x80, 60)); // ...then every key the port holds is released
    CHECK(is(all.back(), 0x80, 61));
    // Only once: the next note plays normally.
    REQUIRE(r.hub->inject(0x90, 62, 100));
    const auto ev = r.drain();
    REQUIRE(ev.size() == 1);
    CHECK(is(ev[0], 0x90, 62));
}

TEST_CASE(midi_input_reuses_a_closed_ports_slot_while_nothing_renders) {
    using Hub = dstudio::MidiInputHub;
    HubRig r(static_cast<int>(Hub::kDeviceSlots) + 1);
    std::vector<std::string> first, second;
    for (uint32_t i = 0; i < Hub::kDeviceSlots; ++i) first.push_back("fake:" + std::to_string(i));
    for (uint32_t i = 1; i <= Hub::kDeviceSlots; ++i) second.push_back("fake:" + std::to_string(i));
    REQUIRE(r.open(first) == SE_OK);
    REQUIRE(r.port->send("fake:0", 0x90, 64, 100));
    REQUIRE(r.drain().size() == 1);
    REQUIRE(r.port->send("fake:0", 0x90, 65, 100)); // still queued when the port closes
    // No renderer and no way to reclaim: the closed slot stays busy.
    CHECK(r.open(second) == SE_ERR_DEVICE);
    // With the RT role free, the slot is reclaimed at once; the held key's note-off waits for the next block.
    int reclaims = 0;
    CHECK(r.open(second, [&] {
        ++reclaims;
        r.hub->reclaim_closing();
        return true;
    }) == SE_OK);
    CHECK(reclaims == 1);
    const auto ev = r.drain();
    REQUIRE(ev.size() == 2);
    CHECK(is(ev[0], 0x80, 64) && is(ev[1], 0x80, 65));
    REQUIRE(r.port->send("fake:16", 0x90, 70, 100));
    const auto ev2 = r.drain();
    REQUIRE(ev2.size() == 1);
    CHECK(is(ev2[0], 0x90, 70));
}

TEST_CASE(midi_input_engine_frees_closed_slots_without_a_device) {
    using Hub = dstudio::MidiInputHub;
    Rig rig;
    auto fake = std::make_unique<FakeMidi>();
    FakeMidi* port = fake.get();
    for (uint32_t i = 0; i <= Hub::kDeviceSlots; ++i) port->present.push_back({"fake:" + std::to_string(i), "Fake"});
    dstudio::testing::set_midi_backend(rig.e, std::move(fake));
    rig.add(1, 0.0f).d.arm = 1;
    rig.publish();
    std::vector<const char*> ids;
    std::vector<std::string> names;
    for (uint32_t i = 0; i <= Hub::kDeviceSlots; ++i) names.push_back("fake:" + std::to_string(i));
    for (uint32_t i = 0; i < Hub::kDeviceSlots; ++i) ids.push_back(names[i].c_str());
    REQUIRE(se_midi_inputs_open(rig.e, ids.data(), ids.size()) == SE_OK);
    REQUIRE(port->send("fake:0", 0x90, 72, 100));
    CHECK(rig.peaks(4800).first > 0.05);
    ids.erase(ids.begin());
    ids.push_back(names[Hub::kDeviceSlots].c_str());
    REQUIRE(se_midi_inputs_open(rig.e, ids.data(), ids.size()) == SE_OK); // nothing rendered in between
    rig.peaks(96000);
    CHECK(rig.peaks(4800).first < 1e-5); // the closed port's note was released
}

TEST_CASE(midi_capture_skips_events_of_an_earlier_pass) {
    HubRig r(0);
    r.hub->capture_enable(true);
    const uint32_t first = r.hub->capture_pass();
    CHECK(first != 0);
    dstudio::MidiEvent on{};
    on.status = 0x90;
    on.data1 = 60;
    on.data2 = 100;
    r.hub->capture_enable(false);
    CHECK(r.hub->capture_pass() == 0);
    r.hub->capture_enable(true);
    const uint32_t second = r.hub->capture_pass();
    CHECK(second != first);
    // The audio thread read the pass before the restart and pushes late.
    r.hub->capture(on, 100, first);
    on.data1 = 62;
    r.hub->capture(on, 200, second);
    se_midi_captured c[4]{};
    REQUIRE(r.hub->capture_read(c, 4) == 1);
    CHECK(c[0].data1 == 62 && c[0].frame == 200);
    // Read after the pass stops: its events are still there.
    on.data1 = 64;
    r.hub->capture(on, 300, second);
    r.hub->capture_enable(false);
    REQUIRE(r.hub->capture_read(c, 4) == 1);
    CHECK(c[0].data1 == 64);
}

TEST_CASE(midi_capture_frame_compensates_latency_and_folds_into_the_loop) {
    using dstudio::midi_capture_frame;
    CHECK(midi_capture_frame(10000, 480, false, false, 0, 0) == 9520);
    CHECK(midi_capture_frame(100, 480, false, false, 0, 0) == -380); // before the start: the app trims
    // Loop 48000..96000: right after a wrap the performer still hears the end of the loop.
    CHECK(midi_capture_frame(48100, 480, true, true, 48000, 96000) == 95620);
    CHECK(midi_capture_frame(48100, 480, false, true, 48000, 96000) == 47620); // first pass: no fold
    CHECK(midi_capture_frame(60000, 480, true, true, 48000, 96000) == 59520);
    CHECK(midi_capture_frame(48000, 48000, true, true, 48000, 96000) == 48000);
}

TEST_CASE(midi_input_render_path_does_not_allocate) {
    Rig rig;
    rig.add(1, -1.0f);
    rig.add(2, 1.0f, "synth-v2");
    rig.publish();
    REQUIRE(se_midi_set_focus(rig.e, 1) == SE_OK);
    REQUIRE(se_midi_capture_enable(rig.e, 1) == SE_OK);
    REQUIRE(se_transport_play(rig.e) == SE_OK);
    std::vector<float> buf(2 * 4096);
    REQUIRE(se_render_offline(rig.e, buf.data(), kBlock) == SE_OK);
    const long before = rt_alloc_count();
    const uint32_t sizes[] = {256, 1, 333, 4096, 64};
    for (int i = 0; i < 300; ++i) {
        for (int k = 0; k < 4; ++k)
            REQUIRE(se_midi_inject(rig.e, static_cast<uint8_t>(i % 3 == 0 ? 0x80 : 0x90),
                                   static_cast<uint8_t>(48 + (i + k) % 24), 100) == SE_OK);
        if (i % 50 == 0) se_midi_set_focus(rig.e, i % 100 == 0 ? 2 : 1);
        RtAllocGuard g;
        REQUIRE(se_render_offline(rig.e, buf.data(), sizes[i % 5]) == SE_OK);
    }
    CHECK(rt_alloc_count() - before == 0);
    se_midi_captured c[64];
    size_t n = 0;
    REQUIRE(se_midi_capture_read(rig.e, c, 64, &n) == SE_OK);
    CHECK(n > 0);
}
