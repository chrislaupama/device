// Gate (engine/src/devices/gate.cpp): threshold and ratio, hold, attack /
// release, lookahead, detector high-pass, mix and readouts.
#include "device_harness.hpp"

using namespace devtest;

namespace {

// Defaults of saine_core::builtin::gate, registry order: threshold, ratio,
// attack, hold, release, lookahead, sidechain_hpf, mix.
const std::vector<float> kDefaults = {-40.f, 10.f, 0.5f, 50.f, 100.f, 5.f, 80.f, 100.f};
constexpr double kRate = 48000.0;
constexpr uint32_t kLatency = 960; // 20 ms

DeviceRig rig() { return DeviceRig("gate", kDefaults, kRate, 256); }

// Gain (dB) of the output relative to the input at the end of a run.
double end_gain_db(const Signal& y, const Signal& x) { return db(peak(y, y.size() - 960) / peak(x, x.size() - 960 - kLatency, x.size() - kLatency)); }

} // namespace

TEST_CASE(gate_opens_above_and_closes_below_the_threshold) {
    auto r = rig();
    CHECK_EQ(r.instance().latency(), kLatency);
    const Signal loud = tone(24000, 500.0, kRate, 0.1); // -20 dB
    CHECK_NEAR(end_gain_db(r.run_mono(loud), loud), 0.0, 0.01);
    auto c = rig();
    c.set("ratio", 100.f);
    const Signal quiet = tone(48000, 500.0, kRate, 0.003); // -50 dB
    CHECK(end_gain_db(c.run_mono(quiet), quiet) < -80.0);
}

TEST_CASE(gate_ratio_is_a_downward_expander) {
    auto r = rig();
    r.set("ratio", 2.f);
    r.set("hold", 0.f);
    r.set("release", 5.f);
    const Signal x = tone(48000, 500.0, kRate, 0.01); // -40 dB peak: 10 dB under -30
    r.set("threshold", -30.f);
    // 10 dB under the threshold at 2:1 = 10 dB more reduction.
    CHECK_NEAR(end_gain_db(r.run_mono(x), x), -10.0, 0.5);
}

TEST_CASE(gate_holds_then_releases) {
    auto r = rig();
    r.set("ratio", 100.f);
    r.set("hold", 100.f);
    r.set("release", 10.f);
    Signal x = concat(tone(9600, 500.0, kRate, 0.1), tone(19200, 500.0, kRate, 0.0005));
    const auto y = r.run_mono(x);
    const std::size_t stop = 9600 + kLatency;
    // Still open 80 ms after the signal fell under the threshold, closed 200 ms after.
    CHECK_NEAR(db(peak(y, stop + 3360, stop + 3840) / 0.0005), 0.0, 0.5);
    CHECK(db(peak(y, stop + 9600, stop + 10080) / 0.0005) < -60.0);
}

TEST_CASE(gate_lookahead_keeps_the_onset) {
    const auto onset_db = [](float lookahead) {
        auto r = rig();
        r.set("ratio", 100.f);
        r.set("lookahead", lookahead);
        r.set("attack", 1.f);
        Signal x = concat(Signal(9600, 0.0f), tone(4800, 1000.0, kRate, 0.5));
        const auto y = r.run_mono(x);
        return db(peak(y, 9600 + kLatency, 9600 + kLatency + 24) / peak(x, 9600, 9624));
    };
    CHECK(onset_db(0.f) < -6.0); // the first half millisecond is cut
    CHECK(onset_db(5.f) > -0.5);  // opened ahead of the hit
}

TEST_CASE(gate_sidechain_hpf_ignores_low_end) {
    const Signal bass = tone(48000, 40.0, kRate, 0.1); // -20 dB at 40 Hz, threshold -26
    auto low = rig();
    low.set("ratio", 100.f);
    low.set("threshold", -26.f);
    low.set("sidechain_hpf", 20.f);
    CHECK_NEAR(end_gain_db(low.run_mono(bass), bass), 0.0, 0.5);
    auto high = rig();
    high.set("ratio", 100.f);
    high.set("threshold", -26.f);
    high.set("sidechain_hpf", 500.f);
    CHECK(end_gain_db(high.run_mono(bass), bass) < -20.0);
}

TEST_CASE(gate_mix_and_readouts) {
    auto r = rig();
    r.set("ratio", 100.f);
    r.set("mix", 0.f);
    const Signal quiet = tone(48000, 500.0, kRate, 0.003);
    CHECK_NEAR(end_gain_db(r.run_mono(quiet), quiet), 0.0, 0.01);
    auto g = rig();
    g.set("ratio", 100.f);
    g.run_mono(quiet);
    const auto ro = g.readouts();
    REQUIRE(ro.size() == 3);
    CHECK_NEAR(ro[0], db(0.003), 0.1);
    CHECK(ro[2] < -60.0f);
    g.run_mono(Signal(48000, 0.0f));
    CHECK_EQ(g.readouts()[2], 0.0f); // nothing audible to reduce
}
