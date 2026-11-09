// Sampler (sampler-steam): pitch tracking, warp, reverse, play modes, loop,
// slices, mono legato, filter, envelope, band-limited pitching, live notes
// and sample hand-over through the engine.
#include "device_harness.hpp"
#include "sampler_defaults.hpp"

using namespace testutil;
using namespace devtest;

namespace {

constexpr double kRate = 48000.0;

// Zero-crossing frequency estimate over [from, to).
double freq(const Signal& s, std::size_t from, std::size_t to) {
    int n = 0;
    std::size_t first = 0, last = 0;
    for (std::size_t i = from + 1; i < std::min(to, s.size()); ++i) {
        if (s[i - 1] < 0.0f && s[i] >= 0.0f) {
            if (n == 0) first = i;
            last = i;
            ++n;
        }
    }
    return n > 1 ? (n - 1) * kRate / static_cast<double>(last - first) : 0.0;
}

// Last frame above -60 dBFS.
std::size_t tail(const Signal& s) {
    for (std::size_t i = s.size(); i-- > 0;)
        if (std::fabs(s[i]) > 1e-3f) return i;
    return 0;
}

DeviceRig sampler_with(const Signal& l, std::vector<se_builtin_slice> slices = {}) {
    DeviceRig rig("sampler-steam", sampler_defaults(), kRate, 256);
    TestSample t;
    t.l = l;
    t.slices = std::move(slices);
    rig.load_samples({t});
    return rig;
}

} // namespace

TEST_CASE(sampler_is_silent_without_a_sample_and_plays_with_one) {
    DeviceRig empty("sampler-steam", sampler_defaults(), kRate, 256);
    auto [el, er] = empty.play(9600, {note_on(0, 60)});
    CHECK(peak(el) == 0.0 && peak(er) == 0.0);
    auto rig = sampler_with(tone(24000, 440.0, kRate, 0.5));
    auto [l, r] = rig.play(24000, {note_on(0, 60)});
    CHECK(peak(l, 1000, 20000) > 0.4);
    CHECK_NEAR(freq(l, 2000, 20000), 440.0, 2.0);
    CHECK(peak(r, 1000, 20000) > 0.4); // mono sample on both sides
}

TEST_CASE(sampler_tracks_pitch_from_the_root_key) {
    auto rig = sampler_with(tone(48000, 440.0, kRate, 0.5));
    auto [l, r] = rig.play(48000, {note_on(0, 72)});
    CHECK_NEAR(freq(l, 1000, 20000), 880.0, 3.0);
    // Re-pitch: an octave up plays twice as fast, so it ends after ~0.5 s.
    CHECK_NEAR(static_cast<double>(tail(l)), 24000.0, 400.0);
    rig.set("root", 48.0f);
    auto [l2, r2] = rig.play(48000, {note_on(0, 48)});
    CHECK_NEAR(freq(l2, 1000, 20000), 440.0, 2.0);
    rig.set("transpose", -12.0f);
    rig.set("fineTune", 0.0f);
    auto [l3, r3] = rig.play(48000, {note_on(0, 48)});
    CHECK_NEAR(freq(l3, 1000, 20000), 220.0, 1.5);
}

TEST_CASE(sampler_warp_keeps_length_and_changes_pitch) {
    auto rig = sampler_with(tone(48000, 440.0, kRate, 0.5));
    rig.set("warp", 3.0f); // quarter note: 0.5 s at 120 BPM
    auto [l, r] = rig.play(72000, {note_on(0, 72)}, 120.0);
    CHECK_NEAR(freq(l, 4000, 20000), 880.0, 6.0);
    CHECK_NEAR(static_cast<double>(tail(l)), 24000.0, 2500.0);
    auto [l2, r2] = rig.play(72000, {note_on(0, 60)}, 60.0); // 1 s at 60 BPM
    CHECK_NEAR(freq(l2, 4000, 40000), 440.0, 4.0);
    CHECK_NEAR(static_cast<double>(tail(l2)), 48000.0, 2500.0);
    // A steady tone stays steady through the grain seams (no dropouts).
    double lo = 1.0;
    for (std::size_t w = 4000; w + 960 < 40000; w += 960) lo = std::min(lo, rms(l2, w, w + 960));
    CHECK(lo > 0.25);
}

TEST_CASE(sampler_reverse_plays_backwards) {
    Signal ramp(4800);
    for (std::size_t i = 0; i < ramp.size(); ++i) ramp[i] = 0.1f + 0.8f * static_cast<float>(i) / 4800.0f;
    auto rig = sampler_with(ramp);
    auto [fw, fr] = rig.play(6000, {note_on(0, 60)});
    rig.set("reverse", 1.0f);
    auto [rv, rr] = rig.play(6000, {note_on(0, 60)});
    CHECK(fw[200] < 0.2f && fw[4500] > 0.8f);
    CHECK(rv[200] > 0.8f && rv[4500] < 0.2f);
}

TEST_CASE(sampler_one_shot_trigger_gate_and_toggle) {
    auto rig = sampler_with(tone(24000, 300.0, kRate, 0.5));
    rig.set("ampRelease", 10.0f);
    // Trigger (default): an early note-off is ignored.
    auto [t, t2] = rig.play(30000, {note_on(0, 60), note_off(2400, 60)});
    CHECK(tail(t) > 22000);
    rig.set("playMode", 1.0f); // Gate
    auto [g, g2] = rig.play(30000, {note_on(0, 60), note_off(2400, 60)});
    CHECK(tail(g) < 2400 + 2400);
    rig.set("playMode", 2.0f); // Toggle: the second press stops it
    auto [x, x2] = rig.play(30000, {note_on(0, 60), note_off(100, 60), note_on(4800, 60), note_off(4900, 60)});
    CHECK(rms(x, 2000, 4000) > 0.2);
    CHECK(tail(x) < 4800 + 2400);
}

TEST_CASE(sampler_classic_loop_sustains_while_held) {
    auto rig = sampler_with(tone(9600, 480.0, kRate, 0.5));
    rig.set("mode", 0.0f); // Classic
    rig.set("loop", 1.0f);
    rig.set("loopStart", 0.5f);
    auto [l, r] = rig.play(48000, {note_on(0, 60), note_off(36000, 60)});
    CHECK(rms(l, 20000, 34000) > 0.3); // far past the 0.2 s sample
    CHECK(tail(l) < 36000 + 9600);     // released
    // The loop seam is continuous: no sample-to-sample jump beyond the sine's slope.
    double jump = 0;
    for (std::size_t i = 9601; i < 30000; ++i) jump = std::max(jump, static_cast<double>(std::fabs(l[i] - l[i - 1])));
    CHECK(jump < 0.5 * 2 * 3.1416 * 480.0 / kRate * 1.6);
}

TEST_CASE(sampler_slice_mode_plays_one_slice_per_key) {
    Signal s = concat(tone(12000, 400.0, kRate, 0.5), tone(12000, 1600.0, kRate, 0.5));
    auto rig = sampler_with(s, {{0.0f, 0.0f, 0.0f, 0.0f}, {0.5f, -6.0f, 0.0f, 0.0f}});
    rig.set("mode", 2.0f);
    auto [a, a2] = rig.play(16000, {note_on(0, 60)});
    CHECK_NEAR(freq(a, 1000, 10000), 400.0, 3.0);
    CHECK(tail(a) < 12500);
    auto [b, b2] = rig.play(16000, {note_on(0, 61)});
    CHECK_NEAR(freq(b, 1000, 10000), 1600.0, 8.0);
    CHECK_NEAR(db(peak(b, 1000, 10000)), db(0.5) - 6.0, 0.6); // slice gain
    auto [c, c2] = rig.play(8000, {note_on(0, 62)});           // no third slice
    CHECK(peak(c) == 0.0);
}

TEST_CASE(sampler_mono_legato_glides_without_restarting) {
    auto rig = sampler_with(tone(96000, 440.0, kRate, 0.5));
    rig.set("legato", 1.0f);
    rig.set("glide", 50.0f);
    auto [l, r] = rig.play(48000, {note_on(0, 60), note_on(12000, 72), note_off(30000, 72), note_off(40000, 60)});
    CHECK_NEAR(freq(l, 2000, 11000), 440.0, 3.0);
    CHECK_NEAR(freq(l, 20000, 29000), 880.0, 5.0);
    // Releasing the top key falls back to the held one.
    CHECK_NEAR(freq(l, 33000, 39000), 440.0, 4.0);
}

TEST_CASE(sampler_poly_voice_limit_and_stealing) {
    auto rig = sampler_with(tone(96000, 200.0, kRate, 0.1));
    rig.set("voicing", 1.0f);
    rig.set("voices", 2.0f);
    std::vector<NoteAt> n;
    for (int k = 0; k < 6; ++k) n.push_back(note_on(static_cast<std::size_t>(k) * 1000, static_cast<uint8_t>(60 + k)));
    rig.play(12000, n);
    int playing = 0;
    for (float p : rig.readouts()) playing += p >= 0.0f;
    CHECK(playing == 2);
}

TEST_CASE(sampler_filter_and_amp_envelope) {
    auto rig = sampler_with(tone(48000, 5000.0, kRate, 0.5));
    rig.set("filterOn", 1.0f);
    rig.set("cutoff", 500.0f);
    auto [l, r] = rig.play(24000, {note_on(0, 60)});
    CHECK(db(rms(l, 4000, 20000)) < db(0.5 / std::sqrt(2.0)) - 30.0); // LP 24 two decades... >30 dB down
    rig.set("filterOn", 0.0f);
    rig.set("ampAttack", 200.0f);
    rig.reset(); // the first note still sounds
    auto [a, a2] = rig.play(24000, {note_on(0, 60)});
    CHECK(peak(a, 0, 480) < 0.03);        // linear: 5% of the level 10 ms into a 200 ms attack
    CHECK(peak(a, 12000, 14000) > 0.45);  // fully open after it
}

TEST_CASE(sampler_envelope_edits_reach_held_notes) {
    auto rig = sampler_with(tone(48000, 440.0, kRate, 0.5));
    rig.set("mode", 0.0f); // Classic: held notes loop
    rig.set("loop", 1.0f);
    rig.set("ampRelease", 4000.0f);
    rig.play(4800, {note_on(0, 60)});
    rig.set("ampRelease", 10.0f); // while the note is held
    auto [l, r] = rig.play(9600, {note_off(0, 60)});
    CHECK(peak(l, 0, 240) > 0.2);
    CHECK(peak(l, 4800, 9600) < 0.001); // the new 10 ms release, not 4 s
}

TEST_CASE(sampler_pitching_up_does_not_alias) {
    // A 15 kHz sine two octaves up would alias to audible tones; the
    // band-limited reader removes it.
    auto rig = sampler_with(tone(48000, 15000.0, kRate, 0.5));
    auto [l, r] = rig.play(12000, {note_on(0, 84)});
    CHECK(db(rms(l, 1000, 6000)) < -50.0);
    auto [ok, ok2] = rig.play(12000, {note_on(0, 60)}); // in place: passes
    CHECK(db(rms(ok, 1000, 10000)) > db(0.5 / std::sqrt(2.0)) - 1.5);
}

TEST_CASE(sampler_far_above_four_times_does_not_alias) {
    // Three and five octaves up (8x, 32x) read octave-down copies of the
    // sample: a 4 kHz sine at 8x and a 1 kHz sine at 32x would land at
    // 32 kHz, above Nyquist (just past the read filter's corner, the worst
    // case: without the copies these alias at -16 and -10 dB).
    auto rig = sampler_with(tone(48000, 4000.0, kRate, 0.5));
    auto [l, r] = rig.play(12000, {note_on(0, 96)});
    CHECK(db(rms(l, 1000, 4000)) < -38.0);
    auto rig32 = sampler_with(tone(48000, 1000.0, kRate, 0.5));
    auto [l32, r32] = rig32.play(4800, {note_on(0, 120)});
    CHECK(db(rms(l32, 200, 1400)) < -38.0);
    // A 500 Hz sine at 8x (4 kHz) passes.
    auto pass = sampler_with(tone(48000, 500.0, kRate, 0.5));
    auto [p, p2] = pass.play(12000, {note_on(0, 96)});
    CHECK(db(rms(p, 1000, 4000)) > db(0.5 / std::sqrt(2.0)) - 1.0);
}

TEST_CASE(sampler_plays_in_the_instrument_slot_with_live_notes) {
    EngineHandle e(48000, 256);
    const uint64_t asset = load_asset(e, {sine(48000, 440.0, 48000.0, 0.5)});
    std::vector<float> values = sampler_defaults();
    std::vector<TestSample> table(1);
    table[0].slot = 0;
    auto blob = state_blob_v2(values, table, asset);
    se_device_desc inst{};
    inst.device_id = 77;
    inst.plugin_id = "sampler-steam";
    inst.format = SE_PLUGIN_BUILTIN;
    inst.enabled = 1;
    inst.state = blob.data();
    inst.state_len = blob.size();
    se_track_desc_v6 t{};
    t.track_id = 1;
    t.gain = 1.0f;
    t.instrument = 1;
    t.instrument_plugin = &inst;
    se_session_desc_v6 s{};
    s.tracks = &t;
    s.track_count = 1;
    s.master_gain = 1.0f;
    REQUIRE(se_session_publish_v6(e, &s) == SE_OK);
    REQUIRE(se_plugin_is_live(e, 77) == 1);
    auto quiet = render(e, 4800, 256);
    double q = 0;
    for (float v : quiet) q = std::max(q, static_cast<double>(std::fabs(v)));
    CHECK(q == 0.0);
    REQUIRE(se_builtin_note(e, 77, 60, 127) == SE_OK);
    auto loud = render(e, 9600, 256);
    double pk = 0;
    for (float v : loud) pk = std::max(pk, static_cast<double>(std::fabs(v)));
    CHECK(pk > 0.25);
    float ro[8];
    size_t n = 0;
    REQUIRE(se_builtin_readouts(e, 77, ro, 8, &n) == SE_OK);
    CHECK(n == 8 && ro[0] > 0.1f && ro[0] < 0.4f); // ~0.2 s into a 1 s sample
    // Releasing the asset while the device plays it is safe: the device holds it.
    REQUIRE(se_asset_release(e, asset) == SE_OK);
    render(e, 4800, 256);
    // A republish without the asset empties the slot (the voice stops).
    REQUIRE(se_session_publish_v6(e, &s) == SE_OK);
    auto after = render(e, 4800, 256);
    double pa = 0;
    for (std::size_t i = 512; i < after.size(); ++i) pa = std::max(pa, static_cast<double>(std::fabs(after[i])));
    CHECK(pa == 0.0);
    CHECK(se_builtin_note(e, 77, 128, 1) == SE_ERR_INVALID_ARG);
    CHECK(se_builtin_note(e, 999, 60, 1) == SE_ERR_NOT_FOUND);
    // Instruments refuse the effect chain.
    t.instrument_plugin = nullptr;
    t.devices = &inst;
    t.device_count = 1;
    REQUIRE(se_session_publish_v6(e, &s) == SE_OK);
    se_plugin_message msgs[4];
    size_t nm = 0;
    REQUIRE(se_plugin_load_errors(e, msgs, 4, &nm) == SE_OK);
    CHECK(nm == 1);
}

TEST_CASE(builtin_state_v2_is_validated) {
    std::vector<TestSample> table(2);
    table[0].slot = 0;
    table[1].slot = 3;
    table[1].slices = {{0.0f, 0.0f, 0.0f, 0.0f}, {0.5f, 0.0f, 0.0f, 0.0f}};
    auto ok = state_blob_v2({1.f, 2.f}, table, 10, 5);
    saine::BuiltinStateView v;
    REQUIRE(saine::parse_builtin_state(ok.data(), ok.size(), v) == nullptr);
    CHECK(v.value_count == 2 && v.sample_count == 2 && v.slice_count == 2 && v.sidechain == 5);
    auto short_blob = ok;
    short_blob.pop_back();
    CHECK(saine::parse_builtin_state(short_blob.data(), short_blob.size(), v) != nullptr);
    table[1].slices[1].start = 1.0f;
    auto bad_slice = state_blob_v2({1.f}, table, 10);
    CHECK(saine::parse_builtin_state(bad_slice.data(), bad_slice.size(), v) != nullptr);
    table[1].slices[1].start = std::nanf("");
    auto nan_slice = state_blob_v2({1.f}, table, 10);
    CHECK(saine::parse_builtin_state(nan_slice.data(), nan_slice.size(), v) != nullptr);
}
