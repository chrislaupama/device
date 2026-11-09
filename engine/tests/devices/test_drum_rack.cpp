// Drum Rack and Pad Sampler: note-to-pad mapping, round-robin layers, pad
// switch, transpose, gate, start / end, warp, readouts.
#include "device_harness.hpp"
#include "sampler_defaults.hpp"

#include <string>

using namespace testutil;
using namespace devtest;

namespace {

constexpr double kRate = 48000.0;

std::size_t tail(const Signal& s) {
    for (std::size_t i = s.size(); i-- > 0;)
        if (std::fabs(s[i]) > 1e-3f) return i;
    return 0;
}

TestSample dc(uint32_t slot, std::size_t n, float v) {
    TestSample t;
    t.slot = slot;
    t.l.assign(n, v);
    return t;
}

std::string pid(int pad, const char* id) { return "p" + std::to_string(pad) + "_" + id; }

} // namespace

TEST_CASE(drum_rack_maps_notes_36_to_51_onto_pads) {
    DeviceRig rig("drum-rack", drum_rack_defaults(), kRate, 256);
    // Pad 1 (slot 0) holds +0.25, pad 2 (slot 8) +0.5, pad 16 (slot 120) -0.5.
    rig.load_samples({dc(0, 4800, 0.25f), dc(8, 4800, 0.5f), dc(120, 4800, -0.5f)});
    auto [a, a2] = rig.play(6000, {note_on(0, 36, 127)});
    CHECK_NEAR(a[2400], 0.25f, 0.01f);
    auto [b, b2] = rig.play(6000, {note_on(0, 37, 127)});
    CHECK_NEAR(b[2400], 0.5f, 0.01f);
    auto [c, c2] = rig.play(6000, {note_on(0, 51, 127)});
    CHECK_NEAR(c[2400], -0.5f, 0.01f);
    auto [d, d2] = rig.play(6000, {note_on(0, 52, 127), note_on(0, 35, 127), note_on(0, 38, 127)}); // empty pad 3 and unmapped notes
    CHECK(peak(d) == 0.0);
    // Velocity scales the hit.
    auto [v, v2] = rig.play(6000, {note_on(0, 36, 64)});
    CHECK(v[2400] < 0.1f);
    // Readouts: pad 1 active with a playhead in its layer 0.
    rig.play(1200, {note_on(0, 36, 127)});
    auto ro = rig.readouts();
    CHECK(ro.size() == 48);
    CHECK(ro[0] > 0.5f);                    // lvl0
    CHECK(ro[1] > 0.1f && ro[1] < 0.4f);    // pos0: 1200 of 4800 frames
    CHECK(ro[2] == 0.0f);                   // layer0
    CHECK(ro[3] == 0.0f && ro[4] == -1.0f); // pad 2 idle
}

TEST_CASE(drum_rack_layers_alternate_and_pads_switch_off) {
    DeviceRig rig("drum-rack", drum_rack_defaults(), kRate, 256);
    rig.load_samples({dc(0, 2400, 0.2f), dc(1, 2400, 0.6f)});
    auto [l, r] = rig.play(12000, {note_on(0, 36, 127), note_on(4000, 36, 127), note_on(8000, 36, 127)});
    CHECK_NEAR(l[1200], 0.2f, 0.01f);
    CHECK_NEAR(l[5200], 0.6f, 0.01f);
    CHECK_NEAR(l[9200], 0.2f, 0.01f);
    rig.set(pid(1, "on").c_str(), 0.0f);
    auto [o, o2] = rig.play(6000, {note_on(0, 36, 127)});
    CHECK(peak(o) == 0.0);
}

TEST_CASE(drum_rack_pad_settings_shape_each_hit) {
    DeviceRig rig("drum-rack", drum_rack_defaults(), kRate, 256);
    rig.load_samples({dc(0, 9600, 0.5f)});
    // Transpose +12 plays twice as fast (re-pitch).
    rig.set(pid(1, "transpose").c_str(), 12.0f);
    auto [t, t2] = rig.play(12000, {note_on(0, 36, 127)});
    CHECK_NEAR(static_cast<double>(tail(t)), 4800.0, 200.0);
    rig.set(pid(1, "transpose").c_str(), 0.0f);
    // Gate on: the note-off stops the sound; off (default): the hit plays out.
    auto [h, h2] = rig.play(12000, {note_on(0, 36, 127), note_off(1000, 36)});
    CHECK(tail(h) > 9000);
    rig.set(pid(1, "gate").c_str(), 1.0f);
    auto [g, g2] = rig.play(12000, {note_on(0, 36, 127), note_off(1000, 36)});
    CHECK(tail(g) < 1000 + 2400);
    rig.set(pid(1, "gate").c_str(), 0.0f);
    // Start / End clip the region; fades and gain apply.
    rig.set(pid(1, "start").c_str(), 0.5f);
    auto [s, s2] = rig.play(12000, {note_on(0, 36, 127)});
    CHECK_NEAR(static_cast<double>(tail(s)), 4800.0, 100.0);
    rig.set(pid(1, "start").c_str(), 0.0f);
    rig.set(pid(1, "gain").c_str(), -6.0f);
    rig.set(pid(1, "fadeIn").c_str(), 50.0f);
    auto [f, f2] = rig.play(12000, {note_on(0, 36, 127)});
    CHECK(f[1200] < 0.15f);                                  // a quarter into a half-length fade in
    CHECK_NEAR(db(f[6000]), db(0.5) - 6.0, 0.3);
    // Pan hard left keeps the left side and silences the right.
    rig.set(pid(1, "pan").c_str(), -1.0f);
    auto [pl, pr] = rig.play(12000, {note_on(0, 36, 127)});
    CHECK(std::fabs(pl[6000]) > 0.2f && std::fabs(pr[6000]) < 1e-4f);
}

TEST_CASE(drum_rack_warp_stretches_a_hit_to_a_note_length) {
    DeviceRig rig("drum-rack", drum_rack_defaults(), kRate, 256);
    TestSample t;
    t.slot = 0;
    t.l = tone(12000, 500.0, kRate, 0.5);
    rig.load_samples({t});
    rig.set("p1_warp", 4.0f); // eighth note at 120 BPM = 0.25 s = 12000 frames... at 60 BPM 24000
    auto [l, r] = rig.play(36000, {note_on(0, 36, 127)}, 60.0);
    CHECK_NEAR(static_cast<double>(tail(l)), 24000.0, 2500.0);
}

TEST_CASE(pad_sampler_answers_every_note) {
    DeviceRig rig("pad-sampler", pad_defaults(), kRate, 256);
    rig.load_samples({dc(0, 4800, 0.3f), dc(3, 4800, -0.3f)});
    auto [l, r] = rig.play(12000, {note_on(0, 20, 127), note_on(6000, 100, 127)});
    CHECK_NEAR(l[2400], 0.3f, 0.01f);
    CHECK_NEAR(l[8400], -0.3f, 0.01f); // the next layer (round robin)
    rig.set("reverse", 1.0f);
    rig.set("end", 0.5f);
    auto [e, e2] = rig.play(6000, {note_on(0, 60, 127)});
    CHECK(tail(e) < 2500);
    auto ro = rig.readouts();
    CHECK(ro.size() == 3);
}
