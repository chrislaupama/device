// Character Mod: eleven modulation circuits behind one set of controls.
// Parameters in the order of dstudio_core::builtin::character_mod (checked by
// a Rust test). The panel's
// Sweep view (crates/dstudio-app/src/device_fields.rs) draws the same
// delay / frequency / gain ranges, so keep the two in step.
//
// Shared signal flow (per sample):
//   modulation  m = (1 - a2) shape(p + drift + offset) + a2 sin(p2 + offset)
//               p: the LFO (Rate in Hz, or Note when synced); p2: a second
//               LFO from Voice (negative: 3.7x faster "sheen", positive:
//               0.31x slower "wobble", amount |voice| * 0.35); drift: a slow
//               random wander of phase and depth; offset: the right
//               channel's phase offset, scaled by Width
//   amount      depth / 50 (50% = the circuit's own depth, 100% = double),
//               x (1 + 0.5 voice+) x drift depth wander
//   input       Input Off / Lo / Hi drives the circuit 0 / +6 / +12 dB into a
//               level-compensated tanh (with Drive)
//   circuit     wet L / R (below), Hiss (filtered noise), Darken (12 dB/oct
//               low-pass), Wow (tape only)
//   stereo      wet mid x Depth-M, side x Depth-S x Width
//   out         equal-power dry / wet Mix, Output trim, Mono Audition sum
//
// Circuits (delays in ms at 50% depth; the LFO is a triangle on the BBD
// circuits, a sine elsewhere):
//   Single BBD   Chorus 6.5 +- 3 (right channel half a cycle later at full
//                Width); Vibrato 4 +- 2.2, the delayed signal alone
//   Crossed      two lines in anti-phase, cross-mixed; modes 1-4 move the
//                centre / swing / cross amount, 3 adds a second LFO, 4 a
//                second tap per line
//   Twin BBD     3.55 +- 1.85 (I, II) or +- 0.35 (I+II), left and right in
//                anti-phase: the classic twin-line chorus
//   Ensemble     three taps per channel 120 deg apart, each a slow + fast
//                (10.3x) vibrato, weighted across the stereo field
//   Tape         ADT: an 18 ms copy wandering +- 1.5 plus Wow; TZF Flange /
//                Phase: the dry path delayed by c = 1 + 2 ms and the copy
//                sweeping c +- 2 / +- 0.25 ms through it
//   Photocell    four all-pass stages (staggered) swept by a lamp with a
//                fast attack and slow decay; Chorus mixes them with the dry
//                signal, Vibrato is the shifted signal alone
//   Flanger      BBD flanger: Soft 0.6 + 2.2 (fb 0.45), Jet 0.35 + 3.5
//                (fb 0.78), Dark 0.5 + 3 (fb 0.6, 2.5 kHz in the loop)
//   Phaser       Four Plain / Four Peak (fb 0.55) / OTA Dark (saturating
//                stages, fb 0.35, darker) / Dual Six (two 6-stage chains in
//                quadrature) / Barber (three 6-stage chains rising in a
//                loop with raised-cosine crossfades: an endless rise)
//   Tremolo      Optical (lamp-smoothed sine), Bias (squarer, tube-like),
//                Harmonic (low and high band in anti-phase, 650 Hz split)
//   Rotary       800 Hz crossover; horn and drum rotors with their own
//                inertia (horn 0.6 s up / 0.9 s down, drum 3.2 / 4.5 s), each
//                with Doppler delay and amplitude modulation at two mics;
//                Slow = Rate x 0.12, Fast = Rate, Stop brakes to a halt
//   Panner       Pan: equal-power auto-pan; Rotary-Pan: a source circling
//                the listener (pan, plus darker, quieter and slightly
//                delayed when behind)
#include "builtin.hpp"
#include "dsp.hpp"

#include <algorithm>
#include <cmath>

namespace dstudio {

namespace {

enum Param : uint32_t {
    kModel,
    kModeSingle,
    kModeCrossed,
    kModeTwin,
    kModeTape,
    kModePhotocell,
    kModeFlanger,
    kModePhaser,
    kModeTrem,
    kModeRotary,
    kModePanner,
    kInput,
    kRate,
    kSync,
    kNote,
    kDepth,
    kMix,
    kWidth,
    kDepthM,
    kDepthS,
    kDrift,
    kVoice,
    kHiss,
    kDrive,
    kDarken,
    kWow,
    kTrim,
    kSeed,
    kMonoAudition,
    kPatch,
    kParamCount
};
enum Readout : uint32_t { kPhase, kReadoutCount };
enum Circuit : int { kSingle, kCrossed, kTwin, kEnsemble, kTape, kPhotocell, kFlanger, kPhaser, kTremolo, kRotary, kPanner };

constexpr const char* kParamIds[kParamCount] = {
    "model",  "mode_single", "mode_crossed", "mode_twin", "mode_tape", "mode_photocell", "mode_flanger", "mode_phaser",
    "mode_trem", "mode_rotary", "mode_panner", "input", "rate", "sync", "note", "depth", "mix", "width", "depth_m", "depth_s",
    "drift", "voice", "art_hiss", "art_drive", "art_darken", "art_wow", "trim", "seed", "mono_audition", "patch"};
constexpr const char* kReadoutIds[kReadoutCount] = {"lfo_phase"};
constexpr float kDefaults[kParamCount] = {2.f, 0.f, 0.f, 0.f, 0.f, 0.f, 0.f, 0.f, 0.f, 0.f, 0.f, 0.f, 0.513f, 0.f, 10.f,
                                          50.f, 50.f, 100.f, 100.f, 100.f, 35.f, 0.f, 35.f, 35.f, 35.f, 35.f, 0.f, 1.f, 0.f, 0.f};
constexpr float kMaxDelayMs = 48.0f;
constexpr uint32_t kControl = 16;
constexpr int kBarberChains = 3;
constexpr int kStages = 6;

// Which artifacts each circuit has (Hiss, Drive, Darken, Wow).
constexpr bool kHasHiss[] = {true, true, true, true, true, true, true, false, false, false, false};
constexpr bool kHasDrive[] = {true, true, true, true, true, true, true, true, true, true, false};
constexpr bool kHasDarken[] = {true, true, true, true, true, false, true, true, false, true, false};

struct CrossedMode {
    float centre, swing, cross, lfo2, tap2;
};
constexpr CrossedMode kCrossedModes[] = {{5.0f, 1.8f, 0.35f, 0.0f, 0.0f}, {7.5f, 2.6f, 0.5f, 0.0f, 0.0f}, {6.0f, 2.2f, 0.4f, 1.37f, 0.0f}, {9.0f, 2.8f, 0.5f, 0.0f, 0.62f}};

// First-order all-pass section: y = a x + s, s = x - a y.
inline float allpass(float x, float a, float& s) noexcept {
    const float y = a * x + s;
    s = x - a * y;
    return y;
}
// All-pass coefficient for a break frequency (tangent warp).
inline float ap_coef(float hz, float sr) noexcept {
    const float t = std::tan(dsp::kPi * std::clamp(hz, 10.0f, 0.45f * sr) / sr);
    return (t - 1.0f) / (t + 1.0f);
}

class CharacterMod final : public BuiltinProcessor {
public:
    void prepare(double sample_rate, uint32_t) override {
        sr_ = sample_rate;
        srf_ = static_cast<float>(sample_rate);
        ms_ = static_cast<float>(1e-3 * sample_rate);
        const auto len = static_cast<std::size_t>(std::ceil(kMaxDelayMs * ms_)) + 8;
        for (auto& d : line_) d.prepare(len);
        for (auto& d : dry_line_) d.prepare(len);
        for (auto& d : rot_line_) d.prepare(static_cast<std::size_t>(std::ceil(8.0f * ms_)) + 8);
        for (auto* s : {&amount_, &width_, &mix_dry_, &mix_wet_, &trim_, &depth_m_, &depth_s_, &drive_, &hiss_, &a2_})
            s->set_time(30.0f, sr_);
        darken_.set_time(4.0f, sr_); // stepped once per 16-sample control interval
        lamp_up_ = dsp::pole(8.0f, sr_);
        lamp_down_ = dsp::pole(35.0f, sr_);
        trem_up_ = dsp::pole(5.0f, sr_);
        trem_down_ = dsp::pole(40.0f, sr_);
        horn_up_ = dsp::pole(600.0f, sr_);
        horn_down_ = dsp::pole(900.0f, sr_);
        drum_up_ = dsp::pole(3200.0f, sr_);
        drum_down_ = dsp::pole(4500.0f, sr_);
        for (auto& f : xover_) f.set(800.0f, sr_);
        for (auto& f : split_) f.set(650.0f, sr_);
        for (auto& f : hiss_lp_) f.set(4000.0f, sr_);
        for (auto& f : loop_lp_) f.set(2500.0f, sr_);
        for (auto& f : ota_lp_) f.set(3200.0f, sr_);
        for (auto& f : back_lp_) f.set(2200.0f, sr_);
    }

    void reset() noexcept override {
        for (auto& d : line_) d.reset();
        for (auto& d : dry_line_) d.reset();
        for (auto& d : rot_line_) d.reset();
        for (auto* f : {&dark_a_[0], &dark_a_[1], &dark_b_[0], &dark_b_[1], &xover_[0], &xover_[1], &split_[0], &split_[1], &hiss_lp_[0],
                        &hiss_lp_[1], &loop_lp_[0], &loop_lp_[1], &ota_lp_[0], &ota_lp_[1], &back_lp_[0], &back_lp_[1]})
            f->reset();
        for (auto& c : ap_)
            for (auto& ch : c)
                for (float& s : ch) s = 0.0f;
        fb_[0] = fb_[1] = 0.0f;
        lamp_[0] = lamp_[1] = 0.5f;
        horn_speed_ = drum_speed_ = 0.0f;
        horn_angle_ = drum_angle_ = 0.0;
        clock_.reset();
        phase2_ = 0.0;
        barber_ = 0.0;
        seeded_ = -1.0f;
        primed_ = false;
        countdown_ = 0;
    }

    void process(float* l, float* r, uint32_t n, const float* p, const BuiltinContext& ctx, float* ro) noexcept override {
        const int circuit = std::clamp(static_cast<int>(std::lround(p[kModel])), 0, 10);
        const int mode = circuit == kEnsemble ? 0 : std::max(0, static_cast<int>(std::lround(p[mode_param(circuit)])));
        const float seed = std::clamp(std::round(p[kSeed]), 1.0f, 9999.0f);
        if (seed != seeded_) {
            const auto s = static_cast<uint32_t>(seed);
            drift_a_.seed(s * 3u + 1u);
            drift_b_.seed(s * 7u + 5u);
            wow_.seed(s * 11u + 2u);
            noise_[0].seed(s * 13u + 3u);
            noise_[1].seed(s * 17u + 9u);
            seeded_ = seed;
        }
        const float voice = std::clamp(p[kVoice], -100.0f, 100.0f) * 0.01f;
        const float amount = std::clamp(p[kDepth], 0.0f, 100.0f) * 0.02f * (1.0f + 0.5f * std::max(voice, 0.0f));
        const float width = std::clamp(p[kWidth], 0.0f, 100.0f) * 0.01f;
        const float drift = std::clamp(p[kDrift], 0.0f, 100.0f) * 0.01f;
        const float hiss = kHasHiss[circuit] ? std::clamp(p[kHiss], 0.0f, 100.0f) * 0.01f : 0.0f;
        const float drive = kHasDrive[circuit] ? std::clamp(p[kDrive], 0.0f, 100.0f) * 0.01f : 0.0f;
        const float darken = kHasDarken[circuit] ? std::clamp(p[kDarken], 0.0f, 100.0f) * 0.01f * (1.0f - 0.5f * std::max(-voice, 0.0f)) : 0.0f;
        const float wow = circuit == kTape ? std::clamp(p[kWow], 0.0f, 100.0f) * 0.01f : 0.0f;
        constexpr float kInputGain[] = {1.0f, 2.0f, 4.0f};
        const float in_gain = kInputGain[std::clamp(static_cast<int>(std::lround(p[kInput])), 0, 2)];
        float dry = 1.0f, wet = 0.0f;
        dsp::mix_gains(std::clamp(p[kMix], 0.0f, 100.0f) * 0.01f, 0.0f, dry, wet);
        const float trim = dsp::db_to_gain(std::clamp(p[kTrim], -48.0f, 48.0f));
        const float dm = std::clamp(p[kDepthM], 0.0f, 100.0f) * 0.01f;
        const float ds = std::clamp(p[kDepthS], 0.0f, 100.0f) * 0.01f * width;
        const bool mono = p[kMonoAudition] >= 0.5f;
        const float a2 = std::fabs(voice) * 0.35f;
        const float rate2 = voice < 0.0f ? 3.7f : 0.31f;
        clock_.begin(std::clamp(p[kRate], 0.001f, 50.0f), p[kSync] >= 0.5f, dsp::table_at(dsp::kModNoteQuarters, p[kNote]), ctx.transport,
                     ctx.offset, sr_);
        // Darken: 16 kHz (0%) down to about 1.9 kHz (100%), 12 dB/oct.
        const float dark_hz = 16000.0f * std::pow(0.12f, darken);
        if (!primed_ || circuit != circuit_) {
            amount_.snap(amount);
            width_.snap(width);
            mix_dry_.snap(dry);
            mix_wet_.snap(wet);
            trim_.snap(trim);
            depth_m_.snap(dm);
            depth_s_.snap(ds);
            drive_.snap(drive);
            hiss_.snap(hiss);
            darken_.snap(dark_hz);
            a2_.snap(a2);
            if (circuit == kRotary && circuit_ != kRotary) horn_speed_ = drum_speed_ = rotor_target(mode, p[kRate]);
            circuit_ = circuit;
            primed_ = true;
            countdown_ = 0;
        }
        const float inc = static_cast<float>(clock_.inc);
        const float drift_inc_a = 0.11f / srf_, drift_inc_b = 0.07f / srf_, wow_inc = 0.6f / srf_, flutter_inc = 7.0f / srf_;
        const float horn_target = circuit == kRotary ? rotor_target(mode, p[kRate]) : 0.0f;
        float* io[2] = {l, r};
        for (uint32_t i = 0; i < n; ++i) {
            const float ph = clock_.tick();
            phase2_ += static_cast<double>(inc * rate2);
            if (phase2_ >= 1.0) phase2_ -= std::floor(phase2_);
            const float p2 = static_cast<float>(phase2_);
            const float dr_ph = drift * 0.12f * drift_a_.tick(drift_inc_a);
            const float dr_amt = 1.0f + 0.25f * drift * drift_b_.tick(drift_inc_b);
            const float amt = amount_.next(amount) * dr_amt;
            const float w = width_.next(width);
            const float g2 = a2_.next(a2);
            const float gd = mix_dry_.next(dry), gw = mix_wet_.next(wet), gt = trim_.next(trim);
            const float gm = depth_m_.next(dm), gs = depth_s_.next(ds);
            const float drv = drive_.next(drive), hs = hiss_.next(hiss);
            if (countdown_ == 0) {
                const float hz = darken_.next(dark_hz);
                for (int c = 0; c < 2; ++c) {
                    dark_a_[c].set(hz, sr_);
                    dark_b_[c].set(hz, sr_);
                }
                countdown_ = kControl;
            }
            --countdown_;
            const Mod mod{ph + dr_ph, p2, g2, voice, circuit <= kTwin || circuit == kFlanger};
            const float x[2] = {io[0][i], io[1][i]};
            // Input stage: level into the circuit and its saturation.
            float xin[2];
            const float k = 1.0f + 3.0f * drv;
            for (int c = 0; c < 2; ++c) {
                const float pre = x[c] * in_gain;
                xin[c] = drv > 0.0f || in_gain > 1.0f ? std::tanh(k * pre) / (k * in_gain) : x[c];
            }
            float y[2] = {0.0f, 0.0f};
            float xd[2] = {x[0], x[1]};
            float wow_ms = 0.0f;
            if (wow > 0.0f) {
                flutter_ += static_cast<double>(flutter_inc);
                if (flutter_ >= 1.0) flutter_ -= 1.0;
                wow_ms = wow * (1.2f * wow_.tick(wow_inc) + 0.15f * dsp::sin2pi(static_cast<float>(flutter_)));
            }
            run_circuit(circuit, mode, mod, amt, w, xin, y, xd, horn_target, wow_ms);
            // Artifacts: hiss, darken.
            for (int c = 0; c < 2; ++c) {
                if (hs > 0.0f) y[c] += hiss_lp_[c].lp(noise_[c].next()) * dsp::db_to_gain(-96.0f + 36.0f * hs);
                if (kHasDarken[circuit]) y[c] = dark_b_[c].lp(dark_a_[c].lp(y[c]));
            }
            // Wet mid / side, mix, trim.
            const float mid = 0.5f * (y[0] + y[1]) * gm, side = 0.5f * (y[0] - y[1]) * gs;
            float ol = (gd * xd[0] + gw * (mid + side)) * gt;
            float orr = (gd * xd[1] + gw * (mid - side)) * gt;
            if (mono) ol = orr = 0.5f * (ol + orr);
            io[0][i] = ol;
            io[1][i] = orr;
        }
        ro[kPhase] = static_cast<float>(clock_.phase);
    }

private:
    struct Mod {
        float ph, p2, a2, voice;
        bool triangle;
        // Unit modulation -1..1 at a phase offset (cycles).
        float at(float offset) const noexcept {
            const float main = triangle ? dsp::tri(ph + offset) : dsp::sin2pi(ph + offset);
            return (1.0f - a2) * main + a2 * dsp::sin2pi(p2 + offset);
        }
    };

    static uint32_t mode_param(int circuit) noexcept {
        constexpr uint32_t kModes[] = {kModeSingle, kModeCrossed, kModeTwin, kModeSingle, kModeTape, kModePhotocell,
                                       kModeFlanger, kModePhaser, kModeTrem, kModeRotary, kModePanner};
        return kModes[circuit];
    }

    static float rotor_target(int mode, float rate) noexcept {
        const float fast = std::clamp(rate, 0.02f, 20.0f);
        return mode == 0 ? 0.12f * fast : mode == 1 ? fast : 0.0f;
    }

    float tap(int c, float ms) noexcept { return line_[c].read(std::max(1.0f, ms * ms_ - 1.0f)); }

    void run_circuit(int circuit, int mode, const Mod& m, float amt, float w, const float* x, float* y, float* xd, float horn_target,
                     float wow_ms) noexcept {
        switch (circuit) {
        case kSingle: {
            const bool vib = mode == 1;
            for (int c = 0; c < 2; ++c) {
                const float u = m.at(c == 0 ? 0.0f : 0.5f * w);
                y[c] = vib ? tap(c, 4.0f + 2.2f * amt * u) : tap(c, 6.5f + 3.0f * amt * u);
                line_[c].push(x[c]);
            }
            break;
        }
        case kCrossed: {
            const CrossedMode& cm = kCrossedModes[std::min(mode, 3)];
            float t[2];
            for (int c = 0; c < 2; ++c) {
                float u = m.at(c == 0 ? 0.0f : 0.5f * w);
                if (cm.lfo2 > 0.0f) u = 0.7f * u + 0.3f * dsp::sin2pi(m.ph * cm.lfo2 + (c == 0 ? 0.25f : 0.75f));
                const float d = cm.centre + cm.swing * amt * u;
                t[c] = tap(c, d);
                if (cm.tap2 > 0.0f) t[c] = 0.6f * t[c] + 0.4f * tap(c, cm.tap2 * cm.centre - cm.swing * 0.6f * amt * u);
                line_[c].push(x[c]);
            }
            y[0] = (1.0f - cm.cross) * t[0] + cm.cross * t[1];
            y[1] = (1.0f - cm.cross) * t[1] + cm.cross * t[0];
            break;
        }
        case kTwin: {
            const float swing = mode == 2 ? 0.35f : 1.85f;
            for (int c = 0; c < 2; ++c) {
                y[c] = tap(c, 3.55f + swing * amt * m.at(c == 0 ? 0.0f : 0.5f * w));
                line_[c].push(x[c]);
            }
            break;
        }
        case kEnsemble: {
            const float fast = 10.3f;
            float t[2][3];
            for (int c = 0; c < 2; ++c) {
                for (int v = 0; v < 3; ++v) {
                    const float off = static_cast<float>(v) / 3.0f + (c == 0 ? 0.0f : w / 6.0f);
                    const float u = 0.8f * m.at(off) + 0.2f * dsp::sin2pi(m.ph * fast + off);
                    t[c][v] = tap(c, 6.0f + 1.6f * amt * u);
                }
                line_[c].push(x[c]);
            }
            // Voices weighted across the stereo field; Width blends them to the centre.
            const float a = 1.0f, b = 0.7f, cc = 0.4f + 0.6f * (1.0f - w);
            const float norm = 1.0f / (a + b + cc);
            y[0] = (a * t[0][0] + b * t[0][1] + cc * t[0][2]) * norm;
            y[1] = (cc * t[1][0] + b * t[1][1] + a * t[1][2]) * norm;
            break;
        }
        case kTape: {
            for (int c = 0; c < 2; ++c) {
                const float u = m.at(c == 0 ? 0.0f : 0.5f * w);
                const float wobble = wow_ms * (c == 0 ? 1.2f : 1.0f);
                dry_line_[c].push(x[c]);
                if (mode == 0) {
                    y[c] = tap(c, 18.0f + (c == 0 ? 0.0f : 2.0f * w) + 1.5f * amt * u + wobble);
                } else {
                    const float centre = 1.0f + 2.0f * std::min(amt, 2.0f);
                    const float swing = mode == 1 ? 2.0f : 0.25f;
                    xd[c] = dry_line_[c].read(centre * ms_);
                    y[c] = tap(c, centre + swing * amt * u + wobble);
                }
                line_[c].push(x[c]);
            }
            break;
        }
        case kPhotocell: {
            constexpr float kStagger[4] = {1.0f, 0.42f, 2.6f, 6.1f};
            for (int c = 0; c < 2; ++c) {
                const float target = 0.5f + 0.5f * m.at(c == 0 ? 0.0f : 0.25f * w);
                lamp_[c] = target + (target > lamp_[c] ? lamp_up_ : lamp_down_) * (lamp_[c] - target);
                const float f = 90.0f * std::exp(std::log(25.0f) * std::clamp(0.5f + (lamp_[c] - 0.5f) * amt, 0.0f, 1.0f));
                float v = x[c];
                for (int s = 0; s < 4; ++s) v = allpass(v, ap_coef(f * kStagger[s], srf_), ap_[0][c][s]);
                y[c] = mode == 1 ? v : 0.5f * (x[c] + v);
            }
            break;
        }
        case kFlanger: {
            const float base = mode == 1 ? 0.35f : mode == 2 ? 0.5f : 0.6f;
            const float span = mode == 1 ? 3.5f : mode == 2 ? 3.0f : 2.2f;
            const float fb = mode == 1 ? 0.78f : mode == 2 ? 0.6f : 0.45f;
            for (int c = 0; c < 2; ++c) {
                const float u = 0.5f + 0.5f * m.at(c == 0 ? 0.0f : 0.25f * w);
                float t = tap(c, base + span * std::min(amt, 2.0f) * u);
                if (mode == 2) t = loop_lp_[c].lp(t);
                y[c] = t;
                line_[c].push(x[c] + fb * t);
            }
            break;
        }
        case kPhaser: run_phaser(mode, m, amt, w, x, y); break;
        case kTremolo: {
            // Voice > 0 squares the wave towards a chop.
            const float kv = 1.0f + 6.0f * std::max(m.voice, 0.0f);
            const float kv_norm = 1.0f / std::tanh(kv);
            for (int c = 0; c < 2; ++c) {
                float u = m.at(c == 0 ? 0.0f : 0.5f * w);
                if (m.voice > 0.0f) u = std::tanh(kv * u) * kv_norm;
                const float d = std::clamp(0.6f * amt, 0.0f, 1.0f);
                if (mode == 2) {
                    const float lo = split_[c].lp(x[c]), hi = x[c] - lo;
                    const float gl = 1.0f - d * (0.5f - 0.5f * u), gh = 1.0f - d * (0.5f + 0.5f * u);
                    y[c] = lo * gl + hi * gh;
                } else {
                    float shape = u;
                    if (mode == 1) shape = std::tanh(2.5f * u) / std::tanh(2.5f);
                    float target = 0.5f + 0.5f * shape;
                    if (mode == 0) {
                        lamp_[c] = target + (target > lamp_[c] ? trem_up_ : trem_down_) * (lamp_[c] - target);
                        target = lamp_[c];
                    }
                    y[c] = x[c] * (1.0f - d * (1.0f - target));
                }
            }
            break;
        }
        case kRotary: run_rotary(amt, w, x, y, horn_target); break;
        case kPanner: {
            const float u = m.at(0.0f);
            const float v = std::clamp(0.5f * amt, 0.0f, 1.0f) * w * u;
            const float pos = 0.125f + 0.125f * v;
            constexpr float kSqrt2 = 1.41421356237f;
            float gl = kSqrt2 * dsp::cos2pi(pos), gr = kSqrt2 * dsp::sin2pi(pos);
            if (mode == 1) {
                // Behind the listener (cos < 0): darker, quieter, slightly later.
                const float back = 0.5f - 0.5f * m.at(0.25f) * std::clamp(0.5f * amt, 0.0f, 1.0f);
                for (int c = 0; c < 2; ++c) {
                    rot_line_[c].push(x[c]);
                    const float d = (0.3f + 0.25f * back) * ms_;
                    const float delayed = rot_line_[c].read(d);
                    const float dark = back_lp_[c].lp(delayed);
                    y[c] = (delayed + back * (dark - delayed)) * (1.0f - 0.37f * back);
                }
                y[0] *= gl;
                y[1] *= gr;
            } else {
                y[0] = x[0] * gl;
                y[1] = x[1] * gr;
            }
            break;
        }
        default: y[0] = x[0]; y[1] = x[1]; break;
        }
    }

    void run_phaser(int mode, const Mod& m, float amt, float w, const float* x, float* y) noexcept {
        const float span = std::clamp(amt, 0.0f, 2.0f);
        const float ln_c = 0.5f * std::log(180.0f * 4200.0f), h = 0.5f * std::log(4200.0f / 180.0f) * span;
        if (mode == 4) {
            // Barber pole: three chains, each sweeping upwards across the
            // range and fading in / out (raised cosine) as it wraps.
            barber_ += static_cast<double>(clock_.inc);
            if (barber_ >= 1.0) barber_ -= std::floor(barber_);
            const float lo = std::log(80.0f), hi = std::log(6000.0f);
            for (int c = 0; c < 2; ++c) {
                float acc = 0.0f;
                for (int k = 0; k < kBarberChains; ++k) {
                    const float t = dsp::wrap01(static_cast<float>(barber_) + static_cast<float>(k) / kBarberChains + (c == 0 ? 0.0f : 0.15f * w));
                    const float a = ap_coef(std::exp(lo + (hi - lo) * t), srf_);
                    const float env = 0.5f - 0.5f * dsp::cos2pi(t);
                    float v = x[c];
                    for (int s = 0; s < kStages; ++s) v = allpass(v, a, ap_[k][c][s]);
                    acc += env * v;
                }
                y[c] = 0.5f * x[c] + acc * (2.0f / kBarberChains) * 0.5f;
            }
            return;
        }
        const int stages = mode == 3 ? 6 : 4;
        const float fb = mode == 1 ? 0.55f : mode == 2 ? 0.35f : 0.0f;
        for (int c = 0; c < 2; ++c) {
            const float off = mode == 3 ? (c == 0 ? 0.0f : 0.25f) : (c == 0 ? 0.0f : 0.5f * w);
            const float a = ap_coef(std::exp(ln_c + h * m.at(off)), srf_);
            float v = x[c] + fb * fb_[c];
            for (int s = 0; s < stages; ++s) {
                if (mode == 2) v = std::tanh(1.4f * v) * (1.0f / 1.4f);
                v = allpass(v, a, ap_[0][c][s]);
            }
            if (mode == 2) v = ota_lp_[c].lp(v);
            fb_[c] = v;
            y[c] = 0.5f * (x[c] + v);
        }
    }

    void run_rotary(float amt, float w, const float* x, float* y, float horn_target) noexcept {
        // Rotor inertia.
        const float drum_target = horn_target * 0.88f;
        horn_speed_ = horn_target + (horn_target > horn_speed_ ? horn_up_ : horn_down_) * (horn_speed_ - horn_target);
        drum_speed_ = drum_target + (drum_target > drum_speed_ ? drum_up_ : drum_down_) * (drum_speed_ - drum_target);
        horn_angle_ += static_cast<double>(horn_speed_ / srf_);
        drum_angle_ += static_cast<double>(drum_speed_ / srf_);
        if (horn_angle_ >= 1.0) horn_angle_ -= 1.0;
        if (drum_angle_ >= 1.0) drum_angle_ -= 1.0;
        const float in = 0.5f * (x[0] + x[1]);
        const float low = xover_[1].lp(xover_[0].lp(in)), high = in - low;
        rot_line_[0].push(high);
        rot_line_[1].push(low);
        const float a = std::clamp(amt, 0.0f, 2.0f);
        const float ha = static_cast<float>(horn_angle_), da = static_cast<float>(drum_angle_);
        for (int c = 0; c < 2; ++c) {
            const float mic = c == 0 ? -0.25f * w : 0.25f * w; // mics at +-90 deg at full width
            const float hs = dsp::sin2pi(ha + mic), hc = dsp::cos2pi(ha + mic);
            const float ds = dsp::sin2pi(da + mic), dc = dsp::cos2pi(da + mic);
            const float horn = rot_line_[0].read((1.2f + 0.45f * a * hs) * ms_) * (1.0f - 0.25f * a * (0.5f - 0.5f * hc));
            const float drum = rot_line_[1].read((1.2f + 0.15f * a * ds) * ms_) * (1.0f - 0.12f * a * (0.5f - 0.5f * dc));
            y[c] = horn + drum;
        }
    }

    double sr_ = 48000.0;
    float srf_ = 48000.0f, ms_ = 48.0f;
    dsp::LfoClock clock_;
    double phase2_ = 0.0, barber_ = 0.0, flutter_ = 0.0;
    dsp::DelayLine line_[2], dry_line_[2], rot_line_[2];
    dsp::Smoother amount_, width_, mix_dry_, mix_wet_, trim_, depth_m_, depth_s_, drive_, hiss_, a2_, darken_;
    dsp::OnePole dark_a_[2], dark_b_[2], xover_[2], split_[2], hiss_lp_[2], loop_lp_[2], ota_lp_[2], back_lp_[2];
    dsp::Wander drift_a_, drift_b_, wow_;
    dsp::Rng noise_[2];
    float ap_[kBarberChains][2][kStages]{};
    float fb_[2]{};
    float lamp_[2]{0.5f, 0.5f};
    float lamp_up_ = 0.0f, lamp_down_ = 0.0f, trem_up_ = 0.0f, trem_down_ = 0.0f;
    float horn_up_ = 0.0f, horn_down_ = 0.0f, drum_up_ = 0.0f, drum_down_ = 0.0f;
    float horn_speed_ = 0.0f, drum_speed_ = 0.0f;
    double horn_angle_ = 0.0, drum_angle_ = 0.0;
    float seeded_ = -1.0f;
    int circuit_ = -1;
    uint32_t countdown_ = 0;
    bool primed_ = false;
};

std::unique_ptr<BuiltinProcessor> create() { return std::make_unique<CharacterMod>(); }

} // namespace

extern const BuiltinDescriptor kCharacterModDescriptor;
const BuiltinDescriptor kCharacterModDescriptor{"character-mod", kParamIds, kReadoutIds, &create, {}, kDefaults};

} // namespace dstudio
