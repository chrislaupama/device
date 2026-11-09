// Shaper (engine/src/devices/shaper.cpp): curve law and depth, each lane's
// effect (volume, time, width, pan, filter, crush, pitch), band split, audio
// trigger, latency reporting and transparency, through the shared harness.
#include "device_harness.hpp"
#include "shaper_layout.hpp"

#include <array>
#include <set>
#include <string_view>

using namespace devtest;

namespace {

constexpr double kRate = 48000.0;
constexpr std::size_t kQuarter = 24000; // 120 BPM
enum { kVol, kTime, kWidth, kPan, kFilter, kCrush, kPitch };
enum { kOn, kRateP, kDepth, kBand, kSplit, kPoints };

DeviceRig rig() {
    DeviceRig r("shaper", builtin_defaults("shaper"), kRate, 256);
    r.set_transport(true, 120.0, 0);
    return r;
}

void setp(DeviceRig& r, std::size_t index, float v) {
    r.instance().set_value(static_cast<uint32_t>(index), v);
}
void lane(DeviceRig& r, int l, int offset, float v) { setp(r, shaper_lane(l, offset), v); }

// Replace a lane's shape: points (x, y in the lane's unit, bend %).
void shape(DeviceRig& r, int l, std::initializer_list<std::array<float, 3>> pts) {
    lane(r, l, kPoints, static_cast<float>(pts.size()));
    int k = 0;
    for (const auto& p : pts) {
        setp(r, shaper_point(l, k, 0), p[0]);
        setp(r, shaper_point(l, k, 1), p[1]);
        setp(r, shaper_point(l, k, 2), p[2]);
        ++k;
    }
}
void only(DeviceRig& r, int l) {
    for (int i = 0; i < 7; ++i) lane(r, i, kOn, i == l ? 1.f : 0.f);
}

Signal dc(std::size_t n, float v = 1.0f) { return Signal(n, v); }

template <class F> bool all_equal(const Signal& a, const Signal& b, F&& offset) {
    for (std::size_t i = 0; i < a.size(); ++i) {
        const std::size_t j = offset(i);
        if (j < b.size() && a[i] != b[j]) return false;
    }
    return true;
}

// Zero crossings per second of a signal section.
double crossings(const Signal& x, std::size_t from, std::size_t to) {
    std::size_t n = 0;
    for (std::size_t i = from + 1; i < to; ++i)
        if ((x[i - 1] < 0.0f) != (x[i] < 0.0f)) ++n;
    return static_cast<double>(n) * kRate / static_cast<double>(to - from);
}

} // namespace

TEST_CASE(shaper_default_volume_pump_follows_the_beat) {
    auto r = rig();
    const Signal out = r.run_mono(dc(2 * kQuarter));
    // Default Volume shape: 0 at the beat, bent up to full by half a beat.
    CHECK(std::fabs(out[120]) < 0.1f); // 0.6 ms declick smoothing
    CHECK_NEAR(out[kQuarter / 2 + 1000], 1.0, 1e-3);
    CHECK_NEAR(out[kQuarter - 100], 1.0, 1e-3);
    CHECK(std::fabs(out[kQuarter + 120]) < 0.1f); // the next beat ducks again
    // Positive bend: above the straight line a quarter of the way in.
    CHECK(out[kQuarter / 4] > 0.85f);
}

TEST_CASE(shaper_depth_zero_and_lanes_off_are_transparent) {
    auto r = rig();
    lane(r, kVol, kDepth, 0.f);
    const Signal x = tone(kQuarter, 330.0, kRate, 0.5);
    const Signal y = r.run_mono(x);
    CHECK(all_equal(x, y, [](std::size_t i) { return i; }));
    auto off = rig();
    only(off, -1);
    const Signal z = off.run_mono(x);
    CHECK(all_equal(x, z, [](std::size_t i) { return i; }));
    CHECK(off.instance().latency() == 0);
    // A high-pass Filter lane at Depth 0 rests at the top cutoff, but stays neutral.
    auto hp = rig();
    only(hp, kFilter);
    setp(hp, 5, 2.f); // Filter Type: High-pass
    lane(hp, kFilter, kDepth, 0.f);
    const Signal w = hp.run_mono(x);
    CHECK(all_equal(x, w, [](std::size_t i) { return i; }));
}

TEST_CASE(shaper_audio_trigger_restarts_the_cycle) {
    auto r = rig();
    setp(r, 0, 1.f);   // Trigger: Audio
    setp(r, 1, -20.f); // Threshold
    // Quiet, then a loud hit at 30000: the pump restarts there.
    Signal x = dc(60000, 0.05f);
    for (std::size_t i = 30000; i < x.size(); ++i) x[i] = 0.5f;
    const Signal y = r.run_mono(x);
    CHECK_NEAR(y[20000], 0.05, 1e-3); // never triggered: rests at the curve's end (full)
    CHECK(std::fabs(y[30100]) < 0.1f); // ducked right after the hit
    CHECK_NEAR(y[30000 + kQuarter - 200], 0.5, 1e-3);
    CHECK_NEAR(y[59000], 0.5, 1e-3); // holds the end value until the next trigger
}

TEST_CASE(shaper_width_and_pan) {
    const Signal l = tone(kQuarter, 300.0, kRate, 0.5), rr = tone(kQuarter, 410.0, kRate, 0.5);
    auto mono = rig();
    only(mono, kWidth);
    shape(mono, kWidth, {{0, 0, 0}, {1, 0, 0}}); // 0 % = mono
    auto [ml, mr] = mono.run(l, rr);
    for (std::size_t i = 2000; i < ml.size(); i += 101) CHECK_NEAR(ml[i], mr[i], 1e-5);
    auto right = rig();
    only(right, kPan);
    shape(right, kPan, {{0, 100, 0}, {1, 100, 0}}); // hard right
    auto [pl, pr] = right.run(l, rr);
    CHECK(peak(pl, 2000) < 1e-4);
    CHECK_NEAR(peak(pr, 2000), peak(rr, 2000), 1e-3);
    auto centre = rig();
    only(centre, kPan);
    shape(centre, kPan, {{0, 50, 0}, {1, 50, 0}});
    auto [cl, cr] = centre.run(l, rr);
    CHECK_NEAR(cl[5000], l[5000], 1e-5);
    CHECK_NEAR(cr[5000], rr[5000], 1e-5);
}

TEST_CASE(shaper_filter_lane_sweeps_between_low_and_high) {
    const Signal hi = tone(2 * kQuarter, 3000.0, kRate, 0.5);
    auto closed = rig();
    only(closed, kFilter);
    shape(closed, kFilter, {{0, 0, 0}, {1, 0, 0}}); // at Filter Low (150 Hz)
    const Signal c = closed.run_mono(hi);
    CHECK(db(rms(c, kQuarter) / rms(hi, kQuarter)) < -45.0);
    auto open = rig();
    only(open, kFilter);
    shape(open, kFilter, {{0, 100, 0}, {1, 100, 0}}); // at Filter High (12 kHz)
    const Signal o = open.run_mono(hi);
    CHECK(std::fabs(db(rms(o, kQuarter) / rms(hi, kQuarter))) < 1.0);
    auto hp = rig();
    only(hp, kFilter);
    setp(hp, 5, 2.f); // High-pass
    shape(hp, kFilter, {{0, 100, 0}, {1, 100, 0}});
    const Signal h = hp.run_mono(tone(2 * kQuarter, 300.0, kRate, 0.5));
    CHECK(db(rms(h, kQuarter) / 0.3535) < -40.0);
}

TEST_CASE(shaper_filter_depth_change_is_smooth) {
    auto r = rig();
    only(r, kFilter);
    setp(r, 5, 2.f); // High-pass at Filter High: removes a 100 Hz tone
    shape(r, kFilter, {{0, 100, 0}, {1, 100, 0}});
    lane(r, kFilter, kDepth, 0.f);
    const Signal x = tone(9600, 100.0, kRate, 0.5);
    const Signal a = r.run_mono(Signal(x.begin(), x.begin() + 4800));
    lane(r, kFilter, kDepth, 100.f);
    const Signal b = r.run_mono(Signal(x.begin() + 4800, x.end()));
    Signal y = a;
    y.insert(y.end(), b.begin(), b.end());
    float jump = 0.0f;
    for (std::size_t i = 1; i < y.size(); ++i) jump = std::max(jump, std::fabs(y[i] - y[i - 1]));
    CHECK(jump < 0.02f); // a one-sample switch would jump by up to 0.5
    CHECK(rms(y, 7200) < 0.02);
}

TEST_CASE(shaper_pitch_switch_is_not_automatable) {
    auto r = rig();
    const auto& names = r.instance().descriptor().params;
    for (uint32_t i = 0; i < names.size(); ++i)
        CHECK(r.instance().param_automatable(i) == (std::string_view(names[i]) != "pitch_on"));
    CHECK(!r.instance().param_automatable(static_cast<uint32_t>(names.size())));
}

TEST_CASE(shaper_crush_lane_reduces_bits_and_rate) {
    auto r = rig();
    only(r, kCrush);
    shape(r, kCrush, {{0, 100, 0}, {1, 100, 0}});
    setp(r, 9, 2.f);   // 2 bits
    setp(r, 10, 4.f);  // hold each sample 4 times
    const Signal y = r.run_mono(tone(kQuarter, 100.0, kRate, 0.9));
    std::set<float> levels(y.begin() + 1000, y.end());
    CHECK(levels.size() <= 5);
    std::size_t changes = 0;
    for (std::size_t i = 1001; i < y.size(); ++i) changes += y[i] != y[i - 1] ? 1 : 0;
    CHECK(changes <= (y.size() - 1001) / 4 + 1);
    // Crush at 0: untouched.
    auto clean = rig();
    only(clean, kCrush);
    shape(clean, kCrush, {{0, 0, 0}, {1, 0, 0}});
    const Signal x = tone(kQuarter, 100.0, kRate, 0.9);
    CHECK(all_equal(x, clean.run_mono(x), [](std::size_t i) { return i; }));
}

TEST_CASE(shaper_time_lane_repeats_the_past) {
    auto r = rig();
    only(r, kTime);
    // First half: now; second half: half a cycle back (a repeat of the first half).
    shape(r, kTime, {{0, 0, 0}, {0.5f, 0, 0}, {0.5f, 50, 0}, {1, 50, 0}});
    const Signal x = impulse(kQuarter, 1.0f, 2000);
    const Signal y = r.run_mono(x);
    CHECK_NEAR(y[2000], 1.0, 1e-4);
    CHECK_NEAR(y[2000 + kQuarter / 2], 1.0, 0.02);
    // A flat Time curve at 0 reads "now" exactly.
    auto flat = rig();
    only(flat, kTime);
    shape(flat, kTime, {{0, 0, 0}, {1, 0, 0}});
    const Signal t = tone(kQuarter, 440.0, kRate, 0.5);
    CHECK(all_equal(t, flat.run_mono(t), [](std::size_t i) { return i; }));
}

TEST_CASE(shaper_pitch_lane_shifts_and_reports_latency) {
    auto r = rig();
    only(r, kPitch);
    shape(r, kPitch, {{0, 12, 0}, {1, 12, 0}}); // +12 st all cycle
    CHECK(r.instance().latency() == 962);       // 2 + 40 ms / 2 at 48 kHz
    const Signal y = r.run_mono(tone(2 * kQuarter, 220.0, kRate, 0.5));
    CHECK_NEAR(crossings(y, kQuarter, 2 * kQuarter), 880.0, 30.0);
    auto down = rig();
    only(down, kPitch);
    shape(down, kPitch, {{0, -12, 0}, {1, -12, 0}});
    const Signal d = down.run_mono(tone(2 * kQuarter, 440.0, kRate, 0.5));
    CHECK_NEAR(crossings(d, kQuarter, 2 * kQuarter), 440.0, 30.0);
    // 0 semitones: the plain signal, delayed by exactly the reported latency.
    auto unity = rig();
    only(unity, kPitch);
    shape(unity, kPitch, {{0, 0, 0}, {1, 0, 0}});
    const Signal x = tone(kQuarter, 440.0, kRate, 0.5);
    const Signal u = unity.run_mono(x);
    double worst = 0.0;
    for (std::size_t i = 962; i < u.size(); ++i) worst = std::max(worst, static_cast<double>(std::fabs(u[i] - x[i - 962])));
    CHECK(worst < 1e-5);
    lane(unity, kPitch, kOn, 0.f);
    unity.run_mono(Signal(unity.block(), 0.0f)); // the instance reports the latency of the last block's values too
    CHECK(unity.instance().latency() == 0);
}

TEST_CASE(shaper_band_split_limits_a_lane_to_lows_or_highs) {
    const Signal low = tone(2 * kQuarter, 60.0, kRate, 0.4), high = tone(2 * kQuarter, 5000.0, kRate, 0.4);
    Signal mix(low.size());
    for (std::size_t i = 0; i < mix.size(); ++i) mix[i] = low[i] + high[i];
    auto r = rig();
    shape(r, kVol, {{0, 0, 0}, {1, 0, 0}}); // silence...
    lane(r, kVol, kBand, 1.f);               // ...of the lows only
    lane(r, kVol, kSplit, 400.f);
    const Signal y = r.run_mono(mix);
    // What remains is the high tone (the LR split passes it at unity, phase-shifted).
    CHECK(std::fabs(db(rms(y, kQuarter) / rms(high, kQuarter))) < 0.5);
    CHECK(crossings(y, kQuarter, 2 * kQuarter) > 9000.0);
    // Band = Full with a neutral curve and Lows with a neutral curve sum flat.
    auto flat = rig();
    lane(flat, kVol, kDepth, 0.f);
    lane(flat, kVol, kBand, 2.f);
    const Signal f = flat.run_mono(mix);
    CHECK(std::fabs(db(rms(f, kQuarter) / rms(mix, kQuarter))) < 0.1);
}

TEST_CASE(shaper_mix_and_output) {
    auto r = rig();
    setp(r, 2, 50.f);  // Mix
    setp(r, 3, -6.f);  // Output
    shape(r, kVol, {{0, 0, 0}, {1, 0, 0}});
    const Signal y = r.run_mono(dc(kQuarter));
    CHECK_NEAR(y[kQuarter - 1], 0.5 * std::pow(10.0, -6.0 / 20.0), 1e-4);
}
