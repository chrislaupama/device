// Auto Filter (engine/src/devices/auto_filter.cpp): filter shapes and slopes
// against the panel model, resonance, envelope and LFO modulation (free,
// synced, stereo), DJ mode, drive and mix.
#include "device_harness.hpp"

using namespace devtest;

namespace {

constexpr double kRate = 48000.0;
// Defaults of saine_core::builtin::auto_filter, registry order.
const std::vector<float> kDefaults = {0.f, 0.f, 20000.f, 0.f, 0.f, 0.f, 1.f, 100.f, 0.f,
                                      1.f, 4.f, 0.f,     0.f, 0.f, 100.f, 0.f, 0.f};

DeviceRig rig() { return DeviceRig("auto-filter", kDefaults, kRate, 256); }

double gain_at(DeviceRig& r, double hz, double amp = 0.01) {
    const std::size_t n = 24000;
    const Signal x = tone(n, hz, kRate, amp);
    const Signal y = r.run_mono(x);
    return db(rms(y, n / 2) / rms(x, n / 2));
}

} // namespace

TEST_CASE(auto_filter_defaults_are_open) {
    auto r = rig();
    CHECK_NEAR(gain_at(r, 1000.0), 0.0, 0.02);
    CHECK_NEAR(gain_at(r, 50.0), 0.0, 0.02);
    CHECK(r.instance().latency() == 0u);
}

TEST_CASE(auto_filter_shapes_match_the_panel_model) {
    // saine_core::builtin::auto_filter::filter_response: Q 0.5 at zero
    // resonance gives -6.02 dB at the cutoff for LP and HP, 0 dB for BP.
    auto lp = rig();
    lp.set("frequency", 1000.f);
    CHECK_NEAR(gain_at(lp, 1000.0), -6.02, 0.05);
    auto lp24 = rig();
    lp24.set("frequency", 1000.f);
    lp24.set("slope", 1.f);
    auto lp12 = rig();
    lp12.set("frequency", 1000.f);
    CHECK(gain_at(lp24, 8000.0) < gain_at(lp12, 8000.0) - 20.0);
    auto hp = rig();
    hp.set("filter_type", 1.f);
    hp.set("frequency", 1000.f);
    CHECK_NEAR(gain_at(hp, 1000.0), -6.02, 0.05);
    CHECK(gain_at(hp, 100.0) < -35.0);
    auto bp = rig();
    bp.set("filter_type", 2.f);
    bp.set("frequency", 1000.f);
    CHECK_NEAR(gain_at(bp, 1000.0), 0.0, 0.05);
    auto notch = rig();
    notch.set("filter_type", 3.f);
    notch.set("frequency", 1000.f);
    notch.set("resonance", 0.5f);
    CHECK(gain_at(notch, 1000.0) < -40.0);
}

TEST_CASE(auto_filter_resonance_peaks_and_stays_bounded) {
    auto r = rig();
    r.set("frequency", 1000.f);
    r.set("resonance", 1.f);
    CHECK(gain_at(r, 1000.0) > 12.0);
    auto loud = rig();
    loud.set("frequency", 1000.f);
    loud.set("resonance", 1.25f);
    loud.set("slope", 1.f);
    const Signal y = loud.run_mono(noise(48000, 1.0f));
    CHECK(all_finite(y));
    CHECK(peak(y) < 64.0);
}

TEST_CASE(auto_filter_envelope_opens_the_cutoff) {
    auto r = rig();
    r.set("frequency", 200.f);
    r.set("env_amount", 48.f);
    r.set("env_attack", 1.f);
    r.run_mono(tone(9600, 100.0, kRate, 0.9));
    const float open = r.readouts()[0];
    CHECK(open > 200.f * 8.f);
    auto quiet = rig();
    quiet.set("frequency", 200.f);
    quiet.set("env_amount", 48.f);
    quiet.run_mono(Signal(9600, 0.0f));
    CHECK_NEAR(quiet.readouts()[0], 200.f, 1.f);
    auto down = rig();
    down.set("frequency", 4000.f);
    down.set("env_amount", -36.f);
    down.run_mono(tone(9600, 100.0, kRate, 0.9));
    CHECK(down.readouts()[0] < 4000.f / 4.f);
}

TEST_CASE(auto_filter_lfo_sweeps_free_synced_and_stereo) {
    // Free 2 Hz, 24 st: the cutoff readout swings between ~f/4 and ~4f.
    auto r = rig();
    r.set("frequency", 1000.f);
    r.set("lfo_amount", 24.f);
    r.set("lfo_rate_free", 2.f);
    float lo = 1e9f, hi = 0.f;
    for (int i = 0; i < 48; ++i) {
        r.run_mono(Signal(512, 0.0f));
        lo = std::min(lo, r.readouts()[0]);
        hi = std::max(hi, r.readouts()[0]);
    }
    CHECK(lo < 300.f && hi > 3000.f);

    // Stereo 180: left and right filter differently at the same moment.
    auto s = rig();
    s.set("frequency", 1000.f);
    s.set("lfo_amount", 24.f);
    s.set("lfo_rate_free", 0.5f);
    s.set("lfo_stereo", 180.f);
    const Signal x = noise(24000, 0.3f);
    auto [l, rr] = s.run(x, x);
    CHECK(std::fabs(db(rms(l, 12000) / rms(rr, 12000))) > 1.0);

    // Sync: one cycle per bar at 120 BPM (1/1 = 2 s): equal readouts a cycle apart.
    auto y = rig();
    y.set("frequency", 1000.f);
    y.set("lfo_amount", 24.f);
    y.set("lfo_sync", 1.f);
    y.set("lfo_rate_sync", 0.f);
    std::vector<float> seen;
    for (int i = 0; i < 400; ++i) {
        y.run_mono(Signal(480, 0.0f));
        seen.push_back(y.readouts()[0]);
    }
    CHECK_NEAR(seen[50], seen[250], 0.02f * seen[50]);
    CHECK(std::fabs(seen[50] - seen[150]) > 0.2f * seen[50]);
}

TEST_CASE(auto_filter_synced_lfo_continues_through_automation_splits) {
    // While playing, automation events split blocks into segments; the synced
    // LFO must keep following the beat inside a block, not restart at each piece.
    auto plain = rig();
    auto split = rig();
    for (DeviceRig* r : {&plain, &split}) {
        r->set("frequency", 1000.f);
        r->set("lfo_amount", 36.f);
        r->set("lfo_sync", 1.f);
        r->set("lfo_rate_sync", 10.f); // 1/16: fast enough to move inside a block
        r->set_transport(true, 120.0);
    }
    const uint32_t mix = param_index(split.desc(), "mix");
    std::vector<std::pair<std::size_t, std::pair<uint32_t, float>>> ev;
    for (std::size_t b = 0; b < 40; ++b) {
        ev.push_back({b * 256 + 64, {mix, 100.f}});
        ev.push_back({b * 256 + 192, {mix, 100.f}});
    }
    const Signal x = noise(40 * 256, 0.3f, 5);
    const Signal a = plain.run(x, x).first;
    const Signal b = split.run(x, x, nullptr, nullptr, ev).first;
    double diff = 0.0;
    for (std::size_t i = 0; i < x.size(); ++i) diff = std::max(diff, static_cast<double>(std::fabs(a[i] - b[i])));
    CHECK(diff < 1e-3);
}

TEST_CASE(auto_filter_dj_mode_is_open_at_centre_and_sweeps_both_ways) {
    auto c = rig();
    c.set("mode", 1.f);
    c.set("resonance", 0.5f);
    CHECK_NEAR(gain_at(c, 100.0), 0.0, 0.02);
    CHECK_NEAR(gain_at(c, 10000.0), 0.0, 0.02);
    auto left = rig();
    left.set("mode", 1.f);
    left.set("dj_position", -0.6f);
    CHECK(gain_at(left, 8000.0) < -20.0);
    CHECK_NEAR(gain_at(left, 60.0), 0.0, 0.5);
    auto right = rig();
    right.set("mode", 1.f);
    right.set("dj_position", 0.6f);
    CHECK(gain_at(right, 60.0) < -20.0);
}

TEST_CASE(auto_filter_drive_and_mix) {
    auto clean = rig();
    clean.set("frequency", 5000.f);
    const Signal x = tone(24000, 200.0, kRate, 0.5);
    const Signal a = clean.run_mono(x);
    auto driven = rig();
    driven.set("frequency", 5000.f);
    driven.set("drive", 24.f);
    const Signal b = driven.run_mono(x);
    // Saturation compresses the peak relative to the RMS.
    CHECK(peak(b, 12000) / rms(b, 12000) < 0.9 * peak(a, 12000) / rms(a, 12000));
    CHECK(all_finite(b));
    auto half = rig();
    half.set("frequency", 200.f);
    half.set("mix", 50.f);
    // Far above the cutoff only the dry half remains.
    CHECK_NEAR(gain_at(half, 15000.0), -6.02, 0.2);
}
