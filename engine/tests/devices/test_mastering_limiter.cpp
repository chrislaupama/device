// Mastering Limiter (engine/src/devices/mastering_limiter.cpp): push and
// ceiling, true peak at every oversampling factor, latency per factor,
// styles, links, M/S, monitors, dither and bypass.
#include "device_harness.hpp"
#include "devices/dsp_dynamics.hpp"

using namespace devtest;

namespace {

// Defaults of saine_core::builtin::mastering_limiter, registry order.
const std::vector<float> kDefaults = {0.f, -1.f, 150.f, 1.f, 0.f, 2.f, 10.f, 75.f, 100.f, 0.f, 2.f, 1.f, 0.f, 2.f, 0.f, 0.f, 0.f};
constexpr double kRate = 48000.0;

DeviceRig rig() { return DeviceRig("mastering-limiter", kDefaults, kRate, 256); }

uint32_t expected_latency(uint32_t factor) {
    return saine::dsp::Oversampler::latency(factor) + 240 + (factor < 4 ? saine::dsp::TruePeak4x::kDelay / factor : 0) + 21;
}

double true_peak_db(const Signal& s, std::size_t from) {
    saine::dsp::TruePeak4x tp;
    double m = 0.0;
    for (std::size_t i = 0; i < s.size(); ++i) {
        const double v = tp.next(s[i]);
        if (i >= from) m = std::max(m, v);
    }
    return db(m);
}

// Loud, dense programme material: noise bursts over a bass tone.
Signal programme(std::size_t n) {
    Signal s = noise(n, 0.6f, 11);
    for (std::size_t i = 0; i < n; ++i)
        s[i] = s[i] * (0.4f + 0.6f * static_cast<float>((i / 2400) % 2)) + static_cast<float>(0.3 * std::sin(2 * saine::dsp::kPi * 60.0 * static_cast<double>(i) / kRate));
    return s;
}

} // namespace

TEST_CASE(mastering_limiter_latency_follows_the_oversampling_factor) {
    for (uint32_t i = 0; i < 4; ++i) {
        auto r = rig();
        r.set("oversampling", static_cast<float>(i));
        const uint32_t f = 1u << i;
        CHECK_EQ(r.instance().latency(), expected_latency(f));
        // A quiet tone comes out exactly that late (times the -1 dB push of the defaults).
        const Signal x = tone(9600, 1000.0, kRate, 0.1);
        const auto y = r.run_mono(x);
        const uint32_t lat = expected_latency(f);
        double err = 0.0;
        for (std::size_t k = 4800; k < y.size(); ++k) err = std::max(err, std::fabs(y[k] - x[k - lat] * std::pow(10.0, -1.0 / 20.0)));
        CHECK(err < 1e-3);
    }
}

TEST_CASE(mastering_limiter_holds_the_true_peak_ceiling_at_every_factor) {
    for (uint32_t i = 0; i < 4; ++i) {
        auto r = rig();
        r.set("oversampling", static_cast<float>(i));
        r.set("threshold", -12.f);
        const auto y = r.run_mono(programme(96000));
        CHECK(all_finite(y));
        CHECK(true_peak_db(y, 9600) <= -1.0 + 0.1);
        CHECK(db(peak(y, 9600)) > -2.0); // and not by squashing everything far under it
    }
}

TEST_CASE(mastering_limiter_pushes_quiet_material_to_the_ceiling) {
    auto r = rig();
    r.set("threshold", -10.f);
    const auto y = r.run_mono(tone(48000, 200.0, kRate, 0.05)); // -26 dB: no limiting
    CHECK_NEAR(db(peak(y, 24000) / 0.05), 9.0, 0.1);           // ceiling - threshold
    const auto ro = r.readouts();
    CHECK_NEAR(ro[2], 0.0, 0.01);
}

TEST_CASE(mastering_limiter_styles_and_links_change_the_reduction) {
    const auto reduction = [](float style, float t_link) {
        auto r = rig();
        r.set("style", style);
        r.set("threshold", -10.f);
        r.set("transient_link", t_link);
        Signal left = programme(48000), right = Signal(48000, 0.0f);
        const auto out = r.run(left, right);
        return std::make_pair(rms(out.first, 24000), rms(out.second, 24000));
    };
    // A gentler slow share keeps more level (less pumping) than Dense... both hold the ceiling,
    // and an unlinked fast stage leaves the quiet channel alone.
    const auto clean = reduction(0.f, 100.f), dense = reduction(3.f, 100.f);
    CHECK(clean.first != dense.first);
    auto linked = rig(), unlinked = rig();
    for (auto* r : {&linked, &unlinked}) {
        r->set("threshold", -10.f);
        r->set("release_link", 0.f);
    }
    linked.set("transient_link", 100.f);
    unlinked.set("transient_link", 0.f);
    Signal left = programme(48000), right = tone(48000, 300.0, kRate, 0.05);
    const double lr = rms(linked.run(left, right).second, 24000), ur = rms(unlinked.run(left, right).second, 24000);
    CHECK(ur > lr * 1.5);
}

TEST_CASE(mastering_limiter_mid_side_holds_the_ceiling) {
    auto r = rig();
    r.set("ms_mode", 1.f);
    r.set("threshold", -12.f);
    r.set("transient_link", 0.f);
    r.set("release_link", 0.f);
    Signal left = programme(48000), right = noise(48000, 0.7f, 5);
    const auto [l, rr] = r.run(left, right);
    CHECK(peak(l, 4800) <= std::pow(10.0, -1.0 / 20.0) + 1e-3);
    CHECK(peak(rr, 4800) <= std::pow(10.0, -1.0 / 20.0) + 1e-3);
}

TEST_CASE(mastering_limiter_monitors_and_bypass) {
    const Signal x = programme(48000);
    const uint32_t lat = expected_latency(4);
    auto normal = rig(), unity = rig();
    for (auto* r : {&normal, &unity}) r->set("threshold", -12.f);
    unity.set("unity_monitor", 1.f);
    // Unity removes the push (+11 dB) and keeps the limiting.
    CHECK_NEAR(db(rms(unity.run_mono(x), 24000) / rms(normal.run_mono(x), 24000)), -11.0, 0.01);
    auto delta = rig();
    delta.set("threshold", -1.f);
    delta.set("delta_monitor", 1.f);
    const auto d = delta.run_mono(tone(48000, 200.0, kRate, 0.05)); // nothing limited: nothing removed
    CHECK(peak(d, 24000) < 1e-3);
    auto bypass = rig();
    bypass.set("bypass", 1.f);
    bypass.set("threshold", -20.f);
    const auto b = bypass.run_mono(x);
    double err = 0.0;
    for (std::size_t i = lat; i < x.size(); ++i) err = std::max(err, static_cast<double>(std::fabs(b[i] - x[i - lat])));
    CHECK(err < 1e-6);
}

TEST_CASE(mastering_limiter_dither_quantises_to_the_word_length) {
    auto r = rig();
    r.set("dither_bits", 3.f); // 16
    r.set("dither_shape", 0.f);
    const auto y = r.run_mono(tone(48000, 440.0, kRate, 0.25));
    const double lsb = 1.0 / 32768.0;
    bool on_grid = true;
    for (std::size_t i = 1000; i < y.size(); ++i) on_grid &= std::fabs(y[i] / lsb - std::round(y[i] / lsb)) < 1e-3;
    CHECK(on_grid);
    // On silence the dither is a low noise floor (TPDF: about -96 dBFS RMS at 16 bits).
    auto s = rig();
    s.set("dither_bits", 3.f);
    s.set("dither_shape", 0.f);
    const double floor_db = db(rms(s.run_mono(Signal(48000, 0.0f)), 4800));
    CHECK(floor_db < -90.0);
    CHECK(floor_db > -105.0);
    // Shaping moves the noise up in frequency: more total noise, as designed.
    auto st = rig();
    st.set("dither_bits", 3.f);
    st.set("dither_shape", 3.f);
    CHECK(db(rms(st.run_mono(Signal(48000, 0.0f)), 4800)) > floor_db + 3.0);
}
