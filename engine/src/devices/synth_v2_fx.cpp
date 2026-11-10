#include "synth_v2_fx.hpp"

#include <algorithm>
#include <cmath>
#include <cstring>
#include <vector>

namespace dstudio::synth_v2 {

namespace {

float sync_rate(float hz, bool sync, int note, const FxClock& c) noexcept {
    if (!sync) return hz;
    return static_cast<float>(c.tempo_bpm / 60.0) / lfo_note_beats(note);
}

// An LFO phase that follows the song position while playing and synced,
// and runs free otherwise.
struct FxLfo {
    double phase = 0.0;
    float advance(float hz, bool sync, int note, const FxClock& c, uint32_t n, float sr) noexcept {
        if (sync && c.playing) {
            const auto beats_per_cycle = static_cast<double>(lfo_note_beats(note));
            phase = c.beats / beats_per_cycle;
        }
        phase -= std::floor(phase);
        const double p = phase;
        phase += static_cast<double>(hz) * n / static_cast<double>(sr);
        return static_cast<float>(p);
    }
};

// ---------------------------------------------------------------- tape
// drive -> soft compression -> asymmetric saturation (ADAA) -> head
// bump / warmth tilt -> machine bandwidth -> wow and flutter -> hiss.
struct Tape {
    float sr = 48000.0f;
    AdaaTanh sat[2];
    OnePole bump[2], hf[2], hf2[2], hiss_tone[2];
    DcBlock dc[2];
    DelayBuf wow[2];
    float env = 0.0f;
    double wow_phase = 0.0, flutter_phase = 0.0;
    Rng rng;
    void prepare(float rate) {
        sr = rate;
        for (int c = 0; c < 2; ++c) {
            wow[c].resize(static_cast<std::size_t>(0.03f * rate));
            dc[c].set(rate);
        }
    }
    void reset() noexcept {
        for (int c = 0; c < 2; ++c) {
            sat[c].reset();
            bump[c].reset();
            hf[c].reset();
            hf2[c].reset();
            hiss_tone[c].reset();
            dc[c].reset();
            wow[c].clear();
        }
        env = 0.0f;
        rng.seed(0x7a9e);
    }
    void process(float* l, float* r, uint32_t n, const float* p) noexcept {
        const float drive = p[kTapeDrive];
        const float warmth = p[kTapeWarmth];
        const float comp = p[kTapeCompression];
        const float noise = p[kTapeNoise];
        const int mode = static_cast<int>(p[kTapeMode]);
        const float out = db_to_gain(p[kTapeOutput]);
        // Machine: bandwidth, wow depth (ms), flutter depth (ms), noise colour.
        constexpr float kBw[3] = {18000.0f, 11000.0f, 6500.0f};
        constexpr float kWow[3] = {0.05f, 0.4f, 0.8f};
        constexpr float kFlutter[3] = {0.004f, 0.015f, 0.03f};
        const float pre = 1.0f + 6.0f * drive * drive;
        const float post = 1.0f / std::sqrt(pre);
        for (int c = 0; c < 2; ++c) {
            bump[c].set_hz(110.0f, sr);
            hf[c].set_hz(kBw[mode] * (1.0f - 0.35f * warmth), sr);
            hf2[c].set_hz(kBw[mode] * 1.3f, sr);
            hiss_tone[c].set_hz(6000.0f, sr);
        }
        const float env_a = 1.0f - std::exp(-1.0f / (0.05f * sr));
        const float wow_inc = 0.55f / sr, flutter_inc = 6.3f / sr;
        const float base_ms = 1.0f + kWow[mode] + kFlutter[mode];
        const float hiss = noise * noise * 0.02f;
        for (uint32_t i = 0; i < n; ++i) {
            const float lvl = 0.5f * (std::fabs(l[i]) + std::fabs(r[i]));
            env += env_a * (lvl - env);
            const float g = pre / (1.0f + 3.0f * comp * env * pre);
            const float w = static_cast<float>(kWow[mode] * sin_turns(static_cast<float>(wow_phase)) +
                                               kFlutter[mode] * sin_turns(static_cast<float>(flutter_phase)));
            wow_phase += static_cast<double>(wow_inc);
            flutter_phase += static_cast<double>(flutter_inc);
            if (wow_phase >= 1.0) wow_phase -= 1.0;
            if (flutter_phase >= 1.0) flutter_phase -= 1.0;
            float* io[2] = {l + i, r + i};
            for (int c = 0; c < 2; ++c) {
                float x = *io[c] * g;
                x = sat[c].run(x + 0.08f * drive) - fast_tanh(0.08f * drive);
                x = dc[c].run(x) * post * (1.0f + 0.5f * comp);
                const float low = bump[c].lp(x);
                x += warmth * 0.8f * low;
                x = hf2[c].lp(hf[c].lp(x));
                wow[c].push(x);
                x = wow[c].read((base_ms + w) * 0.001f * sr);
                if (hiss > 0.0f) x += hiss * hiss_tone[c].hp(rng.bipolar());
                *io[c] = x * out;
            }
        }
        env = env < 1e-20f ? 0.0f : env;
        for (int c = 0; c < 2; ++c) {
            bump[c].flush();
            hf[c].flush();
            hf2[c].flush();
        }
    }
};

// ---------------------------------------------------------------- distortion
struct Distortion {
    float sr = 48000.0f;
    AdaaTanh od[2];
    AdaaCubic fuzz[2];
    AdaaTube tube[2];
    AdaaHard hard[2];
    OnePole tone[2], tone2[2];
    DcBlock dc[2];
    void prepare(float rate) {
        sr = rate;
        for (DcBlock& d : dc) d.set(rate);
    }
    void reset() noexcept {
        for (int c = 0; c < 2; ++c) {
            od[c].reset();
            fuzz[c].reset();
            tube[c].reset();
            hard[c].reset();
            tone[c].reset();
            tone2[c].reset();
            dc[c].reset();
        }
    }
    void process(float* l, float* r, uint32_t n, const float* p) noexcept {
        const float drive = p[kDistDrive];
        const int mode = static_cast<int>(p[kDistMode]);
        const float bias = 0.6f * p[kDistBias];
        const float level = db_to_gain(p[kDistLevel]);
        // Fuzz drives hardest; the output is normalised so Drive mostly
        // changes the character, not the loudness.
        constexpr float kMaxGain[4] = {40.0f, 90.0f, 30.0f, 40.0f};
        const float g = 1.0f + kMaxGain[mode] * drive * drive;
        const float comp = 1.0f / (0.6f + 0.4f * std::sqrt(g));
        for (int c = 0; c < 2; ++c) {
            tone[c].set_hz(p[kDistTone], sr);
            tone2[c].set_hz(p[kDistTone] * 1.5f, sr);
        }
        float* io[2] = {l, r};
        for (int c = 0; c < 2; ++c) {
            float* x = io[c];
            for (uint32_t i = 0; i < n; ++i) {
                const float in = x[i] * g + bias;
                float y;
                switch (mode) {
                case 1: y = 1.5f * fuzz[c].run(in) - 1.5f * cubic_f(bias); break;
                case 2: y = tube[c].run(in) - tube_f(bias); break;
                case 3: y = hard[c].run(in) - hardclip_f(bias); break;
                default: y = od[c].run(in) - fast_tanh(bias); break;
                }
                y = dc[c].run(y);
                x[i] = tone2[c].lp(tone[c].lp(y)) * comp * level;
            }
            tone[c].flush();
            tone2[c].flush();
        }
    }
};

// ---------------------------------------------------------------- bitcrusher
struct Bitcrusher {
    float hold[2] = {0, 0};
    float phase = 1.0f; // >= 1: take a new sample
    float period = 1.0f;
    Rng rng;
    void reset() noexcept {
        hold[0] = hold[1] = 0.0f;
        phase = 1.0f;
        period = 1.0f;
        rng.seed(0x51ed);
    }
    void process(float* l, float* r, uint32_t n, const float* p) noexcept {
        const float bits = p[kCrushBits];
        const float factor = p[kCrushDownsample];
        const float dither = p[kCrushDither];
        const float jitter = p[kCrushJitter];
        const float levels = std::exp2(bits - 1.0f); // steps per unit amplitude
        const float step = 1.0f / levels;
        for (uint32_t i = 0; i < n; ++i) {
            phase += 1.0f;
            if (phase >= period) {
                phase -= period;
                period = std::max(1.0f, factor * (1.0f + jitter * 0.5f * rng.bipolar()));
                for (int c = 0; c < 2; ++c) {
                    const float x = c == 0 ? l[i] : r[i];
                    const float d = dither * step * 0.5f * (rng.bipolar() + rng.bipolar());
                    hold[c] = std::clamp(std::round((x + d) * levels) * step, -1.0f - step, 1.0f + step);
                }
            }
            l[i] = hold[0];
            r[i] = hold[1];
        }
    }
};

// ---------------------------------------------------------------- chorus
// Up to four modulated taps per channel around 7..19 ms; the right channel's
// LFOs are offset by Width x 90 degrees.
struct Chorus {
    float sr = 48000.0f;
    DelayBuf line[2];
    float fb[2] = {0, 0};
    FxLfo lfo;
    float last_phase = 0.0f;
    void prepare(float rate) {
        sr = rate;
        for (DelayBuf& d : line) d.resize(static_cast<std::size_t>(0.06f * rate));
    }
    void reset() noexcept {
        for (DelayBuf& d : line) d.clear();
        fb[0] = fb[1] = 0.0f;
        lfo.phase = 0.0;
    }
    void process(float* l, float* r, uint32_t n, const float* p, const FxClock& clk) noexcept {
        const float hz = sync_rate(p[kChorusRate], p[kChorusSync] > 0.5f, static_cast<int>(p[kChorusNote]), clk);
        const int voices = static_cast<int>(p[kChorusVoices]) + 1;
        const float depth = p[kChorusDepth] * sr; // samples, peak to peak
        const float width = p[kChorusWidth];
        const float feedback = p[kChorusFeedback];
        float ph = lfo.advance(hz, p[kChorusSync] > 0.5f, static_cast<int>(p[kChorusNote]), clk, n, sr);
        last_phase = ph;
        const float inc = hz / sr;
        const float norm = 1.0f / std::sqrt(static_cast<float>(voices));
        for (uint32_t i = 0; i < n; ++i) {
            float* io[2] = {l + i, r + i};
            for (int c = 0; c < 2; ++c) {
                line[c].push(*io[c] + fb[c] * feedback);
                float wet = 0.0f;
                for (int v = 0; v < voices; ++v) {
                    const float off = static_cast<float>(v) / static_cast<float>(voices) + (c == 1 ? 0.25f * width : 0.0f);
                    const float m = 0.5f + 0.5f * sin_turns(ph + off);
                    const float base = (0.007f + 0.004f * static_cast<float>(v)) * sr;
                    wet += line[c].read(base + depth * m);
                }
                wet *= norm;
                fb[c] = std::clamp(wet, -2.0f, 2.0f);
                *io[c] = wet;
            }
            ph += inc;
            if (ph >= 1.0f) ph -= 1.0f;
        }
    }
};

// ---------------------------------------------------------------- flanger
// Classic: 0.25 ms + depth sweep; Jet: shorter base and a sharper,
// feedback-forward sweep; Thru-Zero: the reference path is delayed by half
// the depth so the swept tap passes through zero delay.
struct Flanger {
    float sr = 48000.0f;
    DelayBuf line[2], ref[2];
    float fb[2] = {0, 0};
    FxLfo lfo;
    float last_phase = 0.0f;
    void prepare(float rate) {
        sr = rate;
        for (int c = 0; c < 2; ++c) {
            line[c].resize(static_cast<std::size_t>(0.03f * rate));
            ref[c].resize(static_cast<std::size_t>(0.03f * rate));
        }
    }
    void reset() noexcept {
        for (int c = 0; c < 2; ++c) {
            line[c].clear();
            ref[c].clear();
        }
        fb[0] = fb[1] = 0.0f;
        lfo.phase = 0.0;
    }
    void process(float* l, float* r, uint32_t n, const float* p, const FxClock& clk) noexcept {
        const bool sync = p[kFlangerSync] > 0.5f;
        const int note = static_cast<int>(p[kFlangerNote]);
        const float hz = sync_rate(p[kFlangerRate], sync, note, clk);
        const int mode = static_cast<int>(p[kFlangerMode]);
        const float depth = std::max(p[kFlangerDepth] * sr, 1.0f);
        const float feedback = p[kFlangerFeedback] * (mode == 1 ? 1.05f : 1.0f);
        const float width = p[kFlangerWidth];
        const float base = mode == 1 ? 0.0001f * sr : (mode == 2 ? 0.0f : 0.00025f * sr);
        float ph = lfo.advance(hz, sync, note, clk, n, sr);
        last_phase = ph;
        const float inc = hz / sr;
        for (uint32_t i = 0; i < n; ++i) {
            float* io[2] = {l + i, r + i};
            for (int c = 0; c < 2; ++c) {
                const float off = c == 1 ? 0.5f * width : 0.0f;
                float m = 0.5f + 0.5f * sin_turns(ph + off);
                if (mode == 1) m = m * m; // jet: lingers at the short end
                const float x = *io[c];
                line[c].push(x + std::clamp(fb[c], -1.5f, 1.5f) * std::clamp(feedback, -0.95f, 0.95f));
                ref[c].push(x);
                float swept, dry;
                if (mode == 2) {
                    swept = line[c].read(1.0f + depth * m);
                    dry = ref[c].read(1.0f + 0.5f * depth);
                } else {
                    swept = line[c].read(1.0f + base + depth * m);
                    dry = x;
                }
                fb[c] = swept;
                *io[c] = 0.5f * (dry + swept);
            }
            ph += inc;
            if (ph >= 1.0f) ph -= 1.0f;
        }
    }
};

// ---------------------------------------------------------------- phaser
// 2..12 first-order all-pass stages swept around 800 Hz by up to +-3.5
// octaves, with feedback from the last stage.
struct Phaser {
    static constexpr int kMaxStages = 12;
    float sr = 48000.0f;
    float x1[2][kMaxStages] = {}, y1[2][kMaxStages] = {};
    float fb[2] = {0, 0};
    FxLfo lfo;
    float last_phase = 0.0f;
    void prepare(float rate) { sr = rate; }
    void reset() noexcept {
        std::memset(x1, 0, sizeof x1);
        std::memset(y1, 0, sizeof y1);
        fb[0] = fb[1] = 0.0f;
        lfo.phase = 0.0;
    }
    void process(float* l, float* r, uint32_t n, const float* p, const FxClock& clk) noexcept {
        const bool sync = p[kPhaserSync] > 0.5f;
        const int note = static_cast<int>(p[kPhaserNote]);
        const float hz = sync_rate(p[kPhaserRate], sync, note, clk);
        constexpr int kStages[4] = {2, 4, 6, 12};
        const int stages = kStages[std::clamp(static_cast<int>(p[kPhaserStages]), 0, 3)];
        const float depth = p[kPhaserDepth];
        const float feedback = p[kPhaserFeedback];
        const float width = p[kPhaserWidth];
        float ph = lfo.advance(hz, sync, note, clk, n, sr);
        last_phase = ph;
        const float inc = hz / sr;
        float a[2] = {0, 0};
        for (uint32_t i = 0; i < n; ++i) {
            if ((i & 7u) == 0) {
                for (int c = 0; c < 2; ++c) {
                    const float m = sin_turns(ph + (c == 1 ? 0.25f * width : 0.0f));
                    const float f = std::min(800.0f * std::exp2(3.5f * depth * m), 0.45f * sr);
                    const float t = std::tan(kPi * f / sr);
                    a[c] = (t - 1.0f) / (t + 1.0f);
                }
            }
            float* io[2] = {l + i, r + i};
            for (int c = 0; c < 2; ++c) {
                float x = *io[c] + feedback * fb[c];
                for (int s = 0; s < stages; ++s) {
                    const float y = a[c] * x + x1[c][s] - a[c] * y1[c][s];
                    x1[c][s] = x;
                    y1[c][s] = y;
                    x = y;
                }
                fb[c] = std::clamp(x, -2.0f, 2.0f);
                *io[c] = 0.5f * (*io[c] + x);
            }
            ph += inc;
            if (ph >= 1.0f) ph -= 1.0f;
        }
        for (int c = 0; c < 2; ++c)
            for (int s = 0; s < kMaxStages; ++s) {
                if (std::fabs(y1[c][s]) < 1e-20f || !std::isfinite(y1[c][s])) y1[c][s] = 0.0f;
                if (!std::isfinite(x1[c][s])) x1[c][s] = 0.0f;
            }
    }
};

// ---------------------------------------------------------------- filter
// State-variable filter (12 dB) or two in series (24 dB) with a drive
// stage, an envelope follower and an LFO moving the cutoff in semitones.
struct FilterFx {
    float sr = 48000.0f;
    Svf svf[2][2];
    AdaaTanh drive[2];
    float env = 0.0f;
    FxLfo lfo;
    float rnd_a = 0.0f, rnd_b = 0.0f;
    Rng rng;
    float cutoff_now = 1000.0f, last_phase = 0.0f;
    void prepare(float rate) { sr = rate; }
    void reset() noexcept {
        for (auto& ch : svf)
            for (Svf& s : ch) s.reset();
        for (AdaaTanh& d : drive) d.reset();
        env = 0.0f;
        lfo.phase = 0.0;
        rng.seed(0x3f11);
        rnd_a = rng.bipolar();
        rnd_b = rng.bipolar();
    }
    void process(float* l, float* r, uint32_t n, const float* p, const FxClock& clk) noexcept {
        const int type = std::clamp(static_cast<int>(p[kFiltType]), 0, 7);
        const bool steep = (type & 1) != 0;
        const int kind = type / 2; // 0 low, 1 high, 2 band, 3 notch
        const float res = p[kFiltResonance];
        const float q_inv = 2.0f - 1.94f * res;
        const float drive_db = p[kFiltDrive];
        const float pre = db_to_gain(drive_db);
        const float post = drive_db > 0.01f ? 1.0f / std::sqrt(pre) : 1.0f;
        const bool sync = p[kFiltLfoSync] > 0.5f;
        const int note = static_cast<int>(p[kFiltLfoNote]);
        const float hz = sync_rate(p[kFiltLfoRate], sync, note, clk);
        const int shape = static_cast<int>(p[kFiltLfoShape]);
        const float ph0 = lfo.advance(hz, sync, note, clk, n, sr);
        const float env_att = 1.0f - std::exp(-1.0f / (0.005f * sr));
        const float env_rel = 1.0f - std::exp(-1.0f / (0.15f * sr));
        const float env_st = p[kFiltEnv];
        const float ph_step = hz / sr;
        constexpr uint32_t kTick = 16;
        for (uint32_t i0 = 0; i0 < n; i0 += kTick) {
            const uint32_t i1 = std::min(n, i0 + kTick);
            float ph = ph0 + ph_step * static_cast<float>(i0);
            ph -= std::floor(ph);
            if (ph < last_phase) { // a new LFO cycle began since the last tick
                rnd_a = rnd_b;
                rnd_b = rng.bipolar();
            }
            last_phase = ph;
            float lfo_v;
            switch (shape) {
            case 0: lfo_v = sin_turns(ph); break;
            case 1: lfo_v = lfo_shape(1, ph, 0, 0); break;
            case 2: lfo_v = 2.0f * ph - 1.0f; break;
            case 3: lfo_v = ph < 0.5f ? 1.0f : -1.0f; break;
            case 4: lfo_v = rnd_a; break;
            default: lfo_v = lfo_shape(6, ph, rnd_a, rnd_b); break;
            }
            const float lfo_st = p[kFiltLfoAmount] * lfo_v;
            for (uint32_t i = i0; i < i1; ++i) {
                const float lvl = 0.5f * (std::fabs(l[i]) + std::fabs(r[i]));
                env += (lvl > env ? env_att : env_rel) * (lvl - env);
            }
            const float st = lfo_st + env_st * std::min(env * 2.0f, 1.0f);
            const float fc = std::clamp(p[kFiltCutoff] * std::exp2(st / 12.0f), 20.0f, std::min(20000.0f, 0.45f * sr));
            cutoff_now = fc;
            for (auto& ch : svf)
                for (Svf& s : ch) s.set(fc, steep ? std::max(q_inv, 0.3f) : q_inv, sr);
            float* io[2] = {l, r};
            for (int c = 0; c < 2; ++c) {
                for (uint32_t i = i0; i < i1; ++i) {
                    float x = io[c][i];
                    if (drive_db > 0.01f) x = drive[c].run(x * pre) * post;
                    for (int stage = 0; stage < (steep ? 2 : 1); ++stage) {
                        float band, high;
                        const float low = svf[c][stage].run(x, band, high);
                        x = kind == 0 ? low : (kind == 1 ? high : (kind == 2 ? band * q_inv : low + high));
                    }
                    io[c][i] = x;
                }
            }
        }
        env = env < 1e-20f ? 0.0f : env;
        for (auto& ch : svf)
            for (Svf& s : ch) s.flush();
    }
};

// ---------------------------------------------------------------- delay
// Stereo delay with linked or separate times (free or tempo notes). Width:
// 0 mono, 1 stereo, 2 ping-pong. Modes colour the feedback path: Tape
// (low-pass, saturation, wow), Digital (clean), BBD (darker, clipped,
// slight hiss). Grit adds drive in the loop; Mod Depth wobbles the time.
struct DelayFx {
    float sr = 48000.0f;
    DelayBuf line[2];
    OnePole tone[2], low[2];
    AdaaTanh sat[2];
    float fb[2] = {0, 0};
    float time_s[2] = {0.5f, 0.5f}; // smoothed delay times (samples)
    double mod_phase = 0.0;
    Rng rng;
    bool primed = false;
    void prepare(float rate) {
        sr = rate;
        for (DelayBuf& d : line) d.resize(static_cast<std::size_t>(4.1f * rate));
    }
    void reset() noexcept {
        for (int c = 0; c < 2; ++c) {
            line[c].clear();
            tone[c].reset();
            low[c].reset();
            sat[c].reset();
        }
        fb[0] = fb[1] = 0.0f;
        primed = false;
        rng.seed(0xd31a);
    }
    void process(float* l, float* r, uint32_t n, const float* p, const FxClock& clk) noexcept {
        const bool sync = p[kDelaySync] > 0.5f;
        const bool link = p[kDelayLink] > 0.5f;
        const float beat_s = static_cast<float>(60.0 / std::max(clk.tempo_bpm, 1.0));
        float t[2];
        t[0] = sync ? delay_note_beats(static_cast<int>(p[kDelayNoteL])) * beat_s : p[kDelayTimeL];
        t[1] = link ? t[0] : (sync ? delay_note_beats(static_cast<int>(p[kDelayNoteR])) * beat_s : p[kDelayTimeR]);
        const float feedback = p[kDelayFeedback];
        const int mode = static_cast<int>(p[kDelayMode]);
        const float grit = p[kDelayGrit];
        const float mod = p[kDelayModDepth];
        const float width = p[kDelayWidth];
        constexpr float kTone[3] = {4500.0f, 18000.0f, 3200.0f};
        for (int c = 0; c < 2; ++c) {
            tone[c].set_hz(kTone[mode], sr);
            low[c].set_hz(mode == 1 ? 20.0f : 90.0f, sr);
        }
        float target[2];
        for (int c = 0; c < 2; ++c) target[c] = std::clamp(t[c], 0.001f, 4.0f) * sr;
        if (!primed) {
            time_s[0] = target[0];
            time_s[1] = target[1];
            primed = true;
        }
        const float slew = 1.0f - std::exp(-1.0f / (0.08f * sr));
        const float drive = 1.0f + 4.0f * grit + (mode == 1 ? 0.0f : 0.5f);
        const float mod_amp = mod * (mode == 0 ? 0.004f : 0.0025f) * sr;
        const float mod_inc = (mode == 0 ? 0.7f : 0.45f) / sr;
        const float pingpong = std::clamp(width - 1.0f, 0.0f, 1.0f);
        const float stereo = std::clamp(width, 0.0f, 1.0f);
        const float hiss = mode == 2 ? 0.0004f : 0.0f;
        for (uint32_t i = 0; i < n; ++i) {
            const float m = mod_amp * sin_turns(static_cast<float>(mod_phase));
            mod_phase += static_cast<double>(mod_inc);
            if (mod_phase >= 1.0) mod_phase -= 1.0;
            const float in_l = l[i], in_r = r[i];
            const float mid = 0.5f * (in_l + in_r);
            // Input: mono .. stereo by Width; ping-pong feeds the left line only.
            const float sl = mid + stereo * (in_l - mid);
            const float srr = mid + stereo * (in_r - mid);
            float wet[2];
            for (int c = 0; c < 2; ++c) {
                time_s[c] += slew * (target[c] - time_s[c]);
                wet[c] = line[c].read(time_s[c] + m * (c == 0 ? 1.0f : -1.0f));
            }
            for (int c = 0; c < 2; ++c) {
                float y = tone[c].lp(wet[c]);
                y = low[c].hp(y);
                if (mode != 1 || grit > 0.0f) y = sat[c].run(y * drive) / drive;
                if (hiss > 0.0f) y += hiss * rng.bipolar();
                fb[c] = y;
            }
            // Ping-pong: each line feeds the other; stereo: each feeds itself.
            const float to_l = (1.0f - pingpong) * fb[0] + pingpong * fb[1];
            const float to_r = (1.0f - pingpong) * fb[1] + pingpong * fb[0];
            line[0].push((1.0f - pingpong) * sl + pingpong * (sl + srr) * 0.5f + feedback * to_l);
            line[1].push((1.0f - pingpong) * srr + feedback * to_r);
            const float wm = 0.5f * (wet[0] + wet[1]);
            l[i] = wm + stereo * (wet[0] - wm);
            r[i] = wm + stereo * (wet[1] - wm);
        }
        for (int c = 0; c < 2; ++c) {
            tone[c].flush();
            low[c].flush();
            if (!std::isfinite(fb[c])) {
                line[c].clear();
                fb[c] = 0.0f;
            }
        }
    }
};

// ---------------------------------------------------------------- reverb
// Pre-delay -> input diffusion (4 all-passes) -> 8-line feedback delay
// network (Householder mixing, damping low-pass and RT60 gain per line,
// two lines slowly modulated) -> width -> low / high cut. Modes set the
// line lengths, the diffusion and the colour; Spring adds a dispersive
// all-pass chain for its chirp.
struct Reverb {
    static constexpr int kLines = 8;
    static constexpr int kDiff = 4;
    static constexpr int kChirp = 12;
    float sr = 48000.0f;
    DelayBuf pre;
    DelayBuf line[kLines];
    DelayBuf diff[2][kDiff];
    OnePole damp[kLines];
    OnePole lo_cut[2], hi_cut[2], hi_cut2[2];
    float chirp_x[kChirp] = {}, chirp_y[kChirp] = {};
    double mod_phase = 0.0;
    void prepare(float rate) {
        sr = rate;
        pre.resize(static_cast<std::size_t>(0.11f * rate));
        for (DelayBuf& d : line) d.resize(static_cast<std::size_t>(0.16f * rate));
        for (auto& ch : diff)
            for (DelayBuf& d : ch) d.resize(static_cast<std::size_t>(0.03f * rate));
    }
    void reset() noexcept {
        pre.clear();
        for (DelayBuf& d : line) d.clear();
        for (auto& ch : diff)
            for (DelayBuf& d : ch) d.clear();
        for (OnePole& o : damp) o.reset();
        for (int c = 0; c < 2; ++c) {
            lo_cut[c].reset();
            hi_cut[c].reset();
            hi_cut2[c].reset();
        }
        std::memset(chirp_x, 0, sizeof chirp_x);
        std::memset(chirp_y, 0, sizeof chirp_y);
        mod_phase = 0.0;
    }
    void process(float* l, float* r, uint32_t n, const float* p) noexcept {
        const int mode = std::clamp(static_cast<int>(p[kRevMode]), 0, 3);
        // Line lengths (ms) at full size, mutually prime-ish.
        constexpr float kLen[kLines] = {29.7f, 37.1f, 41.1f, 43.7f, 53.0f, 61.3f, 67.9f, 73.3f};
        constexpr float kModeScale[4] = {0.45f, 1.0f, 0.6f, 0.38f};
        constexpr float kDiffMs[kDiff] = {4.7f, 3.6f, 12.7f, 9.3f};
        constexpr float kDiffGain[4] = {0.6f, 0.7f, 0.78f, 0.55f};
        const float scale = kModeScale[mode] * (0.35f + 1.15f * p[kRevSize]);
        const float rt60 = p[kRevDecay];
        const float damp_hz = (mode == 2 ? 1800.0f : 1200.0f) * std::exp2((1.0f - p[kRevDamping]) * 4.0f);
        const float width = p[kRevWidth];
        float len[kLines], gain[kLines];
        for (int k = 0; k < kLines; ++k) {
            len[k] = kLen[k] * scale * 0.001f * sr;
            gain[k] = std::pow(10.0f, -3.0f * len[k] / (rt60 * sr));
            damp[k].set_hz(damp_hz, sr);
        }
        for (int c = 0; c < 2; ++c) {
            lo_cut[c].set_hz(p[kRevLowCut], sr);
            hi_cut[c].set_hz(p[kRevHighCut], sr);
            hi_cut2[c].set_hz(mode == 3 ? 4200.0f : p[kRevHighCut] * 1.4f, sr);
        }
        const float pre_d = std::max(1.0f, p[kRevPreDelay] * sr);
        const float dg = kDiffGain[mode];
        const bool modulate = mode == 1 || mode == 2;
        const float mod_amp = 0.0004f * sr;
        const float mod_inc = 0.7f / sr;
        for (uint32_t i = 0; i < n; ++i) {
            pre.push(0.5f * (l[i] + r[i]) + 0.0f);
            const float x_mono = pre.read(pre_d);
            const float side = 0.5f * (l[i] - r[i]);
            float in[2] = {x_mono + 0.5f * side, x_mono - 0.5f * side};
            for (int c = 0; c < 2; ++c) {
                for (int k = 0; k < kDiff; ++k) {
                    const float d = std::max(1.0f, kDiffMs[k] * (0.5f + 0.5f * scale) * 0.001f * sr) + (c == 1 ? 7.0f : 0.0f);
                    const float z = diff[c][k].read(d);
                    const float v = in[c] + dg * z;
                    diff[c][k].push(v);
                    in[c] = z - dg * v;
                }
            }
            if (mode == 3) { // spring: dispersive chirp
                float x = 0.5f * (in[0] + in[1]);
                for (int k = 0; k < kChirp; ++k) {
                    const float y = -0.62f * x + chirp_x[k] + 0.62f * chirp_y[k];
                    chirp_x[k] = x;
                    chirp_y[k] = y;
                    x = y;
                }
                in[0] = 0.5f * in[0] + x;
                in[1] = 0.5f * in[1] + x;
            }
            const float m = modulate ? mod_amp * sin_turns(static_cast<float>(mod_phase)) : 0.0f;
            mod_phase += static_cast<double>(mod_inc);
            if (mod_phase >= 1.0) mod_phase -= 1.0;
            float y[kLines];
            float sum = 0.0f;
            for (int k = 0; k < kLines; ++k) {
                const float d = len[k] + (k == 0 ? m : (k == 5 ? -m : 0.0f));
                y[k] = damp[k].lp(line[k].read(d)) * gain[k];
                sum += y[k];
            }
            const float h = sum * (2.0f / kLines);
            float out[2] = {0.0f, 0.0f};
            for (int k = 0; k < kLines; ++k) {
                line[k].push(y[k] - h + in[k & 1]);
                out[k & 1] += (k & 2) ? -y[k] : y[k];
            }
            const float mid = 0.5f * (out[0] + out[1]);
            const float sd = 0.5f * (out[0] - out[1]) * width;
            float o[2] = {mid + sd, mid - sd};
            for (int c = 0; c < 2; ++c) o[c] = hi_cut2[c].lp(hi_cut[c].lp(lo_cut[c].hp(o[c]))) * 0.35f;
            l[i] = o[0];
            r[i] = o[1];
        }
        for (OnePole& o : damp) o.flush();
        for (int c = 0; c < 2; ++c) {
            lo_cut[c].flush();
            hi_cut[c].flush();
            hi_cut2[c].flush();
        }
        for (int k = 0; k < kChirp; ++k)
            if (std::fabs(chirp_y[k]) < 1e-20f || !std::isfinite(chirp_y[k])) chirp_y[k] = chirp_x[k] = 0.0f;
    }
};

// ---------------------------------------------------------------- compressor
// Feed-forward, stereo-linked, soft knee. Clean / Punch: peak detector
// (Punch with a faster attack); Opto / Smooth: RMS (Opto with a release that
// slows down as the reduction grows).
struct Compressor {
    float sr = 48000.0f;
    float gr = 0.0f, ms = 0.0f;
    float gr_min = 0.0f;
    void prepare(float rate) { sr = rate; }
    void reset() noexcept { gr = ms = 0.0f; }
    void process(float* l, float* r, uint32_t n, const float* p) noexcept {
        const float thr = p[kCompThreshold];
        const float ratio = p[kCompRatio];
        const int style = static_cast<int>(p[kCompStyle]);
        const float knee = p[kCompKnee];
        const float makeup = db_to_gain(p[kCompMakeup]);
        float att_s = p[kCompAttack], rel_s = p[kCompRelease];
        if (style == 2) att_s *= 0.5f;
        if (style == 3) {
            att_s *= 1.5f;
            rel_s *= 1.5f;
        }
        const float att = 1.0f - std::exp(-1.0f / (att_s * sr));
        const float rel = 1.0f - std::exp(-1.0f / (rel_s * sr));
        const float rms_a = 1.0f - std::exp(-1.0f / ((style == 1 ? 0.01f : 0.025f) * sr));
        const bool rms = style == 1 || style == 3;
        const float slope = 1.0f / ratio - 1.0f;
        gr_min = 0.0f;
        for (uint32_t i = 0; i < n; ++i) {
            const float pk = std::max(std::fabs(l[i]), std::fabs(r[i]));
            float lvl;
            if (rms) {
                ms += rms_a * (pk * pk - ms);
                lvl = 10.0f * std::log10(std::max(ms, 1e-12f));
            } else {
                lvl = 20.0f * std::log10(std::max(pk, 1e-6f));
            }
            const float over = lvl - thr;
            float target = 0.0f;
            if (knee > 0.0f && std::fabs(over) <= 0.5f * knee) {
                const float t = over + 0.5f * knee;
                target = slope * t * t / (2.0f * knee);
            } else if (over > 0.0f) {
                target = slope * over;
            }
            float rel_k = rel;
            if (style == 1) rel_k = rel / (1.0f - 0.08f * std::max(gr, -12.0f)); // opto: slower when deep
            gr += (target < gr ? att : rel_k) * (target - gr);
            gr_min = std::min(gr_min, gr);
            const float g = db_to_gain(gr) * makeup;
            l[i] *= g;
            r[i] *= g;
        }
        if (gr > -1e-6f) gr = 0.0f;
        if (ms < 1e-20f) ms = 0.0f;
    }
};

// ---------------------------------------------------------------- widener
// Mid / side gains, side scaled by Width (0 mono .. 2 extra wide), and the
// side signal high-passed at Low Cut so the bass stays mono.
struct Widener {
    float sr = 48000.0f;
    OnePole side_lp, side_lp2;
    void prepare(float rate) { sr = rate; }
    void reset() noexcept {
        side_lp.reset();
        side_lp2.reset();
    }
    void process(float* l, float* r, uint32_t n, const float* p) noexcept {
        const float width = p[kWideWidth];
        const float mid_g = db_to_gain(p[kWideMid]);
        const float side_g = db_to_gain(p[kWideSide]) * width;
        side_lp.set_hz(p[kWideLowCut], sr);
        side_lp2.set_hz(p[kWideLowCut], sr);
        for (uint32_t i = 0; i < n; ++i) {
            const float m = 0.5f * (l[i] + r[i]) * mid_g;
            float s = 0.5f * (l[i] - r[i]);
            s = s - side_lp2.lp(side_lp.lp(s));
            s *= side_g;
            l[i] = m + s;
            r[i] = m - s;
        }
        side_lp.flush();
        side_lp2.flush();
    }
};

} // namespace

struct FxSlot::Impl {
    Tape tape;
    Distortion distortion;
    Bitcrusher bitcrusher;
    Chorus chorus;
    Flanger flanger;
    Phaser phaser;
    FilterFx filter;
    DelayFx delay;
    Reverb reverb;
    Compressor compressor;
    Widener widener;
    int active = -1;
    std::vector<float> dry_l, dry_r;

    void reset_type(int type) noexcept {
        switch (type) {
        case kTape: tape.reset(); break;
        case kDistortion: distortion.reset(); break;
        case kBitcrusher: bitcrusher.reset(); break;
        case kChorus: chorus.reset(); break;
        case kFlanger: flanger.reset(); break;
        case kPhaser: phaser.reset(); break;
        case kFilter: filter.reset(); break;
        case kDelay: delay.reset(); break;
        case kReverb: reverb.reset(); break;
        case kCompressor: compressor.reset(); break;
        default: widener.reset(); break;
        }
    }
};

FxSlot::FxSlot() : d_(std::make_unique<Impl>()) {}
FxSlot::~FxSlot() = default;

void FxSlot::prepare(double sample_rate, uint32_t max_block) {
    const auto sr = static_cast<float>(sample_rate);
    Impl& d = *d_;
    d.tape.prepare(sr);
    d.distortion.prepare(sr);
    d.chorus.prepare(sr);
    d.flanger.prepare(sr);
    d.phaser.prepare(sr);
    d.filter.prepare(sr);
    d.delay.prepare(sr);
    d.reverb.prepare(sr);
    d.compressor.prepare(sr);
    d.widener.prepare(sr);
    d.dry_l.assign(max_block, 0.0f);
    d.dry_r.assign(max_block, 0.0f);
    reset();
}

void FxSlot::reset() noexcept {
    for (int t = 0; t < kFxTypeCount; ++t) d_->reset_type(t);
    d_->active = -1;
    tel_ = {};
}

void FxSlot::suspend() noexcept { d_->active = -1; }

void FxSlot::process(float* l, float* r, uint32_t n, const float* p, const FxClock& clock) noexcept {
    Impl& d = *d_;
    if (p[kFxOn] < 0.5f) {
        d.active = -1;
        return;
    }
    if (n == 0) return;
    const int type = std::clamp(static_cast<int>(p[kFxType]), 0, kFxTypeCount - 1);
    if (type != d.active) {
        d.reset_type(type);
        d.active = type;
        tel_ = {};
    }
    const float mix = std::clamp(p[kFxMix], 0.0f, 1.0f);
    n = std::min<uint32_t>(n, static_cast<uint32_t>(d.dry_l.size()));
    const bool blend = mix < 1.0f;
    if (blend) {
        std::memcpy(d.dry_l.data(), l, n * sizeof(float));
        std::memcpy(d.dry_r.data(), r, n * sizeof(float));
    }
    switch (type) {
    case kTape: d.tape.process(l, r, n, p); break;
    case kDistortion: d.distortion.process(l, r, n, p); break;
    case kBitcrusher: d.bitcrusher.process(l, r, n, p); break;
    case kChorus:
        d.chorus.process(l, r, n, p, clock);
        tel_.phase = d.chorus.last_phase;
        break;
    case kFlanger:
        d.flanger.process(l, r, n, p, clock);
        tel_.phase = d.flanger.last_phase;
        break;
    case kPhaser:
        d.phaser.process(l, r, n, p, clock);
        tel_.phase = d.phaser.last_phase;
        break;
    case kFilter:
        d.filter.process(l, r, n, p, clock);
        tel_.phase = d.filter.last_phase;
        tel_.cutoff_hz = d.filter.cutoff_now;
        break;
    case kDelay: d.delay.process(l, r, n, p, clock); break;
    case kReverb: d.reverb.process(l, r, n, p); break;
    case kCompressor:
        d.compressor.process(l, r, n, p);
        tel_.gr_db = d.compressor.gr_min;
        break;
    default: d.widener.process(l, r, n, p); break;
    }
    if (!blend) return;
    const float dry = 1.0f - mix;
    for (uint32_t i = 0; i < n; ++i) {
        l[i] = dry * d.dry_l[i] + mix * l[i];
        r[i] = dry * d.dry_r[i] + mix * r[i];
    }
}

} // namespace dstudio::synth_v2
