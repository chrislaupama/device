// Echo: a physical multi-head echo modelled on three machines (tape loop,
// magnetic drum, cartridge), driven by the reference's machine constants
// (head positions in mm, tape speeds in mm/s, oscillation onsets).
// Parameters in the order of saine_core::builtin::echo.
//
// One "tape" (a delay buffer) is written at the record head; every playback
// head reads at delay = head position / tape speed, so the Motor bends all
// head times together (with inertia while Motor Glide is on). Heads that
// feed back return through the loop electronics: bass / treble, a per-pass
// treble loss set by Generations and tape speed, bias, smear (allpass
// diffusion that grows with every pass) and tape saturation. Around that:
// input drive, record level, hiss, wow & flutter, scrape flutter, splice
// dropouts once per loop revolution, a spring tank, per-side tail filters,
// ducking (input or sidechain key, optionally only the feedback), a Throw
// send boost, tempo sync of the first left / right heads, authored heads
// (h1..h6 blocks written by the card's head drags) and Sound-on-Sound.
#include "builtin.hpp"
#include "dsp.hpp"
#include "dsp_delay.hpp"

#include <algorithm>
#include <array>
#include <cmath>

namespace saine {

namespace {

constexpr int kHeadParams = 11;
enum Param : uint32_t {
    kMachine, kModeLoop, kModeDrum, kModeCart, kComboDrum, kMotor, kGlide, kSync, kNoteL, kNoteR, kIntensity, kMix, kMixLock,
    kKillDry, kTrim, kDrive, kRecLevel, kSpacing, kSpacingMorph, kBass, kTreble, kWidth, kHumanize, kGenerations, kSpringOn,
    kSpring, kHiss, kWowFlutter, kSplice, kBias, kScrape, kSmear, kTailOn, kTailHpL, kTailLpL, kTailHpR, kTailLpR, kDuckOn,
    kDuckAmount, kDuckThresh, kDuckAttack, kDuckRelease, kDuckKey, kDuckFbOnly, kThrow, kThrowLevel, kSeed, kMains60,
    kCartLate, kHeads, kHeadsMachine, kHead0, kPatch = kHead0 + 6 * kHeadParams, kParamCount
};
enum HeadParam : uint32_t { hPresent, hOn, hFb, hRight, hPos, hLevel, hPan, hTilt, hPanPinned, hPosPinned, hLevelBeforeMute };

constexpr const char* kParamIds[kParamCount] = {
    "machine", "mode_loop", "mode_drum", "mode_cartridge", "combo_drum", "motor", "motor_glide", "sync", "note_l", "note_r",
    "intensity", "mix", "mix_lock", "kill_dry", "trim", "input_drive", "rec_level", "spacing", "spacing_morph", "tone_bass",
    "tone_treble", "width", "width_humanize", "generations", "spring_on", "spring", "art_hiss", "art_wf", "art_splice",
    "art_bias", "art_scrape", "swell", "tail_on", "tail_hp_l", "tail_lp_l", "tail_hp_r", "tail_lp_r", "duck_on",
    "duck_amount", "duck_thresh", "duck_attack", "duck_release", "duck_key", "duck_fb_only", "throw", "throw_level", "seed",
    "drum_mains_60", "cartridge_late", "heads", "heads_machine",
#define SAINE_HEAD(n) "h" #n "_present", "h" #n "_on", "h" #n "_fb", "h" #n "_right", "h" #n "_pos", "h" #n "_level", "h" #n "_pan", \
                      "h" #n "_tilt", "h" #n "_pan_pinned", "h" #n "_pos_pinned", "h" #n "_level_before_mute"
    SAINE_HEAD(1), SAINE_HEAD(2), SAINE_HEAD(3), SAINE_HEAD(4), SAINE_HEAD(5), SAINE_HEAD(6),
#undef SAINE_HEAD
    "patch"};
// Readouts: wet peak (dB), ducking gain (dB, <= 0), tape speed (mm/s).
enum Readout : uint32_t { kWetDb, kDuckDb, kSpeed, kReadoutCount };
constexpr const char* kReadoutIds[kReadoutCount] = {"wet_db", "duck_db", "speed_mm_s"};

struct MachineSpec {
    float loop_mm, gap_mm;
    int heads;
    float head_mm[4];
    float onset, detent, spacing_um;
    float default_speed;
};
constexpr MachineSpec kMachines[3] = {
    {1600.0f, 18.0f, 3, {27.6f, 52.4f, 75.6f, 0.0f}, 7.4f, 0.45f, 3.0f, 260.0f},
    {388.28f, 40.0f, 4, {34.0f, 68.0f, 102.0f, 136.0f}, 8.1f, 0.30f, 1.5f, 459.46f},
    {2400.0f, 24.0f, 1, {162.56f, 0.0f, 0.0f, 0.0f}, 7.8f, 0.55f, 5.0f, 203.2f},
};
constexpr uint8_t kLoopMasks[12] = {1, 2, 4, 6, 1, 2, 4, 3, 6, 5, 7, 0};
constexpr uint8_t kDrumMasks[12] = {1, 2, 4, 8, 3, 6, 12, 5, 10, 7, 14, 15};
constexpr float kMaxSeconds = 14.0f;
constexpr float kSmearMs[2] = {7.3f, 12.9f};

float note_beats(int i) {
    constexpr float kBase[7] = {8.0f, 4.0f, 2.0f, 1.0f, 0.5f, 0.25f, 0.125f};
    const float b = kBase[std::clamp(i / 3, 0, 6)];
    return i % 3 == 0 ? b * 1.5f : (i % 3 == 1 ? b : b * 2.0f / 3.0f);
}

// Generations 0 .. detent .. 1 -> wear factor 0.35 .. 1 .. 6.
float wear(float gen, float detent) {
    if (gen <= detent) return 0.35f + 0.65f * (gen / std::max(detent, 1e-3f));
    return 1.0f + 5.0f * (gen - detent) / std::max(1.0f - detent, 1e-3f);
}

struct Head {
    float mm = 0.0f;   // distance from the record head
    float gain = 0.0f; // playback level (linear)
    float fb = 0.0f;   // feedback share (0 / 1)
    float pan = 0.0f;  // -1..1
    float tilt = 0.0f; // -1 dark .. +1 bright
    bool on = false;
};

class Echo final : public BuiltinProcessor {
public:
    void prepare(double sample_rate, uint32_t) override {
        sr_ = sample_rate;
        tape_.prepare(static_cast<std::size_t>(static_cast<double>(kMaxSeconds) * sr_) + 64);
        for (auto& a : smear_) a.allocate(static_cast<std::size_t>(0.03 * sr_));
        spring_.prepare(static_cast<std::size_t>(0.1 * sr_));
        reset();
    }

    void reset() noexcept override {
        tape_.reset();
        for (auto& a : smear_) a.clear();
        spring_.reset();
        for (auto& d : disp_) d.reset();
        for (auto& b : loop_eq_) b.reset();
        loop_lp_.reset();
        for (auto& c : tail_) {
            c[0].reset();
            c[1].reset();
        }
        for (auto& t : tilt_) t.reset();
        spring_lp_.reset();
        speed_ = -1.0f;
        env_db_ = -120.0f;
        duck_ = 1.0f;
        throw_g_ = 1.0f;
        wow_ = flutter_ = scrape_ = 0.0f;
        loop_phase_ = 0.0f;
        splice_env_ = 0.0f;
        rng_.seed(0x2468aceu);
        for (auto& k : key_) k = -1.0f;
        primed_ = false;
    }

    void process(float* l, float* r, uint32_t n, const float* p, const BuiltinContext& ctx, float* ro) noexcept override {
        const auto choice = [&](uint32_t i, int hi) { return std::clamp(static_cast<int>(std::lround(p[i])), 0, hi); };
        const int machine = choice(kMachine, 2);
        const MachineSpec& ms = kMachines[machine];
        const float motor = std::clamp(p[kMotor], 0.0f, 1.0f);
        const bool mains = p[kMains60] >= 0.5f;
        float target_speed = machine == 0 ? 120.0f + motor * 280.0f
                             : machine == 1 ? std::max(1.0f, 459.46f * (mains ? 1.2f : 1.0f) * (0.5f + motor))
                                            : 203.2f;

        // ---- heads ----
        Head heads[6];
        int count = build_heads(p, machine, motor, heads);
        bool spring_on = p[kSpringOn] >= 0.5f;
        if (machine == 0 && choice(kModeLoop, 11) >= 4) spring_on = true;
        const bool spring_only = machine == 0 && choice(kModeLoop, 11) == 11 && !authored(p, machine);

        // Sync: scale left / right head times so the first audible head on each side lands on its note.
        float scale_l = 1.0f, scale_r = 1.0f;
        if (p[kSync] >= 0.5f) {
            const double bpm = std::clamp(ctx.transport != nullptr ? ctx.transport->tempo_bpm : 120.0, 20.0, 999.0);
            const auto note_s = [&](int i) { return std::min(static_cast<float>(static_cast<double>(note_beats(i)) * 60.0 / bpm), kMaxSeconds - 0.1f); };
            const float tl = note_s(choice(kNoteL, 20)), tr = note_s(choice(kNoteR, 20));
            const Head* fl = nullptr;
            const Head* fr = nullptr;
            for (int i = 0; i < count; ++i) {
                if (!heads[i].on) continue;
                if (heads[i].pan <= 0.0f && (fl == nullptr || heads[i].mm < fl->mm)) fl = &heads[i];
                if (heads[i].pan > 0.0f && (fr == nullptr || heads[i].mm < fr->mm)) fr = &heads[i];
            }
            if (fl == nullptr) fl = fr;
            if (fr == nullptr) fr = fl;
            if (fl != nullptr) {
                if (machine != 2) target_speed = std::clamp(fl->mm / std::max(tl, 1e-3f), 5.0f, 5000.0f);
                const float base_l = fl->mm / target_speed;
                scale_l = tl / std::max(base_l, 1e-4f);
                scale_r = tr / std::max(fr->mm / target_speed, 1e-4f);
            }
        }
        target_speed = std::clamp(target_speed, 5.0f, 5000.0f);

        // ---- loop gain ----
        const float intensity = std::clamp(p[kIntensity], 0.0f, 10.0f);
        float wire = 1.0f;
        if (machine == 1) wire = std::exp(-2.2f * std::fabs(std::log(target_speed / ms.default_speed)));
        int feeding = 0;
        for (int i = 0; i < count; ++i) feeding += heads[i].fb > 0.0f ? 1 : 0;
        const float loop_gain = std::pow(10.0f, (intensity - ms.onset) * 3.0f / 20.0f) * (machine == 1 ? 0.5f + 0.5f * wire : 1.0f) / static_cast<float>(std::max(1, feeding));
        const bool sos = machine == 2 && choice(kModeCart, 1) == 1;

        // ---- per-block coefficients (only when their inputs change) ----
        const float gen = std::clamp(p[kGenerations], 0.0f, 1.0f);
        const float w = wear(gen, ms.detent);
        const float bias = std::clamp(p[kBias], -100.0f, 100.0f) * 0.01f;
        const float speed_ratio = target_speed / ms.default_speed;
        const float loop_fc = std::clamp(9000.0f / w * std::sqrt(std::max(speed_ratio, 0.05f)) * (1.0f - 0.35f * bias) * (3.0f / (ms.spacing_um + 0.5f)) * 0.75f, 600.0f, 20000.0f);
        update(0, loop_fc, [&] { loop_lp_.set_hz(loop_fc, sr_); });
        const float bass = std::clamp(p[kBass], -10.0f, 10.0f), treble = std::clamp(p[kTreble], -10.0f, 10.0f);
        update(1, bass, [&] { loop_eq_[0].set(dsp::Biquad::kLowShelf, 120.0f, 0.7f, bass, sr_); });
        update(2, treble, [&] { loop_eq_[1].set(dsp::Biquad::kHighShelf, 3500.0f, 0.7f, treble, sr_); });
        const bool tail_on = p[kTailOn] >= 0.5f;
        const float tf[4] = {std::clamp(p[kTailHpL], 20.0f, 2000.0f), std::clamp(p[kTailLpL], 200.0f, 20000.0f), std::clamp(p[kTailHpR], 20.0f, 2000.0f),
                             std::clamp(p[kTailLpR], 200.0f, 20000.0f)};
        update(3, tf[0] + tf[1] * 3.1f + tf[2] * 7.7f + tf[3] * 13.3f, [&] {
            tail_[0][0].set(dsp::Biquad::kHighpass, tf[0], 0.707f, 0.0f, sr_);
            tail_[0][1].set(dsp::Biquad::kLowpass, tf[1], 0.707f, 0.0f, sr_);
            tail_[1][0].set(dsp::Biquad::kHighpass, tf[2], 0.707f, 0.0f, sr_);
            tail_[1][1].set(dsp::Biquad::kLowpass, tf[3], 0.707f, 0.0f, sr_);
        });
        spring_lp_.set_hz(5000.0f, sr_);
        for (auto& t : tilt_) t.set_hz(1800.0f, sr_);

        const float drive = std::pow(10.0f, (std::clamp(p[kDrive], 0.0f, 10.0f) - 5.0f) * 2.4f / 20.0f);
        const float rec = std::pow(10.0f, (std::clamp(p[kRecLevel], 0.0f, 10.0f) - 5.0f) * 2.0f / 20.0f);
        const float sat = (1.0f + 0.6f * std::max(0.0f, -bias)) * rec * (0.7f + 0.3f * w);
        const float trim = dsp::db_to_gain(std::clamp(p[kTrim], -24.0f, 12.0f));
        const float mix = std::clamp(p[kMix], 0.0f, 100.0f) * 0.01f;
        const bool kill_dry = p[kKillDry] >= 0.5f;
        const float hiss = std::clamp(p[kHiss], 0.0f, 100.0f) * 0.01f * 0.0004f * (0.6f + 0.4f * w);
        const float wf = std::clamp(p[kWowFlutter], 0.0f, 100.0f) * 0.01f * (0.4f + 0.6f * std::min(w, 2.0f) * 0.5f);
        const float scrape = std::clamp(p[kScrape], 0.0f, 100.0f) * 0.01f;
        const float splice = std::clamp(p[kSplice], 0.0f, 100.0f) * 0.01f;
        const float smear = std::clamp(p[kSmear], 0.0f, 100.0f) * 0.01f;
        const float spring_amt = spring_on ? std::clamp(p[kSpring], 0.0f, 100.0f) * 0.01f : 0.0f;
        const float width = std::clamp(p[kWidth], 0.0f, 200.0f) * 0.01f;
        const bool duck_on = p[kDuckOn] >= 0.5f;
        const float duck_amount = std::clamp(p[kDuckAmount], 0.0f, 100.0f) * 0.01f;
        const float duck_thresh = std::clamp(p[kDuckThresh], -60.0f, 0.0f);
        const float duck_att = dsp::pole(std::clamp(p[kDuckAttack], 0.5f, 100.0f), sr_);
        const float duck_rel = dsp::pole(std::clamp(p[kDuckRelease], 20.0f, 1000.0f), sr_);
        const bool ext_key = choice(kDuckKey, 1) == 1 && ctx.sc_l != nullptr;
        const bool fb_only = p[kDuckFbOnly] >= 0.5f;
        const float throw_target = p[kThrow] >= 0.5f ? dsp::db_to_gain(std::clamp(p[kThrowLevel], -24.0f, 12.0f)) : 1.0f;
        const float glide = dsp::pole(p[kGlide] >= 0.5f ? 450.0f : 15.0f, sr_);
        const float smooth = dsp::pole(20.0f, sr_);

        // Equal-power pan gains of each head.
        float gl[6], gr[6];
        for (int i = 0; i < count; ++i) {
            const float pan = std::clamp(heads[i].pan * width, -1.0f, 1.0f);
            const float a = (pan + 1.0f) * 0.25f * dsp::kPi;
            gl[i] = std::cos(a) * heads[i].gain;
            gr[i] = std::sin(a) * heads[i].gain;
        }
        if (!primed_) {
            speed_ = target_speed;
            mix_ = mix;
            primed_ = true;
        }

        float wet_peak = 0.0f, duck_min = 1.0f;
        const float loop_s = ms.loop_mm / std::max(speed_, 1.0f);
        for (uint32_t i = 0; i < n; ++i) {
            const float xl = l[i], xr = r[i];
            speed_ = target_speed + glide * (speed_ - target_speed);
            throw_g_ = throw_target + smooth * (throw_g_ - throw_target);
            mix_ = mix + smooth * (mix_ - mix);
            // Transport wobble: wow (~0.6 Hz), flutter (~7 Hz) and scrape (fast, noisy).
            wow_ += 0.6f / static_cast<float>(sr_);
            flutter_ += 7.3f / static_cast<float>(sr_);
            if (wow_ >= 1.0f) wow_ -= 1.0f;
            if (flutter_ >= 1.0f) flutter_ -= 1.0f;
            scrape_ = 0.995f * scrape_ + 0.005f * rng_.bi();
            const float speed_mod = 1.0f + wf * (0.0025f * dsp::fast_sin(wow_) + 0.0007f * dsp::fast_sin(flutter_)) + scrape * 0.002f * scrape_;
            const float spd = speed_ * speed_mod;
            // Splice: a short dip once per loop revolution.
            loop_phase_ += 1.0f / (std::max(loop_s, 0.05f) * static_cast<float>(sr_));
            if (loop_phase_ >= 1.0f) {
                loop_phase_ -= 1.0f;
                splice_env_ = 1.0f;
            }
            splice_env_ *= 0.9985f;
            const float splice_g = 1.0f - splice * 0.45f * splice_env_;

            // Ducking key and envelope (dB).
            const float key = ext_key ? std::max(std::fabs(ctx.sc_l[i]), std::fabs(ctx.sc_r[i])) : std::max(std::fabs(xl), std::fabs(xr));
            const float kdb = dsp::gain_to_db(key);
            env_db_ = kdb + (kdb > env_db_ ? duck_att : duck_rel) * (env_db_ - kdb);
            float duck_t = 1.0f;
            if (duck_on) duck_t = 1.0f - duck_amount * std::clamp((env_db_ - duck_thresh) / 6.0f, 0.0f, 1.0f);
            duck_ = duck_t + (duck_t < duck_ ? duck_att : duck_rel) * (duck_ - duck_t);
            duck_min = std::min(duck_min, duck_);

            // Playback heads.
            float out_l = 0.0f, out_r = 0.0f, fb = 0.0f;
            for (int h = 0; h < count; ++h) {
                if (!heads[h].on && heads[h].fb <= 0.0f) continue;
                const float sc = heads[h].pan > 0.0f ? scale_r : scale_l;
                const float d = heads[h].mm / spd * sc * static_cast<float>(sr_);
                float v = tape_.read(d + 1.0f) * splice_g;
                if (heads[h].tilt != 0.0f) {
                    const float lo = tilt_[h].lp(v);
                    v = lo + (v - lo) * (1.0f + heads[h].tilt);
                }
                if (heads[h].on) {
                    out_l += v * gl[h];
                    out_r += v * gr[h];
                }
                fb += v * heads[h].fb;
            }
            out_l += hiss * rng_.bi();
            out_r += hiss * rng_.bi();

            // Loop electronics.
            float loop = fb * loop_gain * (fb_only ? duck_ : 1.0f);
            loop = loop_eq_[1].process(loop_eq_[0].process(loop));
            loop = loop_lp_.lp(loop);
            if (smear > 0.0f) {
                for (int k = 0; k < 2; ++k) loop = smear_[k].process(loop, kSmearMs[k] * 0.001f * static_cast<float>(sr_), 0.6f * smear);
            }
            // Record: input amp, throw, saturation onto tape.
            const float in = dsp::soft_clip(0.5f * (xl + xr) * drive) * throw_g_;
            float wr = in + loop;
            if (sos) wr += tape_.read(ms.loop_mm / spd * static_cast<float>(sr_) - 1.0f) * 0.82f / w;
            wr = dsp::soft_clip(wr * sat) / sat;
            tape_.push(spring_only ? 0.0f : dsp::flush(wr));

            // Spring tank on the echoes (and the input in "spring only").
            if (spring_amt > 0.0f) {
                const float s_len = 0.047f * static_cast<float>(sr_);
                float y = spring_lp_.lp(spring_.read_linear(s_len - 1.0f));
                float x = (spring_only ? in : 0.5f * (out_l + out_r) + 0.3f * in) + y * 0.72f;
                for (auto& ap : disp_) x = ap.process(x, -0.6f);
                spring_.push(dsp::flush(x));
                out_l += y * spring_amt * 1.2f;
                out_r += y * spring_amt * 1.15f;
            }
            if (tail_on) {
                out_l = tail_[0][1].process(tail_[0][0].process(out_l));
                out_r = tail_[1][1].process(tail_[1][0].process(out_r));
            }
            const float dg = (duck_on && !fb_only) ? duck_ : 1.0f;
            out_l *= trim * dg;
            out_r *= trim * dg;
            wet_peak = std::max(wet_peak, std::max(std::fabs(out_l), std::fabs(out_r)));
            const float dry = kill_dry ? 0.0f : 1.0f - mix_;
            l[i] = xl * dry + out_l * mix_;
            r[i] = xr * dry + out_r * mix_;
        }
        for (auto& b : loop_eq_) b.flush_state();
        for (auto& c : tail_) {
            c[0].flush_state();
            c[1].flush_state();
        }
        loop_lp_.y = dsp::flush(loop_lp_.y);
        spring_lp_.y = dsp::flush(spring_lp_.y);
        for (auto& t : tilt_) t.y = dsp::flush(t.y);
        ro[kWetDb] = dsp::gain_to_db(wet_peak);
        ro[kDuckDb] = dsp::gain_to_db(duck_min);
        ro[kSpeed] = speed_;
    }

private:
    template <class F> void update(int slot, float key, F&& f) noexcept {
        if (key_[slot] != key) {
            key_[slot] = key;
            f();
        }
    }

    static bool authored(const float* p, int machine) noexcept {
        const int hm = std::clamp(static_cast<int>(std::lround(p[kHeadsMachine])), 0, 3);
        return std::lround(p[kHeads]) >= 1 && hm == machine + 1;
    }

    // Heads of the current machine and mode: positions (mm), levels, feedback, pans.
    int build_heads(const float* p, int machine, float motor, Head* heads) noexcept {
        const MachineSpec& ms = kMachines[machine];
        const float humanize = std::clamp(p[kHumanize], 0.0f, 100.0f) * 0.01f;
        // Pan pattern: alternating sides, made irregular by Humanize (seeded).
        dsp::UnitRng prng;
        prng.seed(0x70616e2eu ^ static_cast<uint32_t>(std::clamp(p[kSeed], 0.0f, 65535.0f)) * 2654435761u);
        float pattern[6];
        for (int i = 0; i < 6; ++i) {
            const float side = (i & 1) ? 1.0f : -1.0f;
            float v = std::clamp(side * (1.0f - humanize) + prng.bi() * humanize, -1.0f, 1.0f);
            if (i > 0 && std::fabs(v - pattern[i - 1]) < 0.15f) v = std::clamp(v + (v >= pattern[i - 1] ? 0.15f : -0.15f), -1.0f, 1.0f);
            pattern[i] = v;
        }
        if (authored(p, machine)) {
            const int count = std::clamp(static_cast<int>(std::lround(p[kHeads])), 1, 6);
            const float span = std::max(1.0f, ms.loop_mm - ms.gap_mm - 1.0f - 0.1f);
            for (int i = 0; i < count; ++i) {
                const float* h = p + kHead0 + i * kHeadParams;
                Head& hd = heads[i];
                if (h[hPresent] < 0.5f) continue;
                hd.mm = 0.1f + std::clamp(h[hPos], 0.0f, 1.0f) * span;
                const float lv = std::clamp(h[hLevel], 0.0f, 1.0f);
                hd.gain = lv <= 0.01f ? 0.0f : dsp::db_to_gain(-60.0f + 66.0f * (lv - 0.01f) / 0.99f);
                hd.on = h[hOn] >= 0.5f && hd.gain > 0.0f;
                hd.fb = h[hFb] >= 0.5f ? 1.0f : 0.0f;
                hd.pan = h[hPanPinned] >= 0.5f ? std::clamp(h[hPan], -1.0f, 1.0f) : (h[hRight] >= 0.5f ? std::fabs(pattern[i]) : -std::fabs(pattern[i]));
                hd.tilt = std::clamp(h[hTilt], -1.0f, 1.0f);
            }
            return count;
        }
        // Authentic machine heads.
        uint8_t mask = 1;
        uint8_t fb_mask = 0;
        if (machine == 0) {
            mask = kLoopMasks[std::clamp(static_cast<int>(std::lround(p[kModeLoop])), 0, 11)];
            fb_mask = mask;
        } else if (machine == 1) {
            mask = kDrumMasks[std::clamp(static_cast<int>(std::lround(p[kComboDrum])), 0, 11)];
            const int sel = std::clamp(static_cast<int>(std::lround(p[kModeDrum])), 0, 2);
            fb_mask = sel == 0 ? 0 : (sel == 1 ? mask : 0x0f);
        } else {
            fb_mask = 1;
        }
        const int count = ms.heads;
        const int spacing = std::clamp(static_cast<int>(std::lround(p[kSpacing])), 0, 4);
        const float morph = std::clamp(p[kSpacingMorph], 0.0f, 1.0f);
        for (int i = 0; i < count; ++i) {
            Head& hd = heads[i];
            float mm = ms.head_mm[i];
            if (machine == 2) mm = ms.head_mm[0] - motor * 146.3f;
            if (spacing > 0 && i > 0) {
                const float k = static_cast<float>(i);
                const float f = spacing == 1 ? k + 1.0f : spacing == 2 ? 1.0f + 2.0f / 3.0f * k : spacing == 3 ? 1.0f + 1.618f * k : 1.0f + 2.414f * k;
                mm += morph * (ms.head_mm[0] * f - mm);
            }
            hd.mm = std::clamp(mm, 0.1f, ms.loop_mm - ms.gap_mm - 1.0f);
            hd.on = ((mask >> i) & 1) != 0;
            hd.gain = 1.0f;
            hd.fb = ((fb_mask >> i) & 1) != 0 ? 1.0f : 0.0f;
            hd.pan = count == 1 ? 0.0f : pattern[i];
        }
        return count;
    }

    double sr_ = 48000.0;
    dsp::DelayLine tape_;
    dsp::Allpass smear_[2];
    dsp::DelayLine spring_;
    struct Disp {
        float x1 = 0.0f, y1 = 0.0f;
        float process(float x, float a) noexcept {
            const float y = a * x + x1 - a * y1;
            x1 = x;
            y1 = dsp::flush(y);
            return y;
        }
        void reset() noexcept { x1 = y1 = 0.0f; }
    };
    Disp disp_[16];
    dsp::Biquad loop_eq_[2];
    dsp::ExpOnePole loop_lp_;
    dsp::Biquad tail_[2][2];
    dsp::ExpOnePole tilt_[6];
    dsp::ExpOnePole spring_lp_;
    float key_[4] = {};
    float speed_ = -1.0f, env_db_ = -120.0f, duck_ = 1.0f, throw_g_ = 1.0f, mix_ = 0.0f;
    float wow_ = 0.0f, flutter_ = 0.0f, scrape_ = 0.0f, loop_phase_ = 0.0f, splice_env_ = 0.0f;
    dsp::UnitRng rng_;
    bool primed_ = false;
};

std::unique_ptr<BuiltinProcessor> create() { return std::make_unique<Echo>(); }

} // namespace

extern const BuiltinDescriptor kEchoDescriptor;
constexpr float kIdle[kReadoutCount] = {-120.0f, 0.0f, 0.0f};
// Defaults: the machine and tape params, six authored-head blocks, then the seed.
constexpr float kMainDefaults[] = {0.f, 3.f, 1.f, 0.f, 11.f, 0.5f, 1.f, 0.f, 10.f, 12.f, 5.f, 35.f, 0.f, 0.f, 0.f, 5.f, 5.f,
                                   0.f, 1.f, 0.f, 0.f, 100.f, 45.f, 0.45f, 0.f, 25.f, 100.f, 100.f, 100.f, 0.f, 100.f, 0.f,
                                   1.f, 20.f, 6200.f, 240.f, 20000.f, 0.f, 60.f, -24.f, 5.f, 180.f, 0.f, 0.f, 0.f, 6.f,
                                   20673.f, 0.f, 0.f, 0.f, 0.f};
constexpr float kHeadDefaults[] = {0.f, 0.f, 1.f, 0.f, 0.f, 0.f, 0.f, 0.f, 0.f, 1.f, 0.f};
static_assert(std::size(kMainDefaults) + 6 * std::size(kHeadDefaults) + 1 == kParamCount);
constexpr std::array<float, kParamCount> kDefaults = [] {
    std::array<float, kParamCount> v{};
    std::size_t i = 0;
    for (float x : kMainDefaults) v[i++] = x;
    for (int h = 0; h < 6; ++h)
        for (float x : kHeadDefaults) v[i++] = x;
    return v;
}();
const BuiltinDescriptor kEchoDescriptor{"echo", kParamIds, kReadoutIds, &create, kIdle, kDefaults};

} // namespace saine
