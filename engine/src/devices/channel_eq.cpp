// Channel EQ: eight bands (each Bell, Low Shelf, High Shelf, Notch, High
// Pass or Low Pass), shared high- and low-pass slopes (12..48 dB/oct), an
// output gain and a band listen (solo) switch. Parameters in the order of
// saine_core::builtin::channel_eq (checked by a Rust test).
//
// Each band is a cascade of TPT state-variable sections designed exactly as
// the panel's curve (eq_sections.hpp mirrors saine_core::builtin::eq). Band
// settings glide over ~20 ms, updated every 16 samples while moving, so
// sweeps and automation stay smooth; settled bands run whole blocks.
#include "builtin.hpp"
#include "dsp.hpp"
#include "eq_sections.hpp"

#include <algorithm>
#include <cmath>

namespace saine {

namespace {

constexpr int kBands = 8;
constexpr uint32_t kControl = 16; // samples per coefficient update while gliding

enum Param : uint32_t { kOutput = 32, kType0 = 33, kHpSlope = 41, kLpSlope = 42, kListen = 43, kParamCount = 44 };

constexpr const char* kParamIds[kParamCount] = {
    "band0_freq", "band0_gain2", "band0_q", "band0_enabled", "band1_freq", "band1_gain2", "band1_q", "band1_enabled",
    "band2_freq", "band2_gain2", "band2_q", "band2_enabled", "band3_freq", "band3_gain2", "band3_q", "band3_enabled",
    "band4_freq", "band4_gain2", "band4_q", "band4_enabled", "band5_freq", "band5_gain2", "band5_q", "band5_enabled",
    "band6_freq", "band6_gain2", "band6_q", "band6_enabled", "band7_freq", "band7_gain2", "band7_q", "band7_enabled",
    "output_gain", "band0_type", "band1_type", "band2_type", "band3_type", "band4_type", "band5_type", "band6_type",
    "band7_type", "hp_slope", "lp_slope", "listen"};

// Type choice index -> shape (Bell, Low Shelf, High Shelf, Notch, High Pass, Low Pass).
constexpr eq::Shape kShapes[6] = {eq::Shape::Bell, eq::Shape::LowShelf, eq::Shape::HighShelf,
                                  eq::Shape::Notch, eq::Shape::HighPass, eq::Shape::LowPass};

inline int choice(float v, int count) noexcept { return std::clamp(static_cast<int>(std::lround(v)), 0, count - 1); }

class ChannelEq final : public BuiltinProcessor {
public:
    void prepare(double sample_rate, uint32_t) override {
        fs_ = sample_rate;
        glide_ = static_cast<float>(std::exp(-static_cast<double>(kControl) / (0.02 * fs_)));
        output_.set_time(20.0f, fs_);
        reset();
    }

    void reset() noexcept override {
        for (eq::BandRunner& b : bands_) b.reset();
        listen_st_.clear();
        primed_ = false;
    }

    void process(float* l, float* r, uint32_t n, const float* p, const BuiltinContext&, float*) noexcept override {
        const float hp_slope = 12.0f * static_cast<float>(choice(p[kHpSlope], 4) + 1);
        const float lp_slope = 12.0f * static_cast<float>(choice(p[kLpSlope], 4) + 1);
        bool moving = false;
        for (int b = 0; b < kBands; ++b) {
            eq::BandRunner& br = bands_[b];
            eq::Band t;
            t.shape = kShapes[choice(p[kType0 + b], 6)];
            t.freq = std::clamp(p[4 * b], 10.0f, 24000.0f);
            t.gain_db = std::clamp(p[4 * b + 1], -48.0f, 30.0f);
            t.q = std::clamp(p[4 * b + 2], 0.05f, 40.0f);
            t.slope_db = t.shape == eq::Shape::HighPass ? hp_slope : t.shape == eq::Shape::LowPass ? lp_slope : 12.0f;
            br.target = t;
            br.on = p[4 * b + 3] >= 0.5f;
            moving |= br.target.freq != br.cur.freq || br.target.gain_db != br.cur.gain_db || br.target.q != br.cur.q ||
                      br.level != (br.on ? 1.0f : 0.0f) || !br.primed || br.target.shape != br.cur.shape ||
                      br.target.slope_db != br.cur.slope_db;
        }
        const float out = dsp::db_to_gain(std::clamp(p[kOutput], -48.0f, 48.0f));
        if (!primed_) {
            output_.snap(out);
            primed_ = true;
        }

        const int listen = choice(p[kListen], kBands + 1) - 1;
        if (listen >= 0) {
            const eq::Band& b = bands_[listen].primed ? bands_[listen].cur : bands_[listen].target;
            listen_c_ = eq::band_pass(b.freq, std::clamp(b.q, 0.5f, 8.0f), fs_);
            listen_c_.finish();
        }

        uint32_t pos = 0;
        while (pos < n) {
            // While a band glides, update coefficients every kControl samples; else run the rest at once.
            const uint32_t len = moving ? std::min(kControl, n - pos) : n - pos;
            moving = false;
            eq::Cascade chain;
            for (eq::BandRunner& br : bands_) {
                moving |= br.glide(glide_);
                if (br.dirty) br.redesign(fs_);
                br.add_to(chain);
            }
            float* bl = l + pos;
            float* bm = r + pos;
            if (listen >= 0) {
                eq::run(listen_c_, listen_st_, bl, bm, len);
            } else {
                chain.run(bl, bm, len);
            }
            for (uint32_t i = 0; i < len; ++i) {
                const float g = output_.next(out);
                bl[i] *= g;
                bm[i] *= g;
            }
            pos += len;
        }
        for (eq::BandRunner& br : bands_) br.sanitize_state();
        eq::sanitize(listen_st_);
    }

private:
    double fs_ = 48000.0;
    float glide_ = 0.0f;
    eq::BandRunner bands_[kBands];
    eq::Coef listen_c_{};
    eq::State listen_st_{};
    dsp::Smoother output_;
    bool primed_ = false;
};

std::unique_ptr<BuiltinProcessor> create_channel_eq() { return std::make_unique<ChannelEq>(); }

} // namespace

constexpr float kDefaults[kParamCount] = {20.0f, 0.0f, 0.71f, 0.0f, 75.0f, 0.0f, 1.0f, 1.0f, 100.0f, 0.0f, 0.6f, 1.0f, 250.0f, 0.0f, 0.3f, 1.0f, 1040.0f, 0.0f, 0.41f, 1.0f, 2500.0f, 0.0f, 0.2f, 1.0f, 7500.0f, 0.0f, 1.0f, 1.0f, 20000.0f, 0.0f, 0.71f, 0.0f, 0.0f, 4.0f, 1.0f, 0.0f, 0.0f, 0.0f, 0.0f, 2.0f, 5.0f, 0.0f, 0.0f, 0.0f};
extern const BuiltinDescriptor kChannelEqDescriptor;
const BuiltinDescriptor kChannelEqDescriptor{"channeleq", kParamIds, {}, &create_channel_eq, {}, kDefaults};

} // namespace saine
