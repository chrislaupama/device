// Synth (synth-v2): the built-in polyphonic subtractive instrument.
// Parameters in the order of dstudio_core::builtin::synth_v2 (checked by a
// Rust test).
//
// Per voice: two oscillators (Sine, Triangle, Saw, Square, Sync, FM; each
// with a Shape control) stacked up to 8 times for unison, a sub oscillator,
// a stereo noise source, a multimode filter (zero-delay-feedback ladder for
// Low-pass 24, state-variable filter otherwise) with drive, a 12 dB
// high-pass, an amp envelope and a second envelope, two LFOs (free running
// or retriggered, tempo synced) and the 32-slot modulation matrix. The
// voices are summed and run through the four effect slots and the output
// volume.
//
// Control rate: modulation, pitch and filter coefficients are updated every
// kTick samples (and at every MIDI event); envelopes and audio run per
// sample. Oscillators are band-limited with polyBLEP / polyBLAMP residuals,
// saturating stages use ADAA (synth_v2_dsp.hpp). Nothing in process()
// allocates or locks.
//
// Modulation: a route adds amount x source x target depth (TARGET_DEPTH in
// the Rust definition: semitones for pitch, octaves for cutoff / high-pass /
// LFO rates, the target's normalized range otherwise). Voice targets are
// modulated per voice; effect targets use the most recently played voice's
// sources. A macro mapping holds its target at from + (to - from) x macro
// (normalized), before modulation.
#include "builtin.hpp"
#include "dsp.hpp"
#include "synth_v2_dsp.hpp"
#include "synth_v2_fx.hpp"
#include "synth_v2_params.hpp"

#include <algorithm>
#include <array>
#include <cmath>
#include <cstring>

namespace dstudio {

namespace {

using namespace synth_v2;

constexpr int kVoices = 16;
constexpr int kUnison = 8;
constexpr int kRoutes = 32;
constexpr int kMacros = 8;
constexpr int kMaps = 16;
constexpr int kFxSlots = 4;
constexpr int kHeld = 16;
constexpr uint32_t kTick = 16;
constexpr float kOutScale = 0.5f;

enum Source : int { kSrcNone, kSrcEnv2, kSrcAmpEnv, kSrcLfo1, kSrcLfo2, kSrcVelocity, kSrcKey, kSrcWheel, kSrcAftertouch, kSrcRandom, kSourceCount };
enum Target : int {
    kTgtNone, kTgtPitch, kTgtOsc1Pitch, kTgtOsc2Pitch, kTgtOsc1Shape, kTgtOsc2Shape, kTgtOsc1Level, kTgtOsc2Level,
    kTgtCrossFm, kTgtSubLevel, kTgtNoiseLevel, kTgtCutoff, kTgtResonance, kTgtDrive, kTgtHighPass, kTgtAmp, kTgtPan,
    kTgtDetune, kTgtLfo1Rate, kTgtLfo2Rate, kTgtLfo1Depth, kTgtLfo2Depth, kTgtFxMix1, kTgtFxAmount1 = kTgtFxMix1 + 4,
    kTargetCount = kTgtFxAmount1 + 4
};
// The same table as dstudio_core::builtin::synth_v2::TARGET_DEPTH.
constexpr float kTargetDepth[kTargetCount] = {0.0f, 24.0f, 24.0f, 24.0f, 1.0f, 1.0f, 1.0f, 1.0f, 1.0f, 1.0f,
                                              1.0f, 8.0f,  1.0f,  1.0f,  6.0f, 1.0f, 1.0f, 1.0f, 4.0f, 4.0f,
                                              1.0f, 1.0f,  1.0f,  1.0f,  1.0f, 1.0f, 1.0f, 1.0f, 1.0f, 1.0f};
// The parameter an effect slot's "Amount" target moves, per effect type.
constexpr uint32_t kFxAmount[kFxTypeCount] = {kTapeDrive,   kDistDrive,  kCrushDownsample, kChorusDepth,
                                              kFlangerDepth, kPhaserDepth, kFiltCutoff,     kDelayFeedback,
                                              kRevSize,     kCompThreshold, kWideWidth};

enum Readout : uint32_t {
    kRoVoices, kRoAmpEnv, kRoEnv2, kRoLfo1, kRoLfo2, kRoLfo1Phase, kRoLfo2Phase, kRoFx1Gr,
    kRoOutDb = kRoFx1Gr + 3 * kFxSlots,
    // Summed modulation per target of the newest voice (global sources without one).
    kRoMod,
    // Each source's value (kSrcEnv2 onwards), for the per-source rings around the dials.
    kRoSrc = kRoMod + kTargetCount,
    kReadoutCount = kRoSrc + kSourceCount - 1
};
constexpr const char* kReadoutIds[kReadoutCount] = {
    "voices",     "amp_env",    "env2",       "lfo1",      "lfo2",       "lfo1_phase", "lfo2_phase",
    "fx1_gr",     "fx1_phase",  "fx1_cutoff", "fx2_gr",    "fx2_phase",  "fx2_cutoff", "fx3_gr",
    "fx3_phase",  "fx3_cutoff", "fx4_gr",     "fx4_phase", "fx4_cutoff", "out_db",
    "mod_0", "mod_1", "mod_2", "mod_3", "mod_4", "mod_5", "mod_6", "mod_7",
    "mod_8", "mod_9", "mod_10", "mod_11", "mod_12", "mod_13", "mod_14", "mod_15",
    "mod_16", "mod_17", "mod_18", "mod_19", "mod_20", "mod_21", "mod_22", "mod_23",
    "mod_24", "mod_25", "mod_26", "mod_27", "mod_28", "mod_29",
    "src_env2", "src_amp_env", "src_lfo1", "src_lfo2", "src_velocity", "src_key", "src_wheel", "src_aftertouch", "src_random"};
constexpr float kIdleReadouts[kReadoutCount] = {0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, -120.0f};

// Normalized <-> plain, the codec of dstudio_core::builtin (log tapers are
// geometric, linear ones use n^(1/skew); discrete values round).
float to_norm(uint32_t i, float v) noexcept {
    const Range& g = kRanges[i];
    if (g.max <= g.min) return 0.0f;
    float n;
    if (g.log != 0) {
        n = std::log(std::max(v, g.min) / g.min) / std::log(g.max / g.min);
    } else {
        n = (v - g.min) / (g.max - g.min);
        if (g.skew != 1.0f && n > 0.0f) n = std::pow(n, g.skew);
    }
    return std::clamp(n, 0.0f, 1.0f);
}

float from_norm(uint32_t i, float n) noexcept {
    const Range& g = kRanges[i];
    n = std::clamp(n, 0.0f, 1.0f);
    float v;
    if (g.log != 0) {
        v = g.min * std::pow(g.max / g.min, n);
    } else {
        if (g.skew != 1.0f && n > 0.0f) n = std::pow(n, 1.0f / g.skew);
        v = g.min + (g.max - g.min) * n;
    }
    return g.discrete != 0 ? std::round(v) : v;
}

inline int ichoice(float v, int hi) noexcept { return std::clamp(static_cast<int>(std::lround(v)), 0, hi); }
inline float st_to_hz(float note) noexcept { return 440.0f * std::exp2((note - 69.0f) * (1.0f / 12.0f)); }
inline float wrap1(float p) noexcept { return p >= 1.0f ? p - 1.0f : p; }

// ---------------------------------------------------------------- envelope
// Attack approaches 1.3 and stops at 1 (a convex rise), decay and release
// are exponential (the time is the time to fall by 60 dB).
struct EnvCoef {
    float att = 0.0f, dec = 0.0f, sus = 0.0f, rel = 0.0f;
    void set(float a_s, float d_s, float s, float r_s, float sr) noexcept {
        att = 1.0f - std::exp(-std::log(1.3f / 0.3f) / (std::max(a_s, 1e-4f) * sr));
        dec = std::exp(-6.9077553f / (std::max(d_s, 1e-3f) * sr));
        sus = std::clamp(s, 0.0f, 1.0f);
        rel = std::exp(-6.9077553f / (std::max(r_s, 1e-3f) * sr));
    }
};

struct Env {
    enum Stage : uint8_t { kIdle, kAttack, kDecay, kRelease };
    Stage stage = kIdle;
    float y = 0.0f;
    void gate_on() noexcept { stage = kAttack; }
    void gate_off() noexcept {
        if (stage != kIdle) stage = kRelease;
    }
    void kill() noexcept {
        stage = kIdle;
        y = 0.0f;
    }
    float step(const EnvCoef& c) noexcept {
        switch (stage) {
        case kAttack:
            y += c.att * (1.3f - y);
            if (y >= 1.0f) {
                y = 1.0f;
                stage = kDecay;
            }
            break;
        case kDecay: y = c.sus + (y - c.sus) * c.dec; break;
        case kRelease:
            y *= c.rel;
            if (y < 1e-4f) kill();
            break;
        case kIdle: break;
        }
        return y;
    }
};

// ---------------------------------------------------------------- oscillators
struct OscState {
    float p = 0.0f;           // main phase
    float a = 0.0f, b = 0.0f; // saw: detuned copies; sync: slave phase in a
    float y1 = 0.0f, y2 = 0.0f;
};

inline float saw_bl(float t, float dt) noexcept { return 2.0f * t - 1.0f - blep(t, dt); }

// Renders n samples of one oscillator. pm (turns, may be null) is the
// cross-FM input scaled by pm_amt.
void osc_block(int wave, float shape, float dt, OscState& s, const float* pm, float pm_amt, float* out,
               uint32_t n) noexcept {
    switch (wave) {
    case 0: { // sine, Shape bends it with a sine of itself
        const float bend = 0.18f * shape;
        for (uint32_t i = 0; i < n; ++i) {
            const float ph = s.p + (pm != nullptr ? pm_amt * pm[i] : 0.0f);
            out[i] = sin_turns(ph + bend * sin_turns(ph));
            s.p = wrap1(s.p + dt);
        }
        break;
    }
    case 1: { // triangle, Shape moves the peak toward a saw
        const float k = 0.5f + 0.47f * shape;
        const float up = 2.0f / k, down = 2.0f / (1.0f - k);
        const float corner = (up + down) * dt;
        for (uint32_t i = 0; i < n; ++i) {
            const float t = pm != nullptr ? wrap01(s.p + pm_amt * pm[i]) : s.p;
            float y = t < k ? -1.0f + up * t : 1.0f - down * (t - k);
            y += corner * (blamp(t, dt) - blamp(wrap01(t - k + 1.0f), dt));
            out[i] = y;
            s.p = wrap1(s.p + dt);
        }
        break;
    }
    case 2: { // saw, Shape blends in two detuned copies
        const float mix = 0.7f * shape;
        const float norm = 1.0f / (1.0f + 1.1f * mix);
        const float da = dt * 1.0061f, db = dt * 0.9937f;
        for (uint32_t i = 0; i < n; ++i) {
            const float o = pm != nullptr ? pm_amt * pm[i] : 0.0f;
            float y = saw_bl(pm != nullptr ? wrap01(s.p + o) : s.p, dt);
            if (mix > 0.0f) {
                y += mix * (saw_bl(pm != nullptr ? wrap01(s.a + o) : s.a, da) +
                            saw_bl(pm != nullptr ? wrap01(s.b + o) : s.b, db));
                s.a = wrap1(s.a + da);
                s.b = wrap1(s.b + db);
            }
            out[i] = y * norm;
            s.p = wrap1(s.p + dt);
        }
        break;
    }
    case 3: { // square, Shape narrows the pulse
        const float w = 0.5f - 0.45f * shape;
        const float dc = 2.0f * w - 1.0f;
        for (uint32_t i = 0; i < n; ++i) {
            const float t = pm != nullptr ? wrap01(s.p + pm_amt * pm[i]) : s.p;
            out[i] = (t < w ? 1.0f : -1.0f) - dc + blep(t, dt) - blep(wrap01(t - w + 1.0f), dt);
            s.p = wrap1(s.p + dt);
        }
        break;
    }
    case 4: { // hard sync: a saw slave at 2^(3 Shape) times the master
        const float ratio = std::exp2(3.0f * shape);
        const float ds = std::min(dt * ratio, 0.49f);
        for (uint32_t i = 0; i < n; ++i) {
            s.p += dt;
            s.a += ds;
            float y;
            if (s.p >= 1.0f) {
                s.p -= 1.0f;
                const float before = 2.0f * wrap01(s.a) - 1.0f;
                s.a = wrap01(s.p * ratio);
                y = 2.0f * s.a - 1.0f;
                y -= 0.5f * (before - y) * blep(s.p, dt);
            } else {
                if (s.a >= 1.0f) s.a -= 1.0f;
                y = saw_bl(s.a, ds);
            }
            out[i] = y;
        }
        break;
    }
    default: { // FM: sine with feedback phase modulation by Shape
        const float beta = 0.35f * shape;
        for (uint32_t i = 0; i < n; ++i) {
            const float ph = s.p + (pm != nullptr ? pm_amt * pm[i] : 0.0f) + beta * 0.5f * (s.y1 + s.y2);
            const float y = sin_turns(ph);
            s.y2 = s.y1;
            s.y1 = y;
            out[i] = y;
            s.p = wrap1(s.p + dt);
        }
        break;
    }
    }
}

// Sub oscillator: Sine / Triangle / Saw / Square, Shape adds the 2nd and 3rd
// harmonics.
void sub_block(int wave, float shape, float dt, float& p, float* out, uint32_t n) noexcept {
    const float h = shape;
    const float norm = 1.0f / (1.0f + 0.6f * h);
    for (uint32_t i = 0; i < n; ++i) {
        float y;
        switch (wave) {
        case 1: y = 1.0f - 4.0f * std::fabs(p - 0.5f) + 8.0f * dt * (blamp(p, dt) - blamp(wrap01(p + 0.5f), dt)); break;
        case 2: y = saw_bl(p, dt); break;
        case 3: y = (p < 0.5f ? 1.0f : -1.0f) + blep(p, dt) - blep(wrap01(p + 0.5f), dt); break;
        default: y = sin_turns(p); break;
        }
        if (h > 0.0f) y = (y + h * (0.7f * sin_turns(2.0f * p) + 0.45f * sin_turns(3.0f * p))) * norm;
        out[i] = y;
        p = wrap1(p + dt);
    }
}

// ---------------------------------------------------------------- noise
// White, Pink (three-pole approximation), Brown (leaky integrator), Breath
// (band-passed, fluttering), Metallic (six square waves combined pairwise),
// Digital (sample and hold at 4 kHz), Crackle (sparse decaying clicks).
// Color tilts it (below 0.5 a low-pass closes, above a high-pass opens);
// Width blends the right channel from a copy of the left to an independent
// generator.
struct Noise {
    Rng rng[2];
    float pk[2][3] = {};
    float br[2] = {};
    Svf band[2];
    float flutter = 0.0f;
    float met[6] = {};
    float hold[2] = {}, hold_ph = 0.0f;
    float crack[2] = {};
    OnePole tone[2];
    void reset(uint32_t seed) noexcept {
        rng[0].seed(seed * 2654435761u + 1u);
        rng[1].seed(seed * 2246822519u + 7u);
        std::memset(pk, 0, sizeof pk);
        br[0] = br[1] = 0.0f;
        band[0].reset();
        band[1].reset();
        flutter = 0.0f;
        std::memset(met, 0, sizeof met);
        hold[0] = hold[1] = hold_ph = 0.0f;
        crack[0] = crack[1] = 0.0f;
        tone[0].reset();
        tone[1].reset();
    }
    float gen(int type, int c, float inv_sr) noexcept {
        const float w = rng[c].bipolar();
        switch (type) {
        case 1: {
            float* b = pk[c];
            b[0] = 0.99765f * b[0] + w * 0.0990460f;
            b[1] = 0.96300f * b[1] + w * 0.2965164f;
            b[2] = 0.57000f * b[2] + w * 1.0526913f;
            return (b[0] + b[1] + b[2] + w * 0.1848f) * 0.3f;
        }
        case 2:
            br[c] = 0.997f * br[c] + 0.06f * w;
            return br[c] * 1.6f;
        case 3: {
            float bp, hp;
            band[c].run(w, bp, hp);
            return bp * 2.2f * (0.65f + 0.35f * sin_turns(flutter));
        }
        case 4: {
            if (c == 1) return hold[1];
            constexpr float kHz[6] = {410.6f, 608.8f, 739.2f, 1045.4f, 1080.0f, 1600.0f};
            float sq[6];
            for (int k = 0; k < 6; ++k) {
                met[k] = wrap1(met[k] + kHz[k] * inv_sr);
                sq[k] = met[k] < 0.5f ? 1.0f : -1.0f;
            }
            hold[1] = 0.3f * (sq[0] * sq[1] + sq[2] * sq[3] + sq[4] * sq[5]);
            return hold[1];
        }
        case 5:
            if (c == 0) {
                hold_ph += 4000.0f * inv_sr;
                if (hold_ph >= 1.0f) {
                    hold_ph -= 1.0f;
                    hold[0] = w;
                    hold[1] = rng[1].bipolar();
                }
            }
            return hold[c];
        case 6:
            if (rng[c].unit() < 40.0f * inv_sr) crack[c] = 1.6f * w;
            {
                const float y = crack[c];
                crack[c] *= 0.55f;
                return y;
            }
        default: return w * 0.6f;
        }
    }
    // Adds n samples times gain into l / r.
    void block(int type, float color, float width, float gain, float* l, float* r, uint32_t n, float sr) noexcept {
        const float inv_sr = 1.0f / sr;
        const bool low = color <= 0.5f;
        const float hz = low ? 150.0f * std::exp2(color * 14.0f) : 30.0f * std::exp2((color - 0.5f) * 14.0f);
        const float lift = low ? 1.0f : 1.0f + 2.0f * (color - 0.5f);
        for (OnePole& t : tone) t.set_hz(hz, sr);
        if (type == 3)
            for (Svf& s : band) s.set(1800.0f, 1.4f, sr);
        for (uint32_t i = 0; i < n; ++i) {
            flutter = wrap1(flutter + 3.3f * inv_sr);
            const float a = gen(type, 0, inv_sr);
            const float b = gen(type, 1, inv_sr);
            float y[2] = {a, a + width * (b - a)};
            for (int c = 0; c < 2; ++c) y[c] = (low ? tone[c].lp(y[c]) : tone[c].hp(y[c]) * lift) * gain;
            l[i] += y[0];
            r[i] += y[1];
        }
        for (OnePole& t : tone) t.flush();
        if (type == 3)
            for (Svf& s : band) s.flush();
        for (int c = 0; c < 2; ++c) {
            if (std::fabs(br[c]) < 1e-20f) br[c] = 0.0f;
            for (float& v : pk[c])
                if (std::fabs(v) < 1e-20f) v = 0.0f;
        }
    }
};

// ---------------------------------------------------------------- voice
struct Voice {
    bool active = false;
    bool gate = false;       // key down
    bool pedal_held = false; // key up while the sustain pedal is down
    bool primed = false;     // gain known (the first tick snaps instead of ramping)
    int note = 60;
    float velocity = 1.0f;
    uint32_t order = 0;
    float pitch = 60.0f, target = 60.0f;
    float random = 0.0f;
    float since_on = 0.0f;
    Env amp, env2;
    float lfo_ph[2] = {}, lfo_a[2] = {}, lfo_b[2] = {}, lfo_val[2] = {};
    float lfo_rate_mod[2] = {}, lfo_depth_mod[2] = {}; // last tick's modulation of the LFOs
    OscState osc[2][kUnison];
    float drift[kUnison] = {}, drift_to[kUnison] = {};
    float sub_p = 0.0f;
    Noise noise;
    Ladder ladder[2];
    Svf svf[2];
    AdaaTanh drive[2];
    OnePole hp1[2], hp2[2];
    float gain = 0.0f;
    Rng rng;

    void reset_audio(uint32_t seed, bool random_phase) noexcept {
        rng.seed(seed);
        for (auto& o : osc)
            for (OscState& s : o) {
                s = OscState{};
                if (random_phase) {
                    s.p = rng.unit();
                    s.a = rng.unit();
                    s.b = rng.unit();
                } else {
                    s.a = 0.37f;
                    s.b = 0.71f;
                }
            }
        for (int u = 0; u < kUnison; ++u) drift[u] = drift_to[u] = 0.0f;
        sub_p = 0.0f;
        noise.reset(seed);
        for (int c = 0; c < 2; ++c) {
            ladder[c].reset();
            svf[c].reset();
            drive[c].reset();
            hp1[c].reset();
            hp2[c].reset();
        }
        primed = false;
    }
};

struct Route {
    int source, target;
    float amount;
};

// ---------------------------------------------------------------- synth
class SynthV2 final : public BuiltinProcessor {
public:
    void prepare(double sample_rate, uint32_t max_block) override {
        sr_ = static_cast<float>(sample_rate);
        inv_sr_ = 1.0f / sr_;
        for (FxSlot& f : fx_) f.prepare(sample_rate, max_block);
        volume_.set_time(20.0f, sample_rate);
    }

    void reset() noexcept override {
        for (Voice& v : voices_) {
            v.active = false;
            v.amp.kill();
            v.env2.kill();
        }
        for (FxSlot& f : fx_) f.reset();
        held_n_ = 0;
        order_ = 0;
        newest_ = -1;
        last_note_ = -1.0f;
        bend_ = wheel_cc_ = aftertouch_ = 0.0f;
        pedal_ = false;
        glfo_ph_[0] = glfo_ph_[1] = 0.0f;
        volume_primed_ = false;
        asleep_ = true;
        quiet_ = 0;
        seed_ = 1;
    }

    void process(float* l, float* r, uint32_t n, const float* p, const BuiltinContext& ctx, float* ro) noexcept override {
        std::memset(l, 0, n * sizeof(float));
        std::memset(r, 0, n * sizeof(float));
        load_values(p);
        block_setup(ctx, n);
        const int mode = ichoice(v_[voice_mode], 2);
        if (mode != mode_) {
            for (Voice& v : voices_)
                if (v.active && (v.gate || v.pedal_held)) release(v);
            held_n_ = 0;
            mode_ = mode;
        }

        // Control ticks of at most kTick samples, cut at every MIDI event.
        uint32_t pos = 0, e = 0;
        while (pos < n) {
            while (e < ctx.event_count && ctx.events[e].offset <= pos) midi(ctx.events[e++]);
            uint32_t end = std::min(n, pos + kTick);
            if (e < ctx.event_count && ctx.events[e].offset < end) end = std::max(pos + 1, ctx.events[e].offset);
            tick_beats_ = beats_ + static_cast<double>(pos) * beats_per_sample();
            for (Voice& v : voices_)
                if (v.active) render_voice(v, l + pos, r + pos, end - pos);
            pos = end;
        }
        while (e < ctx.event_count) midi(ctx.events[e++]);

        // Idle: once every voice has ended and the output has stayed below
        // -114 dBFS for longer than the longest delay, the output is exact
        // silence and the effects sleep (their state is cleared once) until
        // the next note.
        bool any = false;
        for (const Voice& v : voices_) any = any || v.active;
        if (any || !asleep_) {
            run_fx(l, r, n, ctx);
            asleep_ = false;
            quiet_ = !any && dsp::stereo_peak(l, r, n) < 2e-6f ? quiet_ + n : 0;
            if (static_cast<float>(quiet_) > 4.5f * sr_) {
                for (FxSlot& f : fx_) f.reset();
                std::memset(l, 0, n * sizeof(float));
                std::memset(r, 0, n * sizeof(float));
                asleep_ = true;
            }
        }

        const float vol = v_[volume] <= kRanges[volume].min ? 0.0f : db_to_gain(v_[volume]);
        if (!volume_primed_) {
            volume_.snap(vol);
            volume_primed_ = true;
        }
        float peak = 0.0f;
        for (uint32_t i = 0; i < n; ++i) {
            const float g = volume_.next(vol);
            l[i] *= g;
            r[i] *= g;
            peak = std::max(peak, std::max(std::fabs(l[i]), std::fabs(r[i])));
        }
        write_readouts(ro, peak);
    }

private:
    // ------------------------------------------------------------ values
    void load_values(const float* p) noexcept {
        for (uint32_t i = 0; i < kParamCount; ++i) v_[i] = std::clamp(p[i], kRanges[i].min, kRanges[i].max);
        // Macro mappings hold their targets.
        for (int j = 0; j < kMaps; ++j) {
            const auto base = static_cast<uint32_t>(map1_macro + 4 * j);
            const int mac = ichoice(v_[base], kMacros);
            if (mac == 0) continue;
            const int late = ichoice(v_[static_cast<uint32_t>(map1_late_param + j)], 127);
            int idx = -1;
            if (late > 0) {
                idx = noise_type - 1 + late;
            } else {
                const int pc = ichoice(v_[base + 1], 501);
                if (pc > 0) idx = pc - 1;
            }
            if (idx < 0 || idx >= kParamCount || (idx >= macro1 && idx <= preset)) continue;
            const float m = std::clamp(v_[static_cast<uint32_t>(macro1 + mac - 1)], 0.0f, 1.0f);
            const float from = v_[base + 2], to = v_[base + 3];
            const auto t = static_cast<uint32_t>(idx);
            v_[t] = from_norm(t, from + (to - from) * m);
        }
        n_routes_ = 0;
        for (int k = 0; k < kRoutes; ++k) {
            const auto b = static_cast<uint32_t>(mod1_source + 4 * k);
            const int s = ichoice(v_[b], kSourceCount - 1);
            const int t = ichoice(v_[b + 1], kTargetCount - 1);
            const float a = std::clamp(v_[b + 2], -1.0f, 1.0f);
            if (s == kSrcNone || t == kTgtNone || a == 0.0f || v_[b + 3] < 0.5f) continue;
            routes_[n_routes_++] = Route{s, t, a * kTargetDepth[t]};
        }
    }

    double beats_per_sample() const noexcept { return static_cast<double>(tempo_) / 60.0 * static_cast<double>(inv_sr_); }

    void block_setup(const BuiltinContext& ctx, uint32_t n) noexcept {
        amp_c_.set(v_[amp_attack], v_[amp_decay], v_[amp_sustain], v_[amp_release], sr_);
        env2_c_.set(v_[env2_attack], v_[env2_decay], v_[env2_sustain], v_[env2_release], sr_);
        if (ctx.transport != nullptr) {
            tempo_ = static_cast<float>(std::clamp(ctx.transport->tempo_bpm, 20.0, 999.0));
            beats_ = ctx.transport->beats() + static_cast<double>(ctx.offset) * beats_per_sample();
            playing_ = ctx.transport->playing;
        } else {
            tempo_ = 120.0f;
            beats_ = 0.0;
            playing_ = false;
        }
        // Free-running LFO phases (references for non-retriggered voices).
        for (int k = 0; k < 2; ++k) {
            const auto b = static_cast<uint32_t>(k == 0 ? lfo1_shape : lfo2_shape);
            const bool sync = v_[b + 2] >= 0.5f;
            const float cyc = lfo_note_beats(ichoice(v_[b + 3], 16));
            if (sync && playing_) {
                const double ph = beats_ / static_cast<double>(cyc);
                glfo_ph_[k] = static_cast<float>(ph - std::floor(ph));
            } else {
                const float hz = sync ? tempo_ / 60.0f / cyc : v_[b + 1];
                glfo_ph_[k] = wrap01(glfo_ph_[k] + hz * static_cast<float>(n) * inv_sr_);
            }
        }
    }

    // ------------------------------------------------------------ MIDI
    void midi(const MidiEvent& ev) noexcept {
        const int type = ev.status & 0xf0;
        switch (type) {
        case 0x90:
            if (ev.data2 > 0) {
                note_on(ev.data1 & 0x7f, static_cast<float>(ev.data2) / 127.0f);
                break;
            }
            [[fallthrough]];
        case 0x80: note_off(ev.data1 & 0x7f); break;
        case 0xb0:
            if (ev.data1 == 1) wheel_cc_ = static_cast<float>(ev.data2) / 127.0f;
            if (ev.data1 == 64) set_pedal(ev.data2 >= 64);
            if (ev.data1 == 120) {
                for (Voice& v : voices_) {
                    v.active = false;
                    v.amp.kill();
                    v.env2.kill();
                }
                held_n_ = 0;
            }
            if (ev.data1 == 123) {
                held_n_ = 0;
                pedal_ = false;
                for (Voice& v : voices_) release(v);
            }
            break;
        case 0xd0: aftertouch_ = static_cast<float>(ev.data1) / 127.0f; break;
        case 0xa0: aftertouch_ = static_cast<float>(ev.data2) / 127.0f; break;
        case 0xe0: {
            const int raw = ((ev.data2 & 0x7f) << 7) | (ev.data1 & 0x7f);
            bend_ = static_cast<float>(raw - 8192) / 8192.0f * static_cast<float>(ichoice(v_[bend_range], 24));
            break;
        }
        default: break;
        }
    }

    void release(Voice& v) noexcept {
        v.gate = false;
        v.pedal_held = false;
        v.amp.gate_off();
        v.env2.gate_off();
    }

    void set_pedal(bool down) noexcept {
        pedal_ = down;
        if (down) return;
        for (Voice& v : voices_)
            if (v.pedal_held) release(v);
    }

    void start_voice(int i, int note, float vel, bool glide) noexcept {
        Voice& v = voices_[static_cast<std::size_t>(i)];
        const bool fresh = !v.active;
        v.active = true;
        v.gate = true;
        v.pedal_held = false;
        v.note = note;
        v.velocity = vel;
        v.order = ++order_;
        v.target = static_cast<float>(note);
        v.pitch = glide && last_note_ >= 0.0f ? last_note_ : v.target;
        v.since_on = 0.0f;
        v.amp.gate_on();
        v.env2.gate_on();
        if (fresh) {
            const bool random_phase = ichoice(v_[unison_voices], 7) > 0 || v_[drift] > 0.0f;
            v.reset_audio(++seed_ * 7919u, random_phase);
        }
        v.random = v.rng.bipolar();
        for (int k = 0; k < 2; ++k) {
            const bool retrig = v_[k == 0 ? lfo1_retrigger : lfo2_retrigger] >= 0.5f;
            v.lfo_ph[k] = retrig ? 0.0f : glfo_ph_[k];
            v.lfo_a[k] = v.rng.bipolar();
            v.lfo_b[k] = v.rng.bipolar();
            v.lfo_rate_mod[k] = v.lfo_depth_mod[k] = 0.0f;
        }
        newest_ = i;
    }

    int pick_mono_note() const noexcept {
        if (held_n_ == 0) return -1;
        const int prio = ichoice(v_[note_priority], 2);
        int best = held_[held_n_ - 1];
        for (int k = 0; k < held_n_; ++k) {
            if (prio == 1) best = std::min(best, held_[k]);
            if (prio == 2) best = std::max(best, held_[k]);
        }
        return best;
    }

    void note_on(int note, float vel) noexcept {
        const int mode = ichoice(v_[voice_mode], 2);
        const bool glide_on = v_[glide] > 0.0f;
        if (mode == 0) {
            int pick = -1;
            for (int i = 0; i < kVoices && pick < 0; ++i)
                if (voices_[static_cast<std::size_t>(i)].active && voices_[static_cast<std::size_t>(i)].note == note) pick = i;
            for (int i = 0; i < kVoices && pick < 0; ++i)
                if (!voices_[static_cast<std::size_t>(i)].active) pick = i;
            if (pick < 0) { // steal: the quietest released voice, else the oldest
                float quiet = 2.0f;
                for (int i = 0; i < kVoices; ++i) {
                    const Voice& v = voices_[static_cast<std::size_t>(i)];
                    if (!v.gate && v.amp.y < quiet) {
                        quiet = v.amp.y;
                        pick = i;
                    }
                }
                uint32_t oldest = UINT32_MAX;
                for (int i = 0; i < kVoices && quiet > 1.0f; ++i)
                    if (voices_[static_cast<std::size_t>(i)].order < oldest) {
                        oldest = voices_[static_cast<std::size_t>(i)].order;
                        pick = i;
                    }
            }
            start_voice(pick, note, vel, glide_on);
            last_note_ = static_cast<float>(note);
            return;
        }
        // Mono / Legato: one voice follows the held notes by priority.
        const bool was_held = held_n_ > 0;
        int k = 0;
        for (int j = 0; j < held_n_; ++j)
            if (held_[j] != note) held_[k++] = held_[j];
        held_n_ = k;
        if (held_n_ == kHeld) {
            std::memmove(held_, held_ + 1, sizeof(int) * (kHeld - 1));
            --held_n_;
        }
        held_[held_n_++] = note;
        const int chosen = pick_mono_note();
        Voice& v = voices_[0];
        const bool sounding = v.active && v.amp.stage != Env::kIdle;
        if (!sounding) {
            start_voice(0, chosen, vel, mode == 1 && glide_on);
        } else if (mode == 1 || !was_held) { // Mono retriggers; Legato only from released keys
            const bool glide = glide_on && (mode == 1 || was_held);
            last_note_ = v.pitch;
            start_voice(0, chosen, vel, glide);
            if (!glide) v.pitch = v.target;
        } else if (chosen != v.note) {
            v.note = chosen;
            v.target = static_cast<float>(chosen);
            if (!glide_on) v.pitch = v.target;
        }
        v.gate = true;
        last_note_ = static_cast<float>(chosen);
    }

    void note_off(int note) noexcept {
        const int mode = ichoice(v_[voice_mode], 2);
        if (mode == 0) {
            for (Voice& v : voices_) {
                if (!v.active || !v.gate || v.note != note) continue;
                if (pedal_) {
                    v.gate = false;
                    v.pedal_held = true;
                } else {
                    release(v);
                }
            }
            return;
        }
        int k = 0;
        for (int j = 0; j < held_n_; ++j)
            if (held_[j] != note) held_[k++] = held_[j];
        held_n_ = k;
        Voice& v = voices_[0];
        if (!v.active) return;
        const int chosen = pick_mono_note();
        if (chosen >= 0) {
            if (chosen != v.note) {
                v.note = chosen;
                v.target = static_cast<float>(chosen);
                if (v_[glide] <= 0.0f) v.pitch = v.target;
                last_note_ = static_cast<float>(chosen);
            }
            return;
        }
        if (pedal_) {
            v.gate = false;
            v.pedal_held = true;
        } else {
            release(v);
        }
    }

    // ------------------------------------------------------------ voices
    // Source values of a voice (or of no voice: only the global ones).
    void sources(const Voice* v, float* src) const noexcept {
        src[kSrcNone] = 0.0f;
        src[kSrcEnv2] = v != nullptr ? v->env2.y : 0.0f;
        src[kSrcAmpEnv] = v != nullptr ? v->amp.y : 0.0f;
        src[kSrcLfo1] = v != nullptr ? v->lfo_val[0] : global_lfo(0);
        src[kSrcLfo2] = v != nullptr ? v->lfo_val[1] : global_lfo(1);
        src[kSrcVelocity] = v != nullptr ? v->velocity : 0.0f;
        src[kSrcKey] = v != nullptr ? std::clamp((v->pitch - 60.0f) / 48.0f, -1.0f, 1.0f) : 0.0f;
        src[kSrcWheel] = std::clamp(v_[mod_wheel] + wheel_cc_, 0.0f, 1.0f);
        src[kSrcAftertouch] = aftertouch_;
        src[kSrcRandom] = v != nullptr ? v->random : 0.0f;
    }

    float global_lfo(int k) const noexcept {
        const auto b = static_cast<uint32_t>(k == 0 ? lfo1_shape : lfo2_shape);
        return lfo_shape(ichoice(v_[b], 6), glfo_ph_[k], 0.0f, 0.0f) * std::clamp(v_[b + 6], 0.0f, 1.0f);
    }

    void modulation(const float* src, float* m) const noexcept {
        std::fill(m, m + kTargetCount, 0.0f);
        for (int k = 0; k < n_routes_; ++k) m[routes_[k].target] += routes_[k].amount * src[routes_[k].source];
    }

    void render_voice(Voice& v, float* l, float* r, uint32_t n) noexcept {
        const float dt_s = static_cast<float>(n) * inv_sr_;
        // LFOs (with last tick's rate / depth modulation).
        for (int k = 0; k < 2; ++k) {
            const auto b = static_cast<uint32_t>(k == 0 ? lfo1_shape : lfo2_shape);
            const int shape = ichoice(v_[b], 6);
            const bool sync = v_[b + 2] >= 0.5f;
            const bool retrig = v_[b + 4] >= 0.5f;
            const float cyc = lfo_note_beats(ichoice(v_[b + 3], 16));
            if (sync && playing_ && !retrig && v.lfo_rate_mod[k] == 0.0f) {
                const double cycles = tick_beats_ / static_cast<double>(cyc);
                const auto ph = static_cast<float>(cycles - std::floor(cycles));
                if (ph < v.lfo_ph[k]) {
                    v.lfo_a[k] = v.lfo_b[k];
                    v.lfo_b[k] = v.rng.bipolar();
                }
                v.lfo_ph[k] = ph;
            } else {
                const float hz = (sync ? tempo_ / 60.0f / cyc : v_[b + 1]) * std::exp2(v.lfo_rate_mod[k]);
                float ph = v.lfo_ph[k] + std::min(hz * dt_s, 1.0f);
                if (ph >= 1.0f) {
                    ph -= 1.0f;
                    v.lfo_a[k] = v.lfo_b[k];
                    v.lfo_b[k] = v.rng.bipolar();
                }
                v.lfo_ph[k] = ph;
            }
            const float fade_s = v_[b + 5];
            const float fade = fade_s > 0.0f ? std::min(1.0f, v.since_on / fade_s) : 1.0f;
            const float depth = std::clamp(v_[b + 6] + v.lfo_depth_mod[k], 0.0f, 1.0f);
            v.lfo_val[k] = lfo_shape(shape, v.lfo_ph[k], v.lfo_a[k], v.lfo_b[k]) * depth * fade;
        }
        float src[kSourceCount], m[kTargetCount];
        sources(&v, src);
        modulation(src, m);
        v.lfo_rate_mod[0] = m[kTgtLfo1Rate];
        v.lfo_rate_mod[1] = m[kTgtLfo2Rate];
        v.lfo_depth_mod[0] = m[kTgtLfo1Depth];
        v.lfo_depth_mod[1] = m[kTgtLfo2Depth];
        v.since_on += dt_s;

        // Pitch: glide, bend, drift.
        const float glide_s = v_[glide];
        if (glide_s > 0.0f)
            v.pitch += (v.target - v.pitch) * (1.0f - std::exp(-3.0f * dt_s / glide_s));
        else
            v.pitch = v.target;
        const float base = v.pitch + bend_ + m[kTgtPitch];
        const int nu = ichoice(v_[unison_voices], 7) + 1;
        const float drift_amt = std::clamp(v_[drift], 0.0f, 1.0f);
        for (int u = 0; u < nu; ++u) {
            if (v.rng.unit() < 2.0f * dt_s) v.drift_to[u] = v.rng.bipolar() * drift_amt * 0.15f;
            v.drift[u] += (v.drift_to[u] - v.drift[u]) * std::min(1.0f, 3.0f * dt_s);
        }
        const float det = std::clamp(v_[unison_detune] + m[kTgtDetune], 0.0f, 1.0f);
        const float spread = std::clamp(v_[unison_spread], 0.0f, 1.0f);
        float note_o[2];
        int wave[2];
        float shape[2], level[2];
        for (int o = 0; o < 2; ++o) {
            const uint32_t b = o == 0 ? osc1_wave : osc2_wave;
            wave[o] = ichoice(v_[b], 5);
            shape[o] = std::clamp(v_[b + 1] + m[kTgtOsc1Shape + o], 0.0f, 1.0f);
            note_o[o] = base + static_cast<float>(12 * (ichoice(v_[b + 2], 4) - 2) + ichoice(v_[b + 3], 24) - 12) +
                        v_[b + 4] * 0.01f + m[kTgtOsc1Pitch + o];
            level[o] = std::clamp(v_[b + 5] + m[kTgtOsc1Level + o], 0.0f, 1.0f);
        }
        const float xfm = std::clamp(v_[cross_fm] + m[kTgtCrossFm], 0.0f, 1.0f);
        const float pm_amt = xfm * xfm * 0.4f;
        const float hz1 = st_to_hz(note_o[0]), hz2 = st_to_hz(note_o[1]);
        const float uni_norm = 1.0f / std::sqrt(static_cast<float>(nu));

        float bl[kTick] = {}, br[kTick] = {}, b1[kTick], b2[kTick];
        const bool need2 = level[1] > 0.0f || pm_amt > 0.0f;
        for (int u = 0; u < nu; ++u) {
            const float x = nu > 1 ? 2.0f * static_cast<float>(u) / static_cast<float>(nu - 1) - 1.0f : 0.0f;
            const float ratio = std::exp2((x * det * det * 0.6f + v.drift[u]) * (1.0f / 12.0f));
            float gl, gr;
            pan_gains(x * spread, gl, gr);
            gl *= 1.41421356f * uni_norm;
            gr *= 1.41421356f * uni_norm;
            if (need2)
                osc_block(wave[1], shape[1], std::min(hz2 * ratio * inv_sr_, 0.45f), v.osc[1][u], nullptr, 0.0f, b2, n);
            if (level[0] > 0.0f) {
                osc_block(wave[0], shape[0], std::min(hz1 * ratio * inv_sr_, 0.45f), v.osc[0][u],
                          pm_amt > 0.0f ? b2 : nullptr, pm_amt, b1, n);
                for (uint32_t i = 0; i < n; ++i) {
                    const float s = b1[i] * level[0] + (level[1] > 0.0f ? b2[i] * level[1] : 0.0f);
                    bl[i] += s * gl;
                    br[i] += s * gr;
                }
            } else if (level[1] > 0.0f) {
                for (uint32_t i = 0; i < n; ++i) {
                    bl[i] += b2[i] * level[1] * gl;
                    br[i] += b2[i] * level[1] * gr;
                }
            }
        }
        const float sub_lev = std::clamp(v_[sub_level] + m[kTgtSubLevel], 0.0f, 1.0f);
        if (sub_lev > 0.0f) {
            const float sub_note = base + static_cast<float>(ichoice(v_[sub_tune], 48) - 24);
            sub_block(ichoice(v_[sub_waveform], 3), std::clamp(v_[sub_shape], 0.0f, 1.0f),
                      std::min(st_to_hz(sub_note) * inv_sr_, 0.45f), v.sub_p, b1, n);
            for (uint32_t i = 0; i < n; ++i) {
                bl[i] += b1[i] * sub_lev;
                br[i] += b1[i] * sub_lev;
            }
        }
        const float noise_lev = std::clamp(v_[noise_level] + m[kTgtNoiseLevel], 0.0f, 1.0f);
        if (noise_lev > 0.0f)
            v.noise.block(ichoice(v_[noise_type], 6), std::clamp(v_[noise_color], 0.0f, 1.0f),
                          std::clamp(v_[noise_width], 0.0f, 1.0f), noise_lev, bl, br, n, sr_);

        // Filter.
        const int ftype = ichoice(v_[filter_type], 4);
        const float vel_scale = 1.0f - std::clamp(v_[filter_velocity], 0.0f, 1.0f) * (1.0f - v.velocity);
        const float oct = std::clamp(v_[key_track], 0.0f, 1.0f) * (v.pitch - 60.0f) / 12.0f +
                          std::clamp(v_[filter_env], -1.0f, 1.0f) * vel_scale * v.env2.y * 7.0f + m[kTgtCutoff];
        const float nyq = 0.45f * sr_;
        const float fc = std::clamp(v_[cutoff] * std::exp2(oct), 20.0f, std::min(20000.0f, nyq));
        const float res = std::clamp(v_[resonance] + m[kTgtResonance], 0.0f, 1.0f);
        const float drv = std::clamp(v_[filter_drive] + m[kTgtDrive], 0.0f, 1.0f);
        const float pre = 1.0f + 5.0f * drv;
        float post;
        float q_inv = 0.0f;
        if (ftype == 0) {
            const float k = 4.0f * res;
            const float g = std::tan(kPi * fc * inv_sr_);
            for (Ladder& f : v.ladder) f.set_g(g, k);
            post = (1.0f + 0.5f * k) / (1.0f + 1.5f * drv);
        } else {
            q_inv = std::max(2.0f - 1.94f * res, 0.06f);
            for (Svf& f : v.svf) f.set(fc, q_inv, sr_);
            post = 1.0f / (1.0f + 1.5f * drv);
        }
        const float hp_hz = std::clamp(v_[high_pass] * std::exp2(m[kTgtHighPass]), 10.0f, nyq);
        const bool hp_on = hp_hz > 10.5f;
        if (hp_on)
            for (int c = 0; c < 2; ++c) {
                v.hp1[c].set_hz(hp_hz, sr_);
                v.hp2[c].set_hz(hp_hz, sr_);
            }

        // Amp: velocity, Amp and Pan modulation; the gain ramps over the tick.
        const float vel_gain = 1.0f - std::clamp(v_[amp_velocity], 0.0f, 1.0f) * (1.0f - v.velocity);
        const float g1 = kOutScale * vel_gain * std::clamp(1.0f + m[kTgtAmp], 0.0f, 2.0f);
        const float g0 = v.primed ? v.gain : g1;
        v.gain = g1;
        v.primed = true;
        float pl, pr;
        pan_gains(std::clamp(m[kTgtPan], -1.0f, 1.0f), pl, pr);
        pl *= 1.41421356f;
        pr *= 1.41421356f;
        const float step = (g1 - g0) / static_cast<float>(n);
        float* io[2] = {bl, br};
        for (uint32_t i = 0; i < n; ++i) {
            const float a = v.amp.step(amp_c_);
            v.env2.step(env2_c_);
            const float g = a * (g0 + step * static_cast<float>(i + 1));
            for (int c = 0; c < 2; ++c) {
                float x = io[c][i];
                if (ftype == 0) {
                    x = v.ladder[c].run(x * pre) * post;
                } else {
                    if (drv > 0.0f) x = v.drive[c].run(x * pre);
                    x *= post;
                    float band, high;
                    const float low = v.svf[c].run(x, band, high);
                    x = ftype == 1 ? low : (ftype == 2 ? band * q_inv : (ftype == 3 ? high : low + high));
                }
                if (hp_on) x = v.hp2[c].hp(v.hp1[c].hp(x));
                io[c][i] = x * g;
            }
            l[i] += bl[i] * pl;
            r[i] += br[i] * pr;
        }
        for (int c = 0; c < 2; ++c) {
            v.ladder[c].flush();
            v.svf[c].flush();
            v.hp1[c].flush();
            v.hp2[c].flush();
        }
        if (v.amp.stage == Env::kIdle) {
            v.active = false;
            v.gate = false;
            v.env2.kill();
        }
    }

    // Voice that drives per-voice modulation sources for effects and readouts: the most
    // recently started voice that is still sounding (null when none is).
    const Voice* mod_voice() const noexcept {
        if (newest_ >= 0 && voices_[static_cast<std::size_t>(newest_)].active) return &voices_[static_cast<std::size_t>(newest_)];
        const Voice* best = nullptr;
        for (const Voice& v : voices_)
            if (v.active && (best == nullptr || v.order > best->order)) best = &v;
        return best;
    }

    // ------------------------------------------------------------ effects
    void run_fx(float* l, float* r, uint32_t n, const BuiltinContext& ctx) noexcept {
        float src[kSourceCount], m[kTargetCount];
        const Voice* nv = mod_voice();
        sources(nv, src);
        modulation(src, m);
        FxClock clk;
        clk.tempo_bpm = static_cast<double>(tempo_);
        clk.beats = beats_;
        clk.playing = playing_;
        (void)ctx;
        for (int s = 0; s < kFxSlots; ++s) {
            const auto base = static_cast<uint32_t>(fx1_type + s * kFxParamCount);
            std::memcpy(fxp_.data(), v_.data() + base, sizeof(float) * kFxParamCount);
            if (fxp_[kFxOn] < 0.5f) {
                fx_[static_cast<std::size_t>(s)].suspend();
                continue;
            }
            const float mm = m[kTgtFxMix1 + s];
            if (mm != 0.0f) fxp_[kFxMix] = from_norm(base + kFxMix, to_norm(base + kFxMix, fxp_[kFxMix]) + mm);
            const float ma = m[kTgtFxAmount1 + s];
            if (ma != 0.0f) {
                const uint32_t a = kFxAmount[ichoice(fxp_[kFxType], kFxTypeCount - 1)];
                fxp_[a] = from_norm(base + a, to_norm(base + a, fxp_[a]) + ma);
            }
            fx_[static_cast<std::size_t>(s)].process(l, r, n, fxp_.data(), clk);
        }
    }

    void write_readouts(float* ro, float peak) const noexcept {
        int count = 0;
        for (const Voice& v : voices_) count += v.active ? 1 : 0;
        ro[kRoVoices] = static_cast<float>(count);
        const Voice* nv = mod_voice();
        ro[kRoAmpEnv] = nv != nullptr ? nv->amp.y : 0.0f;
        ro[kRoEnv2] = nv != nullptr ? nv->env2.y : 0.0f;
        ro[kRoLfo1] = nv != nullptr ? nv->lfo_val[0] : global_lfo(0);
        ro[kRoLfo2] = nv != nullptr ? nv->lfo_val[1] : global_lfo(1);
        ro[kRoLfo1Phase] = nv != nullptr ? nv->lfo_ph[0] : glfo_ph_[0];
        ro[kRoLfo2Phase] = nv != nullptr ? nv->lfo_ph[1] : glfo_ph_[1];
        for (int s = 0; s < kFxSlots; ++s) {
            const FxTelemetry& t = fx_[static_cast<std::size_t>(s)].telemetry();
            const auto o = static_cast<uint32_t>(kRoFx1Gr + 3 * s);
            ro[o] = t.gr_db;
            ro[o + 1] = t.phase;
            ro[o + 2] = t.cutoff_hz;
        }
        ro[kRoOutDb] = peak > 1e-6f ? 20.0f * std::log10(peak) : -120.0f;
        float src[kSourceCount];
        sources(nv, src);
        modulation(src, ro + kRoMod);
        std::copy(src + 1, src + kSourceCount, ro + kRoSrc);
    }

    float sr_ = 48000.0f, inv_sr_ = 1.0f / 48000.0f;
    std::array<float, kParamCount> v_{};
    std::array<float, kFxParamCount> fxp_{};
    std::array<Voice, kVoices> voices_{};
    std::array<FxSlot, kFxSlots> fx_;
    Route routes_[kRoutes] = {};
    int n_routes_ = 0;
    EnvCoef amp_c_, env2_c_;
    int held_[kHeld] = {};
    int held_n_ = 0;
    uint32_t order_ = 0, seed_ = 1;
    int newest_ = -1;
    float last_note_ = -1.0f;
    float bend_ = 0.0f, wheel_cc_ = 0.0f, aftertouch_ = 0.0f;
    bool pedal_ = false;
    float glfo_ph_[2] = {0.0f, 0.0f};
    float tempo_ = 120.0f;
    double beats_ = 0.0;
    double tick_beats_ = 0.0; // beat position of the current control tick
    int mode_ = 0;
    bool playing_ = false;
    dsp::Smoother volume_;
    bool volume_primed_ = false;
    bool asleep_ = true;
    uint32_t quiet_ = 0; // samples without voices below the silence threshold
};

std::unique_ptr<BuiltinProcessor> create_synth_v2() { return std::make_unique<SynthV2>(); }

} // namespace

extern const BuiltinDescriptor kSynthV2Descriptor;
const BuiltinDescriptor kSynthV2Descriptor{"synth-v2", synth_v2::kParamIds, kReadoutIds, &create_synth_v2, kIdleReadouts,
                                           synth_v2::kDefaults, true};

} // namespace dstudio
