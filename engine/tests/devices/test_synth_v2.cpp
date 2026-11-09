// synth-v2: pitch, voices and modes, envelopes, glide, modulation matrix,
// macros, effect slots, sample-accurate notes and determinism. The generic
// RT-safety and robustness checks run in test_builtin.cpp.
#include "device_harness.hpp"
#include "devices/synth_v2_params.hpp"

using namespace testutil;
using namespace devtest;

namespace {

constexpr double kRate = 48000.0;

std::vector<float> defaults() { return {std::begin(saine::synth_v2::kDefaults), std::end(saine::synth_v2::kDefaults)}; }

saine::MidiEvent on(uint32_t at, uint8_t note, uint8_t vel = 100) { return {at, 0x90, note, vel, 0}; }
saine::MidiEvent off(uint32_t at, uint8_t note) { return {at, 0x80, note, 0, 0}; }

// A plain voice: one sine oscillator, no sub / noise / unison / drift,
// open filter, instant envelopes, every effect off.
DeviceRig plain_rig() {
    DeviceRig rig("synth-v2", defaults(), kRate, 256);
    rig.set("osc1_wave", 0.0f);
    rig.set("osc1_shape", 0.0f);
    rig.set("osc2_level", 0.0f);
    rig.set("sub_level", 0.0f);
    rig.set("noise_level", 0.0f);
    rig.set("unison_voices", 0.0f);
    rig.set("drift", 0.0f);
    rig.set("filter_type", 0.0f);
    rig.set("cutoff", 20000.0f);
    rig.set("resonance", 0.0f);
    rig.set("filter_drive", 0.0f);
    rig.set("filter_env", 0.0f);
    rig.set("key_track", 0.0f);
    rig.set("amp_attack", 0.0005f);
    rig.set("amp_sustain", 1.0f);
    rig.set("amp_release", 0.02f);
    for (const char* fx : {"fx1_on", "fx2_on", "fx3_on", "fx4_on"}) rig.set(fx, 0.0f);
    return rig;
}

Signal play(DeviceRig& rig, std::size_t n, const std::vector<saine::MidiEvent>& midi) {
    return rig.run(Signal(n, 0.0f), Signal(n, 0.0f), nullptr, nullptr, {}, midi).first;
}

// Frequency from rising zero crossings in [from, to).
double frequency(const Signal& s, std::size_t from, std::size_t to) {
    std::size_t first = 0, last = 0, count = 0;
    for (std::size_t i = from + 1; i < to; ++i)
        if (s[i - 1] <= 0.0f && s[i] > 0.0f) {
            if (count == 0) first = i;
            last = i;
            ++count;
        }
    return count > 1 ? (static_cast<double>(count - 1) * kRate) / static_cast<double>(last - first) : 0.0;
}

} // namespace

TEST_CASE(synth_v2_plays_the_note_pitch) {
    DeviceRig rig = plain_rig();
    const Signal s = play(rig, 24000, {on(0, 69)});
    CHECK(std::fabs(frequency(s, 2000, 24000) - 440.0) < 0.5);
    rig.set("osc1_octave", 3.0f); // +1 octave
    rig.set("osc1_semi", 19.0f);  // +7 st
    const Signal t = play(rig, 24000, {off(0, 69), on(4800, 57)});
    CHECK(std::fabs(frequency(t, 8000, 24000) - 220.0 * 2.0 * std::pow(2.0, 7.0 / 12.0)) < 1.0);
}

TEST_CASE(synth_v2_note_off_releases_to_silence) {
    DeviceRig rig = plain_rig();
    const Signal s = play(rig, 48000, {on(0, 60), off(12000, 60)});
    CHECK(rms(s, 4000, 12000) > 0.05);
    CHECK(peak(s, 24000) < 1e-4);
    CHECK(rig.readouts()[0] == 0.0f); // no voices left
}

TEST_CASE(synth_v2_notes_start_sample_accurately) {
    DeviceRig rig = plain_rig();
    const Signal s = play(rig, 2048, {on(700, 72)});
    CHECK(peak(s, 0, 700) == 0.0);
    CHECK(peak(s, 700, 800) > 0.01);
}

TEST_CASE(synth_v2_voice_modes) {
    DeviceRig poly = plain_rig();
    play(poly, 4800, {on(0, 60), on(10, 64), on(20, 67), on(30, 71)});
    CHECK(poly.readouts()[0] == 4.0f);
    DeviceRig mono = plain_rig();
    mono.set("voice_mode", 1.0f);
    play(mono, 4800, {on(0, 60), on(10, 64), on(20, 67), on(30, 71)});
    CHECK(mono.readouts()[0] == 1.0f);
    // Legato with Low priority: the lowest held note sounds.
    DeviceRig legato = plain_rig();
    legato.set("voice_mode", 2.0f);
    legato.set("note_priority", 1.0f);
    const Signal s = play(legato, 24000, {on(0, 69), on(10, 81)});
    CHECK(std::fabs(frequency(s, 4000, 24000) - 440.0) < 1.0);
}

TEST_CASE(synth_v2_glide_moves_between_notes) {
    DeviceRig rig = plain_rig();
    rig.set("voice_mode", 1.0f);
    rig.set("glide", 0.5f);
    const Signal s = play(rig, 72000, {on(0, 57), on(24000, 69)});
    const double early = frequency(s, 24000, 26400), late = frequency(s, 60000, 72000);
    CHECK(early > 225.0 && early < 400.0);
    CHECK(std::fabs(late - 440.0) < 2.0);
}

TEST_CASE(synth_v2_modulation_routes_apply) {
    DeviceRig rig = plain_rig();
    rig.set("lfo1_rate", 5.0f);
    rig.set("lfo1_depth", 1.0f);
    rig.set("mod1_source", 3.0f);  // LFO 1
    rig.set("mod1_target", 1.0f);  // Pitch
    rig.set("mod1_amount", 0.25f); // +-6 st
    rig.set("mod1_on", 1.0f);
    const Signal s = play(rig, 48000, {on(0, 69)});
    double lo = 1e9, hi = 0.0;
    for (std::size_t w = 0; w + 2400 <= 48000; w += 2400) {
        const double f = frequency(s, w, w + 2400);
        lo = std::min(lo, f);
        hi = std::max(hi, f);
    }
    CHECK(hi / lo > 1.5); // about a 6 st swing either way
    rig.set("mod1_on", 0.0f);
    const Signal t = play(rig, 24000, {off(0, 69), on(100, 69)});
    CHECK(std::fabs(frequency(t, 4800, 24000) - 440.0) < 1.0);
}

TEST_CASE(synth_v2_macro_mappings_hold_their_targets) {
    DeviceRig rig = plain_rig();
    rig.set("map1_macro", 1.0f);
    rig.set("map1_param", 6.0f); // "Osc 1 Level" (index 5) + 1
    rig.set("map1_from", 0.0f);
    rig.set("map1_to", 1.0f);
    rig.set("macro1", 0.0f);
    CHECK(peak(play(rig, 4800, {on(0, 60)}), 1000) < 1e-6);
    rig.set("macro1", 1.0f);
    CHECK(peak(play(rig, 4800, {}), 1000) > 0.05);
}

TEST_CASE(synth_v2_effect_slots_run_every_type) {
    for (int type = 0; type < 11; ++type) {
        DeviceRig rig = plain_rig();
        rig.set("fx1_on", 1.0f);
        rig.set("fx1_type", static_cast<float>(type));
        rig.set("fx1_mix", 1.0f);
        const Signal s = play(rig, 24000, {on(0, 60), off(9600, 60)});
        CHECK(all_finite(s));
        CHECK(rms(s) > 1e-3); // fully wet: the delay's first echo comes after the note
    }
    // A long reverb keeps sounding after the voice has ended.
    DeviceRig dry = plain_rig(), wet = plain_rig();
    wet.set("fx1_on", 1.0f);
    wet.set("fx1_type", 8.0f);
    wet.set("fx1_mix", 0.5f);
    wet.set("fx1_reverb_decay", 4.0f);
    const Signal a = play(dry, 48000, {on(0, 60), off(4800, 60)});
    const Signal b = play(wet, 48000, {on(0, 60), off(4800, 60)});
    CHECK(rms(a, 12000, 24000) < 1e-5);
    CHECK(rms(b, 12000, 24000) > 1e-3);
}

TEST_CASE(synth_v2_default_patch_is_deterministic_and_in_range) {
    DeviceRig a("synth-v2", defaults(), kRate, 256), b("synth-v2", defaults(), kRate, 256);
    const std::vector<saine::MidiEvent> chord = {on(0, 48), on(5, 55), on(9, 64), off(30000, 48), off(30000, 55), off(30000, 64)};
    const auto [al, ar] = a.run(Signal(48000, 0.0f), Signal(48000, 0.0f), nullptr, nullptr, {}, chord);
    const auto [bl, br] = b.run(Signal(48000, 0.0f), Signal(48000, 0.0f), nullptr, nullptr, {}, chord);
    CHECK(all_finite(al) && all_finite(ar));
    CHECK(peak(al) > 0.05 && peak(al) < 1.5);
    double diff = 0.0;
    for (std::size_t i = 0; i < al.size(); ++i) diff = std::max(diff, static_cast<double>(std::fabs(al[i] - bl[i])));
    CHECK(diff == 0.0);
}

TEST_CASE(synth_v2_is_silent_without_notes) {
    DeviceRig rig("synth-v2", defaults(), kRate, 256);
    const Signal s = play(rig, 24000, {});
    if (peak(s) != 0.0)
        for (std::size_t i = 0; i < s.size(); ++i)
            if (s[i] != 0.0f) {
                std::fprintf(stderr, "first non-zero at %zu: %g\n", i, static_cast<double>(s[i]));
                break;
            }
    CHECK(peak(s) == 0.0);
    // After a note has fully died away the output returns to exact zero.
    const Signal t = play(rig, 12 * 48000, {on(0, 60), off(4800, 60)});
    CHECK(peak(t, 10 * 48000) == 0.0);
}
