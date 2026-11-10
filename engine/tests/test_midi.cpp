// MIDI notes + built-in synth: ABI v2 publish, sample-accurate events,
// pitch, velocity, polyphony/stealing, release, republish continuity,
// transport jumps, mixer integration, export and RT safety.
#include "synth.hpp"
#include "test_harness.hpp"
#include "test_util.hpp"

#include <algorithm>
#include <atomic>
#include <cmath>
#include <stdexcept>
#include <thread>

using namespace testutil;

namespace {

constexpr double kRate = 48000.0;

se_note_desc note(int64_t start, int64_t end, uint8_t pitch, uint8_t velocity = 100) {
    se_note_desc n{};
    n.start = start;
    n.end = end;
    n.pitch = pitch;
    n.velocity = velocity;
    return n;
}

// Owns v2 session storage.
struct MidiSession {
    struct T {
        se_track_desc_v2 d{};
        std::vector<se_clip_desc> clips;
        std::vector<se_note_desc> notes;
    };
    std::vector<T> tracks;
    float master_gain = 1.0f;

    T& add(uint64_t id, bool instrument, std::vector<se_note_desc> notes, float gain = 1.0f, float pan = 0.0f) {
        T t;
        t.d.track_id = id;
        t.d.gain = gain;
        t.d.pan = pan;
        t.d.instrument = instrument ? 1 : 0;
        t.notes = std::move(notes);
        tracks.push_back(std::move(t));
        return tracks.back();
    }
    int32_t publish(se_engine* e) {
        std::vector<se_track_desc_v2> ds;
        for (T& t : tracks) {
            t.d.clips = t.clips.data();
            t.d.clip_count = static_cast<uint32_t>(t.clips.size());
            t.d.notes = t.notes.data();
            t.d.note_count = static_cast<uint32_t>(t.notes.size());
            ds.push_back(t.d);
        }
        se_session_desc_v2 s{};
        s.tracks = ds.data();
        s.track_count = static_cast<uint32_t>(ds.size());
        s.master_gain = master_gain;
        return se_session_publish_v2(e, &s);
    }
};

// One hard-left instrument track (L = synth output exactly, R = 0).
MidiSession single(std::vector<se_note_desc> notes, uint64_t id = 1) {
    MidiSession s;
    s.add(id, true, std::move(notes), 1.0f, -1.0f);
    return s;
}

std::vector<float> render_sizes(se_engine* e, uint32_t total, std::initializer_list<uint32_t> sizes) {
    std::vector<float> out(static_cast<std::size_t>(total) * 2);
    const std::vector<uint32_t> sz(sizes);
    uint32_t done = 0;
    for (std::size_t k = 0; done < total; ++k) {
        const uint32_t n = std::min(sz[k % sz.size()], total - done);
        if (se_render_offline(e, out.data() + 2 * static_cast<std::size_t>(done), n) != SE_OK)
            throw std::runtime_error("render failed");
        done += n;
    }
    return out;
}

std::vector<float> left(const std::vector<float>& inter) {
    std::vector<float> l(inter.size() / 2);
    for (std::size_t i = 0; i < l.size(); ++i) l[i] = inter[2 * i];
    return l;
}

int64_t first_nonzero(const std::vector<float>& x) {
    for (std::size_t i = 0; i < x.size(); ++i)
        if (x[i] != 0.0f) return static_cast<int64_t>(i);
    return -1;
}

int64_t first_difference(const std::vector<float>& a, const std::vector<float>& b) {
    for (std::size_t i = 0; i < std::min(a.size(), b.size()); ++i)
        if (a[i] != b[i]) return static_cast<int64_t>(i);
    return -1;
}

float max_jump(const std::vector<float>& x, std::size_t from, std::size_t to) {
    float m = 0.0f;
    for (std::size_t i = std::max<std::size_t>(from, 1); i < std::min(to, x.size()); ++i)
        m = std::max(m, std::fabs(x[i] - x[i - 1]));
    return m;
}

double rms(const std::vector<float>& x, std::size_t from, std::size_t n) {
    double s = 0.0;
    for (std::size_t i = from; i < from + n; ++i) s += static_cast<double>(x[i]) * x[i];
    return std::sqrt(s / static_cast<double>(n));
}

// Frequency by autocorrelation: finds the correlation peak near `periods`
// periods of `expected_hz` and refines it with parabolic interpolation.
// (Zero crossings are unreliable here: the two detuned oscillators beat.)
double estimate_hz(const std::vector<float>& x, std::size_t from, std::size_t n, double expected_hz, int periods) {
    const double center = periods * kRate / expected_hz;
    const auto lo = static_cast<std::size_t>(center * 0.97);
    const auto hi = static_cast<std::size_t>(center * 1.03) + 1;
    auto r = [&](std::size_t lag) {
        double s = 0.0;
        for (std::size_t i = from; i < from + n; ++i) s += static_cast<double>(x[i]) * x[i + lag];
        return s;
    };
    std::size_t best = lo;
    double best_r = r(lo);
    for (std::size_t lag = lo + 1; lag <= hi; ++lag) {
        const double v = r(lag);
        if (v > best_r) best_r = v, best = lag;
    }
    const double y0 = r(best - 1), y1 = best_r, y2 = r(best + 1);
    const double delta = 0.5 * (y0 - y2) / (y0 - 2.0 * y1 + y2);
    return periods * kRate / (static_cast<double>(best) + delta);
}

} // namespace

// ---------------------------------------------------------------- ABI / validation

TEST_CASE(midi_publish_v2_validation) {
    EngineHandle e(kRate, 256);
    {
        MidiSession s = single({note(0, 100, 128)});
        CHECK_EQ(s.publish(e), SE_ERR_INVALID_ARG);
    }
    {
        MidiSession s = single({note(0, 100, 60, 0)});
        CHECK_EQ(s.publish(e), SE_ERR_INVALID_ARG);
    }
    {
        MidiSession s = single({note(0, 100, 60, 128)});
        CHECK_EQ(s.publish(e), SE_ERR_INVALID_ARG);
    }
    {
        se_track_desc_v2 t{};
        t.track_id = 1;
        t.gain = 1.0f;
        t.instrument = 1;
        t.note_count = 3; // notes == NULL
        se_session_desc_v2 d{&t, 1, 1.0f};
        CHECK_EQ(se_session_publish_v2(e, &d), SE_ERR_INVALID_ARG);
        CHECK_EQ(se_session_publish_v2(e, nullptr), SE_ERR_INVALID_ARG);
        CHECK_EQ(se_session_publish_v2(nullptr, &d), SE_ERR_INVALID_ARG);
    }
    // Zero-length notes are dropped; notes on non-instrument tracks are
    // ignored; an empty v2 session is fine.
    {
        MidiSession s;
        s.add(1, true, {note(100, 100, 60), note(200, 150, 60)});
        s.add(2, false, {note(0, 48000, 69)});
        REQUIRE(s.publish(e) == SE_OK);
        REQUIRE(se_transport_play(e) == SE_OK);
        const auto out = render(e, 4800, 256);
        CHECK(std::all_of(out.begin(), out.end(), [](float v) { return v == 0.0f; }));
        se_session_desc_v2 d{nullptr, 0, 1.0f};
        CHECK_EQ(se_session_publish_v2(e, &d), SE_OK);
    }
    // v1 publish still works alongside v2 (and replaces an instrument graph).
    {
        Session s1;
        s1.add_track(1, {});
        CHECK_EQ(s1.publish(e), SE_OK);
    }
}

TEST_CASE(midi_unsorted_input_is_sorted) {
    // Same notes given in two orders render identically.
    auto run = [](std::vector<se_note_desc> notes) {
        EngineHandle e(kRate, 256);
        MidiSession s = single(std::move(notes));
        REQUIRE(s.publish(e) == SE_OK);
        REQUIRE(se_transport_play(e) == SE_OK);
        return render(e, 12000, 256);
    };
    const auto a = run({note(100, 3000, 60), note(2000, 9000, 64), note(2000, 5000, 67)});
    const auto b = run({note(2000, 5000, 67), note(100, 3000, 60), note(2000, 9000, 64)});
    CHECK(a == b);
}

// ---------------------------------------------------------------- timing

TEST_CASE(midi_note_on_off_exact_frames_odd_blocks) {
    auto run = [](int64_t start, int64_t end, std::initializer_list<uint32_t> sizes) {
        EngineHandle e(kRate, 256);
        MidiSession s = single({note(start, end, 69)});
        REQUIRE(s.publish(e) == SE_OK);
        REQUIRE(se_transport_play(e) == SE_OK);
        return render_sizes(e, 20000, sizes);
    };
    for (int64_t start : {0, 1, 1000, 1023, 4097}) {
        const auto a = run(start, 9000, {7, 333, 64, 1, 129, 500});
        const auto l = left(a);
        CHECK_EQ(first_nonzero(l), start); // note-on frame is the first non-zero sample
        for (std::size_t i = 0; i < l.size(); ++i)
            if (a[2 * i + 1] != 0.0f) {
                CHECK(false); // hard left: right channel silent
                break;
            }
        // Bit-identical for any block partition (incl. blocks > max_block).
        CHECK(a == run(start, 9000, {256}));
        CHECK(a == run(start, 9000, {8192, 3, 999}));
    }
    // Note-off: two notes that differ only in `end` render identically up to
    // the earlier end and diverge exactly there (release starts on that frame).
    for (int64_t end : {5000, 5001, 6143, 7777}) {
        const auto a = left(run(1000, end, {7, 333, 64, 1, 129, 500}));
        const auto b = left(run(1000, end + 2500, {129, 1, 500, 7}));
        CHECK_EQ(first_difference(a, b), end);
    }
}

// ---------------------------------------------------------------- pitch / velocity

TEST_CASE(midi_pitch_accuracy_a4_c4) {
    struct Case {
        uint8_t pitch;
        double hz;
    };
    for (const Case c : {Case{69, 440.0}, Case{60, 261.6255653005986}, Case{81, 880.0}, Case{48, 130.8127826502993}}) {
        EngineHandle e(kRate, 256);
        MidiSession s = single({note(0, 60000, c.pitch, 127)});
        REQUIRE(s.publish(e) == SE_OK);
        REQUIRE(se_transport_play(e) == SE_OK);
        const auto l = left(render(e, 60000, 256));
        const double hz = estimate_hz(l, 9600, 24000, c.hz, 10);
        // The default sound is a saw (-5 cents, level 0.6) plus a square (+5
        // cents, level 0.5): the pair's autocorrelation pitch sits slightly
        // off centre, so allow 2 cents here. Exact single-oscillator pitch is
        // checked in synth_pitch_octave_semitone_fine (test_synth.cpp).
        const double tol = c.hz * (std::exp2(2.0 / 1200.0) - 1.0);
        if (!(std::fabs(hz - c.hz) <= tol)) std::fprintf(stderr, "pitch %u: %.4f Hz (want %.4f)\n", c.pitch, hz, c.hz);
        CHECK_NEAR(hz, c.hz, tol);
    }
}

TEST_CASE(midi_velocity_scales_amplitude) {
    auto run = [](uint8_t vel) {
        EngineHandle e(kRate, 256);
        MidiSession s = single({note(10, 20000, 64, vel)});
        REQUIRE(s.publish(e) == SE_OK);
        REQUIRE(se_transport_play(e) == SE_OK);
        return left(render(e, 30000, 256));
    };
    const auto full = run(127);
    for (int vel_i : {1, 32, 64, 100}) {
        const auto vel = static_cast<uint8_t>(vel_i);
        const auto x = run(vel);
        const double expect = vel / 127.0;
        double worst = 0.0, peak = 0.0;
        for (std::size_t i = 0; i < x.size(); ++i) {
            worst = std::max(worst, std::fabs(x[i] - expect * full[i]));
            peak = std::max(peak, static_cast<double>(std::fabs(full[i])));
        }
        CHECK(worst <= 1e-5 * peak); // linear velocity -> amplitude
    }
    double peak = 0.0;
    for (float v : full) peak = std::max(peak, static_cast<double>(std::fabs(v)));
    CHECK(peak > 0.05 && peak < 0.5);
}

// ---------------------------------------------------------------- polyphony / stealing

TEST_CASE(midi_synth_polyphony_limit_and_slots) {
    dstudio::Synth synth(kRate);
    std::vector<dstudio::NoteEvent> notes;
    for (int i = 0; i < 24; ++i)
        notes.push_back({i * 100, 1000000, static_cast<uint8_t>(40 + i), 100});
    std::vector<float> buf(64);
    uint32_t max_notes = 0, max_slots = 0;
    for (int64_t pos = 0; pos < 4000; pos += 64) {
        synth.render(notes.data(), static_cast<uint32_t>(notes.size()), nullptr, 1, 0, pos, 64, true, buf.data());
        max_notes = std::max(max_notes, synth.note_voices());
        max_slots = std::max(max_slots, synth.active_slots());
    }
    CHECK_EQ(max_notes, dstudio::Synth::kMaxNotes);
    CHECK(max_slots <= dstudio::Synth::kSlots);
    // After the fades (5 ms) only the 16 newest notes remain.
    CHECK_EQ(synth.note_voices(), dstudio::Synth::kMaxNotes);
    CHECK_EQ(synth.active_slots(), dstudio::Synth::kMaxNotes);
    // 40 notes at the same frame: still bounded.
    dstudio::Synth burst(kRate);
    std::vector<dstudio::NoteEvent> chord;
    for (int i = 0; i < 40; ++i) chord.push_back({0, 48000, static_cast<uint8_t>(30 + i), 90});
    burst.render(chord.data(), 40, nullptr, 1, 0, 0, 64, true, buf.data());
    CHECK_EQ(burst.note_voices(), dstudio::Synth::kMaxNotes);
    CHECK(burst.active_slots() <= dstudio::Synth::kSlots);
    for (float v : buf) CHECK(std::isfinite(v));
}

TEST_CASE(midi_voice_stealing_has_no_discontinuity) {
    // 16 sustained low notes, then 4 more: each steals the oldest voice. The
    // stealing notes are quiet so the window measures the stolen voice's
    // fade-out rather than the new note's own attack.
    constexpr int64_t kGap = 2400;
    std::vector<se_note_desc> notes;
    for (int i = 0; i < 16; ++i) notes.push_back(note(i * kGap, 200000, static_cast<uint8_t>(36 + i), 110));
    for (int i = 16; i < 20; ++i) notes.push_back(note(i * kGap, 200000, static_cast<uint8_t>(20 + i), 8));
    EngineHandle e(kRate, 256);
    MidiSession s = single(notes);
    REQUIRE(s.publish(e) == SE_OK);
    REQUIRE(se_transport_play(e) == SE_OK);
    const auto l = left(render_sizes(e, 60000, {77, 256, 13}));
    const auto steady = static_cast<std::size_t>(15 * kGap + 1200); // 16 voices, past the last attack
    const float steady_jump = max_jump(l, steady, steady + 1200);
    float steal_jump = 0.0f;
    for (int k = 16; k < 20; ++k) {
        const auto at = static_cast<std::size_t>(k * kGap);
        steal_jump = std::max(steal_jump, max_jump(l, at - 4, at + 600));
    }
    // What a hard cut would look like: the oldest voice's own value at the
    // steal frame (rendered alone).
    EngineHandle solo(kRate, 256);
    MidiSession s1 = single({notes[0]});
    REQUIRE(s1.publish(solo) == SE_OK);
    REQUIRE(se_transport_play(solo) == SE_OK);
    const auto one = left(render(solo, 16 * kGap + 1, 256));
    float cut = 0.0f;
    for (std::size_t i = 16 * kGap - 200; i <= 16 * kGap; ++i) cut = std::max(cut, std::fabs(one[i]));
    std::fprintf(stderr, "  steady jump %.5f, steal-window jump %.5f, hard-cut step would be up to %.5f\n",
                 static_cast<double>(steady_jump), static_cast<double>(steal_jump), static_cast<double>(cut));
    CHECK(steal_jump <= 1.25f * steady_jump);
    CHECK(cut > 2.0f * steady_jump); // the check above would catch a hard cut
}

// ---------------------------------------------------------------- release

TEST_CASE(midi_release_tail_after_note_end) {
    EngineHandle e(kRate, 256);
    MidiSession s = single({note(1000, 11000, 57)});
    REQUIRE(s.publish(e) == SE_OK);
    REQUIRE(se_transport_play(e) == SE_OK);
    const auto l = left(render_sizes(e, 40000, {200, 33}));
    // The tail sounds after the note-off and decays.
    const double at_off = rms(l, 10000, 960);
    double prev = rms(l, 11000, 960);
    CHECK(prev > 0.3 * at_off);
    for (std::size_t w = 11960; w + 960 <= 11000 + 12000; w += 960) {
        const double r = rms(l, w, 960);
        CHECK(r < prev);
        prev = r;
    }
    // ~250 ms after the note-off the level is ~60 dB down.
    const double later = rms(l, 11000 + 12000 - 480, 960);
    CHECK(later < at_off * 0.0015 && later > at_off * 0.0001);
    // The voice is freed at -90 dB (~375 ms): exact silence afterwards.
    CHECK_EQ(first_nonzero(std::vector<float>(l.begin() + 11000 + 19200, l.end())), -1);
}

// ---------------------------------------------------------------- republish

TEST_CASE(midi_republish_mid_note_keeps_voice) {
    const std::vector<se_note_desc> notes = {note(0, 30000, 62), note(500, 8000, 69), note(40000, 41000, 50)};
    auto reference = [&](std::vector<se_note_desc> n) {
        EngineHandle e(kRate, 256);
        MidiSession s = single(std::move(n), 7);
        REQUIRE(s.publish(e) == SE_OK);
        REQUIRE(se_transport_play(e) == SE_OK);
        return left(render(e, 48000, 100));
    };
    const auto ref = reference(notes);

    // Identical republishes (plus unrelated changes: another track, a new
    // note elsewhere, track order) mid-note: no retrigger, no cut.
    {
        EngineHandle e(kRate, 256);
        MidiSession s = single(notes, 7);
        REQUIRE(s.publish(e) == SE_OK);
        REQUIRE(se_transport_play(e) == SE_OK);
        std::vector<float> out;
        for (int i = 0; i < 480; ++i) {
            if (i % 37 == 5) {
                MidiSession s2;
                s2.add(99, true, {note(0, 1000000, 40)}, 1.0f, 0.0f).d.mute = 1; // muted: no output
                s2.add(7, true, notes, 1.0f, -1.0f);
                REQUIRE(s2.publish(e) == SE_OK);
            } else if (i % 37 == 20) {
                REQUIRE(s.publish(e) == SE_OK);
            }
            const auto blk = left(render(e, 100, 100));
            out.insert(out.end(), blk.begin(), blk.end());
        }
        CHECK(out == ref);
    }
    // Lengthening a sounding note mid-note: the note-off follows the new end.
    {
        std::vector<se_note_desc> longer = notes;
        longer[1].end = 20000;
        const auto ref_long = reference(longer);
        EngineHandle e(kRate, 256);
        MidiSession s = single(notes, 7);
        REQUIRE(s.publish(e) == SE_OK);
        REQUIRE(se_transport_play(e) == SE_OK);
        auto out = left(render(e, 5000, 100));
        MidiSession s2 = single(longer, 7);
        REQUIRE(s2.publish(e) == SE_OK);
        const auto rest = left(render(e, 43000, 100));
        out.insert(out.end(), rest.begin(), rest.end());
        CHECK(out == ref_long);
        // Shortening to before the playhead releases at the next block start.
        std::vector<float> o2;
        EngineHandle e2(kRate, 256);
        REQUIRE(s.publish(e2) == SE_OK);
        REQUIRE(se_transport_play(e2) == SE_OK);
        o2 = left(render(e2, 5000, 100));
        std::vector<se_note_desc> shorter = notes;
        shorter[1].end = 3000;
        MidiSession s3 = single(shorter, 7);
        REQUIRE(s3.publish(e2) == SE_OK);
        const auto r2 = left(render(e2, 4000, 100));
        CHECK(r2[0] != 0.0f);                           // released, not cut
        CHECK(max_jump(r2, 0, 200) < 0.05f);            // no click
    }
    // Deleting a sounding note releases it (tail, not a cut).
    {
        EngineHandle e(kRate, 256);
        MidiSession s = single({note(0, 30000, 62)}, 7);
        REQUIRE(s.publish(e) == SE_OK);
        REQUIRE(se_transport_play(e) == SE_OK);
        const auto before = left(render(e, 5000, 100));
        MidiSession empty = single({}, 7);
        REQUIRE(empty.publish(e) == SE_OK);
        const auto after = left(render(e, 30000, 100));
        CHECK(after[0] != 0.0f);
        CHECK(std::fabs(after[0] - before.back()) < 0.05f);
        CHECK(rms(after, 0, 480) > 0.3 * rms(before, 4520, 480));
        CHECK_EQ(first_nonzero(std::vector<float>(after.begin() + 19200, after.end())), -1);
    }
}

// ---------------------------------------------------------------- transport jumps

TEST_CASE(midi_loop_wrap_and_seek_release_voices) {
    // A: at the loop start; B: held across the loop end.
    const std::vector<se_note_desc> notes = {note(0, 6000, 40), note(20000, 50000, 43)};
    const auto fade = static_cast<std::size_t>(dstudio::Synth::kFadeSeconds * kRate);
    {
        EngineHandle e(kRate, 256);
        MidiSession s = single(notes);
        REQUIRE(s.publish(e) == SE_OK);
        REQUIRE(se_transport_set_loop(e, 1, 0, 30000) == SE_OK);
        REQUIRE(se_transport_play(e) == SE_OK);
        const auto l = left(render_sizes(e, 36000, {333, 7, 256}));
        // B fades within ~5 ms after the wrap; A retriggers exactly at the
        // wrap, so afterwards the output equals the first pass.
        for (std::size_t k = fade + 2; k < 5000; ++k)
            if (l[30000 + k] != l[k]) {
                CHECK(false);
                break;
            }
        CHECK(l[30000] != l[0]); // B is still fading at the wrap frame
        // Without a click, the jump around the wrap is bounded by B's own
        // steady jump plus A's attack jump (triangle inequality); a hard cut
        // of B would add a step of B's instantaneous value.
        const float steady_b = max_jump(l, 25000, 29990);
        const float attack_a = max_jump(l, 0, 2 * fade);
        const float wrap = max_jump(l, 29990, 30000 + 2 * fade);
        float b_level = 0.0f;
        for (std::size_t i = 29700; i < 30000; ++i) b_level = std::max(b_level, std::fabs(l[i]));
        std::fprintf(stderr, "  wrap jump %.5f, bound %.5f, B level %.5f\n", static_cast<double>(wrap),
                     static_cast<double>(steady_b + attack_a), static_cast<double>(b_level));
        CHECK(wrap <= 1.05f * (steady_b + attack_a));
        CHECK(b_level > 2.0f * (steady_b + attack_a));
    }
    // Seek (mid-block in device terms; here at a block edge) behaves the same.
    {
        EngineHandle e(kRate, 256);
        MidiSession s = single(notes);
        REQUIRE(s.publish(e) == SE_OK);
        REQUIRE(se_transport_play(e) == SE_OK);
        const auto first = left(render(e, 25000, 100));
        REQUIRE(se_transport_seek(e, 0) == SE_OK);
        const auto second = left(render(e, 5000, 100));
        for (std::size_t k = fade + 2; k < 5000; ++k)
            if (second[k] != first[k]) {
                CHECK(false);
                break;
            }
        CHECK(std::fabs(second[0] - first.back()) < 0.05f); // fade, not a cut
        // Seeking into the middle of a note does not retrigger it.
        REQUIRE(se_transport_seek(e, 25000) == SE_OK);
        const auto mid = left(render(e, 2000, 100));
        CHECK_EQ(first_nonzero(std::vector<float>(mid.begin() + static_cast<std::ptrdiff_t>(fade) + 2, mid.end())),
                 -1);
    }
    // Stop / pause: voices fade (tail rendered while paused), then silence.
    {
        EngineHandle e(kRate, 256);
        MidiSession s = single(notes);
        REQUIRE(s.publish(e) == SE_OK);
        REQUIRE(se_transport_play(e) == SE_OK);
        const auto a = left(render(e, 22000, 256));
        REQUIRE(se_transport_pause(e) == SE_OK);
        const auto b = left(render(e, 2000, 100));
        CHECK(b[0] != 0.0f);
        CHECK(std::fabs(b[0] - a.back()) < 0.05f);
        CHECK_EQ(first_nonzero(std::vector<float>(b.begin() + static_cast<std::ptrdiff_t>(fade) + 2, b.end())), -1);
        se_transport_info t{};
        se_transport_get(e, &t);
        CHECK_EQ(t.position, 22000); // paused: position does not move
        // Resume: the note in progress is not retriggered.
        REQUIRE(se_transport_play(e) == SE_OK);
        CHECK_EQ(first_nonzero(left(render(e, 2000, 100))), -1);
        // Stop -> play from 0: the note at 0 starts again exactly at frame 0.
        REQUIRE(se_transport_stop(e) == SE_OK);
        REQUIRE(se_transport_play(e) == SE_OK);
        const auto c = left(render(e, 3000, 128));
        CHECK(c[0] != 0.0f);
    }
}

// ---------------------------------------------------------------- mixer integration

TEST_CASE(midi_mute_solo_gain_apply_to_synth) {
    const std::vector<se_note_desc> notes = {note(0, 20000, 64)};
    auto run = [&](float gain, float pan, bool mute, bool solo_other, int unmute_at = -1) {
        EngineHandle e(kRate, 256);
        const uint64_t a = load_asset(e, {constant(48000, 0.25f)});
        MidiSession s;
        auto& t = s.add(1, true, notes, gain, pan);
        t.d.mute = mute ? 1 : 0;
        auto& other = s.add(2, false, {}, 1.0f, 1.0f);
        other.clips.push_back(clip(1, a, 0, 0, 48000));
        other.d.solo = solo_other ? 1 : 0;
        s.master_gain = 1.0f;
        REQUIRE(s.publish(e) == SE_OK);
        REQUIRE(se_transport_play(e) == SE_OK);
        if (unmute_at < 0) return render(e, 24000, 256);
        auto out = render(e, static_cast<uint32_t>(unmute_at), 256);
        s.tracks[0].d.mute = 0;
        REQUIRE(s.publish(e) == SE_OK);
        const auto rest = render(e, 24000 - static_cast<uint32_t>(unmute_at), 256);
        out.insert(out.end(), rest.begin(), rest.end());
        return out;
    };
    // Track 2 is hard right (R only), so the left channel is synth only.
    const auto ref = left(run(1.0f, -1.0f, false, false));
    CHECK(rms(ref, 1000, 10000) > 0.01);
    const auto half = left(run(0.5f, -1.0f, false, false));
    for (std::size_t i = 0; i < ref.size(); ++i)
        if (std::fabs(half[i] - 0.5f * ref[i]) > 1e-7f) {
            CHECK(false);
            break;
        }
    // Centre pan: -3 dB on the left.
    const auto centre = left(run(1.0f, 0.0f, false, false));
    CHECK_NEAR(rms(centre, 1000, 10000) / rms(ref, 1000, 10000), std::sqrt(0.5), 1e-4);
    // Muted / another track soloed: silent left channel.
    const auto muted = left(run(1.0f, -1.0f, true, false));
    CHECK_EQ(first_nonzero(muted), -1);
    const auto soloed = left(run(1.0f, -1.0f, false, true));
    CHECK_EQ(first_nonzero(soloed), -1);
    // The synth keeps running while muted: unmuting mid-note continues the
    // note exactly where the unmuted render is.
    const auto unmuted = left(run(1.0f, -1.0f, true, false, 6000));
    CHECK_EQ(first_nonzero(unmuted), 6000);
    CHECK(std::equal(unmuted.begin() + 6000, unmuted.end(), ref.begin() + 6000));
    // Meters see the synth.
    EngineHandle e(kRate, 256);
    MidiSession s = single(notes);
    REQUIRE(s.publish(e) == SE_OK);
    REQUIRE(se_transport_play(e) == SE_OK);
    render(e, 4800, 256);
    se_meter m[4]{};
    std::size_t n = 0;
    REQUIRE(se_meters_peek(e, m, 4, &n) == SE_OK);
    REQUIRE(n == 2);
    CHECK(m[1].peak_l > 0.01f);
    CHECK_EQ(m[1].peak_r, 0.0f);
}

TEST_CASE(midi_sums_with_audio_clips_on_same_track) {
    EngineHandle e(kRate, 256);
    const uint64_t a = load_asset(e, {constant(48000, 0.125f)});
    MidiSession s = single({note(0, 10000, 72)});
    s.tracks[0].clips.push_back(clip(1, a, 0, 0, 48000));
    REQUIRE(s.publish(e) == SE_OK);
    REQUIRE(se_transport_play(e) == SE_OK);
    const auto both = left(render(e, 12000, 256));
    EngineHandle e2(kRate, 256);
    MidiSession s2 = single({note(0, 10000, 72)});
    REQUIRE(s2.publish(e2) == SE_OK);
    REQUIRE(se_transport_play(e2) == SE_OK);
    const auto synth_only = left(render(e2, 12000, 256));
    for (std::size_t i = 0; i < both.size(); ++i)
        if (std::fabs(both[i] - (synth_only[i] + 0.125f)) > 1e-6f) {
            CHECK(false);
            break;
        }
}

// ---------------------------------------------------------------- export

TEST_CASE(midi_export_includes_notes) {
    EngineHandle e(kRate, 256);
    MidiSession s = single({note(100, 9000, 60), note(4000, 15000, 67, 70)});
    REQUIRE(s.publish(e) == SE_OK);
    REQUIRE(se_transport_play(e) == SE_OK);
    // Live playback with voices sounding while exporting: unaffected.
    const auto live_a = render(e, 5000, 333);
    TempFile f("midi_export.wav");
    REQUIRE(se_export_wav(e, f.path.c_str(), 0, 30000) == SE_OK);
    const auto live_b = render(e, 25000, 333);

    EngineHandle ref(kRate, 256);
    REQUIRE(s.publish(ref) == SE_OK);
    REQUIRE(se_transport_play(ref) == SE_OK);
    const auto expected = render(ref, 30000, 64);
    std::vector<float> live(live_a);
    live.insert(live.end(), live_b.begin(), live_b.end());
    CHECK(live == expected);

    // The exported file is the same audio, bit for bit.
    EngineHandle e3(kRate, 256);
    se_asset_info info3{};
    REQUIRE(se_asset_load_file(e3, f.path.c_str(), &info3) == SE_OK);
    REQUIRE(info3.frames == 30000);
    Session p3;
    p3.add_track(1, {clip(1, info3.asset_id, 0, 0, 30000)}, 1.0f, -1.0f);
    REQUIRE(p3.publish(e3) == SE_OK);
    REQUIRE(se_transport_play(e3) == SE_OK);
    const auto back = render(e3, 30000, 256);
    for (std::size_t i = 0; i < 30000; ++i)
        if (back[2 * i] != expected[2 * i]) {
            CHECK(false);
            break;
        }
    CHECK(rms(left(expected), 200, 8000) > 0.01);
}

// ---------------------------------------------------------------- RT safety

TEST_CASE(rt_dense_midi_with_republish_does_not_allocate) {
    EngineHandle e(kRate, 256);
    auto make = [](int variant) {
        MidiSession s;
        for (int t = 0; t < 8; ++t) {
            std::vector<se_note_desc> notes;
            for (int k = 0; k < 200; ++k) {
                const int64_t start = k * 211 + t * 13 + (variant & 1) * (k % 3);
                notes.push_back(note(start, start + 300 + (k * 97 + variant * 31) % 4000,
                                     static_cast<uint8_t>(36 + (k * 7 + t) % 48), static_cast<uint8_t>(20 + k % 100)));
            }
            s.add(static_cast<uint64_t>(t + 1), true, std::move(notes), 0.3f, 0.0f).d.mute = t == 5 ? 1 : 0;
        }
        return s;
    };
    MidiSession s0 = make(0);
    REQUIRE(s0.publish(e) == SE_OK);
    REQUIRE(se_transport_set_loop(e, 1, 2000, 40000) == SE_OK);
    REQUIRE(se_transport_play(e) == SE_OK);
    std::vector<float> buf(2 * 8192);
    REQUIRE(se_render_offline(e, buf.data(), 256) == SE_OK); // warm-up

    std::atomic<bool> stop{false};
    std::atomic<int> publishes{0};
    std::thread publisher([&] {
        std::vector<MidiSession> sessions = {make(0), make(1), make(2), make(3)};
        int i = 0;
        while (!stop.load()) {
            sessions[static_cast<std::size_t>(i % 4)].publish(e);
            publishes.fetch_add(1);
            ++i;
            std::this_thread::yield();
        }
    });
    std::thread commander([&] {
        int i = 0;
        while (!stop.load()) {
            if (i % 50 == 0) se_transport_seek(e, 1000 + (static_cast<int64_t>(i) * 37) % 30000);
            if (i % 120 == 60) se_transport_pause(e);
            if (i % 120 == 61) se_transport_play(e);
            ++i;
            std::this_thread::yield();
        }
    });

    const long allocs_before = rt_alloc_count();
    const long frees_before = rt_free_count();
    const uint32_t sizes[] = {256, 1, 333, 8192, 64, 255, 77};
    for (int i = 0; i < 1500; ++i) {
        RtAllocGuard g;
        if (se_render_offline(e, buf.data(), sizes[i % 7]) != SE_OK) {
            CHECK(false);
            break;
        }
    }
    stop = true;
    publisher.join();
    commander.join();
    CHECK_EQ(rt_alloc_count() - allocs_before, 0);
    CHECK_EQ(rt_free_count() - frees_before, 0);
    CHECK(publishes.load() > 10);
    for (float v : buf) CHECK(std::isfinite(v));
}

// Step-sequencer patterns as dstudio-core publishes them: every step of a bar
// on 16 pad notes, each step split into repeats with the 0.02-beat gate gap,
// the bar repeated, republished while it plays (each step edit republishes).
// Nothing allocates on the render path, and every step window sounds.
TEST_CASE(rt_step_pattern_repeats_do_not_allocate) {
    EngineHandle e(kRate, 256);
    constexpr int64_t kBar = 96000; // 4/4 at 120 BPM
    auto make = [](uint32_t steps, uint32_t repeat) {
        const double step = static_cast<double>(kBar) / steps, span = step / repeat;
        const double gate = repeat > 1 ? std::max(span * 0.5, span - 480.0) : step;
        std::vector<se_note_desc> notes;
        for (int64_t bar = 0; bar < 4 * kBar; bar += kBar)
            for (uint32_t s = 0; s < steps; ++s)
                for (uint32_t k = 0; k < repeat; ++k)
                    for (uint8_t row = 0; row < 16; ++row) {
                        const auto start = bar + static_cast<int64_t>(s * step + k * span);
                        notes.push_back(note(start, start + static_cast<int64_t>(gate), static_cast<uint8_t>(36 + row), 90));
                    }
        MidiSession s;
        s.add(1, true, std::move(notes), 0.2f);
        return s;
    };
    MidiSession s0 = make(16, 4);
    REQUIRE(s0.publish(e) == SE_OK);
    REQUIRE(se_transport_play(e) == SE_OK);
    std::vector<float> buf(2 * 256);
    REQUIRE(se_render_offline(e, buf.data(), 256) == SE_OK); // warm-up

    std::atomic<bool> stop{false};
    std::thread publisher([&] {
        std::vector<MidiSession> sessions = {make(16, 4), make(32, 2), make(64, 16), make(16, 1)};
        int i = 0;
        while (!stop.load()) {
            sessions[static_cast<std::size_t>(i++ % 4)].publish(e);
            std::this_thread::yield();
        }
    });
    const long allocs_before = rt_alloc_count();
    const long frees_before = rt_free_count();
    std::vector<float> peak_per_window(64, 0.0f); // 1/16 windows over 4 bars
    for (int64_t at = 256; at < 4 * kBar; at += 256) {
        RtAllocGuard g;
        if (se_render_offline(e, buf.data(), 256) != SE_OK) {
            CHECK(false);
            break;
        }
        float& peak = peak_per_window[static_cast<std::size_t>(at / 6000) % 64];
        for (float v : buf) peak = std::max(peak, std::fabs(v));
    }
    stop = true;
    publisher.join();
    CHECK_EQ(rt_alloc_count() - allocs_before, 0);
    CHECK_EQ(rt_free_count() - frees_before, 0);
    for (float p : peak_per_window) CHECK(p > 1e-3f && std::isfinite(p));
}
